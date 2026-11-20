//! The reader side of schemas: the schema of a type and lookups through the
//! entries of an indexed field.

use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot};

use super::{Document, FieldValue, Schema, decode_entry_key, field_prefix, prefix_bounds, schema_key};
use crate::db::{Unit, UnitSnapshot, context, corruption};
use crate::error::UnitError;
use crate::types::Lifecycle;

/// Entries read per scan call by a lookup.
const PAGE: usize = 256;

impl<E: Engine> UnitSnapshot<E> {
    /// The schema of `unit_type`, if it has one.
    ///
    /// # Errors
    ///
    /// `Storage(Corruption)` when the record does not decode.
    pub fn schema(&self, unit_type: &str) -> Result<Option<Schema>, UnitError> {
        match self.snap.get(&schema_key(unit_type))? {
            Some(raw) => Ok(Some(Schema::decode(&raw).map_err(|e| context(e, "SCHEMA"))?)),
            None => Ok(None),
        }
    }

    /// The `Active` units of `unit_type` whose indexed `field` equals `value`,
    /// in unit id order.
    ///
    /// # Errors
    ///
    /// As [`find_range`](Self::find_range).
    pub fn find_by_field(&self, unit_type: &str, field: &str, value: &FieldValue) -> Result<Vec<Unit>, UnitError> {
        self.find_range(unit_type, field, Bound::Included(value), Bound::Included(value), usize::MAX)
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
    /// or not indexed, or a bound is not of the field's type; `Storage(Corruption)`
    /// when an entry disagrees with its unit.
    pub fn find_range(
        &self,
        unit_type: &str,
        field: &str,
        lo: Bound<&FieldValue>,
        hi: Bound<&FieldValue>,
        limit: usize,
    ) -> Result<Vec<Unit>, UnitError> {
        let (start, end) = self.field_bounds(unit_type, field, lo, hi)?;
        let mut out = Vec::new();
        let mut cursor = start;
        let mut inclusive = true;
        while out.len() < limit && cursor < end {
            let from = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = self.snap.scan(from, Bound::Excluded(end.as_slice()), PAGE)?;
            let n = page.len();
            for (k, _) in &page {
                if out.len() == limit {
                    break;
                }
                let (_, _, enc, unit) = decode_entry_key(k).map_err(|e| context(e, "FIELD_INDEX"))?;
                let Some(u) = self.get_unit(unit)? else {
                    return Err(corruption(format!("a FIELD_INDEX entry of unit {unit} has no UNIT_HEAD")));
                };
                if !self.entry_agrees(&u, field, &enc, unit_type) {
                    return Err(corruption(format!("a FIELD_INDEX entry of unit {unit} disagrees with the unit")));
                }
                out.push(u);
            }
            match page.into_iter().last() {
                Some((k, _)) if n == PAGE => {
                    cursor = k;
                    inclusive = false;
                }
                _ => break,
            }
        }
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
        let Some(schema) = self.schema(unit_type)? else {
            return Err(invalid("the unit type has no schema".into()));
        };
        let Some(def) = schema.field(field).filter(|f| f.indexed) else {
            return Err(invalid(format!("field {field:?} is not an indexed field of the schema")));
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
            && unit
                .payload
                .as_deref()
                .and_then(|p| Document::parse(p).ok())
                .and_then(|r| r.get(field).and_then(FieldValue::index_bytes))
                .is_some_and(|e| e == enc)
    }
}
