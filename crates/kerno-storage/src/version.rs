//! An immutable view of the LSM shape: the active memtable, the immutable
//! memtables waiting for flush and the SST levels. A snapshot is an
//! `Arc<Version>` plus a sequence number; everything it references stays
//! alive (and on disk) for as long as the snapshot does.

use std::borrow::Cow;
use std::ops::Bound;
use std::sync::Arc;

use kerno_core::Result;

use crate::format::bloom;
use crate::format::rangedel::Fragment;
use crate::iter::{InternalIterator, LevelIter, MergeIter};
use crate::manifest::FileMeta;
use crate::memtable::{InternalKey, Memtable};
use crate::rangedel::CoverCursor;
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

/// What looking `key` up in `t` decides: `Some(answer)` ends the search,
/// `None` goes on to the next source. `max_tomb` is the highest range
/// tombstone sequence seen so far, raised by this table's fragments.
fn lookup(t: &Table, key: &[u8], hash: u64, max_tomb: &mut u64) -> Result<Option<Option<Vec<u8>>>> {
    // Without a range tombstone seen, which is nearly always, this is the
    // point lookup of a table alone.
    if *max_tomb > 0 && t.largest_seq() < *max_tomb && key >= t.smallest.as_slice() && !t.ends_before(key) {
        return Ok(Some(None));
    }
    if t.has_fragments()
        && let Some(u) = t.covering_seq(key)
    {
        *max_tomb = (*max_tomb).max(u);
    }
    Ok(t.get(key, hash)?.map(|(s, v)| if s >= *max_tomb { v } else { None }))
}

/// The files of `levels` as the MANIFEST records them.
pub fn level_metas(levels: &Levels) -> Vec<Vec<FileMeta>> {
    levels.iter().enumerate().map(|(l, files)| files.iter().map(|t| t.meta(l as u8)).collect()).collect()
}

impl Version {
    /// Newest value of `key` visible at `seq`.
    ///
    /// Sources are consulted from the newest to the oldest, keeping
    /// `max_tomb`, the highest sequence (`<= seq`) of the range tombstones
    /// seen so far that cover `key`. A source's fragments count before its
    /// points (and before its bloom filter, which says nothing about ranges):
    /// the first point found hides behind a tombstone newer than it
    /// (`s < max_tomb`) and is otherwise the answer (SEM-3, SEM-4). A table
    /// whose newest entry is older than `max_tomb` ends the search: nothing
    /// at or below it can be visible.
    pub fn get(&self, key: &[u8], seq: u64) -> Result<Option<Vec<u8>>> {
        let probe = InternalKey { user_key: key.to_vec(), seq };
        let mut max_tomb = 0;
        for mem in std::iter::once(&self.mem).chain(&self.imms) {
            max_tomb = max_tomb.max(mem.covering_seq(key, seq));
            if let Some((s, v)) = mem.get(&probe) {
                return Ok(if s >= max_tomb { v } else { None });
            }
        }
        let hash = bloom::hash(key);
        for t in &self.levels[0] {
            if let Some(found) = lookup(t, key, hash, &mut max_tomb)? {
                return Ok(found);
            }
        }
        for level in &self.levels[1..] {
            let i = level.partition_point(|t| t.ends_before(key));
            if let Some(found) = level.get(i).map(|t| lookup(t, key, hash, &mut max_tomb)).transpose()?.flatten() {
                return Ok(found);
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
        // The fragments of every source the scan may meet; none at all is
        // the common case and costs nothing per key.
        let mem_fragments: Vec<_> =
            std::iter::once(&self.mem).chain(&self.imms).filter_map(|mem| mem.fragments(seq)).collect();
        let mut fragments: Vec<Cow<'_, [Fragment]>> = mem_fragments.iter().map(|l| Cow::Borrowed(&l[..])).collect();
        for mem in std::iter::once(&self.mem).chain(&self.imms) {
            children.push(Box::new(mem.iter(seq)));
        }
        for t in &self.levels[0] {
            if table_in_range(t, start, end) {
                children.push(Box::new(t.iter(true)));
                if t.has_fragments() {
                    fragments.push(Cow::Borrowed(t.fragments()));
                }
            }
        }
        for level in &self.levels[1..] {
            if !level.is_empty() {
                children.push(Box::new(LevelIter::new(level, true)));
                fragments.extend(
                    tables_in_range(level, start, end)
                        .iter()
                        .filter(|t| t.has_fragments())
                        .map(|t| Cow::Borrowed(t.fragments())),
                );
            }
        }
        let mut cover = CoverCursor::new(fragments);
        let mut it = MergeIter::new(children);
        match start {
            Bound::Unbounded => {
                it.seek_to_first()?;
                cover.seek(&[]);
            }
            Bound::Included(k) | Bound::Excluded(k) => {
                it.seek(k)?;
                cover.seek(k);
            }
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
            // The newest version of the key is hidden by a tombstone newer
            // than it; the older ones then are too (SEM-4).
            if !skip
                && let Some(v) = it.value()
                && cover.covering(k) <= it.seq()
            {
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

/// The tables of a sorted level whose extent may intersect the bounds.
fn tables_in_range<'a>(level: &'a [Arc<Table>], start: Bound<&[u8]>, end: Bound<&[u8]>) -> &'a [Arc<Table>] {
    let from = match start {
        Bound::Included(s) | Bound::Excluded(s) => level.partition_point(|t| t.ends_before(s)),
        Bound::Unbounded => 0,
    };
    let to = match end {
        Bound::Included(e) => level.partition_point(|t| t.smallest.as_slice() <= e),
        Bound::Excluded(e) => level.partition_point(|t| t.smallest.as_slice() < e),
        Bound::Unbounded => level.len(),
    };
    &level[from..to.max(from)]
}

/// Whether the extent of `t` may intersect the bounds (it may, where a bound
/// is excluded and the table touches it).
pub fn table_in_range(t: &Table, start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    let after_start = match start {
        Bound::Included(s) | Bound::Excluded(s) => !t.ends_before(s),
        Bound::Unbounded => true,
    };
    let before_end = match end {
        Bound::Included(e) => t.smallest.as_slice() <= e,
        Bound::Excluded(e) => t.smallest.as_slice() < e,
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
