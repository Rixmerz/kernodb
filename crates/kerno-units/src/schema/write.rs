//! The writer side of schemas: definitions, validation of a unit's write and
//! the maintenance of its index entries (see the [module documentation](super)).

use std::collections::BTreeMap;
use std::ops::Bound;

use kerno_core::contract::{Txn, TxnEngine};

use super::{
    Document, FieldDef, FieldValue, Schema, decode_entry_key, entry_key, entry_value, field_prefix, prefix_bounds,
    schema_key,
};
use crate::codec::{Key, TAG_UNIT_HEAD, UnitHead};
use crate::db::{
    ChangeMeta, ChangeSet, CommitInfo, NewUnit, UnitDb, UnitUpdate, Write, context, corruption, key, load_unit,
};
use crate::error::UnitError;
use crate::types::{Lifecycle, UnitId};

/// Heads read per scan call by a backfill.
const HEAD_PAGE: usize = 256;

fn violation(detail: String) -> UnitError {
    UnitError::SchemaViolation { detail }
}

/// The unit types are 1..=64 bytes, like `UNIT_HEAD`'s.
fn check_type(unit_type: &str) -> Result<(), UnitError> {
    if (1..=crate::codec::MAX_UNIT_TYPE_LEN as usize).contains(&unit_type.len()) {
        Ok(())
    } else {
        Err(UnitError::InvalidArgument(format!(
            "unit_type of {} bytes is outside 1..={}",
            unit_type.len(),
            crate::codec::MAX_UNIT_TYPE_LEN
        )))
    }
}

/// What [`ChangeSet::upsert`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpsertOutcome {
    /// No active unit held the key: a new unit was stored.
    Created,
    /// The unit holding the key got a new revision.
    Updated,
    /// The unit holding the key already had this payload: nothing was written.
    Unchanged,
}

/// The unit [`ChangeSet::upsert`] stored or found, and what happened to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upserted {
    pub id: UnitId,
    pub outcome: UpsertOutcome,
}

/// The index encoding of the value `payload` holds in `field`: what two payloads must share to
/// be the same key of [`ChangeSet::upsert`].
///
/// # Errors
///
/// `SchemaViolation` when `payload` is not a record or has no value in `field`.
pub fn upsert_key(payload: &[u8], field: &str) -> Result<Vec<u8>, UnitError> {
    let record = Document::parse(payload).map_err(|m| violation(format!("upsert payload: {m}")))?;
    record
        .get(field)
        .and_then(FieldValue::index_bytes)
        .ok_or_else(|| violation(format!("upsert payload has no value for the unique field {field:?}")))
}

/// A unit of a typed unit type that is not deleted, with its record.
struct Typed {
    unit: UnitId,
    record: Document,
    lifecycle: Lifecycle,
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// The schema of `unit_type` as this change set sees it.
    pub(crate) fn read_schema(&self, unit_type: &str) -> Result<Option<Schema>, UnitError> {
        match self.txn()?.get(&schema_key(unit_type))? {
            Some(raw) => Ok(Some(Schema::decode(&raw).map_err(|e| context(e, "SCHEMA"))?)),
            None => Ok(None),
        }
    }

    /// The writes that keep the entries of a unit in step with the operation
    /// `writes` (the records of its new revision), refusing it with
    /// [`UnitError::SchemaViolation`] when the unit's type has a schema that its
    /// new payload does not satisfy. Reads only, before the first write of the
    /// operation, like [`ChangeSet::plan_fulltext`](crate::fulltext): a refusal
    /// leaves the change set untouched. Nothing for a unit type without a
    /// schema, and nothing for operations that write no head (retention).
    pub(crate) fn plan_schema(&self, unit: UnitId, writes: &[Write]) -> Result<Vec<Write>, UnitError> {
        let head_key = key(Key::UnitHead { unit })?;
        let Some(Write::Put(_, raw)) = writes.iter().find(|w| matches!(w, Write::Put(k, _) if *k == head_key)) else {
            return Ok(Vec::new());
        };
        let head = UnitHead::decode(raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
        let Some(schema) = self.read_schema(&head.unit_type)? else {
            return Ok(Vec::new());
        };
        let new = if head.lifecycle == Lifecycle::Active {
            let payload_key = key(Key::RevPayload { unit, rev: head.head_rev })?;
            let Some(Write::Put(_, payload)) =
                writes.iter().find(|w| matches!(w, Write::Put(k, _) if *k == payload_key))
            else {
                return Ok(Vec::new());
            };
            let record = Document::parse(payload).map_err(|m| violation(format!("unit {unit}: {m}")))?;
            schema.validate(&record).map_err(|m| violation(format!("unit {unit}: {m}")))?;
            Some(record)
        } else {
            None
        };
        let txn = self.txn()?;
        let old = match load_unit(&|k| txn.get(k), unit)? {
            Some((h, _, Some(payload))) if h.lifecycle == Lifecycle::Active => {
                let record = Document::parse(&payload)
                    .map_err(|m| corruption(format!("unit {unit}: its stored payload is not a record: {m}")))?;
                Some(record)
            }
            _ => None,
        };
        let entries =
            |r: Option<&Document>, refuse: fn(String) -> UnitError| -> Result<BTreeMap<usize, Vec<u8>>, UnitError> {
                match r {
                    Some(r) => {
                        Ok(schema.entries(r).map_err(|m| refuse(format!("unit {unit}: {m}")))?.into_iter().collect())
                    }
                    None => Ok(BTreeMap::new()),
                }
            };
        let (old, new) = (entries(old.as_ref(), corruption)?, entries(new.as_ref(), violation)?);
        let mut out = Vec::new();
        let name = |i: usize| schema.fields()[i].name.as_str();
        for (i, enc) in old.iter().filter(|(i, enc)| new.get(*i) != Some(*enc)) {
            out.push(Write::Delete(entry_key(&head.unit_type, name(*i), enc, unit)));
        }
        for (i, enc) in new.iter().filter(|(i, enc)| old.get(*i) != Some(*enc)) {
            if schema.fields()[*i].unique {
                self.ensure_unique(&head.unit_type, name(*i), enc, unit)?;
            }
            out.push(Write::Put(entry_key(&head.unit_type, name(*i), enc, unit), entry_value()));
        }
        Ok(out)
    }

    /// Stores `new` unless an `Active` unit of its type already holds the value `new.payload` has
    /// in the unique `field`; then that unit is updated to `new.payload` (and left alone when it
    /// has it already). The lookup reads this change set's own writes, so a key stored earlier in
    /// it is found, but a unit already revised in this change set cannot be revised again
    /// (`InvalidArgument`; the `kerno` facade's `Batch` merges repeated keys first).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when the type has no schema or `field` is not declared `unique` in it;
    /// `SchemaViolation` when the payload is not a record the schema accepts or has no value in
    /// `field`; otherwise as [`create`](Self::create) and [`update`](Self::update).
    pub fn upsert(&mut self, new: NewUnit, field: &str) -> Result<Upserted, UnitError> {
        let (schema, at) = self.field_of(&new.unit_type, field)?;
        if !schema.fields()[at].unique {
            return Err(UnitError::InvalidArgument(format!(
                "field {field:?} is not declared unique in the schema of {:?}",
                new.unit_type
            )));
        }
        let mut prefix = field_prefix(&new.unit_type, field);
        prefix.extend_from_slice(&upsert_key(&new.payload, field)?);
        let (start, end) = prefix_bounds(&prefix);
        let held = self.txn()?.scan(Bound::Included(&start), Bound::Excluded(&end), 1)?;
        let Some((k, _)) = held.first() else {
            let id = self.create(new)?;
            return Ok(Upserted { id, outcome: UpsertOutcome::Created });
        };
        let (_, _, _, id) = decode_entry_key(k).map_err(|e| context(e, "FIELD_INDEX"))?;
        let current = self.get_unit(id)?.and_then(|u| u.payload);
        if current.as_deref() == Some(new.payload.as_slice()) {
            return Ok(Upserted { id, outcome: UpsertOutcome::Unchanged });
        }
        self.update(id, UnitUpdate { payload: new.payload, schema_version: None, metadata: None }, None)?;
        Ok(Upserted { id, outcome: UpsertOutcome::Updated })
    }

    /// Refuses when an `Active` unit other than `unit` already holds `value`.
    fn ensure_unique(&self, unit_type: &str, field: &str, value: &[u8], unit: UnitId) -> Result<(), UnitError> {
        let mut prefix = field_prefix(unit_type, field);
        prefix.extend_from_slice(value);
        let (start, end) = prefix_bounds(&prefix);
        for (k, _) in self.txn()?.scan(Bound::Included(&start), Bound::Excluded(&end), 2)? {
            let (_, _, _, holder) = decode_entry_key(&k).map_err(|e| context(e, "FIELD_INDEX"))?;
            if holder != unit {
                return Err(violation(format!(
                    "unit {unit}: another unit already holds the unique value of {field:?}"
                )));
            }
        }
        Ok(())
    }

    /// Every unit of `unit_type` that is not deleted, validated against
    /// `schema`: the first that does not satisfy it refuses the definition.
    fn typed_units(&self, unit_type: &str, schema: &Schema) -> Result<Vec<Typed>, UnitError> {
        let txn = self.txn()?;
        let (_, end) = Key::tag_range(TAG_UNIT_HEAD);
        let mut cursor = vec![TAG_UNIT_HEAD];
        let mut inclusive = true;
        let mut out = Vec::new();
        loop {
            let lo = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = txn.scan(lo, Bound::Excluded(end.as_slice()), HEAD_PAGE)?;
            let n = page.len();
            for (k, v) in &page {
                let Key::UnitHead { unit } = Key::decode(k)? else {
                    return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
                };
                let head = UnitHead::decode(v).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
                if head.unit_type != unit_type || head.lifecycle == Lifecycle::Deleted {
                    continue;
                }
                let Some((_, _, Some(payload))) = load_unit(&|k| txn.get(k), unit)? else {
                    return Err(corruption(format!(
                        "unit {unit}: a {:?} head revision has no payload",
                        head.lifecycle
                    )));
                };
                let record = Document::parse(&payload)
                    .map_err(|m| violation(format!("existing unit {unit} is not a record: {m}")))?;
                schema.validate(&record).map_err(|m| violation(format!("existing unit {unit}: {m}")))?;
                out.push(Typed { unit, record, lifecycle: head.lifecycle });
            }
            match page.into_iter().last() {
                Some((k, _)) if n == HEAD_PAGE => {
                    cursor = k;
                    inclusive = false;
                }
                _ => return Ok(out),
            }
        }
    }

    /// The entries of the `Active` `units` for the fields of `schema` selected
    /// by `only` (all indexed fields when `None`), refusing a repeated value of
    /// a unique field.
    fn backfill(
        &self,
        unit_type: &str,
        schema: &Schema,
        units: &[Typed],
        only: Option<usize>,
    ) -> Result<Vec<Write>, UnitError> {
        let mut seen: BTreeMap<(usize, Vec<u8>), UnitId> = BTreeMap::new();
        let mut out = Vec::new();
        for t in units.iter().filter(|t| t.lifecycle == Lifecycle::Active) {
            let entries = schema.entries(&t.record).map_err(|m| violation(format!("existing unit {}: {m}", t.unit)))?;
            for (i, enc) in entries.into_iter().filter(|(i, _)| only.is_none_or(|o| o == *i)) {
                let field = &schema.fields()[i];
                if field.unique && seen.insert((i, enc.clone()), t.unit).is_some() {
                    return Err(violation(format!(
                        "two existing units hold the same value of unique field {:?}",
                        field.name
                    )));
                }
                out.push(Write::Put(entry_key(unit_type, &field.name, &enc, t.unit), entry_value()));
            }
        }
        Ok(out)
    }

    fn put_schema(&mut self, unit_type: &str, schema: &Schema, more: Vec<Write>) -> Result<(), UnitError> {
        self.write(&Write::Put(schema_key(unit_type), schema.encode()))?;
        for w in &more {
            self.write(w)?;
        }
        self.schema_ops += 1;
        Ok(())
    }

    /// Gives `unit_type` a schema, in this change set: validates every
    /// existing unit of the type that is not deleted against it and writes the
    /// entries of the indexed fields of the `Active` ones. A type holds one
    /// schema for good; a refusal (an existing unit it does not accept, a
    /// repeated unique value) writes nothing.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an invalid type or schema, or a type that has one
    /// already; `SchemaViolation` for an existing unit the schema refuses;
    /// `ChangeSetTooLarge` when the entries exceed the engine's batch limits.
    pub fn define_schema(&mut self, unit_type: &str, fields: Vec<FieldDef>) -> Result<(), UnitError> {
        self.guarded(|s| s.define_schema_unguarded(unit_type, fields))
    }

    fn define_schema_unguarded(&mut self, unit_type: &str, fields: Vec<FieldDef>) -> Result<(), UnitError> {
        self.txn()?;
        check_type(unit_type)?;
        let schema = Schema::new(fields)?;
        if self.read_schema(unit_type)?.is_some() {
            return Err(UnitError::InvalidArgument("the unit type already has a schema".into()));
        }
        let units = self.typed_units(unit_type, &schema)?;
        let entries = self.backfill(unit_type, &schema, &units, None)?;
        self.put_schema(unit_type, &schema, entries)
    }

    /// Indexes `field` of `unit_type`: marks it so in the schema and writes the
    /// entries of the `Active` units that have it, in this change set.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when there is no such schema or field, or the field is
    /// indexed already; `SchemaViolation` for an existing value too long to
    /// index; `ChangeSetTooLarge` as for [`define_schema`](Self::define_schema).
    pub fn create_index(&mut self, unit_type: &str, field: &str) -> Result<(), UnitError> {
        self.guarded(|s| s.create_index_unguarded(unit_type, field))
    }

    fn create_index_unguarded(&mut self, unit_type: &str, field: &str) -> Result<(), UnitError> {
        self.txn()?;
        let (mut schema, at) = self.field_of(unit_type, field)?;
        if schema.fields[at].indexed {
            return Err(UnitError::InvalidArgument(format!("field {field:?} is indexed already")));
        }
        schema.fields[at].indexed = true;
        let units = self.typed_units(unit_type, &schema)?;
        let entries = self.backfill(unit_type, &schema, &units, Some(at))?;
        self.put_schema(unit_type, &schema, entries)
    }

    /// Stops indexing `field` of `unit_type` and removes its entries, in this
    /// change set. A unique field keeps its index.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when there is no such schema or field, the field is not
    /// indexed, or it is unique.
    pub fn drop_index(&mut self, unit_type: &str, field: &str) -> Result<(), UnitError> {
        self.guarded(|s| s.drop_index_unguarded(unit_type, field))
    }

    fn drop_index_unguarded(&mut self, unit_type: &str, field: &str) -> Result<(), UnitError> {
        self.txn()?;
        let (mut schema, at) = self.field_of(unit_type, field)?;
        match &schema.fields[at] {
            f if f.unique => {
                return Err(UnitError::InvalidArgument(format!("field {field:?} is unique and keeps its index")));
            }
            f if !f.indexed => return Err(UnitError::InvalidArgument(format!("field {field:?} is not indexed"))),
            _ => {}
        }
        schema.fields[at].indexed = false;
        let (start, end) = prefix_bounds(&field_prefix(unit_type, field));
        self.put_schema(unit_type, &schema, vec![Write::DeleteRange(start, end)])
    }

    fn field_of(&self, unit_type: &str, field: &str) -> Result<(Schema, usize), UnitError> {
        check_type(unit_type)?;
        let Some(schema) = self.read_schema(unit_type)? else {
            return Err(UnitError::InvalidArgument("the unit type has no schema".into()));
        };
        match schema.fields().iter().position(|f| f.name == field) {
            Some(at) => Ok((schema, at)),
            None => Err(UnitError::InvalidArgument(format!("the schema has no field {field:?}"))),
        }
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// [`ChangeSet::define_schema`] in its own change set.
    pub fn define_schema(
        &self,
        unit_type: &str,
        fields: Vec<FieldDef>,
        meta: ChangeMeta,
    ) -> Result<CommitInfo, UnitError> {
        let mut cs = self.begin_changeset(meta)?;
        cs.define_schema(unit_type, fields)?;
        cs.commit()
    }

    /// [`ChangeSet::create_index`] in its own change set.
    pub fn create_index(&self, unit_type: &str, field: &str, meta: ChangeMeta) -> Result<CommitInfo, UnitError> {
        let mut cs = self.begin_changeset(meta)?;
        cs.create_index(unit_type, field)?;
        cs.commit()
    }

    /// [`ChangeSet::drop_index`] in its own change set.
    pub fn drop_index(&self, unit_type: &str, field: &str, meta: ChangeMeta) -> Result<CommitInfo, UnitError> {
        let mut cs = self.begin_changeset(meta)?;
        cs.drop_index(unit_type, field)?;
        cs.commit()
    }
}
