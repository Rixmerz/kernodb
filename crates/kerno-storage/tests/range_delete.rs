//! Range tombstones in the engine (phase1-design.md §5.5, v1.md §7.4, §11):
//! reads that consult them in every source, SSTs that hold only them, their
//! fragments cut where a compaction cuts its output, their disappearance at
//! the bottom, the memtable that fills with them, and a crash at every point
//! of a lifecycle that writes them.
//!
//! The engine runs inline (`Background::Inline`) with a 16 KiB write buffer
//! and an unbounded close, so flushes and compactions happen at known
//! moments and the sweeps number their operations the same way each time.

mod common;

use std::ops::Bound;
use std::sync::Arc;

use common::*;
use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{Durability, WriteBatch};
use kerno_storage::testing::{Inline, Profiled};
use kerno_storage::{Config, KernoDb, KernoSnapshot, TableInfo};
use kerno_testkit::harness::crash::{Step, commits, every_matching_point, every_point_steps};
use kerno_testkit::harness::{Model, apply, assert_snapshot_matches, diff, read_all};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, OpInfo, SimVfs};

fn key(i: u32) -> String {
    format!("key-{i:05}")
}

fn range(start: &str, end: &str) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.delete_range(start, end);
    b
}

fn commit(db: &KernoDb, model: &mut Model, b: &WriteBatch) -> u64 {
    let seq = db.commit(b).unwrap();
    apply(model, b);
    seq
}

/// Every key of `0..n` through `get` and the whole database through `scan`,
/// against the model.
fn check(snap: &KernoSnapshot, model: &Model, n: u32, ctx: &str) {
    let all = read_all(snap).unwrap();
    assert!(all == *model, "{ctx}: scan differs from the model: {}", diff(model, &all));
    for i in 0..n {
        let k = key(i);
        assert_eq!(snap.get(k.as_bytes()).unwrap().as_ref(), model.get(k.as_bytes()), "{ctx}: get({k})");
    }
}

fn flush(db: &KernoDb) {
    db.hooks().flush().unwrap();
}

/// Compacts until nothing is due.
fn settle(db: &KernoDb) {
    while db.hooks().compact_once().unwrap() {}
}

/// Gets of the keys of `0..n` at three moments: with the data and the range
/// in different sources, after the range reached an SST, and after every
/// compaction ran.
fn at_every_stage(db: &KernoDb, model: &Model, n: u32, ctx: &str) {
    check(&db.snapshot().unwrap(), model, n, &format!("{ctx}: as committed"));
    flush(db);
    check(&db.snapshot().unwrap(), model, n, &format!("{ctx}: flushed"));
    settle(db);
    check(&db.snapshot().unwrap(), model, n, &format!("{ctx}: compacted"));
}

#[test]
fn get_respects_newer_memtable_range_over_older_point() {
    let disk = SimVfs::new(1);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 20);
    flush(&db);
    // The points are in an SST; the range is in the active memtable.
    let before = db.snapshot().unwrap();
    let before_model = model.clone();
    commit(&db, &mut model, &range(&key(5), &key(15)));
    assert!(db.level_tables()[0].iter().all(|t| t.fragments.is_empty()), "the range must still be in memory");
    let snap = db.snapshot().unwrap();
    for i in 0..20 {
        let want = !(5..15).contains(&i);
        assert_eq!(snap.get(key(i).as_bytes()).unwrap().is_some(), want, "get({})", key(i));
    }
    check(&snap, &model, 20, "range in the memtable");
    check(&before, &before_model, 20, "snapshot before the range");

    // A newer point inside the range survives it; an older one in the
    // memtable does not.
    let mut b = WriteBatch::new();
    b.put(key(7), "again");
    commit(&db, &mut model, &b);
    let mut b = WriteBatch::new();
    b.put(key(100), "fresh");
    commit(&db, &mut model, &b);
    commit(&db, &mut model, &range(&key(90), &key(110)));
    assert!(!model.contains_key(key(100).as_bytes()));
    check(&db.snapshot().unwrap(), &model, 120, "points and ranges in the memtable");

    // The range in an immutable memtable's place, then in an SST above the
    // points, then compacted with them.
    at_every_stage(&db, &model, 120, "after the range");
    check(&before, &before_model, 20, "snapshot before the range, after compaction");
    drop(before);
    db.close().unwrap();
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    check(&db.snapshot().unwrap(), &model, 120, "after reopen");
}

#[test]
fn same_seq_point_wins_over_range() {
    let disk = SimVfs::new(2);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 10);
    // One commit: the range first, then a point inside it. They share a
    // sequence, and the point is not covered (SEM-3).
    let mut b = WriteBatch::new();
    b.delete_range(key(0), key(10)).put(key(4), "kept");
    let seq = commit(&db, &mut model, &b);
    assert_eq!(model.len(), 1);
    check(&db.snapshot().unwrap(), &model, 10, "in the memtable");
    at_every_stage(&db, &model, 10, "same sequence");
    // The reverse order cancels the point: nothing of it is written.
    let mut b = WriteBatch::new();
    b.put(key(6), "gone").delete_range(key(5), key(8));
    assert_eq!(commit(&db, &mut model, &b), seq + 1);
    at_every_stage(&db, &model, 10, "range after the point");
    // A point written after the range, in the next commit, survives it too,
    // and reads at the sequence in between do not see it.
    let snap = db.snapshot().unwrap();
    let mut b = WriteBatch::new();
    b.put(key(6), "later");
    let later = commit(&db, &mut model, &b);
    assert_eq!(snap.seq() + 1, later);
    assert_eq!(snap.get(key(6).as_bytes()).unwrap(), None);
    at_every_stage(&db, &model, 10, "point after the range");
    assert_eq!(db.snapshot().unwrap().get(key(6).as_bytes()).unwrap().as_deref(), Some(&b"later"[..]));
}

#[test]
fn tombstone_only_sst_round_trip() {
    let disk = SimVfs::new(3);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 30);
    flush(&db);
    commit(&db, &mut model, &range(&key(10), &key(20)));
    flush(&db);
    let l0 = db.level_tables()[0].clone();
    assert_eq!(l0.len(), 2, "one SST of points, one of the range alone");
    let tomb = &l0[0];
    assert_eq!((tomb.entries, tomb.fragments.len()), (0, 1), "no points, one fragment");
    assert_eq!(tomb.fragments[0], (key(10).into_bytes(), key(20).into_bytes(), 31));
    assert_eq!(
        (&tomb.smallest[..], &tomb.largest[..], tomb.largest_exclusive),
        (key(10).as_bytes(), key(20).as_bytes(), true)
    );
    check(&db.snapshot().unwrap(), &model, 30, "tombstone-only SST above the points");
    // It survives a crash with the log gone, and a clean close and reopen.
    drop(db);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    check(&db.snapshot().unwrap(), &model, 30, "after a crash");
    db.close().unwrap();
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    check(&db.snapshot().unwrap(), &model, 30, "after a clean close");
    // A range that covers nothing at all in the files still makes a table.
    commit(&db, &mut model, &range("zzz-0", "zzz-1"));
    flush(&db);
    assert!(db.level_tables()[0].iter().any(|t| t.entries == 0 && t.fragments.len() == 1));
    check(&db.snapshot().unwrap(), &model, 30, "range over no key");
}

/// The shape of the tree, for messages.
fn describe(db: &KernoDb) -> String {
    let s = |b: &[u8]| String::from_utf8_lossy(b).trim_start_matches("key-0").to_string();
    let mut out = String::new();
    for (l, ts) in db.level_tables().iter().enumerate().filter(|(_, ts)| !ts.is_empty()) {
        out += &format!("\nL{l}:");
        for t in ts {
            let frags: Vec<String> = t.fragments.iter().map(|(a, b, q)| format!("{}..{}@{q}", s(a), s(b))).collect();
            out += &format!(
                " #{}[{}..{}{} {}pts {frags:?}]",
                t.number,
                s(&t.smallest),
                s(&t.largest),
                if t.largest_exclusive { ")" } else { "]" },
                t.entries
            );
        }
    }
    out
}

/// L1 and below as `(level, table)`, by level then key.
fn lower_tables(db: &KernoDb) -> Vec<(usize, TableInfo)> {
    db.level_tables().into_iter().enumerate().skip(1).flat_map(|(l, ts)| ts.into_iter().map(move |t| (l, t))).collect()
}

/// A database with a wide range tombstone over keys that were put again after
/// it, so that survivors inside the range make the outputs of the
/// compactions that merge it cut in the middle of it. The older keys are in
/// the last level, so that the tombstone has something below to hide from
/// and stays.
fn wide_range_database() -> (SimVfs, KernoDb, Model) {
    let disk = SimVfs::new(4);
    let mut cfg = config(Durability::Sync);
    cfg.compaction.l0_trigger = 2;
    let db = open_in(&disk.as_vfs(), &cfg).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 400);
    db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    commit(&db, &mut model, &range(&key(50), &key(350)));
    for i in (60..340).step_by(2) {
        put(&db, &mut model, &key(i), &[i as u8; 300]);
    }
    flush(&db);
    settle(&db);
    (disk, db, model)
}

#[test]
fn fragments_clipped_at_output_boundaries() {
    let (_disk, db, model) = wide_range_database();
    check(&db.snapshot().unwrap(), &model, 400, "after the compactions");
    let tables = lower_tables(&db);
    let with_pieces: Vec<&(usize, TableInfo)> = tables.iter().filter(|(_, t)| !t.fragments.is_empty()).collect();
    assert!(with_pieces.len() >= 2, "the wide range must be spread over several outputs: {}", describe(&db));
    for (level, t) in &tables {
        for (start, end, _) in &t.fragments {
            assert!(start < end, "L{level} #{}: empty fragment", t.number);
            assert!(
                *start >= t.smallest && end <= &t.largest,
                "L{level} #{}: fragment {:?}..{:?} outside the table's extent {:?}..{:?}",
                t.number,
                String::from_utf8_lossy(start),
                String::from_utf8_lossy(end),
                String::from_utf8_lossy(&t.smallest),
                String::from_utf8_lossy(&t.largest),
            );
        }
    }
}

#[test]
fn exclusive_largest_files_do_not_overlap() {
    let (disk, db, model) = wide_range_database();
    // Neighbours in a level: the first may end exactly where the next
    // begins, if its extent stops short of that key (MAN-S3).
    let mut touching = 0;
    for level in db.level_tables().iter().skip(1) {
        for w in level.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            assert!(
                if a.largest_exclusive { a.largest <= b.smallest } else { a.largest < b.smallest },
                "#{} and #{} overlap",
                a.number,
                b.number
            );
            if a.largest_exclusive && a.largest == b.smallest {
                touching += 1;
                // The shared key belongs to the second table only.
                let snap = db.snapshot().unwrap();
                assert_eq!(snap.get(&b.smallest).unwrap().as_ref(), model.get(&b.smallest), "get at the shared bound");
            }
        }
    }
    assert!(touching > 0, "no output ended exclusively at the first key of the next");
    // The MANIFEST holding them replays (MAN-S3) and the tables open
    // (SST-O7) in a database opened from the files.
    let image = disk.crash_now(CrashKind::ProcessKillKeepPending);
    drop(db);
    let db = open_in(&image.as_vfs(), &config(Durability::Sync)).unwrap();
    check(&db.snapshot().unwrap(), &model, 400, "after reopen");
}

#[test]
fn range_tombstone_dropped_at_bottom() {
    let disk = SimVfs::new(5);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 200);
    flush(&db);
    // The points go to the bottom. A range over part of them, newer, stays a
    // fragment as long as some file below overlaps it...
    db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    assert_eq!(stat(&db, "range_fragments"), 0);
    commit(&db, &mut model, &range(&key(20), &key(120)));
    flush(&db);
    settle(&db);
    assert!(stat(&db, "range_fragments") > 0, "points below the range keep its fragment");
    assert_eq!(stat(&db, "range_tombstones_dropped"), 0);
    check(&db.snapshot().unwrap(), &model, 200, "range above the points");
    // ...and goes with the compaction that reaches them.
    db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    assert_eq!(stat(&db, "range_fragments"), 0, "no file is left to hide anything from");
    assert!(stat(&db, "range_tombstones_dropped") > 0);
    assert!(stat(&db, "covered_points_dropped") >= 100, "the points under the range were not written again");
    check(&db.snapshot().unwrap(), &model, 200, "range dropped");
    let bottom = db.level_tables();
    assert!(bottom[..6].iter().all(Vec::is_empty), "everything is in the last level");
    assert_eq!(bottom[6].iter().map(|t| t.entries).sum::<u64>(), 100);
}

#[test]
fn memtable_rotates_at_4096_range_tombstones() {
    let cfg = Config { write_buffer_bytes: 64 << 20, ..config(Durability::Buffered) };
    let disk = SimVfs::new(6);
    let db = KernoDb::open(disk.as_vfs(), std::path::Path::new(DIR), &cfg).unwrap();
    let mut model = Model::new();
    let mut b = WriteBatch::new();
    b.put("a", "1");
    commit(&db, &mut model, &b);
    for i in 0..4095u32 {
        commit(&db, &mut model, &range("k", &format!("k{i:05}x")));
    }
    assert_eq!(stat(&db, "memtable_rotations"), 0, "4096 tombstones fit");
    // The 4096th goes in; the memtable is full now, and the commit after
    // that one starts a new one.
    commit(&db, &mut model, &range("k", "kz"));
    assert_eq!(stat(&db, "memtable_rotations"), 0);
    let mut b = WriteBatch::new();
    b.put("b", "2");
    commit(&db, &mut model, &b);
    assert_eq!(stat(&db, "memtable_rotations"), 1, "the memtable rotated at 4096 range tombstones");
    assert!(stat(&db, "memtable_bytes") < 4096, "the new memtable starts empty");
    check(&db.snapshot().unwrap(), &model, 0, "after the rotation");
    // One commit with more tombstones than that fills the memtable too.
    let mut big = WriteBatch::new();
    for i in 0..5000u32 {
        big.delete_range(format!("m{i:05}"), format!("m{i:05}z"));
    }
    commit(&db, &mut model, &big);
    let mut b = WriteBatch::new();
    b.put("c", "3");
    commit(&db, &mut model, &b);
    assert_eq!(stat(&db, "memtable_rotations"), 2);
    flush(&db);
    check(&db.snapshot().unwrap(), &model, 0, "after the flush");
    assert!(stat(&db, "range_fragments") > 0);
}

#[test]
fn bloom_negative_does_not_skip_range_check() {
    let disk = SimVfs::new(7);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 50);
    flush(&db);
    db.compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
    // A table of only the range: its bloom filter has no key, so it answers
    // "absent" for all of them, and must not be taken for "no tombstone".
    commit(&db, &mut model, &range(&key(10), &key(30)));
    flush(&db);
    let l0 = db.level_tables()[0].clone();
    assert_eq!((l0.len(), l0[0].entries), (1, 0));
    let negatives = stat(&db, "bloom_negatives");
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.get(key(20).as_bytes()).unwrap(), None, "the older point is hidden");
    assert!(stat(&db, "bloom_negatives") > negatives, "the filter did answer for the tombstone table");
    assert!(snap.get(key(5).as_bytes()).unwrap().is_some());
    assert!(snap.get(key(30).as_bytes()).unwrap().is_some(), "the end is exclusive");
    check(&snap, &model, 50, "bloom negatives");
}

type Db = Profiled<Inline>;

fn scenario_with_ranges(seed: u64) -> Vec<Step<Db>> {
    ranged_scenario::<Db>(
        seed,
        Arc::new(|db: &Db| db.hooks().compact_range(Bound::Unbounded, Bound::Unbounded)),
        Arc::new(|db: &Db| db.hooks().checkpoint()),
    )
}

#[test]
fn range_delete_crash_at_every_point() {
    let steps = scenario_with_ranges(7);
    assert!(
        commits(&steps).iter().filter(|b| b.has_range_deletes()).count() >= 5,
        "the scenario must write range deletions"
    );
    for (kind, durability) in [
        (CrashKind::PowerLoss, Durability::Sync),
        (CrashKind::ProcessKill, Durability::Sync),
        (CrashKind::ProcessKill, Durability::Buffered),
        (CrashKind::ProcessKillKeepPending, Durability::Sync),
    ] {
        every_point_steps::<Db>(kind, durability, &|_| {}, &steps);
    }
    // Aimed at the operations of the directed compaction and the checkpoint
    // after it, for the kinds of crash that lose different things.
    let filter: kerno_testkit::harness::crash::OpFilter =
        Arc::new(|op: &OpInfo| op.phase == "compact_range" || op.phase == "checkpoint");
    for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill] {
        every_matching_point::<Db>(kind, Durability::Sync, &steps, filter.clone());
    }
}

/// A crash that leaves the files of a flush and a directed compaction behind
/// recovers the database a model of the commits describes, and the range
/// tombstones among them still hide what they hid.
#[test]
fn recovery_keeps_range_tombstones_from_every_source() {
    let steps = scenario_with_ranges(11);
    let batches = commits(&steps);
    let disk = SimVfs::new(11);
    let opts = Options::small_for_tests();
    let (acked, attempted) = kerno_testkit::harness::crash::run_scenario::<Db>(&disk, &steps, &opts);
    assert_eq!((acked, attempted), (batches.len() as u64, batches.len() as u64));
    let mut model = Model::new();
    for b in &batches {
        apply(&mut model, b);
    }
    let image = disk.crash_now(CrashKind::PowerLoss);
    let db: Db = kerno_testkit::harness::open(&image.as_vfs(), &opts).unwrap();
    assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut Rng::new(1), "after the scenario");
}

/// A handle that only reads replays the ranges of the live WAL in memory and
/// hides what they hide, over the files, writing nothing.
#[test]
fn read_only_open_replays_range_deletions() {
    let disk = SimVfs::new(8);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 60);
    flush(&db);
    commit(&db, &mut model, &range(&key(10), &key(40)));
    let mut b = WriteBatch::new();
    b.put(key(20), "again").delete_range(key(50), key(55));
    commit(&db, &mut model, &b);
    let image = disk.crash_now(CrashKind::ProcessKillKeepPending);
    drop(db);
    let ro = Config { access: kerno_storage::AccessMode::ReadOnly, ..config(Durability::Sync) };
    let before = image.mutating_ops();
    let db = KernoDb::open(image.as_vfs(), std::path::Path::new(DIR), &ro).unwrap();
    assert!(db.is_read_only().is_some());
    check(&db.snapshot().unwrap(), &model, 60, "read-only, ranges in the log");
    drop(db);
    assert_eq!(image.mutating_ops(), before, "a read-only open wrote");
}
