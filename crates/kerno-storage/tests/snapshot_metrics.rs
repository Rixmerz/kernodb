//! The snapshot metrics (phase1-design.md §5.7): how many snapshots are alive,
//! the sequence and age of the oldest, and the bytes only snapshots keep
//! alive: tables that a compaction replaced and memtables that were written
//! out, still referenced by a version a snapshot holds.

mod common;

use std::ops::Bound;
use std::time::Duration;

use common::*;
use kerno_core::Durability;
use kerno_core::contract::{Engine, Snapshot};
use kerno_storage::{Config, KernoDb};
use kerno_testkit::harness::Model;
use kerno_testkit::vfs::SimVfs;

/// Both ways the engine runs flushes and compactions.
fn configs() -> [(&'static str, Config); 2] {
    [("inline", config(Durability::Sync)), ("threads", threads_config(Durability::Sync))]
}

fn open_with(disk: &SimVfs, cfg: &Config) -> KernoDb {
    KernoDb::open(disk.as_vfs(), std::path::Path::new(DIR), cfg).unwrap()
}

fn retained(db: &KernoDb) -> u64 {
    db.metrics().snapshots.retained_bytes
}

/// Tables that a directed compaction replaced stay on disk, and count, while
/// a snapshot holds them; they leave with the snapshot.
#[test]
fn retained_bytes_positive_with_pinned_snapshot_then_zero() {
    for (mode, cfg) in configs() {
        let disk = SimVfs::new(1);
        let db = open_with(&disk, &cfg);
        let mut model = Model::new();
        // Two overlapping tables in L0, and nothing left in memory.
        fill(&db, &mut model, 0, 40);
        db.hooks().flush().unwrap();
        fill(&db, &mut model, 20, 40);
        db.hooks().flush().unwrap();
        db.hooks().wait_idle().unwrap();
        assert_eq!(retained(&db), 0, "{mode}: flushed memtables nobody holds are not retained");

        let snap = db.snapshot().unwrap();
        let report = db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
        assert!(report.files_rewritten >= 2, "{mode}: {report:?}");
        assert!(report.pinned_bytes > 0, "{mode}: the snapshot pins the replaced tables: {report:?}");
        assert_eq!(retained(&db), report.pinned_bytes, "{mode}: retained bytes are the pinned tables");
        assert_eq!(stat(&db, "snapshot_retained_bytes"), report.pinned_bytes, "{mode}: also through stats()");
        // They are still on disk and the snapshot still reads them.
        assert_eq!(snap.get(b"key-00000").unwrap().as_deref(), Some(&[0u8; 200][..]), "{mode}");
        assert!(stat(&db, "obsolete_bytes_pending") == 0, "{mode}: nothing is waiting for a deleter");

        drop(snap);
        db.hooks().wait_idle().unwrap();
        assert_eq!(retained(&db), 0, "{mode}: dropping the snapshot releases the tables");
        assert_eq!(model_of(&db), model, "{mode}");
        db.close().unwrap();
    }
}

/// A memtable that was written out is retained while a snapshot taken before
/// the flush holds it, by the size it had.
#[test]
fn retained_memtable_bytes_tracked() {
    for (mode, cfg) in configs() {
        let disk = SimVfs::new(2);
        let db = open_with(&disk, &cfg);
        let mut model = Model::new();
        fill(&db, &mut model, 0, 10);
        let in_memory = db.metrics().shape.memtable_bytes;
        assert!(in_memory > 0);
        let snap = db.snapshot().unwrap();
        db.hooks().flush().unwrap();
        assert_eq!(retained(&db), in_memory, "{mode}: the held memtable counts by its size");

        // A snapshot taken after the flush holds the table, not the memtable.
        let later = db.snapshot().unwrap();
        drop(snap);
        db.hooks().wait_idle().unwrap();
        assert_eq!(retained(&db), 0, "{mode}: the memtable goes with the last snapshot that held it");
        assert_eq!(later.get(b"key-00003").unwrap().as_deref(), Some(&[3u8; 200][..]), "{mode}");
        drop(later);

        // And without a snapshot, a flush retains nothing, however many.
        fill(&db, &mut model, 10, 10);
        db.hooks().flush().unwrap();
        db.hooks().wait_idle().unwrap();
        assert_eq!(retained(&db), 0, "{mode}");
        db.close().unwrap();
    }
}

/// `snapshots_live`, and the sequence and age of the oldest.
#[test]
fn oldest_seq_and_age_reported() {
    let disk = SimVfs::new(3);
    let db = open_with(&disk, &config(Durability::Sync));
    let none = db.metrics().snapshots;
    assert_eq!((none.live, none.oldest_seq, none.oldest_age_ms), (0, 0, 0));

    let mut model = Model::new();
    fill(&db, &mut model, 0, 2);
    let first = db.snapshot().unwrap();
    assert_eq!(first.seq(), 2);
    std::thread::sleep(Duration::from_millis(30));
    fill(&db, &mut model, 2, 3);
    let second = db.snapshot().unwrap();
    let third = db.snapshot().unwrap();
    assert_eq!((second.seq(), third.seq()), (5, 5));

    let m = db.metrics().snapshots;
    assert_eq!((m.live, m.oldest_seq), (3, 2));
    assert!(m.oldest_age_ms >= 30, "the oldest snapshot is {} ms old", m.oldest_age_ms);
    assert_eq!(
        (stat(&db, "snapshots_live"), stat(&db, "snapshot_oldest_seq"), stat(&db, "snapshot_oldest_age_ms") >= 30),
        (3, 2, true),
        "also through stats()"
    );

    drop(first);
    let m = db.metrics().snapshots;
    assert_eq!((m.live, m.oldest_seq), (2, 5), "the next oldest takes over");
    assert!(m.oldest_age_ms < 30, "and it is younger: {} ms", m.oldest_age_ms);
    drop(second);
    drop(third);
    let m = db.metrics().snapshots;
    assert_eq!((m.live, m.oldest_seq, m.oldest_age_ms), (0, 0, 0));
}

/// The registry is shared with every snapshot, so one that outlives the
/// handle (dropped or closed) still reads and deregisters without trouble,
/// and the next handle starts with none.
#[test]
fn registry_survives_handle_drop() {
    for (mode, cfg) in configs() {
        let disk = SimVfs::new(4);
        let db = open_with(&disk, &cfg);
        let mut model = Model::new();
        fill(&db, &mut model, 0, 30);
        let dropped = db.snapshot().unwrap();
        drop(db);
        assert_eq!(dropped.get(b"key-00001").unwrap().as_deref(), Some(&[1u8; 200][..]), "{mode}");
        drop(dropped);

        let db = open_with(&disk, &cfg);
        assert_eq!(db.metrics().snapshots.live, 0, "{mode}: a new handle has its own registry");
        let closed = db.snapshot().unwrap();
        db.close().unwrap();
        assert_eq!(closed.seq(), 30, "{mode}");
        assert_eq!(closed.get(b"key-00029").unwrap().as_deref(), Some(&[29u8; 200][..]), "{mode}");
        drop(closed);

        let db = open_with(&disk, &cfg);
        assert_eq!(db.metrics().snapshots.live, 0, "{mode}");
        db.close().unwrap();
    }
}
