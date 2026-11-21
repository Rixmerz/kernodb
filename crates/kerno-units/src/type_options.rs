//! Per-type options (layer version 7, ADR-0022, `docs/file-format/units-v7.md`).
//!
//! One record per unit type that has an option set:
//!
//! * key `0x77 ‖ type_len: u8 ‖ unit_type` (the shape of a `SCHEMA` key);
//! * value `envelope_version ‖ reserved_flags ‖ options: u8`, version 1.
//!
//! The one option is bit 0, `FULLTEXT`: the payloads of the type are put in
//! the keyword index ([`crate::fulltext`]). The index is **opt-in** (the
//! format cut of ADR-0023): a type without the bit is not indexed. Every
//! other bit is reserved and must be 0. A type without a record has every
//! option off.
//! An all-zero `options` byte is not written; turning an option back off
//! deletes the record.
//!
//! # Why the index stays exact
//!
//! Changing the option re-plans the keyword index of every `Active` unit of
//! the type in the same change set (indexing them, or removing their
//! entries), and from then on every write follows the option in force. Only
//! `Active` units are ever indexed. The index therefore holds exactly the
//! `Active` units of the types with `FULLTEXT`, and `verify_units` (UNIT-49)
//! expects exactly those.
//!
//! Every database this build opens is at layer 7 or above (ADR-0023), so no
//! reader with the old "index everything" rule can open one.

use kerno_core::contract::{Txn, TxnEngine};

use crate::db::{ChangeSet, Write};
use crate::error::{DecodeError, UnitError, UnitRule};

/// The tag of a type-options record.
pub const TAG_TYPE_OPTIONS: u8 = 0x77;
/// `envelope_version` of the value.
const RECORD_VERSION: u8 = 1;
/// Bit 0 of `options`: the type is keyword-indexed.
pub const FULLTEXT: u8 = 0b0000_0001;
const KNOWN_OPTIONS: u8 = FULLTEXT;
const MAX_TYPE_LEN: usize = 64;

/// The key of the options record of `unit_type`.
pub(crate) fn options_key(unit_type: &str) -> Vec<u8> {
    let mut k = vec![TAG_TYPE_OPTIONS, unit_type.len() as u8];
    k.extend_from_slice(unit_type.as_bytes());
    k
}

pub(crate) fn encode_value(options: u8) -> Vec<u8> {
    vec![RECORD_VERSION, 0, options]
}

/// The type named by an options key.
pub(crate) fn decode_key(key: &[u8]) -> Result<String, DecodeError> {
    let ok = key.len() >= 3 && key.len() == 2 + usize::from(key[1]) && usize::from(key[1]) <= MAX_TYPE_LEN;
    match key.get(2..).map(std::str::from_utf8) {
        Some(Ok(t)) if ok => Ok(t.to_owned()),
        _ if ok => Err(DecodeError::new(UnitRule::Unit12, "type options key unit_type is not UTF-8")),
        _ => Err(DecodeError::new(
            UnitRule::Unit3,
            format!("type options key of {} bytes is not tag, length, type", key.len()),
        )),
    }
}

/// The options byte of a record value: envelope first (a newer shape is a
/// newer layer, not damage), then length, then the known bits.
pub(crate) fn decode_value(v: &[u8]) -> Result<u8, DecodeError> {
    let (Some(&version), Some(&flags)) = (v.first(), v.get(1)) else {
        return Err(DecodeError::new(
            UnitRule::Unit9,
            format!("type options record has {} bytes, fewer than its envelope", v.len()),
        ));
    };
    match version {
        0 => return Err(DecodeError::new(UnitRule::Unit6, "type options envelope_version 0")),
        RECORD_VERSION => {}
        n => return Err(DecodeError::new(UnitRule::Unit7, format!("type options envelope_version {n}"))),
    }
    if flags != 0 {
        return Err(DecodeError::new(UnitRule::Unit8, format!("type options reserved_flags {flags:#04x}")));
    }
    if v.len() != 3 {
        return Err(DecodeError::new(UnitRule::Unit9, format!("type options value of {} bytes, not 3", v.len())));
    }
    let options = v[2];
    if options == 0 || options & !KNOWN_OPTIONS != 0 {
        return Err(DecodeError::new(UnitRule::Unit12, format!("type options byte {options:#04x}")));
    }
    Ok(options)
}

/// Shape check of one `0x77` record, for the vectors of `units-v7.md`: the
/// type and its options.
#[doc(hidden)]
pub fn check_shape(key: &[u8], value: &[u8]) -> Result<(String, u8), DecodeError> {
    Ok((decode_key(key)?, decode_value(value)?))
}

/// `(unit, head key, head bytes, head payload key)` of one active unit.
type ActiveHead = (crate::types::UnitId, Vec<u8>, Vec<u8>, Vec<u8>);

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Whether the payloads of `unit_type` go in the keyword index, as this
    /// change set sees it.
    pub(crate) fn fulltext_enabled(&self, unit_type: &str) -> Result<bool, UnitError> {
        let raw = self.txn()?.get(&options_key(unit_type))?;
        Ok(match raw {
            Some(v) => decode_value(&v).map_err(|e| crate::db::context(e, "TYPE_OPTIONS"))? & FULLTEXT != 0,
            None => false,
        })
    }

    /// Turns the keyword index of `unit_type` on (`enabled = true`) or back
    /// off (the default). The type's `Active` units follow in the same change
    /// set: turning it on indexes each of them, turning it off removes their
    /// entries, so the index never disagrees with the units (and a type can
    /// be switched after an import, like a schema). Setting the state the type
    /// already has writes nothing.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an invalid type; `ChangeSetTooLarge` when the
    /// type's units do not fit the engine's write limits (nothing is written).
    pub fn set_fulltext(&mut self, unit_type: &str, enabled: bool) -> Result<(), UnitError> {
        if unit_type.is_empty() || unit_type.len() > MAX_TYPE_LEN {
            return Err(UnitError::InvalidArgument(format!(
                "unit_type of {} bytes is outside 1..={MAX_TYPE_LEN}",
                unit_type.len()
            )));
        }
        if self.fulltext_enabled(unit_type)? == enabled {
            return Ok(());
        }
        let units = self.active_heads_of(unit_type)?;
        let k = options_key(unit_type);
        if enabled {
            self.write(&Write::Put(k, encode_value(FULLTEXT)))?;
        } else {
            self.write(&Write::Delete(k))?;
        }
        self.type_option_ops += 1;
        // Re-plan the index of each active unit under the new option: the
        // writer sees the option record written above.
        for (unit, head_key, head_raw, payload_key) in units {
            let payload = self.txn()?.get(&payload_key)?.unwrap_or_default();
            let writes = [Write::Put(head_key, head_raw), Write::Put(payload_key, payload)];
            for w in self.plan_fulltext(unit, &writes)? {
                self.write(&w)?;
            }
        }
        Ok(())
    }

    /// `(unit, head key, head bytes, head payload key)` of every `Active` unit
    /// of `unit_type`, as this change set sees them (a scan of the heads).
    fn active_heads_of(&self, unit_type: &str) -> Result<Vec<ActiveHead>, UnitError> {
        use crate::codec::{Key, TAG_UNIT_HEAD, UnitHead};
        use std::ops::Bound;
        const PAGE: usize = 512;
        let txn = self.txn()?;
        let (_, end) = Key::tag_range(TAG_UNIT_HEAD);
        let mut cursor = vec![TAG_UNIT_HEAD];
        let mut inclusive = true;
        let mut out = Vec::new();
        loop {
            let lo = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = txn.scan(lo, Bound::Excluded(end.as_slice()), PAGE)?;
            let n = page.len();
            for (k, v) in &page {
                let Key::UnitHead { unit } = Key::decode(k)? else {
                    return Err(UnitError::Storage(kerno_core::Error::Corruption(
                        "a key in the UNIT_HEAD range is not a UNIT_HEAD".into(),
                    )));
                };
                let head =
                    UnitHead::decode(v).map_err(|e| crate::db::context(e, &format!("UNIT_HEAD of unit {unit}")))?;
                if head.lifecycle == crate::types::Lifecycle::Active && head.unit_type == unit_type {
                    let payload_key = Key::RevPayload { unit, rev: head.head_rev }.encode()?;
                    out.push((unit, k.clone(), v.clone(), payload_key));
                }
            }
            match page.into_iter().last() {
                Some((k, _)) if n == PAGE => {
                    cursor = k;
                    inclusive = false;
                }
                _ => return Ok(out),
            }
        }
    }
}

impl<E: TxnEngine> crate::db::UnitDb<E> {
    /// [`ChangeSet::set_fulltext`] in its own change set.
    ///
    /// # Errors
    ///
    /// As [`ChangeSet::set_fulltext`], or the engine fails.
    pub fn set_fulltext(
        &self,
        unit_type: &str,
        enabled: bool,
        meta: crate::db::ChangeMeta,
    ) -> Result<crate::db::CommitInfo, UnitError> {
        let mut cs = self.begin_changeset(meta)?;
        cs.set_fulltext(unit_type, enabled)?;
        cs.commit()
    }
}

impl<E: kerno_core::contract::Engine> crate::db::UnitSnapshot<E> {
    /// Whether `unit_type` is in the keyword index at this snapshot (off by default).
    ///
    /// # Errors
    ///
    /// The engine fails or the options record is damaged.
    pub fn fulltext_enabled(&self, unit_type: &str) -> Result<bool, UnitError> {
        use kerno_core::contract::Snapshot;
        Ok(match self.snap.get(&options_key(unit_type))? {
            Some(v) => decode_value(&v).map_err(|e| crate::db::context(e, "TYPE_OPTIONS"))? & FULLTEXT != 0,
            None => false,
        })
    }
}
