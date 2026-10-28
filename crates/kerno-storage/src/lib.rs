//! Kerno storage engine: a leveled LSM tree implementing the harness
//! contract (`kerno_core::contract::Engine`) directly.
//!
//! **Development format, never for user data.** This crate starts as a
//! copy of the M0 prototype `spike-lsm` (phase1-design.md §5.1) and still
//! writes the M0 development format, not the on-disk format v1
//! (`docs/file-format/v1.md`). That format carries no compatibility promise:
//! no user database may be created with it (SAD §20.6, ADR-0003). The API is
//! explicitly unstable.
//!
//! * Commits go to a checksummed write-ahead log (one file per memtable)
//!   and then into a lock-free skip-list memtable keyed by
//!   (key, descending sequence), so readers at older snapshots simply ignore
//!   newer entries.
//! * Full memtables are frozen and flushed to L0 SSTs by a background
//!   thread; another thread runs leveled compaction (L0 trigger 4, size
//!   ratio 10). Writers are slowed down or stalled when flushes or L0
//!   compactions fall behind.
//! * The LSM shape (levels, last flushed sequence, live WAL range) lives in
//!   a single checksummed MANIFEST that is replaced atomically.
//! * A snapshot is an `Arc<Version>` plus a sequence number; SST files are
//!   deleted only when no MANIFEST and no live `Version` references them.
//! * Every on-disk byte that influences results is covered by a crc32c.
//!
//! See `DESIGN.md` next to this crate for formats, fsync counts, recovery
//! and limitations.

mod block;
mod bloom;
mod coding;
mod compaction;
mod config;
mod db;
mod iter;
mod manifest;
mod memtable;
mod sst;
mod stats;
mod version;
mod wal;

#[doc(hidden)]
pub mod testing;

pub use config::{AccessMode, Config};
pub use db::{KernoDb, KernoSnapshot};
