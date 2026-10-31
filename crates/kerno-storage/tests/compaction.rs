//! The condition-1 compaction levers (phase1-design.md §5.4): flush split,
//! the L0 depth metric, L0 priority, partial L0 compactions, L0 trivial
//! moves, the grandparent cut, rate-based slowdown, periodic SST syncs and
//! the M0 profile that turns them all off.
//!
//! Most tests run the engine inline (no threads) and drive it with the
//! `TestHooks`: `flush` writes the memtable to L0 without compacting, and
//! `compact_once` runs exactly one due compaction. Scenarios that need an LSM
//! shape build it in a first session and change the policy in the next one,
//! because the policy is a property of the handle, not of the files.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, WriteBatch};
use kerno_storage::testing::{M0, Profile};
use kerno_storage::{
    Background, CloseBudget, CompactionConfig, Config, KernoDb, L0Metric, SlowdownPolicy, TableInfo, TestConfig,
};
use kerno_testkit::harness::{Model, apply, read_all};
use kerno_testkit::vfs::{CountingVfs, SimVfs};

const DIR: &str = "/db";

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

/// An inline configuration with an unbounded close, so that sessions leave
/// the shape their policy implies.
fn config(write_buffer: usize, compaction: CompactionConfig) -> Config {
    Config {
        cache_bytes: 1 << 20,
        write_buffer_bytes: write_buffer,
        test: TestConfig { background: Background::Inline, db_id: None },
        compaction,
        close_budget: CloseBudget::Unbounded,
        ..Config::default()
    }
}

/// The M0 policy with 4 KiB files and an L1 of 16 KiB.
fn m0() -> CompactionConfig {
    CompactionConfig::m0(4 << 10)
}

fn open(vfs: &Arc<dyn Vfs>, cfg: &Config) -> KernoDb {
    KernoDb::open(vfs.clone(), Path::new(DIR), cfg).unwrap()
}

fn put(db: &KernoDb, model: &mut Model, key: &str, value: &[u8]) {
    let mut b = WriteBatch::new();
    b.put(key, value);
    db.commit(&b).unwrap();
    apply(model, &b);
}

/// Commits `keys` in one batch, each with a value of `len` bytes tagged with
/// `tag`.
fn put_all(db: &KernoDb, model: &mut Model, keys: impl IntoIterator<Item = String>, tag: u8, len: usize) {
    let mut b = WriteBatch::new();
    for k in keys {
        b.put(k, vec![tag; len]);
    }
    db.commit(&b).unwrap();
    apply(model, &b);
}

fn keys(prefix: &str, range: std::ops::Range<u32>) -> impl Iterator<Item = String> {
    let prefix = prefix.to_string();
    range.map(move |i| format!("{prefix}{i:03}"))
}

fn assert_model(db: &KernoDb, model: &Model, what: &str) {
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == *model, "{what}: the contents differ from the model");
}

fn numbers(tables: &[TableInfo]) -> Vec<u64> {
    tables.iter().map(|t| t.number).collect()
}

/// A device whose L1 holds `b000..b119` in several files, after a clean close
/// with the M0 policy. Returns the model of its contents.
fn l1_of_b_keys(seed: u64) -> (Arc<dyn Vfs>, Model) {
    let vfs = SimVfs::new(seed).as_vfs();
    let mut model = Model::new();
    let db = open(&vfs, &config(4 << 10, m0()));
    for chunk in 0..12 {
        put_all(&db, &mut model, keys("b", chunk * 10..chunk * 10 + 10), 1, 100);
    }
    db.close().unwrap();
    let db = open(&vfs, &config(4 << 10, m0()));
    let tables = db.level_tables();
    assert!(tables[0].is_empty() && tables[1].len() >= 3, "setup: L0 {:?}, L1 {:?}", tables[0], tables[1]);
    assert!(tables[2..].iter().all(Vec::is_empty), "setup: everything is in L1");
    drop(db);
    (vfs, model)
}

/// A policy that never compacts on its own: tests run `compact_once` or
/// choose their own limits.
fn never_due(base: CompactionConfig) -> CompactionConfig {
    CompactionConfig { l0_trigger: 1000, l0_slowdown: 1000, l0_stop: 1000, ..base }
}

/// What a flush of a memtable holding `memtable_keys` must produce: the
/// pieces between which an L1 file lies entirely inside the gap.
fn expected_pieces(memtable_keys: &[String], l1: &[TableInfo]) -> Vec<(String, String)> {
    let mut sorted = memtable_keys.to_vec();
    sorted.sort();
    let mut pieces = vec![(sorted[0].clone(), sorted[0].clone())];
    for w in sorted.windows(2) {
        let contained =
            l1.iter().any(|t| t.smallest.as_slice() > w[0].as_bytes() && t.largest.as_slice() < w[1].as_bytes());
        if contained {
            pieces.push((w[1].clone(), w[1].clone()));
        } else {
            pieces.last_mut().unwrap().1 = w[1].clone();
        }
    }
    pieces
}

fn ranges(tables: &[TableInfo]) -> Vec<(String, String)> {
    let mut r: Vec<_> = tables
        .iter()
        .map(|t| (String::from_utf8(t.smallest.clone()).unwrap(), String::from_utf8(t.largest.clone()).unwrap()))
        .collect();
    r.sort();
    r
}

// ------------------------------------------------------------ flush split

#[test]
fn flush_split_outputs_are_disjoint_and_cover_memtable() {
    let (vfs, mut model) = l1_of_b_keys(1);
    let split = CompactionConfig { flush_split: true, ..never_due(m0()) };
    let db = open(&vfs, &config(1 << 20, split));
    let l1 = db.level_tables()[1].clone();

    let mut memtable_keys: Vec<String> =
        keys("a", 0..5).chain(["b035".into(), "b100".into()]).chain(keys("c", 0..3)).collect();
    memtable_keys.extend(["b036".into(), "b101".into()]);
    for chunk in memtable_keys.chunks(3) {
        put_all(&db, &mut model, chunk.iter().cloned(), 2, 50);
    }
    db.hooks().flush().unwrap();

    let l0 = db.level_tables()[0].clone();
    let got = ranges(&l0);
    assert_eq!(got, expected_pieces(&memtable_keys, &l1), "L1: {:?}", ranges(&l1));
    assert!(got.len() >= 2, "the setup must have something to cut: {got:?}");
    // Key-disjoint, and every key of the memtable is in exactly one piece.
    for w in got.windows(2) {
        assert!(w[0].1 < w[1].0, "pieces overlap: {got:?}");
    }
    for k in &memtable_keys {
        let holders = got.iter().filter(|(lo, hi)| lo <= k && k <= hi).count();
        assert_eq!(holders, 1, "key {k} is in {holders} pieces: {got:?}");
    }
    assert_eq!(l0.iter().map(|t| t.entries).sum::<u64>(), memtable_keys.len() as u64);
    assert_model(&db, &model, "after the split flush");
}

#[test]
fn flush_split_cuts_around_contained_l1_files() {
    let cfg = |flush_split| {
        // One L0 file at a time, oldest first; no trivial move, so that the
        // overlap with L1 is what the compaction reports.
        let policy = CompactionConfig {
            flush_split,
            l0_trigger: 1,
            l0_slowdown: 1000,
            l0_stop: 1000,
            l0_partial: true,
            max_compaction_bytes: Some(1),
            ..m0()
        };
        config(1 << 20, policy)
    };
    let mut l1_files_rewritten = Vec::new();
    for flush_split in [true, false] {
        let (vfs, mut model) = l1_of_b_keys(2);
        let db = open(&vfs, &cfg(flush_split));
        let l1_before = db.level_tables()[1].len();
        // `a` and `c` keys with the whole `b` range between them.
        put_all(&db, &mut model, ["a000".to_string(), "a001".into(), "c000".into(), "c001".into()], 3, 60);
        db.hooks().flush().unwrap();
        let l0 = db.level_tables()[0].clone();
        assert_eq!(l0.len(), if flush_split { 2 } else { 1 }, "flush_split = {flush_split}: {:?}", ranges(&l0));
        if flush_split {
            assert_eq!(ranges(&l0), [("a000".into(), "a001".into()), ("c000".into(), "c001".into())]);
        }
        while db.hooks().compact_once().unwrap() {}
        l1_files_rewritten.push(stat(&db, "l0_compaction_l1_files"));
        // The split pieces overlap no L1 file; the single file overlaps all.
        if !flush_split {
            assert_eq!(stat(&db, "l0_compaction_l1_files"), l1_before as u64);
        }
        assert_model(&db, &model, "after compacting");
        db.close().unwrap();
    }
    assert_eq!(l1_files_rewritten[0], 0, "the pieces of a split flush overlap no L1 file");
    assert!(l1_files_rewritten[1] >= 3, "without the split the flush overlaps the whole `b` range");
}

// ------------------------------------------------------------- l0 metric

#[test]
fn l0_depth_counts_overlap_not_files() {
    let policy = |metric| CompactionConfig { l0_metric: metric, ..m0() };
    let flushes = |db: &KernoDb, model: &mut Model, rounds: u32, overlapping: bool| {
        for round in 0..rounds {
            let prefix = if overlapping { "k".to_string() } else { format!("k{round}-") };
            put_all(db, model, keys(&prefix, 0..5), round as u8, 40);
            db.hooks().flush().unwrap();
        }
    };

    // Six files with disjoint ranges: depth 1, nothing is due under the
    // depth metric (below the trigger of 4 even with the file-count
    // backstop), but the file count reaches the trigger of the M0 metric.
    for (metric, due) in [(L0Metric::Depth, false), (L0Metric::Files, true)] {
        let vfs = SimVfs::new(3).as_vfs();
        let (db, mut model) = (open(&vfs, &config(1 << 20, policy(metric))), Model::new());
        flushes(&db, &mut model, 6, false);
        assert_eq!((stat(&db, "l0_files"), stat(&db, "l0_depth")), (6, 1));
        assert_eq!(db.hooks().compact_once().unwrap(), due, "{metric:?}, 6 disjoint files");
    }

    // Four files over the same keys: depth 4, due under both metrics.
    for metric in [L0Metric::Depth, L0Metric::Files] {
        let vfs = SimVfs::new(4).as_vfs();
        let (db, mut model) = (open(&vfs, &config(1 << 20, policy(metric))), Model::new());
        flushes(&db, &mut model, 4, true);
        assert_eq!((stat(&db, "l0_files"), stat(&db, "l0_depth")), (4, 4));
        assert!(db.hooks().compact_once().unwrap(), "{metric:?}, depth 4");
        assert_eq!(stat(&db, "l0_files"), 0);
        assert_model(&db, &model, "after the compaction");
    }

    // Disjoint files cannot pile up without bound: 8 of them are due under
    // the depth metric (the backstop counts two files as one level).
    let vfs = SimVfs::new(5).as_vfs();
    let (db, mut model) = (open(&vfs, &config(1 << 20, policy(L0Metric::Depth))), Model::new());
    flushes(&db, &mut model, 8, false);
    assert_eq!(stat(&db, "l0_depth"), 1);
    assert!(db.hooks().compact_once().unwrap(), "8 disjoint files");
    assert_model(&db, &model, "after the backstop compaction");
}

// -------------------------------------------------------------- priority

#[test]
fn pick_prefers_l0_at_slowdown() {
    for priority in [true, false] {
        // First session: a large L1 under a generous target.
        let vfs = SimVfs::new(6).as_vfs();
        let mut model = Model::new();
        let generous = CompactionConfig { level_base_bytes: Some(1 << 30), ..m0() };
        let db = open(&vfs, &config(4 << 10, generous));
        for chunk in 0..40 {
            put_all(&db, &mut model, keys("g", chunk * 10..chunk * 10 + 10), 1, 150);
        }
        db.close().unwrap();

        // Second session: L1 is far over its new target (score ~5 or more)
        // and L0 holds 8 files, at the slowdown limit (score 2).
        let policy = CompactionConfig {
            level_base_bytes: Some(10 << 10),
            l0_slowdown: 8,
            l0_stop: 100,
            l0_priority: priority,
            ..m0()
        };
        let db = open(&vfs, &config(1 << 20, policy));
        let l1_bytes: u64 = db.level_tables()[1].iter().map(|t| t.size).sum();
        assert!(l1_bytes > 4 * (10 << 10), "setup: L1 holds {l1_bytes} bytes");
        for round in 0..8 {
            put(&db, &mut model, &format!("z{round}"), b"v");
            db.hooks().flush().unwrap();
        }
        assert_eq!(stat(&db, "l0_files"), 8);
        assert!(db.hooks().compact_once().unwrap());
        if priority {
            assert_eq!(stat(&db, "l0_files"), 0, "L0 at the slowdown limit goes first");
            assert_eq!(stat(&db, "l0_compactions") + stat(&db, "trivial_moves"), 1);
        } else {
            assert_eq!(stat(&db, "l0_files"), 8, "the level with the highest score goes first");
        }
        assert_model(&db, &model, "after one compaction");
        db.close().unwrap();
    }
}

// --------------------------------------------------------------- partial

#[test]
fn l0_partial_takes_oldest_suffix_within_budget() {
    // Six L0 files over the same keys, each rewriting every value: a
    // compaction that took a middle or a newer subset would resurrect old
    // values.
    let six_files = || {
        let vfs = SimVfs::new(7).as_vfs();
        let mut model = Model::new();
        let db = open(&vfs, &config(1 << 20, never_due(m0())));
        for round in 0..6u8 {
            put_all(&db, &mut model, keys("p", 0..30), round + 1, 100);
            db.hooks().flush().unwrap();
        }
        db.close().unwrap();
        (vfs, model)
    };
    let (vfs, _) = six_files();
    let db = open(&vfs, &config(1 << 20, never_due(m0())));
    assert_eq!(db.level_tables()[0].len(), 6);
    let size = db.level_tables()[0][0].size;
    drop(db);

    for (budget, taken) in [(2 * size + size / 2, 2usize), (1, 1), (u64::MAX, 6)] {
        let (vfs, model) = six_files();
        let policy = CompactionConfig { l0_trigger: 4, l0_partial: true, max_compaction_bytes: Some(budget), ..m0() };
        let db = open(&vfs, &config(1 << 20, policy));
        let before = numbers(&db.level_tables()[0]);
        assert!(db.hooks().compact_once().unwrap());
        let after = numbers(&db.level_tables()[0]);
        // L0 is newest first, so the oldest suffix is the tail.
        assert_eq!(after, before[..before.len() - taken], "budget {budget}: kept {after:?} of {before:?}");
        assert_model(&db, &model, &format!("budget {budget}"));
        // The rest drains too, still without resurrecting anything.
        while db.hooks().compact_once().unwrap() {}
        assert_model(&db, &model, &format!("budget {budget}, drained"));
    }
}

// ---------------------------------------------------------- trivial move

#[test]
fn l0_trivial_move_when_no_l1_overlap() {
    let policy = |l0_trivial_move| CompactionConfig { l0_trigger: 1, l0_trivial_move, ..m0() };
    for l0_trivial_move in [true, false] {
        let (vfs, mut model) = l1_of_b_keys(8);
        let db = open(&vfs, &config(1 << 20, policy(l0_trivial_move)));
        put_all(&db, &mut model, keys("z", 0..6), 5, 80);
        db.hooks().flush().unwrap();
        let moved = numbers(&db.level_tables()[0]);
        assert_eq!(moved.len(), 1);
        assert!(db.hooks().compact_once().unwrap());
        let tables = db.level_tables();
        assert!(tables[0].is_empty());
        if l0_trivial_move {
            assert_eq!((stat(&db, "trivial_moves"), stat(&db, "compactions")), (1, 0));
            assert!(numbers(&tables[1]).contains(&moved[0]), "the same file moved to L1");
        } else {
            assert_eq!((stat(&db, "trivial_moves"), stat(&db, "compactions")), (0, 1));
            assert!(!numbers(&tables[1]).contains(&moved[0]), "the file was rewritten");
        }
        // A file that overlaps L1 is merged, not moved.
        put(&db, &mut model, "b010", b"again");
        db.hooks().flush().unwrap();
        let trivial = stat(&db, "trivial_moves");
        assert!(db.hooks().compact_once().unwrap());
        assert_eq!(stat(&db, "trivial_moves"), trivial, "an L0 file overlapping L1 is not moved");
        assert_model(&db, &model, "after the compactions");
        db.close().unwrap();
    }
}

// ------------------------------------------------------------ grandparent

#[test]
fn grandparent_overlap_cuts_outputs() {
    // First session: data spread over small L2 files (L1 holds 4 KiB).
    let build = || {
        let vfs = SimVfs::new(9).as_vfs();
        let mut model = Model::new();
        let policy = CompactionConfig { level_base_bytes: Some(4 << 10), ..m0() };
        let db = open(&vfs, &config(4 << 10, policy));
        for chunk in 0..30 {
            put_all(&db, &mut model, keys("g", chunk * 10..chunk * 10 + 10), 1, 100);
        }
        db.close().unwrap();
        (vfs, model)
    };
    let limit = 8 << 10;
    let mut outputs = Vec::new();
    for gp in [Some(limit), Some(u64::MAX)] {
        let (vfs, mut model) = build();
        // Second session: files as large as can be, so that only the
        // grandparent rule can cut an output.
        let policy = CompactionConfig {
            level_base_bytes: Some(1 << 20),
            target_file_size: Some(1 << 20),
            l0_trigger: 1,
            l0_slowdown: 1000,
            l0_stop: 1000,
            grandparent_overlap_bytes: gp,
            ..m0()
        };
        let db = open(&vfs, &config(1 << 20, policy));
        let before = db.level_tables();
        assert!(before[2].len() >= 5, "setup: L2 has {} files", before[2].len());
        put_all(&db, &mut model, (0..300).step_by(3).map(|i| format!("g{i:03}")), 7, 100);
        db.hooks().flush().unwrap();
        assert!(db.hooks().compact_once().unwrap());
        let after = db.level_tables();
        assert!(after[0].is_empty());
        outputs.push(after[1].len());
        if gp == Some(limit) {
            // No output overlaps much more than the limit of L2 (a cut
            // happens at the first key past the limit).
            let biggest = after[2].iter().map(|t| t.size).max().unwrap();
            for t in &after[1] {
                let overlapped: u64 = after[2]
                    .iter()
                    .filter(|g| g.largest >= t.smallest && g.smallest <= t.largest)
                    .map(|g| g.size)
                    .sum();
                assert!(overlapped <= limit + 2 * biggest, "{overlapped} bytes of L2 under one L1 file");
            }
        }
        assert_model(&db, &model, "after the cut compaction");
    }
    assert_eq!(outputs[1], 1, "without the rule, one big output");
    assert!(outputs[0] >= 3, "{} outputs with the rule", outputs[0]);
}

// ------------------------------------------------------------ rate slowdown

#[test]
fn rate_slowdown_delays_before_stop() {
    // L0 piles up (its compaction is never due) past the slowdown limit but
    // far from the stop limit. Every commit from there on waits for its
    // bytes at 1 MiB/s: 8 KiB take 7.8 ms. Nothing ever stalls.
    let policy = CompactionConfig {
        l0_trigger: 100,
        l0_slowdown: 3,
        l0_stop: 100,
        slowdown: SlowdownPolicy::Rate { initial_bytes_per_sec: 1 << 20 },
        ..m0()
    };
    let cfg = Config { test: TestConfig { background: Background::Threads, db_id: None }, ..config(4 << 10, policy) };
    let vfs = SimVfs::new(10).as_vfs();
    let db = open(&vfs, &cfg);
    let value = vec![4u8; 8 << 10];
    let mut b = WriteBatch::new();
    for i in 0..16u8 {
        b.put([i], value.clone());
        db.commit(&b).unwrap();
        b = WriteBatch::new();
        // Let the flush catch up so that L0 grows one file per commit.
        let deadline = Instant::now() + Duration::from_secs(30);
        while stat(&db, "immutable_memtables") > 0 {
            assert!(Instant::now() < deadline, "flush did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let slowed = stat(&db, "slowdown_commits");
    assert!(slowed >= 8, "{slowed} slowed commits, L0 {} files", stat(&db, "l0_files"));
    assert!(
        stat(&db, "slowdown_micros") >= slowed * 7_000,
        "{slowed} commits slept {} us in all",
        stat(&db, "slowdown_micros")
    );
    assert_eq!((stat(&db, "stall_episodes_l0"), stat(&db, "stall_episodes_memtable")), (0, 0));
    db.close().unwrap();
}

// --------------------------------------------------------- bytes_per_sync

#[test]
fn bytes_per_sync_issues_periodic_syncs() {
    let flush_syncs = |bytes_per_sync: Option<u64>| {
        let (vfs, counters) = CountingVfs::wrap(SimVfs::new(11).as_vfs());
        let policy = CompactionConfig { bytes_per_sync, ..never_due(m0()) };
        let cfg = Config { durability: Durability::Buffered, ..config(4 << 20, policy) };
        let db = open(&vfs, &cfg);
        let mut model = Model::new();
        for i in 0..300u32 {
            put(&db, &mut model, &format!("key-{i:05}"), &[i as u8; 4096]);
        }
        let before = counters.snapshot();
        db.hooks().flush().unwrap();
        let used = counters.snapshot().since(&before);
        let sst = db.level_tables()[0][0].size;
        assert!(sst > 1 << 20, "setup: a {sst} bytes table");
        used.file_syncs
    };
    let without = flush_syncs(None);
    let with = flush_syncs(Some(256 << 10));
    assert!(with >= without + 3, "{with} file syncs with bytes_per_sync, {without} without");
}

// -------------------------------------------------------------- M0 profile

#[test]
fn m0_profile_disables_all_levers() {
    let mut cfg = Config::from(&Options::small_for_tests());
    <M0 as Profile>::adjust(&mut cfg);
    assert_eq!(<M0 as Profile>::NAME, "kerno-m0");
    assert_eq!(cfg.compaction, CompactionConfig::m0(16 << 10));
    assert_eq!(cfg.close_budget, CloseBudget::Unbounded);
    let c = &cfg.compaction;
    assert!(!(c.flush_split || c.l0_priority || c.l0_partial || c.l0_trivial_move));
    assert_eq!(c.l0_metric, L0Metric::Files);
    assert_eq!(c.slowdown, SlowdownPolicy::Sleep { micros: 1000 });
    assert_eq!((c.bytes_per_sync, c.max_background_compactions, c.max_subcompactions), (None, 1, 1));
    assert_eq!(c.heap_merge_threshold, usize::MAX);
    assert_eq!(c.grandparent_overlap_bytes, Some(u64::MAX));
    // And the defaults are not M0.
    let d = CompactionConfig::default();
    assert!(d.flush_split && d.l0_partial && d.l0_trivial_move);
    assert_eq!(d.l0_metric, L0Metric::Depth);
    assert_eq!((d.bytes_per_sync, d.bytes_per_writeback), (None, Some(1 << 20)));
    assert!(!d.dynamic_base_level);
    assert_eq!(d.slowdown, SlowdownPolicy::Rate { initial_bytes_per_sec: 16 << 20 });

    // Behaviour: a flush with L1 files in its gap is one table, and L0 is
    // compacted whole and rewritten.
    let (vfs, mut model) = l1_of_b_keys(12);
    let cfg = Config { compaction: CompactionConfig { l0_trigger: 1, ..m0() }, ..config(1 << 20, m0()) };
    let db = open(&vfs, &cfg);
    put_all(&db, &mut model, ["a000".to_string(), "a001".into(), "c000".into(), "c001".into()], 3, 60);
    db.hooks().flush().unwrap();
    assert_eq!(db.level_tables()[0].len(), 1);
    assert!(db.hooks().compact_once().unwrap());
    assert_eq!((stat(&db, "trivial_moves"), stat(&db, "compactions")), (0, 1));
    assert_model(&db, &model, "under the M0 policy");
}

// ------------------------------------------------------ subcompactions

#[test]
fn subcompactions_install_single_manifest_write() {
    // Two L0 files over the whole `b` range (with deletes) compacted into an
    // L1 of several files, with and without the split.
    let run = |max_subcompactions: usize| {
        let (vfs, mut model) = l1_of_b_keys(13);
        let policy = CompactionConfig { l0_trigger: 2, max_subcompactions, ..m0() };
        let db = open(&vfs, &config(1 << 20, policy));
        for round in 0..2u8 {
            put_all(&db, &mut model, (0..120).step_by(2).map(|i| format!("b{i:03}")), 10 + round, 90);
            let mut b = WriteBatch::new();
            for i in (1..120).step_by(7) {
                b.delete(format!("b{i:03}"));
            }
            db.commit(&b).unwrap();
            apply(&mut model, &b);
            db.hooks().flush().unwrap();
        }
        let writes = stat(&db, "manifest_writes");
        assert!(db.hooks().compact_once().unwrap());
        assert_eq!(stat(&db, "manifest_writes"), writes + 1, "one MANIFEST write for the whole compaction");
        assert_eq!((stat(&db, "compactions"), stat(&db, "l0_files")), (1, 0));
        assert_model(&db, &model, &format!("max_subcompactions = {max_subcompactions}"));
        let l1 = db.level_tables()[1].clone();
        for w in l1.windows(2) {
            assert!(w[0].largest < w[1].smallest, "L1 files overlap");
        }
        (stat(&db, "subcompactions"), l1.iter().map(|t| t.entries).sum::<u64>(), stat(&db, "tombstones_dropped"))
    };
    let (parts, entries, dropped) = run(4);
    let (single_parts, single_entries, single_dropped) = run(1);
    assert!(parts >= 2, "the compaction was not split ({parts} parts)");
    assert_eq!(single_parts, 0);
    assert_eq!((entries, dropped), (single_entries, single_dropped), "the split changes what is kept");
}

#[test]
fn close_compaction_uses_close_subcompactions() {
    use kerno_testkit::vfs::{HookAction, HookOp, HookOpKind, HookPoint, HookVfs};
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    // The compaction of the mandatory close floor is split by
    // `close_subcompactions`, not by `max_subcompactions`: count the threads
    // that write compaction output while the session closes.
    let run = |close_subcompactions: usize| {
        let (sim_vfs, mut model) = l1_of_b_keys(13);
        let closing = Arc::new(AtomicBool::new(false));
        let writers = Arc::new(Mutex::new(HashSet::new()));
        let (flag, seen) = (closing.clone(), writers.clone());
        let vfs: Arc<dyn Vfs> = HookVfs::wrap(sim_vfs.clone(), move |op: &HookOp<'_>| {
            if flag.load(Ordering::SeqCst)
                && op.phase == "compact"
                && op.kind == HookOpKind::WriteAt
                && op.point == HookPoint::Before
            {
                seen.lock().unwrap().insert(std::thread::current().id());
            }
            HookAction::Proceed
        });
        let policy = CompactionConfig { l0_trigger: 2, max_subcompactions: 1, close_subcompactions, ..m0() };
        // Background threads: inline mode never uses `close_subcompactions`.
        // One table is flushed before the close and the other by it, so
        // only the close finds L0 at its trigger.
        let cfg =
            Config { test: TestConfig { background: Background::Threads, db_id: None }, ..config(1 << 20, policy) };
        let db = open(&vfs, &cfg);
        for round in 0..2u8 {
            put_all(&db, &mut model, (0..120).step_by(2).map(|i| format!("b{i:03}")), 10 + round, 90);
            if round == 0 {
                db.hooks().flush().unwrap();
            }
        }
        closing.store(true, Ordering::SeqCst);
        db.close().unwrap();
        closing.store(false, Ordering::SeqCst);
        let db = open(&sim_vfs, &config(1 << 20, m0()));
        assert_model(&db, &model, &format!("close_subcompactions = {close_subcompactions}"));
        writers.lock().unwrap().len()
    };
    assert!(run(4) >= 2, "the close compaction was not split");
    assert_eq!(run(1), 1);
}

// ------------------------------------------------- concurrent compactions

#[test]
fn concurrent_compactions_preserve_model() {
    use kerno_testkit::harness::Workload;
    use kerno_testkit::vfs::{HookAction, HookOp, HookOpKind, HookPoint, HookVfs};

    // Compactions write slowly, so that a second one starts while the first
    // is still running. Small levels give every level something to do.
    let slow_compactions = |op: &HookOp<'_>| {
        let writes = matches!(op.kind, HookOpKind::WriteAt | HookOpKind::SyncData);
        if writes && op.phase == "compact" && op.point == HookPoint::Before {
            HookAction::Delay(Duration::from_millis(1))
        } else {
            HookAction::Proceed
        }
    };
    let mut most_at_once = 0;
    for seed in 0..4u64 {
        let sim = SimVfs::new(seed);
        let vfs: Arc<dyn Vfs> = HookVfs::wrap(sim.as_vfs(), slow_compactions);
        let policy = CompactionConfig {
            level_base_bytes: Some(24 << 10),
            level_multiplier: 3,
            max_background_compactions: 3,
            max_subcompactions: 2,
            ..CompactionConfig::default()
        };
        let cfg = Config {
            test: TestConfig { background: Background::Threads, db_id: None },
            close_budget: CloseBudget::default(),
            ..config(8 << 10, policy)
        };
        let db = open(&vfs, &cfg);
        let (mut model, mut w) = (Model::new(), Workload::new(seed));
        for _ in 0..2500 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        db.hooks().wait_idle().unwrap();
        assert_model(&db, &model, &format!("seed {seed}, idle"));
        most_at_once = most_at_once.max(stat(&db, "compactions_concurrent_max"));
        let snap = db.snapshot().unwrap();
        db.close().unwrap();
        // The snapshot outlived the close of its handle and still reads what
        // the compactions replaced.
        assert!(read_all(&snap).unwrap() == model);
        drop(snap);
        let db = open(&vfs, &cfg);
        assert_model(&db, &model, &format!("seed {seed}, reopened"));
        db.close().unwrap();
    }
    assert!(most_at_once >= 2, "no two compactions ever ran at once ({most_at_once})");
}
