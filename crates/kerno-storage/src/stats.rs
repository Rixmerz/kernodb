//! Engine counters: the collectors behind [`Metrics`], reported through
//! [`KernoDb::metrics`](crate::KernoDb::metrics) and `Engine::stats`.
//!
//! Durations are collected in nanoseconds and reported in microseconds.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use crate::metrics::{
    BackgroundMetrics, CommitPhaseMetrics, LogHistogram, MaintenanceMetrics, Metrics, OpenMetrics, ReadMetrics,
    ShapeMetrics, ShardedCounter, SnapshotMetrics, StallMetrics, TxnMetrics, WriteMetrics,
};

#[derive(Default)]
pub struct Stats {
    // Writes.
    pub commits: AtomicU64,
    /// Groups of commits written as one frame, and the most commits in one.
    pub commit_groups: AtomicU64,
    pub group_commits_max: AtomicU64,
    /// Most bytes queued or in flight in the commit queue at once.
    pub queued_bytes_max: AtomicU64,
    pub wal_bytes: AtomicU64,
    pub wal_syncs: AtomicU64,
    pub memtable_rotations: AtomicU64,
    pub memtable_rotation_nanos: AtomicU64,
    pub memtable_rotation_nanos_max: AtomicU64,
    /// Rotations whose new WAL was the prepared spare (`Config::wal_preallocate`).
    pub wal_recycled: AtomicU64,

    // Commit phases (written by the group leader, under the writer lock).
    pub commit_queue: LogHistogram,
    pub commit_make_room: LogHistogram,
    pub commit_wal_write: LogHistogram,
    pub commit_wal_sync_busy: LogHistogram,
    pub commit_wal_sync_quiet: LogHistogram,
    pub commit_apply: LogHistogram,
    pub commit_total: LogHistogram,

    // Stalls.
    pub stall_episodes_memtable: AtomicU64,
    pub stall_episodes_l0: AtomicU64,
    pub stall_nanos_memtable: AtomicU64,
    pub stall_nanos_l0: AtomicU64,
    pub stall_nanos_max: AtomicU64,
    pub slowdown_commits: AtomicU64,
    pub slowdown_nanos: AtomicU64,

    // Background work.
    /// Flushes and compactions writing right now.
    pub background_active: AtomicU64,
    pub flushes: AtomicU64,
    pub flush_bytes_written: AtomicU64,
    /// Writeback requests of SST writers (`bytes_per_writeback`).
    pub sst_writeback_hints: AtomicU64,
    pub flush_nanos_max: AtomicU64,
    pub compactions: AtomicU64,
    pub compaction_nanos_max: AtomicU64,
    pub trivial_moves: AtomicU64,
    pub subcompactions: AtomicU64,
    pub compactions_concurrent_max: AtomicU64,
    pub compaction_bytes_read: AtomicU64,
    pub compaction_bytes_written: AtomicU64,
    pub tombstones_dropped: AtomicU64,
    /// Range tombstone fragments a compaction did not write because no file
    /// below its output overlapped them.
    pub range_tombstones_dropped: AtomicU64,
    /// Points a compaction did not write because a newer fragment covers them.
    pub covered_points_dropped: AtomicU64,
    pub l0_compactions: AtomicU64,
    pub l0_compaction_input_bytes: AtomicU64,
    pub l0_compaction_input_bytes_max: AtomicU64,
    pub l0_compaction_l1_files: AtomicU64,
    pub l0_compaction_l1_files_max: AtomicU64,
    pub l0_compaction_l1_files_present: AtomicU64,
    pub l0_base_level_skips: AtomicU64,
    pub compactions_while_l0_slowdown: AtomicU64,
    pub l0_files_max: AtomicU64,

    // Reads (one slot per thread).
    pub bloom_checks: ShardedCounter,
    pub bloom_negatives: ShardedCounter,
    pub bloom_false_positives: ShardedCounter,
    pub block_cache_hits: ShardedCounter,
    pub block_cache_misses: ShardedCounter,

    // Maintenance.
    pub manifest_writes: AtomicU64,
    pub manifest_records: AtomicU64,
    pub manifest_checkpoints: AtomicU64,
    /// Size of the MANIFEST file now (a gauge).
    pub manifest_bytes: AtomicU64,
    pub obsolete_files_deleted: AtomicU64,
    /// Bytes of obsolete files queued for deletion and not yet deleted (a
    /// gauge).
    pub obsolete_bytes_pending: AtomicU64,
    pub delete_failures: AtomicU64,

    // Snapshots: bytes only they keep alive (gauges; see `Table::mark_obsolete`
    // and `Memtable::retire`).
    pub retained_table_bytes: AtomicU64,
    pub retained_memtable_bytes: AtomicU64,

    // Write transactions.
    pub txns_committed: AtomicU64,
    pub txns_aborted: AtomicU64,
    pub txn_gate_wait_nanos: AtomicU64,

    // The open of this handle.
    pub last_open_replayed_records: AtomicU64,
    pub last_open_replayed_bytes: AtomicU64,
}

/// Something [`bump`] can add to.
pub trait Counter {
    fn add(&self, n: u64);
}

impl Counter for AtomicU64 {
    fn add(&self, n: u64) {
        self.fetch_add(n, Relaxed);
    }
}

impl Counter for ShardedCounter {
    fn add(&self, n: u64) {
        ShardedCounter::add(self, n);
    }
}

pub fn bump(c: &impl Counter, n: u64) {
    c.add(n);
}

/// Raises `c` to `v` if it is lower.
pub fn bump_max(c: &AtomicU64, v: u64) {
    c.fetch_max(v, Relaxed);
}

/// Nanoseconds from `start` to `end` (0 if the clock went backwards).
pub fn nanos(start: Instant, end: Instant) -> u64 {
    end.saturating_duration_since(start).as_nanos().min(u128::from(u64::MAX)) as u64
}

/// Marks background I/O (a flush or a compaction) as running while it
/// lives, so that commits can tell whether their WAL sync competed with it.
pub struct BackgroundIo<'a>(&'a AtomicU64);

impl<'a> BackgroundIo<'a> {
    pub fn start(stats: &'a Stats) -> Self {
        stats.background_active.fetch_add(1, Relaxed);
        BackgroundIo(&stats.background_active)
    }
}

impl Drop for BackgroundIo<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}

/// Why a writer waits in `make_room`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StallReason {
    /// `max_immutable_memtables` are waiting for a flush.
    Memtable,
    /// L0 reached `l0_stop` files.
    L0,
}

/// One stall episode of one commit: counted once when the writer first
/// waits, whatever the number of wake-ups, and timed until it is dropped.
/// The time is split by the reason in force at each moment.
pub struct StallEpisode<'a> {
    stats: &'a Stats,
    /// Start of the episode, and the reason and start of its current part.
    state: Option<(Instant, StallReason, Instant)>,
}

impl<'a> StallEpisode<'a> {
    pub fn new(stats: &'a Stats) -> Self {
        StallEpisode { stats, state: None }
    }

    fn nanos_for(&self, reason: StallReason) -> &AtomicU64 {
        match reason {
            StallReason::Memtable => &self.stats.stall_nanos_memtable,
            StallReason::L0 => &self.stats.stall_nanos_l0,
        }
    }

    /// Leaves `nanos` that just elapsed out of the episode (a slowdown wait
    /// inside it, which is accounted as slowdown).
    pub fn exclude(&mut self, nanos: u64) {
        if let Some((start, reason, since)) = self.state {
            let d = std::time::Duration::from_nanos(nanos);
            self.state = Some((start + d, reason, since + d));
        }
    }

    /// Called before each wait of the writer.
    pub fn waiting(&mut self, reason: StallReason) {
        let now = Instant::now();
        match self.state {
            None => {
                bump(
                    match reason {
                        StallReason::Memtable => &self.stats.stall_episodes_memtable,
                        StallReason::L0 => &self.stats.stall_episodes_l0,
                    },
                    1,
                );
                self.state = Some((now, reason, now));
            }
            Some((start, current, since)) if current != reason => {
                bump(self.nanos_for(current), nanos(since, now));
                self.state = Some((start, reason, now));
            }
            Some(_) => {}
        }
    }
}

impl Drop for StallEpisode<'_> {
    fn drop(&mut self) {
        if let Some((start, reason, since)) = self.state {
            let now = Instant::now();
            bump(self.nanos_for(reason), nanos(since, now));
            bump_max(&self.stats.stall_nanos_max, nanos(start, now));
        }
    }
}

fn micros(c: &AtomicU64) -> u64 {
    c.load(Relaxed) / 1000
}

fn load(c: &AtomicU64) -> u64 {
    c.load(Relaxed)
}

impl Stats {
    /// The typed metrics, given what only the caller knows: the current
    /// shape (its `l0_files_max` is raised to the current L0) and the
    /// pending compaction bytes. The caller also fills in the live snapshots
    /// (`snapshots.live`, `oldest_seq`, `oldest_age_ms`), which the snapshot
    /// registry knows.
    pub fn metrics(&self, mut shape: ShapeMetrics, pending_compaction_bytes: u64) -> Metrics {
        let l0_now = shape.levels.first().map_or(0, |l| l.files);
        shape.l0_files_max = load(&self.l0_files_max).max(l0_now);
        let busy = self.commit_wal_sync_busy.snapshot();
        let quiet = self.commit_wal_sync_quiet.snapshot();
        let replayed_records = load(&self.last_open_replayed_records);
        Metrics {
            shape,
            writes: WriteMetrics {
                commits: load(&self.commits),
                commit_groups: load(&self.commit_groups),
                group_commits_max: load(&self.group_commits_max),
                queued_bytes_max: load(&self.queued_bytes_max),
                wal_bytes: load(&self.wal_bytes),
                wal_syncs: load(&self.wal_syncs),
                memtable_rotations: load(&self.memtable_rotations),
                memtable_rotation_micros: micros(&self.memtable_rotation_nanos),
                memtable_rotation_micros_max: micros(&self.memtable_rotation_nanos_max),
                wal_recycled: load(&self.wal_recycled),
            },
            commit_phases: CommitPhaseMetrics {
                queue: self.commit_queue.snapshot().latency(),
                make_room: self.commit_make_room.snapshot().latency(),
                wal_write: self.commit_wal_write.snapshot().latency(),
                wal_sync: busy.merge(&quiet).latency(),
                wal_sync_busy: busy.latency(),
                wal_sync_quiet: quiet.latency(),
                apply: self.commit_apply.snapshot().latency(),
                total: self.commit_total.snapshot().latency(),
            },
            stalls: StallMetrics {
                episodes_memtable: load(&self.stall_episodes_memtable),
                episodes_l0: load(&self.stall_episodes_l0),
                micros_memtable: micros(&self.stall_nanos_memtable),
                micros_l0: micros(&self.stall_nanos_l0),
                micros_max: micros(&self.stall_nanos_max),
                slowdown_commits: load(&self.slowdown_commits),
                slowdown_micros: micros(&self.slowdown_nanos),
            },
            background: BackgroundMetrics {
                flushes: load(&self.flushes),
                flush_bytes_written: load(&self.flush_bytes_written),
                sst_writeback_hints: load(&self.sst_writeback_hints),
                flush_micros_max: micros(&self.flush_nanos_max),
                compactions: load(&self.compactions),
                compaction_micros_max: micros(&self.compaction_nanos_max),
                trivial_moves: load(&self.trivial_moves),
                subcompactions: load(&self.subcompactions),
                compactions_concurrent_max: load(&self.compactions_concurrent_max),
                compaction_bytes_read: load(&self.compaction_bytes_read),
                compaction_bytes_written: load(&self.compaction_bytes_written),
                tombstones_dropped: load(&self.tombstones_dropped),
                range_tombstones_dropped: load(&self.range_tombstones_dropped),
                covered_points_dropped: load(&self.covered_points_dropped),
                l0_compactions: load(&self.l0_compactions),
                l0_compaction_input_bytes: load(&self.l0_compaction_input_bytes),
                l0_compaction_input_bytes_max: load(&self.l0_compaction_input_bytes_max),
                l0_compaction_l1_files: load(&self.l0_compaction_l1_files),
                l0_compaction_l1_files_max: load(&self.l0_compaction_l1_files_max),
                l0_compaction_l1_files_present: load(&self.l0_compaction_l1_files_present),
                l0_base_level_skips: load(&self.l0_base_level_skips),
                compactions_while_l0_slowdown: load(&self.compactions_while_l0_slowdown),
                pending_compaction_bytes,
            },
            reads: ReadMetrics {
                bloom_checks: self.bloom_checks.sum(),
                bloom_negatives: self.bloom_negatives.sum(),
                bloom_false_positives: self.bloom_false_positives.sum(),
                block_cache_hits: self.block_cache_hits.sum(),
                block_cache_misses: self.block_cache_misses.sum(),
            },
            maintenance: MaintenanceMetrics {
                manifest_writes: load(&self.manifest_writes),
                manifest_records: load(&self.manifest_records),
                manifest_checkpoints: load(&self.manifest_checkpoints),
                manifest_bytes: load(&self.manifest_bytes),
                obsolete_files_deleted: load(&self.obsolete_files_deleted),
                obsolete_bytes_pending: load(&self.obsolete_bytes_pending),
                delete_failures: load(&self.delete_failures),
            },
            snapshots: SnapshotMetrics {
                retained_bytes: load(&self.retained_table_bytes) + load(&self.retained_memtable_bytes),
                ..Default::default()
            },
            txns: TxnMetrics {
                committed: load(&self.txns_committed),
                aborted: load(&self.txns_aborted),
                gate_wait_micros: micros(&self.txn_gate_wait_nanos),
            },
            last_open: OpenMetrics {
                clean: replayed_records == 0,
                replayed_bytes: load(&self.last_open_replayed_bytes),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stall_episode_is_counted_once_and_split_by_reason() {
        let stats = Stats::default();
        {
            let mut e = StallEpisode::new(&stats);
            e.waiting(StallReason::Memtable);
            e.waiting(StallReason::Memtable);
            std::thread::sleep(std::time::Duration::from_millis(2));
            e.waiting(StallReason::L0);
            std::thread::sleep(std::time::Duration::from_millis(2));
            e.waiting(StallReason::L0);
        }
        // An episode that never waited counts nothing.
        drop(StallEpisode::new(&stats));
        let m = stats.metrics(ShapeMetrics::default(), 0);
        assert_eq!((m.stalls.episodes_memtable, m.stalls.episodes_l0), (1, 0));
        assert!(m.stalls.micros_memtable >= 2000);
        assert!(m.stalls.micros_l0 >= 2000);
        assert!(m.stalls.micros_max >= m.stalls.micros_memtable + m.stalls.micros_l0);
    }

    #[test]
    fn excluded_time_is_left_out_of_the_episode() {
        let stats = Stats::default();
        let mut e = StallEpisode::new(&stats);
        e.exclude(1_000_000_000); // before any wait: no effect
        let outer = Instant::now();
        e.waiting(StallReason::L0);
        std::thread::sleep(std::time::Duration::from_millis(5));
        // A 4 ms slowdown wait happened inside the episode.
        e.exclude(4_000_000);
        drop(e);
        let elapsed = outer.elapsed().as_micros() as u64;
        let m = stats.metrics(ShapeMetrics::default(), 0);
        assert_eq!(m.stalls.episodes_l0, 1);
        assert!(m.stalls.micros_l0 <= elapsed - 4_000, "{:?}, {elapsed} µs", m.stalls);
        assert!(m.stalls.micros_l0 >= 900, "{:?}", m.stalls);
        assert_eq!(m.stalls.micros_max, m.stalls.micros_l0);
    }

    #[test]
    fn background_io_guard_tracks_running_work() {
        let stats = Stats::default();
        let a = BackgroundIo::start(&stats);
        let b = BackgroundIo::start(&stats);
        assert_eq!(load(&stats.background_active), 2);
        drop(a);
        drop(b);
        assert_eq!(load(&stats.background_active), 0);
    }
}
