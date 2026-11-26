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
use crate::runs::{self, Family, Manifest, Pending, Slots, Staged};
use crate::schema::prefix_bounds;
use crate::type_options::{SUBSTRING, TAG_TYPE_OPTIONS};
use crate::types::UnitId;

pub(crate) use check::Check;

/// Key tag of a trigram entry.
pub const TAG_TRIGRAM: u8 = 0x7a;

/// How much of a payload is indexed and searched: its first bytes, as in the
/// keyword index ([`crate::fulltext::MAX_INDEXED_BYTES`]). A unit's entries
/// are written in one engine transaction, which has a batch limit.
pub const MAX_INDEXED_BYTES: usize = crate::fulltext::MAX_INDEXED_BYTES;

/// Longest needle of a search, in bytes.
pub const MAX_NEEDLE_LEN: usize = 1024;

const MAX_TYPE_LEN: usize = 64;

/// Pages of a list read from each run before the list counts as a long one (two in the unit
/// tests, so that their lists of a few hundred units are long).
const PAGE: usize = if cfg!(test) { 2 } else { 8 };

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

/// A packed trigram (24 bits, big-endian, so it orders like its three bytes) as bytes.
fn unpack(g: u32) -> Gram {
    [(g >> 16) as u8, (g >> 8) as u8, g as u8]
}

/// Whether the indexed part of `payload` holds `needle` (already folded).
fn holds(payload: &[u8], needle: &[u8]) -> bool {
    indexed(payload).windows(needle.len()).any(|w| w.iter().zip(needle).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

// ----------------------------------------------------------------- keys

/// `0x7a ‖ type_len ‖ unit_type`: the prefix of every record of a type's index.
pub(crate) fn type_prefix(unit_type: &str) -> Vec<u8> {
    let mut k = vec![TAG_TRIGRAM, unit_type.len() as u8];
    k.extend_from_slice(unit_type.as_bytes());
    k
}

/// The unit type of a trigram key and the length of its index prefix.
pub(crate) fn decode_prefix(k: &[u8]) -> Result<(&str, usize), DecodeError> {
    let shape = || DecodeError::new(UnitRule::Unit3, format!("trigram key of {} bytes has no valid shape", k.len()));
    let tlen = usize::from(*k.get(1).ok_or_else(shape)?);
    if k[0] != TAG_TRIGRAM || tlen == 0 || tlen > MAX_TYPE_LEN || k.len() < 2 + tlen + 1 {
        return Err(shape());
    }
    let unit_type = std::str::from_utf8(&k[2..2 + tlen])
        .map_err(|_| DecodeError::new(UnitRule::Unit12, "trigram key unit_type is not UTF-8"))?;
    Ok((unit_type, 2 + tlen))
}

/// The trigram index as [`crate::runs`] stores it: a list per trigram, postings with no value.
pub(crate) struct Trigrams;

impl Family for Trigrams {
    const WHAT: &'static str = "TRIGRAM";
    const VALUES: bool = false;

    fn list_len(rest: &[u8]) -> Option<usize> {
        (rest.len() >= 3).then_some(3)
    }
}

// --------------------------------------------------------------- writer

/// The index change of one operation: the prefix of its type's index, the number the unit
/// had there (`None`: it was not indexed) and the trigrams of its new content (packed, in no
/// order; none: it leaves the index).
pub(crate) struct GramChange {
    prefix: Vec<u8>,
    old: Option<u64>,
    added: Vec<u32>,
}

/// What a change set will change in the index, per unit type: the numbers given and those
/// that die ([`runs::Pending`]) and, per trigram, the numbers that have it, less the first
/// number the change set gives, in ascending order.
///
/// A batch stages one entry per trigram of every unit, so the lookup is the
/// hot path: the trigram is the map key, as an integer hashed by one
/// multiplication (the keys are the change set's own, not chosen by a
/// stranger to collide), and the type is found first, once per unit.
#[derive(Default)]
pub(crate) struct PendingGrams {
    types: Vec<(Pending, GramOps)>,
}

type GramOps = std::collections::HashMap<u32, Vec<u32>, std::hash::BuildHasherDefault<GramHasher>>;

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

/// The staged lists of one type, in trigram order, for [`runs::flush`].
struct StagedGrams {
    lists: Vec<(u32, Vec<u32>)>,
    at: usize,
    base: u64,
    key: Gram,
}

impl Staged for StagedGrams {
    fn peek(&mut self) -> Option<&[u8]> {
        let (gram, _) = self.lists.get(self.at)?;
        self.key = unpack(*gram);
        Some(&self.key)
    }

    fn take(&mut self, emit: &mut dyn FnMut(runs::Posting)) {
        if let Some((_, offsets)) = self.lists.get(self.at) {
            for off in offsets {
                emit((self.base + u64::from(*off), 0, 0));
            }
        }
        self.at += 1;
    }
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Whether the payloads of `unit_type` go in the trigram index, as this
    /// change set sees it.
    pub(crate) fn substring_enabled(&self, unit_type: &str) -> Result<bool, UnitError> {
        Ok(self.type_options(unit_type)? & SUBSTRING != 0)
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
        let prefix = type_prefix(&unit_type);
        // The document record says whether the index holds the unit, and under which number:
        // the content it had is not read.
        let old = match self.txn()?.get(&runs::doc_key(&prefix, unit))? {
            Some(v) => Some(runs::decode_doc(&v, "TRIGRAM").map_err(|e| context(e, "a trigram document record"))?.0),
            None => None,
        };
        let added = payload.map(distinct).unwrap_or_default();
        if old.is_none() && added.is_empty() {
            return Ok(None);
        }
        Ok(Some(GramChange { prefix, old, added }))
    }

    /// Keeps the index change of an operation on `unit` for
    /// [`flush_substring`](Self::flush_substring), and writes its document record: the number
    /// the unit had dies, and its new content, if it has any, gets the next one.
    pub(crate) fn stage_substring(&mut self, unit: UnitId, change: Option<GramChange>) -> Result<(), UnitError> {
        let Some(c) = change else { return Ok(()) };
        let at = match self.substring_pending.types.iter().position(|(p, _)| p.prefix == c.prefix) {
            Some(at) => at,
            None => {
                let pending = Pending::load::<_, Trigrams>(self.txn()?, c.prefix)?;
                self.substring_pending.types.push((pending, GramOps::default()));
                self.substring_pending.types.len() - 1
            }
        };
        let (pending, ops) = &mut self.substring_pending.types[at];
        if let Some(old) = c.old {
            pending.kill(old);
        }
        let key = runs::doc_key(&pending.prefix, unit);
        let write = if c.added.is_empty() {
            Write::Delete(key)
        } else {
            let num = pending.alloc(unit);
            let off = u32::try_from(num - pending.base())
                .map_err(|_| UnitError::InvalidArgument("a change set indexes at most 2^32 units".into()))?;
            for gram in &c.added {
                ops.entry(*gram).or_default().push(off);
            }
            Write::Put(key, runs::encode_doc(num, 0))
        };
        self.write_owned(write)
    }

    /// Writes the index changes the change set kept: per type, the postings of its new
    /// numbers as one run, the number pages and the manifest ([`runs::flush`]). Called by
    /// `commit`.
    pub(crate) fn flush_substring(&mut self) -> Result<(), UnitError> {
        let mut types = std::mem::take(&mut self.substring_pending).types;
        types.sort_unstable_by(|a, b| a.0.prefix.cmp(&b.0.prefix));
        for (pending, ops) in types {
            // Lists in key order: the trigrams packed big-endian order like their bytes.
            let mut lists: Vec<(u32, Vec<u32>)> = ops.into_iter().collect();
            lists.sort_unstable_by_key(|l| l.0);
            let mut staged = StagedGrams { lists, at: 0, base: pending.base(), key: [0; 3] };
            runs::flush::<E, Trigrams>(self, pending, &mut staged, false)?;
        }
        Ok(())
    }

    /// Turns the trigram index of `unit_type` on (`enabled = true`) or back
    /// off (the default). The type's `Active` units follow in the same change
    /// set: turning it on indexes each of them, turning it off deletes the
    /// type's index, so the index never disagrees with the units. Setting
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
                // What this change set kept for the type goes with its records.
                cs.substring_pending.types.retain(|(p, _)| p.prefix != prefix);
                return cs.write_owned(Write::DeleteRange(start, end));
            }
            for (unit, payload_key) in units {
                let Some(payload) = cs.txn()?.get(&payload_key)? else { continue };
                let added = distinct(&payload);
                if !added.is_empty() {
                    cs.stage_substring(unit, Some(GramChange { prefix: prefix.clone(), old: None, added }))?;
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

/// One trigram list of a search: what was read of it so far, run by run.
struct List {
    gram: Gram,
    /// The numbers read of each run, ascending, and where the read of the run stopped
    /// (`None`: the run's part of the list is read whole).
    parts: Vec<(Vec<u64>, Option<Vec<u8>>)>,
}

impl List {
    fn whole(&self) -> bool {
        self.parts.iter().all(|(_, more)| more.is_none())
    }

    fn len(&self) -> usize {
        self.parts.iter().map(|(n, _)| n.len()).sum()
    }
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
    pub(crate) fn substring_types(&self) -> Result<Vec<String>, UnitError> {
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
    /// * `Storage(Corruption)` when a record of the index does not decode, a page of a run is
    ///   missing, or the index names a unit that does not exist.
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
        let get = |k: &[u8]| self.snap.get(k);
        let manifest = runs::read_manifest::<Trigrams>(&get, &prefix)?;
        if manifest.runs.is_empty() {
            return Ok(Vec::new());
        }
        // A real byte is never the `0x00` that pads the last grams of a payload, unless the
        // needle itself has one: then the index may answer a unit that only has the padding.
        let padded = needle.contains(&0);
        let (nums, exact) = if needle.len() < 3 {
            // Every gram that starts with the needle: each position of a payload has one.
            let scan = |lo: Bound<&[u8]>, hi: Bound<&[u8]>, n: usize| self.snap.scan(lo, hi, n);
            let mut nums = Vec::new();
            for run in &manifest.runs {
                runs::read_prefix::<Trigrams>(&scan, &prefix, run, needle, &mut |p| nums.push(p.0))?;
            }
            nums.sort_unstable();
            nums.dedup();
            (nums, !padded)
        } else {
            let mut wanted: Vec<Gram> = needle.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
            wanted.sort_unstable();
            wanted.dedup();
            (self.intersect(&prefix, &manifest, &wanted)?, needle.len() == 3 && !padded)
        };
        // The units of the numbers, without those that died since their run was written, in
        // id order: numbers ascend in the order units were indexed, which is not always it.
        let mut slots = Slots::default();
        let mut candidates = Vec::with_capacity(nums.len());
        for num in nums {
            let (live, unit) = slots.slot(&get, &prefix, num)?;
            if live {
                candidates.push(unit);
            }
        }
        candidates.sort_unstable();
        if exact {
            candidates.truncate(limit);
            return Ok(candidates);
        }
        // Holding every trigram of the needle is not holding the needle: read the payload.
        let mut out = Vec::new();
        for unit in candidates {
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

    /// The list of `gram`, with the first pages of its part in each run read.
    fn list_of(&self, prefix: &[u8], manifest: &Manifest, gram: Gram) -> Result<List, UnitError> {
        let scan = |lo: Bound<&[u8]>, hi: Bound<&[u8]>, n: usize| self.snap.scan(lo, hi, n);
        let mut parts = Vec::with_capacity(manifest.runs.len());
        for run in &manifest.runs {
            let mut nums = Vec::new();
            let more =
                runs::read_list::<Trigrams>(&scan, prefix, run, &gram, 0, None, PAGE, &mut |p| nums.push(p.0))?;
            parts.push((nums, more));
        }
        Ok(List { gram, parts })
    }

    /// Reads the rest of `list` and returns its numbers, ascending: the runs hold ascending
    /// ranges of numbers, oldest first.
    fn read_list(&self, prefix: &[u8], manifest: &Manifest, list: List) -> Result<Vec<u64>, UnitError> {
        let scan = |lo: Bound<&[u8]>, hi: Bound<&[u8]>, n: usize| self.snap.scan(lo, hi, n);
        let mut out = Vec::with_capacity(list.len());
        for ((nums, mut more), run) in list.parts.into_iter().zip(&manifest.runs) {
            out.extend(nums);
            while let Some(after) = more.take() {
                more = runs::read_list::<Trigrams>(&scan, prefix, run, &list.gram, 0, Some(after), 64, &mut |p| {
                    out.push(p.0);
                })?;
            }
        }
        Ok(out)
    }

    /// The numbers of the index `prefix` that have a posting for every gram of `wanted`,
    /// ascending.
    fn intersect(&self, prefix: &[u8], manifest: &Manifest, wanted: &[Gram]) -> Result<Vec<u64>, UnitError> {
        let mut lists = Vec::with_capacity(wanted.len());
        for gram in wanted {
            let list = self.list_of(prefix, manifest, *gram)?;
            if list.len() == 0 {
                // No unit has this trigram, so none holds the needle.
                return Ok(Vec::new());
            }
            lists.push(list);
        }
        // The lists read whole already first, shortest first: they cost nothing more to
        // read, and they leave the fewest candidates for the long ones.
        lists.sort_by_key(|l| if l.whole() { l.len() } else { usize::MAX });
        let mut lists = lists.into_iter();
        let Some(first) = lists.next() else {
            return Ok(Vec::new());
        };
        let scan = |lo: Bound<&[u8]>, hi: Bound<&[u8]>, n: usize| self.snap.scan(lo, hi, n);
        let mut candidates = self.read_list(prefix, manifest, first)?;
        for list in lists {
            if candidates.is_empty() {
                break;
            }
            if !list.whole() && candidates.len() <= PROBE_MAX {
                // A long list and few candidates: each is looked up in its run, where it is
                // in what was read of the run already or in the page of its number.
                let mut kept = Vec::with_capacity(candidates.len());
                for num in candidates {
                    let at = manifest.runs.partition_point(|r| r.max < num);
                    let (Some(run), Some((nums, more))) = (manifest.runs.get(at), list.parts.get(at)) else {
                        continue;
                    };
                    let found = if more.is_none() || nums.last().is_some_and(|l| *l >= num) {
                        nums.binary_search(&num).is_ok()
                    } else {
                        let mut found = false;
                        runs::read_list::<Trigrams>(&scan, prefix, run, &list.gram, num, None, 1, &mut |p| {
                            found |= p.0 == num;
                        })?;
                        found
                    };
                    if found {
                        kept.push(num);
                    }
                }
                candidates = kept;
            } else {
                let nums = self.read_list(prefix, manifest, list)?;
                candidates.retain(|n| nums.binary_search(n).is_ok());
            }
        }
        Ok(candidates)
    }
}
