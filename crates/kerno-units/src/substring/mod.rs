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

/// What answers the types the trigram index does not hold in a substring search, from the types
/// (`None`: every type), the folded needle and the limit.
type ScanFn<'a> = &'a dyn Fn(Option<&[&str]>, &[u8], usize) -> Result<Vec<UnitId>, UnitError>;

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
    let mut packed = distinct(payload);
    packed.sort_unstable();
    packed.into_iter().map(unpack).collect()
}

/// The distinct trigrams of the indexed part of `payload`, packed ([`pack`]), in no order.
fn distinct(payload: &[u8]) -> Vec<u32> {
    let text = indexed(payload);
    if text.is_empty() {
        return Vec::new();
    }
    // A text repeats most of its trigrams, so they are made distinct here, in a small
    // open-addressing set, and a caller that needs them in order sorts only the distinct
    // ones: sorting every position and then dropping the repeats cost several times more. A gram is packed big-endian in 24
    // bits, which orders like its three bytes; a slot holds the gram plus one (0: empty).
    let bits = (text.len() * 2).next_power_of_two().trailing_zeros().max(4);
    let mut seen = vec![0u32; 1 << bits];
    let mask = seen.len() - 1;
    let mut packed: Vec<u32> = Vec::with_capacity(text.len().min(1 << 12));
    // The gram of a position is the gram before it shifted by the byte two places on; the
    // two `0x00` after the text end the last two.
    let fold = |i: usize| text.get(i).map_or(0, |b| u32::from(b.to_ascii_lowercase()));
    let mut gram = fold(0) << 8 | fold(1);
    // The text, then the two `0x00` (one after a text of one byte). `for_each`, not `for`: a
    // chained iterator is walked part by part that way, without a test per byte.
    let body = text.get(2..).unwrap_or_default().iter().map(|b| u32::from(b.to_ascii_lowercase()));
    body.chain(std::iter::repeat_n(0, text.len().min(2))).for_each(|next| {
        gram = (gram << 8 | next) & 0x00FF_FFFF;
        let mut slot = (gram.wrapping_mul(0x9E37_79B1) >> (32 - bits)) as usize;
        loop {
            match seen[slot] {
                0 => {
                    seen[slot] = gram + 1;
                    packed.push(gram);
                    break;
                }
                g if g == gram + 1 => break,
                _ => slot = (slot + 1) & mask,
            }
        }
    });
    packed
}

/// A trigram as an integer that orders like its three bytes, and back.
fn pack(g: Gram) -> u32 {
    u32::from(g[0]) << 16 | u32::from(g[1]) << 8 | u32::from(g[2])
}

fn unpack(g: u32) -> Gram {
    [(g >> 16) as u8, (g >> 8) as u8, g as u8]
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
#[cfg(test)]
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
    const ENTRY: usize = 16;

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

    fn put_val(_: &mut Vec<u8>, (): &()) {}
}

// --------------------------------------------------------------- writer

/// The index change of one operation: its type prefix, the grams the unit no
/// longer has and those it gains (packed, in no order).
pub(crate) struct GramChange {
    prefix: Vec<u8>,
    gone: Vec<u32>,
    added: Vec<u32>,
}

/// What a change set will change in the index: per unit type (by its key
/// prefix) and trigram, the operations in the order they came, each a unit to
/// add (`true`) or remove (`false`). [`ChangeSet::flush_substring`] keeps the
/// last operation of a unit.
///
/// A batch stages one entry per trigram of every unit, so the lookup is the
/// hot path: the trigram is the map key, as an integer hashed by one
/// multiplication (the keys are the change set's own, not chosen by a
/// stranger to collide), and the type is found first, once per unit.
#[derive(Default)]
pub(crate) struct PendingGrams {
    types: Vec<(Vec<u8>, GramOps)>,
}

type GramOps = std::collections::HashMap<u32, Vec<(UnitId, bool)>, std::hash::BuildHasherDefault<GramHasher>>;

/// Hashes a packed trigram (one `u32`) by a multiplication.
#[derive(Default)]
pub(crate) struct GramHasher(u64);

impl std::hash::Hasher for GramHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3);
        }
    }

    fn write_u32(&mut self, g: u32) {
        // The table takes its index from the low bits and its tag from the high ones.
        let h = u64::from(g).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = h ^ (h >> 32);
    }
}

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
        let payload = match head.inline_payload {
            Some(p) => Some(p),
            None => txn.get(&key(Key::RevPayload { unit, rev: head.head_rev })?)?,
        };
        Ok(payload.map(|p| grams(&p)).unwrap_or_default())
    }

    /// The index change that follows the operation `writes` on `unit`, planned
    /// by reads only, before the first write of the operation (like
    /// [`ChangeSet::plan_fulltext`](crate::fulltext)). `None` for a type
    /// without the option, and for an operation that writes no head. The
    /// caller hands it to [`stage_substring`](Self::stage_substring) once the
    /// operation is written.
    pub(crate) fn plan_substring(
        &self,
        unit: UnitId,
        next: Option<&crate::db::PlannedHead<'_>>,
    ) -> Result<Option<GramChange>, UnitError> {
        let Next::Content(unit_type, payload) = next_of(next) else {
            return Ok(None);
        };
        if !self.substring_enabled(unit_type)? {
            return Ok(None);
        }
        let (old, mut new) = (self.indexed_grams(unit)?, payload.map(distinct).unwrap_or_default());
        if old.is_empty() {
            self.learn_fresh(unit)?;
            // A unit the index does not hold yet (every unit of a bulk load): all its grams
            // are added, and nothing needs them in order.
            return Ok(Some(GramChange { prefix: type_prefix(unit_type), gone: Vec::new(), added: new }));
        }
        // Both lists sorted, so a binary search tells what the other lacks.
        new.sort_unstable();
        let old: Vec<u32> = old.into_iter().map(pack).collect();
        let gone = old.iter().filter(|g| new.binary_search(g).is_err()).copied().collect();
        let added = new.iter().filter(|g| old.binary_search(g).is_err()).copied().collect();
        Ok(Some(GramChange { prefix: type_prefix(unit_type), gone, added }))
    }

    /// Keeps the index change of an operation on `unit` for
    /// [`flush_substring`](Self::flush_substring).
    pub(crate) fn stage_substring(&mut self, unit: UnitId, change: Option<GramChange>) {
        let Some(c) = change else { return };
        let types = &mut self.substring_pending.types;
        let at = match types.iter().position(|(p, _)| *p == c.prefix) {
            Some(at) => at,
            None => {
                types.push((c.prefix, GramOps::default()));
                types.len() - 1
            }
        };
        let ops = &mut types[at].1;
        for (gram, add) in c.gone.iter().map(|g| (g, false)).chain(c.added.iter().map(|g| (g, true))) {
            ops.entry(*gram).or_default().push((unit, add));
        }
    }

    /// Writes the index changes the change set kept, block by block: each
    /// block one changed unit falls in is read once, changed for every unit of
    /// the change set in it, then written back (cut when over [`BLOCK_MAX`],
    /// deleted when emptied). Called by `commit`.
    pub(crate) fn flush_substring(&mut self) -> Result<(), UnitError> {
        let mut types = std::mem::take(&mut self.substring_pending).types;
        // Lists in ascending key order, as the probe needs: by type prefix, then by trigram
        // (packed big-endian, so the integers order like the key bytes). A prefix starts with
        // the length of its type, so none is the prefix of another.
        types.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut probe = crate::blocks::Probe::new(vec![TAG_TRIGRAM + 1]);
        let above = self.fresh_from().map(|u| u.0);
        for (prefix, ops) in types {
            let mut lists: Vec<(u32, Vec<(UnitId, bool)>)> = ops.into_iter().collect();
            lists.sort_unstable_by_key(|l| l.0);
            let mut list = prefix;
            let at = list.len();
            list.extend_from_slice(&[0; 3]);
            for (gram, mut ops) in lists {
                list[at..].copy_from_slice(&unpack(gram));
                // Ascending by unit; of several operations on a unit the last wins. A batch
                // of new units stages them in id order already: nothing to sort then.
                if !ops.windows(2).all(|w| w[0].0 < w[1].0) {
                    ops.reverse();
                    ops.sort_by_key(|(u, _)| *u);
                    ops.dedup_by_key(|(u, _)| *u);
                }
                let ops = ops.into_iter().map(|(u, add)| (u.0, add.then_some(())));
                crate::blocks::flush::<Trigrams, E>(self, &list, ops, false, Some(&mut probe), above)?;
            }
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
                cs.substring_pending.types.retain(|(p, _)| *p != prefix);
                return cs.write_owned(Write::DeleteRange(start, end));
            }
            for (unit, at) in units {
                let txn = cs.txn()?;
                let Some(payload) = at.read(|k| txn.get(k))? else { continue };
                let added = distinct(&payload);
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
        self.search_substring_with(needle, limit, types, None)
    }

    /// [`search_substring`](Self::search_substring), with `scan` answering the types the trigram
    /// index does not hold (instead of `NotIndexed`): a type in `types` that is known but not
    /// indexed, or every type (`None` in its argument) when `types` is `None` and no type is
    /// indexed. `scan` gets the folded needle and the limit, and answers in unit id order.
    fn search_substring_with(
        &self,
        needle: &[u8],
        limit: Option<usize>,
        types: Option<&[&str]>,
        scan: Option<ScanFn<'_>>,
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
        // The types the index does not hold that `scan` answers: `Some(list)`, or `None` for all.
        let mut scanned: Option<Option<Vec<&str>>> = None;
        if indexed.is_empty() && types.is_none() {
            if scan.is_none() {
                return Err(UnitError::NotIndexed {
                    detail: "no unit type is substring-indexed: turn it on for a type with set_substring".into(),
                });
            }
            scanned = Some(None);
        }
        for t in types.unwrap_or_default() {
            if !self.type_known(t)? {
                return Err(UnitError::UnknownType { unit_type: (*t).to_string() });
            }
            if !indexed.iter().any(|i| i == t) {
                if scan.is_none() {
                    return Err(UnitError::NotIndexed {
                        detail: format!("unit type {t:?} is not substring-indexed: turn it on with set_substring"),
                    });
                }
                if let Some(list) = scanned.get_or_insert_with(|| Some(Vec::new())) {
                    list.push(t);
                }
            }
        }
        let needle = needle.to_ascii_lowercase();
        let limit = limit.unwrap_or(usize::MAX);
        let mut out = Vec::new();
        for unit_type in indexed.iter().filter(|i| types.is_none_or(|ts| ts.contains(&i.as_str()))) {
            out.extend(self.search_type(unit_type, &needle, limit)?);
        }
        if let (Some(which), Some(scan)) = (scanned, scan) {
            out.extend(scan(which.as_deref(), &needle, limit)?);
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
            let payload = match head.inline_payload {
                Some(p) => Some(p),
                None => self.snap.get(&key(Key::RevPayload { unit, rev: head.head_rev })?)?,
            };
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

// ------------------------------------------------------------ scan path

/// Heads read per scan call of a substring scan.
const HEAD_PAGE: usize = 1024;
/// Slices of the id space per thread in a substring scan: units are rarely spread evenly over
/// their ids (they are minted by time, in bursts), so the threads take many small slices.
const SLICES_PER_THREAD: usize = 32;

/// A unit a scan reads: its payload in its head, or the revision that holds it.
enum Source {
    Inline(Vec<u8>),
    Rev(crate::types::RevNo),
}

impl<E: Engine> UnitSnapshot<E>
where
    E::Snapshot: Sync,
{
    /// [`search_substring`](Self::search_substring), answering the unit types the trigram index
    /// does not hold by reading their payloads: a type in `types` that is known but not
    /// indexed, and every type when `types` is `None` and no type is indexed (where
    /// `search_substring` refuses with `NotIndexed`). Same matches and order as the index would
    /// give: the first [`MAX_INDEXED_BYTES`] bytes of the head payload of every `Active` unit,
    /// ASCII letters folded. The payloads are searched with `memchr::memmem`, on several
    /// threads when there are many.
    ///
    /// # Errors
    ///
    /// As [`search_substring`](Self::search_substring), without `NotIndexed`.
    pub fn search_substring_or_scan(
        &self,
        needle: &[u8],
        limit: Option<usize>,
        types: Option<&[&str]>,
    ) -> Result<Vec<UnitId>, UnitError> {
        self.search_substring_with(needle, limit, types, Some(&|t, n, l| self.scan_substring(t, n, l)))
    }

    /// The first `limit` `Active` units of `types` (`None`: every type) whose payload holds the
    /// folded `needle`, in id order, by reading every head and payload.
    ///
    /// The `UNIT_HEAD` range is cut in slices of the id space between its first and its last
    /// key, many more slices than threads; each thread takes the next slice, scans its heads and
    /// searches the payloads of the units it keeps. The slices' answers are joined in slice
    /// order, which is id order.
    fn scan_substring(&self, types: Option<&[&str]>, needle: &[u8], limit: usize) -> Result<Vec<UnitId>, UnitError> {
        use crate::codec::TAG_UNIT_HEAD;
        let Some((first, last)) = self.head_span()? else {
            return Ok(Vec::new());
        };
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        let slices = if threads == 1 { 1 } else { threads * SLICES_PER_THREAD };
        let step = (last - first) / slices as u128 + 1;
        // Slice `i` is `[bound(i), bound(i + 1))`; the last one ends with the range.
        let bound = |i: usize| -> Vec<u8> {
            let mut k = vec![TAG_UNIT_HEAD];
            match first.checked_add(step.saturating_mul(i as u128)) {
                Some(id) if i < slices => k.extend_from_slice(&id.to_be_bytes()),
                _ => k[0] = TAG_UNIT_HEAD + 1,
            }
            k
        };
        let finder = memchr::memmem::Finder::new(needle);
        let fold = needle.iter().any(u8::is_ascii_lowercase);
        let slice = |i: usize| -> Result<Vec<UnitId>, UnitError> {
            let units = self.heads_in(&bound(i), &bound(i + 1), types)?;
            let mut out = Vec::new();
            let mut folded = Vec::new();
            for (unit, source) in &units {
                let read;
                let payload: &[u8] = match source {
                    Source::Inline(p) => p,
                    Source::Rev(rev) => {
                        read = self.snap.get(&key(Key::RevPayload { unit: *unit, rev: *rev })?)?;
                        match &read {
                            Some(p) => p,
                            None => continue,
                        }
                    }
                };
                let hay = indexed(payload);
                let hit = if fold {
                    folded.clear();
                    folded.extend(hay.iter().map(u8::to_ascii_lowercase));
                    finder.find(&folded).is_some()
                } else {
                    finder.find(hay).is_some()
                };
                if hit {
                    out.push(*unit);
                    if out.len() == limit {
                        break;
                    }
                }
            }
            Ok(out)
        };
        let mut parts: Vec<(usize, Result<Vec<UnitId>, UnitError>)> = if slices == 1 {
            vec![(0, slice(0))]
        } else {
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|scope| {
                let workers: Vec<_> = (0..threads)
                    .map(|_| {
                        scope.spawn(|| {
                            let mut done = Vec::new();
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if i >= slices {
                                    return done;
                                }
                                done.push((i, slice(i)));
                            }
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .flat_map(|w| {
                        w.join()
                            .unwrap_or_else(|_| vec![(0, Err(corruption("a substring scan thread panicked".into())))])
                    })
                    .collect()
            })
        };
        parts.sort_unstable_by_key(|(i, _)| *i);
        let mut out = Vec::new();
        for (_, part) in parts {
            out.extend(part?);
            if out.len() >= limit {
                out.truncate(limit);
                break;
            }
        }
        Ok(out)
    }

    /// The ids of the first and the last `UNIT_HEAD`, as numbers (`None`: no unit): one seek for
    /// the first, a binary search of seeks for the last.
    fn head_span(&self) -> Result<Option<(u128, u128)>, UnitError> {
        use crate::codec::TAG_UNIT_HEAD;
        let end = [TAG_UNIT_HEAD + 1];
        let id_at_or_after = |from: u128| -> Result<Option<u128>, UnitError> {
            let mut k = vec![TAG_UNIT_HEAD];
            k.extend_from_slice(&from.to_be_bytes());
            let page = self.snap.scan(Bound::Included(&k), Bound::Excluded(&end), 1)?;
            Ok(page.first().and_then(|(k, _)| k.get(1..17)).map(|id| {
                let mut b = [0u8; 16];
                b.copy_from_slice(id);
                u128::from_be_bytes(b)
            }))
        };
        let Some(first) = id_at_or_after(0)? else {
            return Ok(None);
        };
        // `lo` has a head; nothing above `hi`.
        let (mut lo, mut hi) = (first, u128::MAX);
        while lo < hi {
            let mid = lo + (hi - lo) / 2 + 1;
            match id_at_or_after(mid)? {
                Some(id) => lo = id,
                None => hi = mid - 1,
            }
        }
        Ok(Some((first, lo)))
    }

    /// The `Active` units of `types` (`None`: every type) with a head in `[from, to)`, in id
    /// order, with where their payload is. The heads are visited borrowed; the type is the
    /// first field after the envelope (`len: u32 BE ‖ bytes`), so a head of a type not asked
    /// for is skipped on those bytes and only the others are decoded.
    fn heads_in(&self, from: &[u8], to: &[u8], types: Option<&[&str]>) -> Result<Vec<(UnitId, Source)>, UnitError> {
        use crate::codec::UnitHead;
        let wanted = |v: &[u8]| match (types, v.get(2..6)) {
            (None, _) => true,
            (Some(ts), Some(len)) => {
                let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
                v.get(6..6 + len).is_some_and(|t| ts.iter().any(|w| w.as_bytes() == t))
            }
            // Too short to hold a type: let the decoder report it.
            (Some(_), None) => true,
        };
        let mut units = Vec::new();
        let mut cursor = from.to_vec();
        let mut inclusive = true;
        loop {
            let mut last: Option<Vec<u8>> = None;
            let mut failed: Option<UnitError> = None;
            let lo = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let seen = self.snap.scan_visit(lo, Bound::Excluded(to), HEAD_PAGE, &mut |k, v| {
                if failed.is_none() && wanted(v) {
                    let r = (|| {
                        let Key::UnitHead { unit } = Key::decode(k)? else {
                            return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
                        };
                        let head = UnitHead::decode(v).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
                        if head.lifecycle == Lifecycle::Active
                            && types.is_none_or(|ts| ts.contains(&head.unit_type.as_str()))
                        {
                            let source = match head.inline_payload {
                                Some(p) => Source::Inline(p),
                                None => Source::Rev(head.head_rev),
                            };
                            units.push((unit, source));
                        }
                        Ok(())
                    })();
                    failed = r.err();
                }
                last = Some(k.to_vec());
                Ok(())
            })?;
            if let Some(e) = failed {
                return Err(e);
            }
            match last {
                Some(k) if seen == HEAD_PAGE => {
                    cursor = k;
                    inclusive = false;
                }
                _ => return Ok(units),
            }
        }
    }
}
