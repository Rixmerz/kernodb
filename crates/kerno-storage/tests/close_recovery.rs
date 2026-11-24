//! The close-time flush writes plain data blocks, and the flush of what a
//! recovery open replayed is deferred (perf-gaps-2026-11.md, Rank 5 and the
//! recovery open): both must lose nothing, and the deferred work must be
//! completed by the flusher, by `flush`, or by the next close.

mod common;

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kerno_core::Durability;
use kerno_core::vfs::{OpenMode, Vfs};
use kerno_storage::format::features::BLOCK_LZ4;
use kerno_storage::format::manifest::Edit;
use kerno_testkit::harness::Model;
use kerno_testkit::vfs::{CrashKind, DirImage, HookAction, HookOpKind, HookPoint, HookVfs, SimVfs};

use common::*;

/// `incompat` of a table file (the header word at offset 32).
fn incompat(image: &DirImage, file: &std::path::Path) -> u64 {
    le_u64(&image.files[file], 32)
}

/// Ten small commits, which stay in one memtable: the only table the session
/// writes is the one of the close, or of the `flush` asked for.
fn one_memtable(flush_before_close: bool) -> (DirImage, Model) {
    let disk = SimVfs::new(7);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    assert!(config(Durability::Sync).block_compression, "the default compresses");
    let mut model = Model::new();
    for i in 0..10u32 {
        put(&db, &mut model, &format!("key-{i:05}"), &[b'x'; 300]);
    }
    if flush_before_close {
        db.hooks().flush().unwrap();
    }
    db.close().unwrap();
    (disk.dump(), model)
}

#[test]
fn the_close_time_flush_writes_plain_blocks_and_the_file_reopens_and_reads() {
    let (closed, model) = one_memtable(false);
    let ssts = files_with(&closed, "sst");
    assert_eq!(ssts.len(), 1, "{ssts:?}");
    assert_eq!(incompat(&closed, &ssts[0]) & BLOCK_LZ4, 0, "the close-time table is not compressed");
    // The same data flushed while the database runs is compressed.
    let (running, _) = one_memtable(true);
    let ssts = files_with(&running, "sst");
    assert_eq!(ssts.len(), 1, "{ssts:?}");
    assert_ne!(incompat(&running, &ssts[0]) & BLOCK_LZ4, 0, "a running flush compresses");

    // The plain table reopens, reads, takes new commits next to compressed tables, and closes again.
    let (_, db) = open_copy(&closed, &config(Durability::Sync));
    let db = db.unwrap();
    assert_eq!(stat(&db, "last_open_clean"), 1);
    assert!(model_of(&db) == model);
    let mut model = model;
    fill(&db, &mut model, 100, 120);
    assert!(model_of(&db) == model);
    db.hooks().flush().unwrap();
    db.close().unwrap();
}

#[test]
fn a_mixed_close_reopens_with_every_key() {
    let disk = SimVfs::new(9);
    let mut model = Model::new();
    for round in 0..3u32 {
        let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
        assert!(model_of(&db) == model, "round {round}");
        fill(&db, &mut model, round * 150, 150);
        db.close().unwrap();
    }
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    assert!(model_of(&db) == model);
}

#[test]
fn recovery_open_does_not_flush_and_keeps_the_logs_until_the_work_is_done() {
    let (image, model) = unclean_image();
    let tables_before = files_with(&image, "sst");
    let (disk, db) = open_copy(&image, &config(Durability::Sync));
    let db = db.unwrap();
    assert!(stat(&db, "last_open_replayed_bytes") > 0);
    assert_eq!(stat(&db, "flushes"), 0, "recovery did not flush");
    let opened = disk.dump();
    assert_eq!(files_with(&opened, "sst"), tables_before, "no table was written by the open");
    // The old log stays (and the new one is there), and the MANIFEST still
    // names the old one as the oldest live log.
    let wals = files_with(&opened, "wal");
    assert_eq!(wals.len(), 2, "{wals:?}");
    let (replay, _) = manifest_of(&opened);
    let (_, old) = manifest_of(&image);
    assert!(old.len() < manifest_of(&opened).1.len());
    assert_eq!(replay.state.log_number, file_number(&wals[0]));
    assert!(model_of(&db) == model, "the replayed commits are readable at once");

    // The deferred flush is completed on demand: tables appear, the old log goes.
    db.hooks().flush().unwrap();
    assert!(stat(&db, "flushes") >= 1);
    let flushed = disk.dump();
    assert!(files_with(&flushed, "sst").len() > tables_before.len());
    assert_eq!(files_with(&flushed, "wal").len(), 1, "only the live log is left");
    assert!(model_of(&db) == model);
    drop(db);
    let (_, db) = open_copy(&disk.dump(), &config(Durability::Sync));
    assert!(model_of(&db.unwrap()) == model, "a reopen after the deferred flush");
}

#[test]
fn a_close_right_after_a_recovery_open_completes_the_deferred_work() {
    let (image, model) = unclean_image();
    let (disk, db) = open_copy(&image, &config(Durability::Sync));
    let db = db.unwrap();
    assert_eq!(stat(&db, "flushes"), 0);
    db.close().unwrap();
    let closed = disk.dump();
    let (replay, _) = manifest_of(&closed);
    assert!(replay.clean);
    assert!(files_with(&closed, "wal").is_empty());
    let (_, db) = open_copy(&closed, &config(Durability::Sync));
    let db = db.unwrap();
    assert_eq!(stat(&db, "last_open_replayed_bytes"), 0, "nothing is left to replay");
    assert!(model_of(&db) == model);
}

#[test]
fn a_crash_after_a_recovery_open_recovers_every_commit_again_and_again() {
    let (mut image, model) = unclean_image();
    for round in 0..4 {
        let (disk, db) = open_copy(&image, &config(Durability::Sync));
        let db = db.unwrap();
        assert!(model_of(&db) == model, "round {round}");
        // Alternate a process kill and a power loss right after the open.
        image = if round % 2 == 0 { disk.dump() } else { disk.crash_now(CrashKind::PowerLoss).dump() };
        drop(db);
    }
    let (_, db) = open_copy(&image, &config(Durability::Sync));
    assert!(model_of(&db.unwrap()) == model);
}

#[test]
fn new_commits_after_a_deferred_recovery_survive_a_crash() {
    let (image, mut model) = unclean_image();
    let (disk, db) = open_copy(&image, &config(Durability::Sync));
    let db = db.unwrap();
    fill(&db, &mut model, 1000, 30); // lands in the new log, behind the replayed ones
    drop(db);
    let (_, db) = open_copy(&disk.crash_now(CrashKind::PowerLoss).dump(), &config(Durability::Sync));
    assert!(model_of(&db.unwrap()) == model);
}

/// A crash (a failed operation, then the files as they are, or as a power
/// loss leaves them) at every mutating operation of a recovery open: the next
/// open recovers every committed key.
#[test]
fn a_crash_at_every_step_of_a_recovery_open_loses_nothing() {
    let (image, model) = unclean_image();
    let count = |target: Option<u64>| -> (u64, SimVfs, bool) {
        let disk = SimVfs::restore(&image);
        let seen = Arc::new(AtomicU64::new(0));
        let counter = seen.clone();
        let vfs: Arc<dyn Vfs> = HookVfs::wrap(disk.as_vfs(), move |op| {
            let mutating = matches!(
                op.kind,
                HookOpKind::Remove
                    | HookOpKind::Rename
                    | HookOpKind::SyncDir
                    | HookOpKind::WriteAt
                    | HookOpKind::SetLen
                    | HookOpKind::SyncData
            ) || (op.kind == HookOpKind::Open && op.mode == Some(OpenMode::CreateNew));
            if op.point == HookPoint::Before && mutating {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if Some(n) == target {
                    return HookAction::Fail(io::ErrorKind::Other);
                }
            }
            HookAction::Proceed
        });
        let opened = open_in(&vfs, &config(Durability::Sync));
        let failed = opened.is_err();
        drop(opened);
        (seen.load(Ordering::SeqCst), disk, failed)
    };
    let (total, _, failed) = count(None);
    assert!(!failed && total > 0);
    for k in 0..total {
        let (_, disk, failed) = count(Some(k));
        assert!(failed, "op {k} was not on the path of the open");
        for (kind, image) in [("kill", disk.dump()), ("power loss", disk.crash_now(CrashKind::PowerLoss).dump())] {
            let (_, db) = open_copy(&image, &config(Durability::Sync));
            let db = db.unwrap_or_else(|e| panic!("op {k} ({kind}): {e}"));
            assert!(model_of(&db) == model, "op {k} ({kind}): the recovered state differs");
        }
    }
}

#[test]
fn the_flush_edit_of_a_deferred_recovery_moves_the_oldest_log_forward() {
    let (image, _) = unclean_image();
    let (disk, db) = open_copy(&image, &config(Durability::Sync));
    let db = db.unwrap();
    let (before, _) = manifest_of(&disk.dump());
    db.hooks().flush().unwrap();
    let (after, records) = manifest_of(&disk.dump());
    assert!(after.state.log_number > before.state.log_number);
    assert!(after.state.last_seq > before.state.last_seq, "the tables now hold the replayed commits");
    assert!(records.iter().any(|(_, r)| matches!(r.edit, Edit::Flush { .. })));
}
