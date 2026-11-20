//! Write transactions of `KernoDb` (phase1-design.md §5.7): the shared suite
//! with background threads and with the inline profile, and the checks that
//! need the engine's counters, its limits, its read-only mode or a leader
//! that fails while a transaction commits.

use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, Result, WriteBatch};
use kerno_storage::testing::{Inline, Profiled};
use kerno_storage::{AccessMode, Config, KernoDb, KernoSnapshot, Limits};
use kerno_testkit::harness::crash::{Step, every_point_steps, run_scenario, scenario_b};
use kerno_testkit::harness::txn::apply_in_txn;
use kerno_testkit::vfs::{CrashKind, HookAction, HookOpKind, HookPoint, HookVfs, SimVfs};

kerno_testkit::txn_test_suite!(kerno_storage::KernoDb);

/// The same suite with flushes and compactions inline (profile
/// `kerno-inline`).
mod inline {
    kerno_testkit::txn_test_suite!(kerno_storage::testing::Profiled<kerno_storage::testing::Inline>);
}

/// How long a test waits for another thread before it fails as hung.
const LIMIT: Duration = Duration::from_secs(60);

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap_or_else(|| panic!("no stat {name}"))
}

fn config(durability: Durability) -> Config {
    Config::from(&Options { durability, ..Options::small_for_tests() })
}

fn open(disk: &SimVfs, cfg: &Config) -> KernoDb {
    KernoDb::open(disk.as_vfs(), Path::new("/db"), cfg).unwrap()
}

fn put(db: &KernoDb, key: &str) -> u64 {
    let mut b = WriteBatch::new();
    b.put(key, "v");
    db.commit(&b).unwrap()
}

/// A panic in the leader while a transaction commits (here the WAL write)
/// poisons the handle like any commit would: the transaction gets `Io`, every
/// later commit, `begin_write` and snapshot gets `Poisoned`, the write slot
/// is free (nothing hangs) and nothing of the transaction survives a reopen.
#[test]
fn leader_panic_during_txn_commit_poisons() {
    let disk = SimVfs::new(1);
    let armed = Arc::new(AtomicBool::new(false));
    let hook_armed = armed.clone();
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        let wal = op.path.extension().is_some_and(|e| e == "wal");
        if hook_armed.load(Ordering::SeqCst) && wal && op.kind == HookOpKind::WriteAt && op.point == HookPoint::Before {
            return HookAction::Panic;
        }
        HookAction::Proceed
    });
    let cfg = Config { wal_preallocate: false, ..config(Durability::Sync) };
    let db = Arc::new(KernoDb::open(vfs, Path::new("/db"), &cfg).unwrap());
    assert_eq!(put(&db, "before"), 1);

    armed.store(true, Ordering::SeqCst);
    let mut txn = db.begin_write().unwrap();
    txn.put(b"during", b"1").unwrap();
    let err = txn.commit().unwrap_err();
    assert!(matches!(&err, Error::Io(e) if e.to_string().contains("panicked")), "{err:?}");

    // Everything fails fast, on this thread and others: the slot was released.
    let (tx, rx) = mpsc::channel();
    let later = {
        let db = db.clone();
        std::thread::spawn(move || {
            tx.send((db.begin_write().err(), db.commit(&WriteBatch::new()).err(), db.snapshot().err())).unwrap()
        })
    };
    let (begin, empty_commit, snapshot) = rx.recv_timeout(LIMIT).expect("a thread hangs on the write slot");
    later.join().unwrap();
    assert!(matches!(begin, Some(Error::Poisoned(_))), "{begin:?}");
    assert!(matches!(empty_commit, Some(Error::Poisoned(_))), "{empty_commit:?}");
    assert!(matches!(snapshot, Some(Error::Poisoned(_))), "{snapshot:?}");
    assert!(matches!(db.commit(&batch_of("after")), Err(Error::Poisoned(_))));
    // A failed commit is neither committed nor aborted.
    assert_eq!((stat(&db, "txns_committed"), stat(&db, "txns_aborted")), (0, 0));

    drop(db);
    let db = open(&disk, &config(Durability::Sync));
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.seq(), 1, "the transaction consumed no sequence");
    assert_eq!(snap.get(b"before").unwrap().as_deref(), Some(&b"v"[..]));
    assert_eq!(snap.get(b"during").unwrap(), None);
}

fn batch_of(key: &str) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(key, "v");
    b
}

/// `txn_gate_wait_micros` shows how long a transaction held other writers
/// up: a blind commit and a `begin_write` that waited for a transaction add
/// their wait, a free slot adds nothing.
#[test]
fn txn_gate_wait_metric_reported() {
    let disk = SimVfs::new(2);
    let db = Arc::new(open(&disk, &config(Durability::Sync)));
    put(&db, "a");
    assert_eq!(stat(&db, "txn_gate_wait_micros"), 0);
    let txn = db.begin_write().unwrap();
    assert_eq!(stat(&db, "txn_gate_wait_micros"), 0, "taking a free slot is not a wait");

    let (started_tx, started_rx) = mpsc::channel();
    let waiters: Vec<_> = (0..2)
        .map(|i| {
            let db = db.clone();
            let started = started_tx.clone();
            std::thread::spawn(move || {
                started.send(()).unwrap();
                if i == 0 {
                    db.commit(&batch_of("blind")).unwrap()
                } else {
                    db.begin_write().unwrap().commit().unwrap()
                }
            })
        })
        .collect();
    for _ in 0..2 {
        started_rx.recv_timeout(LIMIT).unwrap();
    }
    // Both are now (about to be) waiting for the slot: hold it for a while.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(txn.commit().unwrap(), 1, "an empty transaction returns the current sequence");
    for w in waiters {
        w.join().unwrap();
    }
    let waited = stat(&db, "txn_gate_wait_micros");
    // Two threads waited about 100 ms each; leave room for the one that
    // reached the slot late.
    assert!(waited >= 100_000, "waited {waited} µs");
    assert_eq!((stat(&db, "txns_committed"), stat(&db, "txns_aborted")), (2, 0));
}

/// Commits count when `commit` returns a sequence, aborts when a transaction
/// ends by `abort` or by being dropped, also while a panic unwinds.
#[test]
fn txn_counters() {
    let disk = SimVfs::new(3);
    let db = open(&disk, &config(Durability::Sync));
    let mut t = db.begin_write().unwrap();
    t.put(b"a", b"1").unwrap();
    assert_eq!(t.commit().unwrap(), 1);
    assert_eq!(db.begin_write().unwrap().commit().unwrap(), 1, "an empty transaction commits too");
    db.begin_write().unwrap().abort();
    drop(db.begin_write().unwrap());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut t = db.begin_write().unwrap();
        t.put(b"b", b"1").unwrap();
        panic!("user code");
    }));
    assert!(r.is_err());
    assert_eq!((stat(&db, "txns_committed"), stat(&db, "txns_aborted")), (2, 3));
    assert_eq!(db.metrics().txns.committed, 2);
}

/// A transaction reads from a registered snapshot: it counts as a live one
/// while it lasts and reads the sequence it began on.
#[test]
fn txn_holds_a_registered_snapshot() {
    let disk = SimVfs::new(4);
    let db = open(&disk, &config(Durability::Sync));
    put(&db, "a");
    put(&db, "b");
    let t = db.begin_write().unwrap();
    assert_eq!(t.read_seq(), 2);
    let m = db.metrics().snapshots;
    assert_eq!((m.live, m.oldest_seq), (1, 2));
    t.abort();
    assert_eq!(db.metrics().snapshots.live, 0);
}

/// `begin_write` waits for commits that are queued or being written, and
/// starts on a state that includes them.
#[test]
fn begin_write_waits_for_the_pipeline_to_drain() {
    let disk = SimVfs::new(5);
    let db = Arc::new(open(&disk, &config(Durability::Sync)));
    let writers: Vec<_> = (0..8)
        .map(|i| {
            let db = db.clone();
            std::thread::spawn(move || {
                let mut last = 0;
                for n in 0..50 {
                    last = put(&db, &format!("w{i}/{n:03}"));
                }
                last
            })
        })
        .collect();
    // Transactions interleaved with the blind writers: each one begins on
    // exactly the latest published state, and an empty one reports it.
    for _ in 0..50 {
        let t = db.begin_write().unwrap();
        let seen = t.read_seq();
        let rows = t.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap();
        assert_eq!(rows.len() as u64, seen, "every sequence is one put of a new key");
        assert_eq!(t.commit().unwrap(), seen);
    }
    for w in writers {
        w.join().unwrap();
    }
    assert_eq!(db.snapshot().unwrap().seq(), 400);
}

#[test]
fn read_only_handles_refuse_transactions() {
    let disk = SimVfs::new(6);
    let db = open(&disk, &config(Durability::Sync));
    put(&db, "a");
    db.close().unwrap();
    let cfg = Config { access: AccessMode::ReadOnly, ..config(Durability::Sync) };
    let db = open(&disk, &cfg);
    assert!(matches!(db.begin_write().err(), Some(Error::ReadOnly(_))));
    assert_eq!(db.snapshot().unwrap().get(b"a").unwrap().as_deref(), Some(&b"v"[..]));
}

/// The limits of a batch bound a transaction as it grows, not only at its
/// commit: the write that would pass them is refused and the transaction
/// stays usable and committable.
#[test]
fn batch_limits_bound_the_transaction_eagerly() {
    let disk = SimVfs::new(7);
    let limits = Limits { max_batch_bytes: 100, max_batch_ops: 4, ..Limits::default() };
    let db = open(&disk, &Config { limits, ..config(Durability::Sync) });
    let invalid = |r: kerno_core::Result<()>, what: &str| {
        assert!(matches!(r, Err(Error::InvalidArgument(_))), "{what}: {r:?}");
    };
    let mut t = db.begin_write().unwrap();
    t.put(b"a", &[1u8; 60]).unwrap();
    invalid(t.put(b"b", &[1u8; 60]), "over the byte limit");
    assert_eq!(t.get(b"b").unwrap(), None);
    t.delete(b"c").unwrap();
    t.delete_range(b"d", b"e").unwrap();
    t.delete(b"f").unwrap();
    invalid(t.delete(b"g"), "over the operation limit");
    assert_eq!(t.commit().unwrap(), 1);
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.get(b"a").unwrap().map(|v| v.len()), Some(60));
}

/// A commit that is under the payload limit but encodes over it fails at the
/// commit with `InvalidArgument`, writes nothing and frees the slot.
#[test]
fn oversized_encoding_fails_at_commit_and_frees_the_slot() {
    let disk = SimVfs::new(8);
    let limits = Limits { max_batch_bytes: 64, ..Limits::default() };
    let db = open(&disk, &Config { limits, ..config(Durability::Sync) });
    let mut t = db.begin_write().unwrap();
    // 12 ops of 5 payload bytes each is 60, but each op also has a header.
    for i in 0..12u8 {
        t.put(&[b'k', i], &[7, 7, 7]).unwrap();
    }
    assert!(matches!(t.commit(), Err(Error::InvalidArgument(_))));
    assert_eq!(db.snapshot().unwrap().seq(), 0);
    assert_eq!(db.begin_write().unwrap().commit().unwrap(), 0, "the slot is free");
    assert_eq!(put(&db, "ok"), 1, "and the handle is sound");
}

/// Debug builds turn a self-deadlock into a panic: a blind commit on the
/// thread that holds a transaction.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "write slot is already held")]
fn commit_while_holding_a_transaction_panics_in_debug_builds() {
    let disk = SimVfs::new(9);
    let db = open(&disk, &config(Durability::Sync));
    let _t = db.begin_write().unwrap();
    put(&db, "deadlock");
}

// ---- scenario B with transactions ----

type Db = Profiled<Inline>;

/// `Db` whose blind commits go through a write transaction: `begin_write`,
/// the operations in order, `commit`.
struct ViaTxn(Db);

impl Engine for ViaTxn {
    type Snapshot = KernoSnapshot;

    fn name() -> &'static str {
        "kerno-inline-txn-commits"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        Db::open(vfs, dir, opts).map(ViaTxn)
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        let mut txn = self.0.begin_write()?;
        apply_in_txn(&mut txn, batch)?;
        txn.commit()
    }

    fn snapshot(&self) -> Result<KernoSnapshot> {
        self.0.snapshot()
    }

    fn close(self) -> Result<()> {
        self.0.close()
    }
}

/// A read-modify-write transaction that leaves no sequence behind: it reads
/// what is there, rewrites and deletes over it, reads its own writes back and
/// aborts. Followed by an empty transaction, whose commit is the current
/// sequence. Neither writes a byte, so the sweeps may place them anywhere.
fn txn_steps() -> [Step<ViaTxn>; 2] {
    let rmw = |db: &ViaTxn| -> Result<()> {
        let mut txn = db.0.begin_write()?;
        let rows = txn.scan(Bound::Unbounded, Bound::Unbounded, 3)?;
        if let [(first, _), (second, _), ..] = rows.as_slice() {
            txn.put(first, b"rewritten")?;
            txn.delete_range(first, second)?;
            assert_eq!(txn.get(first)?, None, "the range hides the key written before it");
            txn.put(second, b"rewritten")?;
            assert_eq!(txn.get(second)?.as_deref(), Some(&b"rewritten"[..]));
        }
        txn.abort();
        Ok(())
    };
    let empty = |db: &ViaTxn| -> Result<()> {
        let seq = db.snapshot()?.seq();
        assert_eq!(db.0.begin_write()?.commit()?, seq, "an empty transaction commits the current sequence");
        Ok(())
    };
    [Step::Ext("txn_rmw_abort", Arc::new(rmw)), Step::Ext("txn_empty_commit", Arc::new(empty))]
}

/// Scenario B with every commit a transaction, and transactions that leave
/// no sequence inserted before the handle is dropped and before the close.
fn scenario_b_with_transactions() -> Vec<Step<ViaTxn>> {
    let mut steps = scenario_b::<ViaTxn>(7, Arc::new(|db: &ViaTxn| db.0.hooks().checkpoint()));
    let [rmw, empty] = txn_steps();
    let drop_at = steps.iter().position(|s| matches!(s, Step::Drop)).expect("scenario B drops a handle");
    steps.insert(drop_at, empty);
    steps.insert(drop_at, rmw.clone());
    let close_at = steps.len() - 1;
    steps.insert(close_at, rmw);
    steps
}

/// Transactions add no I/O of their own: scenario B committed blind and
/// scenario B committed through transactions (with the extra transactions
/// that write nothing) perform the same mutating operations, byte for byte.
#[test]
fn transactions_do_the_io_of_blind_commits() {
    let opts = Options::small_for_tests();
    let trace = |run: &dyn Fn(&SimVfs) -> (u64, u64)| {
        let disk = SimVfs::new(7);
        disk.start_trace();
        let (acked, attempted) = run(&disk);
        assert_eq!(acked, attempted, "the scenario completes");
        disk.trace().into_iter().map(|t| t.op).collect::<Vec<_>>()
    };
    let blind = trace(&|disk| {
        let steps = scenario_b::<Db>(7, Arc::new(|db: &Db| db.hooks().checkpoint()));
        run_scenario::<Db>(disk, &steps, &opts)
    });
    let txn = trace(&|disk| run_scenario::<ViaTxn>(disk, &scenario_b_with_transactions(), &opts));
    assert!(blind.len() > 100, "scenario B does {} operations", blind.len());
    assert_eq!(blind.len(), txn.len());
    for (i, (b, t)) in blind.iter().zip(&txn).enumerate() {
        assert_eq!(b, t, "operation {i}");
    }
}

/// A crash at every mutating operation of scenario B committed through
/// transactions: the recovered state is the one after some commit that was
/// acknowledged or attempted, and the database works.
#[test]
fn scenario_b_with_transactions_crash_at_every_point() {
    let steps = scenario_b_with_transactions();
    for (kind, durability) in [
        (CrashKind::PowerLoss, Durability::Sync),
        (CrashKind::ProcessKill, Durability::Sync),
        (CrashKind::ProcessKill, Durability::Buffered),
        (CrashKind::ProcessKillKeepPending, Durability::Sync),
    ] {
        every_point_steps::<ViaTxn>(kind, durability, &|_| {}, &steps);
    }
}
