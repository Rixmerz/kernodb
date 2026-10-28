//! The failure matrix of the LSM engine as a classifier of I/O operations
//! (phase1-design.md §6.3, `docs/architecture/failure-matrix.md`).
//!
//! A sweep that crashes the engine at every mutating operation of a scenario
//! can say which row of the matrix each point belongs to, from three things
//! the traced operation carries: the probe phase of the thread that issued it
//! ([`kerno_core::probe`]), the kind of file it touched and the kind of
//! operation. [`classify`] is that table; [`rows_covered`] folds a trace into
//! the set of rows it hit, and a coverage test fails when a row of the work
//! package it belongs to gets no point, or when an operation maps to no row
//! at all (an I/O the matrix does not know about is a hole in it).
//!
//! Rows that are not a place in the protocol but a mode of the device or of
//! the handle (`RO`, `FL`, `NS`, `KP`) are not derived from operations: a
//! coverage test arms the mode, observes that it fired and records the row
//! itself.

use std::collections::BTreeSet;
use std::path::Path;

use crate::vfs::{OpInfo, OpKind, TraceEntry};

/// The kind of file an operation touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileClass {
    Manifest,
    ManifestTmp,
    Wal,
    /// The spare WAL (`WAL.spare`) of a preallocating engine.
    SpareWal,
    Sst,
    /// A `sync_dir`: the directory itself.
    Dir,
    Other,
}

/// The class of the file or directory of `op`.
pub fn file_class(op: &OpInfo) -> FileClass {
    if op.kind == OpKind::SyncDir {
        return FileClass::Dir;
    }
    let name = op.path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    match name {
        "MANIFEST" => FileClass::Manifest,
        "MANIFEST.tmp" => FileClass::ManifestTmp,
        "WAL.spare" => FileClass::SpareWal,
        _ => match Path::new(name).extension().and_then(|e| e.to_str()) {
            Some("wal") => FileClass::Wal,
            Some("sst") => FileClass::Sst,
            _ => FileClass::Other,
        },
    }
}

/// The rows the protocol places in the commit, rotation, flush, compaction,
/// session, close, checkpoint and creation paths of scenario B, the ones
/// [`classify`] derives from operations (v1.md §9.4, §10; W-1..W-13).
pub const PROTOCOL_ROWS: &[&str] =
    &["L1-L4", "L5'", "L6", "L7'", "L8", "L9", "L10'", "L11", "L12'", "L13'", "CK", "CR", "R0"];

/// The rows that are a mode of the device or the handle, recorded by their
/// own tests.
pub const MODE_ROWS: &[&str] = &["RO", "FL", "NS", "KP"];

/// The rows of `op`: the table of phase1-design.md §6.3 keyed on (probe
/// phase, file kind, operation). Empty if the matrix has no row for it.
pub fn classify(op: &OpInfo) -> &'static [&'static str] {
    use FileClass::*;
    let file = file_class(op);
    match (op.phase, file, op.kind) {
        ("commit", Wal, OpKind::WriteAt | OpKind::SyncData) => &["L1-L4"],
        ("rotate", Wal | SpareWal, _) | ("rotate", Dir, OpKind::SyncDir) => &["L5'"],
        // Preparing the spare WAL from scratch is part of getting the next
        // log ready: a half-made spare is deleted at open.
        ("wal_spare", SpareWal, _) | ("wal_spare", Dir, OpKind::SyncDir) => &["L5'"],
        ("flush", Sst, _) | ("flush", Dir, OpKind::SyncDir) => &["L6"],
        ("flush", Manifest, OpKind::WriteAt | OpKind::SyncData) => &["L7'"],
        ("delete", Wal, OpKind::Remove) => &["L8"],
        // Recycling the flushed WAL into the spare instead of deleting it.
        ("delete", Wal, OpKind::Rename) | ("delete", SpareWal, _) | ("delete", Dir, OpKind::SyncDir) => &["L8"],
        ("compact", Sst, _) | ("compact", Dir, OpKind::SyncDir) => &["L9"],
        ("compact", Manifest, OpKind::WriteAt | OpKind::SyncData) => &["L10'"],
        ("delete", Sst, OpKind::Remove) => &["L11"],
        ("session", Manifest, OpKind::WriteAt | OpKind::SyncData) => &["L12'", "R0"],
        ("session", _, _) => &["L12'"],
        ("close", _, _) => &["L13'"],
        ("checkpoint", ManifestTmp | Manifest | Dir, _) => &["CK"],
        ("create", _, _) => &["CR"],
        _ => &[],
    }
}

/// The rows hit by the operations of `trace`.
pub fn rows_covered(trace: &[TraceEntry]) -> BTreeSet<&'static str> {
    trace.iter().flat_map(|t| classify(&t.op).iter().copied()).collect()
}

/// The first operation of `trace` that belongs to no row, described.
pub fn first_unclassified(trace: &[TraceEntry]) -> Option<String> {
    trace.iter().find(|t| classify(&t.op).is_empty()).map(|t| {
        format!(
            "#{}: {:?} of {} in phase {:?} (file class {:?})",
            t.n,
            t.op.kind,
            t.op.path.display(),
            t.op.phase,
            file_class(&t.op)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn op(kind: OpKind, path: &str, phase: &'static str) -> OpInfo {
        OpInfo { kind, path: PathBuf::from(path), to: None, offset: 0, len: 0, phase }
    }

    #[test]
    fn rows_follow_phase_file_and_operation() {
        let rows = |o: OpInfo| classify(&o).to_vec();
        assert_eq!(rows(op(OpKind::WriteAt, "/db/000007.wal", "commit")), ["L1-L4"]);
        assert_eq!(rows(op(OpKind::SyncDir, "/db", "rotate")), ["L5'"]);
        assert_eq!(rows(op(OpKind::SyncData, "/db/000009.sst", "flush")), ["L6"]);
        assert_eq!(rows(op(OpKind::WriteAt, "/db/MANIFEST", "flush")), ["L7'"]);
        assert_eq!(rows(op(OpKind::Remove, "/db/000003.wal", "delete")), ["L8"]);
        assert_eq!(rows(op(OpKind::Rename, "/db/000003.wal", "delete")), ["L8"]);
        assert_eq!(rows(op(OpKind::WriteAt, "/db/WAL.spare", "delete")), ["L8"]);
        assert_eq!(rows(op(OpKind::WriteAt, "/db/WAL.spare", "wal_spare")), ["L5'"]);
        assert_eq!(rows(op(OpKind::Rename, "/db/WAL.spare", "rotate")), ["L5'"]);
        assert_eq!(rows(op(OpKind::WriteAt, "/db/MANIFEST", "compact")), ["L10'"]);
        assert_eq!(rows(op(OpKind::Remove, "/db/000004.sst", "delete")), ["L11"]);
        assert_eq!(rows(op(OpKind::SyncData, "/db/MANIFEST", "session")), ["L12'", "R0"]);
        assert_eq!(rows(op(OpKind::Create, "/db/000011.wal", "session")), ["L12'"]);
        assert_eq!(rows(op(OpKind::Remove, "/db/000011.wal", "close")), ["L13'"]);
        assert_eq!(rows(op(OpKind::Rename, "/db/MANIFEST.tmp", "checkpoint")), ["CK"]);
        assert_eq!(rows(op(OpKind::Create, "/db/MANIFEST.tmp", "create")), ["CR"]);
        // A WAL write outside a commit has no row.
        assert!(classify(&op(OpKind::WriteAt, "/db/000007.wal", "flush")).is_empty());
    }
}
