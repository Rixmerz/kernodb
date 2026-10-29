//! The follow-up levers of ADR-0001 condition 1 (`docs/testing/f1-decision-cond1.md`
//! §3, levers 4 and 5): L0 compacted below an empty L1 (`dynamic_base_level`),
//! the min-overlap file picker (`file_picker`) and writeback requests of the
//! SST writers (`bytes_per_writeback`).
//!
//! The engine runs inline (no threads) on `SimVfs` and is driven with the
//! `TestHooks`: `flush` writes the memtable to L0 without compacting and
//! `compact_once` runs exactly one due compaction.

use std::path::Path;
use std::sync::Arc;

use kerno_core::WriteBatch;
use kerno_core::contract::Engine;
use kerno_core::vfs::Vfs;
use kerno_storage::{Background, CloseBudget, CompactionConfig, Config, FilePicker, KernoDb, TableInfo, TestConfig};
use kerno_testkit::harness::{Model, apply, read_all};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{OpKind, SimVfs};

const DIR: &str = "/db";

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

/// Inline, with an unbounded close, so that sessions leave the shape their
/// policy implies.
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

/// The M0 policy with 4 KiB files, an L1 of 16 KiB and two L0 files to
/// compact.
fn base_policy() -> CompactionConfig {
    CompactionConfig { l0_trigger: 2, ..CompactionConfig::m0(4 << 10) }
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

fn keys(prefix: &str, range: std::ops::Range<u32>) -> impl Iterator<Item = String> {
    let prefix = prefix.to_string();
    range.map(move |i| format!("{prefix}{i:03}"))
}

fn assert_model(db: &KernoDb, model: &Model, what: &str) {
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == *model, "{what}: the contents differ from the model");
}

fn files_per_level(db: &KernoDb) -> Vec<usize> {
    db.level_tables().iter().map(Vec::len).collect()
}

fn ranges_overlap(a: &[TableInfo]) -> bool {
    let mut sorted: Vec<_> = a.iter().collect();
    sorted.sort_by(|x, y| x.smallest.cmp(&y.smallest));
    sorted.windows(2).any(|w| w[0].largest >= w[1].smallest)
}

// ------------------------------------------------------ dynamic base level

#[test]
fn l0_is_compacted_below_an_empty_l1_when_the_option_is_on() {
    for dynamic in [true, false] {
        let vfs = SimVfs::new(1).as_vfs();
        let db = open(&vfs, &config(1 << 20, CompactionConfig { dynamic_base_level: dynamic, ..base_policy() }));
        let mut model = Model::new();
        for round in 0..2u8 {
            put_all(&db, &mut model, keys("k", 0..40), round, 100);
            db.hooks().flush().unwrap();
        }
        assert!(db.hooks().compact_once().unwrap());
        let files = files_per_level(&db);
        assert_eq!(files[0], 0, "dynamic {dynamic}: L0 is empty");
        if dynamic {
            assert_eq!(files[1], 0, "L1 stays empty: {files:?}");
            assert!(files[2] > 0, "the output is in L2: {files:?}");
            assert_eq!(stat(&db, "l0_base_level_skips"), 1);
        } else {
            assert!(files[1] > 0 && files[2] == 0, "the output is in L1: {files:?}");
            assert_eq!(stat(&db, "l0_base_level_skips"), 0);
        }
        assert_model(&db, &model, &format!("dynamic {dynamic}"));
    }
}

#[test]
fn compactions_below_l1_merge_with_the_files_already_there_and_survive_a_reopen() {
    let vfs = SimVfs::new(2).as_vfs();
    let cfg = config(1 << 20, CompactionConfig { dynamic_base_level: true, ..base_policy() });
    let db = open(&vfs, &cfg);
    let mut model = Model::new();
    for round in 0..6u8 {
        // The same keys every round, plus some that are new each time: the
        // compaction has to replace the old versions in L2, not stack them.
        put_all(&db, &mut model, keys("k", 0..40).chain(keys(&format!("n{round}-"), 0..10)), round, 100);
        db.hooks().flush().unwrap();
        if round % 2 == 1 {
            assert!(db.hooks().compact_once().unwrap(), "round {round}");
            let l2 = &db.level_tables()[2];
            assert!(!ranges_overlap(l2), "round {round}: L2 files overlap");
            assert_model(&db, &model, &format!("round {round}"));
        }
    }
    assert_eq!(stat(&db, "l0_base_level_skips"), 3);
    assert_eq!(files_per_level(&db)[1], 0);
    // The MANIFEST records the files at L2 and the engine reads them back.
    db.close().unwrap();
    let db = open(&vfs, &cfg);
    assert_eq!(files_per_level(&db)[1], 0);
    assert_model(&db, &model, "after a clean reopen");
}

#[test]
fn a_populated_l1_is_never_bypassed() {
    // L1 holds the `b` keys (built with the M0 policy); then L0 holds newer
    // versions of some of them and keys on either side. Writing that L0 below
    // L1 would put the newer versions under the older ones.
    let vfs = SimVfs::new(3).as_vfs();
    let mut model = Model::new();
    let db = open(&vfs, &config(4 << 10, CompactionConfig::m0(4 << 10)));
    for chunk in 0..12 {
        put_all(&db, &mut model, keys("b", chunk * 10..chunk * 10 + 10), 1, 100);
    }
    db.close().unwrap();

    let policy = CompactionConfig { dynamic_base_level: true, ..base_policy() };
    let db = open(&vfs, &config(1 << 20, policy));
    assert!(files_per_level(&db)[1] > 0, "setup: L1 holds the `b` keys");
    put_all(&db, &mut model, keys("b", 20..60).chain(keys("a", 0..5)).chain(keys("c", 0..5)), 7, 100);
    db.hooks().flush().unwrap();
    put_all(&db, &mut model, keys("b", 30..50), 8, 100);
    db.hooks().flush().unwrap();
    while db.hooks().compact_once().unwrap() {}
    assert_eq!(stat(&db, "l0_base_level_skips"), 0, "L1 was not empty");
    assert_model(&db, &model, "newer versions win");
}

#[test]
fn l0_goes_to_l1_once_l2_is_too_big_for_a_compaction_to_take_the_trigger() {
    // 20 KiB per compaction: two 4 KiB L0 files fit with an L2 of up to 12 KiB.
    let policy = CompactionConfig { dynamic_base_level: true, max_compaction_bytes: Some(20 << 10), ..base_policy() };
    let vfs = SimVfs::new(4).as_vfs();
    let db = open(&vfs, &config(1 << 20, policy));
    let mut model = Model::new();
    let mut skips_when_l1_filled = None;
    for round in 0..40u32 {
        // Distinct keys: the database only grows.
        put_all(&db, &mut model, keys(&format!("r{round:02}-"), 0..30), round as u8, 100);
        db.hooks().flush().unwrap();
        while db.hooks().compact_once().unwrap() {}
        let skips = stat(&db, "l0_base_level_skips");
        if skips_when_l1_filled.is_none() && files_per_level(&db)[1] > 0 {
            skips_when_l1_filled = Some(skips);
        }
        if let Some(at) = skips_when_l1_filled {
            // The option never skips a populated L1.
            assert_eq!(skips, at, "round {round}: skipped L1 although it holds files");
        }
    }
    let at = skips_when_l1_filled.expect("L1 never filled up");
    assert!(at >= 1, "the first compactions went below the empty L1");
    assert_model(&db, &model, "after growing past the gate");
    assert!(!ranges_overlap(&db.level_tables()[1]) && !ranges_overlap(&db.level_tables()[2]));
    db.close().unwrap();
    let db = open(&vfs, &config(1 << 20, base_policy()));
    assert_model(&db, &model, "after a reopen with the option off");
}

#[test]
fn a_clean_close_compacts_into_l1_even_when_it_is_empty() {
    // Merging the L0 files with all of L2 would cost the close far more than
    // merging them with L1.
    let vfs = SimVfs::new(8).as_vfs();
    let cfg = config(1 << 20, CompactionConfig { dynamic_base_level: true, ..base_policy() });
    let db = open(&vfs, &cfg);
    let mut model = Model::new();
    for round in 0..2u8 {
        put_all(&db, &mut model, keys("k", 0..40), round, 100);
        db.hooks().flush().unwrap();
    }
    assert_eq!(files_per_level(&db)[1..], [0; 6], "setup: only L0 holds files");
    db.close().unwrap();
    let db = open(&vfs, &cfg);
    let files = files_per_level(&db);
    assert!(files[0] == 0 && files[1] > 0 && files[2] == 0, "the close compacted into L1: {files:?}");
    assert_model(&db, &model, "after the close");
}

#[test]
fn flush_split_cuts_around_the_files_of_the_level_l0_compacts_into() {
    // L2 holds the `b` keys in several files and L1 is empty: with the option
    // on, a flush that has `b` keys between `a` and `c` ones is cut around the
    // L2 files that lie entirely in the gap.
    let build = |split_on_l2: bool| {
        let vfs = SimVfs::new(5).as_vfs();
        let mut model = Model::new();
        let policy = CompactionConfig { dynamic_base_level: true, l0_trigger: 1, ..CompactionConfig::m0(4 << 10) };
        let db = open(&vfs, &config(4 << 10, policy));
        for chunk in 0..12 {
            put_all(&db, &mut model, keys("b", chunk * 10..chunk * 10 + 10), 1, 100);
        }
        db.hooks().flush().unwrap();
        while db.hooks().compact_once().unwrap() {}
        // Not a clean close: that one compacts into L1 (see `compact_for_close`).
        drop(db);
        let policy =
            CompactionConfig { dynamic_base_level: split_on_l2, flush_split: true, ..CompactionConfig::m0(4 << 10) };
        let db = open(&vfs, &config(1 << 20, policy));
        let files = files_per_level(&db);
        assert!(files[1] == 0 && files[2] >= 3, "setup: the `b` keys are in L2 only: {files:?}");
        put_all(&db, &mut model, ["a000".to_string(), "a001".into(), "c000".into(), "c001".into()], 3, 60);
        db.hooks().flush().unwrap();
        let pieces = db.level_tables()[0].len();
        assert_model(&db, &model, "after the flush");
        pieces
    };
    assert_eq!(build(true), 2, "cut around the L2 files");
    assert_eq!(build(false), 1, "L1 is empty: there is nothing to cut around");
}

// ------------------------------------------------------- writeback requests

/// What a session of flushes and compactions asks of the device.
fn trace_of(bytes_per_writeback: Option<u64>) -> (Vec<(OpKind, String, &'static str)>, u64) {
    let sim = SimVfs::new(7);
    sim.start_trace();
    let vfs = sim.as_vfs();
    let policy = CompactionConfig { bytes_per_writeback, bytes_per_sync: None, ..base_policy() };
    let db = open(&vfs, &config(1 << 20, policy));
    let mut model = Model::new();
    for round in 0..4u8 {
        put_all(&db, &mut model, keys("k", 0..60), round, 100);
        db.hooks().flush().unwrap();
        db.hooks().compact_once().unwrap();
    }
    assert_model(&db, &model, "after the session");
    let hints = stat(&db, "sst_writeback_hints");
    db.close().unwrap();
    let ops = sim.trace().into_iter().map(|t| (t.op.kind, t.op.path.display().to_string(), t.op.phase)).collect();
    (ops, hints)
}

#[test]
fn writeback_requests_come_with_the_sst_writes_and_change_nothing_else() {
    let (without, none) = trace_of(None);
    let (with, hints) = trace_of(Some(1));
    assert_eq!(none, 0);
    assert!(hints >= 4, "{hints} requests in a session of flushes and compactions");
    let advisory = |k: &OpKind| matches!(k, OpKind::StartWriteback | OpKind::WaitWriteback);
    let asked: Vec<_> = with.iter().filter(|(k, ..)| *k == OpKind::StartWriteback).collect();
    assert_eq!(asked.len() as u64, hints);
    for (_, path, phase) in with.iter().filter(|(k, ..)| advisory(k)) {
        assert!(path.ends_with(".sst"), "a request for {path}");
        assert!(["flush", "compact", "close"].contains(phase), "a request in phase {phase:?}");
    }
    // Every other operation, in the same order, is the session without them.
    let rest: Vec<_> = with.into_iter().filter(|(k, ..)| !advisory(k)).collect();
    assert!(rest == without, "the requests changed the other operations");
}

#[test]
fn an_sst_writeback_request_never_makes_data_durable() {
    // A power loss right after the requests loses the same unsynced bytes as
    // one without them: the requests are not syncs. The WAL of a `Buffered`
    // handle is the unsynced data here.
    let run = |bytes_per_writeback: Option<u64>| {
        let mut lost = 0;
        for seed in 0..32 {
            let sim = SimVfs::new(seed);
            let vfs = sim.as_vfs();
            let policy = CompactionConfig { bytes_per_writeback, ..base_policy() };
            let cfg = Config { durability: kerno_core::Durability::Buffered, ..config(1 << 20, policy) };
            let db = open(&vfs, &cfg);
            let mut model = Model::new();
            put_all(&db, &mut model, keys("k", 0..30), 1, 100);
            db.hooks().flush().unwrap();
            put_all(&db, &mut model, keys("m", 0..30), 2, 100);
            let after = sim.crash_now(kerno_testkit::vfs::CrashKind::PowerLoss);
            let reopened = KernoDb::open(after.as_vfs(), Path::new(DIR), &cfg).unwrap();
            let got = read_all(&reopened.snapshot().unwrap()).unwrap();
            if got != model {
                lost += 1;
            }
        }
        lost
    };
    let (with, without) = (run(Some(1)), run(None));
    assert!(without > 0, "the unsynced commits must be lost sometimes, or the test proves nothing");
    assert_eq!(with, without, "writeback requests changed what a power loss keeps");
}

// -------------------------------------------------- randomized model check

/// Random histories against the model with all three levers on and tiny
/// levels, so that the output level alternates between L1 and L2, files are
/// picked by overlap, and every table write asks for writeback.
#[test]
fn random_histories_match_the_model_with_all_levers_on() {
    let mut hints = 0;
    for seed in 0..24u64 {
        let policy = CompactionConfig {
            dynamic_base_level: true,
            file_picker: FilePicker::MinOverlap,
            bytes_per_writeback: Some(1),
            l0_trigger: 2,
            l0_slowdown: 6,
            l0_stop: 10,
            level_base_bytes: Some(6 << 10),
            level_multiplier: 3,
            target_file_size: Some(2 << 10),
            max_compaction_bytes: Some(10 << 10),
            grandparent_overlap_bytes: Some(2 << 10),
            l0_partial: true,
            l0_trivial_move: true,
            flush_split: true,
            ..CompactionConfig::m0(4 << 10)
        };
        let cfg = config(4 << 10, policy);
        let vfs = SimVfs::new(seed).as_vfs();
        let mut db = Some(open(&vfs, &cfg));
        let mut model = Model::new();
        let mut rng = Rng::new(seed);
        for step in 0..260 {
            let ctx = format!("seed {seed}, step {step}");
            let d = db.as_ref().unwrap();
            match rng.below(10) {
                0..=5 => {
                    let mut b = WriteBatch::new();
                    for _ in 0..rng.range(1, 8) {
                        let k = format!("key-{:03}", rng.below(90));
                        if rng.chance(1, 5) {
                            b.delete(k);
                        } else {
                            let len = rng.range(20, 160) as usize;
                            b.put(k, rng.bytes(len));
                        }
                    }
                    d.commit(&b).unwrap();
                    apply(&mut model, &b);
                }
                6 => d.hooks().flush().unwrap(),
                7 | 8 => {
                    d.hooks().compact_once().unwrap();
                }
                _ => {
                    if rng.chance(1, 2) {
                        db.take().unwrap().close().unwrap();
                    } else {
                        drop(db.take());
                    }
                    db = Some(open(&vfs, &cfg));
                }
            }
            let d = db.as_ref().unwrap();
            for (n, level) in d.level_tables().iter().enumerate().skip(1) {
                assert!(!ranges_overlap(level), "{ctx}: L{n} files overlap");
            }
            if step % 20 == 19 {
                assert_model(d, &model, &ctx);
            }
        }
        assert_model(db.as_ref().unwrap(), &model, &format!("seed {seed}, end"));
        hints += stat(db.as_ref().unwrap(), "sst_writeback_hints");
    }
    assert!(hints > 0, "no table of any history asked for writeback");
}
