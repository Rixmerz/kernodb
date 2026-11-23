//! Per-type options (layer version 7, ADR-0022, `docs/file-format/units-v7.md`;
//! the second option is layer version 8, ADR-0025, `docs/file-format/units-v8.md`).
//!
//! One record per unit type that has an option set:
//!
//! * key `0x77 ‖ type_len: u8 ‖ unit_type` (the shape of a `SCHEMA` key);
//! * value `envelope_version ‖ reserved_flags ‖ options: u8`, version 1.
//!
//! Bit 0 is `FULLTEXT`: the payloads of the type are put in the keyword index
//! ([`crate::fulltext`]). Bit 1 is `SUBSTRING`: they are put in the trigram
//! index ([`crate::substring`]). Both indexes are **opt-in** (the format cut
//! of ADR-0023): a type without the bit is not indexed. Every other bit is
//! reserved and must be 0. A type without a record has every option off.
//! An all-zero `options` byte is not written; turning the last option back
//! off deletes the record.
//!
//! # Why the indexes stay exact
//!
//! Changing an option re-plans that index for every `Active` unit of the
//! type in the same change set (indexing them, or removing their entries),
//! and from then on every write follows the option in force. Only `Active`
//! units are ever indexed. Each index therefore holds exactly the `Active`
//! units of the types with its bit, and `verify_units` (UNIT-49, UNIT-52)
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
/// Bit 1 of `options`: the type is trigram-indexed for substring search (layer version 8).
pub const SUBSTRING: u8 = 0b0000_0010;
const KNOWN_OPTIONS: u8 = FULLTEXT | SUBSTRING;
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

/// An active unit and the key of its head payload.
pub(crate) type ActiveHead = (crate::types::UnitId, Vec<u8>);

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// The options byte of `unit_type` as this change set sees it (`0`: no
    /// record), read once per type and change set.
    pub(crate) fn type_options(&self, unit_type: &str) -> Result<u8, UnitError> {
        if let Some(options) = self.options_cache.borrow().get(unit_type) {
            return Ok(*options);
        }
        let options = match self.txn()?.get(&options_key(unit_type))? {
            Some(v) => decode_value(&v).map_err(|e| crate::db::context(e, "TYPE_OPTIONS"))?,
            None => 0,
        };
        self.options_cache.borrow_mut().insert(unit_type.to_owned(), options);
        Ok(options)
    }

    /// Whether the payloads of `unit_type` go in the keyword index, as this
    /// change set sees it.
    pub(crate) fn fulltext_enabled(&self, unit_type: &str) -> Result<bool, UnitError> {
        Ok(self.type_options(unit_type)? & FULLTEXT != 0)
    }

    /// Turns the option `bit` of `unit_type` on or off and returns the type's
    /// `Active` units, read before the record changed, for the caller to
    /// re-plan their index; `None` when the type already had that state, and
    /// nothing was written.
    pub(crate) fn set_option(
        &mut self,
        unit_type: &str,
        bit: u8,
        enabled: bool,
    ) -> Result<Option<Vec<ActiveHead>>, UnitError> {
        if unit_type.is_empty() || unit_type.len() > MAX_TYPE_LEN {
            return Err(UnitError::InvalidArgument(format!(
                "unit_type of {} bytes is outside 1..={MAX_TYPE_LEN}",
                unit_type.len()
            )));
        }
        let old = self.type_options(unit_type)?;
        let new = if enabled { old | bit } else { old & !bit };
        if new == old {
            return Ok(None);
        }
        let units = self.active_heads_of(unit_type)?;
        let k = options_key(unit_type);
        if new == 0 {
            self.write_owned(Write::Delete(k))?;
        } else {
            self.write_owned(Write::Put(k, encode_value(new)))?;
        }
        self.type_option_ops += 1;
        Ok(Some(units))
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
        let Some(units) = self.set_option(unit_type, FULLTEXT, enabled)? else {
            return Ok(());
        };
        // Index each active unit, or clear its entries.
        for (unit, payload_key) in units {
            let payload = if enabled { self.txn()?.get(&payload_key)? } else { None };
            let plan = self.plan_document(unit, payload.as_deref())?;
            for w in plan.writes {
                self.write_owned(w)?;
            }
            self.stage_fulltext(plan.change);
        }
        Ok(())
    }

    /// Every `Active` unit of `unit_type` with the key of its head payload, as
    /// this change set sees them (a scan of the heads).
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
                    out.push((unit, payload_key));
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
        Ok(self.type_options(unit_type)? & FULLTEXT != 0)
    }

    /// The options byte of `unit_type` at this snapshot (`0`: no record).
    pub(crate) fn type_options(&self, unit_type: &str) -> Result<u8, UnitError> {
        use kerno_core::contract::Snapshot;
        Ok(match self.snap.get(&options_key(unit_type))? {
            Some(v) => decode_value(&v).map_err(|e| crate::db::context(e, "TYPE_OPTIONS"))?,
            None => 0,
        })
    }
}
