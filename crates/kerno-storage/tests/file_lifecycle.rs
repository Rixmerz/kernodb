//! Deferred deletion of obsolete files (v1.md W-6, W-13): a table that a
//! durable record dropped is deleted by a dedicated thread while the engine
//! lives (failures retried and counted), inline once the handle has closed,
//! and never by a reader.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use kerno_core::Durability;
use kerno_core::contract::{Engine, Snapshot};
use kerno_storage::KernoDb;
use kerno_testkit::harness::Model;
use kerno_testkit::vfs::{HookAction, HookOpKind, HookPoint, HookVfs, OpKind, SimVfs};
use parking_lot::Mutex;

use common::*;

/// Rewrites the same keys until compactions (not trivial moves: those obsolete
/// nothing) have made some tables obsolete.
fn churn(db: &KernoDb, model: &mut Model) {
    churn_until(db, model, 3);
}

fn churn_until(db: &KernoDb, model: &mut Model, compactions: u64) {
    let mut i = 0;
    while stat(db, "compactions") < compactions {
        assert!(i < 100, "no compaction after {i} rounds");
        fill(db, model, 0, 90);
        i += 1;
    }
}

fn ssts_on_disk(disk: &SimVfs) -> usize {
    disk.files().iter().filter(|(p, _)| p.extension().is_some_and(|e| e == "sst")).count()
}

#[test]
fn obsolete_files_deleted_by_background_deleter() {
    let disk = SimVfs::new(1);
    disk.start_trace();
    let db = open_in(&disk.as_vfs(), &threads_config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    churn(&db, &mut model);
    kerno_core::contract::TestHooks::wait_idle(db.hooks()).unwrap();
    assert!(stat(&db, "obsolete_files_deleted") > 0);
    assert_eq!(stat(&db, "obsolete_bytes_pending"), 0, "everything queued has been deleted");
    assert_eq!(stat(&db, "delete_failures"), 0);
    // The live tables are exactly the files on disk: nothing obsolete is left.
    let live: usize = db.level_tables().iter().map(Vec::len).sum();
    assert_eq!(ssts_on_disk(&disk), live);
    // Every removal of a table was done by the deleter thread.
    let removals: Vec<_> = disk
        .trace()
        .into_iter()
        .filter(|t| t.op.kind == OpKind::Remove && t.op.path.extension().is_some_and(|e| e == "sst"))
        .collect();
    assert!(!removals.is_empty());
    for t in &removals {
        assert_eq!(t.thread, "kerno-delete", "{t:?}");
        assert_eq!(t.op.phase, "delete");
    }
    assert!(model_of(&db) == model);
}

#[test]
fn failed_delete_is_retried_and_counted() {
    let disk = SimVfs::new(2);
    let failures = Arc::new(Mutex::new(0u32));
    let budget = failures.clone();
    // The first three removals of any table fail.
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        if op.kind == HookOpKind::Remove
            && op.point == HookPoint::Before
            && op.path.extension().is_some_and(|e| e == "sst")
        {
            let mut left = budget.lock();
            if *left < 3 {
                *left += 1;
                return HookAction::Fail(std::io::ErrorKind::PermissionDenied);
            }
        }
        HookAction::Proceed
    });
    let db = open_in(&vfs, &threads_config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    churn(&db, &mut model);
    // Let flushes and compactions settle so no table is still being written.
    kerno_core::contract::TestHooks::wait_idle(db.hooks()).unwrap();
    // The deleter retries by itself: wait for it, polling a condition.
    let deadline = Instant::now() + Duration::from_secs(20);
    while stat(&db, "obsolete_bytes_pending") > 0 || stat(&db, "delete_failures") < 3 {
        assert!(Instant::now() < deadline, "the failed deletions were not retried: {:?}", db.stats());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(stat(&db, "delete_failures"), 3);
    assert!(stat(&db, "obsolete_files_deleted") >= 3);
    let live: usize = db.level_tables().iter().map(Vec::len).sum();
    assert_eq!(ssts_on_disk(&disk), live, "the retried files are gone too");
    assert!(model_of(&db) == model);
    // A failed deletion is not an error of the engine: it keeps working.
    let mut m = model;
    put(&db, &mut m, "after", b"x");
    db.close().unwrap();
}

#[test]
fn outliving_snapshot_deletes_inline_after_close() {
    let disk = SimVfs::new(3);
    let db = open_in(&disk.as_vfs(), &threads_config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    churn(&db, &mut model);
    db.hooks_wait();
    // From here on the snapshot pins the tables that exist now.
    let snap = db.snapshot().unwrap();
    let pinned = model.clone();
    let done = stat(&db, "compactions");
    churn_until(&db, &mut model, done + 3);
    db.hooks_wait();
    let with_snapshot = ssts_on_disk(&disk);
    db.close().unwrap();
    // The handle is gone but the snapshot still reads, and keeps its tables.
    assert!(model_of_snapshot(&snap) == pinned);
    let live_after_close = ssts_on_disk(&disk);
    assert!(live_after_close > 0);
    let obsolete: Vec<_> = {
        let (replay, _) = manifest_of(&disk.dump());
        let live: Vec<u64> = replay.state.levels.iter().flatten().map(|f| f.number).collect();
        files_with(&disk.dump(), "sst").into_iter().filter(|p| !live.contains(&file_number(p))).collect()
    };
    assert!(!obsolete.is_empty(), "the snapshot pins tables the MANIFEST no longer lists");
    // Dropping the last reference deletes them right here, in this thread.
    disk.start_trace();
    drop(snap);
    let trace = disk.trace();
    assert!(!trace.is_empty(), "nothing was deleted when the snapshot went away");
    let me = std::thread::current().name().unwrap().to_string();
    assert!(trace.iter().all(|t| t.op.kind == OpKind::Remove && t.thread == me), "{trace:?}");
    for p in &obsolete {
        assert!(!disk.as_vfs().exists(p).unwrap(), "{} survived", p.display());
    }
    assert!(ssts_on_disk(&disk) < with_snapshot.max(live_after_close));
}

#[test]
fn readers_never_delete_files() {
    let disk = SimVfs::new(4);
    disk.start_trace();
    let db = open_in(&disk.as_vfs(), &threads_config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    churn(&db, &mut model);
    db.hooks_wait();
    let snap = db.snapshot().unwrap();
    let done = stat(&db, "compactions");
    churn_until(&db, &mut model, done + 3);
    db.hooks_wait();
    let before = disk.trace().len();
    // A reader thread holds the oldest snapshot and drops it last.
    let reader = std::thread::Builder::new()
        .name("reader".into())
        .spawn(move || {
            let n = snap.scan(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded, usize::MAX).unwrap().len();
            drop(snap);
            n
        })
        .unwrap();
    assert!(reader.join().unwrap() > 0);
    db.hooks_wait();
    let removals: Vec<_> = disk.trace()[before..].iter().filter(|t| t.op.kind == OpKind::Remove).cloned().collect();
    assert!(!removals.is_empty(), "dropping the oldest snapshot freed nothing");
    for t in disk.trace().iter().filter(|t| t.op.kind == OpKind::Remove) {
        assert_ne!(t.thread, "reader", "a reader deleted {}", t.op.path.display());
    }
    assert!(stat(&db, "obsolete_bytes_pending") == 0);
}

trait Wait {
    fn hooks_wait(&self);
}

impl Wait for KernoDb {
    fn hooks_wait(&self) {
        self.hooks().wait_idle().unwrap();
    }
}

fn model_of_snapshot(s: &kerno_storage::KernoSnapshot) -> Model {
    kerno_testkit::harness::read_all(s).unwrap()
}
