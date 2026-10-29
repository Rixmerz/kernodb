//! Typed engine metrics (phase1-design.md §3) and the primitives that
//! collect them.
//!
//! [`Metrics`] is a point-in-time snapshot returned by
//! [`KernoDb::metrics`](crate::KernoDb::metrics). [`Metrics::flatten`] turns
//! it into the stable `(name, value)` pairs of `Engine::stats`; every name
//! the M0 engine reported is kept, with one change of meaning:
//! `write_stalls` counts stall **episodes** (a writer entering the wait loop
//! once), no longer wake-ups.
//!
//! The collectors are cheap enough for the commit and read paths:
//!
//! * [`LogHistogram`]: log2 buckets, each split in four, so a bucket spans
//!   at most a quarter of its lower bound. Recording is one relaxed add into
//!   the bucket plus the running sum and maximum. Quantiles are interpolated
//!   inside their bucket, so they are approximate (within 25 %); sums,
//!   counts and maxima are exact.
//! * [`ShardedCounter`]: the read-path counters (bloom filter, block cache)
//!   are split into [`READ_COUNTER_SHARDS`] cache-line-aligned slots, one per
//!   thread (round-robin), and summed on read. The M0 engine used one shared
//!   atomic per counter, which every concurrent reader wrote.

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

/// Number of slots of every read-path counter.
pub const READ_COUNTER_SHARDS: usize = 16;

/// One slot of a [`ShardedCounter`], alone in its cache line.
#[repr(align(64))]
#[derive(Default)]
struct Slot(AtomicU64);

/// A counter written from many threads: each thread adds to its own slot.
pub(crate) struct ShardedCounter {
    slots: [Slot; READ_COUNTER_SHARDS],
}

impl Default for ShardedCounter {
    fn default() -> Self {
        ShardedCounter { slots: std::array::from_fn(|_| Slot::default()) }
    }
}

/// The slot of the calling thread, assigned round-robin on first use.
fn shard() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        // Constant-initialized: no lazy-initialization check on the read path.
        static SHARD: Cell<usize> = const { Cell::new(usize::MAX) };
    }
    SHARD.with(|s| {
        if s.get() == usize::MAX {
            s.set(NEXT.fetch_add(1, Relaxed) % READ_COUNTER_SHARDS);
        }
        s.get()
    })
}

impl ShardedCounter {
    pub fn add(&self, n: u64) {
        self.slots[shard()].0.fetch_add(n, Relaxed);
    }

    pub fn sum(&self) -> u64 {
        self.slots.iter().map(|s| s.0.load(Relaxed)).sum()
    }

    /// Number of slots that hold a non-zero count.
    #[cfg(test)]
    pub fn slots_used(&self) -> usize {
        self.slots.iter().filter(|s| s.0.load(Relaxed) != 0).count()
    }
}

/// Each power of two is split into `2^SUB_BITS` buckets.
const SUB_BITS: u32 = 2;
const SUB: usize = 1 << SUB_BITS;
/// Values below `SUB` have a bucket each; then `SUB` buckets for every
/// power of two from `2^SUB_BITS` to `2^63`.
const BUCKETS: usize = SUB + (u64::BITS - SUB_BITS) as usize * SUB;

fn bucket(v: u64) -> usize {
    if v < SUB as u64 {
        return v as usize;
    }
    let exp = u64::BITS - 1 - v.leading_zeros();
    let mantissa = (v >> (exp - SUB_BITS)) as usize & (SUB - 1);
    SUB + (exp - SUB_BITS) as usize * SUB + mantissa
}

/// Smallest and largest value of bucket `i`.
fn bucket_bounds(i: usize) -> (u128, u128) {
    if i < SUB {
        return (i as u128, i as u128);
    }
    let shift = ((i - SUB) / SUB) as u32;
    let mantissa = ((i - SUB) % SUB) as u128;
    let lo = (SUB as u128 + mantissa) << shift;
    let hi = ((SUB as u128 + mantissa + 1) << shift) - 1;
    (lo, hi)
}

/// A log2 histogram of durations in nanoseconds.
pub(crate) struct LogHistogram {
    buckets: [AtomicU64; BUCKETS],
    sum: AtomicU64,
    max: AtomicU64,
}

impl Default for LogHistogram {
    fn default() -> Self {
        LogHistogram {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            sum: AtomicU64::new(0),
            max: AtomicU64::new(0),
        }
    }
}

impl LogHistogram {
    /// Records a value from any thread (atomic read-modify-writes). The
    /// engine's histograms all have a single writer and use
    /// [`record_single_writer`](Self::record_single_writer).
    #[cfg(test)]
    pub fn record(&self, nanos: u64) {
        self.buckets[bucket(nanos)].fetch_add(1, Relaxed);
        self.sum.fetch_add(nanos, Relaxed);
        self.max.fetch_max(nanos, Relaxed);
    }

    /// [`record`](Self::record) for a histogram with a single writer at a
    /// time (the commit-phase histograms, written under the writer lock):
    /// relaxed loads and stores, no read-modify-write. Concurrent calls
    /// would lose counts.
    pub fn record_single_writer(&self, nanos: u64) {
        let b = &self.buckets[bucket(nanos)];
        b.store(b.load(Relaxed) + 1, Relaxed);
        self.sum.store(self.sum.load(Relaxed).wrapping_add(nanos), Relaxed);
        if nanos > self.max.load(Relaxed) {
            self.max.store(nanos, Relaxed);
        }
    }

    pub fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            buckets: std::array::from_fn(|i| self.buckets[i].load(Relaxed)),
            sum: self.sum.load(Relaxed),
            max: self.max.load(Relaxed),
        }
    }
}

/// The values of a [`LogHistogram`] at one point in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HistogramSnapshot {
    buckets: [u64; BUCKETS],
    sum: u64,
    max: u64,
}

impl Default for HistogramSnapshot {
    fn default() -> Self {
        HistogramSnapshot { buckets: [0; BUCKETS], sum: 0, max: 0 }
    }
}

impl HistogramSnapshot {
    pub fn merge(mut self, other: &HistogramSnapshot) -> Self {
        for (a, b) in self.buckets.iter_mut().zip(&other.buckets) {
            *a += b;
        }
        self.sum += other.sum;
        self.max = self.max.max(other.max);
        self
    }

    pub fn count(&self) -> u64 {
        self.buckets.iter().sum()
    }

    /// The `q`-quantile in nanoseconds (0 when empty), interpolated
    /// linearly inside its bucket and never above the recorded maximum.
    pub fn quantile(&self, q: f64) -> u64 {
        let count = self.count();
        if count == 0 {
            return 0;
        }
        let rank = ((q.clamp(0.0, 1.0) * count as f64).ceil() as u64).clamp(1, count);
        let mut before = 0u64;
        for (i, &c) in self.buckets.iter().enumerate() {
            if c == 0 || before + c < rank {
                before += c;
                continue;
            }
            let (lo, hi) = bucket_bounds(i);
            let value = lo + (hi - lo) * u128::from(rank - before) / u128::from(c);
            return (value.min(u128::from(self.max))) as u64;
        }
        self.max
    }

    pub fn latency(&self) -> LatencyMetrics {
        LatencyMetrics {
            count: self.count(),
            total_micros: self.sum / 1000,
            max_micros: self.max / 1000,
            p50_micros: self.quantile(0.50) / 1000,
            p99_micros: self.quantile(0.99) / 1000,
        }
    }
}

/// Distribution of the duration of one phase (microseconds). Percentiles
/// come from a log2 histogram and are approximate (within 25 %); `count`,
/// `total_micros` and `max_micros` are exact (up to truncation to whole
/// microseconds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LatencyMetrics {
    pub count: u64,
    pub total_micros: u64,
    pub max_micros: u64,
    pub p50_micros: u64,
    pub p99_micros: u64,
}

/// Files and bytes of one LSM level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct LevelMetrics {
    pub files: u64,
    pub bytes: u64,
}

/// The LSM shape now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ShapeMetrics {
    /// One entry per level, L0 first.
    pub levels: Vec<LevelMetrics>,
    /// Maximum number of L0 files whose key ranges contain one same key
    /// (at most `levels[0].files`).
    pub l0_depth: u64,
    /// Range tombstone fragments in the files of the current version.
    pub range_fragments: u64,
    /// Largest number of L0 files this handle has seen.
    pub l0_files_max: u64,
    pub memtable_bytes: u64,
    pub immutable_memtables: u64,
    /// Last commit visible to new snapshots.
    pub last_seq: u64,
    /// Every commit up to this sequence survives a power loss: `last_seq` in
    /// `Sync` mode, the last `sync()` or WAL rotation in `Buffered` mode.
    pub durable_seq: u64,
    pub block_cache_bytes: u64,
}

/// The write path, since the handle was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct WriteMetrics {
    /// Non-empty commits that succeeded.
    pub commits: u64,
    /// WAL frames written for commits (one per group), and the most commits
    /// one of them held.
    pub commit_groups: u64,
    pub group_commits_max: u64,
    /// Most bytes of commits queued or being written at once.
    pub queued_bytes_max: u64,
    pub wal_bytes: u64,
    pub wal_syncs: u64,
    pub memtable_rotations: u64,
    /// Time spent switching to a new memtable inside commits (new WAL and
    /// `sync_dir`, phase1-design.md §5.4 H4): total and largest.
    pub memtable_rotation_micros: u64,
    pub memtable_rotation_micros_max: u64,
    /// Rotations whose new WAL was the prepared spare, preallocated with
    /// zeros (`Config::wal_preallocate`).
    pub wal_recycled: u64,
}

/// Duration of each phase of a successful non-empty commit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CommitPhaseMetrics {
    /// From joining the commit queue until the group it is in starts (one
    /// sample per commit).
    pub queue: LatencyMetrics,
    /// Backpressure (slowdown and stalls) and the memtable switch (one sample
    /// per group, like the three phases below).
    pub make_room: LatencyMetrics,
    /// Encoding the group's frame and writing it to the WAL.
    pub wal_write: LatencyMetrics,
    /// `sync_data` of the WAL (`Sync` durability, or a `sync()` call).
    pub wal_sync: LatencyMetrics,
    /// `wal_sync` while a flush or compaction was writing (H3).
    pub wal_sync_busy: LatencyMetrics,
    /// `wal_sync` while no flush or compaction was writing.
    pub wal_sync_quiet: LatencyMetrics,
    /// Inserting the group into the memtable.
    pub apply: LatencyMetrics,
    /// The whole commit call, including argument checks and the wait for
    /// the queue and the writer lock (one sample per commit).
    pub total: LatencyMetrics,
}

/// Writers held back by background work. An episode is one entry of a
/// commit into the wait loop, whatever the number of wake-ups; its reason is
/// the one that made it wait first, and its time is split by the reason in
/// force at each moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct StallMetrics {
    /// Episodes that began because `max_immutable_memtables` were waiting
    /// for a flush.
    pub episodes_memtable: u64,
    /// Episodes that began because L0 reached `l0_stop` files.
    pub episodes_l0: u64,
    pub micros_memtable: u64,
    pub micros_l0: u64,
    /// The longest episode.
    pub micros_max: u64,
    /// Commits delayed once because L0 had `l0_slowdown` files or more.
    pub slowdown_commits: u64,
    pub slowdown_micros: u64,
}

/// Flushes and compactions, since the handle was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BackgroundMetrics {
    /// Memtables written to L0: by the flush thread, by recovery when
    /// `open` replayed WAL records, and by a clean close.
    pub flushes: u64,
    pub flush_bytes_written: u64,
    /// Writeback requests (`File::start_writeback`) made by SST writers
    /// (`bytes_per_writeback`); 0 when the option is off.
    pub sst_writeback_hints: u64,
    /// The longest flush by the flush thread (SST, `sync_dir`, MANIFEST).
    pub flush_micros_max: u64,
    /// Compactions that rewrote files (trivial moves excluded).
    pub compactions: u64,
    pub compaction_micros_max: u64,
    pub trivial_moves: u64,
    /// Key ranges merged in parallel by split compactions
    /// (`max_subcompactions`); 0 when none was split.
    pub subcompactions: u64,
    /// The most compactions that ran at once.
    pub compactions_concurrent_max: u64,
    pub compaction_bytes_read: u64,
    pub compaction_bytes_written: u64,
    pub tombstones_dropped: u64,
    /// Range tombstone fragments left out of a compaction's output because
    /// no deeper file overlaps them.
    pub range_tombstones_dropped: u64,
    /// Points left out of a compaction's output because a newer range
    /// tombstone covers them.
    pub covered_points_dropped: u64,
    /// Compactions from L0 into L1, with their input bytes (L0 and L1
    /// files) and the number of L1 files they rewrote (H1): totals and
    /// largest.
    pub l0_compactions: u64,
    pub l0_compaction_input_bytes: u64,
    pub l0_compaction_input_bytes_max: u64,
    pub l0_compaction_l1_files: u64,
    pub l0_compaction_l1_files_max: u64,
    /// L1 files that existed when those compactions ran (summed): with
    /// `l0_compaction_l1_files`, the share of L1 each one rewrote.
    pub l0_compaction_l1_files_present: u64,
    /// L0 compactions whose output went below an empty L1
    /// (`dynamic_base_level`); 0 when the option is off.
    pub l0_base_level_skips: u64,
    /// Compactions of a deeper level chosen while L0 had `l0_slowdown`
    /// files or more (H2: L0 has no priority).
    pub compactions_while_l0_slowdown: u64,
    /// Bytes leveled compaction still has to move now: for L1 up to the
    /// second-to-last level, the bytes above the level target, plus every
    /// L0 byte once L0 has `l0_trigger` files.
    pub pending_compaction_bytes: u64,
}

/// The read path. Summed over the per-thread slots.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReadMetrics {
    pub bloom_checks: u64,
    pub bloom_negatives: u64,
    pub bloom_false_positives: u64,
    pub block_cache_hits: u64,
    pub block_cache_misses: u64,
}

/// File maintenance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct MaintenanceMetrics {
    /// MANIFEST changes: appended records plus checkpoints.
    pub manifest_writes: u64,
    /// Records appended to the MANIFEST (v1.md §8, W-3).
    pub manifest_records: u64,
    /// New MANIFEST generations written (W-10).
    pub manifest_checkpoints: u64,
    /// Size of the MANIFEST file now.
    pub manifest_bytes: u64,
    /// Obsolete files deleted by the deleter (W-13).
    pub obsolete_files_deleted: u64,
    /// Bytes of obsolete files waiting to be deleted.
    pub obsolete_bytes_pending: u64,
    /// Deletions that failed (each retry that fails counts again).
    pub delete_failures: u64,
}

/// What the `open` of this handle found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpenMetrics {
    /// No WAL record was replayed (a clean close, or a new database).
    pub clean: bool,
    /// Bytes of valid WAL records replayed.
    pub replayed_bytes: u64,
}

/// A point-in-time snapshot of the metrics of a handle
/// (phase1-design.md §3). Counters run from the `open` of the handle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Metrics {
    pub shape: ShapeMetrics,
    pub writes: WriteMetrics,
    pub commit_phases: CommitPhaseMetrics,
    pub stalls: StallMetrics,
    pub background: BackgroundMetrics,
    pub reads: ReadMetrics,
    pub maintenance: MaintenanceMetrics,
    pub last_open: OpenMetrics,
}

impl Metrics {
    /// The stable `(name, value)` pairs of `Engine::stats`.
    pub fn flatten(&self) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = Vec::with_capacity(128);
        let mut put = |name: &str, v: u64| out.push((name.to_string(), v));

        let s = &self.shape;
        for (i, l) in s.levels.iter().enumerate() {
            put(&format!("l{i}_files"), l.files);
            put(&format!("l{i}_bytes"), l.bytes);
        }
        put("l0_depth", s.l0_depth);
        put("range_fragments", s.range_fragments);
        put("l0_files_max", s.l0_files_max);
        put("memtable_bytes", s.memtable_bytes);
        put("immutable_memtables", s.immutable_memtables);
        put("last_seq", s.last_seq);
        put("durable_seq", s.durable_seq);
        put("block_cache_bytes", s.block_cache_bytes);

        let w = &self.writes;
        put("commits", w.commits);
        put("commit_groups", w.commit_groups);
        put("group_commits_max", w.group_commits_max);
        put("queued_bytes_max", w.queued_bytes_max);
        put("wal_bytes", w.wal_bytes);
        put("wal_syncs", w.wal_syncs);
        put("memtable_rotations", w.memtable_rotations);
        put("memtable_rotation_micros", w.memtable_rotation_micros);
        put("memtable_rotation_micros_max", w.memtable_rotation_micros_max);
        put("wal_recycled", w.wal_recycled);

        let c = &self.commit_phases;
        for (phase, l) in [
            ("queue", &c.queue),
            ("make_room", &c.make_room),
            ("wal_write", &c.wal_write),
            ("wal_sync", &c.wal_sync),
            ("wal_sync_busy", &c.wal_sync_busy),
            ("wal_sync_quiet", &c.wal_sync_quiet),
            ("apply", &c.apply),
            ("total", &c.total),
        ] {
            put(&format!("commit_{phase}_count"), l.count);
            put(&format!("commit_{phase}_micros"), l.total_micros);
            put(&format!("commit_{phase}_max_micros"), l.max_micros);
            put(&format!("commit_{phase}_p50_micros"), l.p50_micros);
            put(&format!("commit_{phase}_p99_micros"), l.p99_micros);
        }

        let st = &self.stalls;
        put("stall_episodes_memtable", st.episodes_memtable);
        put("stall_episodes_l0", st.episodes_l0);
        put("stall_micros_memtable", st.micros_memtable);
        put("stall_micros_l0", st.micros_l0);
        put("stall_micros_max", st.micros_max);
        put("slowdown_commits", st.slowdown_commits);
        put("slowdown_micros", st.slowdown_micros);
        // M0 names.
        put("write_stalls", st.episodes_memtable + st.episodes_l0);
        put("write_stall_micros", st.micros_memtable + st.micros_l0);
        put("write_slowdowns", st.slowdown_commits);

        let b = &self.background;
        put("flushes", b.flushes);
        put("flush_bytes_written", b.flush_bytes_written);
        put("sst_writeback_hints", b.sst_writeback_hints);
        put("flush_micros_max", b.flush_micros_max);
        put("compactions", b.compactions);
        put("compaction_micros_max", b.compaction_micros_max);
        put("trivial_moves", b.trivial_moves);
        put("subcompactions", b.subcompactions);
        put("compactions_concurrent_max", b.compactions_concurrent_max);
        put("compaction_bytes_read", b.compaction_bytes_read);
        put("compaction_bytes_written", b.compaction_bytes_written);
        put("tombstones_dropped", b.tombstones_dropped);
        put("range_tombstones_dropped", b.range_tombstones_dropped);
        put("covered_points_dropped", b.covered_points_dropped);
        put("l0_compactions", b.l0_compactions);
        put("l0_compaction_input_bytes", b.l0_compaction_input_bytes);
        put("l0_compaction_input_bytes_max", b.l0_compaction_input_bytes_max);
        put("l0_compaction_l1_files", b.l0_compaction_l1_files);
        put("l0_compaction_l1_files_max", b.l0_compaction_l1_files_max);
        put("l0_compaction_l1_files_present", b.l0_compaction_l1_files_present);
        put("l0_base_level_skips", b.l0_base_level_skips);
        put("compactions_while_l0_slowdown", b.compactions_while_l0_slowdown);
        put("pending_compaction_bytes", b.pending_compaction_bytes);

        let r = &self.reads;
        put("bloom_checks", r.bloom_checks);
        put("bloom_negatives", r.bloom_negatives);
        put("bloom_false_positives", r.bloom_false_positives);
        put("block_cache_hits", r.block_cache_hits);
        put("block_cache_misses", r.block_cache_misses);

        let m = &self.maintenance;
        put("manifest_writes", m.manifest_writes);
        put("manifest_records", m.manifest_records);
        put("manifest_checkpoints", m.manifest_checkpoints);
        put("manifest_bytes", m.manifest_bytes);
        put("obsolete_files_deleted", m.obsolete_files_deleted);
        put("obsolete_bytes_pending", m.obsolete_bytes_pending);
        put("delete_failures", m.delete_failures);

        put("last_open_clean", u64::from(self.last_open.clean));
        put("last_open_replayed_bytes", self.last_open.replayed_bytes);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharded_counter_slots_are_cache_line_aligned() {
        assert_eq!(std::mem::align_of::<Slot>(), 64);
        assert_eq!(std::mem::size_of::<Slot>(), 64);
        assert_eq!(std::mem::size_of::<ShardedCounter>(), 64 * READ_COUNTER_SHARDS);
    }

    #[test]
    fn sharded_counter_sums_every_thread() {
        let c = ShardedCounter::default();
        std::thread::scope(|s| {
            for _ in 0..READ_COUNTER_SHARDS + 3 {
                s.spawn(|| {
                    for _ in 0..1000 {
                        c.add(1);
                    }
                });
            }
        });
        assert_eq!(c.sum(), 1000 * (READ_COUNTER_SHARDS as u64 + 3));
        // Threads write to more than one slot.
        assert!(c.slots.iter().filter(|s| s.0.load(Relaxed) > 0).count() > 1);
    }

    #[test]
    fn buckets_tile_every_value() {
        assert_eq!(bucket(0), 0);
        assert_eq!(bucket(u64::MAX), BUCKETS - 1);
        let mut next = 0u128;
        for i in 0..BUCKETS {
            let (lo, hi) = bucket_bounds(i);
            assert_eq!(lo, next, "bucket {i}");
            assert!(hi >= lo);
            assert_eq!(bucket(lo as u64), i);
            assert_eq!(bucket(hi as u64), i);
            // A bucket spans at most a quarter of its lower bound.
            assert!(lo < 4 || (hi - lo + 1) * 4 <= lo, "bucket {i}: {lo}..={hi}");
            next = hi + 1;
        }
        assert_eq!(next, 1u128 << 64);
    }

    #[test]
    fn histogram_buckets_and_quantiles() {
        let h = LogHistogram::default();
        assert_eq!(h.snapshot().quantile(0.99), 0);
        assert_eq!(h.snapshot().latency(), LatencyMetrics::default());
        // 99 values of 1 µs and one of 1 ms: p50 near 1 µs, p99 still in
        // the 1 µs bucket, max exact.
        for _ in 0..99 {
            h.record(1_000);
        }
        h.record(1_000_000);
        let s = h.snapshot();
        assert_eq!(s.count(), 100);
        let p50 = s.quantile(0.5);
        assert!((800..=1250).contains(&p50), "p50 {p50}");
        assert!((800..=1250).contains(&s.quantile(0.99)));
        assert_eq!(s.quantile(1.0), 1_000_000);
        let l = s.latency();
        assert_eq!((l.count, l.total_micros, l.max_micros), (100, 1099, 1000));
        // A value of 0 and the largest value fit.
        h.record(0);
        h.record(u64::MAX);
        let s = h.snapshot();
        assert_eq!(s.quantile(0.0), 0);
        assert_eq!(s.quantile(1.0), u64::MAX);
    }

    #[test]
    fn single_writer_record_matches_record() {
        let (a, b) = (LogHistogram::default(), LogHistogram::default());
        for v in [0u64, 7, 1_000, 999_999, 3, 1_000, u64::MAX / 4] {
            a.record(v);
            b.record_single_writer(v);
        }
        assert_eq!(a.snapshot(), b.snapshot());
    }

    #[test]
    fn quantile_stays_within_the_bucket_and_below_max() {
        let h = LogHistogram::default();
        for v in [3_000_000u64, 3_100_000, 3_200_000, 2_500_000] {
            h.record(v);
        }
        let s = h.snapshot();
        let p99 = s.quantile(0.99);
        assert!((2_560_000..=3_200_000).contains(&p99), "p99 {p99}");
        let merged = s.merge(&s);
        assert_eq!(merged.count(), 8);
        assert_eq!(merged.quantile(1.0), 3_200_000);
    }

    #[test]
    fn flatten_keeps_m0_names_and_derives_them() {
        let m = Metrics {
            shape: ShapeMetrics { levels: vec![LevelMetrics::default(); 7], ..Default::default() },
            stalls: StallMetrics {
                episodes_memtable: 2,
                episodes_l0: 3,
                micros_memtable: 10,
                micros_l0: 20,
                slowdown_commits: 4,
                ..Default::default()
            },
            ..Default::default()
        };
        let stats = m.flatten();
        let get = |n: &str| stats.iter().find(|(k, _)| k == n).map(|(_, v)| *v);
        assert_eq!(get("write_stalls"), Some(5));
        assert_eq!(get("write_stall_micros"), Some(30));
        assert_eq!(get("write_slowdowns"), Some(4));
        assert_eq!(get("l6_bytes"), Some(0));
        assert_eq!(get("commit_queue_p99_micros"), Some(0));
        let mut names: Vec<&str> = stats.iter().map(|(k, _)| k.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate stat names");
    }
}
