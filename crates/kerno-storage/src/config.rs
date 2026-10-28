//! Configuration of a [`KernoDb`](crate::KernoDb) handle.
//!
//! Phase 1 API, explicitly unstable (phase1-design.md §3). Later work
//! packages add the fields that are still missing. The default configuration
//! keeps the M0 engine's tuning.

use std::time::Duration;

use kerno_core::contract::Options;
use kerno_core::{Durability, Error, Result};

use crate::commit_queue::QueueLimits;
use crate::compaction::{L0_COMPACTION_TRIGGER, L0_SLOWDOWN, L0_STOP, LEVEL_SIZE_RATIO, MAX_IMMUTABLE};
use crate::format::limits;

/// The smallest write buffer the engine uses (smaller values are raised).
pub(crate) const MIN_WRITE_BUFFER: usize = 4 << 10;

/// Whether a handle may write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    /// Reads and writes.
    #[default]
    ReadWrite,
    /// Reads only: the open takes the exclusive `LOCK`, classifies the
    /// directory and replays the live WALs in memory, and never creates,
    /// syncs, deletes or checkpoints anything (v1.md §9.5, RO-1..RO-3).
    /// Writes fail with [`Error::ReadOnly`]. A database whose files carry
    /// `ro_compat` bits this engine does not know opens like this even when
    /// read-write was asked for.
    ReadOnly,
}

/// Why a handle is read-only ([`KernoDb::is_read_only`](crate::KernoDb::is_read_only)).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadOnlyReason {
    /// [`AccessMode::ReadOnly`] was requested.
    Requested,
    /// A file has `ro_compat` bits this engine does not know (HDR-7): it can
    /// still read the database, but writing could break what those bits
    /// protect.
    UnknownRoCompat {
        /// Name of the first such file.
        file: String,
        /// The unknown bits of its header.
        bits: u64,
    },
}

/// How a handle runs flushes and compactions (test configuration, see
/// [`TestConfig`]).
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Background {
    /// Two background threads, `kerno-flush` and `kerno-compact-0`, as in
    /// the M0 engine (the default).
    #[default]
    Threads,
    /// No threads. The commit that finds the active memtable full rotates
    /// it, writes the immutable memtable to L0 and runs every compaction that
    /// is due, all before it writes its own WAL record; `close` flushes and
    /// compacts in the calling thread as always. The sequence of file-system
    /// operations then depends only on the calls made, so crash sweeps
    /// number their points identically in every run (phase1-design.md §1
    /// item 6).
    Inline,
}

/// Settings that only tests use. Not part of the API.
#[doc(hidden)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestConfig {
    /// Background threads (default) or inline flushes and compactions.
    pub background: Background,
    /// Fixed database id for a new database, so that test vectors are
    /// reproducible (default: a random one).
    pub db_id: Option<[u8; 16]>,
}

/// Sizes the engine accepts in a commit (`docs/file-format/v1.md` §1.4).
/// They are API limits, tighter than the ones the format allows, and are
/// checked before anything is written: a commit over a limit fails with
/// [`Error::InvalidArgument`] and leaves the handle as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Longest key, and longest bound of a range (default 512 B; format
    /// limit 16 KiB).
    pub max_key_len: usize,
    /// Longest value (default 1 MiB; format limit 64 MiB).
    pub max_value_len: usize,
    /// Most bytes a commit encodes to, in its WAL form (default 64 MiB; a
    /// frame holds up to 1 GiB).
    pub max_batch_bytes: usize,
    /// Most operations in one batch (default 2^20; format limit 2^24).
    pub max_batch_ops: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_key_len: 512, max_value_len: 1 << 20, max_batch_bytes: 64 << 20, max_batch_ops: 1 << 20 }
    }
}

impl Limits {
    fn validate(&self) -> Result<()> {
        let check = |name: &str, value: usize, max: u64| {
            if value == 0 || value as u64 > max {
                return Err(Error::InvalidArgument(format!("limits: {name} must be in 1..={max}, got {value}")));
            }
            Ok(())
        };
        check("max_key_len", self.max_key_len, limits::MAX_KEY_LEN as u64)?;
        check("max_value_len", self.max_value_len, limits::MAX_VALUE_LEN as u64)?;
        check("max_batch_bytes", self.max_batch_bytes, u64::from(limits::MAX_FRAME_PAYLOAD))?;
        check("max_batch_ops", self.max_batch_ops, limits::MAX_COMMIT_OPS)
    }
}

/// Group commit (phase1-design.md §5.6): concurrent commits are written as
/// one WAL frame with one `fsync`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupCommitConfig {
    /// `false` writes one frame per commit (the single-commit path; the
    /// `kerno-serial` test profile). Commits still go through the queue, one
    /// at a time.
    pub enabled: bool,
    /// A group stops growing before it would hold more than this many bytes
    /// of encoded commits; a single commit over the limit forms a group of
    /// its own (default 1 MiB; at most the format's frame payload).
    pub max_group_bytes: usize,
    /// Most commits in one group (default 1024; format limit 65 536).
    pub max_group_commits: usize,
    /// A commit waits before queueing while more than this many bytes of
    /// commits are queued or being written (default 64 MiB).
    pub max_queued_bytes: usize,
}

impl Default for GroupCommitConfig {
    fn default() -> Self {
        GroupCommitConfig {
            enabled: true,
            max_group_bytes: 1 << 20,
            max_group_commits: 1024,
            max_queued_bytes: 64 << 20,
        }
    }
}

impl GroupCommitConfig {
    fn validate(&self) -> Result<()> {
        let check = |name: &str, value: usize, max: u64| {
            if value == 0 || value as u64 > max {
                return Err(Error::InvalidArgument(format!("group_commit: {name} must be in 1..={max}, got {value}")));
            }
            Ok(())
        };
        check("max_group_bytes", self.max_group_bytes, u64::from(limits::MAX_FRAME_PAYLOAD))?;
        check("max_group_commits", self.max_group_commits, u64::from(limits::MAX_FRAME_COMMITS))?;
        check("max_queued_bytes", self.max_queued_bytes, u64::MAX)
    }

    /// The queue limits this configuration means: a disabled group commit
    /// is a group of one.
    pub(crate) fn queue_limits(&self) -> QueueLimits {
        QueueLimits {
            max_group_bytes: self.max_group_bytes,
            max_group_commits: if self.enabled { self.max_group_commits } else { 1 },
            max_queued_bytes: self.max_queued_bytes,
        }
    }
}

/// Options of [`KernoDb::open`](crate::KernoDb::open).
#[derive(Clone, Debug)]
pub struct Config {
    /// Read-write (default) or read-only access.
    pub access: AccessMode,
    /// Create the database if `dir` holds none (default `true`). When
    /// `false` and `dir` has no `MANIFEST`, `open` creates and changes
    /// nothing and returns [`Error::Corruption`] if `dir` holds WAL or SST
    /// files (a damaged database, as with `true`), or
    /// [`Error::InvalidArgument`] otherwise (no database).
    pub create_if_missing: bool,
    /// Durability of every commit of this handle (default
    /// [`Durability::Sync`]).
    pub durability: Durability,
    /// Capacity of the block cache in bytes (default 64 MiB).
    pub cache_bytes: usize,
    /// Size of the active memtable that triggers a switch to a new one and a
    /// flush of the old one to L0 (default 8 MiB; at least 4 KiB is used).
    /// Unless [`CompactionConfig`] sets them explicitly, the SST target size
    /// and the L1 target derive from it (the write buffer and 4 x the write
    /// buffer, as in M0).
    pub write_buffer_bytes: usize,
    /// Writers wait while this many memtables are waiting to be flushed
    /// (default 4, the M0 value; at least 1).
    pub max_immutable_memtables: usize,
    /// Size limits of a commit (default [`Limits::default`]).
    pub limits: Limits,
    /// Group commit and the commit queue's limits (default
    /// [`GroupCommitConfig::default`]).
    pub group_commit: GroupCommitConfig,
    /// Open with [`Durability::Sync`] on a platform where its guarantees
    /// have not been verified (default `false`). Only Linux (ext4, xfs) is
    /// verified; elsewhere such an open fails with
    /// [`Error::InvalidArgument`] unless this is set.
    pub allow_unverified_platform: bool,
    /// Test-only settings (default: background threads).
    #[doc(hidden)]
    pub test: TestConfig,
    /// Compaction policy and write throttling (default
    /// [`CompactionConfig::default`]: the condition-1 levers on, with the
    /// sizes derived from `write_buffer_bytes`, so that a `Config` built by
    /// struct update from `Config::default()` keeps the M0 geometry).
    pub compaction: CompactionConfig,
    /// How much compaction `close` may do (default
    /// [`CloseBudget::default`]).
    pub close_budget: CloseBudget,
}

/// How much compaction work a clean close may do (phase1-design.md §5.4,
/// C-10). Whatever it leaves is picked up by the scoring after the next
/// open: the format records no compaction debt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseBudget {
    /// Compact until no level is over its target, however long it takes
    /// (the M0 behaviour).
    Unbounded,
    /// First, always, compact until the L0 metric is below `l0_trigger`; then
    /// one more compaction at a time while both limits hold. A compaction
    /// that starts under the byte budget may exceed it by up to
    /// [`CompactionConfig::max_compaction_bytes`]; one that would end past
    /// `max_duration` at the rate of the close's compactions so far does not
    /// start.
    Bounded {
        /// Bytes read and written by the compactions of one close, the
        /// mandatory ones included.
        max_bytes: u64,
        /// Wall-clock time of the compactions of one close. `None` makes the
        /// close deterministic (tests, crash sweeps, vectors).
        max_duration: Option<Duration>,
    },
}

impl Default for CloseBudget {
    /// 256 MiB and 200 ms.
    fn default() -> Self {
        CloseBudget::Bounded { max_bytes: 256 << 20, max_duration: Some(Duration::from_millis(200)) }
    }
}

impl CloseBudget {
    /// The same budget without the time limit, so that the sequence of file
    /// operations of a close depends only on the calls made.
    pub fn deterministic(self) -> Self {
        match self {
            CloseBudget::Bounded { max_bytes, .. } => CloseBudget::Bounded { max_bytes, max_duration: None },
            CloseBudget::Unbounded => CloseBudget::Unbounded,
        }
    }

    /// Whether a compaction of `next_bytes` (read plus written) beyond the
    /// mandatory ones may start after `compacted_bytes` bytes and `elapsed`
    /// time, `compacting` of it in compactions. The time limit counts the
    /// next compaction at the rate of the ones before it (as instant if there
    /// were none), so that the close does not overrun it by a whole
    /// compaction.
    pub(crate) fn allows(
        &self,
        compacted_bytes: u64,
        elapsed: Duration,
        compacting: Duration,
        next_bytes: u64,
    ) -> bool {
        match *self {
            CloseBudget::Unbounded => true,
            CloseBudget::Bounded { max_bytes, max_duration } => {
                let next = if compacted_bytes == 0 {
                    Duration::ZERO
                } else {
                    let secs = compacting.as_secs_f64() * next_bytes as f64 / compacted_bytes as f64;
                    Duration::try_from_secs_f64(secs).unwrap_or(Duration::MAX)
                };
                compacted_bytes < max_bytes && max_duration.is_none_or(|d| elapsed.saturating_add(next) < d)
            }
        }
    }
}

/// What the L0 limits (`l0_trigger`, `l0_slowdown`, `l0_stop`) count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum L0Metric {
    /// The number of L0 files (M0).
    Files,
    /// The largest number of L0 files whose key ranges contain one same key
    /// (`compaction::l0_depth`). Files of one flush that were split around
    /// L1 files do not overlap, so they count once. As a backstop against an
    /// L0 of many disjoint files (sequential keys), every
    /// [`L0_FILES_PER_DEPTH`](crate::compaction::L0_FILES_PER_DEPTH) files
    /// count as one level of depth at least.
    Depth,
}

/// How a commit is delayed while L0 has `l0_slowdown` files or more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlowdownPolicy {
    /// No delay: writers only wait at the hard limits (`l0_stop`,
    /// `max_immutable_memtables`).
    Off,
    /// Each commit waits once, up to `micros`, for a compaction to finish
    /// (M0: 1000).
    Sleep {
        /// Longest wait of one commit.
        micros: u64,
    },
    /// Each commit waits for as long as its batch needs at the current
    /// rate: `batch bytes / rate`, at most 100 ms. The rate starts at
    /// `initial_bytes_per_sec`, halves (down to 1 MiB/s) at each commit that
    /// finds the L0 metric grown while at or above `l0_slowdown`, and doubles
    /// back up to the initial rate at each commit that finds it fallen or
    /// below the limit.
    Rate {
        /// Rate of the first delayed commit (default 16 MiB/s).
        initial_bytes_per_sec: u64,
    },
}

/// Compaction policy and write throttling (phase1-design.md §5.4).
/// [`CompactionConfig::m0`] reproduces the M0 engine, whose constants these
/// fields replace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionConfig {
    /// L0 is compacted into L1 once it has this many files (M0: 4).
    pub l0_trigger: usize,
    /// Commits are delayed by [`slowdown`](Self::slowdown) while L0 has this
    /// many files (M0: 8).
    pub l0_slowdown: usize,
    /// Commits that need a new memtable wait while L0 has this many files
    /// (M0: 12; at least `l0_trigger`).
    pub l0_stop: usize,
    /// Each level below L1 may hold this many times the bytes of the level
    /// above (M0: 10).
    pub level_multiplier: u64,
    /// Target size of L1 in bytes (`None`: 4 x the effective write buffer,
    /// the M0 value).
    pub level_base_bytes: Option<u64>,
    /// Compaction outputs are cut at this size (`None`: the effective write
    /// buffer, the M0 value).
    pub target_file_size: Option<u64>,
    /// Delay of commits while L0 is at or above `l0_slowdown` (M0: sleep up
    /// to 1 ms).
    pub slowdown: SlowdownPolicy,
    /// SST writers call `sync_data` every this many bytes, in addition to the
    /// final one (M0: `None`, only the final one).
    pub bytes_per_sync: Option<u64>,
    /// Writing a memtable cuts the output between two consecutive keys
    /// whenever an L1 file lies entirely between them (M0: off).
    pub flush_split: bool,
    /// What the L0 limits count (M0: [`L0Metric::Files`]).
    pub l0_metric: L0Metric,
    /// With the L0 metric at or above `l0_slowdown`, L0 is compacted before
    /// any deeper level (M0: off).
    pub l0_priority: bool,
    /// An L0 compaction takes the oldest suffix of L0 that fits
    /// `max_compaction_bytes` instead of all of it (M0: off).
    pub l0_partial: bool,
    /// Input bytes (L0 plus the L1 files they overlap) of a partial L0
    /// compaction (`None`: 25 x the target file size).
    pub max_compaction_bytes: Option<u64>,
    /// A compaction output is cut once it overlaps this many bytes of the
    /// level below the output level (`None`: 10 x the target file size;
    /// `Some(u64::MAX)`: never, the M0 behaviour).
    pub grandparent_overlap_bytes: Option<u64>,
    /// The oldest L0 file moves to L1 without being rewritten when it
    /// overlaps no L1 file (M0: off).
    pub l0_trivial_move: bool,
    /// Merges of more than this many children use a binary heap instead of a
    /// linear scan (M0: never). Flush and compaction merges only; scans
    /// always use the default.
    pub heap_merge_threshold: usize,
    /// Compaction threads, `kerno-compact-<i>` (M0: 1). Compactions that
    /// read the same tables, or write overlapping key ranges to the same
    /// level, never run at once. Not used by [`Background::Inline`].
    pub max_background_compactions: usize,
    /// An L0 compaction merges up to this many disjoint key ranges at once,
    /// cut at L1 file boundaries, and installs their outputs together (M0:
    /// 1, no split).
    pub max_subcompactions: usize,
    /// `max_subcompactions` for the compactions of a clean close, which run
    /// with nothing else competing for the disk and whose mandatory L0 floor
    /// dominates the close of a random-order load (M0: 1). Ignored by
    /// [`Background::Inline`], whose close uses `max_subcompactions`.
    pub close_subcompactions: usize,
}

impl CompactionConfig {
    /// The M0 engine for a write buffer of `write_buffer_bytes` (raised to
    /// the 4 KiB minimum, as the engine does): every condition-1 lever off.
    pub fn m0(write_buffer_bytes: usize) -> Self {
        let write_buffer = write_buffer_bytes.max(MIN_WRITE_BUFFER) as u64;
        CompactionConfig {
            l0_trigger: L0_COMPACTION_TRIGGER,
            l0_slowdown: L0_SLOWDOWN,
            l0_stop: L0_STOP,
            level_multiplier: LEVEL_SIZE_RATIO,
            level_base_bytes: Some((L0_COMPACTION_TRIGGER as u64).saturating_mul(write_buffer)),
            target_file_size: Some(write_buffer),
            slowdown: SlowdownPolicy::Sleep { micros: 1000 },
            bytes_per_sync: None,
            flush_split: false,
            l0_metric: L0Metric::Files,
            l0_priority: false,
            l0_partial: false,
            max_compaction_bytes: None,
            grandparent_overlap_bytes: Some(u64::MAX),
            l0_trivial_move: false,
            heap_merge_threshold: usize::MAX,
            max_background_compactions: 1,
            max_subcompactions: 1,
            close_subcompactions: 1,
        }
    }

    fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::InvalidArgument(format!("compaction: {m}")));
        if self.l0_trigger == 0 || self.l0_slowdown == 0 {
            return bad("l0_trigger and l0_slowdown must be at least 1");
        }
        if self.l0_stop < self.l0_trigger {
            // Writers would wait for a compaction that never starts.
            return bad("l0_stop must be at least l0_trigger");
        }
        if self.level_multiplier == 0 || self.level_base_bytes == Some(0) || self.target_file_size == Some(0) {
            return bad("level_multiplier, level_base_bytes and target_file_size must be at least 1");
        }
        if self.bytes_per_sync == Some(0) {
            return bad("bytes_per_sync must be at least 1");
        }
        if self.max_compaction_bytes == Some(0) || self.grandparent_overlap_bytes == Some(0) {
            return bad("max_compaction_bytes and grandparent_overlap_bytes must be at least 1");
        }
        if self.slowdown == (SlowdownPolicy::Rate { initial_bytes_per_sec: 0 }) {
            return bad("the initial slowdown rate must be at least 1 byte per second");
        }
        if self.heap_merge_threshold == 0 {
            return bad("heap_merge_threshold must be at least 1");
        }
        if self.max_background_compactions == 0 || self.max_subcompactions == 0 || self.close_subcompactions == 0 {
            return bad("max_background_compactions, max_subcompactions and close_subcompactions must be at least 1");
        }
        Ok(())
    }
}

impl Default for CompactionConfig {
    /// The M0 thresholds with the condition-1 levers on (see the field
    /// docs), and `level_base_bytes`, `target_file_size`,
    /// `max_compaction_bytes` and `grandparent_overlap_bytes` left to derive
    /// from the write buffer of the `Config` (as in M0).
    fn default() -> Self {
        CompactionConfig {
            level_base_bytes: None,
            target_file_size: None,
            slowdown: SlowdownPolicy::Rate { initial_bytes_per_sec: 16 << 20 },
            bytes_per_sync: Some(1 << 20),
            flush_split: true,
            l0_metric: L0Metric::Depth,
            l0_priority: false,
            l0_partial: true,
            max_compaction_bytes: None,
            grandparent_overlap_bytes: None,
            l0_trivial_move: true,
            heap_merge_threshold: 8,
            close_subcompactions: 8,
            ..CompactionConfig::m0(MIN_WRITE_BUFFER)
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            access: AccessMode::ReadWrite,
            create_if_missing: true,
            durability: Durability::Sync,
            cache_bytes: 64 << 20,
            write_buffer_bytes: 8 << 20,
            max_immutable_memtables: MAX_IMMUTABLE,
            limits: Limits::default(),
            group_commit: GroupCommitConfig::default(),
            allow_unverified_platform: false,
            test: TestConfig::default(),
            compaction: CompactionConfig::default(),
            close_budget: CloseBudget::default(),
        }
    }
}

impl From<&Options> for Config {
    /// Durability, cache and write buffer come from `opts`, and the
    /// compaction sizes derive from that write buffer as in the M0 engine;
    /// every other field keeps its default.
    fn from(opts: &Options) -> Self {
        Config {
            durability: opts.durability,
            cache_bytes: opts.cache_bytes,
            write_buffer_bytes: opts.write_buffer_bytes,
            ..Config::default()
        }
    }
}

impl Config {
    /// Rejects values the engine cannot run with.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_immutable_memtables == 0 {
            return Err(Error::InvalidArgument("max_immutable_memtables must be at least 1".into()));
        }
        if self.durability == Durability::Sync && !cfg!(target_os = "linux") && !self.allow_unverified_platform {
            return Err(Error::InvalidArgument("durability not verified on this platform".into()));
        }
        self.limits.validate()?;
        self.group_commit.validate()?;
        self.compaction.validate()
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::io;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::KernoDb;
    use kerno_core::WriteBatch;
    use kerno_core::contract::{Engine, Snapshot};
    use kerno_core::vfs::{File, OpenMode, Vfs};
    use kerno_testkit::vfs::SimVfs;
    use parking_lot::{Condvar, Mutex};

    #[test]
    fn default_config_matches_m0_options() {
        let c = Config::default();
        let o = Options::default();
        assert_eq!(c.access, AccessMode::ReadWrite);
        assert!(c.create_if_missing);
        assert_eq!(c.durability, o.durability);
        assert_eq!(c.cache_bytes, o.cache_bytes);
        assert_eq!(c.write_buffer_bytes, o.write_buffer_bytes);
        assert_eq!(c.max_immutable_memtables, 4);
        assert_eq!(c.compaction, CompactionConfig::default());
        assert_eq!((c.compaction.level_base_bytes, c.compaction.target_file_size), (None, None));
    }

    #[test]
    fn m0_compaction_config_has_the_m0_constants() {
        let c = CompactionConfig::m0(8 << 20);
        assert_eq!((c.l0_trigger, c.l0_slowdown, c.l0_stop, c.level_multiplier), (4, 8, 12, 10));
        assert_eq!((c.level_base_bytes, c.target_file_size), (Some(32 << 20), Some(8 << 20)));
        assert_eq!(c.slowdown, SlowdownPolicy::Sleep { micros: 1000 });
        assert_eq!(c.bytes_per_sync, None);
        // The engine raises tiny write buffers to 4 KiB, and so does m0.
        assert_eq!(CompactionConfig::m0(100).target_file_size, Some(4 << 10));
        // Every condition-1 lever is off.
        assert!(!(c.flush_split || c.l0_priority || c.l0_partial || c.l0_trivial_move));
        assert_eq!(c.l0_metric, L0Metric::Files);
        assert_eq!((c.max_compaction_bytes, c.grandparent_overlap_bytes), (None, Some(u64::MAX)));
        assert_eq!((c.heap_merge_threshold, c.max_background_compactions, c.max_subcompactions), (usize::MAX, 1, 1));
    }

    #[test]
    fn default_compaction_config_turns_the_levers_on() {
        let c = CompactionConfig::default();
        assert!(c.flush_split && c.l0_partial && c.l0_trivial_move);
        assert!(!c.l0_priority, "WP04 rejected the hypothesis behind L0 priority");
        assert_eq!(c.l0_metric, L0Metric::Depth);
        assert_eq!(c.slowdown, SlowdownPolicy::Rate { initial_bytes_per_sec: 16 << 20 });
        assert_eq!((c.bytes_per_sync, c.heap_merge_threshold), (Some(1 << 20), 8));
        assert_eq!((c.max_compaction_bytes, c.grandparent_overlap_bytes), (None, None));
        assert_eq!(Config::default().close_budget, CloseBudget::default());
        assert_eq!(
            CloseBudget::default(),
            CloseBudget::Bounded { max_bytes: 256 << 20, max_duration: Some(Duration::from_millis(200)) }
        );
    }

    #[test]
    fn close_budget_allows_work_until_a_limit_is_reached() {
        let ms = Duration::from_millis;
        let b = CloseBudget::Bounded { max_bytes: 100, max_duration: Some(ms(10)) };
        assert!(b.allows(99, ms(9), Duration::ZERO, 0));
        assert!(!b.allows(100, Duration::ZERO, Duration::ZERO, 0));
        assert!(!b.allows(0, ms(10), Duration::ZERO, 0));
        // 40 bytes took 4 ms: 20 more are expected to take 2 ms.
        assert!(b.allows(40, ms(7), ms(4), 20));
        assert!(!b.allows(40, ms(8), ms(4), 20));
        // Nothing compacted yet: no rate to go by.
        assert!(b.allows(0, ms(9), Duration::ZERO, 1 << 40));
        assert!(!b.allows(1, ms(1), Duration::MAX, u64::MAX));
        assert!(b.deterministic().allows(99, Duration::from_secs(60), Duration::MAX, u64::MAX));
        assert!(CloseBudget::Unbounded.allows(u64::MAX, Duration::MAX, Duration::MAX, u64::MAX));
    }

    #[test]
    fn invalid_compaction_config_is_rejected() {
        let base = CompactionConfig::m0(8 << 20);
        for bad in [
            CompactionConfig { l0_trigger: 0, ..base.clone() },
            CompactionConfig { l0_slowdown: 0, ..base.clone() },
            CompactionConfig { l0_stop: 3, ..base.clone() },
            CompactionConfig { level_multiplier: 0, ..base.clone() },
            CompactionConfig { level_base_bytes: Some(0), ..base.clone() },
            CompactionConfig { target_file_size: Some(0), ..base.clone() },
            CompactionConfig { bytes_per_sync: Some(0), ..base.clone() },
            CompactionConfig { max_compaction_bytes: Some(0), ..base.clone() },
            CompactionConfig { grandparent_overlap_bytes: Some(0), ..base.clone() },
            CompactionConfig { slowdown: SlowdownPolicy::Rate { initial_bytes_per_sec: 0 }, ..base.clone() },
            CompactionConfig { heap_merge_threshold: 0, ..base.clone() },
            CompactionConfig { max_background_compactions: 0, ..base.clone() },
            CompactionConfig { max_subcompactions: 0, ..base.clone() },
        ] {
            let sim = SimVfs::new(1);
            let cfg = Config { compaction: bad.clone(), ..Config::default() };
            assert!(
                matches!(KernoDb::open(sim.as_vfs(), Path::new("/db"), &cfg), Err(Error::InvalidArgument(_))),
                "{bad:?} was accepted"
            );
            assert_eq!(sim.mutating_ops(), 0);
        }
    }

    /// The engine's read counters are sharded: lookups from several threads
    /// land in several slots of every read counter of the real `Stats`.
    #[test]
    fn engine_read_counters_use_several_slots() {
        let sim = SimVfs::new(9);
        let cfg = Config::from(&Options::small_for_tests());
        let db = KernoDb::open(sim.as_vfs(), Path::new("/db"), &cfg).unwrap();
        for chunk in 0..20u32 {
            let mut b = WriteBatch::new();
            for i in 0..50 {
                b.put(format!("key-{:06}", chunk * 50 + i), [5u8; 200]);
            }
            db.commit(&b).unwrap();
        }
        db.close().unwrap();
        let db = KernoDb::open(sim.as_vfs(), Path::new("/db"), &Config { cache_bytes: 0, ..cfg }).unwrap();
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    let snap = db.snapshot().unwrap();
                    for i in 0..200u32 {
                        snap.get(format!("key-{:06}", i * 5).as_bytes()).unwrap();
                        snap.get(format!("key-{:06}-absent", i * 5).as_bytes()).unwrap();
                    }
                });
            }
        });
        let st = db.raw_stats();
        for (name, c) in [
            ("bloom_checks", &st.bloom_checks),
            ("bloom_negatives", &st.bloom_negatives),
            ("block_cache_misses", &st.block_cache_misses),
        ] {
            assert!(c.slots_used() > 1, "{name} uses {} slot(s)", c.slots_used());
        }
        db.close().unwrap();
    }

    #[test]
    fn default_test_config_uses_background_threads() {
        let c = Config::default();
        assert_eq!(c.test.background, Background::Threads);
        assert_eq!(c.test.db_id, None);
        let c = Config::from(&Options::small_for_tests());
        assert_eq!(c.test, TestConfig::default());
    }

    #[test]
    fn config_from_options_takes_engine_fields() {
        let o = Options { durability: Durability::Buffered, cache_bytes: 123, write_buffer_bytes: 4567 };
        let c = Config::from(&o);
        assert_eq!(c.durability, Durability::Buffered);
        assert_eq!(c.cache_bytes, 123);
        assert_eq!(c.write_buffer_bytes, 4567);
        assert_eq!(c.access, AccessMode::ReadWrite);
        assert!(c.create_if_missing);
        assert_eq!(c.max_immutable_memtables, 4);
        assert_eq!(c.compaction, CompactionConfig::default());
    }

    #[test]
    fn read_only_access_never_creates_a_database() {
        let sim = SimVfs::new(1);
        let cfg = Config { access: AccessMode::ReadOnly, ..Config::default() };
        match KernoDb::open(sim.as_vfs(), Path::new("/db"), &cfg) {
            Err(Error::InvalidArgument(m)) => assert!(m.contains("no database"), "{m}"),
            other => panic!("expected InvalidArgument, got {:?}", other.err()),
        }
        assert_eq!(sim.mutating_ops(), 0);
        assert!(!sim.as_vfs().exists(Path::new("/db")).unwrap());
    }

    #[test]
    fn zero_immutable_memtables_is_rejected() {
        let sim = SimVfs::new(1);
        let cfg = Config { max_immutable_memtables: 0, ..Config::default() };
        assert!(matches!(KernoDb::open(sim.as_vfs(), Path::new("/db"), &cfg), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), 0);
    }

    #[test]
    fn create_if_missing_false_touches_nothing() {
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        let cfg = Config { create_if_missing: false, ..Config::default() };
        // Missing directory.
        assert!(matches!(KernoDb::open(vfs.clone(), Path::new("/db"), &cfg), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), 0);
        assert!(!vfs.exists(Path::new("/db")).unwrap());
        // Existing directory without a database: still untouched.
        vfs.create_dir_all(Path::new("/db")).unwrap();
        let before = sim.mutating_ops();
        assert!(matches!(KernoDb::open(vfs.clone(), Path::new("/db"), &cfg), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), before);
        assert!(vfs.list(Path::new("/db")).unwrap().is_empty());
    }

    #[test]
    fn create_if_missing_false_opens_an_existing_database() {
        let vfs = SimVfs::new(1).as_vfs();
        let dir = Path::new("/db");
        let db = KernoDb::open(vfs.clone(), dir, &Config::default()).unwrap();
        let mut b = WriteBatch::new();
        b.put("k", "v");
        db.commit(&b).unwrap();
        db.close().unwrap();
        let cfg = Config { create_if_missing: false, ..Config::default() };
        let db = KernoDb::open(vfs, dir, &cfg).unwrap();
        assert_eq!(db.snapshot().unwrap().get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        db.close().unwrap();
    }

    #[test]
    fn create_if_missing_false_reports_damage_as_corruption() {
        let dir = Path::new("/db");
        let no_create = Config { create_if_missing: false, ..Config::default() };
        // WAL or SST files without a MANIFEST are a damaged database, not a
        // missing one: the error must not depend on create_if_missing, and
        // the directory stays untouched.
        let check = |sim: &SimVfs| {
            let before = sim.mutating_ops();
            let msg = match KernoDb::open(sim.as_vfs(), dir, &no_create) {
                Err(Error::Corruption(m)) => m,
                other => panic!("expected Corruption, got {:?}", other.err()),
            };
            assert_eq!(sim.mutating_ops(), before);
            match KernoDb::open(sim.as_vfs(), dir, &Config::default()) {
                Err(Error::Corruption(m)) => assert_eq!(m, msg),
                other => panic!("expected Corruption, got {:?}", other.err()),
            }
        };

        // A cleanly closed database (LOCK, MANIFEST, SSTs) without its MANIFEST.
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        let db = KernoDb::open(vfs.clone(), dir, &Config::default()).unwrap();
        let mut b = WriteBatch::new();
        b.put("k", "v");
        db.commit(&b).unwrap();
        db.close().unwrap();
        assert!(vfs.list(dir).unwrap().iter().any(|n| n.ends_with(".sst")));
        vfs.remove_file(&dir.join("MANIFEST")).unwrap();
        vfs.sync_dir(dir).unwrap();
        check(&sim);

        // Nothing but a canonical WAL.
        let sim = SimVfs::new(2);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(dir).unwrap();
        vfs.open(&dir.join("000005.wal"), OpenMode::CreateNew).unwrap().write_at(b"junk", 0).unwrap();
        check(&sim);

        // Names that are not canonical WAL or SST names are not data files.
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(dir).unwrap();
        for name in ["1.wal", "5.sst", "notes.txt"] {
            vfs.open(&dir.join(name), OpenMode::CreateNew).unwrap();
        }
        let before = sim.mutating_ops();
        assert!(matches!(KernoDb::open(vfs, dir, &no_create), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), before);
    }

    /// Holds every SST creation until [`SstGate::release`], so that no flush
    /// can finish.
    struct SstGate {
        inner: Arc<dyn Vfs>,
        state: Mutex<GateState>,
        cv: Condvar,
    }

    #[derive(Default)]
    struct GateState {
        open: bool,
        held: usize,
    }

    impl SstGate {
        fn held(&self) -> usize {
            self.state.lock().held
        }

        fn release(&self) {
            self.state.lock().open = true;
            self.cv.notify_all();
        }
    }

    impl Vfs for SstGate {
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.inner.create_dir_all(path)
        }
        fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
            if mode == OpenMode::CreateNew && path.extension().is_some_and(|e| e == "sst") {
                let mut s = self.state.lock();
                s.held += 1;
                while !s.open {
                    self.cv.wait(&mut s);
                }
            }
            self.inner.open(path, mode)
        }
        fn exists(&self, path: &Path) -> io::Result<bool> {
            self.inner.exists(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.inner.remove_file(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.inner.rename(from, to)
        }
        fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
            self.inner.list(dir)
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            self.inner.sync_dir(dir)
        }
        fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
            self.inner.lock(path)
        }
    }

    /// Opens the gate when dropped, also when an assertion fails, so that the
    /// threads blocked behind it finish and the test does not hang.
    struct ReleaseOnDrop<'a>(&'a SstGate);

    impl Drop for ReleaseOnDrop<'_> {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    fn stat(db: &KernoDb, name: &str) -> u64 {
        db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap()
    }

    #[test]
    fn max_immutable_memtables_bounds_pending_flushes() {
        let gate = Arc::new(SstGate { inner: SimVfs::new(1).as_vfs(), state: Mutex::default(), cv: Condvar::new() });
        let cfg = Config { max_immutable_memtables: 1, write_buffer_bytes: 4 << 10, ..Config::default() };
        let db = KernoDb::open(gate.clone(), Path::new("/db"), &cfg).unwrap();
        // Every commit fills a memtable. Commit 1 fills the first one.
        // Commit 2 switches to a second one (the first waits for its flush,
        // held at the gate) and fills it. Commit 3 then finds one memtable
        // pending, the limit, and must wait for that flush; with the M0
        // limit of 4 it would switch again and return.
        let value = vec![7u8; 5 << 10];
        let done = AtomicUsize::new(0);
        std::thread::scope(|s| {
            let _release = ReleaseOnDrop(&gate);
            s.spawn(|| {
                for i in 0..3u8 {
                    let mut b = WriteBatch::new();
                    b.put([i], value.as_slice());
                    db.commit(&b).unwrap();
                    done.fetch_add(1, Ordering::SeqCst);
                }
            });
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                let n = done.load(Ordering::SeqCst);
                assert!(n <= 2, "commit {n} returned while a flush was held and a memtable was pending");
                assert!(stat(&db, "immutable_memtables") <= 1);
                if n == 2 && gate.held() == 1 && stat(&db, "write_stalls") > 0 {
                    break;
                }
                // A stall episode is counted when the writer starts waiting;
                // spurious wake-ups must not end it.
                db.wake_waiters();
                assert!(Instant::now() < deadline, "the writer never stalled");
                std::thread::sleep(Duration::from_millis(1));
            }
            // Dropping `_release` lets the flush finish and commit 3 return.
        });
        assert_eq!(done.load(Ordering::SeqCst), 3);
        // One episode, however many times the writer was woken.
        assert_eq!(stat(&db, "write_stalls"), 1);
        assert_eq!(stat(&db, "stall_episodes_memtable"), 1);
        assert_eq!(stat(&db, "stall_episodes_l0"), 0);
        let snap = db.snapshot().unwrap();
        for i in 0..3u8 {
            assert_eq!(snap.get(&[i]).unwrap().as_deref(), Some(value.as_slice()));
        }
        drop(snap);
        db.close().unwrap();
    }
}
