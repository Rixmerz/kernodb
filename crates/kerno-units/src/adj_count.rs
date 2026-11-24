//! Adjacency counts (post-Stage-9 package P1, defect K1, layer version 4).
//!
//! The adjacency index (`REL_OUT`, `REL_IN`) is read by a range scan, so an
//! entry that was lost leaves nothing behind to show it: asked from the end
//! that lost it, `relations_of` used to return a short answer without an
//! error. The `REL` record cannot help, because nothing points from a unit to
//! its relations except the entries themselves.
//!
//! # Design
//!
//! One count record per `(unit, direction, relation_type)` that has at least
//! one entry, written in the same transaction as the entries it counts
//! (`docs/file-format/units-v4.md`):
//!
//! * key `0x72 ‖ unit_id[16] ‖ direction[1] ‖ relation_type ‖ 0x00` (`0` out,
//!   `1` in; the type is 1..=64 bytes, no NUL, as in an adjacency key);
//! * value `envelope_version ‖ reserved_flags ‖ count: u64 BE`, `count >= 1`.
//!
//! The reader compares what its scan saw with the record:
//!
//! * a read of one type costs **one point read** more (the record of that
//!   type);
//! * a read of every type costs **one range scan** more, over the count
//!   records of that unit and direction (one per distinct type, a few
//!   records and normally a single scan call);
//! * the entries themselves are the scan the read did already, tallied per
//!   type as they go by.
//!
//! A count that disagrees is `Corruption`, in either direction (fewer or
//! more entries than recorded). [`crate::UnitSnapshot::query_relations`]
//! turns that into a `Fallback` to the reference path, as for any damaged
//! adjacency answer, and the answer is then complete.
//!
//! The writer keeps the count in the change set and writes the record once, at
//! commit, whatever the number of entries the change set adds to the list.
//!
//! # What it does not see
//!
//! * A list with no count record has none to compare with, so a read of it
//!   behaves as if the count were not there: a record lost with the entries
//!   left behind is not a finding. The writer does not trust the absence
//!   either: it counts the entries of the blocks before the first change it
//!   makes to such a list and records the total, so the record is as good as
//!   the list was at that moment and a block is never written over without
//!   being read (a new list costs one empty seek).
//! * A count record lost **together with** its entries, or rewritten to
//!   agree with a lost entry: two damages at once. `verify_units` reports a
//!   count that disagrees with the entries (UNIT-45), but it cannot report
//!   one that was lost with them, as for any record that is simply absent.
//! * Damage inside the entry itself (its value) is caught by the existing
//!   per-entry check against the `REL` record.
//!
//! `ponytail:` one record per type rather than one per unit: a read of one
//! type must be checkable with a point read, and a per-unit total would force
//! a scan of every type of the unit to check one.

use std::collections::BTreeMap;
use std::ops::Bound;

use kerno_core::contract::{Snapshot, Txn, TxnEngine};

use crate::adjacency::{list_key, tag_of};
use crate::codec::{AdjBlock, ENVELOPE_VERSION, MAX_RELATION_TYPE_LEN, valid_relation_type};
use crate::db::{ChangeSet, Write, scan_all};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::types::{Direction, UnitId};

/// Key tag of an adjacency count record.
pub const TAG_REL_COUNT: u8 = 0x72;
/// The `envelope_version` of a count record: the layer's common envelope.
pub const REL_COUNT_RECORD_VERSION: u8 = ENVELOPE_VERSION;
/// Length of a count value: envelope and a `u64`.
const VALUE_LEN: usize = 10;
/// Blocks read per scan call when a missing record is rebuilt from the blocks.
const PAGE: usize = 64;
/// `tag ‖ unit ‖ direction ‖ type (1..=64) ‖ NUL`.
const KEY_MIN: usize = 1 + 16 + 1 + 1 + 1;
const KEY_MAX: usize = 1 + 16 + 1 + MAX_RELATION_TYPE_LEN as usize + 1;

pub(crate) fn dir_byte(dir: Direction) -> u8 {
    match dir {
        Direction::Out => 0,
        Direction::In => 1,
    }
}

pub(crate) fn dir_of(byte: u8) -> Direction {
    if byte == 0 { Direction::Out } else { Direction::In }
}

/// The key of the count record of `(unit, dir, relation_type)`.
pub(crate) fn count_key(unit: UnitId, dir: Direction, relation_type: &str) -> Vec<u8> {
    let mut k = vec![TAG_REL_COUNT];
    k.extend_from_slice(&unit.0);
    k.push(dir_byte(dir));
    k.extend_from_slice(relation_type.as_bytes());
    k.push(0);
    k
}

/// Scan bounds `[start, end)` of every count record of `unit` in `dir`.
fn count_range(unit: UnitId, dir: Direction) -> (Vec<u8>, Vec<u8>) {
    let mut start = vec![TAG_REL_COUNT];
    start.extend_from_slice(&unit.0);
    start.push(dir_byte(dir));
    let mut end = start.clone();
    // The direction byte is 0 or 1, so adding one never carries.
    if let Some(last) = end.last_mut() {
        *last += 1;
    }
    (start, end)
}

pub(crate) fn encode_value(count: u64) -> Vec<u8> {
    let mut v = vec![REL_COUNT_RECORD_VERSION, 0];
    v.extend_from_slice(&count.to_be_bytes());
    v
}

/// The count of an adjacency record value (at least 1).
fn decode_value(v: &[u8]) -> Result<u64, DecodeError> {
    decode_count(v, 1)
}

/// The count of a record value: envelope first (so a newer shape is a newer
/// layer, not damage), then length, then the range of the count (`min..`).
/// The live-unit counts of layer 5 share this shape and allow 0.
pub(crate) fn decode_count(v: &[u8], min: u64) -> Result<u64, DecodeError> {
    let (Some(&version), Some(&flags)) = (v.first(), v.get(1)) else {
        return Err(DecodeError::new(
            UnitRule::Unit9,
            format!("count record has {} bytes, fewer than its envelope", v.len()),
        ));
    };
    match version {
        0 => return Err(DecodeError::new(UnitRule::Unit6, "count record envelope_version 0")),
        REL_COUNT_RECORD_VERSION => {}
        n => return Err(DecodeError::new(UnitRule::Unit7, format!("count record envelope_version {n}"))),
    }
    if flags != 0 {
        return Err(DecodeError::new(UnitRule::Unit8, format!("count record reserved_flags {flags:#04x}")));
    }
    if v.len() < VALUE_LEN {
        return Err(DecodeError::new(
            UnitRule::Unit9,
            format!("count record has {} bytes after its envelope", v.len() - 2),
        ));
    }
    if v.len() > VALUE_LEN {
        return Err(DecodeError::new(
            UnitRule::Unit10,
            format!("count record has {} trailing bytes", v.len() - VALUE_LEN),
        ));
    }
    let count = u64::from_be_bytes(v[2..].try_into().unwrap_or([0; 8]));
    if count < min {
        return Err(DecodeError::new(
            UnitRule::Unit12,
            "count record with a count of 0 (the record is removed instead)",
        ));
    }
    Ok(count)
}

/// The fields of a count key.
fn decode_key(key: &[u8]) -> Result<(UnitId, Direction, String), DecodeError> {
    if !(KEY_MIN..=KEY_MAX).contains(&key.len()) {
        return Err(DecodeError::new(UnitRule::Unit3, format!("key with tag 0x72 has length {}", key.len())));
    }
    let n = key.len();
    if key[n - 1] != 0 {
        return Err(DecodeError::new(UnitRule::Unit3, "count key has no NUL after its relation type"));
    }
    let dir = match key[17] {
        0 => Direction::Out,
        1 => Direction::In,
        d => return Err(DecodeError::new(UnitRule::Unit12, format!("count key direction {d}"))),
    };
    let relation_type = match std::str::from_utf8(&key[18..n - 1]) {
        Ok(t) if valid_relation_type(t) => t.to_owned(),
        _ => {
            return Err(DecodeError::new(
                UnitRule::Unit12,
                "count key relation_type is empty, has a NUL or is not UTF-8",
            ));
        }
    };
    let mut unit = [0u8; 16];
    unit.copy_from_slice(&key[1..17]);
    Ok((UnitId(unit), dir, relation_type))
}

/// One decoded count record, for `verify_units`.
pub(crate) struct CountRecord {
    pub unit: UnitId,
    pub dir: Direction,
    pub relation_type: String,
    pub count: u64,
}

/// The key and value of a count record, as the writer stores them (for the
/// vectors of `units-v4.md`).
#[doc(hidden)]
pub fn record(unit: UnitId, dir: Direction, relation_type: &str, count: u64) -> (Vec<u8>, Vec<u8>) {
    (count_key(unit, dir, relation_type), encode_value(count))
}

/// Shape check of one `0x72` record, for the vectors of `units-v4.md`: the
/// count it holds.
#[doc(hidden)]
pub fn check_shape(key: &[u8], value: &[u8]) -> Result<u64, DecodeError> {
    decode_record(key, value).map(|c| c.count)
}

/// Shape check of one `0x72` record. The value's envelope is read before the
/// key's length, as for the public id records (units-v3 §4).
pub(crate) fn decode_record(key: &[u8], value: &[u8]) -> Result<CountRecord, DecodeError> {
    if let Some(e) =
        decode_value(value).err().filter(|e| matches!(e.rule, UnitRule::Unit6 | UnitRule::Unit7 | UnitRule::Unit8))
    {
        return Err(e);
    }
    let (unit, dir, relation_type) = decode_key(key)?;
    let count = decode_value(value)?;
    Ok(CountRecord { unit, dir, relation_type, count })
}

/// The write one count change comes to, computed by reads alone.
pub(crate) enum CountStep {
    Put(Vec<u8>, u64),
    Delete(Vec<u8>),
    Nothing,
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Reads the count of the adjacency entries of `(unit, dir,
    /// relation_type)` as this change set sees them and returns the write
    /// that adds `delta` (`+1` or `-1`) to it, without writing. Call it
    /// **before** the entry is written or removed: a missing record is first
    /// built by counting the entries that exist (a new list, or a record that
    /// was lost), and that count must not include the entry being changed. A
    /// count that reaches zero removes its record. The second value is the
    /// count before the change.
    fn plan_count(
        &self,
        unit: UnitId,
        dir: Direction,
        relation_type: &str,
        delta: i64,
    ) -> Result<(CountStep, u64), UnitError> {
        let k = count_key(unit, dir, relation_type);
        let (existing, base) = if let Some(&cached) = self.count_cache.get(&k) {
            (cached > 0, cached)
        } else {
            match self.txn()?.get(&k)? {
                Some(raw) => {
                    let n =
                        decode_value(&raw).map_err(|e| crate::db::context(e, &format!("REL_COUNT of unit {unit}")))?;
                    (true, n)
                }
                // No record: the list is new, or its record was lost. Only the blocks tell which,
                // and the writer must not take a list with entries for an empty one (it would
                // overwrite its tail block), so it counts them, as it always did (one seek for
                // a new list) and records the total with this change.
                None => (false, count_entries(self.txn()?, unit, dir, relation_type)?),
            }
        };
        let next =
            if delta >= 0 { base.saturating_add(delta as u64) } else { base.saturating_sub(delta.unsigned_abs()) };
        let step = if next > 0 {
            CountStep::Put(k, next)
        } else if existing {
            CountStep::Delete(k)
        } else {
            CountStep::Nothing
        };
        Ok((step, base))
    }

    fn apply_count(&mut self, step: CountStep) -> Result<(), UnitError> {
        match step {
            // The record is written once, at commit (`flush_counts`): a change set
            // that adds many relations of one unit writes its count once, not once
            // per relation (ADR-0023, step 3.2). Within the change set the cache is
            // the count, and only `plan_count` reads it.
            CountStep::Put(k, n) => {
                self.count_cache.insert(k.clone(), n);
                self.count_dirty.insert(k.clone());
            }
            CountStep::Delete(k) => {
                self.count_cache.insert(k.clone(), 0);
                self.count_dirty.insert(k.clone());
            }
            CountStep::Nothing => {}
        }
        Ok(())
    }

    /// Writes every count this change set changed, once each: a count of 0 deletes
    /// the record. Called by `commit` before anything else is decided.
    pub(crate) fn flush_counts(&mut self) -> Result<(), UnitError> {
        for k in std::mem::take(&mut self.count_dirty) {
            match self.count_cache.get(&k).copied().unwrap_or(0) {
                0 => self.write(&Write::Delete(k))?,
                n => self.write(&Write::Put(k, encode_value(n)))?,
            }
        }
        Ok(())
    }

    /// Adds `delta` to the count of `(from, Out)` and of `(to, In)` and returns
    /// the two counts as they were before. Both next counts are read before
    /// either is written, so a read that fails (a damaged record, an engine
    /// error) leaves the change set untouched.
    pub(crate) fn adjust_counts(
        &mut self,
        from: UnitId,
        to: UnitId,
        relation_type: &str,
        delta: i64,
    ) -> Result<(u64, u64), UnitError> {
        let (out, before_out) = self.plan_count(from, Direction::Out, relation_type, delta)?;
        let (inn, before_in) = self.plan_count(to, Direction::In, relation_type, delta)?;
        self.apply_count(out)?;
        self.apply_count(inn)?;
        Ok((before_out, before_in))
    }
}

/// The adjacency entries of `(unit, dir, relation_type)` in `txn`'s view: the entries of the
/// blocks of that list. A block that does not decode is an error, as it is for the write that
/// would have to read it.
fn count_entries<T: Txn>(txn: &T, unit: UnitId, dir: Direction, relation_type: &str) -> Result<u64, UnitError> {
    if !valid_relation_type(relation_type) {
        return Ok(0);
    }
    let (start, end) = crate::schema::prefix_bounds(&list_key(tag_of(dir), unit, relation_type));
    let mut cursor = start;
    let mut inclusive = true;
    let mut total = 0u64;
    loop {
        let lo = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
        let page = txn.scan(lo, Bound::Excluded(&end), PAGE)?;
        for (_, v) in &page {
            total += AdjBlock::decode(v).map_err(|e| crate::db::context(e, "REL_ADJ"))?.entries.len() as u64;
        }
        match page.last() {
            Some((k, _)) if page.len() == PAGE => {
                cursor = k.clone();
                inclusive = false;
            }
            _ => return Ok(total),
        }
    }
}

/// The counts a read of `(unit, dir)` must find, by relation type: the one
/// record of `relation_type` (a point read), or every record of the unit and
/// direction (one range scan).
pub(crate) fn expected<S: Snapshot>(
    snap: &S,
    unit: UnitId,
    dir: Direction,
    relation_type: Option<&str>,
) -> Result<BTreeMap<String, u64>, UnitError> {
    let mut out = BTreeMap::new();
    if let Some(t) = relation_type {
        if valid_relation_type(t)
            && let Some(raw) = snap.get(&count_key(unit, dir, t))?
        {
            let n = decode_value(&raw).map_err(|e| crate::db::context(e, &format!("REL_COUNT of unit {unit}")))?;
            out.insert(t.to_owned(), n);
        }
        return Ok(out);
    }
    let (start, end) = count_range(unit, dir);
    scan_all(snap, start, end, |k, v| {
        let (_, _, t) = decode_key(k).map_err(|e| crate::db::context(e, &format!("REL_COUNT of unit {unit}")))?;
        let n = decode_value(v).map_err(|e| crate::db::context(e, &format!("REL_COUNT of unit {unit}")))?;
        out.insert(t, n);
        Ok(())
    })?;
    Ok(out)
}

/// Compares what a scan saw (`seen`, entries per type) with the counts the
/// records give. A type with a record and a different number of entries,
/// none included, is `Corruption`; a type with entries and no record is not
/// (see the module documentation).
pub(crate) fn compare(
    unit: UnitId,
    expected: &BTreeMap<String, u64>,
    seen: &BTreeMap<String, u64>,
) -> Result<(), UnitError> {
    for (t, want) in expected {
        let got = seen.get(t).copied().unwrap_or(0);
        if got != *want {
            return Err(crate::db::corruption(format!(
                "adjacency of unit {unit} has {got} entries of a relation type whose count record says {want}"
            )));
        }
    }
    Ok(())
}
