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

use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::vfs::Vfs;

/// Upper bound for key length accepted by every candidate.
pub const MAX_KEY_LEN: usize = 512;
/// Upper bound for value length accepted by every candidate.
pub const MAX_VALUE_LEN: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Put { key: Vec<u8>, value: Vec<u8> },
    Delete { key: Vec<u8> },
}

impl Op {
    pub fn key(&self) -> &[u8] {
        match self {
            Op::Put { key, .. } | Op::Delete { key } => key,
        }
    }
}

/// An atomic group of writes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteBatch {
    ops: Vec<Op>,
}

impl WriteBatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Put { key: key.into(), value: value.into() });
        self
    }

    pub fn delete(&mut self, key: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Delete { key: key.into() });
        self
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Sum of key and value bytes ("logical bytes" for amplification math).
    pub fn payload_bytes(&self) -> usize {
        self.ops
            .iter()
            .map(|op| match op {
                Op::Put { key, value } => key.len() + value.len(),
                Op::Delete { key } => key.len(),
            })
            .sum()
    }

    /// Checks the size limits shared by every candidate.
    pub fn validate(&self) -> Result<()> {
        for op in &self.ops {
            if op.key().len() > MAX_KEY_LEN {
                return Err(Error::InvalidArgument(format!(
                    "key of {} bytes exceeds MAX_KEY_LEN ({MAX_KEY_LEN})",
                    op.key().len()
                )));
            }
            if let Op::Put { value, .. } = op
                && value.len() > MAX_VALUE_LEN
            {
                return Err(Error::InvalidArgument(format!(
                    "value of {} bytes exceeds MAX_VALUE_LEN ({MAX_VALUE_LEN})",
                    value.len()
                )));
            }
        }
        Ok(())
    }
}

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
