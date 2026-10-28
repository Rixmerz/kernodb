//! SAD §17.6: "the same sequence of operations produces the same logical
//! state in every candidate". Runs one random history (puts, overwrites,
//! deletes, large values, long keys, clean reopens) through every engine
//! and compares full contents and bounded scans at each checkpoint.
//!
//! `differential_range_deletes` does the same with range deletions in the
//! histories of two seeds, and also observes snapshots taken halfway between
//! checkpoints.
//! `LogEngine` implements them natively and is the oracle, and so does
//! `kerno`; the engines that reject them (the spikes and the production
//! engines) run through `kerno_testkit::emulated::Emulated`, which turns each
//! range into point deletions under a commit mutex.
//!
//! `differential_rmw` does it with read-modify-write transactions (what a
//! transaction writes depends on what it read) against a model oracle; the
//! engines run through `Emulated<E>` until they have transactions of their
//! own.

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kerno_testkit::emulated::Emulated;
use spike_core::engine::{Txn, TxnEngine};
use spike_core::harness::{Model, Workload, WorkloadCfg, apply, read_all};
use spike_core::rng::Rng;
use spike_core::vfs::{SimVfs, StdVfs};
use spike_core::{Engine, Error, Op, Options, Snapshot, Vfs, WriteBatch};

const COMMITS: usize = 500;
const CHECKPOINT_EVERY: usize = 50;

/// Contents and some bounded scans at a checkpoint.
#[derive(PartialEq, Debug)]
struct Observation {
    seq: Option<u64>,
    contents: Model,
    scans: Vec<Vec<(Vec<u8>, Vec<u8>)>>,
}

fn history(seed: u64) -> Vec<WriteBatch> {
    let mut w = Workload::new(seed);
    w.key_space = 1500;
    let mut model = Model::new();
    (0..COMMITS)
        .map(|_| {
            let b = w.batch(&model);
            apply(&mut model, &b);
            b
        })
        .collect()
}

fn scan_ranges(seed: u64) -> Vec<(Vec<u8>, Vec<u8>, usize)> {
    let mut rng = Rng::new(seed);
    (0..20)
        .map(|_| {
            let mut a = format!("key-{:08}", rng.below(1500)).into_bytes();
            let mut b = format!("key-{:08}", rng.below(1500)).into_bytes();
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            (a, b, rng.range(1, 60) as usize)
        })
        .collect()
}

fn observe<S: Snapshot>(s: &S, ranges: &[(Vec<u8>, Vec<u8>, usize)], with_seq: bool) -> Observation {
    Observation {
        seq: with_seq.then(|| s.seq()),
        contents: read_all(s).unwrap(),
        scans: ranges
            .iter()
            .map(|(a, b, limit)| s.scan(Bound::Included(a), Bound::Excluded(b), *limit).unwrap())
            .collect(),
    }
}

/// Runs the history; `persistent_seq` is false for reference engines,
/// whose adapters only track the commit sequence in memory.
fn run<E: Engine>(vfs: Arc<dyn Vfs>, dir: &Path, batches: &[WriteBatch], persistent_seq: bool) -> Vec<Observation> {
    let opts = Options::small_for_tests();
    let ranges = scan_ranges(1);
    let mut out = Vec::new();
    let mut db = E::open(vfs.clone(), dir, &opts).unwrap();
    for (i, b) in batches.iter().enumerate() {
        db.commit(b).unwrap();
        if (i + 1) % CHECKPOINT_EVERY == 0 {
            out.push(observe(&db.snapshot().unwrap(), &ranges, persistent_seq));
            db.close().unwrap();
            db = E::open(vfs.clone(), dir, &opts).unwrap();
            let after = observe(&db.snapshot().unwrap(), &ranges, persistent_seq);
            assert_eq!(after, *out.last().unwrap(), "{}: state changed across a clean reopen", E::name());
        }
    }
    db.close().unwrap();
    out
}

fn sim<E: Engine>(batches: &[WriteBatch]) -> Vec<Observation> {
    run::<E>(SimVfs::new(1).as_vfs(), Path::new("/db"), batches, true)
}

fn temp_dir<E: Engine>() -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("kerno-diff-{}-{}-{nanos}", E::name(), std::process::id()))
}

fn real<E: Engine>(batches: &[WriteBatch]) -> Vec<Observation> {
    let dir = temp_dir::<E>();
    let out = run::<E>(StdVfs::shared(), &dir, batches, false);
    std::fs::remove_dir_all(&dir).unwrap();
    out
}

#[test]
fn same_history_same_state_in_every_engine() {
    let batches = history(2026);
    let mut expected = Vec::new();
    let mut model = Model::new();
    let ranges = scan_ranges(1);
    for (i, b) in batches.iter().enumerate() {
        apply(&mut model, b);
        if (i + 1) % CHECKPOINT_EVERY == 0 {
            let scans: Vec<Vec<(Vec<u8>, Vec<u8>)>> = ranges
                .iter()
                .map(|(a, z, limit)| {
                    model
                        .range::<Vec<u8>, _>((Bound::Included(a), Bound::Excluded(z)))
                        .take(*limit)
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<Vec<_>>()
                })
                .collect();
            expected.push((i as u64 + 1, model.clone(), scans));
        }
    }

    let mut results: BTreeMap<&str, Vec<Observation>> = BTreeMap::new();
    results.insert("log", sim::<spike_core::reference::LogEngine>(&batches));
    #[cfg(feature = "candidates")]
    {
        results.insert("btree", sim::<spike_btree::BTreeEngine>(&batches));
        results.insert("lsm", sim::<spike_lsm::LsmEngine>(&batches));
    }
    #[cfg(feature = "kerno")]
    results.insert("kerno", sim::<kerno_storage::KernoDb>(&batches));
    results.insert("redb", real::<spike_bench::adapters::RedbEngine>(&batches));
    results.insert("fjall", real::<spike_bench::adapters::FjallEngine>(&batches));
    results.insert("sqlite", real::<spike_bench::adapters::SqliteEngine>(&batches));

    for (engine, observations) in &results {
        assert_eq!(observations.len(), expected.len(), "{engine}: wrong number of checkpoints");
        for (obs, (seq, contents, scans)) in observations.iter().zip(&expected) {
            if let Some(s) = obs.seq {
                assert_eq!(s, *seq, "{engine}: commit sequence differs at checkpoint {seq}");
            }
            assert!(obs.contents == *contents, "{engine}: contents differ from the model at commit {seq}");
            assert!(obs.scans == *scans, "{engine}: bounded scans differ from the model at commit {seq}");
        }
    }
}

/// Share of the random operations of `differential_range_deletes` that are
/// range deletions (in thousandths).
const RANGE_PER_MILLE: u32 = 60;

fn range_history(seed: u64) -> Vec<WriteBatch> {
    let mut w = Workload::with_config(seed, WorkloadCfg { range_delete_per_mille: RANGE_PER_MILLE });
    w.key_space = 1500;
    let mut model = Model::new();
    (0..COMMITS)
        .map(|_| {
            let b = w.batch(&model);
            apply(&mut model, &b);
            b
        })
        .collect()
}

/// The model's observation after each prefix of `batches` whose length
/// satisfies `at`.
fn expected(batches: &[WriteBatch], at: impl Fn(usize) -> bool) -> Vec<Observation> {
    let ranges = scan_ranges(1);
    let mut model = Model::new();
    let mut out = Vec::new();
    for (i, b) in batches.iter().enumerate() {
        apply(&mut model, b);
        if at(i + 1) {
            let scans = ranges
                .iter()
                .map(|(a, z, limit)| {
                    model
                        .range::<Vec<u8>, _>((Bound::Included(a), Bound::Excluded(z)))
                        .take(*limit)
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<Vec<_>>()
                })
                .collect();
            out.push(Observation { seq: Some(i as u64 + 1), contents: model.clone(), scans });
        }
    }
    out
}

/// Compares every engine's observations with the expected ones (the
/// sequence only where the engine persists it).
fn assert_all_match(results: &BTreeMap<&str, Vec<Observation>>, expected: &[Observation], what: &str) {
    for (engine, observations) in results {
        assert_eq!(observations.len(), expected.len(), "{engine}: wrong number of {what}");
        for (obs, want) in observations.iter().zip(expected) {
            let seq = want.seq.unwrap();
            if let Some(s) = obs.seq {
                assert_eq!(s, seq, "{engine}: commit sequence differs in the {what} at commit {seq}");
            }
            assert!(
                obs.contents == want.contents,
                "{engine}: contents differ from the model in the {what} at commit {seq}"
            );
            assert!(
                obs.scans == want.scans,
                "{engine}: bounded scans differ from the model in the {what} at commit {seq}"
            );
        }
    }
}

/// Like [`run`], and also takes a snapshot halfway between checkpoints and
/// observes it again at the next checkpoint, after the commits that
/// followed it (range deletions included) and right before the reopen.
/// Returns (checkpoint observations, older snapshot observations).
fn run_with_old_snapshots<E: Engine>(
    vfs: Arc<dyn Vfs>,
    dir: &Path,
    batches: &[WriteBatch],
    persistent_seq: bool,
) -> (Vec<Observation>, Vec<Observation>) {
    let opts = Options::small_for_tests();
    let ranges = scan_ranges(1);
    let (mut out, mut old_out) = (Vec::new(), Vec::new());
    let mut db = E::open(vfs.clone(), dir, &opts).unwrap();
    let mut old: Option<E::Snapshot> = None;
    for (i, b) in batches.iter().enumerate() {
        db.commit(b).unwrap();
        if (i + 1) % CHECKPOINT_EVERY == CHECKPOINT_EVERY / 2 {
            old = Some(db.snapshot().unwrap());
        }
        if (i + 1) % CHECKPOINT_EVERY == 0 {
            out.push(observe(&db.snapshot().unwrap(), &ranges, persistent_seq));
            old_out.push(observe(&old.take().expect("taken halfway"), &ranges, persistent_seq));
            db.close().unwrap();
            db = E::open(vfs.clone(), dir, &opts).unwrap();
            let after = observe(&db.snapshot().unwrap(), &ranges, persistent_seq);
            assert_eq!(after, *out.last().unwrap(), "{}: state changed across a clean reopen", E::name());
        }
    }
    db.close().unwrap();
    (out, old_out)
}

type Observed = (Vec<Observation>, Vec<Observation>);

fn sim_old<E: Engine>(batches: &[WriteBatch]) -> Observed {
    run_with_old_snapshots::<E>(SimVfs::new(1).as_vfs(), Path::new("/db"), batches, true)
}

fn real_old<E: Engine>(batches: &[WriteBatch]) -> Observed {
    let dir = temp_dir::<E>();
    let out = run_with_old_snapshots::<E>(StdVfs::shared(), &dir, batches, false);
    std::fs::remove_dir_all(&dir).unwrap();
    out
}

/// Histories of `differential_range_deletes`: 2027, and 1, whose history
/// has a batch made only of empty ranges (commit 162), which `Emulated<E>`
/// must still commit with a sequence of its own.
const RANGE_SEEDS: [u64; 2] = [2027, 1];

#[test]
fn differential_range_deletes() {
    let mut only_empty_ranges = 0;
    for seed in RANGE_SEEDS {
        let batches = range_history(seed);
        only_empty_ranges += batches
            .iter()
            .filter(|b| b.ops().iter().all(|op| matches!(op, Op::DeleteRange { start, end } if start >= end)))
            .count();
        differential_range_deletes_for(seed, &batches);
    }
    assert!(only_empty_ranges > 0, "no history has a batch made only of empty ranges");
}

fn differential_range_deletes_for(seed: u64, batches: &[WriteBatch]) {
    let ranges = batches.iter().flat_map(|b| b.ops()).filter(|op| matches!(op, Op::DeleteRange { .. })).count();
    assert!(ranges >= 100, "seed {seed}: the history must exercise range deletions ({ranges})");

    let mut results: BTreeMap<&str, Observed> = BTreeMap::new();
    // The oracle: native range deletions.
    results.insert("log", sim_old::<spike_core::reference::LogEngine>(batches));
    // Native range tombstones: through the memtable, flushes (the tiny test
    // buffer makes many) and compactions, across the reopens.
    #[cfg(feature = "kerno")]
    {
        results.insert("kerno", sim_old::<kerno_storage::KernoDb>(batches));
        results.insert(
            "kerno-inline",
            sim_old::<kerno_storage::testing::Profiled<kerno_storage::testing::Inline>>(batches),
        );
    }
    #[cfg(feature = "candidates")]
    {
        results.insert("btree", sim_old::<Emulated<spike_btree::BTreeEngine>>(batches));
        results.insert("lsm", sim_old::<Emulated<spike_lsm::LsmEngine>>(batches));
    }
    results.insert("redb", real_old::<Emulated<spike_bench::adapters::RedbEngine>>(batches));
    results.insert("fjall", real_old::<Emulated<spike_bench::adapters::FjallEngine>>(batches));
    results.insert("sqlite", real_old::<Emulated<spike_bench::adapters::SqliteEngine>>(batches));

    let (checkpoints, older): (BTreeMap<_, _>, BTreeMap<_, _>) =
        results.into_iter().map(|(engine, (now, old))| ((engine, now), (engine, old))).unzip();
    assert_all_match(
        &checkpoints,
        &expected(batches, |n| n % CHECKPOINT_EVERY == 0),
        &format!("checkpoints of seed {seed}"),
    );
    assert_all_match(
        &older,
        &expected(batches, |n| n % CHECKPOINT_EVERY == CHECKPOINT_EVERY / 2),
        &format!("snapshots taken halfway of seed {seed}"),
    );
}

/// Batches whose ranges are all empty, and ranges that start at the empty
/// key, through `Emulated<E>`: every batch is a commit with the next
/// sequence (WAL-C3) and the state follows the model. No batch here stores
/// the empty key, so the rewrite must never hand the engine an operation on
/// it, which fjall rejects with a panic.
fn emulated_edge_ranges<E: Engine>(vfs: Arc<dyn Vfs>, dir: &Path) {
    let db = Emulated::<E>::open(vfs, dir, &Options::small_for_tests()).unwrap();
    let mut model = Model::new();
    let mut seq = 0;
    let mut check = |b: &WriteBatch, what: &str| {
        seq += 1;
        assert_eq!(db.commit(b).unwrap(), seq, "{}: {what}", E::name());
        apply(&mut model, b);
        assert_eq!(read_all(&db.snapshot().unwrap()).unwrap(), model, "{}: {what}", E::name());
    };
    let batches: [(&[(&str, &str)], &str); 8] = [
        (&[("x", "x")], "only an empty range, on an empty database"),
        (&[("", "")], "the empty range at the empty key"),
        (&[("", "a")], "a range from the empty key that covers nothing"),
        (&[], "keys"),
        (&[("q", "q"), ("", "")], "only empty ranges, with keys"),
        (&[("", "b")], "a range from the empty key over existing keys"),
        (&[("b", "b")], "an empty range at an existing key"),
        (&[("", "z")], "everything"),
    ];
    for (ranges, what) in batches {
        let mut b = WriteBatch::new();
        if ranges.is_empty() {
            b.put("\0", "nul").put("a", "1").put("b", "2").put("q", "3");
        }
        for (start, end) in ranges {
            b.delete_range(*start, *end);
        }
        check(&b, what);
    }
    assert!(model.is_empty());
    db.close().unwrap();
}

#[test]
fn emulated_engines_commit_empty_ranges_and_ranges_from_the_empty_key() {
    emulated_edge_ranges::<spike_core::reference::LogEngine>(SimVfs::new(1).as_vfs(), Path::new("/db"));
    #[cfg(feature = "candidates")]
    {
        emulated_edge_ranges::<spike_btree::BTreeEngine>(SimVfs::new(1).as_vfs(), Path::new("/db"));
        emulated_edge_ranges::<spike_lsm::LsmEngine>(SimVfs::new(1).as_vfs(), Path::new("/db"));
    }
    edge_ranges_on_real_fs::<spike_bench::adapters::RedbEngine>();
    edge_ranges_on_real_fs::<spike_bench::adapters::FjallEngine>();
    edge_ranges_on_real_fs::<spike_bench::adapters::SqliteEngine>();
}

fn edge_ranges_on_real_fs<E: Engine>() {
    let dir = temp_dir::<Emulated<E>>();
    emulated_edge_ranges::<E>(StdVfs::shared(), &dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Engines without native range deletions reject them up front: nothing is
/// applied, no sequence is consumed, and the handle keeps working.
fn rejects_range_deletes<E: Engine>(vfs: Arc<dyn Vfs>, dir: &Path) {
    let db = E::open(vfs, dir, &Options::small_for_tests()).unwrap();
    let mut b = WriteBatch::new();
    b.put("a", "1");
    assert_eq!(db.commit(&b).unwrap(), 1);
    for (start, end) in [("a", "z"), ("q", "q")] {
        let mut b = WriteBatch::new();
        b.put("b", "2").delete_range(start, end).put("c", "3");
        let r = db.commit(&b);
        assert!(matches!(r, Err(Error::InvalidArgument(_))), "{}: expected InvalidArgument, got {r:?}", E::name());
    }
    let s = db.snapshot().unwrap();
    assert_eq!(s.seq(), 1, "{}: a rejected batch consumed a sequence", E::name());
    assert_eq!(read_all(&s).unwrap().len(), 1, "{}: a rejected batch was applied", E::name());
    assert_eq!(s.get(b"a").unwrap().as_deref(), Some(&b"1"[..]), "{}", E::name());
    drop(s);
    let mut b = WriteBatch::new();
    b.put("d", "4");
    assert_eq!(db.commit(&b).unwrap(), 2, "{}: the handle must keep working", E::name());
    db.close().unwrap();
}

fn rejects_on_real_fs<E: Engine>() {
    let dir = temp_dir::<E>();
    rejects_range_deletes::<E>(StdVfs::shared(), &dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn engines_without_range_deletes_reject_them() {
    #[cfg(feature = "candidates")]
    {
        rejects_range_deletes::<spike_btree::BTreeEngine>(SimVfs::new(1).as_vfs(), Path::new("/db"));
        rejects_range_deletes::<spike_lsm::LsmEngine>(SimVfs::new(1).as_vfs(), Path::new("/db"));
    }
    rejects_on_real_fs::<spike_bench::adapters::RedbEngine>();
    rejects_on_real_fs::<spike_bench::adapters::FjallEngine>();
    rejects_on_real_fs::<spike_bench::adapters::SqliteEngine>();
}

// ---------------------------------------------------------------------------
// Read-modify-write transactions
// ---------------------------------------------------------------------------

/// A transaction of `differential_rmw`: what it writes depends on what it
/// reads, so an engine that shows a transaction a wrong state (its own
/// writes, a range it deleted, a stale value) writes different data.
#[derive(Clone, Debug)]
enum Program {
    /// `ctr/k += 1`.
    Increment { k: u64 },
    /// `acct/a += amount`.
    Deposit { acct: u64, amount: u64 },
    /// Moves `amount` between two accounts if the first has it, else aborts.
    Transfer { from: u64, to: u64, amount: u64 },
    /// Sums `ctr/lo..ctr/hi`, stores the sum under `sum/n`, deletes the range
    /// and writes the sum back to `ctr/lo`, observing the range after each
    /// step.
    Rollup { lo: u64, hi: u64, n: usize },
    /// Random puts, deletes and range deletions over `blob/…` with scans in
    /// between.
    Churn { seed: u64 },
    /// Writes, observes them, and aborts.
    Abandon { seed: u64 },
    /// Only reads: an empty transaction, which commits without a sequence.
    Observe { lo: u64, hi: u64 },
}

const RMW_PROGRAMS: usize = 300;
const RMW_SEEDS: [u64; 2] = [2028, 7];

fn rmw_history(seed: u64) -> Vec<Program> {
    let mut rng = Rng::new(seed);
    (0..RMW_PROGRAMS)
        .map(|n| match rng.below(20) {
            0..=5 => Program::Increment { k: rng.below(12) },
            6..=8 => Program::Deposit { acct: rng.below(6), amount: rng.range(50, 500) },
            9..=12 => {
                let from = rng.below(6);
                Program::Transfer { from, to: (from + 1 + rng.below(5)) % 6, amount: rng.range(1, 300) }
            }
            13 | 14 => {
                let lo = rng.below(10);
                Program::Rollup { lo, hi: lo + rng.range(2, 4), n }
            }
            15 | 16 => Program::Churn { seed: rng.next_u64() },
            17 => Program::Abandon { seed: rng.next_u64() },
            _ => {
                let lo = rng.below(10);
                Program::Observe { lo, hi: lo + rng.range(1, 4) }
            }
        })
        .collect()
}

fn ctr(k: u64) -> Vec<u8> {
    format!("ctr/{k:03}").into_bytes()
}

fn acct(a: u64) -> Vec<u8> {
    format!("acct/{a:03}").into_bytes()
}

fn blob(i: u64) -> Vec<u8> {
    format!("blob/{i:03}").into_bytes()
}

fn num(v: Option<Vec<u8>>) -> u64 {
    v.map_or(0, |v| u64::from_le_bytes(v.try_into().expect("8-byte number")))
}

/// What a program observed, in order: the value of each `get`, the rows of
/// each `scan`.
type Trace = Vec<Option<Vec<u8>>>;

fn see_scan<T: Txn>(
    t: &T,
    start: Bound<&[u8]>,
    end: Bound<&[u8]>,
    limit: usize,
    trace: &mut Trace,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let rows = t.scan(start, end, limit).unwrap();
    trace.push(Some(format!("{rows:?}").into_bytes()));
    rows
}

fn see_get<T: Txn>(t: &T, key: &[u8], trace: &mut Trace) -> Option<Vec<u8>> {
    let v = t.get(key).unwrap();
    trace.push(v.clone());
    v
}

/// Random writes over `blob/…`, with observations after every fourth.
fn blob_writes<T: Txn>(t: &mut T, seed: u64, ops: u64, trace: &mut Trace) {
    let mut rng = Rng::new(seed);
    for i in 0..ops {
        match rng.below(8) {
            0..=3 => {
                let len = if rng.chance(1, 8) { rng.range(2000, 4000) } else { rng.range(0, 60) };
                t.put(&blob(rng.below(40)), &rng.bytes(len as usize)).unwrap();
            }
            4 | 5 => t.delete(&blob(rng.below(40))).unwrap(),
            _ => {
                let (a, b) = (rng.below(40), rng.below(40));
                t.delete_range(&blob(a.min(b)), &blob(a.max(b))).unwrap();
            }
        }
        if i % 4 == 3 {
            let from = blob(rng.below(40));
            see_scan(t, Bound::Included(&from), Bound::Excluded(b"blob0"), 7, trace);
            see_get(t, &blob(rng.below(40)), trace);
        }
    }
}

/// Runs `p` on `t`; `true` to commit, `false` to abort.
fn run_program<T: Txn>(t: &mut T, p: &Program, trace: &mut Trace) -> bool {
    match *p {
        Program::Increment { k } => {
            let v = num(see_get(t, &ctr(k), trace));
            t.put(&ctr(k), &(v + 1).to_le_bytes()).unwrap();
        }
        Program::Deposit { acct: a, amount } => {
            let v = num(see_get(t, &acct(a), trace));
            t.put(&acct(a), &(v + amount).to_le_bytes()).unwrap();
        }
        Program::Transfer { from, to, amount } => {
            let (a, b) = (num(see_get(t, &acct(from), trace)), num(see_get(t, &acct(to), trace)));
            if a < amount {
                return false;
            }
            t.put(&acct(from), &(a - amount).to_le_bytes()).unwrap();
            t.put(&acct(to), &(b + amount).to_le_bytes()).unwrap();
        }
        Program::Rollup { lo, hi, n } => {
            let (start, end) = (ctr(lo), ctr(hi));
            let rows = see_scan(t, Bound::Included(&start), Bound::Excluded(&end), 100, trace);
            let sum: u64 = rows.into_iter().map(|(_, v)| num(Some(v))).sum();
            t.put(format!("sum/{n:04}").as_bytes(), &sum.to_le_bytes()).unwrap();
            t.delete_range(&start, &end).unwrap();
            see_scan(t, Bound::Included(&start), Bound::Excluded(&end), 100, trace);
            t.put(&start, &sum.to_le_bytes()).unwrap();
            see_scan(t, Bound::Included(&start), Bound::Excluded(&end), 100, trace);
            see_get(t, &ctr(lo + 1), trace);
        }
        Program::Churn { seed } => blob_writes(t, seed, 12, trace),
        Program::Abandon { seed } => {
            blob_writes(t, seed, 6, trace);
            return false;
        }
        Program::Observe { lo, hi } => {
            see_scan(t, Bound::Included(&ctr(lo)), Bound::Excluded(&ctr(hi)), 100, trace);
            see_get(t, &acct(lo), trace);
        }
    }
    true
}

/// The oracle: a transaction on a plain map, written independently of
/// `Emulated<E>` and of every engine.
struct Oracle {
    map: Model,
    seq: u64,
}

struct OracleTxn<'a> {
    oracle: &'a mut Oracle,
    work: Model,
    writes: usize,
}

impl Oracle {
    fn begin(&mut self) -> OracleTxn<'_> {
        OracleTxn { work: self.map.clone(), oracle: self, writes: 0 }
    }
}

impl Txn for OracleTxn<'_> {
    fn get(&self, key: &[u8]) -> spike_core::Result<Option<Vec<u8>>> {
        Ok(self.work.get(key).cloned())
    }

    fn scan(
        &self,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: usize,
    ) -> spike_core::Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self.work.range::<[u8], _>((start, end)).take(limit).map(|(k, v)| (k.clone(), v.clone())).collect())
    }

    fn put(&mut self, key: &[u8], value: &[u8]) -> spike_core::Result<()> {
        self.work.insert(key.to_vec(), value.to_vec());
        self.writes += 1;
        Ok(())
    }

    fn delete(&mut self, key: &[u8]) -> spike_core::Result<()> {
        self.work.remove(key);
        self.writes += 1;
        Ok(())
    }

    fn delete_range(&mut self, start: &[u8], end: &[u8]) -> spike_core::Result<()> {
        let doomed: Vec<Vec<u8>> = self
            .work
            .range::<[u8], _>((Bound::Included(start), Bound::Excluded(end)))
            .map(|(k, _)| k.clone())
            .collect();
        for k in doomed {
            self.work.remove(&k);
        }
        self.writes += 1;
        Ok(())
    }

    fn commit(self) -> spike_core::Result<u64> {
        if self.writes > 0 {
            self.oracle.map = self.work;
            self.oracle.seq += 1;
        }
        Ok(self.oracle.seq)
    }
}

/// What a run of `Program`s produced.
struct RmwRun {
    /// What each program observed.
    traces: Vec<Trace>,
    /// The sequence each program's commit returned (`None` for an aborted
    /// program, and everywhere for engines that do not persist sequences).
    seqs: Vec<Option<u64>>,
    /// Contents at every checkpoint.
    checkpoints: Vec<Observation>,
}

fn oracle_run(programs: &[Program]) -> RmwRun {
    let ranges = scan_ranges(1);
    let mut oracle = Oracle { map: Model::new(), seq: 0 };
    let mut run = RmwRun { traces: Vec::new(), seqs: Vec::new(), checkpoints: Vec::new() };
    for (i, p) in programs.iter().enumerate() {
        let mut t = oracle.begin();
        let mut trace = Trace::new();
        let commit = run_program(&mut t, p, &mut trace);
        run.seqs.push(commit.then(|| t.commit().unwrap()));
        run.traces.push(trace);
        if (i + 1) % CHECKPOINT_EVERY == 0 {
            let scans = ranges
                .iter()
                .map(|(a, z, limit)| {
                    oracle
                        .map
                        .range::<Vec<u8>, _>((Bound::Included(a), Bound::Excluded(z)))
                        .take(*limit)
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .collect();
            run.checkpoints.push(Observation { seq: Some(oracle.seq), contents: oracle.map.clone(), scans });
        }
    }
    run
}

/// Runs the programs as transactions of `E`, with a clean reopen at every
/// checkpoint like [`run`].
fn run_rmw<E: TxnEngine>(vfs: Arc<dyn Vfs>, dir: &Path, programs: &[Program], persistent_seq: bool) -> RmwRun {
    let opts = Options::small_for_tests();
    let ranges = scan_ranges(1);
    let mut run = RmwRun { traces: Vec::new(), seqs: Vec::new(), checkpoints: Vec::new() };
    let mut db = E::open(vfs.clone(), dir, &opts).unwrap();
    for (i, p) in programs.iter().enumerate() {
        let mut t = db.begin_write().unwrap();
        let mut trace = Trace::new();
        let seq = if run_program(&mut t, p, &mut trace) {
            Some(t.commit().unwrap())
        } else {
            t.abort();
            None
        };
        run.seqs.push(seq.filter(|_| persistent_seq));
        run.traces.push(trace);
        if (i + 1) % CHECKPOINT_EVERY == 0 {
            run.checkpoints.push(observe(&db.snapshot().unwrap(), &ranges, persistent_seq));
            db.close().unwrap();
            db = E::open(vfs.clone(), dir, &opts).unwrap();
            let after = observe(&db.snapshot().unwrap(), &ranges, persistent_seq);
            assert_eq!(after, *run.checkpoints.last().unwrap(), "{}: state changed across a clean reopen", E::name());
        }
    }
    db.close().unwrap();
    run
}

fn sim_rmw<E: TxnEngine>(programs: &[Program]) -> RmwRun {
    run_rmw::<E>(SimVfs::new(1).as_vfs(), Path::new("/db"), programs, true)
}

fn real_rmw<E: TxnEngine>(programs: &[Program]) -> RmwRun {
    let dir = temp_dir::<E>();
    let out = run_rmw::<E>(StdVfs::shared(), &dir, programs, false);
    std::fs::remove_dir_all(&dir).unwrap();
    out
}

/// Read-modify-write histories through every engine: the transactions of
/// `Emulated<E>` (engines without transactions of their own) must observe and
/// write exactly what the model oracle does — the values each program reads,
/// the sequence of each commit, the contents at every checkpoint — across
/// aborts, empty transactions, range deletions that hide committed and
/// written keys, and clean reopens. `kerno` joins when it has transactions.
#[test]
fn differential_rmw() {
    for seed in RMW_SEEDS {
        let programs = rmw_history(seed);
        let want = oracle_run(&programs);
        let aborted = want.seqs.iter().filter(|s| s.is_none()).count();
        let empty = want.seqs.windows(2).filter(|w| w[0].is_some() && w[0] == w[1]).count();
        assert!(
            aborted >= 10 && empty >= 5,
            "seed {seed}: the history must abort ({aborted}) and commit empty ({empty})"
        );
        assert!(want.seqs.iter().flatten().max() >= Some(&100), "seed {seed}: the history must commit transactions");

        let mut results: BTreeMap<&str, RmwRun> = BTreeMap::new();
        results.insert("log", sim_rmw::<Emulated<spike_core::reference::LogEngine>>(&programs));
        #[cfg(feature = "candidates")]
        {
            results.insert("btree", sim_rmw::<Emulated<spike_btree::BTreeEngine>>(&programs));
            results.insert("lsm", sim_rmw::<Emulated<spike_lsm::LsmEngine>>(&programs));
        }
        results.insert("redb", real_rmw::<Emulated<spike_bench::adapters::RedbEngine>>(&programs));
        results.insert("fjall", real_rmw::<Emulated<spike_bench::adapters::FjallEngine>>(&programs));
        results.insert("sqlite", real_rmw::<Emulated<spike_bench::adapters::SqliteEngine>>(&programs));

        for (engine, got) in &results {
            for (i, (g, w)) in got.traces.iter().zip(&want.traces).enumerate() {
                assert!(
                    g == w,
                    "seed {seed}, {engine}: program {i} ({:?}) observed something else than the oracle",
                    programs[i]
                );
            }
            for (i, (g, w)) in got.seqs.iter().zip(&want.seqs).enumerate() {
                assert!(
                    g.is_none() || g == w,
                    "seed {seed}, {engine}: program {i} ({:?}) committed at {g:?}, the oracle at {w:?}",
                    programs[i]
                );
            }
            assert_eq!(got.checkpoints.len(), want.checkpoints.len(), "{engine}: wrong number of checkpoints");
            for (n, (g, w)) in got.checkpoints.iter().zip(&want.checkpoints).enumerate() {
                if let Some(seq) = g.seq {
                    assert_eq!(Some(seq), w.seq, "seed {seed}, {engine}: sequence differs at checkpoint {n}");
                }
                assert!(g.contents == w.contents, "seed {seed}, {engine}: contents differ at checkpoint {n}");
                assert!(g.scans == w.scans, "seed {seed}, {engine}: bounded scans differ at checkpoint {n}");
            }
        }
    }
}
