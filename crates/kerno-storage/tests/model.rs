//! Model-based property test (phase1-design.md §6.2; model tests are never
//! cut, §10.2): random histories of puts, deletes, snapshots, reopens and
//! power losses, checked against an in-memory model after every step.
//!
//! The engine runs inline (`Background::Inline`) with a 4 KiB write buffer,
//! so that a few commits fill a memtable and flushes and compactions happen
//! inside the history, deterministically: a failing history shrinks to a
//! minimal one and replays exactly. Durability is `Sync`, so a power loss
//! keeps every acknowledged commit.
//!
//! The second property runs the same histories with the condition-1 levers
//! at their most aggressive (`aggressive`), so that a wrong L0 subset or a
//! wrong cut shows up as a stale read.
//!
//! The `prop_mvcc_model_*` properties add what ADR-0001 condition 2 needs:
//! range deletions, flushes, single compactions and directed compactions
//! (`compact_range`) anywhere in the history, with snapshots taken and dropped
//! throughout. The model of every live snapshot is a copy of the model at the
//! moment it was taken, and each step checks all of them (their scan and a
//! `get` of every key), the current state, and the sequence numbers, so a
//! range tombstone that hides a point it should not, or fails to hide one it
//! should, in any source (memtable, L0, deeper levels, a file written by a
//! compaction that cut or dropped it), shows up as a minimal history.
//!
//! `prop_txn_overlay_matches_model` does the same for write transactions
//! (phase1-design.md §5.7): random puts, deletes and range deletions inside a
//! transaction, mirrored on a model, with the transaction's `get` and `scan`
//! (paginated, bounded, inverted bounds included) compared after every
//! operation, over data spread across the memtable and files by flushes and
//! compactions between the transactions, with snapshots held throughout.
//!
//! Cases: `PROPTEST_CASES`, else `SPIKE_SEEDS` (like the harness suites),
//! else 256.

use std::ops::Bound;
use std::path::Path;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Snapshot};
use kerno_storage::{
    Background, CloseBudget, CompactionConfig, Config, KernoDb, KernoSnapshot, L0Metric, TestConfig, WriteTxn,
};
use kerno_testkit::harness::{Model, apply, diff, read_all};
use kerno_testkit::vfs::{CrashKind, SimVfs};
use proptest::prelude::*;
use proptest::sample::Index;

const DIR: &str = "/db";
const KEYS: u8 = 16;

#[derive(Clone, Debug)]
enum Op {
    /// Commits a put of key `k` with a value of `len` bytes.
    Put {
        k: u8,
        len: u16,
    },
    /// Commits a delete of key `k` (present or not).
    Delete {
        k: u8,
    },
    TakeSnapshot,
    /// Commits a range deletion of `[key(a), key(b))` (`a` and `b` count the
    /// alphabet and one key past its end; `a > b` is swapped, `a == b` is the
    /// empty range, which still is a commit).
    DeleteRange {
        a: u8,
        b: u8,
    },
    /// Writes the memtable to a file.
    Flush,
    /// Runs one compaction, if one is due.
    CompactOnce,
    /// Pushes the keys between the bounds to the bottom level
    /// (`compact_range`); `kind` picks which bounds are open.
    CompactRange {
        a: u8,
        b: u8,
        kind: u8,
    },
    /// Drops one live snapshot, if any.
    DropSnapshot(Index),
    /// `close`, then open again.
    CleanReopen,
    /// Drops the handle without `close`, then opens again.
    DropReopen,
    /// Power loss with the handle open, then open the surviving image.
    PowerLoss,
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        12 => (0..KEYS, 0..3000u16).prop_map(|(k, len)| Op::Put { k, len }),
        4 => (0..KEYS).prop_map(|k| Op::Delete { k }),
        2 => Just(Op::TakeSnapshot),
        1 => any::<Index>().prop_map(Op::DropSnapshot),
        1 => Just(Op::CleanReopen),
        1 => Just(Op::DropReopen),
        1 => Just(Op::PowerLoss),
    ]
}

/// The histories of the `prop_mvcc_model_*` properties: puts, deletes, range
/// deletions, flushes and compactions of both kinds, snapshots, reopens.
fn mvcc_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        10 => (0..KEYS, 0..3000u16).prop_map(|(k, len)| Op::Put { k, len }),
        3 => (0..KEYS).prop_map(|k| Op::Delete { k }),
        5 => (0..=KEYS, 0..=KEYS).prop_map(|(a, b)| Op::DeleteRange { a, b }),
        2 => Just(Op::Flush),
        2 => Just(Op::CompactOnce),
        2 => (0..=KEYS, 0..=KEYS, any::<u8>()).prop_map(|(a, b, kind)| Op::CompactRange { a, b, kind }),
        3 => Just(Op::TakeSnapshot),
        1 => any::<Index>().prop_map(Op::DropSnapshot),
        1 => Just(Op::CleanReopen),
        1 => Just(Op::DropReopen),
        1 => Just(Op::PowerLoss),
    ]
}

fn key(k: u8) -> Vec<u8> {
    format!("key-{k:02}").into_bytes()
}

/// A value unique to the put that writes it (`n` counts puts), so that a
/// stale version can never pass for the current one.
fn value(n: u64, len: u16) -> Vec<u8> {
    let mut v = n.to_le_bytes().to_vec();
    v.extend((0..len).map(|i| (n as u8).wrapping_mul(31).wrapping_add(i as u8)));
    v
}

/// The levers the second property test pushes hard: small L0 limits, tiny
/// compaction inputs and outputs, a grandparent cut at nearly every file, the
/// heap merge from two children up, and closes that leave debt behind.
fn aggressive() -> (CompactionConfig, CloseBudget) {
    let compaction = CompactionConfig {
        l0_trigger: 2,
        l0_slowdown: 4,
        l0_stop: 8,
        level_base_bytes: Some(6 << 10),
        level_multiplier: 3,
        target_file_size: Some(2 << 10),
        l0_metric: L0Metric::Depth,
        l0_priority: true,
        l0_partial: true,
        max_compaction_bytes: Some(3 << 10),
        grandparent_overlap_bytes: Some(2 << 10),
        l0_trivial_move: true,
        flush_split: true,
        heap_merge_threshold: 1,
        max_subcompactions: 3,
        ..CompactionConfig::default()
    };
    (compaction, CloseBudget::Bounded { max_bytes: 4 << 10, max_duration: None })
}

fn config(levers: bool) -> Config {
    let mut cfg = Config {
        cache_bytes: 64 << 10,
        write_buffer_bytes: 4 << 10,
        test: TestConfig { background: Background::Inline, db_id: None },
        ..Config::default()
    };
    if levers {
        (cfg.compaction, cfg.close_budget) = aggressive();
    }
    cfg
}

fn open(disk: &SimVfs, levers: bool) -> KernoDb {
    KernoDb::open(disk.as_vfs(), Path::new(DIR), &config(levers)).unwrap_or_else(|e| panic!("open failed: {e}"))
}

/// Full scan and a point read of every key of the alphabet.
fn check(snap: &KernoSnapshot, model: &Model, seq: u64, ctx: &str) {
    assert_eq!(snap.seq(), seq, "{ctx}: sequence");
    let all = read_all(snap).unwrap_or_else(|e| panic!("{ctx}: scan failed: {e}"));
    assert!(all == *model, "{ctx}: scan differs from the model: {}", diff(model, &all));
    for k in 0..KEYS {
        let got = snap.get(&key(k)).unwrap_or_else(|e| panic!("{ctx}: get failed: {e}"));
        assert_eq!(got.as_ref(), model.get(&key(k)), "{ctx}: get(key-{k:02})");
    }
}

struct LiveSnapshot {
    snap: KernoSnapshot,
    model: Model,
    seq: u64,
    /// Step at which it was taken, for messages.
    taken_at: usize,
}

fn run(seed: u64, ops: &[Op], levers: bool) {
    let mut disk = SimVfs::new(seed);
    // Tearing at any byte on odd seeds, as in the harness sweeps.
    if seed % 2 == 1 {
        disk.set_tear_unit(1);
    }
    let mut db = Some(open(&disk, levers));
    let mut model = Model::new();
    let mut seq = 0u64;
    let mut puts = 0u64;
    let mut snaps: Vec<LiveSnapshot> = Vec::new();
    for (step, op) in ops.iter().enumerate() {
        let ctx = format!("step {step} ({op:?})");
        match op {
            Op::Put { k, len } => {
                puts += 1;
                let mut b = WriteBatch::new();
                b.put(key(*k), value(puts, *len));
                seq = commit(db.as_ref().unwrap(), &b, seq, &ctx);
                apply(&mut model, &b);
            }
            Op::Delete { k } => {
                let mut b = WriteBatch::new();
                b.delete(key(*k));
                seq = commit(db.as_ref().unwrap(), &b, seq, &ctx);
                apply(&mut model, &b);
            }
            Op::DeleteRange { a, b } => {
                let (a, b) = (*a.min(b), *a.max(b));
                let mut batch = WriteBatch::new();
                batch.delete_range(key(a), key(b));
                seq = commit(db.as_ref().unwrap(), &batch, seq, &ctx);
                apply(&mut model, &batch);
            }
            Op::Flush => db.as_ref().unwrap().hooks().flush().unwrap_or_else(|e| panic!("{ctx}: flush failed: {e}")),
            Op::CompactOnce => {
                db.as_ref().unwrap().hooks().compact_once().unwrap_or_else(|e| panic!("{ctx}: compaction failed: {e}"));
            }
            Op::CompactRange { a, b, kind } => {
                let (lo, hi) = (key(*a.min(b)), key(*a.max(b)));
                let (start, end) = match kind % 4 {
                    0 => (Bound::Included(&lo[..]), Bound::Excluded(&hi[..])),
                    1 => (Bound::Unbounded, Bound::Included(&hi[..])),
                    2 => (Bound::Excluded(&lo[..]), Bound::Unbounded),
                    _ => (Bound::Unbounded, Bound::Unbounded),
                };
                db.as_ref()
                    .unwrap()
                    .compact_range(start, end)
                    .unwrap_or_else(|e| panic!("{ctx}: compact_range failed: {e}"));
            }
            Op::TakeSnapshot => {
                let snap = db.as_ref().unwrap().snapshot().unwrap_or_else(|e| panic!("{ctx}: {e}"));
                snaps.push(LiveSnapshot { snap, model: model.clone(), seq, taken_at: step });
            }
            Op::DropSnapshot(i) => {
                if !snaps.is_empty() {
                    snaps.remove(i.index(snaps.len()));
                }
            }
            Op::CleanReopen => {
                db.take().unwrap().close().unwrap_or_else(|e| panic!("{ctx}: close failed: {e}"));
                db = Some(open(&disk, levers));
            }
            Op::DropReopen => {
                drop(db.take());
                db = Some(open(&disk, levers));
            }
            Op::PowerLoss => {
                let image = disk.crash_now(CrashKind::PowerLoss);
                // The old handle and its snapshots belong to the machine
                // that lost power.
                drop(db.take());
                snaps.clear();
                disk = image;
                db = Some(open(&disk, levers));
            }
        }
        // The current state, and every snapshot still alive (also those
        // taken by an earlier handle of this process).
        let current = db.as_ref().unwrap().snapshot().unwrap_or_else(|e| panic!("{ctx}: snapshot failed: {e}"));
        check(&current, &model, seq, &format!("{ctx}: current state"));
        for s in &snaps {
            check(&s.snap, &s.model, s.seq, &format!("{ctx}: snapshot taken at step {}", s.taken_at));
        }
    }
    drop(snaps);
    db.take().unwrap().close().unwrap();
    // Nothing but the MANIFEST and SSTs (and the lock file) is left.
    let names = disk.as_vfs().list(Path::new(DIR)).unwrap();
    assert!(
        names.iter().all(|n| n == "LOCK" || n == "MANIFEST" || n.ends_with(".sst")),
        "unexpected files after a clean close: {names:?}"
    );
}

fn commit(db: &KernoDb, b: &WriteBatch, seq: u64, ctx: &str) -> u64 {
    let got = db.commit(b).unwrap_or_else(|e| panic!("{ctx}: commit failed: {e}"));
    assert_eq!(got, seq + 1, "{ctx}: commit sequence");
    got
}

fn proptest_config() -> ProptestConfig {
    let mut cfg = ProptestConfig::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        cfg.cases = std::env::var("SPIKE_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(256);
    }
    cfg
}

proptest! {
    #![proptest_config(proptest_config())]

    #[test]
    fn prop_model_put_delete_snapshot_reopen(seed in any::<u64>(), ops in prop::collection::vec(op(), 1..300)) {
        run(seed, &ops, false);
    }

    /// The same histories under partial L0 compactions, flush split, trivial
    /// moves, the grandparent cut, heap merges and budgeted closes.
    #[test]
    fn prop_partial_l0_and_flush_split_preserve_model(
        seed in any::<u64>(),
        ops in prop::collection::vec(op(), 1..300),
    ) {
        run(seed, &ops, true);
    }

    /// Range deletions, flushes, compactions and `compact_range` against the
    /// MVCC model, with the default configuration.
    #[test]
    fn prop_mvcc_model_snapshots_flush_compaction_ranges(
        seed in any::<u64>(),
        ops in prop::collection::vec(mvcc_op(), 1..300),
    ) {
        run(seed, &ops, false);
    }

    /// The same under the aggressive levers: partial L0 compactions cut
    /// through range tombstones, grandparent cuts, subcompactions that split
    /// a fragment between threads' key ranges.
    #[test]
    fn prop_mvcc_model_ranges_under_aggressive_levers(
        seed in any::<u64>(),
        ops in prop::collection::vec(mvcc_op(), 1..300),
    ) {
        run(seed, &ops, true);
    }
}

// ---- write transactions ----

/// Keys of the transaction histories: more than the page a transaction's scan
/// reads from the snapshot, so that a range hides whole pages of it.
const TXN_KEYS: u8 = 48;

fn txn_key(k: u8) -> Vec<u8> {
    format!("tkey-{k:02}").into_bytes()
}

#[derive(Clone, Debug)]
enum TxnOp {
    Put {
        k: u8,
        len: u16,
    },
    Delete {
        k: u8,
    },
    /// `[key(min), key(max))`: `a == b` is the empty range, still a write.
    DeleteRange {
        a: u8,
        b: u8,
    },
    /// A bounded scan between two keys in any order; `kinds` picks the kind
    /// of each bound.
    Scan {
        a: u8,
        b: u8,
        kinds: u8,
        limit: u8,
    },
}

#[derive(Clone, Copy, Debug)]
enum End {
    Commit,
    Abort,
    Drop,
}

#[derive(Clone, Debug)]
enum Hist {
    Put { k: u8, len: u16 },
    DeleteRange { a: u8, b: u8 },
    Flush,
    CompactOnce,
    TakeSnapshot,
    DropSnapshot(Index),
    Txn { ops: Vec<TxnOp>, end: End },
    CleanReopen,
    DropReopen,
}

fn txn_op() -> impl Strategy<Value = TxnOp> {
    prop_oneof![
        10 => (0..TXN_KEYS, 0..300u16).prop_map(|(k, len)| TxnOp::Put { k, len }),
        4 => (0..TXN_KEYS).prop_map(|k| TxnOp::Delete { k }),
        4 => (0..=TXN_KEYS, 0..=TXN_KEYS).prop_map(|(a, b)| TxnOp::DeleteRange { a, b }),
        4 => (0..=TXN_KEYS, 0..=TXN_KEYS, any::<u8>(), any::<u8>())
            .prop_map(|(a, b, kinds, limit)| TxnOp::Scan { a, b, kinds, limit }),
    ]
}

fn hist() -> impl Strategy<Value = Hist> {
    prop_oneof![
        8 => (0..TXN_KEYS, 0..300u16).prop_map(|(k, len)| Hist::Put { k, len }),
        2 => (0..=TXN_KEYS, 0..=TXN_KEYS).prop_map(|(a, b)| Hist::DeleteRange { a, b }),
        2 => Just(Hist::Flush),
        2 => Just(Hist::CompactOnce),
        2 => Just(Hist::TakeSnapshot),
        1 => any::<Index>().prop_map(Hist::DropSnapshot),
        10 => (prop::collection::vec(txn_op(), 0..14), prop_oneof![
            4 => Just(End::Commit),
            1 => Just(End::Abort),
            1 => Just(End::Drop),
        ])
            .prop_map(|(ops, end)| Hist::Txn { ops, end }),
        1 => Just(Hist::CleanReopen),
        1 => Just(Hist::DropReopen),
    ]
}

type Rows = Vec<(Vec<u8>, Vec<u8>)>;

/// What a scan over `model` returns: the rows within the bounds, in order, at
/// most `limit`; nothing when the bounds are inverted.
fn model_rows(model: &Model, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Rows {
    let after_start = |k: &[u8]| match start {
        Bound::Included(s) => k >= s,
        Bound::Excluded(s) => k > s,
        Bound::Unbounded => true,
    };
    let before_end = |k: &[u8]| match end {
        Bound::Included(e) => k <= e,
        Bound::Excluded(e) => k < e,
        Bound::Unbounded => true,
    };
    model
        .iter()
        .filter(|(k, _)| after_start(k) && before_end(k))
        .take(limit)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Everything the transaction sees, read in pages of 7 rows (an odd size, to
/// restart pages in the middle of hidden ranges), and a `get` of every key.
fn check_txn(t: &WriteTxn<'_>, model: &Model, ctx: &str) {
    let mut seen = Model::new();
    let mut last: Option<Vec<u8>> = None;
    loop {
        let from = last.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
        let page = t.scan(from, Bound::Unbounded, 7).unwrap_or_else(|e| panic!("{ctx}: scan failed: {e}"));
        let done = page.len() < 7;
        for (k, v) in page {
            assert!(last.as_ref().is_none_or(|l| *l < k), "{ctx}: scan out of order");
            last = Some(k.clone());
            seen.insert(k, v);
        }
        if done {
            break;
        }
    }
    assert!(seen == *model, "{ctx}: scan differs from the model: {}", diff(model, &seen));
    for k in 0..TXN_KEYS {
        let got = t.get(&txn_key(k)).unwrap_or_else(|e| panic!("{ctx}: get failed: {e}"));
        assert_eq!(got.as_ref(), model.get(&txn_key(k)), "{ctx}: get(tkey-{k:02})");
    }
}

fn bound(kind: u8, key: &[u8]) -> Bound<&[u8]> {
    match kind % 3 {
        0 => Bound::Included(key),
        1 => Bound::Excluded(key),
        _ => Bound::Unbounded,
    }
}

fn run_txn_history(seed: u64, hist: &[Hist]) {
    let disk = SimVfs::new(seed);
    if seed % 2 == 1 {
        disk.set_tear_unit(1);
    }
    let mut db = Some(open(&disk, false));
    let mut model = Model::new();
    let mut seq = 0u64;
    let mut puts = 0u64;
    let mut snaps: Vec<LiveSnapshot> = Vec::new();
    for (step, h) in hist.iter().enumerate() {
        let ctx = format!("step {step} ({h:?})");
        match h {
            Hist::Put { k, len } => {
                puts += 1;
                let mut b = WriteBatch::new();
                b.put(txn_key(*k), value(puts, *len));
                seq = commit(db.as_ref().unwrap(), &b, seq, &ctx);
                apply(&mut model, &b);
            }
            Hist::DeleteRange { a, b } => {
                let mut batch = WriteBatch::new();
                batch.delete_range(txn_key(*a.min(b)), txn_key(*a.max(b)));
                seq = commit(db.as_ref().unwrap(), &batch, seq, &ctx);
                apply(&mut model, &batch);
            }
            Hist::Flush => db.as_ref().unwrap().hooks().flush().unwrap_or_else(|e| panic!("{ctx}: flush failed: {e}")),
            Hist::CompactOnce => {
                db.as_ref().unwrap().hooks().compact_once().unwrap_or_else(|e| panic!("{ctx}: compaction failed: {e}"));
            }
            Hist::TakeSnapshot => {
                let snap = db.as_ref().unwrap().snapshot().unwrap_or_else(|e| panic!("{ctx}: {e}"));
                snaps.push(LiveSnapshot { snap, model: model.clone(), seq, taken_at: step });
                if snaps.len() > 4 {
                    snaps.remove(0);
                }
            }
            Hist::DropSnapshot(i) => {
                if !snaps.is_empty() {
                    snaps.remove(i.index(snaps.len()));
                }
            }
            Hist::Txn { ops, end } => {
                let mut t =
                    db.as_ref().unwrap().begin_write().unwrap_or_else(|e| panic!("{ctx}: begin_write failed: {e}"));
                assert_eq!(t.read_seq(), seq, "{ctx}: the transaction begins on the latest commit");
                let mut tx_model = model.clone();
                check_txn(&t, &tx_model, &format!("{ctx}: on begin"));
                let mut wrote = false;
                for (i, op) in ops.iter().enumerate() {
                    let ctx = format!("{ctx}, op {i} ({op:?})");
                    match op {
                        TxnOp::Put { k, len } => {
                            puts += 1;
                            let (key, val) = (txn_key(*k), value(puts, *len));
                            t.put(&key, &val).unwrap_or_else(|e| panic!("{ctx}: {e}"));
                            tx_model.insert(key, val);
                            wrote = true;
                        }
                        TxnOp::Delete { k } => {
                            t.delete(&txn_key(*k)).unwrap_or_else(|e| panic!("{ctx}: {e}"));
                            tx_model.remove(&txn_key(*k));
                            wrote = true;
                        }
                        TxnOp::DeleteRange { a, b } => {
                            let (lo, hi) = (txn_key(*a.min(b)), txn_key(*a.max(b)));
                            t.delete_range(&lo, &hi).unwrap_or_else(|e| panic!("{ctx}: {e}"));
                            let doomed: Vec<_> = tx_model.range(lo..hi).map(|(k, _)| k.clone()).collect();
                            doomed.iter().for_each(|k| drop(tx_model.remove(k)));
                            wrote = true;
                        }
                        TxnOp::Scan { a, b, kinds, limit } => {
                            let (ka, kb) = (txn_key(*a), txn_key(*b));
                            let (start, end) = (bound(*kinds, &ka), bound(*kinds / 3, &kb));
                            let limit = [0usize, 1, 2, 5, 50][*limit as usize % 5];
                            let got = t.scan(start, end, limit).unwrap_or_else(|e| panic!("{ctx}: {e}"));
                            let want = model_rows(&tx_model, start, end, limit);
                            assert_eq!(got, want, "{ctx}: bounded scan");
                        }
                    }
                    check_txn(&t, &tx_model, &ctx);
                }
                match end {
                    End::Commit => {
                        let got = t.commit().unwrap_or_else(|e| panic!("{ctx}: commit failed: {e}"));
                        seq += u64::from(wrote);
                        assert_eq!(got, seq, "{ctx}: commit sequence");
                        model = tx_model;
                    }
                    End::Abort => t.abort(),
                    End::Drop => drop(t),
                }
            }
            Hist::CleanReopen => {
                db.take().unwrap().close().unwrap_or_else(|e| panic!("{ctx}: close failed: {e}"));
                db = Some(open(&disk, false));
            }
            Hist::DropReopen => {
                drop(db.take());
                db = Some(open(&disk, false));
            }
        }
        let current = db.as_ref().unwrap().snapshot().unwrap_or_else(|e| panic!("{ctx}: snapshot failed: {e}"));
        let all = read_all(&current).unwrap();
        assert!(all == model, "{ctx}: state differs from the model: {}", diff(&model, &all));
        assert_eq!(current.seq(), seq, "{ctx}: sequence");
        for s in &snaps {
            let all = read_all(&s.snap).unwrap();
            assert!(
                all == s.model,
                "{ctx}: snapshot taken at step {} differs from its model: {}",
                s.taken_at,
                diff(&s.model, &all)
            );
        }
    }
    drop(snaps);
    db.take().unwrap().close().unwrap();
}

proptest! {
    #![proptest_config(proptest_config())]

    /// Transactions against the model: what a transaction sees after every
    /// write equals the model, its commit applies exactly that (consuming a
    /// sequence only when it wrote), an abort or a drop applies nothing, and
    /// snapshots held across it all never change.
    #[test]
    fn prop_txn_overlay_matches_model(seed in any::<u64>(), hist in prop::collection::vec(hist(), 1..40)) {
        run_txn_history(seed, &hist);
    }
}
