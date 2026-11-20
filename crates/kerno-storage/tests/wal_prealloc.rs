//! Preallocated and recycled WALs (`Config::wal_preallocate`, v1.md §6.1 and
//! WAL-R4): the log is `header ‖ zeros` before its first frame and a flushed
//! log becomes the spare for the next one. These tests run the engine inline
//! (deterministic) and check that nothing but the zeros differs for readers:
//! the same data after a clean close and after crashes, recovery that stops
//! at the end of the valid frames, and corruption that is still corruption.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Engine;
use kerno_core::vfs::{OpenMode, Vfs};
use kerno_core::{Error, WriteBatch};
use kerno_storage::testing::{Inline, Profiled};
use kerno_storage::{Background, CompactionConfig, Config, KernoDb, TestConfig};
use kerno_testkit::harness::crash::{commits, run_scenario, scenario};
use kerno_testkit::harness::{Model, apply, read_all};
use kerno_testkit::vfs::{CrashKind, OpKind, SimVfs};
use std::io;

const DIR: &str = "/db";
const WRITE_BUFFER: usize = 16 << 10;

fn config(wal_preallocate: bool) -> Config {
    Config {
        cache_bytes: 1 << 20,
        write_buffer_bytes: WRITE_BUFFER,
        test: TestConfig { background: Background::Inline, db_id: None },
        compaction: CompactionConfig::default(),
        wal_preallocate,
        ..Config::default()
    }
}

fn open(vfs: &Arc<dyn Vfs>, cfg: &Config) -> kerno_core::Result<KernoDb> {
    KernoDb::open(vfs.clone(), Path::new(DIR), cfg)
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

/// `(name, length)` of the engine's files, sorted.
fn files(vfs: &dyn Vfs) -> Vec<(String, u64)> {
    let mut v: Vec<_> = vfs
        .list(Path::new(DIR))
        .unwrap()
        .into_iter()
        .map(|n| {
            let len = vfs.open(&Path::new(DIR).join(&n), OpenMode::Existing).unwrap().len().unwrap();
            (n, len)
        })
        .collect();
    v.sort();
    v
}

fn wals(vfs: &dyn Vfs) -> Vec<(String, u64)> {
    files(vfs).into_iter().filter(|(n, _)| n.ends_with(".wal")).collect()
}

fn put(db: &KernoDb, model: &mut Model, i: u32) -> u64 {
    let mut b = WriteBatch::new();
    b.put(format!("key-{i:05}"), vec![i as u8; 700]);
    let seq = db.commit(&b).unwrap();
    apply(model, &b);
    seq
}

/// Commits until `rotations` memtable switches happened.
fn commit_until_rotations(db: &KernoDb, model: &mut Model, rotations: u64, from: u32) -> u32 {
    let mut i = from;
    while stat(db, "memtable_rotations") < rotations {
        put(db, model, i);
        i += 1;
        assert!(i < from + 2_000, "no {rotations} rotations after {} commits", i - from);
    }
    i
}

#[test]
fn the_log_is_preallocated_and_keeps_its_length_while_commits_fill_it() {
    let vfs = SimVfs::new(1).as_vfs();
    let db = open(&vfs, &config(true)).unwrap();
    let before = wals(&*vfs);
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].1, WRITE_BUFFER as u64, "the WAL is a write buffer of zeros before the first commit");
    let mut model = Model::new();
    for i in 0..10 {
        put(&db, &mut model, i);
    }
    assert!(stat(&db, "wal_bytes") > 7_000);
    assert_eq!(wals(&*vfs), before, "commits overwrite the zeros: the length never changes");
    db.close().unwrap();
}

#[test]
fn without_preallocation_the_log_grows() {
    let vfs = SimVfs::new(1).as_vfs();
    let db = open(&vfs, &config(false)).unwrap();
    let mut model = Model::new();
    let rotations = commit_until_rotations(&db, &mut model, 3, 0);
    assert!(rotations > 0);
    assert_eq!(stat(&db, "wal_recycled"), 0);
    let names: Vec<_> = files(&*vfs).into_iter().map(|(n, _)| n).collect();
    assert!(!names.iter().any(|n| n == "WAL.spare"), "{names:?}");
    let live = wals(&*vfs);
    assert_eq!(live.len(), 1);
    assert!(live[0].1 < WRITE_BUFFER as u64, "a growing log holds only what was written: {live:?}");
    db.close().unwrap();
}

#[test]
fn rotations_take_the_spare_and_flushed_logs_become_the_next_spare() {
    let vfs = SimVfs::new(1).as_vfs();
    let db = open(&vfs, &config(true)).unwrap();
    let mut model = Model::new();
    let end = commit_until_rotations(&db, &mut model, 6, 0);
    // Inline: the first memtable prepared a spare halfway to its rotation (the
    // first log is created by recovery), and the flush of every retired log
    // prepared the next one, so every rotation took a spare.
    assert_eq!(stat(&db, "wal_recycled"), 6);
    let all = files(&*vfs);
    let spare = all.iter().find(|(n, _)| n == "WAL.spare").unwrap_or_else(|| panic!("no spare in {all:?}"));
    assert!(spare.1 >= WRITE_BUFFER as u64);
    // One live log and the spare: the retired logs were recycled, not kept.
    assert_eq!(wals(&*vfs).len(), 1, "{all:?}");
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
    db.close().unwrap();

    // A clean close removes the logs and the spare.
    let names: Vec<_> = files(&*vfs).into_iter().map(|(n, _)| n).collect();
    assert!(!names.iter().any(|n| n.ends_with(".wal") || n == "WAL.spare"), "{names:?}");
    let db = open(&vfs, &config(true)).unwrap();
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model, "{end} commits");
    db.close().unwrap();
}

/// A process kill leaves recycled logs, a spare and the live log on disk.
/// Reopening replays the recycled one, stops at the end of its valid frames
/// (the stale frames of the file's earlier life are gone), and removes the
/// spare.
#[test]
fn recovery_of_a_recycled_log_stops_at_its_last_frame_and_removes_the_spare() {
    for seed in 0..20 {
        let sim = SimVfs::new(seed);
        let vfs = sim.as_vfs();
        let db = open(&vfs, &config(true)).unwrap();
        let mut model = Model::new();
        let end = commit_until_rotations(&db, &mut model, 4, 0);
        // A few more into the recycled live log.
        for i in end..end + 7 {
            put(&db, &mut model, i);
        }
        assert!(files(&*vfs).iter().any(|(n, _)| n == "WAL.spare"));
        let image = sim.crash_now(CrashKind::ProcessKill).as_vfs();
        drop(db);
        assert!(files(&*image).iter().any(|(n, _)| n == "WAL.spare"), "the image holds a spare");

        let db = open(&image, &config(true)).unwrap();
        assert!(files(&*image).iter().all(|(n, _)| n != "WAL.spare"), "recovery removes the leftover spare");
        assert!(read_all(&db.snapshot().unwrap()).unwrap() == model, "seed {seed}");
        assert_eq!(stat(&db, "last_open_clean"), 0);
        db.close().unwrap();
    }
}

/// Power loss with unsynced frames in a recycled log: the data is a prefix of
/// the commits that includes everything acknowledged in `Sync` mode.
#[test]
fn power_loss_in_a_recycled_log_keeps_every_acknowledged_commit() {
    for seed in 0..30 {
        let sim = SimVfs::new(seed);
        let vfs = sim.as_vfs();
        let db = open(&vfs, &config(true)).unwrap();
        let mut model = Model::new();
        let end = commit_until_rotations(&db, &mut model, 3, 0);
        for i in end..end + 5 {
            put(&db, &mut model, i);
        }
        let image = sim.crash_now(CrashKind::PowerLoss).as_vfs();
        drop(db);
        let db = open(&image, &config(true)).unwrap();
        assert!(read_all(&db.snapshot().unwrap()).unwrap() == model, "seed {seed}: Sync commits are durable");
        db.close().unwrap();
    }
}

/// Damage to an early frame of a recycled log, followed by frames that were
/// synced after it, is corruption, as in a log that grew: the zeros after the
/// valid frames and the file's earlier life change nothing.
#[test]
fn damage_in_a_recycled_log_is_corruption() {
    let sim = SimVfs::new(5);
    let vfs = sim.as_vfs();
    let db = open(&vfs, &config(true)).unwrap();
    let mut model = Model::new();
    let end = commit_until_rotations(&db, &mut model, 3, 0);
    for i in end..end + 6 {
        put(&db, &mut model, i);
    }
    assert!(stat(&db, "wal_recycled") > 0);
    let live = wals(&*vfs).pop().unwrap().0;
    let image = sim.crash_now(CrashKind::ProcessKill);
    drop(db);

    // The first frame starts right after the 96-byte header.
    image.corrupt_byte(&Path::new(DIR).join(&live), 96 + 40, 0x10);
    let err = open(&image.as_vfs(), &config(true)).err().expect("damage before synced frames must not open");
    assert!(matches!(err, Error::Corruption(_)), "{err:?}");

    // The same damage on the last (here the only) frame is a torn tail: it
    // cannot be told from the write of the commit in flight.
    let sim = SimVfs::new(6);
    let vfs = sim.as_vfs();
    let db = open(&vfs, &config(true)).unwrap();
    let mut model = Model::new();
    // The commit that made the third rotation is the only frame of the live
    // log.
    commit_until_rotations(&db, &mut model, 3, 0);
    let live = wals(&*vfs).pop().unwrap().0;
    let image = sim.crash_now(CrashKind::ProcessKill);
    drop(db);
    image.corrupt_byte(&Path::new(DIR).join(&live), 96 + 40, 0x10);
    let reopened = open(&image.as_vfs(), &config(true)).unwrap();
    assert!(read_all(&reopened.snapshot().unwrap()).unwrap().len() <= model.len());
    reopened.close().unwrap();
}

/// A leftover spare is never mistaken for a log, whatever it holds, and
/// reopening works without any `Config` agreement: a database that was
/// written with preallocation opens with it off and the other way round.
#[test]
fn preallocation_is_a_property_of_the_handle_not_of_the_database() {
    let sim = SimVfs::new(2);
    let vfs = sim.as_vfs();
    let mut model = Model::new();
    let db = open(&vfs, &config(true)).unwrap();
    let mut next = commit_until_rotations(&db, &mut model, 3, 0);
    let image = sim.crash_now(CrashKind::ProcessKill).as_vfs();
    drop(db);
    // Garbage where a half-written spare would be.
    image.open(&Path::new(DIR).join("WAL.spare"), OpenMode::CreateOrOpen).unwrap().write_at(b"stale", 0).unwrap();
    for prealloc in [false, true, false] {
        let db = open(&image, &config(prealloc)).unwrap();
        assert!(read_all(&db.snapshot().unwrap()).unwrap() == model, "prealloc = {prealloc}");
        for _ in 0..3 {
            put(&db, &mut model, next);
            next += 1;
        }
        db.close().unwrap();
    }
}

/// The scenario every `every_point` sweep uses recycles WALs in the inline
/// profile, so those sweeps cover crashes at every operation of recycling.
#[test]
fn the_crash_sweep_scenario_recycles_logs() {
    type Db = Profiled<Inline>;
    let steps = scenario::<Db>(7);
    let disk = SimVfs::new(7);
    disk.start_trace();
    let opts = kerno_core::contract::Options::small_for_tests();
    let (acked, _) = run_scenario::<Db>(&disk, &steps, &opts);
    assert_eq!(acked, commits(&steps).len() as u64);
    let trace = disk.trace();
    let to_spare = |p: &Option<std::path::PathBuf>| p.as_ref().is_some_and(|p| p.ends_with("WAL.spare"));
    let recycled = trace.iter().filter(|e| e.op.kind == OpKind::Rename && to_spare(&e.op.to)).count();
    let taken = trace.iter().filter(|e| e.op.kind == OpKind::Rename && e.op.path.ends_with("WAL.spare")).count();
    assert!(recycled > 0, "no log was recycled into the spare in {} operations", trace.len());
    assert!(taken > 0, "the spare was never taken by a rotation");
}

/// Space is taken when the log is created, not by the commits that fill it:
/// a full disk fails the open that would create the log, and a commit that
/// fits the preallocation needs no new space.
#[test]
fn a_full_disk_shows_at_creation_not_in_the_commits_that_fit() {
    let disk = SimVfs::new(1);
    let vfs = disk.as_vfs();
    open(&vfs, &config(true)).unwrap().close().unwrap();

    // No room for a write buffer of zeros: the open fails and changes nothing
    // that matters (the next open, with room, works).
    disk.set_space_limit(Some(disk.space_used() + 4096));
    let err = open(&vfs, &config(true)).err().expect("no room for the log");
    assert!(matches!(&err, Error::Io(e) if e.kind() == io::ErrorKind::StorageFull), "{err:?}");
    disk.set_space_limit(None);
    let db = open(&vfs, &config(true)).unwrap();

    // With exactly the space in use, commits inside the log still succeed.
    disk.set_space_limit(Some(disk.space_used()));
    let mut model = Model::new();
    for i in 0..10 {
        put(&db, &mut model, i);
    }
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
    // Closing writes a table, which does need room.
    disk.set_space_limit(None);
    db.close().unwrap();
}

/// The spare is an optimisation: a failure while preparing it is not an
/// error of the engine, leaves no spare and does not poison the handle, and
/// the next flush prepares one again.
#[test]
fn a_failure_preparing_the_spare_is_not_an_engine_failure() {
    let disk = SimVfs::new(4);
    let vfs = disk.as_vfs();
    let db = open(&vfs, &config(true)).unwrap();
    let mut model = Model::new();
    // The first memtable prepares a spare halfway; its first write fails.
    disk.fail_when(|op| op.kind == OpKind::WriteAt && op.path.ends_with("WAL.spare"), 1, None);
    let end = commit_until_rotations(&db, &mut model, 2, 0);
    assert!(disk.fault_injected(), "the spare was never written");
    assert!(files(&*vfs).iter().all(|(n, _)| n != "WAL.spare") || stat(&db, "wal_recycled") > 0);
    let end = commit_until_rotations(&db, &mut model, 6, end);
    assert!(stat(&db, "wal_recycled") > 0, "preparation never recovered after {end} commits");
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
    db.close().unwrap();
    let db = open(&vfs, &config(true)).unwrap();
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
    db.close().unwrap();
}

/// A directory that holds only a leftover spare (and the lock) is a crash
/// during creation, like one with a stale `MANIFEST.tmp` (DIR-2): it is
/// created again and the spare is deleted.
#[test]
fn a_leftover_spare_does_not_make_the_directory_foreign() {
    let disk = SimVfs::new(1);
    let vfs = disk.as_vfs();
    vfs.create_dir_all(Path::new(DIR)).unwrap();
    vfs.open(&Path::new(DIR).join("WAL.spare"), OpenMode::CreateNew).unwrap().write_at(b"half", 0).unwrap();
    let db = open(&vfs, &config(true)).unwrap();
    assert!(files(&*vfs).iter().all(|(n, _)| n != "WAL.spare"));
    db.close().unwrap();
}
