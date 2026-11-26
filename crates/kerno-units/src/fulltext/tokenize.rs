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
    // Counted in a hash map and ordered once at the end: a text repeats most of its terms, so
    // a term is allocated and ordered once, not per occurrence. A word of ASCII letters and
    // digits (most words) is lower-cased in a reused buffer, which is what `tokens` does to
    // it; any other word goes through `tokens` itself, so the terms are the same.
    let mut counts: std::collections::HashMap<String, u32, std::hash::BuildHasherDefault<TermHasher>> =
        std::collections::HashMap::default();
    let mut len = 0u32;
    let mut lower = String::new();
    let text = &text[..end];
    let bytes = text.as_bytes();
    let mut count = |word: &str, ascii: bool| {
        lower.clear();
        if ascii {
            lower.push_str(&word[..word.len().min(MAX_TERM_LEN)]);
            lower.make_ascii_lowercase();
        } else {
            lower.extend(tokens(word));
        }
        len += 1;
        match counts.get_mut(lower.as_str()) {
            Some(n) => *n += 1,
            None => {
                counts.insert(lower.clone(), 1);
            }
        }
    };
    // The words are the maximal runs of alphanumeric characters, as in `tokens`. An ASCII
    // byte is judged as a byte (for ASCII, `char::is_alphanumeric` is `is_ascii_alphanumeric`);
    // any other character is decoded and judged as `tokens` judges it.
    let (mut i, mut start, mut ascii) = (0, None, true);
    while i < bytes.len() {
        let (alnum, width) = if bytes[i] < 0x80 {
            (bytes[i].is_ascii_alphanumeric(), 1)
        } else {
            let c = text[i..].chars().next().unwrap_or(' ');
            (c.is_alphanumeric(), c.len_utf8())
        };
        match (alnum, start) {
            (true, None) => (start, ascii) = (Some(i), width == 1),
            (true, Some(_)) => ascii &= width == 1,
            (false, Some(from)) => {
                count(&text[from..i], ascii);
                start = None;
            }
            (false, None) => {}
        }
        i += width;
    }
    if let Some(from) = start {
        count(&text[from..], ascii);
    }
    let doc = Doc { len, terms: counts.into_iter().collect() };
    (doc.len > 0).then_some(doc)
}

/// FNV-1a: the terms are short and the map is the document's own.
#[derive(Clone, Copy)]
pub(crate) struct TermHasher(u64);

impl Default for TermHasher {
    fn default() -> Self {
        TermHasher(0xCBF2_9CE4_8422_2325)
    }
}

impl std::hash::Hasher for TermHasher {
    fn finish(&self) -> u64 {
        // The table takes its index from the low bits and its tag from the high ones.
        self.0 ^ (self.0 >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x0100_0000_01B3);
        }
    }
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
