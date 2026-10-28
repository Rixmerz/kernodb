//! Kerno storage engine: a leveled LSM tree implementing the harness
//! contract (`kerno_core::contract::Engine`) directly.
//!
//! **Draft format, never for user data.** This crate started as a copy of
//! the M0 prototype `spike-lsm` (phase1-design.md §5.1) and writes and reads
//! only the on-disk format v1 of `docs/file-format/v1.md`, which stays a
//! draft until the end of Phase 1: every file carries the `DRAFT` feature
//! bit, so no database created now can be read by the frozen format
//! (SAD §20.6, ADR-0003). The API is explicitly unstable.
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
//!   a single checksummed MANIFEST that is replaced atomically; each
//!   replacement is a new generation holding one snapshot record.
//! * A snapshot is an `Arc<Version>` plus a sequence number; SST files are
//!   deleted only when no MANIFEST and no live `Version` references them.
//! * Every on-disk byte that influences results is covered by a crc32c.
//!
//! See `DESIGN.md` next to this crate for formats, fsync counts, recovery
//! and limitations.

mod block;
mod compaction;
mod config;
mod db;
mod files;
mod flush;
mod iter;
mod manifest;
mod memtable;
pub mod metrics;
mod sst;
mod stats;
mod version;
mod wal;

/// On-disk format v1 as pure code (codecs, validation rules, vector
/// helpers); not a public API.
#[doc(hidden)]
pub mod format;
#[doc(hidden)]
pub mod testing;

pub use config::{AccessMode, CloseBudget, CompactionConfig, Config, L0Metric, Limits, SlowdownPolicy};
#[doc(hidden)]
pub use config::{Background, TestConfig};
#[doc(hidden)]
pub use db::TableInfo;
pub use db::{CloseReport, KernoDb, KernoSnapshot};
pub use metrics::Metrics;
