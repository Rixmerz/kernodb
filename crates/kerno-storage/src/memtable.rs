//! In-memory write buffer: a lock-free skip list keyed by
//! (user key ascending, sequence descending), holding values and tombstones.
//!
//! The single writer inserts while any number of readers iterate; a reader
//! at snapshot `seq` ignores entries with a larger sequence number, so the
//! writer never has to copy or lock the table.

use std::cmp::Ordering;
use std::ops::Bound;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};

use crossbeam_skiplist::SkipMap;
use crossbeam_skiplist::map::Entry;
use kerno_core::Result;
use parking_lot::RwLock;

use crate::format::rangedel::{self, Fragment};
use crate::format::wal::CanonicalCommit;
use crate::iter::InternalIterator;
use crate::stats::Stats;

/// Rough per-entry bookkeeping cost (skip-list node, key struct, vectors).
const ENTRY_OVERHEAD: usize = 64;

/// A memtable that holds this many range tombstones is full whatever its
/// size: every read pays for each tombstone of every memtable it consults
/// (phase1-design.md §5.5).
pub const MAX_RANGE_TOMBSTONES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternalKey {
    pub user_key: Vec<u8>,
    pub seq: u64,
}

impl Ord for InternalKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.user_key.cmp(&other.user_key).then_with(|| other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for InternalKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// `None` is a tombstone.
type Value = Option<Vec<u8>>;

/// A range tombstone `[start, end)` committed at `seq`.
type RangeTombstone = (Vec<u8>, Vec<u8>, u64);

pub struct Memtable {
    map: SkipMap<InternalKey, Value>,
    /// Range tombstones in commit order; append-only. A reader at snapshot
    /// `seq` ignores those committed after it, like it does the points.
    ranges: RwLock<Vec<RangeTombstone>>,
    /// `ranges.len()`, readable without the lock (the common case is none).
    range_count: AtomicUsize,
    bytes: AtomicUsize,
    max_seq: AtomicU64,
    /// The WAL file that holds this memtable's commits.
    wal_number: u64,
    /// Bytes written to that file when this memtable stopped being the
    /// active one (0: not known, or still active).
    wal_len: AtomicU64,
    /// Set when the memtable left the current version (it was written out):
    /// from then on only snapshots keep it alive.
    retired: OnceLock<Retired>,
}

/// A memtable's bytes counted in `snapshot_retained_bytes`.
struct Retired {
    stats: Arc<Stats>,
    bytes: u64,
}

impl Drop for Memtable {
    fn drop(&mut self) {
        if let Some(r) = self.retired.get() {
            r.stats.retained_memtable_bytes.fetch_sub(r.bytes, Relaxed);
        }
    }
}

impl Memtable {
    pub fn new(wal_number: u64) -> Self {
        Memtable {
            map: SkipMap::new(),
            ranges: RwLock::new(Vec::new()),
            range_count: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            max_seq: AtomicU64::new(0),
            wal_number,
            wal_len: AtomicU64::new(0),
            retired: OnceLock::new(),
        }
    }

    /// Called when the memtable leaves the current version after its flush:
    /// its bytes count as retained by snapshots until the last reference
    /// drops. Nothing is applied to a memtable from then on, so its size is
    /// final.
    pub fn retire(&self, stats: &Arc<Stats>) {
        let bytes = self.approx_bytes() as u64;
        if self.retired.set(Retired { stats: stats.clone(), bytes }).is_ok() {
            stats.retained_memtable_bytes.fetch_add(bytes, Relaxed);
        }
    }

    /// Records how many bytes of its WAL file are written: what a recycled
    /// log has to zero (the rest of the file already is).
    pub fn set_wal_len(&self, len: u64) {
        self.wal_len.store(len, Relaxed);
    }

    pub fn wal_len(&self) -> u64 {
        self.wal_len.load(Relaxed)
    }

    pub fn wal_number(&self) -> u64 {
        self.wal_number
    }

    pub fn approx_bytes(&self) -> usize {
        self.bytes.load(Relaxed)
    }

    /// Largest sequence number applied (0 if empty).
    pub fn max_seq(&self) -> u64 {
        self.max_seq.load(Relaxed)
    }

    /// Whether nothing, points or range tombstones, was applied.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty() && self.range_count() == 0
    }

    /// Range tombstones applied so far.
    pub fn range_count(&self) -> usize {
        self.range_count.load(Relaxed)
    }

    /// Whether the memtable takes no more commits: it holds `write_buffer`
    /// bytes or [`MAX_RANGE_TOMBSTONES`] range tombstones.
    pub fn is_full(&self, write_buffer: usize) -> bool {
        self.approx_bytes() >= write_buffer || self.range_count() >= MAX_RANGE_TOMBSTONES
    }

    /// Applies a commit, in canonical form (WAL-C1), under sequence `seq`:
    /// the memtable holds exactly what the WAL replays, so a commit has the
    /// same effect before and after a restart. The ranges go in before the
    /// points, as the canonical form orders them (WAL-C2): a point of the
    /// same commit survives a range of it (SEM-3).
    pub fn apply_commit<B: AsRef<[u8]>>(&self, seq: u64, commit: &CanonicalCommit<B>) {
        if !commit.ranges.is_empty() {
            let mut ranges = self.ranges.write();
            for (start, end) in &commit.ranges {
                let (start, end) = (start.as_ref(), end.as_ref());
                self.bytes.fetch_add(start.len() + end.len() + ENTRY_OVERHEAD, Relaxed);
                ranges.push((start.to_vec(), end.to_vec(), seq));
            }
            self.range_count.store(ranges.len(), Relaxed);
        }
        for (key, value) in &commit.points {
            let (key, value) = (key.as_ref(), value.as_ref().map(|v| v.as_ref().to_vec()));
            let size = key.len() + value.as_ref().map_or(0, Vec::len) + ENTRY_OVERHEAD;
            self.bytes.fetch_add(size, Relaxed);
            self.map.insert(InternalKey { user_key: key.to_vec(), seq }, value);
        }
        self.max_seq.fetch_max(seq, Relaxed);
    }

    /// Newest version of `probe.user_key` with sequence `<= probe.seq`, with
    /// its sequence: `Some((s, Some(v)))` value, `Some((s, None))` tombstone,
    /// `None` not present. Range tombstones are not consulted: see
    /// [`Memtable::covering_seq`].
    pub fn get(&self, probe: &InternalKey) -> Option<(u64, Value)> {
        let e = self.map.lower_bound(Bound::Included(probe))?;
        (e.key().user_key == probe.user_key).then(|| (e.key().seq, e.value().clone()))
    }

    /// The highest sequence `<= seq` of the range tombstones that cover `key`
    /// (0 if none does).
    // ponytail: a linear pass over the tombstones, at most MAX_RANGE_TOMBSTONES
    // of them; a sorted structure if reads of range-heavy memtables get hot.
    pub fn covering_seq(&self, key: &[u8], seq: u64) -> u64 {
        if self.range_count() == 0 {
            return 0;
        }
        let ranges = self.ranges.read();
        ranges
            .iter()
            .filter(|(s, e, q)| *q <= seq && s.as_slice() <= key && key < e.as_slice())
            .map(|r| r.2)
            .max()
            .unwrap_or(0)
    }

    /// The range tombstones committed at or before `seq` as sorted, disjoint
    /// fragments, each with the highest sequence that covers it.
    // ponytail: recomputed on every call (a scan, a flush); cache the result
    // by (tombstone count, seq) if scans over range-heavy memtables get hot.
    pub fn fragments(&self, seq: u64) -> Vec<Fragment> {
        if self.range_count() == 0 {
            return Vec::new();
        }
        let visible: Vec<RangeTombstone> = self.ranges.read().iter().filter(|r| r.2 <= seq).cloned().collect();
        rangedel::fragment(&visible)
    }

    /// Iterator over every version visible at `seq` (several per key,
    /// newest first).
    pub fn iter(&self, seq: u64) -> MemIter<'_> {
        MemIter { map: &self.map, cur: None, seq }
    }
}

pub struct MemIter<'a> {
    map: &'a SkipMap<InternalKey, Value>,
    cur: Option<Entry<'a, InternalKey, Value>>,
    seq: u64,
}

impl<'a> MemIter<'a> {
    fn skip_invisible(&mut self) {
        while let Some(e) = &self.cur {
            if e.key().seq <= self.seq {
                break;
            }
            let next = e.next();
            self.cur = next;
        }
    }

    fn entry(&self) -> &Entry<'a, InternalKey, Value> {
        self.cur.as_ref().expect("iterator is not valid")
    }
}

impl InternalIterator for MemIter<'_> {
    fn valid(&self) -> bool {
        self.cur.is_some()
    }

    fn seek_to_first(&mut self) -> Result<()> {
        self.cur = self.map.front();
        self.skip_invisible();
        Ok(())
    }

    fn seek(&mut self, target: &[u8]) -> Result<()> {
        let probe = InternalKey { user_key: target.to_vec(), seq: u64::MAX };
        self.cur = self.map.lower_bound(Bound::Included(&probe));
        self.skip_invisible();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        let next = self.entry().next();
        self.cur = next;
        self.skip_invisible();
        Ok(())
    }

    fn key(&self) -> &[u8] {
        &self.entry().key().user_key
    }

    fn seq(&self) -> u64 {
        self.entry().key().seq
    }

    fn value(&self) -> Option<&[u8]> {
        self.entry().value().as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::wal::{CommitOp, canonicalize};

    /// A batch of `(key, value)` pairs, `None` being a delete, applied in
    /// its canonical form.
    fn apply(m: &Memtable, seq: u64, ops: &[(&str, Option<&str>)]) {
        let commit = canonicalize(ops.iter().map(|(k, v)| match v {
            Some(v) => CommitOp::Put { key: k.as_bytes(), value: v.as_bytes() },
            None => CommitOp::Delete { key: k.as_bytes() },
        }));
        m.apply_commit(seq, &commit);
    }

    fn probe(k: &str, seq: u64) -> InternalKey {
        InternalKey { user_key: k.into(), seq }
    }

    #[test]
    fn versions_are_filtered_by_sequence() {
        let m = Memtable::new(1);
        apply(&m, 1, &[("a", Some("1")), ("b", Some("1"))]);
        apply(&m, 2, &[("a", Some("2")), ("b", None)]);
        apply(&m, 3, &[("a", Some("x")), ("a", None), ("a", Some("3"))]);
        assert_eq!(m.get(&probe("a", 0)), None);
        assert_eq!(m.get(&probe("a", 1)), Some((1, Some(b"1".to_vec()))));
        assert_eq!(m.get(&probe("a", 2)), Some((2, Some(b"2".to_vec()))));
        assert_eq!(m.get(&probe("a", 3)), Some((3, Some(b"3".to_vec()))));
        assert_eq!(m.get(&probe("b", 1)), Some((1, Some(b"1".to_vec()))));
        assert_eq!(m.get(&probe("b", 9)), Some((2, None)));
        assert_eq!(m.get(&probe("c", 9)), None);
        assert_eq!(m.max_seq(), 3);

        let mut it = m.iter(2);
        it.seek_to_first().unwrap();
        let mut seen = Vec::new();
        while it.valid() {
            seen.push((it.key().to_vec(), it.seq(), it.value().map(<[u8]>::to_vec)));
            it.next().unwrap();
        }
        assert_eq!(
            seen,
            vec![
                (b"a".to_vec(), 2, Some(b"2".to_vec())),
                (b"a".to_vec(), 1, Some(b"1".to_vec())),
                (b"b".to_vec(), 2, None),
                (b"b".to_vec(), 1, Some(b"1".to_vec())),
            ]
        );
        let mut it = m.iter(3);
        it.seek(b"a\0").unwrap();
        assert_eq!((it.key(), it.seq()), (&b"b"[..], 2));
    }

    fn apply_ops(m: &Memtable, seq: u64, ops: &[CommitOp<'_>]) {
        m.apply_commit(
            seq,
            &canonicalize(ops.iter().map(|op| match op {
                CommitOp::Put { key, value } => CommitOp::Put { key, value },
                CommitOp::Delete { key } => CommitOp::Delete { key },
                CommitOp::DeleteRange { start, end } => CommitOp::DeleteRange { start, end },
            })),
        );
    }

    #[test]
    fn range_tombstones_are_filtered_by_sequence() {
        let m = Memtable::new(1);
        assert!(m.is_empty());
        apply_ops(&m, 1, &[CommitOp::Put { key: b"b", value: b"1" }]);
        apply_ops(&m, 2, &[CommitOp::DeleteRange { start: b"a", end: b"c" }]);
        apply_ops(&m, 3, &[CommitOp::DeleteRange { start: b"b", end: b"d" }]);
        // The same commit: the point survives the range.
        apply_ops(&m, 4, &[CommitOp::DeleteRange { start: b"x", end: b"z" }, CommitOp::Put { key: b"y", value: b"4" }]);
        assert_eq!(m.range_count(), 3);
        assert_eq!(m.covering_seq(b"b", 1), 0);
        assert_eq!(m.covering_seq(b"b", 2), 2);
        assert_eq!(m.covering_seq(b"b", 9), 3);
        assert_eq!(m.covering_seq(b"a", 9), 2);
        assert_eq!(m.covering_seq(b"c", 9), 3);
        assert_eq!(m.covering_seq(b"d", 9), 0, "the end is exclusive");
        assert_eq!(m.get(&probe("y", 4)), Some((4, Some(b"4".to_vec()))));
        assert_eq!(m.covering_seq(b"y", 4), 4, "the range of the commit is there; the point wins at equal sequences");
        let frags = |seq| m.fragments(seq).iter().map(|f| (f.start.clone(), f.end.clone(), f.seq)).collect::<Vec<_>>();
        assert!(frags(1).is_empty());
        assert_eq!(frags(2), vec![(b"a".to_vec(), b"c".to_vec(), 2)]);
        assert_eq!(
            frags(3),
            vec![(b"a".to_vec(), b"b".to_vec(), 2), (b"b".to_vec(), b"d".to_vec(), 3)],
            "the highest sequence covers each part"
        );
    }

    #[test]
    fn range_only_memtable_is_not_empty_and_fills_up() {
        let m = Memtable::new(1);
        assert!(!m.is_full(1 << 20));
        for i in 0..MAX_RANGE_TOMBSTONES as u64 {
            assert!(!m.is_full(1 << 30), "full after {i} tombstones");
            apply_ops(&m, i + 1, &[CommitOp::DeleteRange { start: b"a", end: b"b" }]);
            assert!(!m.is_empty());
        }
        assert!(m.is_full(1 << 30), "{MAX_RANGE_TOMBSTONES} tombstones fill the memtable");
        assert!(m.approx_bytes() > 0, "tombstones are counted in the size");
    }
}
