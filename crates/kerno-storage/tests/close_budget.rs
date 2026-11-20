//! The close budget (phase1-design.md §5.4, C-10): `close` always brings the
//! L0 metric below `l0_trigger`, then compacts one more step at a time while
//! the byte and time budgets last, and reports what it did.
//!
//! The scenarios run inline with byte-only budgets (`max_duration: None`),
//! so every close does the same work in every run.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Engine;
use kerno_core::vfs::{Vfs, dir_size};
use kerno_core::{Durability, WriteBatch};
use kerno_storage::testing::{Inline, Profile, Profiled};
use kerno_storage::{Background, CloseBudget, CloseReport, CompactionConfig, Config, KernoDb, TestConfig};
use kerno_testkit::harness::crash;
use kerno_testkit::harness::{Model, Workload, apply, read_all};
use kerno_testkit::vfs::{CrashKind, SimVfs};

const DIR: &str = "/db";

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

fn bounded(max_bytes: u64) -> CloseBudget {
    CloseBudget::Bounded { max_bytes, max_duration: None }
}

fn config(write_buffer: usize, compaction: CompactionConfig, close_budget: CloseBudget) -> Config {
    Config {
        cache_bytes: 1 << 20,
        write_buffer_bytes: write_buffer,
        test: TestConfig { background: Background::Inline, db_id: None },
        compaction,
        close_budget,
        ..Config::default()
    }
}

fn open(vfs: &Arc<dyn Vfs>, cfg: &Config) -> KernoDb {
    KernoDb::open(vfs.clone(), Path::new(DIR), cfg).unwrap()
}

fn put_all(db: &KernoDb, model: &mut Model, keys: impl IntoIterator<Item = String>, tag: u8, len: usize) {
    let mut b = WriteBatch::new();
    for k in keys {
        b.put(k, vec![tag; len]);
    }
    db.commit(&b).unwrap();
    apply(model, &b);
}

fn key_range(from: u32, to: u32) -> impl Iterator<Item = String> {
    (from..to).map(|i| format!("key-{i:04}"))
}

/// Level targets far below the data, so that closing leaves compaction work
/// at every level: L1 holds 8 KiB, each level below four times the last.
fn small_levels() -> CompactionConfig {
    CompactionConfig { level_base_bytes: Some(8 << 10), level_multiplier: 4, ..CompactionConfig::default() }
}

/// A device with compaction debt that costs bytes to pay: its keys are spread
/// over the deeper levels (a first session with small level targets), and
/// then rewritten under a huge L1 target, so that L1 holds a second version
/// of everything, overlapping all of L2 and below.
fn indebted(seed: u64) -> (Arc<dyn Vfs>, Model) {
    let vfs = SimVfs::new(seed).as_vfs();
    let mut model = Model::new();
    let rounds = |db: &KernoDb, model: &mut Model, tag: u8| {
        for chunk in 0..100 {
            put_all(db, model, key_range(chunk * 10, chunk * 10 + 10), tag, 120);
        }
    };
    let db = open(&vfs, &config(4 << 10, small_levels(), CloseBudget::Unbounded));
    rounds(&db, &mut model, 1);
    db.close().unwrap();
    let generous = CompactionConfig { level_base_bytes: Some(1 << 30), ..small_levels() };
    let db = open(&vfs, &config(4 << 10, generous, CloseBudget::Unbounded));
    rounds(&db, &mut model, 2);
    db.close().unwrap();
    (vfs, model)
}

/// Opens `indebted` under `small_levels` and closes it with `budget`.
fn close_with(seed: u64, budget: CloseBudget) -> (Arc<dyn Vfs>, Model, CloseReport) {
    let (vfs, mut model) = indebted(seed);
    let db = open(&vfs, &config(4 << 10, small_levels(), budget));
    put_all(&db, &mut model, key_range(0, 20), 3, 120);
    let report = db.close().unwrap();
    (vfs, model, report)
}

#[test]
fn close_respects_byte_budget() {
    let (_, _, full) = close_with(1, CloseBudget::Unbounded);
    assert!(!full.budget_exhausted);
    assert!(full.compacted_bytes > 200 << 10, "setup: the unbounded close compacted {} bytes", full.compacted_bytes);
    assert!(full.flushed_bytes > 0);

    let max_bytes = full.compacted_bytes / 4;
    let (vfs, model, r) = close_with(1, bounded(max_bytes));
    assert!(r.budget_exhausted, "{r:?}");
    assert!(r.compacted_bytes < full.compacted_bytes, "{r:?} vs {full:?}");
    // A compaction that starts under the budget may finish over it, by at
    // most one compaction (here, one file of a level and the files it
    // overlaps below).
    assert!(r.compacted_bytes <= max_bytes + (128 << 10), "{} bytes for a budget of {max_bytes}", r.compacted_bytes);
    assert_eq!(r.flushed_bytes, full.flushed_bytes);

    // What the close left is a complete, readable database.
    let db = open(&vfs, &config(4 << 10, small_levels(), CloseBudget::Unbounded));
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
}

#[test]
fn close_with_zero_budget_still_bounds_l0() {
    let vfs = SimVfs::new(2).as_vfs();
    let mut model = Model::new();
    // Eight overlapping L0 files and no compaction while the session runs.
    let idle = CompactionConfig { l0_trigger: 1000, l0_slowdown: 1000, l0_stop: 1000, ..CompactionConfig::default() };
    let db = open(&vfs, &config(1 << 20, idle, CloseBudget::Unbounded));
    for round in 0..8u8 {
        put_all(&db, &mut model, key_range(0, 30), round, 100);
        db.hooks().flush().unwrap();
    }
    db.close().unwrap();

    let policy = CompactionConfig::default();
    let db = open(&vfs, &config(1 << 20, policy.clone(), bounded(0)));
    assert_eq!(stat(&db, "l0_depth"), 8);
    let report = db.close().unwrap();
    assert!(report.l0_depth < policy.l0_trigger as u64, "{report:?}");
    assert!(report.l0_files <= 2 * policy.l0_trigger as u64, "{report:?}");
    assert!(report.compacted_bytes > 0, "{report:?}");

    let db = open(&vfs, &config(1 << 20, policy, bounded(0)));
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
}

#[test]
fn close_floor_takes_only_the_oldest_files_it_needs() {
    // Eight L0 files over the same keys: with `l0_partial`, the mandatory
    // compaction moves the five oldest and keeps the three newest (depth 3,
    // below a trigger of 4); without it, all of them.
    for (l0_partial, kept) in [(true, 3usize), (false, 0)] {
        let vfs = SimVfs::new(3).as_vfs();
        let mut model = Model::new();
        let idle =
            CompactionConfig { l0_trigger: 1000, l0_slowdown: 1000, l0_stop: 1000, ..CompactionConfig::default() };
        let db = open(&vfs, &config(1 << 20, idle, CloseBudget::Unbounded));
        for round in 0..8u8 {
            put_all(&db, &mut model, key_range(0, 30), round, 100);
            db.hooks().flush().unwrap();
        }
        db.close().unwrap();

        let policy = CompactionConfig { l0_trigger: 4, l0_partial, ..CompactionConfig::default() };
        let db = open(&vfs, &config(1 << 20, policy.clone(), bounded(0)));
        let before: Vec<u64> = db.level_tables()[0].iter().map(|t| t.number).collect();
        assert_eq!(before.len(), 8);
        db.close().unwrap();

        let db = open(&vfs, &config(1 << 20, policy, bounded(0)));
        let after: Vec<u64> = db.level_tables()[0].iter().map(|t| t.number).collect();
        // L0 is newest first.
        assert_eq!(after, before[..kept], "l0_partial = {l0_partial}");
        assert!(read_all(&db.snapshot().unwrap()).unwrap() == model, "l0_partial = {l0_partial}");
    }
}

#[test]
fn l0_bounded_across_100_short_sessions() {
    let vfs = SimVfs::new(3).as_vfs();
    let mut model = Model::new();
    let mut w = Workload::new(3);
    let policy = CompactionConfig::default();
    // The worst budget: nothing beyond the mandatory L0 work.
    let cfg = config(4 << 10, policy.clone(), bounded(0));
    let db = open(&vfs, &cfg);
    for _ in 0..400 {
        let b = w.batch(&model);
        db.commit(&b).unwrap();
        apply(&mut model, &b);
    }
    db.close().unwrap();
    for session in 0..100 {
        let db = open(&vfs, &cfg);
        let b = w.batch(&model);
        db.commit(&b).unwrap();
        apply(&mut model, &b);
        let report = db.close().unwrap();
        assert!(report.l0_depth < policy.l0_trigger as u64, "session {session}: {report:?}");
        assert!(report.l0_files <= 2 * policy.l0_trigger as u64, "session {session}: {report:?}");
    }
    let db = open(&vfs, &cfg);
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
}

#[test]
fn space_bounded_across_100_short_sessions() {
    let vfs = SimVfs::new(4).as_vfs();
    let mut model = Model::new();
    let mut w = Workload::new(4);
    // The default budget, without the time limit.
    let cfg = config(4 << 10, CompactionConfig::default(), CloseBudget::default().deterministic());
    for session in 0..100 {
        let db = open(&vfs, &cfg);
        for _ in 0..3 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        db.close().unwrap();
        let live: usize = model.iter().map(|(k, v)| k.len() + v.len()).sum();
        let size = dir_size(&*vfs, Path::new(DIR)).unwrap();
        assert!(size <= 4 * live as u64 + (1 << 20), "session {session}: {size} bytes on disk for {live} live bytes");
    }
}

/// Inline, with a close budget that is always spent: every close leaves
/// compaction work behind.
struct Budgeted;

impl Profile for Budgeted {
    const NAME: &'static str = "kerno-test-budgeted";

    fn adjust(cfg: &mut Config) {
        cfg.test.background = Background::Inline;
        cfg.close_budget = bounded(0);
        // The final flush of a close is always compacted into L1, which then
        // is over its target.
        cfg.compaction.l0_trigger = 1;
        cfg.compaction.level_base_bytes = Some(1 << 10);
        cfg.compaction.level_multiplier = 2;
    }
}

#[test]
fn budgeted_close_crash_at_every_point() {
    type Db = Profiled<Budgeted>;
    assert_eq!(<Db as Engine>::name(), "kerno-test-budgeted");
    // The sweep means something only if its closes leave debt behind.
    let steps = crash::scenario::<Db>(7);
    let disk = SimVfs::new(7);
    let opts = kerno_core::contract::Options::small_for_tests();
    crash::run_scenario::<Db>(&disk, &steps, &opts);
    let db = <Db as Engine>::open(disk.as_vfs(), Path::new(DIR), &opts).unwrap();
    assert!(stat(db.db(), "pending_compaction_bytes") > 0, "scenario A leaves no compaction debt");
    drop(db);
    // Scenario A closes twice; a crash anywhere in a budgeted close (power
    // loss or process death) must recover every acknowledged commit.
    crash::every_point::<Db>(CrashKind::PowerLoss, Durability::Sync);
    crash::every_point::<Db>(CrashKind::ProcessKill, Durability::Sync);
    // The default inline profile is the control.
    crash::every_point::<Profiled<Inline>>(CrashKind::ProcessKill, Durability::Sync);
}

#[test]
fn compaction_debt_resumes_after_open() {
    let (vfs, model, report) = close_with(5, bounded(1));
    assert!(report.budget_exhausted, "{report:?}");
    let db = open(&vfs, &config(4 << 10, small_levels(), bounded(1)));
    // The format records no debt: the scoring finds it again.
    let pending = stat(&db, "pending_compaction_bytes");
    assert!(pending > 0, "the budgeted close left nothing to do");
    let mut steps = 0;
    while db.hooks().compact_once().unwrap() {
        steps += 1;
    }
    assert!(steps > 0);
    assert_eq!(stat(&db, "pending_compaction_bytes"), 0);
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
    // Nothing left to resume: a close now has no debt to leave.
    let report = db.close().unwrap();
    assert!(!report.budget_exhausted && report.compacted_bytes == 0, "{report:?}");
}
