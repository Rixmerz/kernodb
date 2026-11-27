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
///
/// The distinct terms are kept back to back in one buffer, with their places and frequencies
/// in term order: a document costs a few allocations, not one per term.
#[derive(Clone, Debug)]
pub(crate) struct Doc {
    pub len: u32,
    /// The distinct terms, each once, back to back (in order of first appearance).
    text: String,
    /// Per distinct term, ascending by term: its bytes in `text` and its frequency.
    spans: Vec<(u32, u32, u32)>,
    /// [`term_hash`] of each term, in the order of `spans`: computed where the text is cut
    /// (maybe on another thread), so staging the document only looks its terms up.
    hashes: Vec<u64>,
}

impl PartialEq for Doc {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.terms().eq(other.terms())
    }
}

impl Eq for Doc {}

impl Doc {
    /// The distinct terms and their frequencies, ascending by term.
    pub(crate) fn terms(&self) -> impl ExactSizeIterator<Item = (&str, u32)> + '_ {
        self.spans.iter().map(|&(s, e, tf)| (&self.text[s as usize..e as usize], tf))
    }

    /// The frequency of `term`, if the document has it.
    /// [`terms`](Self::terms) with the [`term_hash`] of each.
    pub(crate) fn terms_hashed(&self) -> impl Iterator<Item = (&str, u32, u64)> + '_ {
        self.terms().zip(self.hashes.iter()).map(|((t, tf), &h)| (t, tf, h))
    }

    pub(crate) fn get(&self, term: &str) -> Option<u32> {
        self.spans
            .binary_search_by(|&(s, e, _)| self.text[s as usize..e as usize].cmp(term))
            .ok()
            .map(|at| self.spans[at].2)
    }
}

/// Distinct byte strings, numbered in order of first appearance: an open-addressing table over
/// one buffer that holds each string once. What a text's terms are counted with, and what a
/// change set keeps its posting lists by.
#[derive(Default)]
pub(crate) struct Interner {
    text: Vec<u8>,
    spans: Vec<(u32, u32)>,
    /// [`term_hash`] of each string, by number: a growing table needs no hashing again.
    hashes: Vec<u64>,
    /// `0` (empty) or `1 +` a number. Length a power of two (or 0), at most half full.
    slots: Vec<u32>,
}

impl Interner {
    pub(crate) fn with_capacity(strings: usize, bytes: usize) -> Self {
        Interner {
            text: Vec::with_capacity(bytes),
            spans: Vec::with_capacity(strings),
            hashes: Vec::with_capacity(strings),
            slots: vec![0; (strings.max(8) * 2).next_power_of_two()],
        }
    }

    /// The number of `s`, and whether it is new.
    pub(crate) fn intern(&mut self, s: &[u8]) -> (usize, bool) {
        self.intern_hashed(s, term_hash(s))
    }

    /// [`intern`](Self::intern) of `s` whose [`term_hash`] is `hash`, computed ahead.
    pub(crate) fn intern_hashed(&mut self, s: &[u8], hash: u64) -> (usize, bool) {
        if self.slots.is_empty() {
            self.slots = vec![0; 16];
        }
        let mask = self.slots.len() - 1;
        let mut at = hash as usize & mask;
        loop {
            match self.slots[at] {
                0 => break,
                n => {
                    let (from, to) = self.spans[n as usize - 1];
                    if self.hashes[n as usize - 1] == hash && self.text[from as usize..to as usize] == *s {
                        return (n as usize - 1, false);
                    }
                    at = (at + 1) & mask;
                }
            }
        }
        let from = self.text.len() as u32;
        self.text.extend_from_slice(s);
        self.spans.push((from, self.text.len() as u32));
        self.hashes.push(hash);
        self.slots[at] = self.spans.len() as u32;
        if self.spans.len() * 2 > self.slots.len() {
            self.grow();
        }
        (self.spans.len() - 1, true)
    }

    /// The string numbered `i`.
    pub(crate) fn get(&self, i: usize) -> &[u8] {
        let (from, to) = self.spans[i];
        &self.text[from as usize..to as usize]
    }

    /// The numbers in ascending order of their strings.
    ///
    /// Sorted on the first eight bytes as a number first (zero-padded: a string with no NUL
    /// orders the same), the whole strings compared only on a tie.
    pub(crate) fn ascending(&self) -> Vec<u32> {
        let mut keyed: Vec<(u64, u32)> = (0..self.spans.len())
            .map(|i| {
                let s = self.get(i);
                let mut head = [0u8; 8];
                let n = s.len().min(8);
                head[..n].copy_from_slice(&s[..n]);
                (u64::from_be_bytes(head), i as u32)
            })
            .collect();
        keyed.sort_unstable_by(|a, b| {
            a.0.cmp(&b.0).then_with(|| self.get(a.1 as usize).cmp(self.get(b.1 as usize)))
        });
        keyed.into_iter().map(|(_, i)| i).collect()
    }

    fn grow(&mut self) {
        let len = self.slots.len() * 2;
        self.slots.clear();
        self.slots.resize(len, 0);
        let mask = len - 1;
        for i in 0..self.spans.len() {
            let mut at = self.hashes[i] as usize & mask;
            while self.slots[at] != 0 {
                at = (at + 1) & mask;
            }
            self.slots[at] = i as u32 + 1;
        }
    }
}

/// Counts the terms of a text.
struct Counter {
    terms: Interner,
    tf: Vec<u32>,
}

impl Counter {
    fn with_capacity(bytes: usize) -> Self {
        // Distinct terms are a fraction of the words, and a word takes at least two bytes.
        let terms = (bytes / 8).max(8);
        Counter { terms: Interner::with_capacity(terms, bytes / 2), tf: Vec::with_capacity(terms) }
    }

    fn add(&mut self, term: &[u8]) {
        match self.terms.intern(term) {
            (_, true) => self.tf.push(1),
            (i, false) => self.tf[i] += 1,
        }
    }

    fn finish(self, len: u32) -> Option<Doc> {
        if len == 0 {
            return None;
        }
        let order = self.terms.ascending();
        let spans: Vec<(u32, u32, u32)> = order
            .iter()
            .map(|&i| {
                let (from, to) = self.terms.spans[i as usize];
                (from, to, self.tf[i as usize])
            })
            .collect();
        let hashes = order.iter().map(|&i| self.terms.hashes[i as usize]).collect();
        // Every term is a `str` (an ASCII word lowered, or what `tokens` gave), so is the buffer.
        let text = String::from_utf8(self.terms.text).ok()?;
        Some(Doc { len, text, spans, hashes })
    }
}

/// A hash of a term for [`Interner`]: eight bytes at a
/// time (the table only needs a spread, and hashes the terms of its own text).
pub(crate) fn term_hash(t: &[u8]) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut h = (t.len() as u64).wrapping_mul(K);
    let (words, rest) = t.as_chunks::<8>();
    for w in words {
        h = (h.rotate_left(26) ^ u64::from_le_bytes(*w)).wrapping_mul(K);
    }
    if !rest.is_empty() {
        let mut w = [0u8; 8];
        w[..rest.len()].copy_from_slice(rest);
        h = (h.rotate_left(26) ^ u64::from_le_bytes(w)).wrapping_mul(K);
    }
    h ^ (h >> 31)
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
    let text = &text[..end];
    let bytes = text.as_bytes();
    // A word of ASCII letters and digits (most words) is cut and lower-cased in a buffer on the
    // stack, which is what `tokens` does to it; any other word goes through `tokens` itself, so
    // the terms are the same. Only a term not seen before in the text is copied. ASCII bytes
    // are classified and lower-cased through `ASCII_CLASS`; a non-ASCII char is decoded and
    // asked `char::is_alphanumeric`, as `tokens` does.
    let mut counter = Counter::with_capacity(bytes.len());
    let mut len = 0u32;
    let mut lower = [0u8; MAX_TERM_LEN];
    // Whether the char at `i` (a non-ASCII lead byte) is alphanumeric, and its width.
    let wide = |i: usize| {
        let c = text[i..].chars().next().unwrap_or(' ');
        (c.is_alphanumeric(), c.len_utf8())
    };
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        // Separators.
        let b = bytes[i];
        match ASCII_CLASS[b as usize] {
            SEP => {
                i += 1;
                continue;
            }
            WIDE => {
                let (alnum, w) = wide(i);
                if !alnum {
                    i += w;
                    continue;
                }
            }
            _ => {}
        }
        // A word from `from`: ASCII letters and digits are lower-cased into `lower` as they go.
        let from = i;
        let mut ascii = true;
        while i < n {
            let b = bytes[i];
            let class = ASCII_CLASS[b as usize];
            if class >= ALNUM {
                let at = i - from;
                if at < MAX_TERM_LEN {
                    lower[at] = class - ALNUM;
                }
                i += 1;
            } else if class == WIDE {
                let (alnum, w) = wide(i);
                if !alnum {
                    break;
                }
                ascii = false;
                i += w;
            } else {
                break;
            }
        }
        len += 1;
        if ascii {
            counter.add(&lower[..(i - from).min(MAX_TERM_LEN)]);
        } else {
            let term: String = tokens(&text[from..i]).collect();
            counter.add(term.as_bytes());
        }
    }
    counter.finish(len)
}

/// [`ASCII_CLASS`] of a byte that separates words.
const SEP: u8 = 0;
/// [`ASCII_CLASS`] of a byte at or above `0x80`: the char it starts is decoded.
const WIDE: u8 = 1;
/// [`ASCII_CLASS`] of an ASCII letter or digit: `ALNUM` below its lower case (which is at
/// least `b'0'`, so above `WIDE`).
const ALNUM: u8 = 2;

/// Per byte: [`SEP`], [`WIDE`], or `ALNUM +` the byte lower-cased (`u8` wraps: the lower-cased
/// byte is recovered by subtracting `ALNUM`, and every one is at least `b'0' + ALNUM`).
const ASCII_CLASS: [u8; 256] = {
    let mut t = [SEP; 256];
    let mut b = 0usize;
    while b < 256 {
        let c = b as u8;
        t[b] = if c >= 0x80 {
            WIDE
        } else if c.is_ascii_alphanumeric() {
            c.to_ascii_lowercase() + ALNUM
        } else {
            SEP
        };
        b += 1;
    }
    t
};

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

#[cfg(test)]
fn analyze_reference(payload: &[u8]) -> Option<Doc> {
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
    let text = &text[..end];
    let bytes = text.as_bytes();
    // A word of ASCII letters and digits (most words) is cut and lower-cased in a buffer on the
    // stack, which is what `tokens` does to it; any other word goes through `tokens` itself, so
    // the terms are the same. Only a term not seen before in the text is copied.
    let mut counter = Counter::with_capacity(bytes.len());
    let mut len = 0u32;
    let mut lower = [0u8; MAX_TERM_LEN];
    let mut count = |from: usize, to: usize, ascii: bool| {
        len += 1;
        if ascii {
            let word = &bytes[from..to.min(from + MAX_TERM_LEN)];
            let lower = &mut lower[..word.len()];
            lower.copy_from_slice(word);
            lower.make_ascii_lowercase();
            counter.add(lower);
        } else {
            let term: String = tokens(&text[from..to]).collect();
            counter.add(term.as_bytes());
        }
    };
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
                count(from, i, ascii);
                start = None;
            }
            (false, None) => {}
        }
        i += width;
    }
    if let Some(from) = start {
        count(from, bytes.len(), ascii);
    }
    counter.finish(len)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn same(text: &str) {
        let (new, old) = (analyze(text.as_bytes()), analyze_reference(text.as_bytes()));
        match (&new, &old) {
            (Some(a), Some(b)) => {
                assert_eq!(a.len, b.len, "{text:?}");
                assert!(a.terms().eq(b.terms()), "{text:?}");
            }
            (None, None) => {}
            _ => panic!("{text:?}: {:?} vs {:?}", new.is_some(), old.is_some()),
        }
    }

    #[test]
    fn analyze_matches_the_reference() {
        let long = "Abc".repeat(40);
        let cases = [
            "",
            "   ",
            "fn main() { println!(\"Hello, World\"); }",
            "Straße ÉCOLE école naïve café_42 x²y",
            "don't stop: ÀÉÎ ÕÜ 東京タワー 123abc ABC",
            "mixed\u{00a0}nbsp\u{2003}em—dash…ellipsis",
            "İstanbul ΣΊΣΥΦΟΣ ǅ ﬁ ß ẞ",
            "trailing word",
            "Ωmega",
            "aΩ",
            "٣٤٥ digits ① ½",
            &long,
            &format!("{long}é{long}"),
            &format!("é{long}"),
            "a\u{0301}b combining",
            "emoji 😀wow😀 ok",
        ];
        for c in cases {
            same(c);
        }
        // Every pair of a small alphabet of ASCII and non-ASCII chars, in short strings.
        let alphabet = ['a', 'Z', '0', ' ', '_', '-', 'é', 'Σ', 'İ', '東', '·', '\u{0301}', '😀', '²'];
        for &x in &alphabet {
            for &y in &alphabet {
                for &z in &alphabet {
                    same(&format!("{x}{y}{z}{x}"));
                }
            }
        }
    }
}
