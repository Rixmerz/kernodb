//! Substring search: a trigram index of the payloads (ADR-0025, layer
//! version 8, `docs/file-format/units-v8.md`).
//!
//! # What is indexed
//!
//! The head payload of every `Active` unit of a unit type that has the
//! `SUBSTRING` option ([`crate::type_options`]), whatever its bytes: text or
//! not. Only the first [`MAX_INDEXED_BYTES`] bytes are indexed and searched.
//! ASCII letters are folded to lower case, in the index and in the needle, so
//! a search matches them in either case; every other byte must be equal (the
//! rule of SQL's `LIKE`).
//!
//! # Design
//!
//! One record per distinct trigram of a unit, written by the same engine
//! transaction that writes the unit's head, like the keyword index:
//!
//! * `TRIGRAM` `0x7a ‖ type_len: u8 ‖ unit_type ‖ gram[3] ‖ unit[16]` →
//!   `envelope[2]`.
//!
//! The grams of a payload are the three bytes that start at each of its
//! positions, the payload followed by two `0x00`. So every position has a
//! gram, and a needle of one or two bytes is the prefix of a gram of every
//! unit that holds it: a short needle is a range scan too, not a scan of the
//! payloads.
//!
//! The type comes first in the key, so a search of one type reads only that
//! type's entries, and turning the option off is one range deletion.
//!
//! A write plans the entries by reads only: the grams of the payload the head
//! had (read from the transaction) against those of the new one. Only the
//! difference is written, so an update that changes a few bytes writes a few
//! entries.
//!
//! # A search
//!
//! The units that hold every trigram of the needle are found by intersecting
//! its trigram lists, the shortest first; a list much longer than the
//! candidates left is probed by point reads instead of being read. A needle
//! of up to three bytes is answered by the index alone. A longer one is then
//! checked against the payload of each candidate, since a unit can hold every
//! trigram of a needle without holding the needle.
//!
//! `ponytail:` one engine record per trigram and unit. A search reads the
//! whole list of at least one trigram of the needle, so it is linear in the
//! units that share the needle's rarest trigram, and a write costs one record
//! per distinct trigram of the payload. Upgrade path: posting blocks (many
//! units per record) once the engine has a reverse seek to find a block.

mod check;

#[cfg(test)]
mod tests;

use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::codec::Key;
use crate::db::{ChangeSet, UnitDb, UnitSnapshot, Write, context, corruption, key, scan_all};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::fulltext::{Next, next_of};
use crate::schema::prefix_bounds;
use crate::type_options::{SUBSTRING, TAG_TYPE_OPTIONS};
use crate::types::{Lifecycle, UnitId};

pub(crate) use check::Check;

/// Key tag of a trigram entry.
pub const TAG_TRIGRAM: u8 = 0x7a;

/// How much of a payload is indexed and searched: its first bytes, as in the
/// keyword index ([`crate::fulltext::MAX_INDEXED_BYTES`]). A unit's entries
/// are written in one engine transaction, which has a batch limit.
pub const MAX_INDEXED_BYTES: usize = crate::fulltext::MAX_INDEXED_BYTES;

/// Longest needle of a search, in bytes.
pub const MAX_NEEDLE_LEN: usize = 1024;

const RECORD_VERSION: u8 = 1;
const MAX_TYPE_LEN: usize = 64;

/// Entries read per engine scan call.
const PAGE: usize = 256;

/// With at most this many candidates left, a trigram list that did not fit
/// one page is probed by point reads instead of being read whole.
const PROBE_MAX: usize = 64;

type Gram = [u8; 3];

// ---------------------------------------------------------------- grams

fn indexed(payload: &[u8]) -> &[u8] {
    &payload[..payload.len().min(MAX_INDEXED_BYTES)]
}

/// The distinct trigrams of the indexed part of `payload`, sorted: the three
/// folded bytes at each position, the payload followed by two `0x00`.
pub(crate) fn grams(payload: &[u8]) -> Vec<Gram> {
    let text = indexed(payload);
    let at = |i: usize| text.get(i).map_or(0, u8::to_ascii_lowercase);
    let mut out: Vec<Gram> = (0..text.len()).map(|i| [at(i), at(i + 1), at(i + 2)]).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Whether the indexed part of `payload` holds `needle` (already folded).
fn holds(payload: &[u8], needle: &[u8]) -> bool {
    indexed(payload).windows(needle.len()).any(|w| w.iter().zip(needle).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

// ----------------------------------------------------------------- keys

/// `0x7a ‖ type_len ‖ unit_type`: the prefix of every entry of a type.
pub(crate) fn type_prefix(unit_type: &str) -> Vec<u8> {
    let mut k = vec![TAG_TRIGRAM, unit_type.len() as u8];
    k.extend_from_slice(unit_type.as_bytes());
    k
}

fn entry_key(prefix: &[u8], gram: Gram, unit: UnitId) -> Vec<u8> {
    let mut k = Vec::with_capacity(prefix.len() + 3 + 16);
    k.extend_from_slice(prefix);
    k.extend_from_slice(&gram);
    k.extend_from_slice(&unit.0);
    k
}

fn entry_value() -> Vec<u8> {
    vec![RECORD_VERSION, 0]
}

/// The unit of an entry key that is known to be of the right length.
fn unit_of(k: &[u8]) -> Option<UnitId> {
    let tail = k.len().checked_sub(16)?;
    <[u8; 16]>::try_from(&k[tail..]).ok().map(UnitId)
}

/// The type, the trigram and the unit of an entry key.
pub(crate) fn decode_key(k: &[u8]) -> Result<(String, Gram, UnitId), DecodeError> {
    let shape = || DecodeError::new(UnitRule::Unit3, format!("trigram key of {} bytes has no valid shape", k.len()));
    let tlen = usize::from(*k.get(1).ok_or_else(shape)?);
    if k[0] != TAG_TRIGRAM || tlen == 0 || tlen > MAX_TYPE_LEN || k.len() != 2 + tlen + 3 + 16 {
        return Err(shape());
    }
    let unit_type = std::str::from_utf8(&k[2..2 + tlen])
        .map_err(|_| DecodeError::new(UnitRule::Unit12, "trigram key unit_type is not UTF-8"))?;
    let at = 2 + tlen;
    let unit = unit_of(k).ok_or_else(shape)?;
    Ok((unit_type.to_owned(), [k[at], k[at + 1], k[at + 2]], unit))
}

/// The value of an entry: the envelope alone.
pub(crate) fn check_value(v: &[u8]) -> Result<(), DecodeError> {
    let e = |rule, m: &str| DecodeError::new(rule, format!("trigram record: {m}"));
    match v {
        [] | [_] => Err(e(UnitRule::Unit9, "fewer bytes than its envelope")),
        [0, ..] => Err(e(UnitRule::Unit6, "envelope_version 0")),
        [RECORD_VERSION, 0] => Ok(()),
        [RECORD_VERSION, _] => Err(e(UnitRule::Unit8, "reserved_flags set")),
        [RECORD_VERSION, _, ..] => Err(e(UnitRule::Unit10, "trailing bytes")),
        [_, ..] => Err(e(UnitRule::Unit7, "a newer envelope_version")),
    }
}

// --------------------------------------------------------------- writer

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Whether the payloads of `unit_type` go in the trigram index, as this
    /// change set sees it.
    pub(crate) fn substring_enabled(&self, unit_type: &str) -> Result<bool, UnitError> {
        Ok(self.type_options(unit_type)? & SUBSTRING != 0)
    }

    /// The trigrams the index holds of `unit` before the operation being
    /// planned: those of its head payload when it is `Active`, else none.
    fn indexed_grams(&self, unit: UnitId) -> Result<Vec<Gram>, UnitError> {
        let txn = self.txn()?;
        let Some(head) = crate::history::read_head(&|k| txn.get(k), unit)? else {
            return Ok(Vec::new());
        };
        if head.lifecycle != Lifecycle::Active {
            return Ok(Vec::new());
        }
        Ok(txn.get(&key(Key::RevPayload { unit, rev: head.head_rev })?)?.map(|p| grams(&p)).unwrap_or_default())
    }

    /// The index writes that follow the operation `writes` on `unit`, planned
    /// by reads only, before the first write of the operation (like
    /// [`ChangeSet::plan_fulltext`](crate::fulltext)): the entries the unit no
    /// longer has are deleted and the new ones written. Nothing for a type
    /// without the option, and nothing for an operation that writes no head.
    pub(crate) fn plan_substring(&self, unit: UnitId, writes: &[Write]) -> Result<Vec<Write>, UnitError> {
        let Next::Content(unit_type, payload) = next_of(unit, writes)? else {
            return Ok(Vec::new());
        };
        if !self.substring_enabled(&unit_type)? {
            return Ok(Vec::new());
        }
        let (old, new) = (self.indexed_grams(unit)?, payload.map(grams).unwrap_or_default());
        let prefix = type_prefix(&unit_type);
        // Both lists are sorted, so a binary search tells what the other lacks.
        let gone = old.iter().filter(|g| new.binary_search(g).is_err());
        let added = new.iter().filter(|g| old.binary_search(g).is_err());
        Ok(gone
            .map(|g| Write::Delete(entry_key(&prefix, *g, unit)))
            .chain(added.map(|g| Write::Put(entry_key(&prefix, *g, unit), entry_value())))
            .collect())
    }

    /// Turns the trigram index of `unit_type` on (`enabled = true`) or back
    /// off (the default). The type's `Active` units follow in the same change
    /// set: turning it on indexes each of them, turning it off deletes the
    /// type's entries, so the index never disagrees with the units. Setting
    /// the state the type already has writes nothing.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an invalid type; `ChangeSetTooLarge` when the
    /// type's units do not fit the engine's write limits (nothing is written).
    pub fn set_substring(&mut self, unit_type: &str, enabled: bool) -> Result<(), UnitError> {
        self.guarded(|cs| {
            let Some(units) = cs.set_option(unit_type, SUBSTRING, enabled)? else {
                return Ok(());
            };
            cs.substring_ops += 1;
            let prefix = type_prefix(unit_type);
            if !enabled {
                let (start, end) = prefix_bounds(&prefix);
                return cs.write_owned(Write::DeleteRange(start, end));
            }
            for (unit, payload_key) in units {
                let Some(payload) = cs.txn()?.get(&payload_key)? else { continue };
                for gram in grams(&payload) {
                    cs.write_owned(Write::Put(entry_key(&prefix, gram, unit), entry_value()))?;
                }
            }
            Ok(())
        })
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// [`ChangeSet::set_substring`] in its own change set.
    ///
    /// # Errors
    ///
    /// As [`ChangeSet::set_substring`], or the engine fails.
    pub fn set_substring(
        &self,
        unit_type: &str,
        enabled: bool,
        meta: crate::db::ChangeMeta,
    ) -> Result<crate::db::CommitInfo, UnitError> {
        let mut cs = self.begin_changeset(meta)?;
        cs.set_substring(unit_type, enabled)?;
        cs.commit()
    }

    /// [`UnitSnapshot::search_substring`] on a snapshot of the latest commit.
    ///
    /// # Errors
    ///
    /// As [`UnitSnapshot::search_substring`].
    pub fn search_substring(
        &self,
        needle: &[u8],
        limit: Option<usize>,
        types: Option<&[&str]>,
    ) -> Result<Vec<UnitId>, UnitError> {
        self.snapshot()?.search_substring(needle, limit, types)
    }
}

// ---------------------------------------------------------------- query

/// One trigram list of a search: what was read of it so far.
struct List {
    /// The range of the list's entries, `[start, end)`.
    start: Vec<u8>,
    end: Vec<u8>,
    /// The units read, in id order.
    units: Vec<UnitId>,
    /// The key the next page starts after; `None` when the list is read whole.
    more: Option<Vec<u8>>,
}

impl<E: Engine> UnitSnapshot<E> {
    /// Whether `unit_type` is in the trigram index at this snapshot (off by default).
    ///
    /// # Errors
    ///
    /// The engine fails or the options record is damaged.
    pub fn substring_enabled(&self, unit_type: &str) -> Result<bool, UnitError> {
        Ok(self.type_options(unit_type)? & SUBSTRING != 0)
    }

    /// The unit types that have the trigram index on, in name order.
    fn substring_types(&self) -> Result<Vec<String>, UnitError> {
        let mut out = Vec::new();
        scan_all(&self.snap, vec![TAG_TYPE_OPTIONS], vec![TAG_TYPE_OPTIONS + 1], |k, v| {
            let (unit_type, options) =
                crate::type_options::check_shape(k, v).map_err(|e| context(e, "TYPE_OPTIONS"))?;
            if options & SUBSTRING != 0 {
                out.push(unit_type);
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// The units whose payload holds `needle`, in unit id order, at most `limit` (`None`: all of
    /// them), optionally only those of the unit types in `types` (`None` searches every type
    /// with the trigram index on). ASCII letters match in either case; every other byte must be
    /// equal. Only the first [`MAX_INDEXED_BYTES`] bytes of a payload are searched.
    ///
    /// A search that could never match is an error, not an empty answer, like
    /// [`search_text`](Self::search_text).
    ///
    /// # Errors
    ///
    /// * `InvalidArgument` for `limit == Some(0)`, an empty needle or one over
    ///   [`MAX_NEEDLE_LEN`] bytes, `Some(&[])` as `types`, or an invalid type name.
    /// * `NotIndexed` when no type has the trigram index on (`set_substring`), or when a type in
    ///   `types` has not.
    /// * `UnknownType` for a type in `types` that no unit, schema or option ever named.
    /// * `Storage(Corruption)` when an entry of the index does not decode or names a unit that
    ///   does not exist.
    pub fn search_substring(
        &self,
        needle: &[u8],
        limit: Option<usize>,
        types: Option<&[&str]>,
    ) -> Result<Vec<UnitId>, UnitError> {
        let invalid = |m: &str| UnitError::InvalidArgument(format!("search_substring: {m}"));
        if limit == Some(0) {
            return Err(invalid("limit must be at least 1"));
        }
        if needle.is_empty() {
            return Err(invalid("the needle is empty"));
        }
        if needle.len() > MAX_NEEDLE_LEN {
            return Err(invalid("the needle is longer than 1024 bytes"));
        }
        if types.is_some_and(<[&str]>::is_empty) {
            return Err(invalid("types is empty: give at least one unit type, or no restriction"));
        }
        for t in types.unwrap_or_default() {
            if t.is_empty() || t.len() > MAX_TYPE_LEN {
                return Err(invalid("a unit type name is empty or longer than 64 bytes"));
            }
        }
        let indexed = self.substring_types()?;
        if indexed.is_empty() {
            return Err(UnitError::NotIndexed {
                detail: "no unit type is substring-indexed: turn it on for a type with set_substring".into(),
            });
        }
        for t in types.unwrap_or_default() {
            if !self.type_known(t)? {
                return Err(UnitError::UnknownType { unit_type: (*t).to_string() });
            }
            if !indexed.iter().any(|i| i == t) {
                return Err(UnitError::NotIndexed {
                    detail: format!("unit type {t:?} is not substring-indexed: turn it on with set_substring"),
                });
            }
        }
        let needle = needle.to_ascii_lowercase();
        let limit = limit.unwrap_or(usize::MAX);
        let mut out = Vec::new();
        for unit_type in indexed.iter().filter(|i| types.is_none_or(|ts| ts.contains(&i.as_str()))) {
            out.extend(self.search_type(unit_type, &needle, limit)?);
        }
        // A unit has one type, so the per-type answers share no unit.
        out.sort_unstable();
        out.truncate(limit);
        Ok(out)
    }

    /// The first `limit` units of `unit_type` that hold `needle` (folded), in id order.
    fn search_type(&self, unit_type: &str, needle: &[u8], limit: usize) -> Result<Vec<UnitId>, UnitError> {
        let prefix = type_prefix(unit_type);
        // A real byte is never the `0x00` that pads the last grams of a payload, unless the
        // needle itself has one: then the index may answer a unit that only has the padding.
        let padded = needle.contains(&0);
        let (mut candidates, exact) = if needle.len() < 3 {
            // Every gram that starts with the needle: each position of a payload has one.
            let mut from = prefix;
            from.extend_from_slice(needle);
            let mut units = self.read_list(&mut self.list_of(from)?)?;
            units.sort_unstable();
            units.dedup();
            (units, !padded)
        } else {
            let mut wanted: Vec<Gram> = needle.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
            wanted.sort_unstable();
            wanted.dedup();
            (self.intersect(&prefix, &wanted)?, needle.len() == 3 && !padded)
        };
        if exact {
            candidates.truncate(limit);
            return Ok(candidates);
        }
        // Holding every trigram of the needle is not holding the needle: read the payload.
        let mut out = Vec::new();
        for unit in candidates {
            let get = |k: &[u8]| self.snap.get(k);
            let Some(head) = crate::history::read_head(&get, unit)? else {
                return Err(corruption(format!("a TRIGRAM entry of unit {unit} has no UNIT_HEAD")));
            };
            let payload = self.snap.get(&key(Key::RevPayload { unit, rev: head.head_rev })?)?;
            if payload.is_some_and(|p| holds(&p, needle)) {
                out.push(unit);
                if out.len() == limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// The list of the entries whose key starts with `prefix`, with its first page read.
    fn list_of(&self, prefix: Vec<u8>) -> Result<List, UnitError> {
        let (start, end) = prefix_bounds(&prefix);
        let mut list = List { start, end, units: Vec::new(), more: None };
        let page = self.snap.scan(Bound::Included(&list.start), Bound::Excluded(&list.end), PAGE)?;
        self.keep_page(&mut list, page)?;
        Ok(list)
    }

    fn keep_page(&self, list: &mut List, page: Vec<(Vec<u8>, Vec<u8>)>) -> Result<(), UnitError> {
        for (k, _) in &page {
            // The type and the gram are the scanned prefix: only the length is left to check.
            let unit = if k.len() >= list.start.len() + 16 { unit_of(k) } else { None };
            list.units.push(unit.ok_or_else(|| corruption("a TRIGRAM key has no unit".into()))?);
        }
        list.more = if page.len() == PAGE { page.into_iter().last().map(|(k, _)| k) } else { None };
        Ok(())
    }

    /// Reads the rest of `list` and returns its units, in key order.
    fn read_list(&self, list: &mut List) -> Result<Vec<UnitId>, UnitError> {
        while let Some(after) = list.more.take() {
            let page = self.snap.scan(Bound::Excluded(&after), Bound::Excluded(&list.end), PAGE)?;
            self.keep_page(list, page)?;
        }
        Ok(std::mem::take(&mut list.units))
    }

    /// The units of the type with `prefix` that have an entry for every gram of `wanted`, in
    /// id order.
    fn intersect(&self, prefix: &[u8], wanted: &[Gram]) -> Result<Vec<UnitId>, UnitError> {
        let mut lists = Vec::with_capacity(wanted.len());
        for gram in wanted {
            let mut from = prefix.to_vec();
            from.extend_from_slice(gram);
            let list = self.list_of(from)?;
            if list.units.is_empty() {
                // No unit has this trigram, so none holds the needle.
                return Ok(Vec::new());
            }
            lists.push(list);
        }
        // The lists that fit one page first, shortest first: they cost nothing more to read,
        // and they leave the fewest candidates for the long ones.
        lists.sort_by_key(|l| if l.more.is_none() { l.units.len() } else { usize::MAX });
        let mut lists = lists.into_iter();
        let Some(mut first) = lists.next() else {
            return Ok(Vec::new());
        };
        let mut candidates = self.read_list(&mut first)?;
        for mut list in lists {
            if candidates.is_empty() {
                break;
            }
            if list.more.is_some() && candidates.len() <= PROBE_MAX {
                // A long list and few candidates: one point read per candidate.
                let mut probe = list.start.clone();
                let mut kept = Vec::with_capacity(candidates.len());
                for unit in candidates {
                    probe.truncate(list.start.len());
                    probe.extend_from_slice(&unit.0);
                    if self.snap.get(&probe)?.is_some() {
                        kept.push(unit);
                    }
                }
                candidates = kept;
            } else {
                let units = self.read_list(&mut list)?;
                candidates.retain(|u| units.binary_search(u).is_ok());
            }
        }
        Ok(candidates)
    }
}
