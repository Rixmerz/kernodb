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
//! A posting list per type and trigram, cut in blocks of at most
//! [`BLOCK_MAX`] unit ids, one engine record each:
//!
//! * `TRIGRAM` `0x7a ‖ type_len: u8 ‖ unit_type ‖ gram[3] ‖ 0x00 ‖ bound[16]`
//!   or `… ‖ gram[3] ‖ 0x01` (the tail block) →
//!   `envelope[2] ‖ unit[16] × n`, `1 <= n <= BLOCK_MAX`, ascending.
//!
//! The blocks of a list hold consecutive ranges of unit ids: every unit of a
//! block is above the bound of the block before it and at most its own bound;
//! the tail block, last, has no bound. So the block of a unit is the first
//! key at or after `gram ‖ 0x00 ‖ unit`: one forward seek. Unit ids are
//! UUIDv7, ordered by creation, so new units go to the tail block.
//!
//! The grams of a payload are the three bytes that start at each of its
//! positions, the payload followed by two `0x00`. So every position has a
//! gram, and a needle of one or two bytes is the prefix of a gram of every
//! unit that holds it: a short needle is a range scan too, not a scan of the
//! payloads.
//!
//! The type comes first in the key, so a search of one type reads only that
//! type's blocks, and turning the option off is one range deletion.
//!
//! # A write
//!
//! An operation plans its index change by reads only, before its first
//! write: the grams of the payload the head had (read from the transaction)
//! against those of the new one. The change set keeps the difference, gram by
//! gram, and writes it at commit ([`ChangeSet::flush_substring`]): each block
//! it changes is read and written once per change set, whatever the number of
//! units of the change set in it. A block over [`BLOCK_MAX`] ids is cut, and
//! an emptied one deleted.
//!
//! One record per trigram and unit, the first design, made a batch of 10,000
//! names write 700,000 records; the per-record cost of the engine (the
//! transaction's ordered map, the memtable, freeing them) was 70% of a
//! benchmark run (ADR-0025).
//!
//! # A search
//!
//! The units that hold every trigram of the needle are found by intersecting
//! its trigram lists, the shortest first; a list much longer than the
//! candidates left is probed by seeks instead of being read. A needle of up
//! to three bytes is answered by the index alone. A longer one is then
//! checked against the payload of each candidate, since a unit can hold every
//! trigram of a needle without holding the needle.
//!
//! `ponytail:` a block stores whole 16-byte ids and a small commit rewrites
//! the tail block of each of its grams (up to 2 KiB each). Upgrade path:
//! dense per-type numbers with delta-coded postings, as SQLite's FTS5 does,
//! if index size or small-commit cost matters.

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

/// Most unit ids in one block of a posting list.
pub const BLOCK_MAX: usize = 128;

const RECORD_VERSION: u8 = 1;
const MAX_TYPE_LEN: usize = 64;

/// The byte after the gram of a bounded block's key, and of the tail's.
const BOUNDED: u8 = 0x00;
const TAIL: u8 = 0x01;

/// Blocks read per engine scan call (two in the unit tests, so that their
/// lists of a few hundred units take several pages).
const PAGE: usize = if cfg!(test) { 2 } else { 256 };

/// With at most this many candidates left, a trigram list that did not fit
/// one page is probed by seeks instead of being read whole.
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

/// `0x7a ‖ type_len ‖ unit_type`: the prefix of every block of a type.
pub(crate) fn type_prefix(unit_type: &str) -> Vec<u8> {
    let mut k = vec![TAG_TRIGRAM, unit_type.len() as u8];
    k.extend_from_slice(unit_type.as_bytes());
    k
}

/// `type prefix ‖ gram`: the prefix of the blocks of one posting list.
fn list_key(prefix: &[u8], gram: Gram) -> Vec<u8> {
    let mut k = Vec::with_capacity(prefix.len() + 3 + 17);
    k.extend_from_slice(prefix);
    k.extend_from_slice(&gram);
    k
}

/// The key of the block of a list with the bound `bound` (`None`: the tail).
pub(crate) fn block_key(list: &[u8], bound: Option<UnitId>) -> Vec<u8> {
    let mut k = Vec::with_capacity(list.len() + 17);
    k.extend_from_slice(list);
    match bound {
        Some(unit) => {
            k.push(BOUNDED);
            k.extend_from_slice(&unit.0);
        }
        None => k.push(TAIL),
    }
    k
}

/// The block value of `units`, ascending.
#[cfg(test)]
pub(crate) fn encode_block(units: &[UnitId]) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + 16 * units.len());
    v.extend_from_slice(&[RECORD_VERSION, 0]);
    for u in units {
        v.extend_from_slice(&u.0);
    }
    v
}

/// The type, the trigram and the bound (`None`: the tail) of a block key.
pub(crate) fn decode_key(k: &[u8]) -> Result<(String, Gram, Option<UnitId>), DecodeError> {
    let shape = || DecodeError::new(UnitRule::Unit3, format!("trigram key of {} bytes has no valid shape", k.len()));
    let tlen = usize::from(*k.get(1).ok_or_else(shape)?);
    if k[0] != TAG_TRIGRAM || tlen == 0 || tlen > MAX_TYPE_LEN || k.len() < 2 + tlen + 4 {
        return Err(shape());
    }
    let unit_type = std::str::from_utf8(&k[2..2 + tlen])
        .map_err(|_| DecodeError::new(UnitRule::Unit12, "trigram key unit_type is not UTF-8"))?;
    let at = 2 + tlen;
    let gram = [k[at], k[at + 1], k[at + 2]];
    let bound = match (&k[at + 3], k.len() - at - 4) {
        (&TAIL, 0) => None,
        (&BOUNDED, 16) => Some(UnitId(k[at + 4..].try_into().map_err(|_| shape())?)),
        _ => return Err(shape()),
    };
    Ok((unit_type.to_owned(), gram, bound))
}

/// The unit ids of a block value: at least one, at most [`BLOCK_MAX`], strictly ascending.
pub(crate) fn decode_block(v: &[u8]) -> Result<Vec<UnitId>, DecodeError> {
    let e = |rule, m: &str| DecodeError::new(rule, format!("trigram block: {m}"));
    let body = crate::blocks::body(v, "trigram block")?;
    if body.is_empty() || !body.len().is_multiple_of(16) {
        return Err(e(UnitRule::Unit9, "no whole unit id, or a partial one"));
    }
    if body.len() / 16 > BLOCK_MAX {
        return Err(e(UnitRule::Unit11, "more unit ids than a block holds"));
    }
    let units: Vec<UnitId> = body.as_chunks::<16>().0.iter().map(|c| UnitId(*c)).collect();
    if units.windows(2).any(|w| w[0] >= w[1]) {
        return Err(e(UnitRule::Unit13, "unit ids not strictly ascending"));
    }
    Ok(units)
}

/// The trigram blocks as [`crate::blocks`] stores them: unit ids, no value.
struct Trigrams;

impl crate::blocks::Block for Trigrams {
    type Val = ();
    const MAX: usize = BLOCK_MAX;
    const WHAT: &'static str = "TRIGRAM";

    fn decode(v: &[u8]) -> Result<Vec<(crate::blocks::Id, ())>, DecodeError> {
        Ok(decode_block(v)?.into_iter().map(|u| (u.0, ())).collect())
    }

    fn encode(entries: &[(crate::blocks::Id, ())]) -> Vec<u8> {
        let mut v = Vec::with_capacity(2 + 16 * entries.len());
        v.extend_from_slice(&[RECORD_VERSION, 0]);
        for (u, ()) in entries {
            v.extend_from_slice(u);
        }
        v
    }
}

// --------------------------------------------------------------- writer

/// The index change of one operation: its type prefix, the grams the unit no
/// longer has and those it gains.
pub(crate) struct GramChange {
    prefix: Vec<u8>,
    gone: Vec<Gram>,
    added: Vec<Gram>,
}

/// What a change set will change in the index: per list key, the units to add
/// (`true`) or remove (`false`), the last operation of a unit winning.
pub(crate) type PendingGrams =
    std::collections::BTreeMap<Vec<u8>, std::collections::BTreeMap<crate::blocks::Id, Option<()>>>;

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

    /// The index change that follows the operation `writes` on `unit`, planned
    /// by reads only, before the first write of the operation (like
    /// [`ChangeSet::plan_fulltext`](crate::fulltext)). `None` for a type
    /// without the option, and for an operation that writes no head. The
    /// caller hands it to [`stage_substring`](Self::stage_substring) once the
    /// operation is written.
    pub(crate) fn plan_substring(&self, unit: UnitId, writes: &[Write]) -> Result<Option<GramChange>, UnitError> {
        let Next::Content(unit_type, payload) = next_of(unit, writes)? else {
            return Ok(None);
        };
        if !self.substring_enabled(&unit_type)? {
            return Ok(None);
        }
        let (old, new) = (self.indexed_grams(unit)?, payload.map(grams).unwrap_or_default());
        // Both lists are sorted, so a binary search tells what the other lacks.
        let gone = old.iter().filter(|g| new.binary_search(g).is_err()).copied().collect();
        let added = new.iter().filter(|g| old.binary_search(g).is_err()).copied().collect();
        Ok(Some(GramChange { prefix: type_prefix(&unit_type), gone, added }))
    }

    /// Keeps the index change of an operation on `unit` for
    /// [`flush_substring`](Self::flush_substring).
    pub(crate) fn stage_substring(&mut self, unit: UnitId, change: Option<GramChange>) {
        let Some(c) = change else { return };
        for (gram, add) in c.gone.iter().map(|g| (g, false)).chain(c.added.iter().map(|g| (g, true))) {
            self.substring_pending.entry(list_key(&c.prefix, *gram)).or_default().insert(unit.0, add.then_some(()));
        }
    }

    /// Writes the index changes the change set kept, block by block: each
    /// block one changed unit falls in is read once, changed for every unit of
    /// the change set in it, then written back (cut when over [`BLOCK_MAX`],
    /// deleted when emptied). Called by `commit`.
    pub(crate) fn flush_substring(&mut self) -> Result<(), UnitError> {
        for (list, ops) in std::mem::take(&mut self.substring_pending) {
            crate::blocks::flush::<Trigrams, E>(self, &list, ops, false)?;
        }
        Ok(())
    }

    /// Turns the trigram index of `unit_type` on (`enabled = true`) or back
    /// off (the default). The type's `Active` units follow in the same change
    /// set: turning it on indexes each of them, turning it off deletes the
    /// type's blocks, so the index never disagrees with the units. Setting
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
            let prefix = type_prefix(unit_type);
            let (start, end) = prefix_bounds(&prefix);
            if !enabled {
                // What this change set kept for the type goes with its blocks.
                cs.substring_pending.retain(|k, _| !k.starts_with(&prefix));
                return cs.write_owned(Write::DeleteRange(start, end));
            }
            for (unit, payload_key) in units {
                let Some(payload) = cs.txn()?.get(&payload_key)? else { continue };
                let added = grams(&payload);
                cs.stage_substring(unit, Some(GramChange { prefix: prefix.clone(), gone: Vec::new(), added }));
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
        for (_, v) in &page {
            list.units.extend(decode_block(v).map_err(|e| context(e, "TRIGRAM"))?);
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
                // A long list and few candidates: one seek per candidate, to its block.
                let mut kept = Vec::with_capacity(candidates.len());
                for unit in candidates {
                    let from = block_key(&list.start, Some(unit));
                    let page = self.snap.scan(Bound::Included(&from), Bound::Excluded(&list.end), 1)?;
                    if let Some((_, v)) = page.first()
                        && decode_block(v).map_err(|e| context(e, "TRIGRAM"))?.binary_search(&unit).is_ok()
                    {
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
