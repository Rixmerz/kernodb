//! The self-check of schemas and their indexes, run by `verify_units`
//! (UNIT-50 and UNIT-51).
//!
//! The walk hands every schema record and index entry to [`Check::record`];
//! after it, [`Check::findings`] reads the head payload of every unit of a
//! typed unit type, rebuilds the entries the index must hold and compares them
//! with the ones it holds, entry by entry. A finding names the schema record,
//! unit ids and field positions, never a field value: an entry key embeds one.

use std::collections::{BTreeMap, BTreeSet};

use kerno_core::contract::Snapshot;

use super::{
    Document, Schema, TAG_FIELD_INDEX, TAG_SCHEMA, check_entry_value, decode_entry_key, decode_schema_key,
    entry_payload, field_prefix, schema_key,
};
use crate::codec::{Key, UnitHead};
use crate::error::{DecodeError, UnitError};
use crate::types::{Lifecycle, UnitId};
use crate::verify::CrossRule;

type Found = (CrossRule, Vec<u8>, String);
/// `(encoded value, unit)` of every entry of one field.
type Entries = BTreeSet<(Vec<u8>, UnitId)>;

#[derive(Default)]
pub(crate) struct Check {
    schemas: BTreeMap<String, Schema>,
    /// The entries of the store, by `(unit_type, field)`: `(encoded value, unit)`.
    entries: BTreeMap<(String, String), Entries>,
    /// The payload an entry carries (layer 13), by `(unit_type, field, unit)`.
    carried: BTreeMap<(String, String, UnitId), Vec<u8>>,
    /// A record did not decode: that is a structure finding already, and the
    /// comparison would only repeat it.
    damaged: bool,
}

impl Check {
    /// Whether a key with this first byte is a schema record.
    pub(crate) fn owns(tag: Option<&u8>) -> bool {
        matches!(tag, Some(&(TAG_SCHEMA | TAG_FIELD_INDEX)))
    }

    pub(crate) fn record(&mut self, k: &[u8], v: &[u8]) -> Result<(), DecodeError> {
        let r = self.keep(k, v);
        self.damaged |= r.is_err();
        r
    }

    fn keep(&mut self, k: &[u8], v: &[u8]) -> Result<(), DecodeError> {
        if k.first() == Some(&TAG_SCHEMA) {
            let unit_type = decode_schema_key(k)?;
            self.schemas.insert(unit_type, Schema::decode(v)?);
        } else {
            let (unit_type, field, enc, unit) = decode_entry_key(k)?;
            check_entry_value(v)?;
            if let Some(p) = entry_payload(v) {
                self.carried.insert((unit_type.clone(), field.clone(), unit), p.to_vec());
            }
            self.entries.entry((unit_type, field)).or_default().insert((enc, unit));
        }
        Ok(())
    }

    /// Every disagreement. `compare` is false when a head did not decode (its
    /// type is unknown, like UNIT-47 and UNIT-49). `mark` is the layer version of the mark.
    pub(crate) fn findings<S: Snapshot>(
        &self,
        snap: &S,
        heads: &BTreeMap<UnitId, UnitHead>,
        compare: bool,
        mark: Option<u32>,
    ) -> Result<Vec<Found>, UnitError> {
        let mut out = Vec::new();
        if self.damaged {
            return Ok(out);
        }
        let first_covering = self.schemas.iter().find(|(_, s)| s.has_covering()).map(|(t, _)| schema_key(t));
        if let (Some(key), Some(mark)) = (first_covering, mark)
            && mark < crate::codec::LAYER_VERSION_COVERING
        {
            out.push((CrossRule::Unit53, key, "a covering field in a database whose layer mark is below 13".into()));
        }
        if !compare {
            return Ok(out);
        }
        for (unit_type, schema) in &self.schemas {
            self.compare_type(snap, heads, unit_type, schema, &mut out)?;
        }
        // Entries of a field that has no index definition.
        for (unit_type, field) in self.entries.keys() {
            let indexed = self.schemas.get(unit_type).and_then(|s| s.field(field)).is_some_and(|f| f.indexed);
            if !indexed {
                out.push((
                    CrossRule::Unit50,
                    field_prefix(unit_type, field),
                    "index entries of a field that has no index definition".into(),
                ));
            }
        }
        Ok(out)
    }

    fn compare_type<S: Snapshot>(
        &self,
        snap: &S,
        heads: &BTreeMap<UnitId, UnitHead>,
        unit_type: &str,
        schema: &Schema,
        out: &mut Vec<Found>,
    ) -> Result<(), UnitError> {
        let key = schema_key(unit_type);
        let mut want: BTreeMap<usize, Entries> = BTreeMap::new();
        // The head payload of each active unit, kept only when a covering entry needs it.
        let mut payloads: BTreeMap<UnitId, Vec<u8>> = BTreeMap::new();
        for (unit, head) in heads.iter().filter(|(_, h)| h.unit_type == unit_type && h.lifecycle != Lifecycle::Deleted)
        {
            // A missing payload is a finding of its own (UNIT-26, UNIT-30).
            // The verifier keeps its heads without the payload they carry (`units-v11.md` §2):
            // without a REV_PAYLOAD, the head is read again for it.
            let payload = match &head.inline_payload {
                Some(p) => Some(p.clone()),
                None => match snap.get(&Key::RevPayload { unit: *unit, rev: head.head_rev }.encode()?)? {
                    Some(p) => Some(p),
                    None => snap
                        .get(&Key::UnitHead { unit: *unit }.encode()?)?
                        .and_then(|raw| UnitHead::decode(&raw).ok())
                        .filter(|h| h.head_rev == head.head_rev)
                        .and_then(|h| h.inline_payload),
                },
            };
            let Some(payload) = payload else {
                continue;
            };
            // The parse and validation messages name fields read from the payload, which
            // is content (IU-10): the finding says only that the unit does not fit.
            let unfit = || format!("unit {unit}: its payload does not satisfy the schema of its type");
            let record = Document::parse(&payload).and_then(|r| schema.validate(&r).map(|()| r)).map_err(|_| unfit());
            let entries = record.and_then(|r| schema.entries(&r).map_err(|_| unfit()));
            if head.lifecycle == Lifecycle::Active && schema.has_covering() {
                payloads.insert(*unit, payload.clone());
            }
            match entries {
                Err(detail) => out.push((CrossRule::Unit51, key.clone(), detail)),
                // An archived unit holds no entries.
                Ok(entries) if head.lifecycle == Lifecycle::Active => {
                    for (i, enc) in entries {
                        let held = want.entry(i).or_default();
                        // `held` is ordered by value, so an earlier unit with this value is the first
                        // entry at or after `(enc, lowest id)`.
                        let repeated = schema.fields()[i].unique
                            && held.range((enc.clone(), UnitId([0; 16]))..).next().is_some_and(|(e, _)| *e == enc);
                        if repeated {
                            out.push((
                                CrossRule::Unit51,
                                key.clone(),
                                format!("unit {unit}: another active unit holds its value of unique field #{i}"),
                            ));
                        }
                        held.insert((enc, *unit));
                    }
                }
                Ok(_) => {}
            }
        }
        let none = BTreeSet::new();
        for (i, field) in schema.fields().iter().enumerate().filter(|(_, f)| f.indexed) {
            let have = self.entries.get(&(unit_type.to_owned(), field.name.clone())).unwrap_or(&none);
            let expected = want.get(&i).unwrap_or(&none);
            for (_, unit) in expected.difference(have) {
                out.push((
                    CrossRule::Unit50,
                    key.clone(),
                    format!("unit {unit}: the entry of indexed field #{i} is missing"),
                ));
            }
            for (_, unit) in have.difference(expected) {
                out.push((
                    CrossRule::Unit50,
                    key.clone(),
                    format!("unit {unit}: an entry of indexed field #{i} does not match its payload or its lifecycle"),
                ));
            }
            // UNIT-53: what a matching entry carries is its unit's payload, exactly when the field
            // is covering and the payload is one the head stores inline.
            for (_, unit) in have.intersection(expected) {
                let carried = self.carried.get(&(unit_type.to_owned(), field.name.clone(), *unit));
                let detail = if !field.covering {
                    carried.map(|_| "carries a payload, but the field is not covering")
                } else {
                    let Some(p) = payloads.get(unit) else { continue };
                    let inline = p.len() <= crate::codec::INLINE_PAYLOAD_MAX as usize;
                    match carried {
                        Some(c) if !inline || c != p => Some("carries a payload that is not its unit's head payload"),
                        None if inline => Some("carries no payload, but its unit's payload is inline-sized"),
                        _ => None,
                    }
                };
                if let Some(d) = detail {
                    out.push((CrossRule::Unit53, key.clone(), format!("unit {unit}: the entry of covering field #{i} {d}")));
                }
            }
        }
        Ok(())
    }
}
