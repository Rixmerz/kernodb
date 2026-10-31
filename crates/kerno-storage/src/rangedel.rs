//! Range tombstones outside the file format: how reads consult the
//! fragments of many sources and how a flush or compaction distributes the
//! fragments it keeps among its output files (phase1-design.md §5.5,
//! v1.md §7.4, SEM-4).
//!
//! A fragment `[start, end)@seq` hides the point `p@s` of a key it covers
//! exactly when `s < seq` (and, in a snapshot `S`, `seq <= S`; the fragments
//! of a table are always visible, SEM-5). Fragment lists here are sorted and
//! disjoint, as SST-F1 requires.

use std::borrow::Cow;

use crate::format::rangedel::{self, Fragment};

/// The fragments of every source a read consults, each list sorted and
/// disjoint, with one forward-only cursor per list. A scan asks for the
/// highest covering sequence of the keys it meets in increasing order, so
/// every list is walked at most once: the cost per key is the number of
/// lists, not the number of fragments.
pub struct CoverCursor<'a> {
    lists: Vec<Cow<'a, [Fragment]>>,
    /// Per list, the first fragment that does not end at or before the last
    /// key asked for.
    pos: Vec<usize>,
}

impl<'a> CoverCursor<'a> {
    pub fn new(lists: Vec<Cow<'a, [Fragment]>>) -> Self {
        let pos = vec![0; lists.len()];
        CoverCursor { lists, pos }
    }

    /// Positions every cursor for keys from `key` on (they may move back).
    pub fn seek(&mut self, key: &[u8]) {
        for (list, pos) in self.lists.iter().zip(&mut self.pos) {
            *pos = list.partition_point(|f| f.end.as_slice() <= key);
        }
    }

    /// The highest sequence among the fragments covering `key` (0 if none).
    /// `key` must not be smaller than the last one asked for since the last
    /// [`seek`](Self::seek).
    pub fn covering(&mut self, key: &[u8]) -> u64 {
        let mut best = 0;
        for (list, pos) in self.lists.iter().zip(&mut self.pos) {
            while list.get(*pos).is_some_and(|f| f.end.as_slice() <= key) {
                *pos += 1;
            }
            if let Some(f) = list.get(*pos)
                && f.start.as_slice() <= key
            {
                best = best.max(f.seq);
            }
        }
        best
    }
}

/// Merges fragment lists into one sorted, disjoint list in which each part
/// has the highest sequence that covers it (SST-F2).
pub fn aggregate<'a>(lists: impl IntoIterator<Item = &'a [Fragment]>) -> Vec<Fragment> {
    let mut lists = lists.into_iter().filter(|l| !l.is_empty());
    let (Some(first), second) = (lists.next(), lists.next()) else { return Vec::new() };
    let Some(second) = second else { return first.to_vec() };
    let tombstones: Vec<(&[u8], &[u8], u64)> = [first, second]
        .into_iter()
        .chain(lists)
        .flatten()
        .map(|f| (f.start.as_slice(), f.end.as_slice(), f.seq))
        .collect();
    rangedel::fragment(&tombstones)
}

/// The parts of `fragments` inside `[lo, hi)`, cut at those bounds (`None`:
/// unbounded).
pub fn clip(fragments: Vec<Fragment>, lo: Option<&[u8]>, hi: Option<&[u8]>) -> Vec<Fragment> {
    if lo.is_none() && hi.is_none() {
        return fragments;
    }
    fragments
        .into_iter()
        .filter_map(|mut f| {
            if let Some(lo) = lo
                && f.start.as_slice() < lo
            {
                f.start = lo.to_vec();
            }
            if let Some(hi) = hi
                && f.end.as_slice() > hi
            {
                f.end = hi.to_vec();
            }
            (f.start < f.end).then_some(f)
        })
        .collect()
}

/// The fragments a flush or compaction keeps, handed out piece by piece to
/// the output files in key order. An output that ends before key `K` (the
/// first key of the next output) takes the parts of the fragments below `K`,
/// and the next one starts at `K` with the rest, so the extents of the
/// outputs of one level stay disjoint (MAN-S3) however wide a fragment is.
pub struct FragmentSplitter {
    fragments: Vec<Fragment>,
    next: usize,
}

impl FragmentSplitter {
    /// `fragments` sorted and disjoint.
    pub fn new(fragments: Vec<Fragment>) -> Self {
        FragmentSplitter { fragments, next: 0 }
    }

    /// The parts below `boundary` of the fragments not handed out yet
    /// (`None`: all that is left).
    pub fn take_below(&mut self, boundary: Option<&[u8]>) -> Vec<Fragment> {
        let mut out = Vec::new();
        while let Some(f) = self.fragments.get_mut(self.next) {
            match boundary {
                Some(b) if f.start.as_slice() >= b => break,
                Some(b) if f.end.as_slice() > b => {
                    out.push(Fragment {
                        start: std::mem::replace(&mut f.start, b.to_vec()),
                        end: b.to_vec(),
                        seq: f.seq,
                    });
                    break;
                }
                _ => {
                    out.push(std::mem::replace(f, Fragment { start: Vec::new(), end: Vec::new(), seq: 0 }));
                    self.next += 1;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(start: &str, end: &str, seq: u64) -> Fragment {
        Fragment { start: start.into(), end: end.into(), seq }
    }

    fn cover(c: &mut CoverCursor<'_>, key: &str) -> u64 {
        c.covering(key.as_bytes())
    }

    #[test]
    fn cursor_takes_the_highest_sequence_over_all_lists() {
        let a = [frag("b", "d", 5), frag("f", "h", 2)];
        let b = [frag("c", "g", 9)];
        let mut c = CoverCursor::new(vec![Cow::Borrowed(&a[..]), Cow::Borrowed(&b[..]), Cow::Owned(Vec::new())]);
        c.seek(b"");
        let got: Vec<u64> = ["a", "b", "c", "d", "f", "g", "h", "z"].iter().map(|k| cover(&mut c, k)).collect();
        assert_eq!(got, [0, 5, 9, 9, 9, 2, 0, 0]);
        // A seek moves the cursors back.
        c.seek(b"c");
        assert_eq!(cover(&mut c, "c"), 9);
        assert_eq!(CoverCursor::new(Vec::new()).covering(b"a"), 0, "no list covers nothing");
    }

    #[test]
    fn aggregate_merges_and_keeps_single_lists_as_they_are() {
        let a = vec![frag("a", "m", 3)];
        let b = [frag("f", "z", 7)];
        assert_eq!(aggregate([&a[..]]), a);
        assert_eq!(aggregate([&a[..], &[][..]]), a);
        assert_eq!(aggregate([&a[..], &b[..]]), vec![frag("a", "f", 3), frag("f", "z", 7)]);
        assert!(aggregate(std::iter::empty()).is_empty());
    }

    #[test]
    fn clip_cuts_at_both_bounds() {
        let list = || vec![frag("a", "c", 1), frag("e", "k", 2), frag("m", "p", 3)];
        assert_eq!(clip(list(), None, None), list());
        assert_eq!(clip(list(), Some(b"b"), Some(b"n")), vec![frag("b", "c", 1), frag("e", "k", 2), frag("m", "n", 3)]);
        assert_eq!(clip(list(), Some(b"c"), Some(b"e")), vec![], "nothing lies in [c, e)");
        assert_eq!(clip(list(), None, Some(b"f")), vec![frag("a", "c", 1), frag("e", "f", 2)]);
        assert_eq!(clip(list(), Some(b"k"), None), vec![frag("m", "p", 3)]);
    }

    #[test]
    fn splitter_hands_out_every_part_once() {
        let mut s = FragmentSplitter::new(vec![frag("a", "c", 1), frag("e", "k", 2), frag("m", "p", 3)]);
        assert_eq!(s.take_below(Some(b"b")), vec![frag("a", "b", 1)]);
        assert_eq!(s.take_below(Some(b"c")), vec![frag("b", "c", 1)]);
        assert_eq!(s.take_below(Some(b"e")), vec![], "nothing below the next start");
        assert_eq!(s.take_below(Some(b"h")), vec![frag("e", "h", 2)]);
        assert_eq!(s.take_below(None), vec![frag("h", "k", 2), frag("m", "p", 3)]);
        assert_eq!(s.take_below(None), vec![], "everything was handed out once");
    }
}
