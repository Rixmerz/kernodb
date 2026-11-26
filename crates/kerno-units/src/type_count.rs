//! Live-unit counts per type (post-Stage-9 package P5, defect K2, layer version 5).
//!
//! The secondary index by type (`IDX_TYPE`) is read by a range scan, and the
//! `IDX_META` total only guards the primary index as a whole. An `IDX_TYPE`
//! entry that was lost left a typed query one unit short with no error, and a
//! lost `IDX_PRIMARY` entry went unseen once `IDX_META` was lowered to match.
//! Nothing but the entries said which units a type has.
//!
//! # Design
//!
//! One count record per `unit_type` (`docs/file-format/units-v5.md`), written
//! in the same transaction as every head change that moves a unit in or out of
//! the `Active` set:
//!
//! * key `0x73 ‖ len[1] ‖ unit_type` (`len` in 1..=64, as in an `IDX_TYPE`
//!   key: a unit type may hold a NUL, so it is length-prefixed, not
//!   terminated);
//! * value `envelope_version ‖ reserved_flags ‖ count: u64 BE`, the number of
//!   `Active` units of that type. Unlike an adjacency count, **zero is
//!   stored**: a type that once had units keeps its record, so a record that
//!   is absent means "never counted" (a database written before layer 5) and
//!   the writer never has to rebuild a count from the heads for a type it
//!   already knows.
//!
//! The count is its own record, not a field of `IDX_META`: `IDX_META` is
//! written only by an index build and its watermark defines freshness, while
//! the count is kept by the writers whether or not an index exists, so it is
//! an independent witness of what the index must hold.
//!
//! Readers compare it with what the index scan saw:
//!
//! * the `IDX_TYPE` path costs **one point read** more, and, only when the
//!   type has a record, scans the whole range of the type even when the
//!   filter fixes a schema version (the count is per type, not per version);
//! * the `IDX_PRIMARY` path costs **one range scan** more over the count
//!   records (one per type) and tallies the entries it already visits by
//!   type, which also sees a lowered `IDX_META` total (K2).
//!
//! A type with a record and a different number of entries, none included, is
//! `Corruption`; [`crate::UnitSnapshot::query`] turns that into a `Fallback`
//! to the reference path, which answers completely.
//!
//! # What it does not see
//!
//! * A type counted before layer 5 has no record until the first write that
//!   moves a unit of that type in or out of `Active`; the writer then counts
//!   the active heads once (a scan of every `UNIT_HEAD`) and records the
//!   total. Until then reads behave as before. An absent record is never a
//!   finding: it is indistinguishable from a legacy type.
//! * A record lost **together with** its entry, or rewritten to agree with a
//!   lost entry: two damages at once. `verify_units` still sees the missing
//!   entry (UNIT-37, UNIT-38) and a count that disagrees with the heads
//!   (UNIT-47).
//!
//! `ponytail:` the first count of a type scans every `UNIT_HEAD`, O(N) once
//! per type per database; upgrade path: a count of the units created per
//! type kept from the first write of a layer-5 database, if a workload
//! creates many distinct types in a large store.

use std::collections::BTreeMap;
use std::ops::Bound;

use kerno_core::contract::{Snapshot, Txn, TxnEngine};

use crate::adj_count::{decode_count, encode_value};
use crate::codec::{Key, MAX_UNIT_TYPE_LEN, TAG_UNIT_HEAD, UnitHead};
use crate::db::{ChangeSet, Write, context, corruption, scan_all};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::types::Lifecycle;

/// Key tag of a live-unit count record.
pub const TAG_TYPE_COUNT: u8 = 0x73;
/// `tag ‖ len ‖ unit_type (1..=64)`.
const KEY_MIN: usize = 1 + 1 + 1;
const KEY_MAX: usize = 1 + 1 + MAX_UNIT_TYPE_LEN as usize;
/// Heads read per scan call when a count is built from the heads.
const HEAD_PAGE: usize = 256;

/// The key of the count record of `unit_type`.
pub(crate) fn count_key(unit_type: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(2 + unit_type.len());
    k.extend_from_slice(&[TAG_TYPE_COUNT, unit_type.len() as u8]);
    k.extend_from_slice(unit_type.as_bytes());
    k
}

/// Scan bounds `[start, end)` of every count record.
fn all_counts() -> (Vec<u8>, Vec<u8>) {
    Key::tag_range(TAG_TYPE_COUNT)
}

/// The key and value of a count record, as the writer stores them (for the
/// vectors of `units-v5.md`).
#[doc(hidden)]
pub fn record(unit_type: &str, count: u64) -> (Vec<u8>, Vec<u8>) {
    (count_key(unit_type), encode_value(count))
}

/// Shape check of one `0x73` record, for the vectors of `units-v5.md`: the
/// count it holds.
#[doc(hidden)]
pub fn check_shape(key: &[u8], value: &[u8]) -> Result<u64, DecodeError> {
    decode_record(key, value).map(|(_, count)| count)
}

/// The unit type of a count key.
fn decode_key(key: &[u8]) -> Result<String, DecodeError> {
    let ok = (KEY_MIN..=KEY_MAX).contains(&key.len()) && key.len() == 2 + usize::from(key[1]);
    if !ok {
        return Err(DecodeError::new(
            UnitRule::Unit3,
            format!("key with tag 0x73 has length {} for its own length byte", key.len()),
        ));
    }
    match std::str::from_utf8(&key[2..]) {
        Ok(t) if !t.is_empty() => Ok(t.to_owned()),
        _ => Err(DecodeError::new(UnitRule::Unit12, "count key unit_type is empty or not UTF-8")),
    }
}

/// Shape check of one `0x73` record, as for `0x72` (`adj_count`): the value's
/// envelope is read before the key's length. Returns `(unit_type, count)`.
pub(crate) fn decode_record(key: &[u8], value: &[u8]) -> Result<(String, u64), DecodeError> {
    if let Some(e) =
        decode_count(value, 0).err().filter(|e| matches!(e.rule, UnitRule::Unit6 | UnitRule::Unit7 | UnitRule::Unit8))
    {
        return Err(e);
    }
    let unit_type = decode_key(key)?;
    Ok((unit_type, decode_count(value, 0)?))
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Reads the live count of `unit_type` as this change set sees it and
    /// returns the write that adds `delta` (`+1` or `-1`) to it, without
    /// writing. Call it **before** the head is written: a missing record is
    /// built by counting the `Active` heads that exist (a type counted before
    /// layer 5), and that count must not include the change being made.
    /// Reads only, so a failure leaves the change set untouched.
    pub(crate) fn plan_live(&self, unit_type: &str, delta: i64) -> Result<(Vec<u8>, u64), UnitError> {
        let k = count_key(unit_type);
        let txn = self.txn()?;
        // A count this change set moved already is kept in it until commit.
        if let Some(&n) = self.live_pending.get(&k) {
            let next = if delta >= 0 { n.saturating_add(delta as u64) } else { n.saturating_sub(delta.unsigned_abs()) };
            return Ok((k, next));
        }
        let base = match txn.get(&k)? {
            Some(raw) => {
                let (_, n) = decode_record(&k, &raw).map_err(|e| context(e, "TYPE_COUNT"))?;
                n
            }
            None => count_active_heads(txn, unit_type)?,
        };
        // ponytail: saturates like the adjacency counts; a count that was
        // already too low stays wrong and `verify_units` reports it (UNIT-47).
        let next =
            if delta >= 0 { base.saturating_add(delta as u64) } else { base.saturating_sub(delta.unsigned_abs()) };
        Ok((k, next))
    }

    /// Keeps a planned count for [`flush_live`](Self::flush_live): a change set of many units
    /// of one type writes its count record once, not once per unit.
    pub(crate) fn put_live(&mut self, (k, n): (Vec<u8>, u64)) -> Result<(), UnitError> {
        self.txn()?;
        self.live_pending.insert(k, n);
        Ok(())
    }

    /// Writes the counts the change set kept, in key order. Called by `commit`.
    pub(crate) fn flush_live(&mut self) -> Result<(), UnitError> {
        let mut counts: Vec<(Vec<u8>, u64)> = std::mem::take(&mut self.live_pending).into_iter().collect();
        counts.sort_unstable();
        for (k, n) in counts {
            self.write_owned(Write::Put(k, encode_value(n)))?;
        }
        Ok(())
    }
}

/// The `Active` units of `unit_type` in `txn`'s view, from the heads alone.
fn count_active_heads<T: Txn>(txn: &T, unit_type: &str) -> Result<u64, UnitError> {
    let (_, end) = Key::tag_range(TAG_UNIT_HEAD);
    let mut cursor = vec![TAG_UNIT_HEAD];
    let mut inclusive = true;
    let mut total = 0u64;
    loop {
        let lo = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
        let page = txn.scan(lo, Bound::Excluded(end.as_slice()), HEAD_PAGE)?;
        let n = page.len();
        for (k, v) in &page {
            let Key::UnitHead { unit } = Key::decode(k)? else {
                return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
            };
            let head = UnitHead::decode(v).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
            total += u64::from(head.lifecycle == Lifecycle::Active && head.unit_type == unit_type);
        }
        match page.into_iter().last() {
            Some((k, _)) if n == HEAD_PAGE => {
                cursor = k;
                inclusive = false;
            }
            _ => return Ok(total),
        }
    }
}

/// The count record of `unit_type` as `snap` sees it, if there is one (a
/// point read).
pub(crate) fn expected_one<S: Snapshot>(snap: &S, unit_type: &str) -> Result<Option<u64>, UnitError> {
    let k = count_key(unit_type);
    let Some(raw) = snap.get(&k)? else {
        return Ok(None);
    };
    let (_, n) = decode_record(&k, &raw).map_err(|e| context(e, "TYPE_COUNT"))?;
    Ok(Some(n))
}

/// Every count record, by unit type (one range scan).
pub(crate) fn expected_all<S: Snapshot>(snap: &S) -> Result<BTreeMap<String, u64>, UnitError> {
    let (start, end) = all_counts();
    let mut out = BTreeMap::new();
    scan_all(snap, start, end, |k, v| {
        let (t, n) = decode_record(k, v).map_err(|e| context(e, "TYPE_COUNT"))?;
        out.insert(t, n);
        Ok(())
    })?;
    Ok(out)
}

/// Compares what a scan saw (`seen`, entries per type) with the counts the
/// records give. A type with a record and a different number of entries,
/// none included, is `Corruption`; a type with entries and no record is not
/// (see the module documentation).
pub(crate) fn compare(expected: &BTreeMap<String, u64>, seen: &BTreeMap<String, u64>) -> Result<(), UnitError> {
    for (t, want) in expected {
        let got = seen.get(t).copied().unwrap_or(0);
        if got != *want {
            return Err(corruption(format!(
                "the index holds {got} entries of a unit type whose count record says {want} active units"
            )));
        }
    }
    Ok(())
}
