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

/// Merges several sources and yields exactly one entry per user key: the
/// newest one (highest sequence; ties go to the earlier child, so children
/// must be ordered newest source first). Tombstones are yielded too; the
/// caller decides whether to skip or keep them.
pub struct MergeIter<'a> {
    children: Vec<Box<dyn InternalIterator + 'a>>,
    current: Option<usize>,
    scratch: Vec<u8>,
}

impl<'a> MergeIter<'a> {
    pub fn new(children: Vec<Box<dyn InternalIterator + 'a>>) -> Self {
        MergeIter { children, current: None, scratch: Vec::new() }
    }

    fn pick(&mut self) {
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

    fn cur(&self) -> &dyn InternalIterator {
        &*self.children[self.current.expect("iterator is not valid")]
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
        self.pick();
        Ok(())
    }

    fn seek(&mut self, target: &[u8]) -> Result<()> {
        for c in &mut self.children {
            c.seek(target)?;
        }
        self.pick();
        Ok(())
    }

    /// Advances past every remaining version of the current user key.
    fn next(&mut self) -> Result<()> {
        let MergeIter { children, current, scratch } = self;
        let cur = current.expect("iterator is not valid");
        scratch.clear();
        scratch.extend_from_slice(children[cur].key());
        for c in children.iter_mut() {
            while c.valid() && c.key() == scratch.as_slice() {
                c.next()?;
            }
        }
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
        self.idx = self.tables.partition_point(|t| t.largest.as_slice() < target);
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
