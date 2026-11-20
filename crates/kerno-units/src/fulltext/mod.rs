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
//! * `POSTING` `0x74 ‖ term ‖ 0x00 ‖ unit[16]` → `envelope[2] ‖ tf: u32 BE ‖
//!   dl: u32 BE`: the frequency of `term` in the unit and the unit's length
//!   in terms. A term has no NUL, so the range of one term is
//!   `[tag ‖ term ‖ 0x00, tag ‖ term ‖ 0x01)`. A query reads that range and
//!   nothing else per term.
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
//! # Legacy databases and the layer mark
//!
//! A unit written before this module has no index entries and is not found
//! until a write rewrites it; `since` in `STATS` is what lets
//! [`crate::verify`] tell it from a unit whose entries were lost (UNIT-49).
//!
//! `ponytail:` these records do not raise the layer mark: that needs a new
//! `LAYER_VERSION_*` in `codec.rs` and `docs/file-format/units-v6.md`, which
//! this change did not touch. A build older than this one reads the keys as
//! "a newer layer" (UNIT-2) in `verify_units`. Upgrade path: layer version 6
//! written with the first `STATS`, like the count records of layer 5.
//!
//! `ponytail:` a query reads the whole posting list of each of its terms and
//! scores every unit in it (no top-k pruning); fine up to ~10^5 postings per
//! term. A rebuild of legacy units is not provided.

mod check;
mod tokenize;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::codec::{Key, UnitHead};
use crate::db::{ChangeSet, UnitDb, UnitSnapshot, Write, context, scan_all};
use crate::error::{DecodeError, UnitError, UnitRule};
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

/// A unit a query found and its BM25 score (higher is a better match).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextHit {
    pub unit: UnitId,
    pub score: f64,
}

// ---------------------------------------------------------------- keys

pub(crate) fn posting_key(term: &str, unit: UnitId) -> Vec<u8> {
    let mut k = vec![TAG_POSTING];
    k.extend_from_slice(term.as_bytes());
    k.push(0);
    k.extend_from_slice(&unit.0);
    k
}

/// Scan bounds `[start, end)` of the postings of one term.
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

/// The term and unit of a posting key.
pub(crate) fn decode_posting_key(key: &[u8]) -> Result<(String, UnitId), DecodeError> {
    let n = key.len();
    if !(1 + 1 + 1 + 16..=1 + MAX_TERM_LEN + 1 + 16).contains(&n) || key[0] != TAG_POSTING || key[n - 17] != 0 {
        return Err(DecodeError::new(UnitRule::Unit3, format!("posting key of {n} bytes is not tag, term, NUL, unit")));
    }
    let term = &key[1..n - 17];
    if !valid_term(term) {
        return Err(DecodeError::new(
            UnitRule::Unit12,
            "posting key term is empty, over 64 bytes, has a NUL or is not UTF-8",
        ));
    }
    let mut unit = [0u8; 16];
    unit.copy_from_slice(&key[n - 16..]);
    Ok((String::from_utf8_lossy(term).into_owned(), UnitId(unit)))
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

pub(crate) fn encode_posting(p: Posting) -> Vec<u8> {
    let mut v = vec![RECORD_VERSION, 0];
    v.extend_from_slice(&p.tf.to_be_bytes());
    v.extend_from_slice(&p.dl.to_be_bytes());
    v
}

pub(crate) fn decode_posting(v: &[u8]) -> Result<Posting, DecodeError> {
    let b = fixed::<8>(v, "posting record")?;
    let p = Posting { tf: be32(&b[..4]), dl: be32(&b[4..]) };
    if p.tf == 0 || p.dl < p.tf {
        return Err(DecodeError::new(
            UnitRule::Unit12,
            format!("posting record tf {} in a document of {} terms", p.tf, p.dl),
        ));
    }
    Ok(p)
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

/// What an operation does to the text of a unit.
enum Next {
    /// The writes do not change the unit's text (retention drops history).
    Unchanged,
    /// The unit's indexed document afterwards; `None` when it has none
    /// (not `Active`, not text, or without a term).
    Doc(Option<Doc>),
}

/// Reads the unit's new head and payload off the writes of its operation.
fn next_of(unit: UnitId, writes: &[Write]) -> Result<Next, UnitError> {
    let head_key = Key::UnitHead { unit }.encode()?;
    let Some(Write::Put(_, raw)) = writes.iter().find(|w| matches!(w, Write::Put(k, _) if *k == head_key)) else {
        return Ok(Next::Unchanged);
    };
    let head = UnitHead::decode(raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
    if head.lifecycle != Lifecycle::Active {
        return Ok(Next::Doc(None));
    }
    let payload_key = Key::RevPayload { unit, rev: head.head_rev }.encode()?;
    Ok(writes
        .iter()
        .find_map(|w| match w {
            Write::Put(k, p) if *k == payload_key => Some(Next::Doc(analyze(p))),
            _ => None,
        })
        .unwrap_or(Next::Unchanged))
}

impl<E: TxnEngine> ChangeSet<'_, E> {
    /// The index writes that follow the operation `writes` on `unit`, planned
    /// by reads only (like [`ChangeSet::plan_live`](crate::type_count)):
    /// call it before the first write of the operation, so a refusal leaves
    /// no index entry without its head. A change set sees its own earlier
    /// writes, so the statistics of several units add up.
    pub(crate) fn plan_fulltext(&self, unit: UnitId, writes: &[Write]) -> Result<Vec<Write>, UnitError> {
        let Next::Doc(new) = next_of(unit, writes)? else {
            return Ok(Vec::new());
        };
        let txn = self.txn()?;
        let old = match txn.get(&doc_key(unit))? {
            Some(raw) => Some(decode_doc(&raw).map_err(|e| context(e, &format!("DOC of unit {unit}")))?),
            None => None,
        };
        if old.is_none() && new.is_none() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        // Only the postings the new text no longer has go; the others are overwritten.
        for t in
            old.iter().flat_map(|(_, terms)| terms).filter(|t| !new.as_ref().is_some_and(|d| d.terms.contains_key(*t)))
        {
            out.push(Write::Delete(posting_key(t, unit)));
        }
        match &new {
            Some(doc) => {
                for (t, tf) in &doc.terms {
                    out.push(Write::Put(posting_key(t, unit), encode_posting(Posting { tf: *tf, dl: doc.len })));
                }
                out.push(Write::Put(doc_key(unit), encode_doc(doc)));
            }
            None => out.push(Write::Delete(doc_key(unit))),
        }
        let mut stats = match txn.get(&STATS_KEY)? {
            Some(raw) => decode_stats(&raw).map_err(|e| context(e, "FULLTEXT_STATS"))?,
            None => Stats { since: self.commit_seq().0, docs: 0, total_len: 0 },
        };
        // ponytail: saturates like the counts of `type_count`; a statistic that
        // was already wrong stays wrong and `verify_units` reports it (UNIT-49).
        if let Some((dl, _)) = &old {
            stats.docs = stats.docs.saturating_sub(1);
            stats.total_len = stats.total_len.saturating_sub(u64::from(*dl));
        }
        if let Some(doc) = &new {
            stats.docs = stats.docs.saturating_add(1);
            stats.total_len = stats.total_len.saturating_add(u64::from(doc.len));
        }
        out.push(Write::Put(STATS_KEY.to_vec(), encode_stats(stats)));
        Ok(out)
    }
}

// --------------------------------------------------------------- query

impl<E: Engine> UnitSnapshot<E> {
    /// The units whose text matches `query`, best first, at most `limit`.
    ///
    /// The query is cut into terms like the indexed text and a unit's score
    /// is the BM25 sum over the distinct terms it holds (see the [module
    /// documentation](self)); a unit with none of them is not a hit. An
    /// empty query, a query without a term and `limit == 0` answer nothing.
    /// Ties are in unit id order. Reads only the posting ranges of the query
    /// terms and the statistics record.
    ///
    /// # Errors
    ///
    /// `Storage(Corruption)` when a record of the index does not decode.
    pub fn search_text(&self, query: &str, limit: usize) -> Result<Vec<TextHit>, UnitError> {
        let terms: BTreeSet<String> = tokens(query).collect();
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
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
        let mut scores: BTreeMap<UnitId, f64> = BTreeMap::new();
        for term in &terms {
            let (start, end) = posting_range(term);
            let mut postings = Vec::new();
            scan_all(&self.snap, start, end, |k, v| {
                let (_, unit) = decode_posting_key(k).map_err(|e| context(e, "POSTING"))?;
                postings.push((unit, decode_posting(v).map_err(|e| context(e, "POSTING"))?));
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
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.unit.cmp(&b.unit)));
        hits.truncate(limit);
        Ok(hits)
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// [`UnitSnapshot::search_text`] on a snapshot of the latest commit.
    pub fn search_text(&self, query: &str, limit: usize) -> Result<Vec<TextHit>, UnitError> {
        self.snapshot()?.search_text(query, limit)
    }
}
