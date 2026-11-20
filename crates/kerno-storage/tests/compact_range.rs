//! Directed compaction, `compact_range` (phase1-design.md §5.5, ADR-0001
//! condition 2): what leaves the files, what a snapshot keeps, what the
//! MANIFEST remembers, what the block cache holds, and a crash at every point
//! of it.
//!
//! The engine runs inline with a 16 KiB write buffer and an unbounded close
//! (see `common`), so the sweeps number their operations the same way each
//! time.

mod common;

use std::ops::Bound;
use std::sync::Arc;

use common::*;
use kerno_core::contract::{Engine, Snapshot};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::format::manifest::Edit;
use kerno_storage::testing::{Inline, Profiled};
use kerno_storage::{AccessMode, Config, KernoDb};
use kerno_testkit::harness::crash::{Step, every_matching_point, every_point_steps};
use kerno_testkit::harness::{Model, apply, assert_snapshot_matches};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, DirImage, OpInfo, SimVfs};

/// The bytes that must not be found in any file after a directed compaction.
const MARKER: &[u8] = b"zz-marker";

fn marker_key(i: u32) -> String {
    format!("zz-marker-{i:04}")
}

fn open_db(disk: &SimVfs) -> KernoDb {
    open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap()
}

fn range_batch(start: &str, end: &str) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.delete_range(start, end);
    b
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The names of the live files of `image` (and of those unlinked but still
/// open) that hold `needle`.
fn files_holding(image: &DirImage, needle: &[u8]) -> Vec<String> {
    let mut found: Vec<String> =
        image.files.iter().filter(|(_, bytes)| contains(bytes, needle)).map(|(p, _)| p.display().to_string()).collect();
    found.extend(image.unlinked_open.iter().filter(|b| contains(b, needle)).map(|_| "(unlinked, open)".to_string()));
    found
}

/// Values and keys with the marker, written, overwritten, deleted one by one
/// and deleted by ranges, over data of other keys: enough volume that the
/// marker is in files at several levels, in the log and in the MANIFEST.
fn dirty_database(disk: &SimVfs) -> (KernoDb, Model) {
    let db = open_db(disk);
    let mut model = Model::new();
    fill(&db, &mut model, 0, 300);
    for i in 0..120 {
        put(&db, &mut model, &marker_key(i), &[MARKER, &[i as u8; 200]].concat());
    }
    // Overwritten versions, and keys deleted one at a time.
    for i in 0..30 {
        put(&db, &mut model, &marker_key(i), &[MARKER, b"-second-", &[i as u8; 150]].concat());
    }
    for i in 30..50 {
        let mut b = WriteBatch::new();
        b.delete(marker_key(i));
        let seq = db.commit(&b).unwrap();
        apply(&mut model, &b);
        assert!(seq > 0);
    }
    // The rest by ranges: one in the middle, one over the tail.
    for (a, z) in [(50, 90), (90, 1000)] {
        let b = range_batch(&marker_key(a), &marker_key(z));
        db.commit(&b).unwrap();
        apply(&mut model, &b);
    }
    // And the first ones, deleted last.
    let b = range_batch(&marker_key(0), &marker_key(30));
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    // More ordinary data after them, so that the ranges are not all in the
    // newest file.
    fill(&db, &mut model, 300, 200);
    assert!(model.keys().all(|k| !contains(k, MARKER)), "every marker key is deleted");
    (db, model)
}

#[test]
fn compact_range_removes_marker_from_all_live_files() {
    let disk = SimVfs::new(1);
    let (db, model) = dirty_database(&disk);
    assert!(!files_holding(&disk.dump(), MARKER).is_empty(), "test setup: the marker must be on disk before");
    let report = db.compact_range(Bound::Included(MARKER), Bound::Excluded(b"zz-markes")).unwrap();
    assert_eq!((report.pinned_files, report.pinned_bytes), (0, 0), "nothing holds the replaced files");
    assert!(report.files_rewritten > 0 && report.bytes_read > 0 && report.bytes_written > 0, "{report:?}");
    // Not a byte of it in any file the database has: SSTs, the MANIFEST
    // (whose records name the keys of the files that went), the log.
    let image = disk.dump();
    assert_eq!(files_holding(&image, MARKER), Vec::<String>::new(), "the marker survives in these files");
    // The data of the other keys is untouched, and everything below the
    // range is in the last level.
    assert!(model.len() >= 500);
    check_state(&db, &model, "after compact_range");
    db.close().unwrap();
    let image = disk.dump();
    assert_eq!(files_holding(&image, MARKER), Vec::<String>::new());
    let db = open_db(&disk);
    check_state(&db, &model, "after reopen");
}

fn check_state(db: &KernoDb, model: &Model, ctx: &str) {
    let all = kerno_testkit::harness::read_all(&db.snapshot().unwrap()).unwrap();
    assert!(all == *model, "{ctx}: {}", kerno_testkit::harness::diff(model, &all));
}

#[test]
fn compact_range_reports_pinned_snapshot() {
    let disk = SimVfs::new(2);
    let (db, model) = dirty_database(&disk);
    // A snapshot of the state before the data was deleted again would hold
    // the files it reads; take one now, of everything.
    let snap = db.snapshot().unwrap();
    let held = read_all_of(&snap);
    let before: Vec<_> = sst_files(&disk);
    let report = db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    assert!(report.pinned_files > 0 && report.pinned_bytes > 0, "{report:?}");
    assert!(report.pinned_files <= report.files_rewritten);
    // The files it keeps are on disk still...
    let after = sst_files(&disk);
    let kept: Vec<_> = before.iter().filter(|f| after.contains(f)).collect();
    assert_eq!(kept.len() as u64, report.pinned_files, "pinned {kept:?} of {before:?}, now {after:?}");
    assert_eq!(read_all_of(&snap), held, "the snapshot reads what it read");
    check_state(&db, &model, "current state");
    // ...and go with the snapshot.
    drop(snap);
    db.hooks().wait_idle().unwrap();
    let gone = sst_files(&disk);
    assert!(kept.iter().all(|f| !gone.contains(f)), "{gone:?} still has {kept:?}");
}

fn read_all_of(snap: &kerno_storage::KernoSnapshot) -> Vec<(Vec<u8>, Vec<u8>)> {
    snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap()
}

fn sst_files(disk: &SimVfs) -> Vec<String> {
    let mut v: Vec<String> = disk
        .dump()
        .files
        .keys()
        .filter(|p| p.extension().is_some_and(|e| e == "sst"))
        .map(|p| p.display().to_string())
        .collect();
    v.sort();
    v
}

#[test]
fn compact_range_checkpoints_manifest() {
    let disk = SimVfs::new(3);
    let (db, _model) = dirty_database(&disk);
    let (replay, records) = manifest_of(&disk.dump());
    let (generation, before) = (replay.header.file_number, records.len());
    assert!(before > 3, "the history is long before: {before} records");
    let checkpoints = stat(&db, "manifest_checkpoints");
    db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    assert_eq!(stat(&db, "manifest_checkpoints"), checkpoints + 1, "one checkpoint");
    let (replay, records) = manifest_of(&disk.dump());
    assert_eq!(replay.header.file_number, generation + 1, "a new generation");
    assert_eq!(records.len(), 1, "history is one SNAPSHOT");
    assert!(matches!(records[0].1.edit, Edit::Snapshot { .. }));
    assert!(!replay.clean, "the checkpoint of a live handle is not the CLEAN one");
}

#[test]
fn compact_range_purges_cache() {
    let disk = SimVfs::new(4);
    let (db, model) = dirty_database(&disk);
    // Everything that is live gets read once: its blocks are cached.
    check_state(&db, &model, "warm");
    let warm = db.metrics().shape.block_cache_bytes;
    assert!(warm > 0, "the reads must fill the cache");
    db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    // The compaction reads without filling the cache; the tables it replaced
    // are gone from it.
    assert_eq!(db.metrics().shape.block_cache_bytes, 0, "blocks of replaced tables are still cached");
    check_state(&db, &model, "after");
    assert!(db.metrics().shape.block_cache_bytes > 0, "the new tables fill it again");
}

#[test]
fn compact_range_pushes_only_the_range_to_the_bottom() {
    let disk = SimVfs::new(5);
    let db = open_db(&disk);
    let mut model = Model::new();
    fill(&db, &mut model, 0, 400);
    db.hooks().flush().unwrap();
    while db.hooks().compact_once().unwrap() {}
    let key = |i: u32| format!("key-{i:05}").into_bytes();
    let report = db.compact_range(Bound::Included(&key(100)), Bound::Excluded(&key(200))).unwrap();
    assert!(report.files_rewritten > 0, "{report:?}");
    // Whatever overlaps the range is in the last level; the files above the
    // last level do not touch it.
    for (level, tables) in db.level_tables().iter().enumerate().take(6) {
        for t in tables {
            assert!(t.largest < key(100) || t.smallest >= key(200), "L{level} #{} still overlaps the range", t.number);
        }
    }
    check_state(&db, &model, "after");
    // A range with nothing in it does nothing.
    let report = db.compact_range(Bound::Included(b"zzz"), Bound::Unbounded).unwrap();
    assert_eq!(report, Default::default());
}

#[test]
fn compact_range_needs_a_writable_handle() {
    let disk = SimVfs::new(6);
    let (db, _) = dirty_database(&disk);
    db.close().unwrap();
    let cfg = Config { access: AccessMode::ReadOnly, ..config(Durability::Sync) };
    let db = KernoDb::open(disk.as_vfs(), std::path::Path::new(DIR), &cfg).unwrap();
    let before = disk.mutating_ops();
    assert!(matches!(db.compact_range(Bound::Unbounded, Bound::Unbounded), Err(Error::ReadOnly(_))));
    assert_eq!(disk.mutating_ops(), before, "a read-only handle writes nothing");
}

type Db = Profiled<Inline>;

fn steps_with_compact_range(seed: u64) -> Vec<Step<Db>> {
    ranged_scenario::<Db>(
        seed,
        Arc::new(|db: &Db| {
            db.hooks().compact_range(Bound::Included(b"key-00000010"), Bound::Excluded(b"key-00000080"))
        }),
        Arc::new(|db: &Db| db.hooks().checkpoint()),
    )
}

#[test]
fn compact_range_crash_at_every_point() {
    let steps = steps_with_compact_range(9);
    let in_compact_range: kerno_testkit::harness::crash::OpFilter =
        Arc::new(|op: &OpInfo| op.phase == "compact_range" || op.phase == "checkpoint");
    for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill, CrashKind::ProcessKillKeepPending] {
        every_matching_point::<Db>(kind, Durability::Sync, &steps, in_compact_range.clone());
    }
    every_point_steps::<Db>(CrashKind::PowerLoss, Durability::Sync, &|_| {}, &steps);
}

/// The hook of the contract is `compact_range`, and recovers the same state.
#[test]
fn compact_range_hook_matches_the_method() {
    let disk = SimVfs::new(8);
    let (db, model) = dirty_database(&disk);
    db.hooks().compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    assert_eq!(files_holding(&disk.dump(), MARKER), Vec::<String>::new());
    check_state(&db, &model, "after the hook");
    let image = disk.crash_now(CrashKind::PowerLoss);
    drop(db);
    let db = open_in(&image.as_vfs(), &config(Durability::Sync)).unwrap();
    assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut Rng::new(1), "after a power loss");
}

/// With background threads: a directed compaction claims its files like the
/// compactor does, while a writer keeps committing (puts and range
/// deletions over other keys) and the flush and compaction threads run.
#[test]
fn compact_range_runs_beside_background_threads_and_a_writer() {
    let disk = SimVfs::new(10);
    let db = Arc::new(open_in(&disk.as_vfs(), &threads_config(Durability::Sync)).unwrap());
    let mut model = Model::new();
    fill(&db, &mut model, 0, 400);
    let writer = {
        let db = db.clone();
        std::thread::spawn(move || {
            let mut mine = Model::new();
            for round in 0..40u32 {
                for i in 0..10u32 {
                    let mut b = WriteBatch::new();
                    b.put(format!("w-{:05}", round * 10 + i), vec![round as u8; 300]);
                    db.commit(&b).unwrap();
                    apply(&mut mine, &b);
                }
                let mut b = WriteBatch::new();
                b.delete_range(format!("w-{:05}", round * 10 + 3), format!("w-{:05}", round * 10 + 6));
                db.commit(&b).unwrap();
                apply(&mut mine, &b);
            }
            mine
        })
    };
    let mut rewritten = 0;
    for _ in 0..6 {
        let b = range_batch("key-00100", "key-00200");
        db.commit(&b).unwrap();
        apply(&mut model, &b);
        rewritten +=
            db.compact_range(Bound::Included(b"key-00050"), Bound::Excluded(b"key-00250")).unwrap().files_rewritten;
    }
    assert!(rewritten > 0);
    let mine = writer.join().unwrap();
    model.extend(mine);
    db.hooks().wait_idle().unwrap();
    check_state(&db, &model, "after the writer and the directed compactions");
    // Nothing is left to hide inside the range: its tombstones went to the bottom.
    db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    assert_eq!(stat(&db, "range_fragments"), 0);
    check_state(&db, &model, "after the last directed compaction");
}
