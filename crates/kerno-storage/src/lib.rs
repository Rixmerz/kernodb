//! Kerno storage engine: a leveled LSM tree implementing the harness
//! contract (`kerno_core::contract::Engine`) directly.
//!
//! **Frozen format, unstable API.** This crate started as a copy of the M0
//! prototype `spike-lsm` (phase1-design.md §5.1) and writes and reads only
//! the on-disk format v1 of `docs/file-format/v1.md`, frozen by ADR-0013: the
//! `DRAFT` feature bit is retired and a database made by the draft is
//! refused as `Incompatible("pre-freeze draft format")`. The API is
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
//!   a checksummed MANIFEST that is an edit log: a snapshot record followed
//!   by one appended record per flush, compaction or session start, replaced
//!   by a new snapshot generation at clean close and when it grows.
//! * A snapshot is an `Arc<Version>` plus a sequence number; SST files are
//!   deleted only when no MANIFEST and no live `Version` references them.
//! * Every on-disk byte that influences results is covered by a crc32c.
//!
//! * [`KernoDb::begin_write`] starts a read-modify-write transaction that
//!   reads its own writes and holds the engine's one write slot until it
//!   commits or ends, so it is serializable; snapshots are registered and
//!   the engine reports how many are alive and what they keep on disk.
//! * [`verify`] checks a whole database and [`inspect`] reads the headers of
//!   its files; neither writes (v1.md §9.5).
//!
//! See `DESIGN.md` next to this crate for formats, fsync counts, recovery
//! and limitations.

mod block;
mod commit_queue;
mod compaction;
mod config;
mod db;
mod deleter;
mod files;
mod flush;
mod inspect;
mod iter;
mod manifest;
mod memtable;
pub mod metrics;
mod rangedel;
mod recovery;
mod snapshots;
mod sst;
mod stats;
mod sync;
mod txn;
mod verify;
mod version;
mod wal;

/// On-disk format v1 as pure code (codecs, validation rules, vector
/// helpers); not a public API.
#[doc(hidden)]
pub mod format;
#[doc(hidden)]
pub mod testing;

pub use config::{
    AccessMode, CloseBudget, CompactionConfig, Config, FilePicker, GroupCommitConfig, L0Metric, Limits, ReadOnlyReason,
    SlowdownPolicy,
};
#[doc(hidden)]
pub use config::{Background, TestConfig};
#[doc(hidden)]
pub use db::TableInfo;
pub use db::{CloseReport, CompactRangeReport, KernoDb, KernoSnapshot};
pub use inspect::{FileInfo, FileType, FormatInfo, HeaderInfo, inspect};
pub use metrics::Metrics;
pub use txn::WriteTxn;
pub use verify::{Issue, LevelReport, Orphan, VerifyOptions, VerifyReport, WalReport, verify};
