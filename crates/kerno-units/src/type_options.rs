//! Per-type options (layer version 7, ADR-0022, `docs/file-format/units-v7.md`).
//!
//! One record per unit type that has an option set:
//!
//! * key `0x77 ‖ type_len: u8 ‖ unit_type` (the shape of a `SCHEMA` key);
//! * value `envelope_version ‖ reserved_flags ‖ options: u8`, version 1.
//!
//! The one option is bit 0, `NO_FULLTEXT`: the payloads of the type are not
//! put in the keyword index ([`crate::fulltext`]). Every other bit is
//! reserved and must be 0. A type without a record has every option off.
//! An all-zero `options` byte is not written; turning an option back off
//! deletes the record.
//!
//! # Why the index stays exact
//!
//! An option changes only while the type has **no `Active` unit**, as the
//! change set sees it (its live count, [`crate::type_count`]). Only `Active`
//! units are in the keyword index, so at that moment the type has no index
//! entry, and from then on every write follows the option in force. The index
//! therefore holds exactly the `Active` units of the types without
//! `NO_FULLTEXT`, and `verify_units` (UNIT-49) expects exactly those.
//!
//! The first record raises the layer mark to 7 in its own transaction. A
//! build of layer 6 or older then refuses the database (UNIT-16), so no older
//! writer can index a type that opted out, or the other way round.

use kerno_core::contract::{Txn, TxnEngine};

use crate::db::{ChangeSet, Write};
use crate::error::{DecodeError, UnitError, UnitRule};

/// The tag of a type-options record.
pub const TAG_TYPE_OPTIONS: u8 = 0x77;
/// `envelope_version` of the value.
const RECORD_VERSION: u8 = 1;
/// Bit 0 of `options`: the type is not keyword-indexed.
pub const NO_FULLTEXT: u8 = 0b0000_0001;
const KNOWN_OPTIONS: u8 = NO_FULLTEXT;
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

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Whether the payloads of `unit_type` go in the keyword index, as this
    /// change set sees it.
    pub(crate) fn fulltext_enabled(&self, unit_type: &str) -> Result<bool, UnitError> {
        if self.layer_version < crate::codec::LAYER_VERSION_TYPE_OPTIONS && self.type_option_ops == 0 {
            // No layer-7 mark and no record written here: no type has options.
            return Ok(true);
        }
        let raw = self.txn()?.get(&options_key(unit_type))?;
        Ok(match raw {
            Some(v) => decode_value(&v).map_err(|e| crate::db::context(e, "TYPE_OPTIONS"))? & NO_FULLTEXT == 0,
            None => true,
        })
    }

    /// Turns the keyword index of `unit_type` off (`enabled = false`) or back
    /// on. Allowed only while the type has no `Active` unit, as this change
    /// set sees it, so the index never holds a unit it should not, nor misses
    /// one it should. Setting the state the type already has writes nothing.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an invalid type or a type with `Active` units.
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
        let (_, active) = self.plan_live(unit_type, 0)?;
        if active > 0 {
            return Err(UnitError::InvalidArgument(format!(
                "unit type {unit_type:?} has {active} active units; the keyword index of a type can change only \
                 while it has none"
            )));
        }
        let k = options_key(unit_type);
        if enabled {
            self.write(&Write::Delete(k))?;
        } else {
            self.write(&Write::Put(k, encode_value(NO_FULLTEXT)))?;
        }
        self.type_option_ops += 1;
        Ok(())
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
