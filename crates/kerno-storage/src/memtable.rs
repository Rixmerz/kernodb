//! In-memory write buffer: a lock-free skip list keyed by
//! (user key ascending, sequence descending), holding values and tombstones.
//!
//! The single writer inserts while any number of readers iterate; a reader
//! at snapshot `seq` ignores entries with a larger sequence number, so the
//! writer never has to copy or lock the table.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, OnceLock};

use crossbeam_skiplist::SkipMap;
use crossbeam_skiplist::map::Entry;
use kerno_core::Result;
use parking_lot::{Mutex, RwLock};

use crate::format::rangedel::Fragment;
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

/// Bits of a memtable's key filter (128 KiB): over eight per key for the
/// most keys the default write buffer holds, and still correct, only less
/// selective, for a larger buffer.
const FILTER_BITS: u64 = 1 << 20;

/// The two bits of a key in the filter, from its bloom hash (the halves of
/// the hash are independent enough for two probes).
fn filter_bits(hash: u64) -> [u64; 2] {
    [hash % FILTER_BITS, (hash >> 32) % FILTER_BITS]
}

/// One piece of the range index: `[start, end)` (start is the map key) and
/// the sequences of every tombstone covering all of it, ascending.
#[derive(Clone, Debug)]
struct Piece {
    end: Vec<u8>,
    seqs: Vec<u64>,
}

/// The memtable's range tombstones, kept fragmented as they arrive: sorted,
/// disjoint pieces, each with the sequences that cover it. A tombstone costs
/// `O(log n + k)` to add (`k` the pieces it overlaps), a point lookup
/// `O(log n)`, and a reader at any snapshot sees exactly the tombstones
/// committed at or before it.
#[derive(Default)]
struct RangeIndex {
    pieces: BTreeMap<Vec<u8>, Piece>,
    count: usize,
    max_seq: u64,
}

impl RangeIndex {
    /// Splits the piece that strictly contains `at`, if any, at `at`.
    fn split_at(&mut self, at: &[u8]) {
        let Some((start, piece)) =
            self.pieces.range_mut::<[u8], _>((Bound::Unbounded, Bound::Excluded(at))).next_back()
        else {
            return;
        };
        if start.as_slice() < at && at < piece.end.as_slice() {
            let tail = Piece { end: std::mem::replace(&mut piece.end, at.to_vec()), seqs: piece.seqs.clone() };
            self.pieces.insert(at.to_vec(), tail);
        }
    }

    fn insert(&mut self, start: &[u8], end: &[u8], seq: u64) {
        self.count += 1;
        self.max_seq = self.max_seq.max(seq);
        if start >= end {
            return;
        }
        self.split_at(start);
        self.split_at(end);
        let mut gaps: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut cursor = start.to_vec();
        for (s, piece) in self.pieces.range_mut::<[u8], _>((Bound::Included(start), Bound::Excluded(end))) {
            if *s > cursor {
                gaps.push((std::mem::take(&mut cursor), s.clone()));
            }
            let at = piece.seqs.partition_point(|&q| q < seq);
            piece.seqs.insert(at, seq);
            cursor.clone_from(&piece.end);
        }
        if cursor.as_slice() < end {
            gaps.push((cursor, end.to_vec()));
        }
        for (s, e) in gaps {
            self.pieces.insert(s, Piece { end: e, seqs: vec![seq] });
        }
    }

    fn covering_seq(&self, key: &[u8], seq: u64) -> u64 {
        let Some((_, piece)) = self.pieces.range::<[u8], _>((Bound::Unbounded, Bound::Included(key))).next_back()
        else {
            return 0;
        };
        if key >= piece.end.as_slice() {
            return 0;
        }
        let i = piece.seqs.partition_point(|&q| q <= seq);
        if i == 0 { 0 } else { piece.seqs[i - 1] }
    }

    /// The same fragments `rangedel::fragment` gives for the tombstones at
    /// or before `seq`: adjacent pieces of equal sequence merged.
    fn fragments(&self, seq: u64) -> Vec<Fragment> {
        let mut out: Vec<Fragment> = Vec::new();
        for (start, piece) in &self.pieces {
            let i = piece.seqs.partition_point(|&q| q <= seq);
            if i == 0 {
                continue;
            }
            let q = piece.seqs[i - 1];
            match out.last_mut() {
                Some(last) if last.end == *start && last.seq == q => last.end.clone_from(&piece.end),
                _ => out.push(Fragment { start: start.clone(), end: piece.end.clone(), seq: q }),
            }
        }
        out
    }
}

pub struct Memtable {
    map: SkipMap<InternalKey, Value>,
    /// Range tombstones, fragmented incrementally as they are applied. A
    /// reader at snapshot `seq` ignores those committed after it, like it
    /// does the points.
    ranges: RwLock<RangeIndex>,
    /// The fragments every reader at or after the newest tombstone sees,
    /// built on the first such read after a tombstone arrives (not on the
    /// write path) and shared by the reads that follow: `(count, list)`.
    latest: Mutex<Option<(usize, Arc<Vec<Fragment>>)>>,
    /// `ranges.len()`, readable without the lock (the common case is none).
    range_count: AtomicUsize,
    bytes: AtomicUsize,
    max_seq: AtomicU64,
    /// The WAL file that holds this memtable's commits.
    wal_number: u64,
    /// A frozen memtable whose commit wrote no WAL frame (`Buffered`
    /// durability, see [`Memtable::frozen_logless`]): it has no log of its
    /// own and is durable only once its table is in the MANIFEST.
    logless: bool,
    /// Replayed by a recovery whose flush was deferred: its table records
    /// `REASON_RECOVERY`.
    recovered: bool,
    /// The other logs a deferred recovery replayed into this memtable, after
    /// `wal_number` (or folded into it because they held no keys): the flush
    /// of the memtable retires them with it.
    extra_wals: Vec<u64>,
    /// Bytes written to that file when this memtable stopped being the
    /// active one (0: not known, or still active).
    wal_len: AtomicU64,
    /// Set when the memtable left the current version (it was written out):
    /// from then on only snapshots keep it alive.
    retired: OnceLock<Retired>,
    /// A bloom filter of the user keys in `map`, so that a point read of a
    /// key this memtable never held skips the search of the skip list (most
    /// reads of a write-heavy load look for keys that were flushed long ago,
    /// and pay that search in the active memtable and in every one waiting
    /// to be flushed). Allocated by the first write.
    ///
    /// The writer sets a key's bits before it inserts the key, and a reader
    /// only looks for entries at or below a sequence that was published
    /// after its commit was applied, so a clear bit means the key is not
    /// among the entries the reader may see. A set bit means nothing.
    filter: OnceLock<Box<[AtomicU64]>>,
    /// A memtable that holds one large commit as it came, already sorted (see
    /// [`Memtable::frozen`]): its points are read by binary search, `map` stays empty.
    frozen: Option<Frozen>,
}

/// A point of a frozen memtable: a key and its value (`None`: a tombstone).
pub type FrozenPoint = (Vec<u8>, Option<Vec<u8>>);

/// The one commit of a frozen memtable: its sequence and its canonical form, whose points are
/// sorted by key, each key once (WAL-C1).
struct Frozen {
    seq: u64,
    commit: Arc<CanonicalCommit<Vec<u8>>>,
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
            ranges: RwLock::new(RangeIndex::default()),
            latest: Mutex::new(None),
            range_count: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
            max_seq: AtomicU64::new(0),
            wal_number,
            logless: false,
            recovered: false,
            extra_wals: Vec::new(),
            wal_len: AtomicU64::new(0),
            retired: OnceLock::new(),
            filter: OnceLock::new(),
            frozen: None,
        }
    }

    /// A memtable that holds only the commit `commit` under sequence `seq`, kept as it is: no
    /// skip-list node, no copy of a key or value, reads by binary search. It is full from the
    /// start, so the writer switches to a new memtable for the next commit. Same answers as a
    /// memtable the commit was applied to (recovery replays the commit into an ordinary one).
    pub fn frozen(wal_number: u64, seq: u64, commit: Arc<CanonicalCommit<Vec<u8>>>) -> Self {
        let mut m = Memtable::new(wal_number);
        if !commit.ranges.is_empty() {
            let ranges = m.ranges.get_mut();
            for (start, end) in &commit.ranges {
                ranges.insert(start, end, seq);
            }
            m.range_count.store(ranges.count, Relaxed);
        }
        let bytes = commit.points.iter().map(|(k, v)| k.len() + v.as_ref().map_or(0, Vec::len) + 16).sum::<usize>()
            + commit.ranges.iter().map(|(s, e)| s.len() + e.len() + ENTRY_OVERHEAD).sum::<usize>();
        m.bytes.store(bytes, Relaxed);
        m.max_seq.store(seq, Relaxed);
        m.frozen = Some(Frozen { seq, commit });
        m
    }

    /// [`frozen`](Self::frozen) for a commit that wrote no WAL frame: under
    /// `Buffered` durability a large commit goes to its table without one. It
    /// names no log (`wal_number` 0), so its flush retires none and the
    /// MANIFEST's oldest live log is that of the next memtable that has one.
    pub fn frozen_logless(seq: u64, commit: Arc<CanonicalCommit<Vec<u8>>>) -> Self {
        let mut m = Memtable::frozen(0, seq, commit);
        m.logless = true;
        m
    }

    /// Whether this memtable's commit is in no WAL (see [`frozen_logless`](Self::frozen_logless)).
    pub fn is_logless(&self) -> bool {
        self.logless
    }

    /// A memtable that recovery replays into (see `recovery::apply`).
    pub fn recovered(wal_number: u64) -> Self {
        let mut m = Memtable::new(wal_number);
        m.recovered = true;
        m
    }

    pub fn is_recovered(&self) -> bool {
        self.recovered
    }

    /// Records that log `number` also belongs to this memtable (see
    /// [`extra_wals`](Self::extra_wals)); the log it is named for is not one.
    pub fn add_wal(&mut self, number: u64) {
        if number != 0 && number != self.wal_number && !self.extra_wals.contains(&number) {
            self.extra_wals.push(number);
        }
    }

    /// The logs besides [`wal_number`](Self::wal_number) that hold only
    /// commits of this memtable.
    pub fn extra_wals(&self) -> &[u64] {
        &self.extra_wals
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

    /// Raises `max_seq` to `seq`: the sequences of commits that wrote no keys
    /// and were dropped with an empty memtable still count as flushed with
    /// this one.
    pub fn raise_max_seq(&self, seq: u64) {
        self.max_seq.fetch_max(seq, Relaxed);
    }

    /// Whether nothing, points or range tombstones, was applied.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
            && self.range_count() == 0
            && self.frozen.as_ref().is_none_or(|f| f.commit.points.is_empty())
    }

    /// Range tombstones applied so far.
    pub fn range_count(&self) -> usize {
        self.range_count.load(Relaxed)
    }

    /// Whether the memtable takes no more commits: it holds `write_buffer`
    /// bytes or [`MAX_RANGE_TOMBSTONES`] range tombstones.
    pub fn is_full(&self, write_buffer: usize) -> bool {
        self.frozen.is_some() || self.approx_bytes() >= write_buffer || self.range_count() >= MAX_RANGE_TOMBSTONES
    }

    /// Applies a commit, in canonical form (WAL-C1), under sequence `seq`:
    /// the memtable holds exactly what the WAL replays, so a commit has the
    /// same effect before and after a restart. The ranges go in before the
    /// points, as the canonical form orders them (WAL-C2): a point of the
    /// same commit survives a range of it (SEM-3).
    pub fn apply_commit<B: AsRef<[u8]>>(&self, seq: u64, commit: &CanonicalCommit<B>) {
        debug_assert!(self.frozen.is_none(), "a frozen memtable takes no commit");
        if !commit.ranges.is_empty() {
            let mut ranges = self.ranges.write();
            for (start, end) in &commit.ranges {
                let (start, end) = (start.as_ref(), end.as_ref());
                self.bytes.fetch_add(start.len() + end.len() + ENTRY_OVERHEAD, Relaxed);
                ranges.insert(start, end, seq);
            }
            self.range_count.store(ranges.count, Relaxed);
        }
        let filter = (!commit.points.is_empty())
            .then(|| self.filter.get_or_init(|| (0..FILTER_BITS / 64).map(|_| AtomicU64::new(0)).collect()));
        for (key, value) in &commit.points {
            let (key, value) = (key.as_ref(), value.as_ref().map(|v| v.as_ref().to_vec()));
            if let Some(words) = filter {
                for bit in filter_bits(crate::format::bloom::hash(key)) {
                    words[(bit / 64) as usize].fetch_or(1 << (bit % 64), Relaxed);
                }
            }
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
        if let Some(f) = &self.frozen {
            if f.seq > probe.seq {
                return None;
            }
            let at = f.commit.points.binary_search_by(|(k, _)| k.as_slice().cmp(&probe.user_key)).ok()?;
            return Some((f.seq, f.commit.points[at].1.clone()));
        }
        let e = self.map.lower_bound(Bound::Included(probe))?;
        (e.key().user_key == probe.user_key).then(|| (e.key().seq, e.value().clone()))
    }

    /// Whether the memtable may hold a version of the key with the bloom
    /// hash `hash` (`format::bloom::hash`): `false` only when it holds none
    /// that a reader may see (see the `filter` field).
    pub fn may_hold(&self, hash: u64) -> bool {
        if self.frozen.is_some() {
            // No filter: the binary search is the check.
            return true;
        }
        match self.filter.get() {
            Some(words) => {
                filter_bits(hash).iter().all(|bit| words[(bit / 64) as usize].load(Relaxed) & 1 << (bit % 64) != 0)
            }
            // No point write yet.
            None => false,
        }
    }

    /// The highest sequence `<= seq` of the range tombstones that cover `key`
    /// (0 if none does). `O(log n)` in the memtable's tombstones.
    pub fn covering_seq(&self, key: &[u8], seq: u64) -> u64 {
        if self.range_count() == 0 {
            return 0;
        }
        self.ranges.read().covering_seq(key, seq)
    }

    /// The range tombstones committed at or before `seq` as sorted, disjoint
    /// fragments, each with the highest sequence that covers it. Readers at
    /// or after the newest tombstone share one list, built once per change.
    /// `None` when there are none (no allocation on the common path).
    pub fn fragments(&self, seq: u64) -> Option<Arc<Vec<Fragment>>> {
        if self.range_count() == 0 {
            return None;
        }
        let ranges = self.ranges.read();
        let list = if seq < ranges.max_seq {
            Arc::new(ranges.fragments(seq))
        } else {
            let mut latest = self.latest.lock();
            match latest.as_ref() {
                Some((count, list)) if *count == ranges.count => list.clone(),
                _ => {
                    let list = Arc::new(ranges.fragments(u64::MAX));
                    *latest = Some((ranges.count, list.clone()));
                    list
                }
            }
        };
        (!list.is_empty()).then_some(list)
    }

    /// The points of a frozen memtable without range tombstones, sorted by key, each key once,
    /// all under the returned sequence (`None` for any other memtable). The flush walks them
    /// directly: no tombstone can hide one and no other version of a key exists.
    pub fn frozen_points(&self) -> Option<(u64, &[FrozenPoint])> {
        let f = self.frozen.as_ref()?;
        (f.commit.ranges.is_empty() && self.range_count() == 0).then_some((f.seq, &f.commit.points[..]))
    }

    /// Iterator over every version visible at `seq` (several per key,
    /// newest first).
    pub fn iter(&self, seq: u64) -> MemIter<'_> {
        // A reader before the frozen commit sees none of it.
        let frozen = self.frozen.as_ref().map(|f| (f.seq, if f.seq <= seq { &f.commit.points[..] } else { &[][..] }));
        MemIter { map: &self.map, cur: None, seq, frozen, at: 0, positioned: false }
    }
}

pub struct MemIter<'a> {
    map: &'a SkipMap<InternalKey, Value>,
    cur: Option<Entry<'a, InternalKey, Value>>,
    seq: u64,
    /// A frozen memtable's sequence and the points the reader sees, walked by `at`.
    frozen: Option<(u64, &'a [FrozenPoint])>,
    at: usize,
    /// Set by the first seek (a frozen iterator is not valid before one, as a skip list's).
    positioned: bool,
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
        match self.frozen {
            Some((_, points)) => self.positioned && self.at < points.len(),
            None => self.cur.is_some(),
        }
    }

    fn seek_to_first(&mut self) -> Result<()> {
        if self.frozen.is_some() {
            (self.at, self.positioned) = (0, true);
            return Ok(());
        }
        self.cur = self.map.front();
        self.skip_invisible();
        Ok(())
    }

    fn seek(&mut self, target: &[u8]) -> Result<()> {
        if let Some((_, points)) = self.frozen {
            (self.at, self.positioned) = (points.partition_point(|(k, _)| k.as_slice() < target), true);
            return Ok(());
        }
        let probe = InternalKey { user_key: target.to_vec(), seq: u64::MAX };
        self.cur = self.map.lower_bound(Bound::Included(&probe));
        self.skip_invisible();
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        if self.frozen.is_some() {
            self.at += 1;
            return Ok(());
        }
        let next = self.entry().next();
        self.cur = next;
        self.skip_invisible();
        Ok(())
    }

    fn key(&self) -> &[u8] {
        match self.frozen {
            Some((_, points)) => &points[self.at].0,
            None => &self.entry().key().user_key,
        }
    }

    fn seq(&self) -> u64 {
        match self.frozen {
            Some((seq, _)) => seq,
            None => self.entry().key().seq,
        }
    }

    fn value(&self) -> Option<&[u8]> {
        match self.frozen {
            Some((_, points)) => points[self.at].1.as_deref(),
            None => self.entry().value().as_deref(),
        }
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

    /// The key filter never rules out a key the memtable holds, as a value or as a tombstone,
    /// and rules out most keys it does not.
    #[test]
    fn the_filter_never_hides_a_stored_key() {
        let m = Memtable::new(0);
        let hash = |k: &str| crate::format::bloom::hash(k.as_bytes());
        assert!(!m.may_hold(hash("anything")), "an empty memtable holds no key");
        let keys: Vec<String> = (0..5000).map(|i| format!("key-{i}")).collect();
        for (i, k) in keys.iter().enumerate() {
            apply(&m, i as u64 + 1, &[(k, if i % 3 == 0 { None } else { Some("v") })]);
        }
        for k in &keys {
            assert!(m.may_hold(hash(k)), "{k}");
            assert!(m.get(&probe(k, u64::MAX)).is_some());
        }
        let wrong = (0..5000).filter(|i| m.may_hold(hash(&format!("other-{i}")))).count();
        assert!(wrong < 50, "{wrong} of 5000 absent keys pass the filter");
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
        let frags = |seq| {
            m.fragments(seq)
                .unwrap_or_default()
                .iter()
                .map(|f| (f.start.clone(), f.end.clone(), f.seq))
                .collect::<Vec<_>>()
        };
        assert!(frags(1).is_empty());
        assert_eq!(frags(2), vec![(b"a".to_vec(), b"c".to_vec(), 2)]);
        assert_eq!(
            frags(3),
            vec![(b"a".to_vec(), b"b".to_vec(), 2), (b"b".to_vec(), b"d".to_vec(), 3)],
            "the highest sequence covers each part"
        );
    }

    /// The incremental index answers like the reference fragmenter and a
    /// linear pass over the tombstones, at every snapshot, whatever the
    /// order and overlap of the tombstones.
    #[test]
    fn range_index_matches_the_reference() {
        use crate::format::rangedel;
        use kerno_testkit::rng::Rng;
        for seed in 0..300u64 {
            let mut rng = Rng::new(seed);
            let m = Memtable::new(1);
            let mut tombs: Vec<(Vec<u8>, Vec<u8>, u64)> = Vec::new();
            let n = 1 + rng.below(40);
            for seq in 1..=n {
                let a = rng.below(26) as u8 + b'a';
                let b = rng.below(26) as u8 + b'a';
                let (s, e) = (vec![a.min(b)], vec![a.max(b) + u8::from(rng.below(3) == 0)]);
                apply_ops(&m, seq, &[CommitOp::DeleteRange { start: &s, end: &e }]);
                tombs.push((s, e, seq));
                for snap in [seq / 2, seq, u64::MAX] {
                    let visible: Vec<_> = tombs.iter().filter(|t| t.2 <= snap).cloned().collect();
                    assert_eq!(
                        m.fragments(snap).map(|l| l.to_vec()).unwrap_or_default(),
                        rangedel::fragment(&visible),
                        "seed {seed} seq {seq} snap {snap}"
                    );
                    for k in b'a'..=b'{' {
                        let want = visible
                            .iter()
                            .filter(|t| t.0.as_slice() <= [k].as_slice() && [k].as_slice() < t.1.as_slice())
                            .map(|t| t.2)
                            .max();
                        assert_eq!(m.covering_seq(&[k], snap), want.unwrap_or(0), "seed {seed} key {k} snap {snap}");
                    }
                }
            }
        }
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
