//! The writer side of schemas: definitions, validation of a unit's write and
//! the maintenance of its index entries (see the [module documentation](super)).

use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;

use kerno_core::contract::{Snapshot, Txn, TxnEngine};

use super::{
    Document, FieldDef, RecordRef, Schema, entry_key, entry_key_parts, entry_value_for, field_prefix,
    prefix_bounds, schema_key,
};
use crate::codec::{Key, TAG_UNIT_HEAD, UnitHead};
use crate::db::{
    ChangeMeta, ChangeSet, CommitInfo, NewUnit, UnitDb, UnitUpdate, Write, context, corruption, load_unit,
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
    // The whole record is read, as `Document::parse` would, but no document is built: only the
    // key field's value is encoded.
    let mut found = None;
    super::walk_record(payload, |name, value| {
        if name == field {
            found = Some(value.index_bytes());
        }
    })
    .map_err(|m| violation(format!("upsert payload: {m}")))?;
    found.ok_or_else(|| violation(format!("upsert payload has no value for the unique field {field:?}")))
}

/// A unit of a typed unit type that is not deleted, with its record.
struct Typed {
    unit: UnitId,
    record: Document,
    payload: Vec<u8>,
    lifecycle: Lifecycle,
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// The schema of `unit_type` as this change set sees it.
    ///
    /// Read once per type and change set: [`ChangeSet::write_owned`] forgets what was read when
    /// it writes a schema record (or deletes a range).
    pub(crate) fn read_schema(&self, unit_type: &str) -> Result<Option<Arc<Schema>>, UnitError> {
        if let Some(known) = self.schema_cache.borrow().get(unit_type) {
            return Ok(known.clone());
        }
        let schema = match self.txn()?.get(&schema_key(unit_type))? {
            Some(raw) => Some(Arc::new(Schema::decode(&raw).map_err(|e| context(e, "SCHEMA"))?)),
            None => None,
        };
        self.schema_cache.borrow_mut().insert(unit_type.to_owned(), schema.clone());
        Ok(schema)
    }

    /// The writes that keep the entries of a unit in step with the operation
    /// `writes` (the records of its new revision), refusing it with
    /// [`UnitError::SchemaViolation`] when the unit's type has a schema that its
    /// new payload does not satisfy. Reads only, before the first write of the
    /// operation, like [`ChangeSet::plan_fulltext`](crate::fulltext): a refusal
    /// leaves the change set untouched. Nothing for a unit type without a
    /// schema, and nothing for operations that write no head (retention).
    pub(crate) fn plan_schema(
        &self,
        unit: UnitId,
        next: Option<&crate::db::PlannedHead<'_>>,
    ) -> Result<Vec<Write>, UnitError> {
        let Some(&(ref head, payload)) = next else {
            return Ok(Vec::new());
        };
        let Some(schema) = self.read_schema(&head.unit_type)? else {
            return Ok(Vec::new());
        };
        let new = if head.lifecycle == Lifecycle::Active {
            let Some(payload) = payload else {
                return Ok(Vec::new());
            };
            // Read in place: the record is checked and its entries encoded, no document built.
            let record = RecordRef::parse(payload).map_err(|m| violation(format!("unit {unit}: {m}")))?;
            schema.validate_ref(&record).map_err(|m| violation(format!("unit {unit}: {m}")))?;
            Some(record)
        } else {
            None
        };
        let txn = self.txn()?;
        let stored = match load_unit(&|k| txn.get(k), unit, true)? {
            Some((h, Some(payload))) if h.lifecycle == Lifecycle::Active => Some(payload),
            _ => None,
        };
        let old = match &stored {
            Some(payload) => Some(
                RecordRef::parse(payload)
                    .map_err(|m| corruption(format!("unit {unit}: its stored payload is not a record: {m}")))?,
            ),
            None => None,
        };
        let entries =
            |r: Option<&RecordRef<'_>>, refuse: fn(String) -> UnitError| -> Result<BTreeMap<usize, Vec<u8>>, UnitError> {
                match r {
                    Some(r) => Ok(schema
                        .entries_ref(r)
                        .map_err(|m| refuse(format!("unit {unit}: {m}")))?
                        .into_iter()
                        .collect()),
                    None => Ok(BTreeMap::new()),
                }
            };
        let (old, new) = (entries(old.as_ref(), corruption)?, entries(new.as_ref(), violation)?);
        let mut out = Vec::with_capacity(old.len() + new.len());
        let name = |i: usize| schema.fields()[i].name.as_str();
        for (i, enc) in old.iter().filter(|(i, enc)| new.get(*i) != Some(*enc)) {
            out.push(Write::Delete(entry_key(&head.unit_type, name(*i), enc, unit)));
        }
        // An entry of a covering field holds the payload, so it is written again whenever the
        // unit gets a new revision, also when its value did not change (units-v13.md).
        for (i, enc) in &new {
            let def = &schema.fields()[*i];
            let same = old.get(i) == Some(enc);
            if same && !def.covering {
                continue;
            }
            if def.unique && !same {
                self.ensure_unique(&head.unit_type, name(*i), enc, unit)?;
            }
            out.push(Write::Put(entry_key(&head.unit_type, name(*i), enc, unit), entry_value_for(def, payload)));
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
    /// `NotUnique` when the type has no schema or `field` is not declared `unique` in it;
    /// `SchemaViolation` when the payload is not a record the schema accepts or has no value in
    /// `field`; otherwise as [`create`](Self::create) and [`update`](Self::update).
    pub fn upsert(&mut self, new: NewUnit, field: &str) -> Result<Upserted, UnitError> {
        self.upsert_prepared(new, field, None)
    }

    /// [`upsert`](Self::upsert), with `hash` the content hash of `new.payload` computed ahead
    /// (`ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, ..)`; `None`: computed here, as `upsert` does).
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert).
    pub fn upsert_prepared(
        &mut self,
        new: NewUnit,
        field: &str,
        hash: Option<crate::payload::ContentHash>,
    ) -> Result<Upserted, UnitError> {
        self.upsert_keyed(new, field, hash, None)
    }

    /// [`upsert_prepared`](Self::upsert_prepared), with `key` what [`upsert_key`] gives for
    /// `new.payload` and `field`, computed ahead (`None`: computed here).
    ///
    /// # Errors
    ///
    /// As [`upsert`](Self::upsert).
    pub fn upsert_keyed(
        &mut self,
        new: NewUnit,
        field: &str,
        hash: Option<crate::payload::ContentHash>,
        key: Option<&[u8]>,
    ) -> Result<Upserted, UnitError> {
        self.prepared_hash.set(hash.map(|h| (new.payload.len(), h)));
        let done = self.upsert_inner(new, field, key);
        self.prepared_hash.set(None);
        done
    }

    fn upsert_inner(&mut self, new: NewUnit, field: &str, key: Option<&[u8]>) -> Result<Upserted, UnitError> {
        check_type(&new.unit_type)?;
        let not_unique = |detail: String| UnitError::NotUnique { detail };
        let Some(schema) = self.read_schema(&new.unit_type)? else {
            return Err(not_unique(format!("the unit type {:?} has no schema", new.unit_type)));
        };
        match schema.field(field) {
            Some(def) if def.unique => {}
            Some(_) => {
                return Err(not_unique(format!(
                    "field {field:?} is not declared unique in the schema of {:?}",
                    new.unit_type
                )));
            }
            None => return Err(not_unique(format!("the schema of {:?} has no field {field:?}", new.unit_type))),
        }
        let mut prefix = field_prefix(&new.unit_type, field);
        let field_len = prefix.len();
        match key {
            Some(key) => prefix.extend_from_slice(key),
            None => prefix.extend_from_slice(&upsert_key(&new.payload, field)?),
        }
        let held = self.unique_holders(&prefix, field_len)?;
        let Some(&id) = held.first() else {
            // No unit holds the value: the create's own uniqueness check of this field would scan
            // the same range again, with no write in between (see `ensure_unique`).
            *self.unique_free.get_mut() = Some(prefix);
            let created = self.create(new);
            *self.unique_free.get_mut() = None;
            let id = created?;
            return Ok(Upserted { id, outcome: UpsertOutcome::Created });
        };
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
        let field_len = prefix.len();
        prefix.extend_from_slice(value);
        // `upsert` found no holder of this value just before, and nothing was written since.
        if self.unique_free.borrow().as_deref() == Some(prefix.as_slice()) {
            return Ok(());
        }
        for holder in self.unique_holders(&prefix, field_len)? {
            if holder != unit {
                return Err(violation(format!(
                    "unit {unit}: another unit already holds the unique value of {field:?}"
                )));
            }
        }
        Ok(())
    }

    /// The units whose entry of a unique field starts with `prefix` (`field_prefix ‖ value`) as
    /// this change set sees them, in id order (at most two of the stored ones are read: one
    /// holder is the rule, a second one the violation). The stored entries come from the state
    /// the transaction began on, with one bounded seek, and this change set's own entries of
    /// unique fields from `unique_staged`: the transaction never folds its buffer of writes
    /// for it. Without that state (a range deletion), the transaction is scanned. `field_len`
    /// is the length of the `field_prefix` part: a field the snapshot holds no entry of at all
    /// (learned once per change set) is not read from it again.
    fn unique_holders(&self, prefix: &[u8], field_len: usize) -> Result<Vec<UnitId>, UnitError> {
        let (start, end) = prefix_bounds(prefix);
        let unit_of = |k: &[u8]| -> Result<UnitId, UnitError> {
            Ok(entry_key_parts(k).map_err(|e| context(e, "FIELD_INDEX"))?.3)
        };
        let Some(base) = self.base()? else {
            let page = self.txn()?.scan(Bound::Included(&start), Bound::Excluded(&end), 2)?;
            return page.iter().map(|(k, _)| unit_of(k)).collect();
        };
        let staged = self.unique_staged.get(prefix).map_or(&[][..], Vec::as_slice);
        let removed = staged.iter().filter(|(_, put)| !put).count();
        let field = &prefix[..field_len];
        let known = self.base_field_empty.borrow().get(field).copied();
        let field_empty = match known {
            Some(empty) => empty,
            None => {
                let (fs, fe) = prefix_bounds(field);
                let empty = base.scan(Bound::Included(&fs), Bound::Excluded(&fe), 1)?.is_empty();
                self.base_field_empty.borrow_mut().insert(field.to_vec(), empty);
                empty
            }
        };
        let mut held = Vec::new();
        if !field_empty {
            for (k, _) in base.scan(Bound::Included(&start), Bound::Excluded(&end), 2 + removed)? {
                held.push(unit_of(&k)?);
            }
        }
        for (unit, put) in staged {
            if *put {
                if !held.contains(unit) {
                    held.push(*unit);
                }
            } else {
                held.retain(|h| h != unit);
            }
        }
        held.sort_unstable();
        Ok(held)
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
                let Some((_, Some(payload))) = load_unit(&|k| txn.get(k), unit, true)? else {
                    return Err(corruption(format!(
                        "unit {unit}: a {:?} head revision has no payload",
                        head.lifecycle
                    )));
                };
                let record = Document::parse(&payload)
                    .map_err(|m| violation(format!("existing unit {unit} is not a record: {m}")))?;
                schema.validate(&record).map_err(|m| violation(format!("existing unit {unit}: {m}")))?;
                out.push(Typed { unit, record, payload, lifecycle: head.lifecycle });
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
        only: Option<&[usize]>,
    ) -> Result<Vec<Write>, UnitError> {
        let mut seen: BTreeMap<(usize, Vec<u8>), UnitId> = BTreeMap::new();
        let mut out = Vec::new();
        for t in units.iter().filter(|t| t.lifecycle == Lifecycle::Active) {
            let entries = schema.entries(&t.record).map_err(|m| violation(format!("existing unit {}: {m}", t.unit)))?;
            for (i, enc) in entries.into_iter().filter(|(i, _)| only.is_none_or(|o| o.contains(i))) {
                let field = &schema.fields()[i];
                if field.unique && seen.insert((i, enc.clone()), t.unit).is_some() {
                    return Err(violation(format!(
                        "two existing units hold the same value of unique field {:?}",
                        field.name
                    )));
                }
                out.push(Write::Put(
                    entry_key(unit_type, &field.name, &enc, t.unit),
                    entry_value_for(field, Some(&t.payload)),
                ));
            }
        }
        Ok(out)
    }

    fn put_schema(&mut self, unit_type: &str, schema: &Schema, more: Vec<Write>) -> Result<(), UnitError> {
        // A covering field needs a build that keeps its entries' payloads: the first one raises
        // the mark to 13 in this same transaction, so an older build refuses the database.
        if schema.has_covering() {
            self.raise_layer(crate::codec::LAYER_VERSION_COVERING)?;
        }
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
    /// A schema that differs from the type's only in the `covering` flags is accepted: the
    /// entries of those fields are written again for every `Active` unit, in this change set.
    ///
    /// `InvalidArgument` for an invalid type or schema; `SchemaExists` for a type that has a
    /// different schema already (the same schema again is a no-op); `SchemaViolation` for an existing unit the schema refuses;
    /// `ChangeSetTooLarge` when the entries exceed the engine's batch limits.
    pub fn define_schema(&mut self, unit_type: &str, fields: Vec<FieldDef>) -> Result<(), UnitError> {
        self.guarded(|s| s.define_schema_unguarded(unit_type, fields))
    }

    fn define_schema_unguarded(&mut self, unit_type: &str, fields: Vec<FieldDef>) -> Result<(), UnitError> {
        self.txn()?;
        check_type(unit_type)?;
        let schema = Schema::new(fields)?;
        if let Some(have) = self.read_schema(unit_type)? {
            // The same schema again changes nothing, so a caller can define on every start.
            if *have == schema {
                return Ok(());
            }
            // Only `covering` differs: the entries of those fields are written again, with or
            // without the payload (the keys are the same, so every entry is overwritten).
            let plain = |s: &Schema| -> Vec<FieldDef> {
                s.fields().iter().map(|f| FieldDef { covering: false, ..f.clone() }).collect()
            };
            if plain(&have) != plain(&schema) {
                return Err(UnitError::SchemaExists { unit_type: unit_type.into() });
            }
            let changed: Vec<usize> =
                (0..schema.fields().len()).filter(|&i| have.fields()[i].covering != schema.fields()[i].covering).collect();
            let units = self.typed_units(unit_type, &schema)?;
            let entries = self.backfill(unit_type, &schema, &units, Some(&changed))?;
            return self.put_schema(unit_type, &schema, entries);
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
        let entries = self.backfill(unit_type, &schema, &units, Some(&[at]))?;
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
        schema.fields[at].covering = false;
        let (start, end) = prefix_bounds(&field_prefix(unit_type, field));
        self.put_schema(unit_type, &schema, vec![Write::DeleteRange(start, end)])
    }

    fn field_of(&self, unit_type: &str, field: &str) -> Result<(Schema, usize), UnitError> {
        check_type(unit_type)?;
        let Some(schema) = self.read_schema(unit_type)? else {
            return Err(UnitError::InvalidArgument("the unit type has no schema".into()));
        };
        match schema.fields().iter().position(|f| f.name == field) {
            Some(at) => Ok((Schema::clone(&schema), at)),
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
