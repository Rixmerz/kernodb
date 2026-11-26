//! Posting runs: the storage of the keyword index ([`crate::fulltext`]) and of the trigram
//! index ([`crate::substring`]) since layer version 11 (ADR-0027,
//! `docs/file-format/units-v11.md`).
//!
//! # Why
//!
//! Posting blocks (ADR-0025, ADR-0026) kept one list per term under its own keys, so a change
//! set read and rewrote the tail block of every list it touched: about 1,500 lists for one
//! 3 KB text, 16 or 24 bytes per posting. A change set now writes what it adds as one *run*:
//! its lists in key order, packed in pages of about 4 KiB, with no read of what the index
//! held. Units are named by a dense *document number*, so a posting is a varint delta.
//!
//! # Records of one index
//!
//! An index is the records under one key prefix `P` (`0x74` for the keyword index,
//! `0x7a ‖ type_len ‖ type` for the trigram index of a unit type):
//!
//! * `P ‖ 0x00` the **manifest**: the next document number and the live runs, oldest first.
//! * `P ‖ 0x01 ‖ unit[16]` the **document record** of an indexed unit: its number and a
//!   family value (the length of the document in terms, for the keyword index).
//! * `P ‖ 0x02 ‖ page: u64 BE` a **number page**: the unit of each of 256 consecutive
//!   numbers, and whether the number is live. A number is given once, to one unit and one
//!   content: a unit whose content changes gets a new number and the old one dies.
//! * `P ‖ 0x03 ‖ run: u64 BE ‖ list ‖ last: u64 BE` a **run page**: consecutive lists (or
//!   parts of one) of the run, keyed by the last posting it holds.
//!
//! # A read
//!
//! The postings of a list are those of every live run, oldest first: runs hold consecutive
//! ranges of numbers, so the concatenation is ascending. In a run the page of `(list, n)` is
//! the first key at or after it. A dead number is dropped by the reader (only runs whose
//! manifest entry counts a dead number need the look), and by the next merge of its run.
//!
//! # A lost page is seen
//!
//! Every page carries the key of the page before it in its run, and the manifest the key of
//! the last page of each run. So a read that lands on a page whose predecessor ends at or
//! after the key it looked for, or past the end of a run whose last key is not below it, knows
//! a page is missing (`Corruption`), without a count record per list.
//!
//! # Merges
//!
//! Runs are merged in tiers: when a change set would leave [`MERGE_FANOUT`] runs of one
//! level at the end of the manifest, it writes them and its own postings as one run of the
//! next level instead, in the same transaction, if their pages sum to at most
//! [`MERGE_MAX_BYTES`]. `compact` merges whatever fits one transaction.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::Bound;

use kerno_core::contract::{Txn, TxnEngine};

use crate::db::{ChangeSet, Write, context, corruption};
use crate::error::{DecodeError, UnitError, UnitRule};
use crate::types::UnitId;

/// The byte after the prefix of an index: which of its records a key is.
pub(crate) const KIND_MANIFEST: u8 = 0x00;
pub(crate) const KIND_DOC: u8 = 0x01;
pub(crate) const KIND_NUM: u8 = 0x02;
pub(crate) const KIND_RUN: u8 = 0x03;

/// Numbers in one number page.
pub(crate) const SLOTS_PER_PAGE: u64 = 256;
/// Bytes of one slot of a number page: `state ‖ unit[16]`.
const SLOT_LEN: usize = 17;
/// A page is closed once its value reaches this size (small in the unit tests, so that
/// their few postings take several pages).
pub(crate) const PAGE_TARGET: usize = if cfg!(test) { 96 } else { 4096 };
/// The longest value of a run page a reader accepts.
pub(crate) const PAGE_MAX: usize = 64 * 1024;
/// Runs of one level at the end of the manifest that are merged into one of the next.
pub(crate) const MERGE_FANOUT: usize = 4;
/// Most bytes of run pages one merge at commit reads (and so about what it writes).
pub(crate) const MERGE_MAX_BYTES: u64 = 16 << 20;
/// A change set that already holds this many bytes, the merge's input included, does not
/// merge: the engine's transaction limit is 64 MiB.
const COMMIT_BUDGET: u64 = 32 << 20;
/// Most bytes of run pages one merge of `compact` reads.
pub(crate) const COMPACT_MAX_BYTES: u64 = 40 << 20;
/// Most runs in a manifest (a reader refuses more: UNIT-11).
pub(crate) const MAX_RUNS: usize = 4096;
/// Pages a merge reads per scan call.
const MERGE_SCAN: usize = 64;

const ENVELOPE: [u8; 2] = [1, 0];

/// What a family of posting runs stores.
pub(crate) trait Family {
    /// The record family, for the context of a decode error.
    const WHAT: &'static str;
    /// Whether a posting carries a term frequency and a document length.
    const VALUES: bool;
    /// The length of the list key at the start of `rest`, or `None` when `rest` does not
    /// start with one. No list key is the prefix of another.
    fn list_len(rest: &[u8]) -> Option<usize>;
}

type Get<'a> = dyn Fn(&[u8]) -> kerno_core::Result<Option<Vec<u8>>> + 'a;
type Scan<'a> = dyn Fn(Bound<&[u8]>, Bound<&[u8]>, usize) -> kerno_core::Result<Vec<(Vec<u8>, Vec<u8>)>> + 'a;

// ------------------------------------------------------------- varints

/// Appends `v` as a little-endian base-128 integer.
pub(crate) fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Reads the integer at `b[*at..]` and moves `at` past it. `None` when the bytes end inside
/// it, when it does not fit 64 bits, or when it is not the shortest encoding of its value.
#[inline]
pub(crate) fn varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *b.get(*at)?;
        *at += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        v |= u64::from(byte & 0x7F) << shift;
        if byte < 0x80 {
            return (byte != 0 || shift == 0).then_some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

fn cut(what: &str, m: &str) -> DecodeError {
    DecodeError::new(UnitRule::Unit9, format!("{what}: {m}"))
}

fn bad(what: &str, m: &str) -> DecodeError {
    DecodeError::new(UnitRule::Unit12, format!("{what}: {m}"))
}

fn need(b: &[u8], at: &mut usize, what: &str) -> Result<u64, DecodeError> {
    varint(b, at).ok_or_else(|| cut(what, "an integer is cut short or not in its shortest form"))
}

// ---------------------------------------------------------------- keys

/// `prefix ‖ kind`.
pub(crate) fn kind_key(prefix: &[u8], kind: u8) -> Vec<u8> {
    let mut k = Vec::with_capacity(prefix.len() + 1);
    k.extend_from_slice(prefix);
    k.push(kind);
    k
}

pub(crate) fn manifest_key(prefix: &[u8]) -> Vec<u8> {
    kind_key(prefix, KIND_MANIFEST)
}

pub(crate) fn doc_key(prefix: &[u8], unit: UnitId) -> Vec<u8> {
    let mut k = Vec::with_capacity(prefix.len() + 17);
    k.extend_from_slice(prefix);
    k.push(KIND_DOC);
    k.extend_from_slice(&unit.0);
    k
}

pub(crate) fn num_key(prefix: &[u8], page: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(prefix.len() + 9);
    k.extend_from_slice(prefix);
    k.push(KIND_NUM);
    k.extend_from_slice(&page.to_be_bytes());
    k
}

/// `prefix ‖ 0x03 ‖ run`: the prefix of the pages of one run.
pub(crate) fn run_prefix(prefix: &[u8], run: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(prefix.len() + 9 + 24);
    k.extend_from_slice(prefix);
    k.push(KIND_RUN);
    k.extend_from_slice(&run.to_be_bytes());
    k
}

/// The end of the keys of the run `run`: the start of the next id (run ids are below
/// `u64::MAX`, they are commit sequences).
fn run_end(prefix: &[u8], run: u64) -> Vec<u8> {
    run_prefix(prefix, run.saturating_add(1))
}

/// `list ‖ num: u64 BE`: what a page key holds after its run, and what orders postings.
pub(crate) fn suffix(list: &[u8], num: u64) -> Vec<u8> {
    let mut s = Vec::with_capacity(list.len() + 8);
    s.extend_from_slice(list);
    s.extend_from_slice(&num.to_be_bytes());
    s
}

/// The list and the number of a page key suffix.
pub(crate) fn split_suffix<F: Family>(s: &[u8]) -> Result<(&[u8], u64), DecodeError> {
    match F::list_len(s) {
        Some(n) if s.len() == n + 8 => {
            let mut num = [0u8; 8];
            num.copy_from_slice(&s[n..]);
            Ok((&s[..n], u64::from_be_bytes(num)))
        }
        _ => Err(DecodeError::new(
            UnitRule::Unit3,
            format!("{}: a run page key of {} bytes after its run has no list and number", F::WHAT, s.len()),
        )),
    }
}

/// What kind of record of an index a key is, by the bytes after the prefix.
pub(crate) enum RecordKey<'a> {
    Manifest,
    Doc(UnitId),
    Num(u64),
    /// The run and the suffix ([`split_suffix`]).
    Run(u64, &'a [u8]),
}

/// Decodes the part of a key after the prefix of its index.
pub(crate) fn decode_key<'a, F: Family>(rest: &'a [u8]) -> Result<RecordKey<'a>, DecodeError> {
    let shape =
        || DecodeError::new(UnitRule::Unit3, format!("{}: a key of {} bytes has no valid shape", F::WHAT, rest.len()));
    match rest {
        [KIND_MANIFEST] => Ok(RecordKey::Manifest),
        [KIND_DOC, unit @ ..] => Ok(RecordKey::Doc(UnitId(unit.try_into().map_err(|_| shape())?))),
        [KIND_NUM, page @ ..] => Ok(RecordKey::Num(u64::from_be_bytes(page.try_into().map_err(|_| shape())?))),
        [KIND_RUN, tail @ ..] if tail.len() > 8 => {
            let mut run = [0u8; 8];
            run.copy_from_slice(&tail[..8]);
            split_suffix::<F>(&tail[8..])?;
            Ok(RecordKey::Run(u64::from_be_bytes(run), &tail[8..]))
        }
        _ => Err(shape()),
    }
}

// ------------------------------------------------------------ envelope

/// Checks a value's envelope and returns its body. The version comes first (a newer one is
/// a newer layer, not damage), then the flags.
pub(crate) fn body<'a>(v: &'a [u8], what: &str) -> Result<&'a [u8], DecodeError> {
    crate::blocks::body(v, what)
}

// ------------------------------------------------------------ manifest

/// One live run of an index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    /// The commit sequence of the change set that wrote it.
    pub id: u64,
    /// `0` for the postings of one change set, one more for each merge above it.
    pub level: u8,
    /// The numbers of its postings are within `min..=max`.
    pub min: u64,
    pub max: u64,
    /// Its pages, their postings and the bytes of their values.
    pub pages: u64,
    pub postings: u64,
    pub bytes: u64,
    /// The numbers with a posting in it that have died since it was written.
    pub dead: u64,
    /// The suffix of the key of its last page.
    pub last: Vec<u8>,
}

/// The manifest of an index. No record is an index that never held a posting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Manifest {
    /// The next document number: every number given so far is below it.
    pub next: u64,
    /// The numbers that are live: the units the index holds.
    pub live: u64,
    /// The live runs, oldest first: ascending ids and ascending, disjoint ranges of numbers.
    pub runs: Vec<Run>,
}

impl Manifest {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(16 + self.runs.len() * 48);
        v.extend_from_slice(&ENVELOPE);
        put_varint(&mut v, self.next);
        put_varint(&mut v, self.live);
        put_varint(&mut v, self.runs.len() as u64);
        for r in &self.runs {
            put_varint(&mut v, r.id);
            v.push(r.level);
            put_varint(&mut v, r.min);
            put_varint(&mut v, r.max - r.min);
            put_varint(&mut v, r.pages);
            put_varint(&mut v, r.postings);
            put_varint(&mut v, r.bytes);
            put_varint(&mut v, r.dead);
            put_varint(&mut v, r.last.len() as u64);
            v.extend_from_slice(&r.last);
        }
        v
    }

    pub(crate) fn decode<F: Family>(v: &[u8]) -> Result<Self, DecodeError> {
        let what = F::WHAT;
        let b = body(v, what)?;
        let mut at = 0;
        let next = need(b, &mut at, what)?;
        let live = need(b, &mut at, what)?;
        if live == 0 || live > next {
            return Err(bad(what, "a manifest with no live number, or more than it gave"));
        }
        let n = need(b, &mut at, what)?;
        if n > MAX_RUNS as u64 {
            return Err(DecodeError::new(UnitRule::Unit11, format!("{what}: a manifest of {n} runs")));
        }
        let mut runs: Vec<Run> = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let id = need(b, &mut at, what)?;
            let level = *b.get(at).ok_or_else(|| cut(what, "the manifest ends inside a run"))?;
            at += 1;
            let min = need(b, &mut at, what)?;
            let span = need(b, &mut at, what)?;
            let pages = need(b, &mut at, what)?;
            let postings = need(b, &mut at, what)?;
            let bytes = need(b, &mut at, what)?;
            let dead = need(b, &mut at, what)?;
            let last_len = need(b, &mut at, what)? as usize;
            let last = b
                .get(at..at.saturating_add(last_len))
                .ok_or_else(|| cut(what, "the manifest ends inside a run"))?
                .to_vec();
            at += last_len;
            let max = min.checked_add(span).filter(|m| *m < next);
            let Some(max) = max else {
                return Err(bad(what, "a run of the manifest holds numbers that were never given"));
            };
            if pages == 0 || postings == 0 || dead > postings || id == u64::MAX {
                return Err(bad(what, "a run of the manifest has no page, no posting or more dead numbers than postings"));
            }
            split_suffix::<F>(&last).map_err(|_| bad(what, "the last key of a run of the manifest has no valid shape"))?;
            if runs.last().is_some_and(|p| p.id >= id || p.max >= min) {
                return Err(DecodeError::new(
                    UnitRule::Unit13,
                    format!("{what}: the runs of the manifest are not in ascending order"),
                ));
            }
            runs.push(Run { id, level, min, max, pages, postings, bytes, dead, last });
        }
        if at != b.len() {
            return Err(DecodeError::new(UnitRule::Unit10, format!("{what}: the manifest has trailing bytes")));
        }
        Ok(Manifest { next, live, runs })
    }

    /// The run that holds the number `num`, if one does.
    fn run_of(&mut self, num: u64) -> Option<&mut Run> {
        let at = self.runs.partition_point(|r| r.max < num);
        self.runs.get_mut(at).filter(|r| r.min <= num)
    }
}

/// The manifest of the index `prefix` as `get` sees it (none: an empty one).
pub(crate) fn read_manifest<F: Family>(get: &Get<'_>, prefix: &[u8]) -> Result<Manifest, UnitError> {
    match get(&manifest_key(prefix))? {
        Some(v) => Ok(Manifest::decode::<F>(&v).map_err(|e| context(e, "the manifest of a posting index"))?),
        None => Ok(Manifest::default()),
    }
}

// --------------------------------------------------------- doc records

/// The value of a document record: the number of the unit and the family's value.
pub(crate) fn encode_doc(num: u64, aux: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity(14);
    v.extend_from_slice(&ENVELOPE);
    v.extend_from_slice(&num.to_be_bytes());
    v.extend_from_slice(&aux.to_be_bytes());
    v
}

pub(crate) fn decode_doc(v: &[u8], what: &str) -> Result<(u64, u32), DecodeError> {
    let b = body(v, what)?;
    if b.len() < 12 {
        return Err(cut(what, "a document record is cut short"));
    }
    if b.len() > 12 {
        return Err(DecodeError::new(UnitRule::Unit10, format!("{what}: a document record has trailing bytes")));
    }
    let (num, aux) = b.split_at(8);
    Ok((u64::from_be_bytes(num.try_into().unwrap_or([0; 8])), u32::from_be_bytes(aux.try_into().unwrap_or([0; 4]))))
}

// -------------------------------------------------------- number pages

/// The slots of a number page: whether each number is live, and its unit.
pub(crate) fn decode_num(v: &[u8], what: &str) -> Result<Vec<(bool, UnitId)>, DecodeError> {
    let b = body(v, what)?;
    if b.is_empty() || !b.len().is_multiple_of(SLOT_LEN) {
        return Err(cut(what, "a number page has no whole slot, or a partial one"));
    }
    if b.len() / SLOT_LEN > SLOTS_PER_PAGE as usize {
        return Err(DecodeError::new(UnitRule::Unit11, format!("{what}: a number page of more than 256 slots")));
    }
    b.chunks_exact(SLOT_LEN)
        .map(|s| {
            let live = match s[0] {
                0 => false,
                1 => true,
                _ => return Err(bad(what, "a slot of a number page is neither live nor dead")),
            };
            Ok((live, UnitId(s[1..].try_into().unwrap_or([0; 16]))))
        })
        .collect()
}

pub(crate) fn encode_num(slots: &[(bool, UnitId)]) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + slots.len() * SLOT_LEN);
    v.extend_from_slice(&ENVELOPE);
    for (live, unit) in slots {
        v.push(u8::from(*live));
        v.extend_from_slice(&unit.0);
    }
    v
}

/// The number pages a read looked at.
#[derive(Default)]
pub(crate) struct Slots {
    pages: HashMap<u64, Vec<(bool, UnitId)>>,
}

impl Slots {
    /// Whether the number `num` of the index `prefix` is live, and its unit. A number
    /// without a slot is `Corruption`: postings and document records only name given numbers.
    pub(crate) fn slot(&mut self, get: &Get<'_>, prefix: &[u8], num: u64) -> Result<(bool, UnitId), UnitError> {
        let page = num / SLOTS_PER_PAGE;
        let slots = match self.pages.entry(page) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let slots = match get(&num_key(prefix, page))? {
                    Some(v) => decode_num(&v, "a number page").map_err(|e| context(e, "a posting index"))?,
                    None => Vec::new(),
                };
                e.insert(slots)
            }
        };
        slots
            .get((num % SLOTS_PER_PAGE) as usize)
            .copied()
            .ok_or_else(|| corruption(format!("a posting index names the document number {num}, which has no slot")))
    }
}

// ----------------------------------------------------------- run pages

/// One posting: the document number and, for a family with values, the frequency of the
/// term in the document and the length of the document in terms.
pub(crate) type Posting = (u64, u32, u32);

/// The header of a run page.
pub(crate) struct Page<'a> {
    /// Numbers of the first posting of each item are stored less this.
    pub base: u64,
    /// The suffix of the page before it in its run (empty: it is the first).
    pub prev: &'a [u8],
    /// The items.
    rest: &'a [u8],
}

/// One item of a page: a list, or the part of one the page holds.
pub(crate) struct Item<'a> {
    pub list: &'a [u8],
    /// Its postings (at least one).
    pub n: u64,
    body: &'a [u8],
    base: u64,
}

impl<'a> Page<'a> {
    pub(crate) fn decode<F: Family>(v: &'a [u8]) -> Result<Self, DecodeError> {
        let what = F::WHAT;
        if v.len() > PAGE_MAX {
            return Err(DecodeError::new(UnitRule::Unit11, format!("{what}: a run page of {} bytes", v.len())));
        }
        let b = body(v, what)?;
        let mut at = 0;
        let base = need(b, &mut at, what)?;
        let prev_len = need(b, &mut at, what)? as usize;
        let prev =
            b.get(at..at.saturating_add(prev_len)).ok_or_else(|| cut(what, "a run page ends inside its header"))?;
        if !prev.is_empty() {
            split_suffix::<F>(prev).map_err(|_| bad(what, "the previous key of a run page has no valid shape"))?;
        }
        let rest = &b[at + prev_len..];
        if rest.is_empty() {
            return Err(cut(what, "a run page with no item"));
        }
        Ok(Page { base, prev, rest })
    }

    /// The next item, or `None` at the end of the page.
    pub(crate) fn next<F: Family>(&mut self) -> Result<Option<Item<'a>>, DecodeError> {
        let what = F::WHAT;
        if self.rest.is_empty() {
            return Ok(None);
        }
        let b = self.rest;
        let list_len = F::list_len(b).ok_or_else(|| bad(what, "an item of a run page does not start with a list key"))?;
        let mut at = list_len;
        let n = need(b, &mut at, what)?;
        let len = need(b, &mut at, what)? as usize;
        let body = b.get(at..at.saturating_add(len)).ok_or_else(|| cut(what, "a run page ends inside an item"))?;
        if n == 0 {
            return Err(cut(what, "an item of a run page has no posting"));
        }
        self.rest = &b[at + len..];
        Ok(Some(Item { list: &b[..list_len], n, body, base: self.base }))
    }
}

impl Item<'_> {
    /// Calls `emit` on each posting, ascending, and returns the number of the last.
    pub(crate) fn postings<F: Family>(&self, mut emit: impl FnMut(Posting)) -> Result<u64, DecodeError> {
        let what = F::WHAT;
        let b = self.body;
        let mut at = 0;
        let mut num = 0u64;
        for i in 0..self.n {
            let d = need(b, &mut at, what)?;
            num = if i == 0 {
                self.base.checked_add(d).ok_or_else(|| bad(what, "a document number over 64 bits"))?
            } else {
                if d == 0 {
                    return Err(DecodeError::new(
                        UnitRule::Unit13,
                        format!("{what}: document numbers not strictly ascending"),
                    ));
                }
                num.checked_add(d).ok_or_else(|| bad(what, "a document number over 64 bits"))?
            };
            let (tf, dl) = if F::VALUES {
                let tf = need(b, &mut at, what)?;
                let dl = need(b, &mut at, what)?;
                if tf == 0 || dl < tf || dl > u64::from(u32::MAX) {
                    return Err(bad(what, "a posting with a frequency of 0 or above its document length"));
                }
                (tf as u32, dl as u32)
            } else {
                (0, 0)
            };
            emit((num, tf, dl));
        }
        if at != b.len() {
            return Err(DecodeError::new(UnitRule::Unit10, format!("{what}: an item of a run page has trailing bytes")));
        }
        Ok(num)
    }
}

/// What a whole page holds, checked in full: for the verifier and the merge.
pub(crate) struct Checked {
    pub prev: Vec<u8>,
    /// `(start of the list key in `lists`, its length, its postings in `postings`)` per item.
    pub items: Vec<(usize, usize, std::ops::Range<usize>)>,
    pub lists: Vec<u8>,
    pub postings: Vec<Posting>,
}

impl Checked {
    pub(crate) fn list(&self, item: usize) -> &[u8] {
        let (at, len, _) = &self.items[item];
        &self.lists[*at..*at + *len]
    }
}

/// Decodes every item of the page `v` stored under the key suffix `key`: the lists strictly
/// ascending, the numbers of each ascending, and the key that of the last posting.
pub(crate) fn check_page<F: Family>(key: &[u8], v: &[u8]) -> Result<Checked, DecodeError> {
    let what = F::WHAT;
    let mut page = Page::decode::<F>(v)?;
    let mut out = Checked { prev: page.prev.to_vec(), items: Vec::new(), lists: Vec::new(), postings: Vec::new() };
    let mut last = 0u64;
    while let Some(item) = page.next::<F>()? {
        if out.items.last().is_some_and(|(at, len, _)| &out.lists[*at..*at + *len] >= item.list) {
            return Err(DecodeError::new(UnitRule::Unit13, format!("{what}: the lists of a run page do not ascend")));
        }
        let from = out.postings.len();
        last = item.postings::<F>(|p| out.postings.push(p))?;
        out.items.push((out.lists.len(), item.list.len(), from..out.postings.len()));
        out.lists.extend_from_slice(item.list);
    }
    let (list, num) = split_suffix::<F>(key)?;
    if out.items.last().is_none_or(|(at, len, _)| &out.lists[*at..*at + *len] != list) || num != last {
        return Err(bad(what, "the key of a run page is not that of its last posting"));
    }
    if !out.prev.is_empty() && out.prev.as_slice() >= key {
        return Err(DecodeError::new(UnitRule::Unit13, format!("{what}: a run page is not above the page before it")));
    }
    Ok(out)
}

// -------------------------------------------------------------- builder

/// One page of a run about to be written: its key suffix and its value.
type BuiltPage = (Vec<u8>, Vec<u8>);

/// Builds the pages of a run from its lists, given in ascending order.
pub(crate) struct RunBuilder {
    values: bool,
    base: u64,
    pages: Vec<BuiltPage>,
    /// The page being filled (empty: none started) and the suffix of the one before it.
    page: Vec<u8>,
    prev: Vec<u8>,
    /// The list being written, its postings in the open item, and their count.
    list: Vec<u8>,
    item: Vec<u8>,
    n: u64,
    last: u64,
    /// The suffix of the last posting closed into `page`.
    page_last: Vec<u8>,
    postings: u64,
    bytes: u64,
}

/// A run built in memory.
pub(crate) struct Built {
    pub(crate) pages: Vec<BuiltPage>,
    postings: u64,
    bytes: u64,
}

impl RunBuilder {
    /// A builder of a run whose numbers are all at or above `base`.
    pub(crate) fn new<F: Family>(base: u64) -> Self {
        RunBuilder {
            values: F::VALUES,
            base,
            pages: Vec::new(),
            page: Vec::new(),
            prev: Vec::new(),
            list: Vec::new(),
            item: Vec::new(),
            n: 0,
            last: 0,
            page_last: Vec::new(),
            postings: 0,
            bytes: 0,
        }
    }

    /// A builder whose first page names `prev` as the page before it.
    #[cfg(test)]
    pub(crate) fn after(mut self, prev: &[u8]) -> Self {
        self.prev = prev.to_vec();
        self
    }

    /// Starts the list `list`, above every list given so far.
    pub(crate) fn begin(&mut self, list: &[u8]) {
        self.close_item();
        self.list.clear();
        self.list.extend_from_slice(list);
    }

    /// Adds a posting to the list begun, above every posting of it given so far.
    #[inline]
    pub(crate) fn push(&mut self, (num, tf, dl): Posting) {
        put_varint(&mut self.item, if self.n == 0 { num - self.base } else { num - self.last });
        if self.values {
            put_varint(&mut self.item, u64::from(tf));
            put_varint(&mut self.item, u64::from(dl));
        }
        self.n += 1;
        self.last = num;
        if self.page.len() + self.list.len() + self.item.len() + 6 >= PAGE_TARGET {
            self.close_item();
            self.close_page();
        }
    }

    fn close_item(&mut self) {
        if self.n == 0 {
            return;
        }
        if self.page.is_empty() {
            self.page.reserve(PAGE_TARGET + 64);
            self.page.extend_from_slice(&ENVELOPE);
            put_varint(&mut self.page, self.base);
            put_varint(&mut self.page, self.prev.len() as u64);
            self.page.extend_from_slice(&self.prev);
        }
        self.page.extend_from_slice(&self.list);
        put_varint(&mut self.page, self.n);
        put_varint(&mut self.page, self.item.len() as u64);
        self.page.extend_from_slice(&self.item);
        self.page_last.clear();
        self.page_last.extend_from_slice(&self.list);
        self.page_last.extend_from_slice(&self.last.to_be_bytes());
        self.postings += self.n;
        self.item.clear();
        self.n = 0;
    }

    fn close_page(&mut self) {
        if self.page.is_empty() {
            return;
        }
        self.bytes += self.page.len() as u64;
        self.prev.clone_from(&self.page_last);
        self.pages.push((std::mem::take(&mut self.page_last), std::mem::take(&mut self.page)));
    }

    /// The run, or `None` when no posting was given.
    pub(crate) fn finish(mut self) -> Option<Built> {
        self.close_item();
        self.close_page();
        (!self.pages.is_empty()).then_some(Built { pages: self.pages, postings: self.postings, bytes: self.bytes })
    }
}

// --------------------------------------------------------------- reader

/// Reads postings of the list `list` in the run `run` of the index `prefix`: those with a
/// number at or above `from` when the read starts (`resume` is `None`), or those after the
/// page `resume` (the key a previous call returned). At most `max_pages` pages are read;
/// `Some(key)` is returned when the list may go on after the last one.
///
/// A page missing where the list must be is `Corruption`: see the [module
/// documentation](self).
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_list<F: Family>(
    scan: &Scan<'_>,
    prefix: &[u8],
    run: &Run,
    list: &[u8],
    from: u64,
    resume: Option<Vec<u8>>,
    max_pages: usize,
    emit: &mut dyn FnMut(Posting),
) -> Result<Option<Vec<u8>>, UnitError> {
    let target = suffix(list, from);
    read_span::<F>(scan, prefix, run, &target, &|l| l.cmp(list), from, resume, max_pages, emit)
}

/// Reads every posting of the lists of `run` whose key starts with `lists`.
pub(crate) fn read_prefix<F: Family>(
    scan: &Scan<'_>,
    prefix: &[u8],
    run: &Run,
    lists: &[u8],
    emit: &mut dyn FnMut(Posting),
) -> Result<(), UnitError> {
    let within = |l: &[u8]| if l.starts_with(lists) { std::cmp::Ordering::Equal } else { l.cmp(lists) };
    read_span::<F>(scan, prefix, run, lists, &within, 0, None, usize::MAX, emit).map(|_| ())
}

/// The read behind [`read_list`] and [`read_prefix`]: the postings, at or above `from`, of
/// the lists `within` calls `Equal` (a range of lists: `Less` below it, `Greater` above),
/// starting at the page of `target`, the lowest page key suffix the range can have.
#[allow(clippy::too_many_arguments)]
fn read_span<F: Family>(
    scan: &Scan<'_>,
    prefix: &[u8],
    run: &Run,
    target: &[u8],
    within: &dyn Fn(&[u8]) -> std::cmp::Ordering,
    from: u64,
    resume: Option<Vec<u8>>,
    max_pages: usize,
    emit: &mut dyn FnMut(Posting),
) -> Result<Option<Vec<u8>>, UnitError> {
    use std::cmp::Ordering;
    let lost = || corruption(format!("{}: a page of the run {} is missing", F::WHAT, run.id));
    let runp = run_prefix(prefix, run.id);
    let end = run_end(prefix, run.id);
    let mut budget = max_pages;
    // The first page of a run is asked for alone: most lists fit one page.
    let (mut lo, mut first) = match resume {
        Some(key) => (Bound::Excluded(key), false),
        None => {
            let mut k = runp.clone();
            k.extend_from_slice(target);
            (Bound::Included(k), true)
        }
    };
    // The suffix of the page read last, when one was: the next must name it.
    let mut before: Option<Vec<u8>> = match &lo {
        Bound::Excluded(k) => Some(k.get(runp.len()..).unwrap_or_default().to_vec()),
        _ => None,
    };
    while budget > 0 {
        let ask = if first { 1 } else { budget.min(16) };
        let lo_ref = match &lo {
            Bound::Included(k) => Bound::Included(k.as_slice()),
            Bound::Excluded(k) => Bound::Excluded(k.as_slice()),
            Bound::Unbounded => Bound::Unbounded,
        };
        let pages = scan(lo_ref, Bound::Excluded(&end), ask)?;
        let got = pages.len();
        for (k, v) in pages {
            let key = k.get(runp.len()..).unwrap_or_default();
            let mut page = Page::decode::<F>(&v).map_err(|e| context(e, "a run page"))?;
            match &before {
                // The page found for a key holds it only if the page before it ends below it.
                None if page.prev >= target => return Err(lost()),
                Some(b) if page.prev != b.as_slice() => return Err(lost()),
                _ => {}
            }
            while let Some(item) = page.next::<F>().map_err(|e| context(e, "a run page"))? {
                match within(item.list) {
                    Ordering::Less => {}
                    Ordering::Equal => {
                        item.postings::<F>(|p| {
                            if p.0 >= from {
                                emit(p);
                            }
                        })
                        .map_err(|e| context(e, "a run page"))?;
                    }
                    Ordering::Greater => return Ok(None),
                }
            }
            // The page ended inside the range, or with it: only the next page tells.
            if key == run.last.as_slice() {
                return Ok(None);
            }
            let (last_list, _) = split_suffix::<F>(key).map_err(|e| context(e, "a run page"))?;
            if within(last_list) != Ordering::Equal {
                // The key says the page ends above the range, yet no item of it was.
                return Err(corruption(format!("{}: the key of a run page is not that of its last list", F::WHAT)));
            }
            before = Some(key.to_vec());
            lo = Bound::Excluded(k);
            budget -= 1;
            if budget == 0 {
                break;
            }
        }
        if got < ask {
            // The run ended. With no page read at all the range is not in the run, unless
            // the run's last page (which the manifest names) should have been found.
            return match &before {
                None if run.last.as_slice() < target => Ok(None),
                _ => Err(lost()),
            };
        }
        first = false;
    }
    match lo {
        Bound::Excluded(k) => Ok(Some(k)),
        _ => Ok(None),
    }
}

// --------------------------------------------------------------- writer

/// What a change set will change in one index: the numbers it gives and the numbers that
/// die. The postings of the new numbers are kept by the family and given to [`flush`].
pub(crate) struct Pending {
    pub(crate) prefix: Vec<u8>,
    manifest: Manifest,
    /// The units of the numbers given: `manifest.next + i`.
    slots: Vec<UnitId>,
    kills: Vec<u64>,
}

impl Pending {
    /// The index `prefix` as the transaction holds it.
    pub(crate) fn load<T: Txn, F: Family>(txn: &T, prefix: Vec<u8>) -> Result<Self, UnitError> {
        let manifest = read_manifest::<F>(&|k| txn.get(k), &prefix)?;
        Ok(Pending { prefix, manifest, slots: Vec::new(), kills: Vec::new() })
    }

    /// The first number this change set gives.
    pub(crate) fn base(&self) -> u64 {
        self.manifest.next
    }

    /// Gives `unit` the next number.
    pub(crate) fn alloc(&mut self, unit: UnitId) -> u64 {
        self.slots.push(unit);
        self.manifest.next + self.slots.len() as u64 - 1
    }

    /// The number `num` dies.
    pub(crate) fn kill(&mut self, num: u64) {
        self.kills.push(num);
    }

    /// The live runs as the change set found them.
    pub(crate) fn runs(&self) -> &[Run] {
        &self.manifest.runs
    }
}

/// How many runs at the end of `runs` a change set merges with its own postings, and the
/// level of the run it writes.
fn merge_plan(runs: &[Run], cap: u64, held: u64) -> (usize, u8) {
    let (mut start, mut level) = (runs.len(), 0u8);
    loop {
        let mut k = start;
        while k > 0 && runs[k - 1].level <= level {
            k -= 1;
        }
        let bytes: u64 = runs[k..].iter().map(|r| r.bytes).sum();
        if start - k + 1 >= MERGE_FANOUT && bytes <= cap && held.saturating_add(bytes) <= COMMIT_BUDGET {
            start = k;
            level = level.saturating_add(1);
        } else {
            return (start, level);
        }
    }
}

/// Where `compact` starts merging: the runs from the newest back whose pages sum to at most
/// [`COMPACT_MAX_BYTES`], when they are several, or one that holds dead numbers.
pub(crate) fn compact_start(runs: &[Run]) -> Option<usize> {
    let mut start = runs.len();
    let mut bytes = 0u64;
    while start > 0 && bytes + runs[start - 1].bytes <= COMPACT_MAX_BYTES {
        start -= 1;
        bytes += runs[start].bytes;
    }
    match &runs[start..] {
        [] => None,
        [one] if one.dead == 0 => None,
        _ => Some(start),
    }
}

/// The pages of one run being merged, read in order and checked as they come.
struct Cursor<'a> {
    run: &'a Run,
    runp: Vec<u8>,
    end: Vec<u8>,
    /// Pages scanned and not yet opened.
    buffered: VecDeque<(Vec<u8>, Vec<u8>)>,
    /// The key of the last page scanned (`None`: none yet), and whether the scan ended.
    after: Option<Vec<u8>>,
    done: bool,
    /// The page open, the item it is at, and the suffix of the page before it.
    page: Option<Checked>,
    item: usize,
    prev: Vec<u8>,
    seen: u64,
}

impl<'a> Cursor<'a> {
    fn new(prefix: &[u8], run: &'a Run) -> Self {
        Cursor {
            run,
            runp: run_prefix(prefix, run.id),
            end: run_end(prefix, run.id),
            buffered: VecDeque::new(),
            after: None,
            done: false,
            page: None,
            item: 0,
            prev: Vec::new(),
            seen: 0,
        }
    }

    /// Makes the next item current. `false` at the end of the run, which is then checked
    /// against the manifest: its pages and its last key.
    fn advance<F: Family>(&mut self, scan: &Scan<'_>) -> Result<bool, UnitError> {
        if let Some(p) = &self.page {
            self.item += 1;
            if self.item < p.items.len() {
                return Ok(true);
            }
        }
        if self.buffered.is_empty() && !self.done {
            let lo = match &self.after {
                Some(k) => Bound::Excluded(k.as_slice()),
                None => Bound::Included(self.runp.as_slice()),
            };
            let pages = scan(lo, Bound::Excluded(&self.end), MERGE_SCAN)?;
            self.done = pages.len() < MERGE_SCAN;
            if let Some((k, _)) = pages.last() {
                self.after = Some(k.clone());
            }
            self.buffered.extend(pages);
        }
        let lost = |run: &Run| corruption(format!("{}: a page of the run {} is missing", F::WHAT, run.id));
        let Some((k, v)) = self.buffered.pop_front() else {
            self.page = None;
            if self.seen != self.run.pages || self.prev != self.run.last {
                return Err(lost(self.run));
            }
            return Ok(false);
        };
        let key = &k[self.runp.len()..];
        let page = check_page::<F>(key, &v).map_err(|e| context(e, "a run page"))?;
        if page.prev != self.prev {
            return Err(lost(self.run));
        }
        self.prev = key.to_vec();
        self.seen += 1;
        self.page = Some(page);
        self.item = 0;
        Ok(true)
    }

    fn list(&self) -> Option<&[u8]> {
        self.page.as_ref().map(|p| p.list(self.item))
    }

    fn postings(&self) -> &[Posting] {
        match &self.page {
            Some(p) => &p.postings[p.items[self.item].2.clone()],
            None => &[],
        }
    }
}

/// The lists a change set kept for one index, in ascending key order, for [`flush`].
pub(crate) trait Staged {
    /// The key of the next staged list, if one is left.
    fn peek(&mut self) -> Option<&[u8]>;
    /// Gives the postings of the next staged list to `emit`, ascending, and moves past it.
    fn take(&mut self, emit: &mut dyn FnMut(Posting));
}

/// Writes what the change set kept for one index: the number pages of the numbers given and
/// of those that died, its postings as a run (merged with the runs of [`merge_plan`], or
/// with every run that fits when `compact`), and the manifest. Called by `commit`, once
/// per index.
pub(crate) fn flush<E: TxnEngine, F: Family>(
    cs: &mut ChangeSet<'_, E>,
    pending: Pending,
    staged: &mut dyn Staged,
    compact: bool,
) -> Result<(), UnitError> {
    let Pending { prefix, mut manifest, slots, mut kills } = pending;
    if slots.is_empty() && kills.is_empty() && !compact {
        return Ok(());
    }
    let base = manifest.next;
    let next = base + slots.len() as u64;
    kills.sort_unstable();
    kills.dedup();
    let seq = cs.commit_seq().0;
    let held = cs.bytes_so_far() as u64;

    // The number pages: those of the numbers given, and those of the numbers that die.
    let mut pages: BTreeMap<u64, Vec<(bool, UnitId)>> = BTreeMap::new();
    let mut dirty: Vec<u64> = Vec::new();
    let existed = base > 0 || !manifest.runs.is_empty();
    let writes = {
        let txn = cs.txn()?;
        let load = |page: u64, pages: &mut BTreeMap<u64, Vec<(bool, UnitId)>>| -> Result<(), UnitError> {
            if !pages.contains_key(&page) {
                let slots = match txn.get(&num_key(&prefix, page))? {
                    Some(v) => decode_num(&v, "a number page").map_err(|e| context(e, "a posting index"))?,
                    None => Vec::new(),
                };
                pages.insert(page, slots);
            }
            Ok(())
        };
        if !slots.is_empty() {
            let first = base / SLOTS_PER_PAGE;
            if base % SLOTS_PER_PAGE != 0 {
                load(first, &mut pages)?;
            }
            for (i, unit) in slots.iter().enumerate() {
                let num = base + i as u64;
                let page = pages.entry(num / SLOTS_PER_PAGE).or_default();
                if page.len() as u64 != num % SLOTS_PER_PAGE {
                    return Err(corruption(format!(
                        "{}: the number pages do not end at the next number of the manifest",
                        F::WHAT
                    )));
                }
                page.push((true, *unit));
            }
            dirty.extend(first..=(next - 1) / SLOTS_PER_PAGE);
        }
        let mut new_dead = 0u64;
        for &num in &kills {
            let page = num / SLOTS_PER_PAGE;
            load(page, &mut pages)?;
            match pages.get_mut(&page).and_then(|p| p.get_mut((num % SLOTS_PER_PAGE) as usize)) {
                Some(slot) if slot.0 => slot.0 = false,
                _ => {
                    return Err(corruption(format!(
                        "{}: a document record names the number {num}, which is not live",
                        F::WHAT
                    )));
                }
            }
            if dirty.last() != Some(&page) && !dirty.contains(&page) {
                dirty.push(page);
            }
            if num >= base {
                new_dead += 1;
            } else {
                match manifest.run_of(num) {
                    Some(run) => run.dead += 1,
                    None => {
                        return Err(corruption(format!("{}: the live number {num} is in no run", F::WHAT)));
                    }
                }
            }
        }

        // The run: this change set's postings, and the runs it merges them with.
        let (start, level) = if compact {
            match compact_start(&manifest.runs) {
                Some(start) => {
                    let level = manifest.runs[start..].iter().map(|r| r.level).max().unwrap_or(0);
                    (start, level.saturating_add(1))
                }
                None => (manifest.runs.len(), 0),
            }
        } else if slots.is_empty() {
            (manifest.runs.len(), 0)
        } else {
            merge_plan(&manifest.runs, MERGE_MAX_BYTES, held)
        };
        let inputs = &manifest.runs[start..];
        let run_base = inputs.first().map_or(base, |r| r.min);
        let mut builder = RunBuilder::new::<F>(run_base);
        let scan = |lo: Bound<&[u8]>, hi: Bound<&[u8]>, n: usize| txn.scan(lo, hi, n);
        // Whether a number is live: the pages loaded above hold what this change set did to
        // them, and any other page is as the transaction holds it.
        let mut other = Slots::default();
        let mut live = |num: u64| -> Result<bool, UnitError> {
            match pages.get(&(num / SLOTS_PER_PAGE)) {
                Some(p) => p
                    .get((num % SLOTS_PER_PAGE) as usize)
                    .map(|s| s.0)
                    .ok_or_else(|| corruption(format!("{}: a posting names the number {num}, which has no slot", F::WHAT))),
                None => Ok(other.slot(&|k| txn.get(k), &prefix, num)?.0),
            }
        };
        let mut cursors: Vec<Cursor<'_>> = Vec::with_capacity(inputs.len());
        for run in inputs {
            let mut c = Cursor::new(&prefix, run);
            c.advance::<F>(&scan)?;
            cursors.push(c);
        }
        let mut list: Vec<u8> = Vec::new();
        let mut failed: Option<UnitError> = None;
        loop {
            // The lowest list any source is at.
            list.clear();
            let mut any = false;
            for c in &cursors {
                if let Some(l) = c.list()
                    && (!any || l < list.as_slice())
                {
                    list.clear();
                    list.extend_from_slice(l);
                    any = true;
                }
            }
            let staged_here = match staged.peek() {
                Some(l) if !any || l <= list.as_slice() => {
                    if !any || l < list.as_slice() {
                        list.clear();
                        list.extend_from_slice(l);
                    }
                    any = true;
                    true
                }
                _ => false,
            };
            if !any {
                break;
            }
            builder.begin(&list);
            // Oldest run first: their numbers ascend from run to run.
            for c in &mut cursors {
                while c.list() == Some(list.as_slice()) {
                    if c.run.dead == 0 {
                        for p in c.postings() {
                            builder.push(*p);
                        }
                    } else {
                        for p in c.postings() {
                            if live(p.0)? {
                                builder.push(*p);
                            }
                        }
                    }
                    c.advance::<F>(&scan)?;
                }
            }
            if staged_here {
                if new_dead == 0 {
                    staged.take(&mut |p| builder.push(p));
                } else {
                    staged.take(&mut |p| match live(p.0) {
                        Ok(true) => builder.push(p),
                        Ok(false) => {}
                        Err(e) => failed = Some(e),
                    });
                    if let Some(e) = failed.take() {
                        return Err(e);
                    }
                }
            }
        }
        let merged: Vec<u64> = inputs.iter().map(|r| r.id).collect();
        let max = if slots.is_empty() { inputs.last().map_or(0, |r| r.max) } else { next - 1 };
        drop(cursors);
        let built = builder.finish();
        manifest.runs.truncate(start);
        manifest.next = next;
        manifest.live = (manifest.live + slots.len() as u64).saturating_sub(kills.len() as u64);
        if let Some(b) = &built {
            let last = b.pages.last().map(|(k, _)| k.clone()).unwrap_or_default();
            manifest.runs.push(Run {
                id: seq,
                level,
                min: run_base,
                max,
                pages: b.pages.len() as u64,
                postings: b.postings,
                bytes: b.bytes,
                // The dead numbers of the runs merged, and of this change set, were dropped.
                dead: 0,
                last,
            });
        }

        // Writes, after every read of the flush.
        let mut writes: Vec<Write> = Vec::with_capacity(dirty.len() + merged.len() + 2);
        if manifest.live == 0 || manifest.runs.is_empty() {
            // Nothing is indexed any more: no number is live, so the index goes back to
            // nothing, and numbers start again at 0. (Every live number has a posting in a
            // run, so no run means no live number.)
            if existed {
                let (from, to) = (kind_key(&prefix, KIND_NUM), kind_key(&prefix, KIND_RUN + 1));
                writes.push(Write::DeleteRange(from, to));
                writes.push(Write::Delete(manifest_key(&prefix)));
            }
        } else {
            for id in merged {
                writes.push(Write::DeleteRange(run_prefix(&prefix, id), run_end(&prefix, id)));
            }
            dirty.sort_unstable();
            for page in dirty {
                if let Some(slots) = pages.get(&page) {
                    writes.push(Write::Put(num_key(&prefix, page), encode_num(slots)));
                }
            }
            if let Some(b) = built {
                let runp = run_prefix(&prefix, seq);
                for (key, value) in b.pages {
                    let mut k = Vec::with_capacity(runp.len() + key.len());
                    k.extend_from_slice(&runp);
                    k.extend_from_slice(&key);
                    writes.push(Write::Put(k, value));
                }
            }
            writes.push(Write::Put(manifest_key(&prefix), manifest.encode()));
        }
        writes
    };
    for w in writes {
        cs.write_owned(w)?;
    }
    Ok(())
}

// -------------------------------------------------------------- compact

/// No staged list: a merge of stored runs alone.
struct Nothing;

impl Staged for Nothing {
    fn peek(&mut self) -> Option<&[u8]> {
        None
    }

    fn take(&mut self, _: &mut dyn FnMut(Posting)) {}
}

impl<E: TxnEngine> crate::db::UnitDb<E> {
    /// Merges the runs of the keyword index and of the trigram index of every type, as far
    /// as one transaction per merge allows ([`COMPACT_MAX_BYTES`] of run pages): the newest
    /// runs of each index that fit become one run, without the postings of dead numbers.
    /// Each merge is its own change set (purpose `Index`); an index with nothing to merge
    /// commits nothing. Returns the number of merges. `compact` calls it.
    ///
    /// A read consults every live run of an index, so this is what makes reads of an index
    /// written by many small change sets as cheap as they get.
    ///
    /// # Errors
    ///
    /// The engine fails, or a record of an index is damaged (`Storage(Corruption)`).
    pub fn merge_posting_runs(&self) -> Result<u64, UnitError> {
        self.check_poisoned()?;
        let mut merges = 0;
        while self.merge_index::<crate::fulltext::Postings>(&crate::fulltext::PREFIX)? {
            merges += 1;
        }
        for unit_type in self.snapshot()?.substring_types()? {
            let prefix = crate::substring::type_prefix(&unit_type);
            while self.merge_index::<crate::substring::Trigrams>(&prefix)? {
                merges += 1;
            }
        }
        Ok(merges)
    }

    /// One merge of the index `prefix`, if [`compact_start`] finds one to make.
    fn merge_index<F: Family>(&self, prefix: &[u8]) -> Result<bool, UnitError> {
        use kerno_core::contract::Snapshot;
        let snap = self.snapshot()?;
        let manifest = read_manifest::<F>(&|k| snap.snap.get(k), prefix)?;
        if compact_start(&manifest.runs).is_none() {
            return Ok(false);
        }
        drop(snap);
        let mut cs = ChangeSet::begin_index(self)?;
        let pending = Pending::load::<_, F>(cs.txn()?, prefix.to_vec())?;
        // Read again inside the transaction: another writer may have merged since.
        if compact_start(pending.runs()).is_none() {
            return Ok(false);
        }
        flush::<E, F>(&mut cs, pending, &mut Nothing, true)?;
        cs.commit()?;
        Ok(true)
    }
}

// ------------------------------------------------------------- verifier

/// What the verifier saw of one run.
#[derive(Default)]
struct RunSeen {
    pages: u64,
    postings: u64,
    bytes: u64,
    /// The suffix of the last page seen.
    last: Vec<u8>,
    /// Dead numbers with a posting in the run.
    dead: std::collections::BTreeSet<u64>,
    /// A page did not name the page before it, or held a number outside the run's range.
    broken: bool,
}

/// The self-check of one index (the records under one prefix), for `verify_units`: the
/// verifier hands it every record of the index in key order ([`Check::record`]), so the
/// manifest, the document records and the number pages are known when the run pages come.
/// After the walk [`Check::findings`] tells what disagrees.
#[derive(Default)]
pub(crate) struct Check {
    prefix: Vec<u8>,
    first_key: Option<Vec<u8>>,
    manifest: Option<Manifest>,
    /// The document records: unit to number and family value.
    pub(crate) docs: BTreeMap<UnitId, (u64, u32)>,
    /// The slots of the number pages seen, by number, while the pages are consecutive.
    slots: Vec<(bool, UnitId)>,
    /// The number pages are not `0, 1, 2, …`, each full but the last.
    gap: bool,
    runs: BTreeMap<u64, RunSeen>,
    /// A run page of a run the manifest does not list: `(key, run)`, the first of each.
    stray: BTreeMap<u64, Vec<u8>>,
    /// A posting names a number with no slot: the key of the first such page.
    unknown: Option<Vec<u8>>,
}

impl Check {
    pub(crate) fn new(prefix: &[u8]) -> Self {
        Check { prefix: prefix.to_vec(), ..Check::default() }
    }

    /// One record of the index, met in key order. `posting` is called with the list, the
    /// unit and the values of every posting of a live number in a live run.
    pub(crate) fn record<F: Family>(
        &mut self,
        k: &[u8],
        v: &[u8],
        posting: &mut dyn FnMut(&[u8], UnitId, u32, u32),
    ) -> Result<(), DecodeError> {
        self.first_key.get_or_insert_with(|| k.to_vec());
        match decode_key::<F>(k.get(self.prefix.len()..).unwrap_or_default())? {
            RecordKey::Manifest => self.manifest = Some(Manifest::decode::<F>(v)?),
            RecordKey::Doc(unit) => {
                self.docs.insert(unit, decode_doc(v, F::WHAT)?);
            }
            RecordKey::Num(page) => {
                let slots = decode_num(v, F::WHAT)?;
                if page.checked_mul(SLOTS_PER_PAGE) != Some(self.slots.len() as u64) {
                    self.gap = true;
                } else {
                    self.slots.extend(slots);
                }
            }
            RecordKey::Run(run, suffix) => {
                let page = check_page::<F>(suffix, v)?;
                let Some(listed) = self.manifest.as_ref().and_then(|m| m.runs.iter().find(|r| r.id == run)) else {
                    self.stray.entry(run).or_insert_with(|| k.to_vec());
                    return Ok(());
                };
                let seen = self.runs.entry(run).or_default();
                seen.broken |= page.prev != seen.last;
                seen.pages += 1;
                seen.bytes += v.len() as u64;
                seen.postings += page.postings.len() as u64;
                seen.last = suffix.to_vec();
                for item in 0..page.items.len() {
                    let list = page.list(item);
                    for (num, tf, dl) in &page.postings[page.items[item].2.clone()] {
                        seen.broken |= *num < listed.min || *num > listed.max;
                        match usize::try_from(*num).ok().and_then(|n| self.slots.get(n)) {
                            Some((true, unit)) => posting(list, *unit, *tf, *dl),
                            Some((false, _)) => {
                                seen.dead.insert(*num);
                            }
                            None => {
                                self.unknown.get_or_insert_with(|| k.to_vec());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The first key of the index, if it has a record.
    pub(crate) fn first_key(&self) -> Option<&[u8]> {
        self.first_key.as_deref()
    }

    /// `(key, detail)` of every disagreement between the records of the index: `what` names
    /// the index in a detail.
    pub(crate) fn findings(&self, what: &str) -> Vec<(Vec<u8>, String)> {
        let mut out = Vec::new();
        let Some(first) = &self.first_key else {
            return out;
        };
        let Some(manifest) = &self.manifest else {
            out.push((first.clone(), format!("{what}: records of an index without its manifest")));
            return out;
        };
        let mkey = manifest_key(&self.prefix);
        if self.gap || self.slots.len() as u64 != manifest.next {
            out.push((
                mkey.clone(),
                format!(
                    "{what}: the number pages hold {} numbers in a row, the manifest gave {}",
                    self.slots.len(),
                    manifest.next
                ),
            ));
        }
        let live_slots = self.slots.iter().filter(|s| s.0).count();
        if !self.gap && live_slots as u64 != manifest.live {
            out.push((
                mkey.clone(),
                format!("{what}: {live_slots} numbers are live, the manifest counts {}", manifest.live),
            ));
        }
        // Every document record names a live number of its unit, and every live number has one.
        let mut live = live_slots;
        for (unit, (num, _)) in &self.docs {
            match usize::try_from(*num).ok().and_then(|n| self.slots.get(n)) {
                Some((true, u)) if u == unit => live = live.saturating_sub(1),
                _ => out.push((
                    doc_key(&self.prefix, *unit),
                    format!("{what}: the document record of unit {unit} names a number that is not its live number"),
                )),
            }
        }
        if live != 0 && !self.gap {
            for (num, (_, unit)) in self.slots.iter().enumerate().filter(|(_, s)| s.0) {
                if self.docs.get(unit).is_none_or(|(n, _)| *n != num as u64) {
                    out.push((
                        num_key(&self.prefix, num as u64 / SLOTS_PER_PAGE),
                        format!("{what}: the live number {num} of unit {unit} has no document record"),
                    ));
                }
            }
        }
        for run in &manifest.runs {
            let seen = self.runs.get(&run.id);
            let same = seen.is_some_and(|s| {
                !s.broken
                    && s.pages == run.pages
                    && s.postings == run.postings
                    && s.bytes == run.bytes
                    && s.last == run.last
                    && s.dead.len() as u64 == run.dead
            });
            if !same {
                out.push((
                    run_prefix(&self.prefix, run.id),
                    format!(
                        "{what}: the pages of the run {} disagree with the manifest (a page is missing, added or changed)",
                        run.id
                    ),
                ));
            }
        }
        for (run, key) in &self.stray {
            out.push((key.clone(), format!("{what}: a page of the run {run}, which the manifest does not list")));
        }
        if let Some(key) = &self.unknown {
            out.push((key.clone(), format!("{what}: a posting names a number that was never given")));
        }
        out
    }
}

// ---------------------------------------------------------- test support

/// The postings of an index by list, as its live runs hold them (dead numbers included), with
/// the manifest: for the tests of the families.
#[cfg(test)]
pub(crate) fn dump<S: kerno_core::contract::Snapshot, F: Family>(
    snap: &S,
    prefix: &[u8],
) -> (Manifest, BTreeMap<Vec<u8>, Vec<Posting>>) {
    let manifest = read_manifest::<F>(&|k| snap.get(k), prefix).unwrap();
    let mut lists: BTreeMap<Vec<u8>, Vec<Posting>> = BTreeMap::new();
    for run in &manifest.runs {
        let (runp, end) = (run_prefix(prefix, run.id), run_end(prefix, run.id));
        for (k, v) in snap.scan(Bound::Included(&runp), Bound::Excluded(&end), usize::MAX).unwrap() {
            let page = check_page::<F>(&k[runp.len()..], &v).unwrap();
            for item in 0..page.items.len() {
                lists.entry(page.list(item).to_vec()).or_default().extend(&page.postings[page.items[item].2.clone()]);
            }
        }
    }
    (manifest, lists)
}

/// The units of the live numbers of an index, by list: what a search reads.
#[cfg(test)]
pub(crate) fn dump_units<S: kerno_core::contract::Snapshot, F: Family>(
    snap: &S,
    prefix: &[u8],
) -> BTreeMap<Vec<u8>, Vec<(UnitId, u32, u32)>> {
    let mut slots = Slots::default();
    let mut out = BTreeMap::new();
    for (list, postings) in dump::<S, F>(snap, prefix).1 {
        let mut units: Vec<(UnitId, u32, u32)> = postings
            .into_iter()
            .filter_map(|(num, tf, dl)| {
                let (live, unit) = slots.slot(&|k| snap.get(k), prefix, num).unwrap();
                live.then_some((unit, tf, dl))
            })
            .collect();
        units.sort_unstable();
        if !units.is_empty() {
            out.insert(list, units);
        }
    }
    out
}

/// The raw writes that replace the runs of an index by one run that holds its postings as
/// `edit` leaves them, with a manifest that agrees with the pages: for tests that forge a
/// posting under the writer's back.
#[cfg(test)]
pub(crate) fn forge<S: kerno_core::contract::Snapshot, F: Family>(
    snap: &S,
    prefix: &[u8],
    edit: impl FnOnce(&mut BTreeMap<Vec<u8>, Vec<Posting>>),
) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    let (mut manifest, mut lists) = dump::<S, F>(snap, prefix);
    let (from, to) = (kind_key(prefix, KIND_RUN), kind_key(prefix, KIND_RUN + 1));
    let mut writes: BTreeMap<Vec<u8>, Option<Vec<u8>>> = snap
        .scan(Bound::Included(&from), Bound::Excluded(&to), usize::MAX)
        .unwrap()
        .into_iter()
        .map(|(k, _)| (k, None))
        .collect();
    edit(&mut lists);
    let id = manifest.runs.last().map_or(1, |r| r.id);
    let mut builder = RunBuilder::new::<F>(0);
    let mut slots = Slots::default();
    let mut dead = std::collections::BTreeSet::new();
    for (list, postings) in &lists {
        builder.begin(list);
        for p in postings {
            if !slots.slot(&|k| snap.get(k), prefix, p.0).is_ok_and(|s| s.0) {
                dead.insert(p.0);
            }
            builder.push(*p);
        }
    }
    manifest.runs.clear();
    manifest.live = (0..manifest.next).filter(|n| slots.slot(&|k| snap.get(k), prefix, *n).is_ok_and(|s| s.0)).count() as u64;
    if let Some(b) = builder.finish() {
        let runp = run_prefix(prefix, id);
        manifest.runs.push(Run {
            id,
            level: 1,
            min: 0,
            max: manifest.next.saturating_sub(1),
            pages: b.pages.len() as u64,
            postings: b.postings,
            bytes: b.bytes,
            dead: dead.len() as u64,
            last: b.pages.last().map(|(k, _)| k.clone()).unwrap_or_default(),
        });
        for (key, value) in b.pages {
            writes.insert([&runp[..], &key[..]].concat(), Some(value));
        }
    }
    writes.insert(manifest_key(prefix), Some(manifest.encode()));
    writes.into_iter().collect()
}
