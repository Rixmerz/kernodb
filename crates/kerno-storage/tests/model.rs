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
//! Cases: `PROPTEST_CASES`, else `SPIKE_SEEDS` (like the harness suites),
//! else 256.

use std::path::Path;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Snapshot};
use kerno_storage::{Background, Config, KernoDb, KernoSnapshot, TestConfig};
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

fn config() -> Config {
    Config {
        cache_bytes: 64 << 10,
        write_buffer_bytes: 4 << 10,
        test: TestConfig { background: Background::Inline, db_id: None },
        ..Config::default()
    }
}

fn open(disk: &SimVfs) -> KernoDb {
    KernoDb::open(disk.as_vfs(), Path::new(DIR), &config()).unwrap_or_else(|e| panic!("open failed: {e}"))
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

fn run(seed: u64, ops: &[Op]) {
    let mut disk = SimVfs::new(seed);
    // Tearing at any byte on odd seeds, as in the harness sweeps.
    if seed % 2 == 1 {
        disk.set_tear_unit(1);
    }
    let mut db = Some(open(&disk));
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
                db = Some(open(&disk));
            }
            Op::DropReopen => {
                drop(db.take());
                db = Some(open(&disk));
            }
            Op::PowerLoss => {
                let image = disk.crash_now(CrashKind::PowerLoss);
                // The old handle and its snapshots belong to the machine
                // that lost power.
                drop(db.take());
                snaps.clear();
                disk = image;
                db = Some(open(&disk));
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
        run(seed, &ops);
    }
}
