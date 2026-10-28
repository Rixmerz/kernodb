//! A full disk (ENOSPC, `io::ErrorKind::StorageFull`) in the commit path,
//! in a flush and during open, with background threads and inline.
//!
//! The rule (phase1-design.md §4): the operation that hits the full disk
//! returns `Io`, the handle is `Poisoned` from then on, and once space is
//! available again a reopen recovers every acknowledged commit and keeps
//! working.

use std::io;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::{OpenMode, Vfs};
use kerno_core::{Error, WriteBatch};
use kerno_storage::{Background, Config, KernoDb, TestConfig};
use kerno_testkit::harness::{Model, apply, assert_snapshot_matches};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, OpKind, SimVfs};

const DIR: &str = "/db";
const MODES: [Background; 2] = [Background::Threads, Background::Inline];
const FILLER: &str = "/spare/filler";

fn config(background: Background) -> Config {
    Config { write_buffer_bytes: 16 << 10, test: TestConfig { background, db_id: None }, ..Config::default() }
}

fn open(vfs: &Arc<dyn Vfs>, mode: Background) -> kerno_core::Result<KernoDb> {
    KernoDb::open(vfs.clone(), Path::new(DIR), &config(mode))
}

fn is_disk_full(e: &Error) -> bool {
    matches!(e, Error::Io(io) if io.kind() == io::ErrorKind::StorageFull)
}

fn batch(i: u64) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(format!("key-{i:04}"), vec![i as u8; 500]);
    if i.is_multiple_of(3) {
        b.delete(format!("key-{:04}", i / 2));
    }
    b
}

/// Commits batches `from..to` (sequences `from + 1..=to`), each below the
/// write buffer in total, so that nothing is flushed.
fn commit_range(db: &KernoDb, model: &mut Model, from: u64, to: u64) {
    for i in from..to {
        let b = batch(i);
        assert_eq!(db.commit(&b).unwrap(), i + 1);
        apply(model, &b);
    }
}

/// A file outside the database that holds `bytes` of space until removed.
fn add_filler(vfs: &Arc<dyn Vfs>, bytes: usize) {
    vfs.create_dir_all(Path::new(FILLER).parent().unwrap()).unwrap();
    let f = vfs.open(Path::new(FILLER), OpenMode::CreateNew).unwrap();
    f.write_at(&vec![0xAA; bytes], 0).unwrap();
    f.sync_data().unwrap();
}

fn assert_poisoned<T: std::fmt::Debug>(r: kerno_core::Result<T>, ctx: &str) {
    assert!(matches!(r, Err(Error::Poisoned(_))), "{ctx}: expected Poisoned, got {r:?}");
}

/// Reopens `disk`, checks it holds exactly `model` at `seq`, then commits,
/// closes, loses power and checks again.
fn reopen_and_check(disk: &SimVfs, mode: Background, mut model: Model, seq: u64, ctx: &str) {
    let mut rng = Rng::new(seq);
    let db = open(&disk.as_vfs(), mode).unwrap_or_else(|e| panic!("{ctx}: reopen failed: {e}"));
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.seq(), seq, "{ctx}: recovered sequence");
    assert_snapshot_matches(&snap, &model, &mut rng, ctx);
    drop(snap);
    let b = batch(9999);
    assert_eq!(db.commit(&b).unwrap(), seq + 1, "{ctx}: commit after recovery");
    apply(&mut model, &b);
    db.close().unwrap();
    let image = disk.crash_now(CrashKind::PowerLoss);
    let db = open(&image.as_vfs(), mode).unwrap_or_else(|e| panic!("{ctx}: reopen after power loss: {e}"));
    assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &format!("{ctx}, after power loss"));
    db.close().unwrap();
}

#[test]
fn enospc_commit_poisons_then_recovers_after_space_freed() {
    for mode in MODES {
        let ctx = format!("{mode:?}");
        let disk = SimVfs::new(1);
        let vfs = disk.as_vfs();
        add_filler(&vfs, 1 << 20);
        let db = open(&vfs, mode).unwrap();
        let mut model = Model::new();
        commit_range(&db, &mut model, 0, 10);

        // The disk is full: the next WAL append fails and has no effect.
        disk.set_space_limit(Some(disk.space_used()));
        let err = db.commit(&batch(10)).unwrap_err();
        assert!(is_disk_full(&err), "{ctx}: the commit must fail with the disk-full I/O error, got {err}");
        // From then on the handle refuses everything, even once space is back.
        vfs.remove_file(Path::new(FILLER)).unwrap();
        assert_poisoned(db.commit(&batch(10)), &ctx);
        assert_poisoned(db.snapshot().map(|s| s.seq()), &ctx);
        assert_poisoned(db.hooks().wait_idle(), &ctx);
        assert_poisoned(db.close(), &ctx);

        // Space was freed (the limit stays): a reopen recovers exactly the
        // acknowledged commits.
        reopen_and_check(&disk, mode, model, 10, &ctx);
    }
}

#[test]
fn enospc_during_flush_poisons_handle() {
    for mode in MODES {
        let ctx = format!("{mode:?}");
        let disk = SimVfs::new(2);
        let vfs = disk.as_vfs();
        let db = open(&vfs, mode).unwrap();
        let mut model = Model::new();
        commit_range(&db, &mut model, 0, 12);

        // Rotating the memtable needs no space (an empty WAL); writing the
        // SST does.
        disk.start_trace();
        disk.set_space_limit(Some(disk.space_used()));
        let err = db.hooks().flush().unwrap_err();
        match mode {
            // The flush ran in this thread and returns the I/O error.
            Background::Inline => assert!(is_disk_full(&err), "{ctx}: expected the disk-full error, got {err}"),
            // The background flush failed and poisoned the handle.
            Background::Threads => {
                assert!(matches!(&err, Error::Poisoned(m) if m.starts_with("flush failed")), "{ctx}: got {err}")
            }
        }
        let trace = disk.trace();
        let failed = trace.iter().rev().find(|e| e.op.kind == OpKind::WriteAt).expect("a write was attempted");
        assert_eq!(failed.op.phase, "flush", "{ctx}: the write that hit the full disk: {failed:?}");
        assert!(failed.op.path.extension().is_some_and(|x| x == "sst"), "{ctx}: {failed:?}");
        if mode == Background::Threads {
            assert_eq!(failed.thread, "kerno-flush", "{ctx}: {failed:?}");
        }

        assert_poisoned(db.commit(&batch(12)), &ctx);
        assert_poisoned(db.hooks().flush(), &ctx);
        assert_poisoned(db.hooks().wait_idle(), &ctx);
        assert_poisoned(db.hooks().compact_once(), &ctx);
        drop(db);

        disk.set_space_limit(None);
        reopen_and_check(&disk, mode, model, 12, &ctx);
    }
}

#[test]
fn enospc_during_open_fails_and_retry_succeeds() {
    for mode in MODES {
        // A database left without a clean close: recovery has to write an
        // L0 table, a MANIFEST and a new WAL.
        let disk = SimVfs::new(3);
        let mut model = Model::new();
        {
            let db = open(&disk.as_vfs(), mode).unwrap();
            commit_range(&db, &mut model, 0, 15);
        }
        let base = disk.dump();
        let opening = SimVfs::restore(&base);
        let start = opening.mutating_ops();
        drop(open(&opening.as_vfs(), mode).unwrap());
        let ops = opening.mutating_ops() - start;
        assert!(ops > 5, "recovery must write: {ops} operations");

        // The disk becomes full at every operation of that open in turn.
        let mut failed = 0;
        for point in 1..=ops {
            let ctx = format!("{mode:?}, disk full from operation {point} of {ops} of open");
            let disk = SimVfs::restore_seeded(&base, point);
            disk.fill_disk_after(point);
            match open(&disk.as_vfs(), mode) {
                Ok(db) => drop(db),
                Err(e) => {
                    assert!(is_disk_full(&e), "{ctx}: open must fail with the disk-full error, got {e}");
                    failed += 1;
                    // A failed open changes no existing file (it may leave
                    // an unreferenced, incomplete new one).
                    let after = disk.dump();
                    for (path, bytes) in &base.files {
                        assert!(after.files.get(path) == Some(bytes), "{ctx}: {} changed", path.display());
                    }
                    // Still full: a retry fails the same way.
                    let again = open(&disk.as_vfs(), mode).err();
                    assert!(again.as_ref().is_some_and(is_disk_full), "{ctx}: retry on a full disk: {again:?}");
                }
            }
            disk.set_space_limit(None);
            reopen_and_check(&disk, mode, model.clone(), 15, &ctx);
        }
        assert!(failed > 0, "{mode:?}: no fill point made open fail");
    }
}
