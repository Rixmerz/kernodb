//! The reader side of schemas: the schema of a type and lookups through the
//! entries of an indexed field.

use std::ops::Bound;
use std::sync::Arc;

use kerno_core::contract::{Engine, Snapshot};

use super::{
    Document, FieldValue, MAX_INDEXED_TEXT_LEN, MAX_INDEXED_VALUE_LEN, Schema, entry_key_parts, field_prefix,
    prefix_bounds, schema_key,
};
use crate::db::{Unit, UnitSnapshot, context, corruption};
use crate::error::UnitError;
use crate::types::Lifecycle;

/// Entries read per scan call by a lookup.
const PAGE: usize = 256;

/// Owned copies of shared units, for the methods that hand out `Unit`.
fn owned(units: Vec<Arc<Unit>>) -> Vec<Unit> {
    units.into_iter().map(Arc::unwrap_or_clone).collect()
}

impl<E: Engine> UnitSnapshot<E> {
    /// The schema of `unit_type`, if it has one.
    ///
    /// # Errors
    ///
    /// `Storage(Corruption)` when the record does not decode.
    pub fn schema(&self, unit_type: &str) -> Result<Option<Schema>, UnitError> {
        let seq = self.snap.seq();
        if let Some(hit) = self.read_cache.schema(seq, unit_type) {
            return Ok(hit.map(|s| (*s).clone()));
        }
        let schema = match self.snap.get(&schema_key(unit_type))? {
            Some(raw) => Some(Schema::decode(&raw).map_err(|e| context(e, "SCHEMA"))?),
            None => None,
        };
        self.read_cache.put_schema(seq, unit_type, schema.as_ref());
        Ok(schema)
    }

    /// The `Active` units of `unit_type` whose indexed `field` equals `value`,
    /// in unit id order.
    ///
    /// # Errors
    ///
    /// As [`find_range`](Self::find_range), and `InvalidArgument` for a text or bytes `value` too
    /// long to index (no unit can hold it).
    pub fn find_by_field(&self, unit_type: &str, field: &str, value: &FieldValue) -> Result<Vec<Unit>, UnitError> {
        Ok(owned(self.find_by_field_shared(unit_type, field, value)?))
    }

    /// [`find_by_field`](Self::find_by_field), sharing the units the read cache holds instead of
    /// cloning them.
    pub fn find_by_field_shared(
        &self,
        unit_type: &str,
        field: &str,
        value: &FieldValue,
    ) -> Result<Vec<Arc<Unit>>, UnitError> {
        // A value too long to index can never have been stored, so "no unit" would be a wrong
        // answer to a wrong question. (A range bound this long is meaningful and still allowed.)
        self.field_bounds(unit_type, field, Bound::Included(value), Bound::Included(value))?;
        if value.index_bytes().is_some_and(|e| e.len() > MAX_INDEXED_VALUE_LEN) {
            return Err(UnitError::InvalidArgument(format!(
                "the value is too long to be indexed (at most {MAX_INDEXED_TEXT_LEN} bytes of text or bytes, each \
                 0x00 byte counting twice), so no unit can hold it"
            )));
        }
        self.find_range_shared(unit_type, field, Bound::Included(value), Bound::Included(value), usize::MAX)
    }

    /// The `Active` units of `unit_type` whose indexed `field` lies between
    /// `lo` and `hi`, by value then unit id, at most `limit`. Text and bytes
    /// order as bytes, floats as numbers (`-0.0` equals `0.0`).
    ///
    /// Each unit returned is read and verified like `get_unit`, and its entry
    /// must agree with its payload. An entry that is *missing* cannot be seen
    /// from here: that is [`verify_units`](Self::verify_units) (UNIT-50).
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when the type has no schema, the field is not declared
    /// or not indexed, a bound is not of the field's type, `limit` is 0, or the bounds leave an
    /// empty range (`lo` above `hi`, or equal with one excluded); `Storage(Corruption)`
    /// when an entry disagrees with its unit.
    pub fn find_range(
        &self,
        unit_type: &str,
        field: &str,
        lo: Bound<&FieldValue>,
        hi: Bound<&FieldValue>,
        limit: usize,
    ) -> Result<Vec<Unit>, UnitError> {
        Ok(owned(self.find_range_shared(unit_type, field, lo, hi, limit)?))
    }

    /// [`find_range`](Self::find_range), sharing the units the read cache holds instead of
    /// cloning them.
    pub fn find_range_shared(
        &self,
        unit_type: &str,
        field: &str,
        lo: Bound<&FieldValue>,
        hi: Bound<&FieldValue>,
        limit: usize,
    ) -> Result<Vec<Arc<Unit>>, UnitError> {
        // A range that can hold no value, or a limit of 0, can never match: refuse, do not answer [].
        if limit == 0 {
            return Err(UnitError::InvalidArgument("find_range: limit must be at least 1".into()));
        }
        let (start, end) = self.field_bounds(unit_type, field, lo, hi)?;
        if start >= end {
            return Err(UnitError::InvalidArgument("find_range: the range is empty (low is above high)".into()));
        }
        self.scan_range(unit_type, field, start, end, limit)
    }

    /// The scan behind [`find_range`](Self::find_range), without its refusal of an empty range: the
    /// query language derives bounds from a predicate (`n > i64::MAX`), and no unit matching it is
    /// then a true answer.
    pub(crate) fn find_range_lenient(
        &self,
        unit_type: &str,
        field: &str,
        lo: Bound<&FieldValue>,
        hi: Bound<&FieldValue>,
        limit: usize,
    ) -> Result<Vec<Unit>, UnitError> {
        let (start, end) = self.field_bounds(unit_type, field, lo, hi)?;
        if start >= end {
            return Ok(Vec::new());
        }
        Ok(owned(self.scan_range(unit_type, field, start, end, limit)?))
    }

    fn scan_range(
        &self,
        unit_type: &str,
        field: &str,
        start: Vec<u8>,
        end: Vec<u8>,
        limit: usize,
    ) -> Result<Vec<Arc<Unit>>, UnitError> {
        // A range this snapshot's sequence already scanned and checked: its units, as read then.
        let seq = self.snap.seq();
        let cache_key = (start, end, limit);
        if let Some(ids) = self.read_cache.range(seq, &cache_key) {
            let mut out = Vec::with_capacity(ids.len());
            for (&id, u) in ids.iter().zip(self.get_units_shared(&ids)?) {
                let Some(u) = u else {
                    return Err(corruption(format!("a FIELD_INDEX entry of unit {id} has no UNIT_HEAD")));
                };
                out.push(u);
            }
            return Ok(out);
        }
        let (start, end) = (cache_key.0.clone(), &cache_key.1);
        // The entries first (unit and encoded value), then their units in one batch.
        let mut entries: Vec<(crate::types::UnitId, Vec<u8>)> = Vec::new();
        let mut cursor = start;
        let mut inclusive = true;
        while entries.len() < limit && &cursor < end {
            let from = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = self.snap.scan(from, Bound::Excluded(end.as_slice()), PAGE)?;
            let n = page.len();
            for (k, _) in &page {
                if entries.len() == limit {
                    break;
                }
                let (_, _, enc, unit) = entry_key_parts(k).map_err(|e| context(e, "FIELD_INDEX"))?;
                entries.push((unit, enc.to_vec()));
            }
            match page.into_iter().last() {
                Some((k, _)) if n == PAGE => {
                    cursor = k;
                    inclusive = false;
                }
                _ => break,
            }
        }
        let ids: Vec<crate::types::UnitId> = entries.iter().map(|(u, _)| *u).collect();
        let mut out = Vec::with_capacity(entries.len());
        for ((unit, enc), u) in entries.iter().zip(self.get_units_shared(&ids)?) {
            let Some(u) = u else {
                return Err(corruption(format!("a FIELD_INDEX entry of unit {unit} has no UNIT_HEAD")));
            };
            if !self.entry_agrees(&u, field, enc, unit_type) {
                return Err(corruption(format!("a FIELD_INDEX entry of unit {unit} disagrees with the unit")));
            }
            out.push(u);
        }
        let ids = out.iter().map(|u| u.id).collect();
        self.read_cache.put_range(seq, cache_key, ids);
        Ok(out)
    }

    /// The key range `[start, end)` of the entries of indexed `field` between
    /// `lo` and `hi`, checked as [`find_range`](Self::find_range) documents.
    pub(crate) fn field_bounds(
        &self,
        unit_type: &str,
        field: &str,
        lo: Bound<&FieldValue>,
        hi: Bound<&FieldValue>,
    ) -> Result<(Vec<u8>, Vec<u8>), UnitError> {
        let invalid = |m: String| UnitError::InvalidArgument(m);
        let not_indexed = |detail: String| UnitError::NotIndexed { detail };
        let Some(schema) = self.schema(unit_type)? else {
            return Err(not_indexed("the unit type has no schema".into()));
        };
        let Some(def) = schema.field(field).filter(|f| f.indexed) else {
            return Err(not_indexed(format!("field {field:?} is not an indexed field of the schema")));
        };
        let prefix = field_prefix(unit_type, field);
        // The bounds of one value are `[prefix ‖ enc, prefix ‖ enc ‖ 0xFF…)`.
        let edge = |b: Bound<&FieldValue>, is_lo: bool| -> Result<Bound<Vec<u8>>, UnitError> {
            let (Bound::Included(v) | Bound::Excluded(v)) = b else {
                return Ok(Bound::Unbounded);
            };
            if v.field_type() != def.ty {
                return Err(invalid(format!("a bound is a {:?}, field {field:?} is a {:?}", v.field_type(), def.ty)));
            }
            let Some(enc) = v.index_bytes() else {
                return Err(invalid("a bound is NaN".into()));
            };
            let mut at = prefix.clone();
            at.extend_from_slice(&enc);
            let (start, end) = prefix_bounds(&at);
            // Included lo and excluded hi start at the value; the other two skip past it.
            Ok(Bound::Included(if matches!(b, Bound::Included(_)) == is_lo { start } else { end }))
        };
        let (field_start, field_end) = prefix_bounds(&prefix);
        let start = match edge(lo, true)? {
            Bound::Included(k) => k,
            _ => field_start,
        };
        let end = match edge(hi, false)? {
            Bound::Included(k) => k,
            _ => field_end,
        };
        Ok((start, end))
    }

    /// Whether `unit` is an `Active` unit of `unit_type` whose `field` encodes
    /// to `enc`.
    fn entry_agrees(&self, unit: &Unit, field: &str, enc: &[u8], unit_type: &str) -> bool {
        unit.lifecycle == Lifecycle::Active
            && unit.unit_type == unit_type
            && unit.payload.as_deref().is_some_and(|p| Document::field_encodes_to(p, field, enc))
    }
}
