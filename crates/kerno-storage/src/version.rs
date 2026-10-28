//! An immutable view of the LSM shape: the active memtable, the immutable
//! memtables waiting for flush and the SST levels. A snapshot is an
//! `Arc<Version>` plus a sequence number; everything it references stays
//! alive (and on disk) for as long as the snapshot does.

use std::ops::Bound;
use std::sync::Arc;

use kerno_core::Result;

use crate::format::bloom;
use crate::iter::{InternalIterator, LevelIter, MergeIter};
use crate::manifest::FileMeta;
use crate::memtable::{InternalKey, Memtable};
use crate::sst::Table;

pub struct Version {
    pub mem: Arc<Memtable>,
    /// Newest first.
    pub imms: Vec<Arc<Memtable>>,
    /// `levels[0]` newest first (files may overlap); `levels[1..]` sorted by
    /// key, non-overlapping.
    pub levels: Vec<Vec<Arc<Table>>>,
}

pub type Levels = Vec<Vec<Arc<Table>>>;

/// The files of `levels` as the MANIFEST records them.
pub fn level_metas(levels: &Levels) -> Vec<Vec<FileMeta>> {
    levels.iter().enumerate().map(|(l, files)| files.iter().map(|t| t.meta(l as u8)).collect()).collect()
}

impl Version {
    /// Newest value of `key` visible at `seq`.
    pub fn get(&self, key: &[u8], seq: u64) -> Result<Option<Vec<u8>>> {
        let probe = InternalKey { user_key: key.to_vec(), seq };
        for mem in std::iter::once(&self.mem).chain(&self.imms) {
            if let Some(v) = mem.get(&probe) {
                return Ok(v);
            }
        }
        let hash = bloom::hash(key);
        for t in &self.levels[0] {
            if let Some((_, v)) = t.get(key, hash)? {
                return Ok(v);
            }
        }
        for level in &self.levels[1..] {
            let i = level.partition_point(|t| t.largest.as_slice() < key);
            if let Some(t) = level.get(i)
                && let Some((_, v)) = t.get(key, hash)?
            {
                return Ok(v);
            }
        }
        Ok(None)
    }

    /// Up to `limit` live entries in `(start, end)` visible at `seq`.
    pub fn scan(
        &self,
        seq: u64,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        if limit == 0 || range_is_empty(start, end) {
            return Ok(Vec::new());
        }
        let mut children: Vec<Box<dyn InternalIterator + '_>> = Vec::new();
        for mem in std::iter::once(&self.mem).chain(&self.imms) {
            children.push(Box::new(mem.iter(seq)));
        }
        for t in &self.levels[0] {
            if table_in_range(t, start, end) {
                children.push(Box::new(t.iter(true)));
            }
        }
        for level in &self.levels[1..] {
            if !level.is_empty() {
                children.push(Box::new(LevelIter::new(level, true)));
            }
        }
        let mut it = MergeIter::new(children);
        match start {
            Bound::Unbounded => it.seek_to_first()?,
            Bound::Included(k) | Bound::Excluded(k) => it.seek(k)?,
        }
        let mut out = Vec::new();
        while it.valid() && out.len() < limit {
            let k = it.key();
            let past_end = match end {
                Bound::Included(e) => k > e,
                Bound::Excluded(e) => k >= e,
                Bound::Unbounded => false,
            };
            if past_end {
                break;
            }
            let skip = matches!(start, Bound::Excluded(s) if k == s);
            if !skip && let Some(v) = it.value() {
                out.push((k.to_vec(), v.to_vec()));
            }
            it.next()?;
        }
        Ok(out)
    }
}

fn range_is_empty(start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    match (start, end) {
        (Bound::Included(a), Bound::Included(b)) => a > b,
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => a >= b,
        _ => false,
    }
}

fn table_in_range(t: &Table, start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    let after_start = match start {
        Bound::Included(s) | Bound::Excluded(s) => t.largest.as_slice() >= s,
        Bound::Unbounded => true,
    };
    let before_end = match end {
        Bound::Included(e) | Bound::Excluded(e) => t.smallest.as_slice() <= e,
        Bound::Unbounded => true,
    };
    after_start && before_end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ranges() {
        use Bound::*;
        let (a, b): (&[u8], &[u8]) = (b"a", b"b");
        assert!(!range_is_empty(Included(a), Included(a)));
        assert!(range_is_empty(Included(a), Excluded(a)));
        assert!(range_is_empty(Excluded(a), Included(a)));
        assert!(range_is_empty(Included(b), Included(a)));
        assert!(!range_is_empty(Excluded(a), Excluded(b)));
        assert!(!range_is_empty(Unbounded, Excluded(a)));
        assert!(!range_is_empty(Excluded(b), Unbounded));
    }
}
