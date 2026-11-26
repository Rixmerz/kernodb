//! The tokenizer of the full-text index: what a text and a query are cut into.
//!
//! A term is a maximal run of alphanumeric characters (Unicode `Alphabetic`
//! and `Numeric`), lower-cased. Everything else separates terms. Indexing and
//! querying share [`tokens`], so a query term matches exactly the terms the
//! index holds.
//!
//! `ponytail:` the split is `char::is_alphanumeric`, not the UAX #29 word
//! boundaries: an apostrophe splits (`don't` is `don` and `t`), and text
//! without spaces (Chinese, Japanese) is one term per run of ideographs.
//! Upgrade path: `unicode-segmentation` as a dependency and a new record
//! version, since the terms of stored text would change.

use std::collections::BTreeMap;

/// Longest term in bytes. A longer one is cut at a character boundary, in the
/// index and in the query alike.
pub const MAX_TERM_LEN: usize = 64;

/// How much of a payload is indexed: the first `MAX_INDEXED_BYTES` bytes,
/// cut at a character boundary. A unit's index entries are written in one
/// engine transaction, which has a batch limit, so the distinct terms of one
/// unit must stay far below it (256 KiB is at most 128 Ki distinct terms).
pub const MAX_INDEXED_BYTES: usize = 256 * 1024;

/// The terms of `text`, in order of appearance, repeats included.
pub fn tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| {
        let mut t = w.to_lowercase();
        let mut end = t.len().min(MAX_TERM_LEN);
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        t.truncate(end);
        t
    })
}

/// What the index keeps of one payload: its length in terms and the
/// frequency of each distinct term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Doc {
    pub len: u32,
    pub terms: BTreeMap<String, u32>,
}

/// The document of a payload, or `None` when it is not indexed: bytes that
/// are neither UTF-8 nor a canonical record, and text without a single term.
///
/// A UTF-8 payload is indexed from its raw text, as always. A payload that is
/// not UTF-8 but decodes as a canonical record (`schema::Document`) is indexed
/// from its Text field values, in field order, joined by a newline.
pub(crate) fn analyze(payload: &[u8]) -> Option<Doc> {
    let owned;
    let text = match std::str::from_utf8(payload) {
        Ok(t) => t,
        Err(_) => {
            owned = record_text(payload)?;
            owned.as_str()
        }
    };
    let mut end = text.len().min(MAX_INDEXED_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut doc = Doc { len: 0, terms: BTreeMap::new() };
    for t in tokens(&text[..end]) {
        doc.len += 1;
        *doc.terms.entry(t).or_insert(0) += 1;
    }
    (doc.len > 0).then_some(doc)
}

/// The Text field values of a canonical record, joined by a newline, or
/// `None` when `payload` is not one.
fn record_text(payload: &[u8]) -> Option<String> {
    let doc = crate::schema::Document::parse(payload).ok()?;
    let texts: Vec<&str> = doc
        .iter()
        .filter_map(|(_, v)| match v {
            crate::schema::FieldValue::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    Some(texts.join("\n"))
}
