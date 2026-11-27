//! A large commit under `Buffered` durability writes no WAL frame: it goes to
//! its table, and is durable once the table is in the MANIFEST. A crash must
//! still leave a prefix of the commits (never a later commit without an
//! earlier one), `sync()` must still make everything durable, read-only
//! handles must read through the sequence gap the commit leaves in the log,
//! and `Sync` durability keeps writing the frame.

mod common;

use std::time::Duration;

use kerno_core::contract::Engine;
use kerno_core::{Durability, WriteBatch};
use kerno_storage::{AccessMode, Config, KernoDb};
use kerno_testkit::harness::{Model, apply};
use kerno_testkit::vfs::{CrashKind, SimVfs};

use common::*;

/// Enough points that a commit is kept as a frozen memtable (`FROZEN_MIN_POINTS`).
const BIG_POINTS: u32 = 33 * 1024;

fn buffered() -> Config {
    Config { write_buffer_bytes: 1 << 20, bulk_without_wal: true, ..threads_config(Durability::Buffered) }
}

fn small(i: u32) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(format!("small-{i:04}"), [i as u8; 100]);
    b
}

fn large(i: u32) -> WriteBatch {
    let mut b = WriteBatch::new();
    for k in 0..BIG_POINTS {
        b.put(format!("big-{i}-{k:06}"), [i as u8; 8]);
    }
    b
}

enum Step {
    Commit(WriteBatch),
    Sync,
    Close,
}

/// Small commits, a large one, more small ones, another large one, `sync()`,
/// one more small commit, close.
fn scenario() -> Vec<Step> {
    let mut s = Vec::new();
    s.extend((0..3).map(|i| Step::Commit(small(i))));
    s.push(Step::Commit(large(1)));
    s.extend((3..6).map(|i| Step::Commit(small(i))));
    s.push(Step::Commit(large(2)));
    s.push(Step::Sync);
    s.push(Step::Commit(small(6)));
    s.push(Step::Close);
    s
}

/// The model after each prefix of the scenario's commits.
fn prefixes(steps: &[Step]) -> Vec<Model> {
    let mut models = vec![Model::new()];
    for step in steps {
        if let Step::Commit(b) = step {
            let mut m = models.last().unwrap().clone();
            apply(&mut m, b);
            models.push(m);
        }
    }
    models
}

/// Runs the scenario on `disk`, stopping at the first error. Returns how many
/// commits were acknowledged before the last `sync()` that returned, and
/// whether the close returned.
fn run(disk: &SimVfs, steps: Vec<Step>) -> (usize, bool) {
    let Ok(db) = open_in(&disk.as_vfs(), &buffered()) else { return (0, false) };
    let (mut done, mut synced) = (0, 0);
    let mut db = Some(db);
    for step in steps {
        let ok = match step {
            Step::Commit(b) => db.as_ref().unwrap().commit(&b).map(|_| done += 1).is_ok(),
            Step::Sync => db.as_ref().unwrap().sync().map(|_| synced = done).is_ok(),
            Step::Close => return (synced, db.take().unwrap().close().is_ok()),
        };
        if !ok {
            break;
        }
    }
    (synced, false)
}

fn check_image(after: &SimVfs, models: &[Model], synced: usize, closed: bool, point: u64) {
    let db = open_in(&after.as_vfs(), &buffered())
        .unwrap_or_else(|e| panic!("crash point {point}: the reopen failed: {e}"));
    let got = model_of(&db);
    let k = models.iter().position(|m| *m == got);
    let k = k.unwrap_or_else(|| panic!("crash point {point}: the reopened state is no prefix of the commits"));
    assert!(k >= synced, "crash point {point}: {k} commits survived, {synced} were synced");
    if closed {
        assert_eq!(k, models.len() - 1, "crash point {point}: the close returned");
    }
    // The reopened database takes commits and reopens again.
    let mut b = WriteBatch::new();
    b.put("after", b"crash");
    db.commit(&b).unwrap();
    db.close().unwrap_or_else(|e| panic!("crash point {point}: close after the reopen: {e}"));
    open_in(&after.as_vfs(), &buffered()).unwrap().close().unwrap();
}

#[test]
fn a_large_buffered_commit_writes_no_frame() {
    let disk = SimVfs::new(1);
    let db = open_in(&disk.as_vfs(), &buffered()).unwrap();
    let mut model = Model::new();
    put(&db, &mut model, "a", b"1");
    let b = large(1);
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    put(&db, &mut model, "c", b"3");
    assert!(stat(&db, "wal_bytes") < 64 << 10, "the large commit wrote no frame");
    assert!(model_of(&db) == model);
    db.sync().unwrap();
    db.close().unwrap();
    let db = open_in(&disk.as_vfs(), &buffered()).unwrap();
    assert!(model_of(&db) == model);
    db.close().unwrap();
}

#[test]
fn a_large_sync_mode_commit_still_writes_its_frame() {
    let disk = SimVfs::new(2);
    let cfg = Config { write_buffer_bytes: 1 << 20, ..threads_config(Durability::Sync) };
    let db = open_in(&disk.as_vfs(), &cfg).unwrap();
    let mut model = Model::new();
    put(&db, &mut model, "a", b"1");
    db.commit(&large(1)).unwrap();
    assert!(stat(&db, "wal_bytes") > u64::from(BIG_POINTS) * 8, "Sync mode writes the large commit's frame");
    // Durable on return, from the log alone.
    let after = disk.crash_now(CrashKind::PowerLoss);
    let db2 = open_in(&after.as_vfs(), &cfg).unwrap();
    apply(&mut model, &large(1));
    assert!(model_of(&db2) == model);
    db2.close().unwrap();
    drop(db);
}

#[test]
fn sync_after_a_large_buffered_commit_makes_it_durable() {
    for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill] {
        let disk = SimVfs::new(3);
        let db = open_in(&disk.as_vfs(), &buffered()).unwrap();
        let mut model = Model::new();
        put(&db, &mut model, "a", b"1");
        let b = large(1);
        db.commit(&b).unwrap();
        apply(&mut model, &b);
        db.sync().unwrap();
        let after = disk.crash_now(kind);
        let db2 = open_in(&after.as_vfs(), &buffered()).unwrap();
        assert!(model_of(&db2) == model, "{kind:?}");
        db2.close().unwrap();
        drop(db);
    }
}

/// The log after a log-less commit continues above it: once its table is in
/// the MANIFEST, recovery accepts the gap and replays what follows.
#[test]
fn commits_after_an_installed_large_commit_replay_across_the_gap() {
    let disk = SimVfs::new(4);
    let db = open_in(&disk.as_vfs(), &buffered()).unwrap();
    let mut model = Model::new();
    put(&db, &mut model, "a", b"1");
    let b = large(1);
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    put(&db, &mut model, "c", b"3");
    put(&db, &mut model, "d", b"4");
    db.sync().unwrap();
    let after = disk.crash_now(CrashKind::PowerLoss);
    let db2 = open_in(&after.as_vfs(), &buffered()).unwrap();
    assert!(model_of(&db2) == model);
    let mut m2 = model.clone();
    put(&db2, &mut m2, "e", b"5");
    db2.close().unwrap();
    let db3 = open_in(&after.as_vfs(), &buffered()).unwrap();
    assert!(model_of(&db3) == m2);
    db3.close().unwrap();
    drop(db);
}

#[test]
fn crash_at_every_step_leaves_a_prefix() {
    let models = prefixes(&scenario());
    // The number of mutating operations of a run without a crash (threads make it vary a little).
    let reference = SimVfs::new(5);
    let start = reference.mutating_ops();
    let (_, closed) = run(&reference, scenario());
    assert!(closed);
    let total = reference.mutating_ops() - start + 20;
    let mut crashed = 0;
    for point in 1..=total {
        for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill] {
            let disk = SimVfs::new(5);
            disk.crash_after(point, kind);
            let (synced, closed) = run(&disk, scenario());
            let Some(after) = disk.take_crash_image() else {
                continue;
            };
            crashed += 1;
            check_image(&after, &models, synced, closed, point);
        }
    }
    assert!(crashed as u64 > total, "the sweep crashed {crashed} times over {total} points");
}

#[test]
fn read_only_handles_see_a_large_commit_once_installed() {
    let disk = SimVfs::new(6);
    let db = open_in(&disk.as_vfs(), &buffered()).unwrap();
    let mut model = Model::new();
    put(&db, &mut model, "a", b"1");
    let ro_cfg = Config { access: AccessMode::ReadOnly, ..buffered() };
    let ro = KernoDb::open(disk.as_vfs(), std::path::Path::new(DIR), &ro_cfg).unwrap();
    // Slow syncs keep the large commit's table from being installed for a while.
    disk.set_sync_latency(Duration::from_millis(300));
    let b = large(1);
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    // Pending or installed: either way no error, and never a later commit without it.
    ro.refresh().unwrap();
    let seen = model_of(&ro);
    assert!(seen.contains_key(b"a".as_slice()));
    let ro2 = KernoDb::open(disk.as_vfs(), std::path::Path::new(DIR), &ro_cfg).unwrap();
    assert!(model_of(&ro2).contains_key(b"a".as_slice()));
    // A later commit waits for the install, then lands in the log above the gap.
    put(&db, &mut model, "c", b"3");
    disk.set_sync_latency(Duration::ZERO);
    ro.refresh().unwrap();
    assert!(model_of(&ro) == model, "the large commit and the one after it are visible");
    let ro3 = KernoDb::open(disk.as_vfs(), std::path::Path::new(DIR), &ro_cfg).unwrap();
    assert!(model_of(&ro3) == model);
    drop((ro, ro2, ro3));
    db.close().unwrap();
}

#[test]
fn bulk_without_wal_is_off_by_default() {
    assert!(!Config::default().bulk_without_wal, "off by default");
}
