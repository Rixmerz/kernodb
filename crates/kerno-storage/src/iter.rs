//! Internal iterators and the k-way merge used by scans, flushes and
//! compactions.

use std::cmp::Ordering;
use std::sync::Arc;

use kerno_core::Result;

use crate::sst::{Table, TableIter};

/// Cursor over entries ordered by (user key ascending, sequence descending).
/// `key`, `seq` and `value` may only be called while `valid()`.
pub trait InternalIterator {
    fn valid(&self) -> bool;
    fn seek_to_first(&mut self) -> Result<()>;
    /// Positions at the first entry whose user key is `>= target`.
    fn seek(&mut self, target: &[u8]) -> Result<()>;
    fn next(&mut self) -> Result<()>;
    fn key(&self) -> &[u8];
    fn seq(&self) -> u64;
    /// `None` for a tombstone.
    fn value(&self) -> Option<&[u8]>;
}

/// Merges of more than this many children use a binary heap (see
/// [`MergeIter::with_heap_threshold`]).
pub const DEFAULT_HEAP_THRESHOLD: usize = 8;

/// Merges several sources and yields exactly one entry per user key: the
/// newest one (highest sequence; ties go to the earlier child, so children
/// must be ordered newest source first). Tombstones are yielded too; the
/// caller decides whether to skip or keep them.
///
/// With few children the next entry is found by scanning all of them; with
/// more than the heap threshold, by a binary heap of the valid children.
/// Both give the same results.
pub struct MergeIter<'a> {
    children: Vec<Box<dyn InternalIterator + 'a>>,
    current: Option<usize>,
    scratch: Vec<u8>,
    /// Indexes of the valid children as a binary min-heap by
    /// [`MergeIter::before`]; `Some` in heap mode only.
    heap: Option<Vec<usize>>,
}

impl<'a> MergeIter<'a> {
    pub fn new(children: Vec<Box<dyn InternalIterator + 'a>>) -> Self {
        Self::with_heap_threshold(children, DEFAULT_HEAP_THRESHOLD)
    }

    /// A merge that uses a heap when it has more than `threshold` children.
    pub fn with_heap_threshold(children: Vec<Box<dyn InternalIterator + 'a>>, threshold: usize) -> Self {
        let heap = (children.len() > threshold).then(Vec::new);
        MergeIter { children, current: None, scratch: Vec::new(), heap }
    }

    /// Whether child `a` comes before child `b`: smaller key, then higher
    /// sequence, then earlier child. Both must be valid.
    fn before(&self, a: usize, b: usize) -> bool {
        let (ca, cb) = (&self.children[a], &self.children[b]);
        ca.key().cmp(cb.key()).then_with(|| cb.seq().cmp(&ca.seq())).then_with(|| a.cmp(&b)) == Ordering::Less
    }

    fn pick(&mut self) {
        if let Some(heap) = &self.heap {
            self.current = heap.first().copied();
            return;
        }
        let mut best: Option<usize> = None;
        for (i, c) in self.children.iter().enumerate() {
            if !c.valid() {
                continue;
            }
            best = match best {
                None => Some(i),
                Some(b) => {
                    let bc = &self.children[b];
                    match c.key().cmp(bc.key()).then_with(|| bc.seq().cmp(&c.seq())) {
                        Ordering::Less => Some(i),
                        _ => Some(b),
                    }
                }
            };
        }
        self.current = best;
    }

    /// Rebuilds the heap from the valid children (after a seek).
    fn build_heap(&mut self) {
        let Some(mut heap) = self.heap.take() else { return };
        heap.clear();
        heap.extend((0..self.children.len()).filter(|&i| self.children[i].valid()));
        self.heap = Some(heap);
        for i in (0..self.heap.as_ref().map_or(0, Vec::len) / 2).rev() {
            self.sift_down(i);
        }
    }

    /// Restores the heap property below position `i`.
    fn sift_down(&mut self, mut i: usize) {
        let Some(mut heap) = self.heap.take() else { return };
        loop {
            let (l, r) = (2 * i + 1, 2 * i + 2);
            let mut first = i;
            if l < heap.len() && self.before(heap[l], heap[first]) {
                first = l;
            }
            if r < heap.len() && self.before(heap[r], heap[first]) {
                first = r;
            }
            if first == i {
                break;
            }
            heap.swap(i, first);
            i = first;
        }
        self.heap = Some(heap);
    }

    fn cur(&self) -> &dyn InternalIterator {
        &*self.children[self.current.expect("iterator is not valid")]
    }

    /// Heap mode: advances every child positioned at `key`.
    fn skip_heap(&mut self, key: &[u8]) -> Result<()> {
        while let Some(&top) = self.heap.as_ref().and_then(|h| h.first()) {
            if self.children[top].key() != key {
                break;
            }
            self.children[top].next()?;
            if !self.children[top].valid()
                && let Some(heap) = self.heap.as_mut()
            {
                heap.swap_remove(0);
            }
            self.sift_down(0);
        }
        Ok(())
    }
}

impl InternalIterator for MergeIter<'_> {
    fn valid(&self) -> bool {
        self.current.is_some()
    }

    fn seek_to_first(&mut self) -> Result<()> {
        for c in &mut self.children {
            c.seek_to_first()?;
        }
        self.build_heap();
        self.pick();
        Ok(())
    }

    fn seek(&mut self, target: &[u8]) -> Result<()> {
        for c in &mut self.children {
            c.seek(target)?;
        }
        self.build_heap();
        self.pick();
        Ok(())
    }

    /// Advances past every remaining version of the current user key.
    fn next(&mut self) -> Result<()> {
        let cur = self.current.expect("iterator is not valid");
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        scratch.extend_from_slice(self.children[cur].key());
        if self.heap.is_some() {
            self.skip_heap(&scratch)?;
        } else {
            for c in &mut self.children {
                while c.valid() && c.key() == scratch.as_slice() {
                    c.next()?;
                }
            }
        }
        self.scratch = scratch;
        self.pick();
        Ok(())
    }

    fn key(&self) -> &[u8] {
        self.cur().key()
    }

    fn seq(&self) -> u64 {
        self.cur().seq()
    }

    fn value(&self) -> Option<&[u8]> {
        self.cur().value()
    }
}

/// Concatenates the tables of one sorted, non-overlapping level (L1+),
/// opening one table iterator at a time.
pub struct LevelIter<'a> {
    tables: &'a [Arc<Table>],
    fill_cache: bool,
    idx: usize,
    cur: Option<TableIter<'a>>,
}

impl<'a> LevelIter<'a> {
    pub fn new(tables: &'a [Arc<Table>], fill_cache: bool) -> Self {
        LevelIter { tables, fill_cache, idx: 0, cur: None }
    }

    /// Moves to the next tables while the current one is exhausted.
    fn skip_exhausted(&mut self) -> Result<()> {
        while self.cur.as_ref().is_some_and(|c| !c.valid()) {
            self.idx += 1;
            if self.idx >= self.tables.len() {
                self.cur = None;
                break;
            }
            let mut it = self.tables[self.idx].iter(self.fill_cache);
            it.seek_to_first()?;
            self.cur = Some(it);
        }
        Ok(())
    }

    fn it(&self) -> &TableIter<'a> {
        self.cur.as_ref().expect("iterator is not valid")
    }
}

impl InternalIterator for LevelIter<'_> {
    fn valid(&self) -> bool {
        self.cur.as_ref().is_some_and(|c| c.valid())
    }

    fn seek_to_first(&mut self) -> Result<()> {
        self.idx = 0;
        self.cur = None;
        if let Some(t) = self.tables.first() {
            let mut it = t.iter(self.fill_cache);
            it.seek_to_first()?;
            self.cur = Some(it);
        }
        self.skip_exhausted()
    }

    fn seek(&mut self, target: &[u8]) -> Result<()> {
        self.idx = self.tables.partition_point(|t| t.ends_before(target));
        self.cur = None;
        if let Some(t) = self.tables.get(self.idx) {
            let mut it = t.iter(self.fill_cache);
            it.seek(target)?;
            self.cur = Some(it);
        }
        self.skip_exhausted()
    }

    fn next(&mut self) -> Result<()> {
        self.cur.as_mut().expect("iterator is not valid").next()?;
        self.skip_exhausted()
    }

    fn key(&self) -> &[u8] {
        self.it().key()
    }

    fn seq(&self) -> u64 {
        self.it().seq()
    }

    fn value(&self) -> Option<&[u8]> {
        self.it().value()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// In-memory iterator over (key, seq, value) triples, for tests.
    pub struct VecIter {
        entries: Vec<(Vec<u8>, u64, Option<Vec<u8>>)>,
        pos: usize,
    }

    impl VecIter {
        pub fn new(mut entries: Vec<(&str, u64, Option<&str>)>) -> Self {
            entries.sort_by(|a, b| a.0.cmp(b.0).then(b.1.cmp(&a.1)));
            let entries =
                entries.into_iter().map(|(k, s, v)| (k.as_bytes().to_vec(), s, v.map(|v| v.as_bytes().to_vec())));
            VecIter { entries: entries.collect(), pos: usize::MAX }
        }
    }

    impl InternalIterator for VecIter {
        fn valid(&self) -> bool {
            self.pos < self.entries.len()
        }
        fn seek_to_first(&mut self) -> Result<()> {
            self.pos = 0;
            Ok(())
        }
        fn seek(&mut self, target: &[u8]) -> Result<()> {
            self.pos = self.entries.partition_point(|e| e.0.as_slice() < target);
            Ok(())
        }
        fn next(&mut self) -> Result<()> {
            self.pos += 1;
            Ok(())
        }
        fn key(&self) -> &[u8] {
            &self.entries[self.pos].0
        }
        fn seq(&self) -> u64 {
            self.entries[self.pos].1
        }
        fn value(&self) -> Option<&[u8]> {
            self.entries[self.pos].2.as_deref()
        }
    }

    fn collect(it: &mut dyn InternalIterator) -> Vec<(String, u64, Option<String>)> {
        let mut out = Vec::new();
        while it.valid() {
            let v = it.value().map(|v| String::from_utf8(v.to_vec()).unwrap());
            out.push((String::from_utf8(it.key().to_vec()).unwrap(), it.seq(), v));
            it.next().unwrap();
        }
        out
    }

    fn s(k: &str, seq: u64, v: Option<&str>) -> (String, u64, Option<String>) {
        (k.to_string(), seq, v.map(str::to_string))
    }

    #[test]
    fn merge_keeps_newest_version_and_tombstones() {
        // Newest source first, as in a Version: memtable, L0, L1.
        let mem = VecIter::new(vec![("b", 9, None), ("b", 8, Some("b8")), ("d", 7, Some("d7"))]);
        let l0 = VecIter::new(vec![("a", 5, Some("a5")), ("b", 4, Some("b4")), ("c", 6, None)]);
        let l1 = VecIter::new(vec![("a", 1, Some("a1")), ("c", 2, Some("c2")), ("e", 3, Some("e3"))]);
        let mut m = MergeIter::new(vec![Box::new(mem), Box::new(l0), Box::new(l1)]);
        m.seek_to_first().unwrap();
        assert_eq!(
            collect(&mut m),
            vec![s("a", 5, Some("a5")), s("b", 9, None), s("c", 6, None), s("d", 7, Some("d7")), s("e", 3, Some("e3"))]
        );
        m.seek(b"bb").unwrap();
        assert_eq!(collect(&mut m), vec![s("c", 6, None), s("d", 7, Some("d7")), s("e", 3, Some("e3"))]);
        m.seek(b"e").unwrap();
        assert_eq!(collect(&mut m), vec![s("e", 3, Some("e3"))]);
        m.seek(b"f").unwrap();
        assert!(!m.valid());
    }

    /// Deterministic pseudo-random sources with repeated keys and sequences.
    fn sources(seed: u64, n: usize) -> Vec<Vec<(String, u64, Option<String>)>> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut next = move |m: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % m
        };
        (0..n)
            .map(|c| {
                (0..next(40))
                    .map(|_| {
                        let v = (next(4) > 0).then(|| format!("v{c}"));
                        (format!("k{:02}", next(30)), next(6) + 1, v)
                    })
                    .collect()
            })
            .collect()
    }

    fn merged(sources: &[Vec<(String, u64, Option<String>)>], threshold: usize) -> Vec<(String, u64, Option<String>)> {
        let children = sources
            .iter()
            .map(|e| {
                let entries = e.iter().map(|(k, s, v)| (k.as_str(), *s, v.as_deref())).collect();
                Box::new(VecIter::new(entries)) as Box<dyn InternalIterator>
            })
            .collect();
        let mut m = MergeIter::with_heap_threshold(children, threshold);
        m.seek_to_first().unwrap();
        let mut all = collect(&mut m);
        // A seek into the middle must agree too.
        m.seek(b"k15").unwrap();
        all.extend(collect(&mut m));
        all
    }

    #[test]
    fn heap_and_linear_merges_agree() {
        for seed in 0..200 {
            let n = 1 + (seed as usize % 20);
            let src = sources(seed, n);
            let linear = merged(&src, usize::MAX);
            assert_eq!(merged(&src, 0), linear, "seed {seed}, {n} children, heap from the start");
            assert_eq!(merged(&src, 8), linear, "seed {seed}, {n} children");
        }
    }

    #[test]
    fn merge_with_no_or_empty_children() {
        let mut m = MergeIter::new(Vec::new());
        m.seek_to_first().unwrap();
        assert!(!m.valid());
        let mut m = MergeIter::new(vec![Box::new(VecIter::new(vec![])), Box::new(VecIter::new(vec![("x", 1, None)]))]);
        m.seek_to_first().unwrap();
        assert_eq!(collect(&mut m), vec![s("x", 1, None)]);
    }
}
