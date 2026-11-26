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

use std::collections::{BTreeSet, HashMap};
use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::codec::{Key, UnitHead};
use crate::db::{ChangeSet, UnitDb, UnitSnapshot, Write, context};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::runs::{self, Family, Pending, Slots, Staged};
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


/// A unit a query found and its BM25 score (higher is a better match).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextHit {
    pub unit: UnitId,
    pub score: f64,
}

// ---------------------------------------------------------------- keys

/// The prefix of the records of the keyword index ([`crate::runs`]).
pub(crate) const PREFIX: [u8; 1] = [TAG_POSTING];

/// `term ‖ 0x00`: the key of the list of one term in a run.
pub(crate) fn list_key(term: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(term.len() + 1);
    k.extend_from_slice(term.as_bytes());
    k.push(0);
    k
}

/// The key of the document record of `unit`.
pub(crate) fn doc_key(unit: UnitId) -> Vec<u8> {
    runs::doc_key(&PREFIX, unit)
}

pub(crate) const STATS_KEY: [u8; 1] = [TAG_STATS];

fn valid_term(t: &[u8]) -> bool {
    (1..=MAX_TERM_LEN).contains(&t.len()) && !t.contains(&0) && std::str::from_utf8(t).is_ok()
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

/// The keyword index as [`crate::runs`] stores it: a list per term (`term ‖ 0x00`, a term
/// has no NUL), postings with the frequency of the term and the length of the document.
pub(crate) struct Postings;

impl Family for Postings {
    const WHAT: &'static str = "POSTING";
    const VALUES: bool = true;

    fn list_len(rest: &[u8]) -> Option<usize> {
        let nul = rest.iter().take(MAX_TERM_LEN + 1).position(|b| *b == 0)?;
        valid_term(&rest[..nul]).then_some(nul + 1)
    }
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

/// What a change set will change in the index: the numbers it gives and those that die
/// ([`runs::Pending`]) and, per term, the postings of the new numbers (the number less the
/// first one the change set gives, the frequency, the document length), ascending.
pub(crate) type PendingPostings = Option<(Pending, TermLists)>;

type TermLists = HashMap<String, Vec<(u32, u32, u32)>, std::hash::BuildHasherDefault<tokenize::TermHasher>>;

/// The index change of one operation: the number and the length the unit's document had
/// (`None`: it had none) and its new document (`None`: it has none any more).
pub(crate) struct TextChange {
    unit: UnitId,
    old: Option<(u64, u32)>,
    new: Option<Doc>,
}

/// The staged lists of a change set, in term order, for [`runs::flush`].
struct StagedTerms {
    lists: Vec<(Vec<u8>, Vec<(u32, u32, u32)>)>,
    at: usize,
    base: u64,
}

impl Staged for StagedTerms {
    fn peek(&mut self) -> Option<&[u8]> {
        self.lists.get(self.at).map(|(k, _)| k.as_slice())
    }

    fn take(&mut self, emit: &mut dyn FnMut(runs::Posting)) {
        if let Some((_, postings)) = self.lists.get(self.at) {
            for (off, tf, dl) in postings {
                emit((self.base + u64::from(*off), *tf, *dl));
            }
        }
        self.at += 1;
    }
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
        // The document record says whether the index holds the unit, under which number and
        // with which length: the text it had is not read.
        let old = match self.txn()?.get(&doc_key(unit))? {
            Some(raw) => Some(runs::decode_doc(&raw, "DOC").map_err(|e| context(e, &format!("DOC of unit {unit}")))?),
            None => None,
        };
        if old.is_none() && new.is_none() {
            return Ok(TextPlan::default());
        }
        Ok(TextPlan { writes: Vec::new(), change: Some(TextChange { unit, old, new }) })
    }

    /// Keeps the posting and statistics changes of an operation planned by
    /// [`plan_document`](Self::plan_document), for [`flush_fulltext`](Self::flush_fulltext),
    /// and writes the unit's document record: the number the unit had dies, and its new
    /// document, if it has one, gets the next.
    pub(crate) fn stage_fulltext(&mut self, change: Option<TextChange>) -> Result<(), UnitError> {
        let Some(c) = change else { return Ok(()) };
        if self.text_pending.is_none() {
            let pending = Pending::load::<_, Postings>(self.txn()?, PREFIX.to_vec())?;
            self.text_pending = Some((pending, TermLists::default()));
        }
        let Some((pending, lists)) = self.text_pending.as_mut() else { return Ok(()) };
        let (mut docs, mut len) = (0i64, 0i64);
        if let Some((num, dl)) = c.old {
            pending.kill(num);
            docs -= 1;
            len -= i64::from(dl);
        }
        let write = match c.new {
            Some(doc) => {
                let num = pending.alloc(c.unit);
                let off = u32::try_from(num - pending.base())
                    .map_err(|_| UnitError::InvalidArgument("a change set indexes at most 2^32 units".into()))?;
                docs += 1;
                len += i64::from(doc.len);
                for (term, tf) in doc.terms {
                    lists.entry(term).or_default().push((off, tf, doc.len));
                }
                Write::Put(doc_key(c.unit), runs::encode_doc(num, doc.len))
            }
            None => Write::Delete(doc_key(c.unit)),
        };
        let stats = self.text_stats.get_or_insert((0, 0));
        stats.0 += docs;
        stats.1 += len;
        self.write_owned(write)
    }

    /// Writes the posting changes and the statistics delta the change set
    /// kept: the postings of its new numbers as one run, the number pages and the manifest
    /// ([`runs::flush`]), and the statistics record once. Called by `commit`.
    pub(crate) fn flush_fulltext(&mut self) -> Result<(), UnitError> {
        if let Some((pending, lists)) = self.text_pending.take() {
            // Terms in ascending order: a term's list key is the term and a NUL, and a term
            // has no NUL, so the keys order like the terms.
            let mut lists: Vec<(Vec<u8>, Vec<(u32, u32, u32)>)> = lists
                .into_iter()
                .map(|(term, postings)| {
                    let mut k = term.into_bytes();
                    k.push(0);
                    (k, postings)
                })
                .collect();
            lists.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            let mut staged = StagedTerms { lists, at: 0, base: pending.base() };
            runs::flush::<E, Postings>(self, pending, &mut staged, false)?;
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
        let get = |k: &[u8]| self.snap.get(k);
        let scan = |lo: Bound<&[u8]>, hi: Bound<&[u8]>, n: usize| self.snap.scan(lo, hi, n);
        let manifest = runs::read_manifest::<Postings>(&get, &PREFIX)?;
        let mut slots = Slots::default();
        // Scored by document number; a number is one unit's.
        let mut scores: HashMap<u64, f64> = HashMap::new();
        let mut postings: Vec<runs::Posting> = Vec::new();
        for term in &terms {
            let list = list_key(term);
            postings.clear();
            for run in &manifest.runs {
                let from = postings.len();
                runs::read_list::<Postings>(&scan, &PREFIX, run, &list, 0, None, usize::MAX, &mut |p| {
                    postings.push(p);
                })?;
                if run.dead > 0 {
                    // A number that died since the run was written is no posting any more.
                    let mut kept = from;
                    for at in from..postings.len() {
                        if slots.slot(&get, &PREFIX, postings[at].0)?.0 {
                            postings[kept] = postings[at];
                            kept += 1;
                        }
                    }
                    postings.truncate(kept);
                }
            }
            let df = postings.len() as f64;
            let idf = (1.0 + ((n_docs - df).max(0.0) + 0.5) / (df + 0.5)).ln();
            for (num, tf, dl) in &postings {
                let (tf, dl) = (f64::from(*tf), f64::from(*dl));
                *scores.entry(*num).or_insert(0.0) += idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * dl / avg_len));
            }
        }
        // Hits are best first, ties in unit id order. A number is mapped to its unit (a read of
        // its number page) only when it is among the best, or ties with the last of them: a
        // search scores every unit that holds a term and answers a few.
        let mut hits: Vec<(u64, f64)> = scores.into_iter().collect();
        let by_score = |a: &(u64, f64), b: &(u64, f64)| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0));
        let best_first = |a: &TextHit, b: &TextHit| b.score.total_cmp(&a.score).then(a.unit.cmp(&b.unit));
        // The best hits are taken off the front a slice at a time, so a search sorts (and, with
        // `types`, reads the head of) about `limit` units, not every unit that holds a term.
        let mut out = Vec::with_capacity(limit.min(hits.len()));
        let mut from = 0;
        while out.len() < limit && from < hits.len() {
            let rest = &mut hits[from..];
            let want = (limit - out.len()).min(rest.len());
            let mut n = want;
            if n < rest.len() {
                rest.select_nth_unstable_by(n - 1, by_score);
                // Every hit that ties with the last one taken comes along, so that the order
                // among them is decided by their unit ids, not by their numbers.
                let edge = rest[n - 1].1;
                let mut at = n;
                while at < rest.len() {
                    if rest[at].1.total_cmp(&edge).is_eq() {
                        rest.swap(n, at);
                        n += 1;
                    }
                    at += 1;
                }
            }
            let mut slice = Vec::with_capacity(n);
            for (num, score) in &rest[..n] {
                let (_, unit) = slots.slot(&get, &PREFIX, *num)?;
                slice.push(TextHit { unit, score: *score });
            }
            slice.sort_unstable_by(best_first);
            // The ties past `want` go back: they are the best of what is left.
            let taken = slice.len().min(want);
            if taken < slice.len() {
                let keep: std::collections::HashSet<UnitId> = slice[..taken].iter().map(|h| h.unit).collect();
                let mut front = 0;
                for at in 0..n {
                    if keep.contains(&slots.slot(&get, &PREFIX, rest[at].0)?.1) {
                        rest.swap(front, at);
                        front += 1;
                    }
                }
            }
            for hit in &slice[..taken] {
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
            from += taken;
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
