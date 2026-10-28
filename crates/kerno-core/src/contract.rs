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
//! * **Range deletions** — [`Op::DeleteRange`] deletes `[start, end)` and
//!   follows the same ordering: it deletes the keys put earlier in its
//!   batch, and a key put later in the batch survives it. An engine either
//!   implements it ([`RangeDeleteEngine`]) or rejects a batch that contains
//!   one with [`Error::InvalidArgument`] before writing anything; it never
//!   ignores it. A batch whose only operations are empty ranges
//!   (`start == end`) is still a non-empty commit.
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
//! * **Transactions** — an engine may offer read-modify-write transactions
//!   ([`TxnEngine`]) on top of the single writer.
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
//! [`Error::InvalidArgument`]: crate::Error::InvalidArgument
//! [`Op::DeleteRange`]: crate::Op::DeleteRange

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

/// Marker for engines that implement [`Op::DeleteRange`](crate::Op::DeleteRange)
/// themselves, with the semantics in the [module documentation](crate::contract):
/// `[start, end)`, applied in batch order, visible exactly from the commit
/// that contains it. The range deletion suite
/// (`kerno_testkit::range_delete_test_suite!`) requires it; an engine
/// without native support runs that suite through `kerno_testkit`'s
/// `Emulated<E>`, which turns each range into point deletions.
pub trait RangeDeleteEngine: Engine {}

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

/// An engine with read-modify-write transactions (ADR-0002, Phase 1): an
/// exclusive write transaction that reads its own writes and commits them as
/// one batch.
///
/// A separate trait, so that engines without it (the frozen spikes) need no
/// change; `kerno_testkit::emulated::Emulated` gives any [`Engine`] this
/// interface. The shared suite `txn_test_suite!` of `kerno-testkit` checks
/// these semantics.
///
/// # Semantics
///
/// * **Serializable by exclusion** — [`begin_write`](Self::begin_write)
///   returns the engine's one write slot, and the transaction keeps it until
///   it commits, aborts or is dropped. Meanwhile every other
///   `begin_write` **and every blind [`Engine::commit`]** blocks (a long
///   transaction stalls all writers by design), so nothing commits between
///   the reads of a transaction and its own commit: a read-modify-write can
///   never lose an update. Readers are not blocked: [`Engine::snapshot`]
///   works at any time and never sees an open transaction's writes. Calling
///   `begin_write` or `commit` on the thread that holds a transaction
///   deadlocks.
/// * **Reads** — a transaction reads the latest committed state as of
///   `begin_write` (nobody else can commit afterwards) overlaid with its own
///   writes, in order: a later write to a key wins over an earlier one, a
///   `delete_range` hides the keys written before it, and a key written after
///   it is visible again. `scan` follows the rules of [`Snapshot::scan`].
/// * **Writes** — `put`, `delete` and `delete_range` follow the batch rules
///   of [`WriteBatch`] and check their limits at once: an oversized key or
///   value, or a range with `start > end`, returns
///   [`Error::InvalidArgument`], has no effect and leaves the transaction
///   usable.
/// * **Commit** — [`Txn::commit`] applies the writes that succeeded, in
///   order, as one atomic batch with the next commit sequence, and returns
///   that sequence. A transaction without any successful write returns the
///   current sequence and consumes none, exactly like an empty batch; one
///   whose only writes are empty ranges is a non-empty batch and does
///   consume one. Durability, atomicity, failed-`fsync` poisoning and crash
///   behaviour are those of [`Engine::commit`].
/// * **Abort** — [`Txn::abort`] and dropping the transaction (also while a
///   panic unwinds) write nothing, consume no sequence and release the write
///   slot.
///
/// [`Error::InvalidArgument`]: crate::Error::InvalidArgument
pub trait TxnEngine: Engine {
    type Txn<'a>: Txn
    where
        Self: 'a;

    /// Waits for the write slot and starts a transaction on the latest
    /// committed state (see the [trait documentation](TxnEngine)).
    fn begin_write(&self) -> Result<Self::Txn<'_>>;
}

/// A write transaction (see [`TxnEngine`] for the semantics).
pub trait Txn: Sized {
    /// The value of `key` as the transaction sees it.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;

    /// Up to `limit` entries with keys in `(start, end)` as the transaction
    /// sees them, in ascending key order; paginated like [`Snapshot::scan`].
    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;

    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()>;

    fn delete(&mut self, key: &[u8]) -> Result<()>;

    /// Deletes `[start, end)`; `start == end` deletes nothing.
    fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()>;

    /// Commits the writes and returns their commit sequence.
    fn commit(self) -> Result<u64>;

    /// Discards the writes (same as dropping the transaction).
    fn abort(self) {}
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

    /// Writes what the active write buffer holds and compacts every file
    /// that overlaps `(start, end)` level by level until no deeper level
    /// overlaps it, so that the data and the range deletions in it leave the
    /// live files (directed compaction, ADR-0001 condition 2). Returns when
    /// that is done.
    fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<()> {
        let _ = (start, end);
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
        assert!(is_unsupported(hooks.compact_range(Bound::Included(b"a"), Bound::Excluded(b"b"))));
        assert!(is_unsupported(hooks.compact_range(Bound::Unbounded, Bound::Unbounded)));
        let idle: &dyn TestHooks = &Idle;
        assert!(idle.wait_idle().is_ok());
        assert!(is_unsupported(idle.compact_once()));
        assert!(is_unsupported(idle.compact_range(Bound::Unbounded, Bound::Unbounded)));
    }
}
