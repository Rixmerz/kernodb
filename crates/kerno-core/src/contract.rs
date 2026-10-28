//! The contract both storage candidates implement.
//!
//! The contract is deliberately narrow: an ordered key-value store with
//! atomic write batches, a monotonic commit sequence and point-in-time
//! snapshots. Documents, revisions and indexes are layered on top of this in
//! later phases, so this is the surface whose cost and correctness the spike
//! has to measure.
//!
//! # Semantics every implementation must provide
//!
//! * **Atomicity** — a [`WriteBatch`] is applied entirely or not at all,
//!   including after a crash at any point.
//! * **Ordering** — within a batch, later operations on the same key win.
//! * **Commit sequence** — every non-empty commit gets the next sequence
//!   number (starting at 1). The sequence is persisted: after reopening,
//!   [`Snapshot::seq`] of a fresh snapshot equals the last recovered commit.
//!   An empty batch is a no-op that returns the current sequence.
//! * **Durability** — with [`Durability::Sync`], once `commit` returns `Ok`
//!   the batch survives power loss. With [`Durability::Buffered`] it survives
//!   a process crash; what survives power loss is engine-specific and must be
//!   documented by the engine (it must still be a consistent prefix of
//!   commits or a detected error, never silently damaged data).
//! * **Isolation** — a [`Snapshot`] observes exactly the state after commit
//!   `seq()`, for as long as it lives, regardless of later commits, flushes,
//!   compactions or page reuse. Readers never block the writer for longer
//!   than a short critical section.
//! * **Single writer** — `commit` may be called from several threads; the
//!   engine serializes them. Snapshots can be used concurrently from any
//!   number of threads.
//! * **Exclusive open** — opening a directory that is already open (in this
//!   or another process) fails with [`Error::Locked`].
//! * **Integrity** — damaged data is reported as [`Error::Corruption`] (or an
//!   I/O error), never returned as if it were valid.
//! * **Failed `fsync`** — after a failed sync the engine must not assume any
//!   unsynced write reached the disk; it returns an error for that commit and
//!   becomes [`Error::Poisoned`] until reopened.
//!
//! [`Error::Locked`]: crate::Error::Locked
//! [`Error::Corruption`]: crate::Error::Corruption
//! [`Error::Poisoned`]: crate::Error::Poisoned

use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use crate::batch::WriteBatch;
use crate::error::Result;
use crate::vfs::Vfs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// `commit` returns only after the batch is on stable storage.
    Sync,
    /// `commit` does not wait for stable storage (no `fsync` per commit).
    Buffered,
}

#[derive(Clone, Debug)]
pub struct Options {
    pub durability: Durability,
    /// Memory budget for the engine's cache of pages / blocks.
    pub cache_bytes: usize,
    /// Size of in-memory write buffering before data is reorganized on disk
    /// (memtable size for LSM). Engines that do not buffer ignore it.
    pub write_buffer_bytes: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { durability: Durability::Sync, cache_bytes: 64 << 20, write_buffer_bytes: 8 << 20 }
    }
}

impl Options {
    /// Tiny buffers and caches so that tests hit splits, merges, flushes,
    /// compactions and cache eviction with little data.
    #[doc(hidden)]
    pub fn small_for_tests() -> Self {
        Options { durability: Durability::Sync, cache_bytes: 64 << 10, write_buffer_bytes: 16 << 10 }
    }
}

pub trait Engine: Sized + Send + Sync + 'static {
    type Snapshot: Snapshot;

    /// Short identifier used in reports ("btree", "lsm", ...).
    fn name() -> &'static str;

    /// Opens (creating if needed) the database stored in `dir`, running
    /// crash recovery if the previous handle was not closed cleanly.
    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self>;

    /// Atomically applies `batch` and returns its commit sequence number.
    fn commit(&self, batch: &WriteBatch) -> Result<u64>;

    /// A consistent read view of the latest committed state.
    fn snapshot(&self) -> Result<Self::Snapshot>;

    /// Clean shutdown: everything committed is durable when this returns.
    fn close(self) -> Result<()>;

    /// Engine-specific counters for benchmark reports (tree height, pages,
    /// SST files per level, compaction bytes, ...).
    fn stats(&self) -> Vec<(String, u64)> {
        Vec::new()
    }
}

pub trait Snapshot: Send + 'static {
    /// Commit sequence this snapshot observes.
    fn seq(&self) -> u64;

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Up to `limit` entries with keys in `(start, end)`, in ascending key
    /// order. Pagination: call again with `Bound::Excluded(last_key)`.
    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
}

/// An engine with an explicit durability barrier: `sync()` (ADR-0002,
/// invariant I-17).
///
/// A separate trait, so that engines without it (the frozen spikes) need no
/// change. The shared suite `sync_test_suite!` of `kerno-testkit` checks
/// these semantics.
///
/// # Semantics
///
/// * `sync` returns `Ok(s)`, the *durable sequence*: every commit with a
///   sequence `<= s` survives a power loss from then on, and `s` is at least
///   the sequence of every commit acknowledged before `sync` was called.
///   With no concurrent commits, `s` is exactly the last acknowledged
///   sequence (right after `open`, the recovered one).
/// * With [`Durability::Sync`] every acknowledged commit is durable already,
///   so `sync` may return without any I/O.
/// * A failed `sync` returns the error (an [`Error::Io`](crate::Error::Io))
///   and poisons the handle like a failed commit: until the database is
///   reopened, `commit` and `sync` return
///   [`Error::Poisoned`](crate::Error::Poisoned). Nothing written since the
///   last successful `sync` may be assumed durable (Linux drops the pages of
///   a failed `fsync`): a power loss then recovers a prefix of the commits
///   that reaches at least the last successful `sync`.
/// * On a poisoned handle `sync` returns `Poisoned` without any I/O. An
///   engine with read-only handles refuses `sync` on them.
pub trait SyncEngine: Engine {
    /// Makes every commit acknowledged so far durable and returns the
    /// durable sequence (see the [trait documentation](SyncEngine)).
    fn sync(&self) -> Result<u64>;
}

/// Test-only control over an engine's background work, so that suites can
/// put the engine into a known state (everything flushed, compaction run)
/// instead of waiting on timing.
///
/// Every method has a default body that returns
/// `Err(Error::InvalidArgument("unsupported"))`, so methods added later never
/// break existing implementers; an engine overrides what it supports. The
/// trait stays object safe (`KernoDb::hooks() -> &dyn TestHooks`).
#[doc(hidden)]
pub trait TestHooks {
    /// Blocks until no background work (flush, compaction, deferred
    /// deletion) is running or queued. Returns the background error that
    /// poisoned the handle, if any.
    fn wait_idle(&self) -> Result<()> {
        Err(unsupported())
    }

    /// Rotates the active write buffer if it holds commits and writes every
    /// immutable buffer to disk, returning when that is done.
    fn flush(&self) -> Result<()> {
        Err(unsupported())
    }

    /// Runs at most one compaction. `Ok(true)` if one ran, `Ok(false)` if
    /// none was due.
    fn compact_once(&self) -> Result<bool> {
        Err(unsupported())
    }
}

fn unsupported() -> crate::Error {
    crate::Error::InvalidArgument("unsupported".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    struct NoHooks;
    impl TestHooks for NoHooks {}

    struct Idle;
    impl TestHooks for Idle {
        fn wait_idle(&self) -> Result<()> {
            Ok(())
        }
    }

    fn is_unsupported<T: std::fmt::Debug>(r: Result<T>) -> bool {
        matches!(r, Err(Error::InvalidArgument(m)) if m == "unsupported")
    }

    #[test]
    fn test_hooks_default_to_unsupported() {
        let hooks: &dyn TestHooks = &NoHooks;
        assert!(is_unsupported(hooks.wait_idle()));
        assert!(is_unsupported(hooks.flush()));
        assert!(is_unsupported(hooks.compact_once()));
        let idle: &dyn TestHooks = &Idle;
        assert!(idle.wait_idle().is_ok());
        assert!(is_unsupported(idle.compact_once()));
    }
}
