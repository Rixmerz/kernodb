//! Keyword search: a full-text inverted index ranked by BM25 (Phase B).
//!
//! # What is indexed
//!
//! The head payload of every `Active` unit whose bytes are UTF-8 text with at
//! least one term ([`tokens`]). An archived or deleted unit is not indexed,
//! so it is not found; unarchiving it indexes it again. Only the first
//! [`MAX_INDEXED_BYTES`] bytes of a payload are read.
//!
//! # Design
//!
//! Four record families, written by the same engine transaction that writes
//! the unit's head (the path of [`crate::type_count`]), so an index never
//! disagrees with the unit it describes, across a crash or a reopen. Every
//! indexed document has a dense *ordinal* (`ord: u32`), given in ascending
//! order from the `next_ord` of `STATS` and never given again: a unit keeps it
//! while it stays in the index (an update re-indexes it under the same one),
//! drops it when it leaves (delete, archive, text without a term) and gets a
//! new one when it comes back. The postings name the ordinal, as SQLite FTS5
//! names a rowid: a unit id is 16 bytes of which ~9 are random, an ordinal
//! delta is one or two (`docs/file-format/units-v12.md`).
//!
//! * `POSTING` the posting list of a term, cut in blocks of at most
//!   [`BLOCK_MAX`] postings (ADR-0026, the design of [`crate::blocks`]):
//!   `0x74 ‖ term ‖ 0x00 ‖ 0x00 ‖ bound_ord: u32 BE` or `0x74 ‖ term ‖ 0x00 ‖ 0x01`
//!   (the tail block) → `envelope[2] ‖ n × (varint(ord delta) ‖ varint(tf) ‖
//!   varint(dl))`, strictly ascending by ordinal, the first delta from 0: the
//!   frequency of `term` in the document and its length in terms (LEB128, in
//!   the shortest form). A term has no NUL, so the range of one term is
//!   `[tag ‖ term ‖ 0x00, tag ‖ term ‖ 0x01)`, and the block of an ordinal is
//!   the first key at or after `… ‖ 0x00 ‖ ord`. A query reads that range and
//!   nothing else per term.
//! * `DOC` `0x75 ‖ unit[16]` → `envelope[2] ‖ ord: u32 BE ‖ dl: u32 BE ‖ n: u32 BE ‖ n ×
//!   (len: u8 ‖ term)`: the ordinal, the length and the distinct terms the unit has in the
//!   index. A change reads it to know which postings to remove, so it never
//!   re-reads the old payload.
//! * `ORD` `0x7b ‖ ord: u32 BE` → `envelope[2] ‖ unit[16]`: the unit of an ordinal, written
//!   and deleted with the `DOC`.
//! * `STATS` `0x76` → `envelope[2] ‖ since: u64 BE ‖ docs: u64 BE ‖
//!   total_len: u64 BE ‖ next_ord: u64 BE`: the documents indexed and their summed length (the
//!   `N` and the average length of BM25), the commit sequence of the
//!   change set that wrote the record first, and the next ordinal to give. An
//!   ordinal past `u32::MAX` is refused (`InvalidArgument`), never wrapped.
//!
//! A unit without a term has no record at all, so a database that holds no
//! text is byte-for-byte what it was before this module.
//!
//! # Ranking
//!
//! BM25 with `k1 = 1.2`, `b = 0.75` and the non-negative idf
//! `ln(1 + (N - n + 0.5) / (n + 0.5))`. A query is cut by the same tokenizer
//! as the text; a unit's score is the sum over the distinct query terms it
//! holds (OR semantics). Hits come in descending score, ties by ascending
//! ordinal (the order the documents entered the index). Only the ordinals
//! returned (and, in a search restricted to types, those whose head is read)
//! are resolved to units, with one `multi_get` of their `ORD` records.
//!
//! # A write
//!
//! An operation plans its index change by reads only, before its first
//! write: the `DOC` of the unit says which terms it had. The writes of the
//! `DOC` record go with the unit's; the posting changes and the statistics
//! delta stay in the change set and are written at commit
//! ([`ChangeSet::flush_fulltext`]): each posting block they change is read and
//! written once per change set, whatever the number of its units, and the
//! `STATS` record once.
//!
//! # The layer mark
//!
//! The ordinal postings are layer version 12 (`docs/file-format/units-v12.md`),
//! the version every database starts at: a build refuses a database below it.
//!
//! `ponytail:` a query reads the whole posting list of each of its terms and
//! scores every unit in it (no top-k pruning of the lists); fine up to ~10^5
//! postings per term. Only the `limit` best are sorted and, in a search
//! restricted to some types, have their head read. A rebuild of legacy units
//! is not provided.

mod check;
mod tokenize;

#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap};
use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::db::NewUnit;
use crate::db::{ChangeSet, UnitDb, UnitSnapshot, Write, context, scan_all};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::payload::{ContentHash, PAYLOAD_CODEC_OPAQUE};
use crate::type_options::TAG_TYPE_OPTIONS;
use crate::types::{Lifecycle, UnitId};
use tokenize::{Doc, analyze};

pub(crate) use check::Check;
pub use tokenize::{MAX_INDEXED_BYTES, MAX_TERM_LEN, tokens};

/// Key tags of the full-text records.
pub const TAG_POSTING: u8 = 0x74;
pub const TAG_DOC: u8 = 0x75;
pub const TAG_STATS: u8 = 0x76;
/// `ORD` `0x7b ‖ ord: u32 BE` → `envelope[2] ‖ unit[16]`: the unit of an ordinal.
pub const TAG_ORD: u8 = 0x7b;

/// BM25 term-frequency saturation.
pub const K1: f64 = 1.2;
/// BM25 length normalisation.
pub const B: f64 = 0.75;

const RECORD_VERSION: u8 = 1;

/// Most postings in one block.
pub const BLOCK_MAX: usize = 128;

/// The dense number of an indexed document (`units-v12.md`).
pub(crate) type Ord = u32;

/// A unit a query found and its BM25 score (higher is a better match).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextHit {
    pub unit: UnitId,
    pub score: f64,
}

// ---------------------------------------------------------------- keys

const BOUNDED: u8 = 0x00;
const TAIL: u8 = 0x01;

/// `0x74 ‖ term ‖ 0x00`: the prefix of the blocks of one term's posting list.
pub(crate) fn list_key(term: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(term.len() + 2);
    k.push(TAG_POSTING);
    k.extend_from_slice(term.as_bytes());
    k.push(0);
    k
}

/// The key of the block of `list` with the bound `bound` (`None`: the tail).
fn list_block_key(list: &[u8], bound: Option<Ord>) -> Vec<u8> {
    let mut k = Vec::with_capacity(list.len() + 5);
    k.extend_from_slice(list);
    match bound {
        Some(o) => {
            k.push(BOUNDED);
            k.extend_from_slice(&o.to_be_bytes());
        }
        None => k.push(TAIL),
    }
    k
}

/// The key of the block of `term`'s list with the bound `bound` (`None`: the tail).
#[cfg(test)]
pub(crate) fn block_key(term: &str, bound: Option<Ord>) -> Vec<u8> {
    list_block_key(&list_key(term), bound)
}

/// Scan bounds `[start, end)` of the blocks of one term.
fn posting_range(term: &str) -> (Vec<u8>, Vec<u8>) {
    let mut start = vec![TAG_POSTING];
    start.extend_from_slice(term.as_bytes());
    let mut end = start.clone();
    start.push(0);
    end.push(1);
    (start, end)
}

pub(crate) fn doc_key(unit: UnitId) -> Vec<u8> {
    let mut k = Vec::with_capacity(17);
    k.push(TAG_DOC);
    k.extend_from_slice(&unit.0);
    k
}

/// `0x7b ‖ ord: u32 BE`.
pub(crate) fn ord_key(ord: Ord) -> [u8; 5] {
    let b = ord.to_be_bytes();
    [TAG_ORD, b[0], b[1], b[2], b[3]]
}

pub(crate) const STATS_KEY: [u8; 1] = [TAG_STATS];

fn valid_term(t: &[u8]) -> bool {
    (1..=MAX_TERM_LEN).contains(&t.len()) && !t.contains(&0) && std::str::from_utf8(t).is_ok()
}

/// The term and bound (`None`: the tail) of a posting block key.
pub(crate) fn decode_posting_key(key: &[u8]) -> Result<(String, Option<Ord>), DecodeError> {
    let n = key.len();
    let shape = || DecodeError::new(UnitRule::Unit3, format!("posting key of {n} bytes is not tag, term, NUL, bound"));
    let nul = key.iter().skip(1).position(|b| *b == 0).map(|p| p + 1).ok_or_else(shape)?;
    if key[0] != TAG_POSTING {
        return Err(shape());
    }
    let term = &key[1..nul];
    if !valid_term(term) {
        return Err(DecodeError::new(
            UnitRule::Unit12,
            "posting key term is empty, over 64 bytes, has a NUL or is not UTF-8",
        ));
    }
    let bound = match &key[nul + 1..] {
        [TAIL] => None,
        [BOUNDED, a, b, c, d] => Some(u32::from_be_bytes([*a, *b, *c, *d])),
        _ => return Err(shape()),
    };
    Ok((String::from_utf8_lossy(term).into_owned(), bound))
}

/// The unit of a `DOC` key.
pub(crate) fn decode_doc_key(key: &[u8]) -> Result<UnitId, DecodeError> {
    match key.get(1..).and_then(|b| <[u8; 16]>::try_from(b).ok()) {
        Some(unit) if key[0] == TAG_DOC => Ok(UnitId(unit)),
        _ => Err(DecodeError::new(UnitRule::Unit3, format!("doc key of {} bytes is not tag, unit", key.len()))),
    }
}

/// The ordinal of an `ORD` key.
pub(crate) fn decode_ord_key(key: &[u8]) -> Result<Ord, DecodeError> {
    match key {
        [TAG_ORD, a, b, c, d] => Ok(u32::from_be_bytes([*a, *b, *c, *d])),
        _ => Err(DecodeError::new(UnitRule::Unit3, format!("ord key of {} bytes is not tag, ord", key.len()))),
    }
}

// -------------------------------------------------------------- values

/// The bytes after the envelope: the version first (a newer one is a newer
/// layer, not damage), then the flags.
fn body<'a>(v: &'a [u8], what: &str) -> Result<&'a [u8], DecodeError> {
    let (Some(&version), Some(&flags)) = (v.first(), v.get(1)) else {
        return Err(DecodeError::new(
            UnitRule::Unit9,
            format!("{what} has {} bytes, fewer than its envelope", v.len()),
        ));
    };
    match version {
        0 => return Err(DecodeError::new(UnitRule::Unit6, format!("{what} envelope_version 0"))),
        RECORD_VERSION => {}
        n => return Err(DecodeError::new(UnitRule::Unit7, format!("{what} envelope_version {n}"))),
    }
    if flags != 0 {
        return Err(DecodeError::new(UnitRule::Unit8, format!("{what} reserved_flags {flags:#04x}")));
    }
    Ok(&v[2..])
}

/// `body` of exactly `N` bytes.
fn fixed<'a, const N: usize>(v: &'a [u8], what: &str) -> Result<&'a [u8; N], DecodeError> {
    let b = body(v, what)?;
    b.try_into().map_err(|_| {
        let rule = if b.len() < N { UnitRule::Unit9 } else { UnitRule::Unit10 };
        DecodeError::new(rule, format!("{what} has {} bytes after its envelope, expected {N}", b.len()))
    })
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn be64(b: &[u8]) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    u64::from_be_bytes(a)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Posting {
    pub tf: u32,
    pub dl: u32,
}

fn put_varint(out: &mut Vec<u8>, mut n: u32) {
    while n >= 0x80 {
        out.push((n as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// A LEB128 `u32` off the front of `b`, in its shortest form.
fn take_varint(b: &mut &[u8]) -> Option<u32> {
    let mut n: u64 = 0;
    for i in 0..5 {
        let (&byte, rest) = b.split_first()?;
        *b = rest;
        n |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            // Not the shortest form (a trailing zero group), or over u32.
            if (i > 0 && byte == 0) || n > u64::from(u32::MAX) {
                return None;
            }
            return Some(n as u32);
        }
    }
    None
}

/// The postings of a block value: at least one, at most [`BLOCK_MAX`], strictly ascending by
/// ordinal.
pub(crate) fn decode_block(v: &[u8]) -> Result<Vec<(Ord, Posting)>, DecodeError> {
    let e = |rule, m: &str| DecodeError::new(rule, format!("posting block: {m}"));
    let mut b = crate::blocks::body(v, "posting block")?;
    if b.is_empty() {
        return Err(e(UnitRule::Unit9, "no posting"));
    }
    let mut out: Vec<(Ord, Posting)> = Vec::with_capacity(16);
    while !b.is_empty() {
        if out.len() == BLOCK_MAX {
            return Err(e(UnitRule::Unit11, "more postings than a block holds"));
        }
        let (Some(d), Some(tf), Some(dl)) = (take_varint(&mut b), take_varint(&mut b), take_varint(&mut b)) else {
            return Err(e(UnitRule::Unit9, "a partial or malformed posting"));
        };
        let ord = match out.last() {
            None => d,
            Some(_) if d == 0 => return Err(e(UnitRule::Unit13, "ordinals not strictly ascending")),
            Some((prev, _)) => prev.checked_add(d).ok_or_else(|| e(UnitRule::Unit12, "an ordinal over u32"))?,
        };
        if tf == 0 || dl < tf {
            return Err(e(UnitRule::Unit12, &format!("tf {tf} in a document of {dl} terms")));
        }
        out.push((ord, Posting { tf, dl }));
    }
    Ok(out)
}

fn encode_block(entries: &[(Ord, Posting)]) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + 6 * entries.len());
    v.extend_from_slice(&[RECORD_VERSION, 0]);
    let mut prev = None;
    for (ord, p) in entries {
        put_varint(&mut v, prev.map_or(*ord, |p| ord - p));
        put_varint(&mut v, p.tf);
        put_varint(&mut v, p.dl);
        prev = Some(*ord);
    }
    v
}

fn encode_doc(ord: Ord, doc: &Doc) -> Vec<u8> {
    let mut v = Vec::with_capacity(14 + doc.term_bytes().len() + doc.text_len());
    v.extend_from_slice(&[RECORD_VERSION, 0]);
    v.extend_from_slice(&ord.to_be_bytes());
    v.extend_from_slice(&doc.len.to_be_bytes());
    v.extend_from_slice(&(doc.term_bytes().len() as u32).to_be_bytes());
    for t in doc.term_bytes() {
        v.push(t.len() as u8);
        v.extend_from_slice(t);
    }
    v
}

/// A decoded `DOC` value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DocRecord {
    pub ord: Ord,
    pub dl: u32,
    pub terms: BTreeSet<String>,
}

/// The ordinal, the length and the distinct terms of a `DOC` value.
pub(crate) fn decode_doc(v: &[u8]) -> Result<DocRecord, DecodeError> {
    let b = body(v, "doc record")?;
    if b.len() < 12 {
        return Err(DecodeError::new(UnitRule::Unit9, format!("doc record has {} bytes after its envelope", b.len())));
    }
    let (ord, dl, n) = (be32(&b[..4]), be32(&b[4..8]), be32(&b[8..12]));
    let mut rest = &b[12..];
    let mut terms = BTreeSet::new();
    for _ in 0..n {
        let Some((&len, tail)) = rest.split_first() else {
            return Err(DecodeError::new(UnitRule::Unit9, "doc record ends before its last term"));
        };
        let len = usize::from(len);
        if tail.len() < len {
            return Err(DecodeError::new(UnitRule::Unit9, "doc record ends inside a term"));
        }
        let (term, tail) = tail.split_at(len);
        if !valid_term(term) {
            return Err(DecodeError::new(
                UnitRule::Unit12,
                "doc record term is empty, over 64 bytes, has a NUL or is not UTF-8",
            ));
        }
        terms.insert(String::from_utf8_lossy(term).into_owned());
        rest = tail;
    }
    if !rest.is_empty() {
        return Err(DecodeError::new(UnitRule::Unit10, format!("doc record has {} trailing bytes", rest.len())));
    }
    if terms.len() != n as usize {
        return Err(DecodeError::new(UnitRule::Unit12, "doc record repeats a term"));
    }
    Ok(DocRecord { ord, dl, terms })
}

fn encode_ord(unit: UnitId) -> Vec<u8> {
    let mut v = Vec::with_capacity(18);
    v.extend_from_slice(&[RECORD_VERSION, 0]);
    v.extend_from_slice(&unit.0);
    v
}

/// The unit of an `ORD` value.
pub(crate) fn decode_ord(v: &[u8]) -> Result<UnitId, DecodeError> {
    Ok(UnitId(*fixed::<16>(v, "ord record")?))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stats {
    pub since: u64,
    pub docs: u64,
    pub total_len: u64,
    /// The next ordinal to allocate: above every ordinal ever given.
    pub next_ord: u64,
}

fn encode_stats(s: Stats) -> Vec<u8> {
    let mut v = vec![RECORD_VERSION, 0];
    for n in [s.since, s.docs, s.total_len, s.next_ord] {
        v.extend_from_slice(&n.to_be_bytes());
    }
    v
}

pub(crate) fn decode_stats(v: &[u8]) -> Result<Stats, DecodeError> {
    let b = fixed::<32>(v, "stats record")?;
    Ok(Stats { since: be64(&b[..8]), docs: be64(&b[8..16]), total_len: be64(&b[16..24]), next_ord: be64(&b[24..]) })
}

// -------------------------------------------------------------- writer

/// What an operation does to the content of a unit, for the indexes of its
/// payload (this one and [`crate::substring`]).
pub(crate) enum Next<'a> {
    /// The writes do not change the unit's content (retention drops history).
    Unchanged,
    /// The unit's type and its indexed payload afterwards; `None` when it has
    /// none (the unit is not `Active`).
    Content(&'a str, Option<&'a [u8]>),
}

/// The unit's new head and payload, as [`crate::db::planned_head`] reads them off the writes of
/// its operation (`None`: they write no head).
pub(crate) fn next_of<'a>(planned: Option<&'a crate::db::PlannedHead<'a>>) -> Next<'a> {
    let Some((head, payload)) = planned else {
        return Next::Unchanged;
    };
    if head.lifecycle != Lifecycle::Active {
        return Next::Content(&head.unit_type, None);
    }
    payload.map_or(Next::Unchanged, |p| Next::Content(&head.unit_type, Some(p)))
}

/// What a change set will change in the posting lists: per term, the postings
/// to put (`Some`) or remove (`None`) by unit, the last operation of a unit
/// winning.
#[derive(Default)]
pub(crate) struct PendingPostings {
    /// The terms staged, numbered.
    terms: tokenize::Interner,
    /// The operations by term number, in staging order.
    lists: Vec<Vec<(Ord, Option<Posting>)>>,
}

/// The index change of one operation that is not a record of its own: the
/// postings the unit gives up and gains, and what its document does to the
/// statistics.
pub(crate) struct TextChange {
    ord: Ord,
    gone: Vec<String>,
    /// The new document: a posting of each of its terms.
    put: Option<Doc>,
    /// The change of the number of documents and of their summed length.
    docs: i64,
    len: i64,
}

/// What an operation does to the full-text index: the records it writes with
/// the unit (the `DOC`) and the change kept for commit.
#[derive(Default)]
pub(crate) struct TextPlan {
    pub writes: Vec<Write>,
    pub change: Option<TextChange>,
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// The index change that follows the operation `writes` on `unit`, planned
    /// by reads only (like [`ChangeSet::plan_live`](crate::type_count)):
    /// call it before the first write of the operation, so a refusal leaves
    /// no index entry without its head. The caller writes the plan's `writes`
    /// with the operation's and hands the plan to
    /// [`stage_fulltext`](Self::stage_fulltext) after.
    pub(crate) fn plan_fulltext(&self, unit: UnitId, writes: &[Write]) -> Result<TextPlan, UnitError> {
        let next = crate::db::planned_head(unit, writes)?;
        self.plan_fulltext_at(unit, next.as_ref())
    }

    /// [`plan_fulltext`](Self::plan_fulltext) of the head and payload the writes put, read off
    /// them already ([`crate::db::planned_head`]).
    pub(crate) fn plan_fulltext_at(
        &self,
        unit: UnitId,
        next: Option<&crate::db::PlannedHead<'_>>,
    ) -> Result<TextPlan, UnitError> {
        let Next::Content(unit_type, payload) = next_of(next) else {
            return Ok(TextPlan::default());
        };
        // The index is opt-in per type (ADR-0023), and turning it off removes the
        // entries of the type's units in that same change set: a unit of a type
        // that is not indexed has no document to add or to remove. Decided
        // before the payload is cut into terms, which most writes never need.
        if !self.fulltext_enabled(unit_type)? {
            return Ok(TextPlan::default());
        }
        self.plan_document(unit, payload)
    }

    /// The index change that makes the text of `payload` the document of
    /// `unit` (`None`: the unit has no document), whatever its type's option:
    /// what [`ChangeSet::set_fulltext`](crate::type_options) needs to index or
    /// clear the units a type already has.
    pub(crate) fn plan_document(&self, unit: UnitId, payload: Option<&[u8]>) -> Result<TextPlan, UnitError> {
        // A payload cut into terms ahead of its operation (`create_prepared`), else here.
        let prepared = self.prepared_text.borrow_mut().take().filter(|p| Some(p.len) == payload.map(<[u8]>::len));
        let (new, record) = match prepared {
            Some(p) => (p.doc, p.record),
            None => (payload.and_then(analyze), None),
        };
        // A unit with an id above every stored one that has no revision in this change set yet
        // (one being created) has no document anywhere: no read.
        let fresh = self.fresh_from().is_some_and(|f| unit >= f) && !self.is_touched(unit);
        let old = match if fresh { None } else { self.txn()?.get(&doc_key(unit))? } {
            Some(raw) => Some(decode_doc(&raw).map_err(|e| context(e, &format!("DOC of unit {unit}")))?),
            None => None,
        };
        if old.is_none() && new.is_none() {
            return Ok(TextPlan::default());
        }
        if old.is_none() {
            // A unit the index takes in for the first time: maybe a new unit of a bulk load.
            self.learn_fresh(unit)?;
        }
        // Only the postings the new text no longer has go; the others are overwritten.
        let gone = old
            .iter()
            .flat_map(|o| &o.terms)
            .filter(|t| !new.as_ref().is_some_and(|d| d.get(t).is_some()))
            .cloned()
            .collect();
        let (mut docs, mut len) = (0i64, 0i64);
        if let Some(o) = &old {
            docs -= 1;
            len -= i64::from(o.dl);
        }
        // A unit keeps its ordinal while it stays in the index; one that comes in gets the next.
        let ord = match &old {
            Some(o) => o.ord,
            None => self.allocate_text_ord()?,
        };
        let mut writes = Vec::with_capacity(2);
        let put = match new {
            Some(doc) => {
                docs += 1;
                len += i64::from(doc.len);
                let record = match record {
                    Some(r) => with_ord(r, ord),
                    None => encode_doc(ord, &doc),
                };
                writes.push(Write::Put(doc_key(unit), record));
                if old.is_none() {
                    writes.push(Write::Put(ord_key(ord).to_vec(), encode_ord(unit)));
                }
                Some(doc)
            }
            None => {
                writes.push(Write::Delete(doc_key(unit)));
                writes.push(Write::Delete(ord_key(ord).to_vec()));
                None
            }
        };
        Ok(TextPlan { writes, change: Some(TextChange { ord, gone, put, docs, len }) })
    }

    /// The next ordinal of the keyword index: the `next_ord` of `STATS` (read once per change
    /// set), then counting up. A change set that is refused or aborted leaves a gap, never a
    /// reuse. Past `u32::MAX` the write is refused.
    fn allocate_text_ord(&self) -> Result<Ord, UnitError> {
        let (base, next) = match self.text_ords.get() {
            Some(c) => c,
            None => {
                let n = match self.txn()?.get(&STATS_KEY)? {
                    Some(raw) => decode_stats(&raw).map_err(|e| context(e, "FULLTEXT_STATS"))?.next_ord,
                    None => 0,
                };
                (n, n)
            }
        };
        let ord = Ord::try_from(next).map_err(|_| {
            UnitError::InvalidArgument(
                "the keyword index has given out every document ordinal (2^32): export the database and \
                 import it to number its documents again"
                    .into(),
            )
        })?;
        self.text_ords.set(Some((base, next + 1)));
        Ok(ord)
    }

    /// Keeps the posting and statistics changes of an operation planned by
    /// [`plan_document`](Self::plan_document), for [`flush_fulltext`](Self::flush_fulltext).
    pub(crate) fn stage_fulltext(&mut self, change: Option<TextChange>) {
        let Some(c) = change else { return };
        for term in c.gone {
            let hash = tokenize::term_hash(term.as_bytes());
            stage(&mut self.text_pending, &term, hash, c.ord, None);
        }
        if let Some(doc) = &c.put {
            for (term, tf, hash) in doc.terms_hashed() {
                stage(&mut self.text_pending, term, hash, c.ord, Some(Posting { tf, dl: doc.len }));
            }
        }
        let (docs, len) = self.text_stats.get_or_insert((0, 0));
        *docs += c.docs;
        *len += c.len;
    }

    /// Writes the posting changes and the statistics delta the change set
    /// kept: each posting block a changed unit falls in is read once and
    /// written once, and the statistics record is written once. Called by
    /// `commit`.
    pub(crate) fn flush_fulltext(&mut self) -> Result<(), UnitError> {
        // Terms in ascending order, as the probe needs: a term's list key is the term and a
        // NUL, and a term has no NUL, so the keys order like the terms and none is a prefix of
        // another.
        let mut probe = crate::blocks::Probe::new(vec![TAG_POSTING + 1]);
        let ords = self.text_ords.get();
        // Every ordinal at or above the first this change set gave is above every stored posting.
        let above = ords.map(|(base, _)| base);
        let PendingPostings { terms, mut lists } = std::mem::take(&mut self.text_pending);
        // One buffer for the list keys: `list_key` of each term, built from its bytes.
        let mut list = Vec::new();
        for i in terms.ascending() {
            list.clear();
            list.push(TAG_POSTING);
            list.extend_from_slice(terms.get(i as usize));
            list.push(0);
            let mut ops = std::mem::take(&mut lists[i as usize]);
            // Ascending by ordinal; of several operations on one the last wins. New documents are
            // staged in ordinal order already: nothing to sort then.
            if !ops.windows(2).all(|w| w[0].0 < w[1].0) {
                ops.reverse();
                ops.sort_by_key(|(o, _)| *o);
                ops.dedup_by_key(|(o, _)| *o);
            }
            flush_postings(self, &list, ops, &mut probe, above)?;
        }
        let gave = ords.is_some_and(|(base, next)| next > base);
        let (docs, len) = match self.text_stats.take() {
            Some(d) => d,
            None if gave => (0, 0),
            None => return Ok(()),
        };
        let mut stats = match self.txn()?.get(&STATS_KEY)? {
            Some(raw) => decode_stats(&raw).map_err(|e| context(e, "FULLTEXT_STATS"))?,
            None => Stats { since: self.commit_seq().0, docs: 0, total_len: 0, next_ord: 0 },
        };
        // ponytail: saturates like the counts of `type_count`; a statistic that
        // was already wrong stays wrong and `verify_units` reports it (UNIT-49).
        let add = |n: u64, d: i64| {
            if d >= 0 { n.saturating_add(d.unsigned_abs()) } else { n.saturating_sub(d.unsigned_abs()) }
        };
        stats.docs = add(stats.docs, docs);
        stats.total_len = add(stats.total_len, len);
        if let Some((_, next)) = ords {
            stats.next_ord = stats.next_ord.max(next);
        }
        self.write_owned(Write::Put(STATS_KEY.to_vec(), encode_stats(stats)))
    }
}

/// Writes the changes `ops` (ascending by ordinal, one per ordinal: `Some` puts the posting,
/// `None` removes it) to the posting list `list`, as [`crate::blocks::flush`] does for blocks
/// keyed by a 16-byte id: each block an ordinal falls in is read once and written once, cut
/// from the low end when over [`BLOCK_MAX`] and deleted when emptied. `above`: every ordinal at
/// or above it is above every stored posting.
fn flush_postings<E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    list: &[u8],
    ops: Vec<(Ord, Option<Posting>)>,
    probe: &mut crate::blocks::Probe,
    above: Option<u64>,
) -> Result<(), UnitError> {
    let Some(&(first, _)) = ops.first() else {
        return Ok(());
    };
    let tail_key = list_block_key(list, None);
    let (_, end) = crate::schema::prefix_bounds(list);
    // The tail holds every ordinal above the last bound, and ordinals are given in ascending
    // order: the postings a change set adds are almost always in it, found by one point read.
    let mut tail = match cs.txn()?.get(&tail_key)? {
        // A tail that does not decode is not used here: the seek reads it if the change needs
        // it, and reports it then.
        Some(v) => decode_block(&v).ok(),
        None => {
            // No tail: postings above every stored one go to a new tail, and so do any when the
            // list has no key at all.
            if above.is_some_and(|a| a <= u64::from(first)) || probe.is_empty(cs, list, &end)? {
                let puts = ops.into_iter().filter_map(|(o, v)| v.map(|v| (o, v))).collect();
                return write_postings(cs, list, puts, tail_key);
            }
            None
        }
    };
    let mut ops = ops.into_iter().peekable();
    while let Some(&(first, _)) = ops.peek() {
        let (block, mut entries, bound) = match tail.take_if(|t| t.first().is_some_and(|e| e.0 <= first)) {
            Some(t) => (Some(tail_key.clone()), t, None),
            None => {
                let from = list_block_key(list, Some(first));
                let page = cs.txn()?.scan(Bound::Included(&from), Bound::Excluded(&end), 1)?;
                match page.into_iter().next() {
                    Some((k, v)) => {
                        let entries = decode_block(&v).map_err(|e| context(e, "POSTING"))?;
                        let (_, bound) = decode_posting_key(&k).map_err(|e| context(e, "POSTING"))?;
                        (Some(k), entries, bound)
                    }
                    None => (None, Vec::new(), None),
                }
            }
        };
        while let Some((ord, op)) = ops.next_if(|(o, _)| bound.is_none_or(|b| *o <= b)) {
            match (entries.binary_search_by_key(&ord, |e| e.0), op) {
                (Err(at), Some(p)) => entries.insert(at, (ord, p)),
                (Ok(at), Some(p)) => entries[at].1 = p,
                (Ok(at), None) => {
                    entries.remove(at);
                }
                (Err(_), None) => {}
            }
        }
        let key = block.clone().unwrap_or_else(|| list_block_key(list, bound));
        if entries.is_empty() {
            if block.is_some() {
                cs.write_owned(Write::Delete(key))?;
            }
            continue;
        }
        write_postings(cs, list, entries, key)?;
    }
    Ok(())
}

/// Writes `entries` as blocks of at most [`BLOCK_MAX`]: full blocks from the low end, each
/// bounded by its last ordinal, and the rest under `last_key`.
fn write_postings<E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    list: &[u8],
    entries: Vec<(Ord, Posting)>,
    mut last_key: Vec<u8>,
) -> Result<(), UnitError> {
    let mut chunks = entries.chunks(BLOCK_MAX).peekable();
    while let Some(chunk) = chunks.next() {
        let k = if chunks.peek().is_some() {
            list_block_key(list, chunk.last().map(|e| e.0))
        } else {
            std::mem::take(&mut last_key)
        };
        cs.write_owned(Write::Put(k, encode_block(chunk)))?;
    }
    Ok(())
}

/// A payload cut into terms ahead of the operation that writes it, by
/// [`ChangeSet::prepare_texts`]: the operation then does not cut it again.
pub struct PreparedText {
    len: usize,
    doc: Option<Doc>,
    /// `encode_doc(0, doc)`: the `DOC` record with ordinal 0, encoded where the text is cut
    /// (maybe on another thread). The ordinal is given later and only those bytes change
    /// ([`with_ord`]).
    record: Option<Vec<u8>>,
}

impl PreparedText {
    fn new(len: usize, doc: Option<Doc>) -> Self {
        let record = doc.as_ref().map(|d| encode_doc(0, d));
        PreparedText { len, doc, record }
    }
}

/// Where the ordinal sits in a `DOC` record (after the envelope's two bytes).
const DOC_ORD_AT: usize = 2;

/// A `DOC` record encoded with ordinal 0, given the ordinal `ord`: `encode_doc(ord, doc)`.
fn with_ord(mut record: Vec<u8>, ord: Ord) -> Vec<u8> {
    record[DOC_ORD_AT..DOC_ORD_AT + 4].copy_from_slice(&ord.to_be_bytes());
    record
}

/// What [`ChangeSet::prepare_texts`] computes of one payload ahead of its create: its content
/// hash and, when its type is keyword-indexed, its terms.
pub struct PreparedPayload {
    len: usize,
    hash: ContentHash,
    text: Option<PreparedText>,
}

/// Fewer payloads than this are cut on the calling thread.
const PREPARE_PARALLEL_MIN: usize = 64;

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// Hashes the payloads of `items` (`(unit_type, payload)`) and cuts into terms those whose
    /// type has a keyword index, on several threads when there are many, for
    /// [`create_prepared`](Self::create_prepared): the same hashes and documents the creates
    /// would make one by one, so the stored bytes are the same.
    ///
    /// # Errors
    ///
    /// The change set is closed, or a type-options record is damaged.
    pub fn prepare_texts(&self, items: &[(&str, &[u8])]) -> Result<Vec<PreparedPayload>, UnitError> {
        let mut wanted = Vec::with_capacity(items.len());
        for (unit_type, payload) in items {
            wanted.push((self.fulltext_enabled(unit_type)?, *payload));
        }
        let cut = |part: &[(bool, &[u8])]| -> Vec<PreparedPayload> {
            // One counter for the texts of the part: its table is allocated once.
            let mut counter = tokenize::Counter::default();
            part.iter()
                .map(|&(w, p)| PreparedPayload {
                    len: p.len(),
                    hash: ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, p),
                    text: w.then(|| PreparedText::new(p.len(), tokenize::analyze_with(&mut counter, p))),
                })
                .collect()
        };
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        if threads == 1 || wanted.len() < PREPARE_PARALLEL_MIN {
            return Ok(cut(&wanted));
        }
        let per = wanted.len().div_ceil(threads);
        Ok(std::thread::scope(|scope| {
            let parts: Vec<_> = wanted.chunks(per).map(|part| scope.spawn(move || cut(part))).collect();
            // A panic of `analyze` on a worker is a panic of the caller, as it would have been.
            parts.into_iter().flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
        }))
    }

    /// [`create`](Self::create), with the payload already hashed and cut into terms by
    /// [`prepare_texts`](Self::prepare_texts) (`None`: do it here, as `create` does).
    ///
    /// # Errors
    ///
    /// As [`create`](Self::create).
    pub fn create_prepared(&mut self, new: NewUnit, prepared: Option<PreparedPayload>) -> Result<UnitId, UnitError> {
        if let Some(p) = prepared {
            self.prepared_hash.set(Some((p.len, p.hash)));
            *self.prepared_text.get_mut() = p.text;
        }
        let created = self.create(new);
        *self.prepared_text.get_mut() = None;
        self.prepared_hash.set(None);
        created
    }
}

/// Records that `unit` has `value` (`None`: no posting) in the list of `term`. The term is
/// copied only the first time the change set stages it.
fn stage(pending: &mut PendingPostings, term: &str, hash: u64, ord: Ord, value: Option<Posting>) {
    match pending.terms.intern_hashed(term.as_bytes(), hash) {
        (_, true) => {
            // Most terms of a load are staged by several documents: no growth from one.
            let mut list = Vec::with_capacity(4);
            list.push((ord, value));
            pending.lists.push(list);
        }
        (i, false) => pending.lists[i].push((ord, value)),
    }
}

// --------------------------------------------------------------- query

impl<E: Engine> UnitSnapshot<E> {
    /// Whether any unit type has keyword indexing on (`set_fulltext`). The index is opt-in per
    /// type, so a search with none on can never find anything.
    pub fn has_keyword_index(&self) -> Result<bool, UnitError> {
        let (options, after) = ([TAG_TYPE_OPTIONS], [TAG_TYPE_OPTIONS + 1]);
        Ok(!self.snap.scan(Bound::Included(&options), Bound::Excluded(&after), 1)?.is_empty())
    }

    /// Whether `unit_type` has ever been used: a unit, a schema or a type option names it.
    /// Types are implicit, so this is the only meaning "the type exists" can have.
    pub(crate) fn type_known(&self, unit_type: &str) -> Result<bool, UnitError> {
        Ok(crate::type_count::expected_one(&self.snap, unit_type)?.is_some()
            || self.snap.get(&crate::type_options::options_key(unit_type))?.is_some()
            || self.schema(unit_type)?.is_some())
    }

    /// The units whose text matches `query`, best first, at most `limit`, optionally only those
    /// of the unit types in `types` (`None` searches every keyword-indexed type).
    ///
    /// The query is cut into terms like the indexed text and a unit's score
    /// is the BM25 sum over the distinct terms it holds (see the [module
    /// documentation](self)); a unit with none of them is not a hit. The statistics behind the
    /// score (`N`, average length) are those of the whole index, so a unit scores the same with
    /// or without `types`. Ties are in ascending ordinal: the order the units entered the index.
    /// Reads only the posting ranges of the query terms, the statistics record, the `ORD` records
    /// of the hits returned (one `multi_get`) and, with `types`, the heads of the best-scored
    /// units until `limit` of them are of a wanted type.
    ///
    /// A search that could never match is an error, not an empty answer: see below. A database
    /// where the query simply finds nothing answers an empty list.
    ///
    /// # Errors
    ///
    /// * `InvalidArgument` for `limit == 0`, a query with no term (empty, or only punctuation),
    ///   `Some(&[])` as `types`, or an invalid type name.
    /// * `NotIndexed` when no type is keyword-indexed (`set_fulltext`), or when a type in `types`
    ///   is not ([`has_keyword_index`](Self::has_keyword_index) tells the first case apart).
    /// * `UnknownType` for a type in `types` that no unit, schema or option ever named.
    /// * `Storage(Corruption)` when a record of the index does not decode.
    pub fn search_text(&self, query: &str, limit: usize, types: Option<&[&str]>) -> Result<Vec<TextHit>, UnitError> {
        let invalid = |m: &str| UnitError::InvalidArgument(format!("search_text: {m}"));
        if limit == 0 {
            return Err(invalid("limit must be at least 1"));
        }
        let terms: BTreeSet<String> = tokens(query).collect();
        if terms.is_empty() {
            return Err(invalid("the query has no term (it is empty or holds only punctuation)"));
        }
        if types.is_some_and(<[&str]>::is_empty) {
            return Err(invalid("types is empty: give at least one unit type, or no restriction"));
        }
        for t in types.unwrap_or_default() {
            if t.is_empty() || t.len() > crate::codec::MAX_UNIT_TYPE_LEN as usize {
                return Err(invalid("a unit type name is empty or longer than 64 bytes"));
            }
        }
        if !self.has_keyword_index()? {
            return Err(UnitError::NotIndexed {
                detail: "no unit type is keyword-indexed: turn it on for a type with set_fulltext".into(),
            });
        }
        for t in types.unwrap_or_default() {
            if !self.type_known(t)? {
                return Err(UnitError::UnknownType { unit_type: (*t).to_string() });
            }
            if !self.fulltext_enabled(t)? {
                return Err(UnitError::NotIndexed {
                    detail: format!("unit type {t:?} is not keyword-indexed: turn it on with set_fulltext"),
                });
            }
        }
        let Some(raw) = self.snap.get(&STATS_KEY)? else {
            return Ok(Vec::new());
        };
        let stats = decode_stats(&raw).map_err(|e| context(e, "FULLTEXT_STATS"))?;
        if stats.docs == 0 {
            return Ok(Vec::new());
        }
        let n_docs = stats.docs as f64;
        let avg_len = (stats.total_len as f64 / n_docs).max(1.0);
        let mut scores: HashMap<Ord, f64> = HashMap::new();
        for term in &terms {
            let (start, end) = posting_range(term);
            let mut postings = Vec::new();
            scan_all(&self.snap, start, end, |_, v| {
                postings.extend(decode_block(v).map_err(|e| context(e, "POSTING"))?);
                Ok(())
            })?;
            let df = postings.len() as f64;
            let idf = (1.0 + ((n_docs - df).max(0.0) + 0.5) / (df + 0.5)).ln();
            for (ord, p) in postings {
                let (tf, dl) = (f64::from(p.tf), f64::from(p.dl));
                *scores.entry(ord).or_insert(0.0) += idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / avg_len));
            }
        }
        let mut hits: Vec<(Ord, f64)> = scores.into_iter().collect();
        let best_first = |a: &(Ord, f64), b: &(Ord, f64)| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0));
        // The best hits are taken off the front a slice at a time, so a search sorts (and
        // resolves to units, and with `types` reads the head of) about `limit` documents, not
        // every one that holds a term.
        let mut out = Vec::with_capacity(limit.min(hits.len()));
        let mut from = 0;
        while out.len() < limit && from < hits.len() {
            let rest = &mut hits[from..];
            let n = (limit - out.len()).min(rest.len());
            if n < rest.len() {
                rest.select_nth_unstable_by(n - 1, best_first);
            }
            rest[..n].sort_unstable_by(best_first);
            let keys: Vec<[u8; 5]> = rest[..n].iter().map(|(o, _)| ord_key(*o)).collect();
            let refs: Vec<&[u8]> = keys.iter().map(|k| &k[..]).collect();
            let units = self.snap.multi_get(&refs)?;
            for (&(ord, score), raw) in rest[..n].iter().zip(units) {
                let Some(raw) = raw else {
                    return Err(UnitError::Storage(kerno_core::Error::Corruption(format!(
                        "a POSTING of ordinal {ord} has no ORD record"
                    ))));
                };
                let unit = decode_ord(&raw).map_err(|e| context(e, "ORD"))?;
                if let Some(types) = types {
                    let Some(head) = crate::history::read_head(&|k| self.snap.get(k), unit)? else {
                        return Err(UnitError::Storage(kerno_core::Error::Corruption(format!(
                            "a POSTING of unit {unit} has no UNIT_HEAD"
                        ))));
                    };
                    if !types.contains(&head.unit_type.as_str()) {
                        continue;
                    }
                }
                out.push(TextHit { unit, score });
            }
            from += n;
        }
        Ok(out)
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// [`UnitSnapshot::search_text`] on a snapshot of the latest commit.
    pub fn search_text(&self, query: &str, limit: usize, types: Option<&[&str]>) -> Result<Vec<TextHit>, UnitError> {
        self.snapshot()?.search_text(query, limit, types)
    }
}
