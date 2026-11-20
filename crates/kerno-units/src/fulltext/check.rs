//! The self-check of the full-text index, run by `verify_units` (UNIT-49).
//!
//! The verifier walks every key once. It hands each head payload it meets
//! (`Active` head revision) to [`Check::expect`], and each full-text record
//! to [`Check::record`]; after the walk [`Check::findings`] compares what the
//! payloads say the index must hold with what it holds, down to every
//! posting. The expected index is built in memory, like the rest of the
//! verifier's state (`ponytail:` O(distinct terms of the store); upgrade path
//! a per-unit hash of the postings).
//!
//! A finding names the `DOC` or `STATS` key and unit ids and counts, never a
//! term: terms are the content of the payloads.

use std::collections::{BTreeMap, BTreeSet};

use super::tokenize::{Doc, analyze};
use super::{
    Posting, STATS_KEY, Stats, TAG_DOC, TAG_POSTING, TAG_STATS, decode_doc, decode_doc_key, decode_posting,
    decode_posting_key, decode_stats, doc_key,
};
use crate::error::{DecodeError, UnitRule};
use crate::types::UnitId;

#[derive(Default)]
pub(crate) struct Check {
    /// The document each `Active` text unit must have, with the commit of its head.
    expected: BTreeMap<UnitId, (u64, Doc)>,
    docs: BTreeMap<UnitId, (u32, BTreeSet<String>)>,
    postings: BTreeMap<UnitId, BTreeMap<String, Posting>>,
    stats: Option<Stats>,
    /// A record did not decode: that is a structure finding already, and the
    /// comparison would only repeat it as a pile of cross findings.
    damaged: bool,
}

impl Check {
    /// Whether a key with this first byte is a full-text record.
    pub(crate) fn owns(tag: Option<&u8>) -> bool {
        matches!(tag, Some(&(TAG_POSTING | TAG_DOC | TAG_STATS)))
    }

    /// The head payload of an `Active` unit, whose head was written at
    /// `updated_seq`.
    pub(crate) fn expect(&mut self, unit: UnitId, updated_seq: u64, payload: &[u8]) {
        if let Some(doc) = analyze(payload) {
            self.expected.insert(unit, (updated_seq, doc));
        }
    }

    /// One full-text record of the walk.
    pub(crate) fn record(&mut self, k: &[u8], v: &[u8]) -> Result<(), DecodeError> {
        let r = self.keep(k, v);
        self.damaged |= r.is_err();
        r
    }

    fn keep(&mut self, k: &[u8], v: &[u8]) -> Result<(), DecodeError> {
        match k.first() {
            Some(&TAG_POSTING) => {
                let (term, unit) = decode_posting_key(k)?;
                self.postings.entry(unit).or_default().insert(term, decode_posting(v)?);
            }
            Some(&TAG_DOC) => {
                let unit = decode_doc_key(k)?;
                self.docs.insert(unit, decode_doc(v)?);
            }
            _ => {
                if k != STATS_KEY {
                    return Err(DecodeError::new(UnitRule::Unit3, format!("stats key has {} bytes", k.len())));
                }
                self.stats = Some(decode_stats(v)?);
            }
        }
        Ok(())
    }

    /// `(key, detail)` of every disagreement.
    pub(crate) fn findings(&self) -> Vec<(Vec<u8>, String)> {
        let mut out = Vec::new();
        if self.damaged {
            return out;
        }
        let Some(stats) = self.stats else {
            // Nothing wrote a statistic, so nothing may have written an entry.
            if let Some(unit) = self.docs.keys().chain(self.postings.keys()).next() {
                out.push((doc_key(*unit), "full-text entries without a statistics record".into()));
            }
            return out;
        };
        let total: u64 = self.docs.values().map(|(dl, _)| u64::from(*dl)).sum();
        if stats.docs != self.docs.len() as u64 || stats.total_len != total {
            out.push((
                STATS_KEY.to_vec(),
                format!(
                    "statistics say {} documents of {} terms, the store has {} of {}",
                    stats.docs,
                    stats.total_len,
                    self.docs.len(),
                    total
                ),
            ));
        }
        // A unit written since the first statistic must be indexed; an older
        // one may predate the index (legacy), and is checked only if it is.
        for (unit, (seq, _)) in &self.expected {
            if *seq >= stats.since && !self.docs.contains_key(unit) {
                out.push((doc_key(*unit), format!("unit {unit} has text but no DOC record")));
            }
        }
        for (unit, (dl, terms)) in &self.docs {
            let Some((_, want)) = self.expected.get(unit) else {
                out.push((doc_key(*unit), format!("DOC of unit {unit}, which is not an Active unit with text")));
                continue;
            };
            let same_terms = want.terms.keys().eq(terms.iter());
            let same_postings = self.postings.get(unit).is_some_and(|p| {
                p.len() == want.terms.len()
                    && p.iter().all(|(t, p)| want.terms.get(t) == Some(&p.tf) && p.dl == want.len)
            });
            if *dl != want.len || !same_terms || !same_postings {
                out.push((doc_key(*unit), format!("unit {unit}: DOC and postings disagree with its payload")));
            }
        }
        for unit in self.postings.keys().filter(|u| !self.docs.contains_key(u)) {
            out.push((doc_key(*unit), format!("postings of unit {unit}, which has no DOC record")));
        }
        out
    }
}
