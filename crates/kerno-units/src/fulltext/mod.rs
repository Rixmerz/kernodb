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
//! Three record families, written by the same engine transaction that writes
//! the unit's head (the path of [`crate::type_count`]), so an index never
//! disagrees with the unit it describes, across a crash or a reopen:
//!
//! * `POSTING` the posting list of a term, cut in blocks of at most
//!   [`BLOCK_MAX`] postings (ADR-0026, the design of [`crate::substring`]):
//!   `0x74 ‖ term ‖ 0x00 ‖ 0x00 ‖ bound[16]` or `0x74 ‖ term ‖ 0x00 ‖ 0x01`
//!   (the tail block) → `envelope[2] ‖ (unit[16] ‖ tf: u32 BE ‖ dl: u32 BE) × n`,
//!   ascending by unit: the frequency of `term` in the unit and the unit's
//!   length in terms. A term has no NUL, so the range of one term is
//!   `[tag ‖ term ‖ 0x00, tag ‖ term ‖ 0x01)`, and the block of a unit is the
//!   first key at or after `… ‖ 0x00 ‖ unit`. A query reads that range and
//!   nothing else per term. One record per term and unit, the first design
//!   (ADR-0023), made a batch of units write one record per distinct term of
//!   each: the per-record cost of the engine was the top of the profile of the
//!   keyword-indexed store (ADR-0026).
//! * `DOC` `0x75 ‖ unit[16]` → `envelope[2] ‖ dl: u32 BE ‖ n: u32 BE ‖ n ×
//!   (len: u8 ‖ term)`: the length and the distinct terms the unit has in the
//!   index. A change reads it to know which postings to remove, so it never
//!   re-reads the old payload.
//! * `STATS` `0x76` → `envelope[2] ‖ since: u64 BE ‖ docs: u64 BE ‖
//!   total_len: u64 BE`: the documents indexed and their summed length (the
//!   `N` and the average length of BM25), and the commit sequence of the
//!   change set that wrote the record first.
//!
//! A unit without a term has no record at all, so a database that holds no
//! text is byte-for-byte what it was before this module.
//!
//! # Ranking
//!
//! BM25 with `k1 = 1.2`, `b = 0.75` and the non-negative idf
//! `ln(1 + (N - n + 0.5) / (n + 0.5))`. A query is cut by the same tokenizer
//! as the text; a unit's score is the sum over the distinct query terms it
//! holds (OR semantics). Hits come in descending score, ties by unit id.
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
//! The posting blocks are layer version 9 (`docs/file-format/units-v9.md`),
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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::codec::{Key, UnitHead};
use crate::db::{ChangeSet, UnitDb, UnitSnapshot, Write, context, scan_all};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::type_options::TAG_TYPE_OPTIONS;
use crate::types::{Lifecycle, UnitId};
use tokenize::{Doc, analyze};

pub(crate) use check::Check;
pub use tokenize::{MAX_INDEXED_BYTES, MAX_TERM_LEN, tokens};

/// Key tags of the full-text records.
pub const TAG_POSTING: u8 = 0x74;
pub const TAG_DOC: u8 = 0x75;
pub const TAG_STATS: u8 = 0x76;

/// BM25 term-frequency saturation.
pub const K1: f64 = 1.2;
/// BM25 length normalisation.
pub const B: f64 = 0.75;

const RECORD_VERSION: u8 = 1;

/// Most postings in one block.
pub const BLOCK_MAX: usize = 128;
/// `unit[16] ‖ tf ‖ dl`.
const ENTRY_LEN: usize = 24;

/// A unit a query found and its BM25 score (higher is a better match).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextHit {
    pub unit: UnitId,
    pub score: f64,
}

// ---------------------------------------------------------------- keys

/// `0x74 ‖ term ‖ 0x00`: the prefix of the blocks of one term's posting list.
pub(crate) fn list_key(term: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(term.len() + 2);
    k.push(TAG_POSTING);
    k.extend_from_slice(term.as_bytes());
    k.push(0);
    k
}

/// The key of the block of `term`'s list with the bound `bound` (`None`: the tail).
#[cfg(test)]
pub(crate) fn block_key(term: &str, bound: Option<UnitId>) -> Vec<u8> {
    crate::blocks::block_key(&list_key(term), bound.as_ref().map(|u| &u.0))
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
    let mut k = vec![TAG_DOC];
    k.extend_from_slice(&unit.0);
    k
}

pub(crate) const STATS_KEY: [u8; 1] = [TAG_STATS];

fn valid_term(t: &[u8]) -> bool {
    (1..=MAX_TERM_LEN).contains(&t.len()) && !t.contains(&0) && std::str::from_utf8(t).is_ok()
}

/// The term and bound (`None`: the tail) of a posting block key.
pub(crate) fn decode_posting_key(key: &[u8]) -> Result<(String, Option<UnitId>), DecodeError> {
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
    let bound = crate::blocks::bound_of(&key[..=nul], key).map_err(|_| shape())?;
    Ok((String::from_utf8_lossy(term).into_owned(), bound.map(UnitId)))
}

/// The unit of a `DOC` key.
pub(crate) fn decode_doc_key(key: &[u8]) -> Result<UnitId, DecodeError> {
    match key.get(1..).and_then(|b| <[u8; 16]>::try_from(b).ok()) {
        Some(unit) if key[0] == TAG_DOC => Ok(UnitId(unit)),
        _ => Err(DecodeError::new(UnitRule::Unit3, format!("doc key of {} bytes is not tag, unit", key.len()))),
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

/// The posting blocks as [`crate::blocks`] stores them.
pub(crate) struct Postings;

impl crate::blocks::Block for Postings {
    type Val = Posting;
    const MAX: usize = BLOCK_MAX;
    const WHAT: &'static str = "POSTING";

    fn decode(v: &[u8]) -> Result<Vec<(crate::blocks::Id, Posting)>, DecodeError> {
        let e = |rule, m: &str| DecodeError::new(rule, format!("posting block: {m}"));
        let body = crate::blocks::body(v, "posting block")?;
        if body.is_empty() || !body.len().is_multiple_of(ENTRY_LEN) {
            return Err(e(UnitRule::Unit9, "no whole posting, or a partial one"));
        }
        if body.len() / ENTRY_LEN > BLOCK_MAX {
            return Err(e(UnitRule::Unit11, "more postings than a block holds"));
        }
        let mut out = Vec::with_capacity(body.len() / ENTRY_LEN);
        for c in body.chunks_exact(ENTRY_LEN) {
            let mut unit = [0u8; 16];
            unit.copy_from_slice(&c[..16]);
            let p = Posting { tf: be32(&c[16..20]), dl: be32(&c[20..]) };
            if p.tf == 0 || p.dl < p.tf {
                return Err(e(UnitRule::Unit12, &format!("tf {} in a document of {} terms", p.tf, p.dl)));
            }
            out.push((unit, p));
        }
        crate::blocks::ascending(&out, "posting block")?;
        Ok(out)
    }

    fn encode(entries: &[(crate::blocks::Id, Posting)]) -> Vec<u8> {
        let mut v = Vec::with_capacity(2 + ENTRY_LEN * entries.len());
        v.extend_from_slice(&[RECORD_VERSION, 0]);
        for (unit, p) in entries {
            v.extend_from_slice(unit);
            v.extend_from_slice(&p.tf.to_be_bytes());
            v.extend_from_slice(&p.dl.to_be_bytes());
        }
        v
    }
}

/// The postings of a block value.
pub(crate) fn decode_block(v: &[u8]) -> Result<Vec<(UnitId, Posting)>, DecodeError> {
    Ok(<Postings as crate::blocks::Block>::decode(v)?.into_iter().map(|(u, p)| (UnitId(u), p)).collect())
}

fn encode_doc(doc: &Doc) -> Vec<u8> {
    let mut v = vec![RECORD_VERSION, 0];
    v.extend_from_slice(&doc.len.to_be_bytes());
    v.extend_from_slice(&(doc.terms.len() as u32).to_be_bytes());
    for t in doc.terms.keys() {
        v.push(t.len() as u8);
        v.extend_from_slice(t.as_bytes());
    }
    v
}

/// The length and the distinct terms of a `DOC` value.
pub(crate) fn decode_doc(v: &[u8]) -> Result<(u32, BTreeSet<String>), DecodeError> {
    let b = body(v, "doc record")?;
    if b.len() < 8 {
        return Err(DecodeError::new(UnitRule::Unit9, format!("doc record has {} bytes after its envelope", b.len())));
    }
    let (dl, n) = (be32(&b[..4]), be32(&b[4..8]));
    let mut rest = &b[8..];
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
    Ok((dl, terms))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stats {
    pub since: u64,
    pub docs: u64,
    pub total_len: u64,
}

fn encode_stats(s: Stats) -> Vec<u8> {
    let mut v = vec![RECORD_VERSION, 0];
    for n in [s.since, s.docs, s.total_len] {
        v.extend_from_slice(&n.to_be_bytes());
    }
    v
}

pub(crate) fn decode_stats(v: &[u8]) -> Result<Stats, DecodeError> {
    let b = fixed::<24>(v, "stats record")?;
    Ok(Stats { since: be64(&b[..8]), docs: be64(&b[8..16]), total_len: be64(&b[16..]) })
}

// -------------------------------------------------------------- writer

/// What an operation does to the content of a unit, for the indexes of its
/// payload (this one and [`crate::substring`]).
pub(crate) enum Next<'a> {
    /// The writes do not change the unit's content (retention drops history).
    Unchanged,
    /// The unit's type and its indexed payload afterwards; `None` when it has
    /// none (the unit is not `Active`).
    Content(String, Option<&'a [u8]>),
}

/// Reads the unit's new head and payload off the writes of its operation.
pub(crate) fn next_of(unit: UnitId, writes: &[Write]) -> Result<Next<'_>, UnitError> {
    let head_key = Key::UnitHead { unit }.encode()?;
    let Some(Write::Put(_, raw)) = writes.iter().find(|w| matches!(w, Write::Put(k, _) if *k == head_key)) else {
        return Ok(Next::Unchanged);
    };
    let head = UnitHead::decode(raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
    if head.lifecycle != Lifecycle::Active {
        return Ok(Next::Content(head.unit_type, None));
    }
    let payload_key = Key::RevPayload { unit, rev: head.head_rev }.encode()?;
    Ok(writes
        .iter()
        .find_map(|w| match w {
            Write::Put(k, p) if *k == payload_key => Some(p.as_slice()),
            _ => None,
        })
        .map_or(Next::Unchanged, |p| Next::Content(head.unit_type, Some(p))))
}

/// What a change set will change in the posting lists: per term, the postings
/// to put (`Some`) or remove (`None`) by unit, the last operation of a unit
/// winning.
pub(crate) type PendingPostings = BTreeMap<String, BTreeMap<crate::blocks::Id, Option<Posting>>>;

/// The index change of one operation that is not a record of its own: the
/// postings the unit gives up and gains, and what its document does to the
/// statistics.
pub(crate) struct TextChange {
    unit: UnitId,
    gone: Vec<String>,
    put: Vec<(String, Posting)>,
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
        let Next::Content(unit_type, payload) = next_of(unit, writes)? else {
            return Ok(TextPlan::default());
        };
        // The index is opt-in per type (ADR-0023), and turning it off removes the
        // entries of the type's units in that same change set: a unit of a type
        // that is not indexed has no document to add or to remove. Decided
        // before the payload is cut into terms, which most writes never need.
        if !self.fulltext_enabled(&unit_type)? {
            return Ok(TextPlan::default());
        }
        self.plan_document(unit, payload)
    }

    /// The index change that makes the text of `payload` the document of
    /// `unit` (`None`: the unit has no document), whatever its type's option:
    /// what [`ChangeSet::set_fulltext`](crate::type_options) needs to index or
    /// clear the units a type already has.
    pub(crate) fn plan_document(&self, unit: UnitId, payload: Option<&[u8]>) -> Result<TextPlan, UnitError> {
        let new = payload.and_then(analyze);
        let old = match self.txn()?.get(&doc_key(unit))? {
            Some(raw) => Some(decode_doc(&raw).map_err(|e| context(e, &format!("DOC of unit {unit}")))?),
            None => None,
        };
        if old.is_none() && new.is_none() {
            return Ok(TextPlan::default());
        }
        // Only the postings the new text no longer has go; the others are overwritten.
        let gone = old
            .iter()
            .flat_map(|(_, terms)| terms)
            .filter(|t| !new.as_ref().is_some_and(|d| d.terms.contains_key(*t)))
            .cloned()
            .collect();
        let (mut docs, mut len) = (0i64, 0i64);
        if let Some((dl, _)) = &old {
            docs -= 1;
            len -= i64::from(*dl);
        }
        let (write, put) = match new {
            Some(doc) => {
                docs += 1;
                len += i64::from(doc.len);
                let write = Write::Put(doc_key(unit), encode_doc(&doc));
                let put = doc.terms.into_iter().map(|(t, tf)| (t, Posting { tf, dl: doc.len })).collect();
                (write, put)
            }
            None => (Write::Delete(doc_key(unit)), Vec::new()),
        };
        Ok(TextPlan { writes: vec![write], change: Some(TextChange { unit, gone, put, docs, len }) })
    }

    /// Keeps the posting and statistics changes of an operation planned by
    /// [`plan_document`](Self::plan_document), for [`flush_fulltext`](Self::flush_fulltext).
    pub(crate) fn stage_fulltext(&mut self, change: Option<TextChange>) {
        let Some(c) = change else { return };
        for term in c.gone {
            stage(&mut self.text_pending, term, c.unit, None);
        }
        for (term, p) in c.put {
            stage(&mut self.text_pending, term, c.unit, Some(p));
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
        for (term, ops) in std::mem::take(&mut self.text_pending) {
            crate::blocks::flush::<Postings, E>(self, &list_key(&term), ops, false)?;
        }
        let Some((docs, len)) = self.text_stats.take() else {
            return Ok(());
        };
        let mut stats = match self.txn()?.get(&STATS_KEY)? {
            Some(raw) => decode_stats(&raw).map_err(|e| context(e, "FULLTEXT_STATS"))?,
            None => Stats { since: self.commit_seq().0, docs: 0, total_len: 0 },
        };
        // ponytail: saturates like the counts of `type_count`; a statistic that
        // was already wrong stays wrong and `verify_units` reports it (UNIT-49).
        let add = |n: u64, d: i64| {
            if d >= 0 { n.saturating_add(d.unsigned_abs()) } else { n.saturating_sub(d.unsigned_abs()) }
        };
        stats.docs = add(stats.docs, docs);
        stats.total_len = add(stats.total_len, len);
        self.write_owned(Write::Put(STATS_KEY.to_vec(), encode_stats(stats)))
    }
}

/// Records that `unit` has `value` (`None`: no posting) in the list of `term`.
fn stage(pending: &mut PendingPostings, term: String, unit: UnitId, value: Option<Posting>) {
    pending.entry(term).or_default().insert(unit.0, value);
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
    /// or without `types`. Ties are in unit id order. Reads only the posting ranges of the query
    /// terms, the statistics record and, with `types`, the heads of the best-scored units until
    /// `limit` of them are of a wanted type.
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
        let mut scores: HashMap<UnitId, f64> = HashMap::new();
        for term in &terms {
            let (start, end) = posting_range(term);
            let mut postings = Vec::new();
            scan_all(&self.snap, start, end, |_, v| {
                postings.extend(decode_block(v).map_err(|e| context(e, "POSTING"))?);
                Ok(())
            })?;
            let df = postings.len() as f64;
            let idf = (1.0 + ((n_docs - df).max(0.0) + 0.5) / (df + 0.5)).ln();
            for (unit, p) in postings {
                let (tf, dl) = (f64::from(p.tf), f64::from(p.dl));
                *scores.entry(unit).or_insert(0.0) += idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / avg_len));
            }
        }
        let mut hits: Vec<TextHit> = scores.into_iter().map(|(unit, score)| TextHit { unit, score }).collect();
        let best_first = |a: &TextHit, b: &TextHit| b.score.total_cmp(&a.score).then(a.unit.cmp(&b.unit));
        // The best hits are taken off the front a slice at a time, so a search sorts (and, with
        // `types`, reads the head of) about `limit` units, not every unit that holds a term.
        let mut out = Vec::with_capacity(limit.min(hits.len()));
        let mut from = 0;
        while out.len() < limit && from < hits.len() {
            let rest = &mut hits[from..];
            let n = (limit - out.len()).min(rest.len());
            if n < rest.len() {
                rest.select_nth_unstable_by(n - 1, best_first);
            }
            rest[..n].sort_unstable_by(best_first);
            for hit in &rest[..n] {
                if let Some(types) = types {
                    let Some(head) = crate::history::read_head(&|k| self.snap.get(k), hit.unit)? else {
                        return Err(UnitError::Storage(kerno_core::Error::Corruption(format!(
                            "a POSTING of unit {} has no UNIT_HEAD",
                            hit.unit
                        ))));
                    };
                    if !types.contains(&head.unit_type.as_str()) {
                        continue;
                    }
                }
                out.push(*hit);
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
