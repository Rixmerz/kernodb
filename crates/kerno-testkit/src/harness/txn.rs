//! Write transaction suite for engines that implement [`TxnEngine`]
//! (ADR-0002, Phase 1; see [`txn_test_suite!`](crate::txn_test_suite), module
//! `shared_txn`). The semantics are in the documentation of
//! [`TxnEngine`](kerno_core::contract::TxnEngine).
//!
//! | Test | What it checks |
//! |---|---|
//! | `read_your_writes_get_and_scan` | a fixed history, then random puts, deletes and range deletions mirrored on a model: after every operation `get`, full and bounded `scan`s of the transaction equal the model, over committed state and over wide ranges that hide whole pages of it |
//! | `abort_leaves_no_trace_and_no_seq` | `abort` after puts, deletes and range deletions: state and sequence unchanged, the next commit gets the next sequence, the write slot is free, a reopen shows nothing |
//! | `drop_is_abort` | the same by dropping the transaction |
//! | `empty_txn_returns_current_seq` | a transaction without successful writes (also after rejected ones) returns the current sequence and consumes none; one with only an empty range is a batch and consumes one; also after a reopen |
//! | `begin_write_is_exclusive` | a second `begin_write` does not return while a transaction is open, and returns, seeing its writes, once it committed, aborted or was dropped |
//! | `blind_commit_waits_for_open_txn` | `Engine::commit` from another thread does not return while a transaction is open; it is applied after the transaction's commit, or alone after its abort |
//! | `snapshot_does_not_see_uncommitted_txn` | snapshots work while a transaction is open and never see its writes; older snapshots stay as they were after its commit |
//! | `txn_sees_latest_committed_state` | transactions and blind commits interleaved: every transaction starts on exactly the latest state |
//! | `txn_limits_enforced` | oversized keys and values and `start > end` return `InvalidArgument` at once, change nothing and leave the transaction usable; the limits themselves are accepted |
//! | `concurrent_counter_increments_no_lost_updates` | 16 threads increment shared counters: a transaction that read `v` commits at sequence `base + 1 + v`, nothing is lost |
//! | `bank_transfer_reads_in_txn` | concurrent transfers that read both balances and the total inside the transaction, with readers checking the total in snapshots: the total never changes |
//! | `panic_inside_txn_aborts_cleanly` | a panic in code that holds a transaction releases the write slot and applies nothing |
//! | `txn_range_delete_then_put` | the interplay of range deletions with writes before and after them, `scan` pagination across hidden ranges, and the commit result |
//! | `txn_commit_crash_at_every_point` | every commit of scenario A (with range deletions) goes through a transaction ([`TxnCommits`]) and is swept with a crash at every mutating operation: power loss and process kill with `Sync`, process kill with `Buffered` |
//!
//! The suite is validated first on `Emulated<LogEngine>`
//! (`kerno-testkit/tests/reference_engine.rs`). Every wait on another thread
//! fails the test instead of hanging after [`WRITERS_TIMEOUT`]; "must stay
//! blocked" is observed for [`WATCH`].

use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options, Snapshot, Txn, TxnEngine};
use kerno_core::{Durability, Error, MAX_KEY_LEN, MAX_VALUE_LEN, Op, Result, WriteBatch};

use super::concurrency::{WRITERS_TIMEOUT, call_with_timeout};
use super::crash;
use super::range::model_scan;
use super::{Model, SuiteConfig, Workload, apply, diff, for_each_seed, open, seeds, show};
use crate::rng::Rng;
use crate::vfs::{CrashKind, SimVfs, Vfs};

/// How long a call that must stay blocked is watched before the test
/// concludes that it does. A broken engine returns at once, so this only
/// bounds the time a correct one spends in each such check.
pub const WATCH: Duration = Duration::from_millis(150);

fn options(durability: Durability) -> Options {
    Options { durability, ..Options::small_for_tests() }
}

fn new_db<E: Engine>(seed: u64) -> (SimVfs, E) {
    let disk = SimVfs::new(seed);
    let db = open::<E>(&disk.as_vfs(), &options(Durability::Sync)).expect("open a new database");
    (disk, db)
}

/// Commits `b` and applies it to `model`.
fn commit_batch<E: Engine>(db: &E, model: &mut Model, b: &WriteBatch) -> u64 {
    let seq = db.commit(b).unwrap_or_else(|e| panic!("commit failed: {e}"));
    apply(model, b);
    seq
}

fn put_batch(pairs: &[(&str, &str)]) -> WriteBatch {
    let mut b = WriteBatch::new();
    for (k, v) in pairs {
        b.put(*k, *v);
    }
    b
}

/// Replays `batch` on `txn` operation by operation.
pub fn apply_in_txn<T: Txn>(txn: &mut T, batch: &WriteBatch) -> Result<()> {
    for op in batch.ops() {
        match op {
            Op::Put { key, value } => txn.put(key, value)?,
            Op::Delete { key } => txn.delete(key)?,
            Op::DeleteRange { start, end } => txn.delete_range(start, end)?,
        }
    }
    Ok(())
}

/// Reads everything `txn` sees through paginated scans (odd page size on
/// purpose, to exercise `Excluded` restarts).
fn read_all_txn<T: Txn>(txn: &T) -> Result<Model> {
    let mut out = Model::new();
    let mut last: Option<Vec<u8>> = None;
    loop {
        let start = last.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
        let page = txn.scan(start, Bound::Unbounded, 97)?;
        let done = page.len() < 97;
        for (k, v) in page {
            if let Some(prev) = &last {
                assert!(k > *prev, "scan returned keys out of order");
            }
            last = Some(k.clone());
            out.insert(k, v);
        }
        if done {
            return Ok(out);
        }
    }
}

fn random_bound<'a>(rng: &mut Rng, key: &'a [u8]) -> Bound<&'a [u8]> {
    match rng.below(4) {
        0 => Bound::Included(key),
        1 => Bound::Excluded(key),
        _ => Bound::Unbounded,
    }
}

/// What `txn` sees must equal `model`: a full scan, `get` of every key in
/// `probes` and some bounded scans between random probes with random kinds of
/// bounds (inverted ones included).
fn check_txn<T: Txn>(txn: &T, model: &Model, probes: &[Vec<u8>], rng: &mut Rng, ctx: &str) {
    let all = read_all_txn(txn).unwrap_or_else(|e| panic!("{ctx}: full scan failed: {e}"));
    assert!(all == *model, "{ctx}: full scan differs from the model: {}", diff(model, &all));
    for k in probes {
        let got = txn.get(k).unwrap_or_else(|e| panic!("{ctx}: get failed: {e}"));
        assert!(got.as_ref() == model.get(k), "{ctx}: get({}) differs from the model", show(k));
    }
    for _ in 0..8 {
        let a = &probes[rng.below(probes.len() as u64) as usize];
        let b = &probes[rng.below(probes.len() as u64) as usize];
        let (start, end) = (random_bound(rng, a), random_bound(rng, b));
        let limit = [1usize, 3, 7, 50][rng.below(4) as usize];
        let got = txn.scan(start, end, limit).unwrap_or_else(|e| panic!("{ctx}: scan failed: {e}"));
        let want = model_scan(model, start, end, limit);
        assert!(
            got == want,
            "{ctx}: scan {start:?}, {end:?} limit {limit} returned {} rows, expected {}",
            got.len(),
            want.len()
        );
    }
}

/// `snap` holds exactly `model`.
fn assert_state<S: Snapshot>(snap: &S, model: &Model, ctx: &str) {
    let all = super::read_all(snap).unwrap_or_else(|e| panic!("{ctx}: full scan failed: {e}"));
    assert!(all == *model, "{ctx}: state differs from the model: {}", diff(model, &all));
}

fn spawn<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> JoinHandle<T> {
    std::thread::Builder::new().name(name.into()).spawn(f).expect("spawn a thread")
}

/// Joins a thread that has already reported through a channel.
fn join<T>(handle: JoinHandle<T>) -> T {
    handle.join().unwrap_or_else(|panic| resume_unwind(panic))
}

/// Runs `f(i)` on `n` threads that start together and returns their results,
/// by thread. Re-raises the panic of a thread; fails after
/// [`WRITERS_TIMEOUT`] if they do not finish.
fn on_threads<R: Send + 'static>(n: usize, ctx: &str, f: impl Fn(usize) -> R + Send + Sync + 'static) -> Vec<R> {
    let f = Arc::new(f);
    let start = Arc::new(Barrier::new(n));
    let (tx, rx) = mpsc::channel();
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let (f, start, tx) = (f.clone(), start.clone(), tx.clone());
            spawn(&format!("txn-{i}"), move || {
                start.wait();
                let r = catch_unwind(AssertUnwindSafe(|| f(i)));
                let _ = tx.send((i, r));
            })
        })
        .collect();
    drop(tx);
    let deadline = Instant::now() + WRITERS_TIMEOUT;
    let mut out: Vec<Option<R>> = (0..n).map(|_| None).collect();
    for _ in 0..n {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((i, Ok(r))) => out[i] = Some(r),
            Ok((_, Err(panic))) => resume_unwind(panic),
            Err(_) => panic!("{ctx}: the threads did not finish within {WRITERS_TIMEOUT:?}: a transaction hangs"),
        }
    }
    handles.into_iter().for_each(join);
    out.into_iter().map(|r| r.expect("one result per thread")).collect()
}

/// How a transaction ends without committing.
#[derive(Clone, Copy, Debug)]
enum End {
    Abort,
    Drop,
}

fn end_txn<T: Txn>(txn: T, end: End) {
    match end {
        End::Abort => txn.abort(),
        End::Drop => drop(txn),
    }
}

fn counter(v: Option<Vec<u8>>) -> u64 {
    v.map_or(0, |v| u64::from_le_bytes(v.as_slice().try_into().expect("a counter has 8 bytes")))
}

/// `E` whose `commit` applies the batch through a transaction: `begin_write`,
/// the operations in order, `commit`. Running the crash sweeps and any other
/// `Engine` suite on `TxnCommits<E>` exercises the transaction commit path
/// with everything the suites check about commits.
pub struct TxnCommits<E>(E);

impl<E: TxnEngine> Engine for TxnCommits<E> {
    type Snapshot = E::Snapshot;

    fn name() -> &'static str {
        "txn-commits"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        E::open(vfs, dir, opts).map(TxnCommits)
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        let mut txn = self.0.begin_write()?;
        apply_in_txn(&mut txn, batch)?;
        txn.commit()
    }

    fn snapshot(&self) -> Result<E::Snapshot> {
        self.0.snapshot()
    }

    fn close(self) -> Result<()> {
        self.0.close()
    }

    fn stats(&self) -> Vec<(String, u64)> {
        self.0.stats()
    }
}

/// A fixed history first, then random operations on a transaction mirrored
/// on a model, with a full comparison after every operation, then the same
/// over data too large for one page of the snapshot, and finally the
/// committed result.
pub fn read_your_writes_get_and_scan<E: TxnEngine>() {
    let (_disk, db) = new_db::<E>(1);
    let mut model = Model::new();
    commit_batch(&db, &mut model, &put_batch(&[("a", "1"), ("b", "2"), ("d", "4")]));
    let mut t = db.begin_write().unwrap();
    assert_eq!(t.get(b"a").unwrap().as_deref(), Some(&b"1"[..]), "committed data is visible");
    t.put(b"b", b"20").unwrap();
    t.put(b"c", b"3").unwrap();
    t.delete(b"d").unwrap();
    assert_eq!(t.get(b"b").unwrap().as_deref(), Some(&b"20"[..]), "an overwrite is visible");
    assert_eq!(t.get(b"c").unwrap().as_deref(), Some(&b"3"[..]), "an insert is visible");
    assert_eq!(t.get(b"d").unwrap(), None, "a delete is visible");
    let rows = t.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap();
    let keys: Vec<&[u8]> = rows.iter().map(|(k, _)| k.as_slice()).collect();
    assert_eq!(keys, [&b"a"[..], b"b", b"c"], "scan merges the writes with the committed keys");
    assert_eq!(rows[1].1, b"20");
    let rows = t.scan(Bound::Excluded(b"a"), Bound::Included(b"c"), 1).unwrap();
    assert_eq!(rows, [(b"b".to_vec(), b"20".to_vec())], "limit applies to the merged view");
    let rows = t.scan(Bound::Excluded(b"b"), Bound::Unbounded, 10).unwrap();
    assert_eq!(rows, [(b"c".to_vec(), b"3".to_vec())], "pagination resumes after the last key");
    t.abort();

    let keys: Vec<Vec<u8>> = (0..60).map(|i| format!("k{i:03}").into_bytes()).collect();
    for_each_seed("read_your_writes_get_and_scan", seeds(12), |seed| {
        let ctx = format!("seed {seed}");
        let mut rng = Rng::new(seed);
        let (_disk, db) = new_db::<E>(seed);
        let mut model = Model::new();
        let mut base = WriteBatch::new();
        for k in &keys {
            if rng.chance(2, 3) {
                let len = rng.range(0, 50) as usize;
                base.put(k.clone(), rng.bytes(len));
            }
        }
        let base_seq = if base.is_empty() { 0 } else { commit_batch(&db, &mut model, &base) };
        let mut t = db.begin_write().unwrap();
        for step in 0..120 {
            let ctx = format!("{ctx}, step {step}");
            let mut b = WriteBatch::new();
            let key = &keys[rng.below(keys.len() as u64) as usize];
            match rng.below(10) {
                0..=3 => {
                    let len = rng.range(0, 200) as usize;
                    b.put(key.clone(), rng.bytes(len));
                }
                4 | 5 => {
                    b.delete(key.clone());
                }
                6 => {
                    let other = &keys[rng.below(keys.len() as u64) as usize];
                    let (lo, hi) = if key <= other { (key, other) } else { (other, key) };
                    b.delete_range(lo.clone(), hi.clone());
                }
                _ => {}
            }
            apply_in_txn(&mut t, &b).unwrap_or_else(|e| panic!("{ctx}: write failed: {e}"));
            apply(&mut model, &b);
            check_txn(&t, &model, &keys, &mut rng, &ctx);
        }
        let seq = t.commit().unwrap();
        assert_eq!(seq, base_seq + 1, "{ctx}: commit sequence");
        assert_state(&db.snapshot().unwrap(), &model, &format!("{ctx}: after commit"));
    });

    // More committed keys than one page of the snapshot, and ranges that hide
    // whole pages of them.
    let (_disk, db) = new_db::<E>(2);
    let mut model = Model::new();
    let mut base = WriteBatch::new();
    for i in 0..3000 {
        base.put(format!("w{i:05}"), format!("{i}"));
    }
    commit_batch(&db, &mut model, &base);
    let mut t = db.begin_write().unwrap();
    let mut b = WriteBatch::new();
    b.delete_range("w00100", "w02600").put("w00500", "back").delete("w02999").put("w02600x", "new");
    apply_in_txn(&mut t, &b).unwrap();
    apply(&mut model, &b);
    let probes: Vec<Vec<u8>> =
        [0, 99, 100, 500, 1500, 2599, 2600, 2601, 2998, 2999].iter().map(|i| format!("w{i:05}").into_bytes()).collect();
    let mut rng = Rng::new(9);
    check_txn(&t, &model, &probes, &mut rng, "wide range");
    let first = t.scan(Bound::Included(b"w00098"), Bound::Unbounded, 4).unwrap();
    let want = model_scan(&model, Bound::Included(b"w00098"), Bound::Unbounded, 4);
    assert_eq!(first, want, "a limit that crosses a hidden range of 2500 keys");
    let inside = t.scan(Bound::Included(b"w01000"), Bound::Excluded(b"w02000"), 10).unwrap();
    assert!(inside.is_empty(), "a scan entirely inside a deleted range: {} rows", inside.len());
    assert_eq!(t.commit().unwrap(), 2);
    assert_state(&db.snapshot().unwrap(), &model, "wide range, after commit");
}

fn discards_writes<E: TxnEngine>(end: End) {
    let ctx = format!("{end:?}");
    let (disk, db) = new_db::<E>(3);
    let mut model = Model::new();
    commit_batch(&db, &mut model, &put_batch(&[("a", "1"), ("b", "2"), ("c", "3")]));
    let db = Arc::new(db);
    {
        let mut t = db.begin_write().unwrap();
        t.put(b"a", b"changed").unwrap();
        t.put(b"new", b"x").unwrap();
        t.delete(b"b").unwrap();
        t.delete_range(b"c", b"z").unwrap();
        assert_eq!(t.get(b"a").unwrap().as_deref(), Some(&b"changed"[..]));
        end_txn(t, end);
    }
    // The write slot is free again, and nothing was applied or consumed.
    let seq = call_with_timeout(&db, "begin_write after the transaction ended", &ctx, |db: &E| {
        db.begin_write().and_then(|t| t.commit())
    })
    .unwrap();
    assert_eq!(seq, 1, "{ctx}: an empty transaction returns the current sequence");
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.seq(), 1, "{ctx}: the discarded transaction consumed a sequence");
    assert_state(&snap, &model, &ctx);
    drop(snap);
    let mut b = WriteBatch::new();
    b.put("d", "4");
    assert_eq!(commit_batch(&*db, &mut model, &b), 2, "{ctx}: the next commit gets the next sequence");
    // And nothing was written behind the engine's back: a clean reopen.
    let Ok(db) = Arc::try_unwrap(db) else { panic!("{ctx}: a thread still holds the handle") };
    db.close().unwrap();
    let db: E = open(&disk.as_vfs(), &options(Durability::Sync)).unwrap();
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.seq(), 2, "{ctx}: sequence after reopening");
    assert_state(&snap, &model, &format!("{ctx}: after reopening"));
}

/// `abort` after writes of every kind leaves no trace and consumes no
/// sequence.
pub fn abort_leaves_no_trace_and_no_seq<E: TxnEngine>() {
    discards_writes::<E>(End::Abort);
}

/// Dropping a transaction is aborting it.
pub fn drop_is_abort<E: TxnEngine>() {
    discards_writes::<E>(End::Drop);
}

/// A transaction without successful writes returns the current sequence and
/// consumes none.
pub fn empty_txn_returns_current_seq<E: TxnEngine>() {
    let (disk, db) = new_db::<E>(4);
    assert_eq!(db.begin_write().unwrap().commit().unwrap(), 0, "a new database is at sequence 0");
    let mut model = Model::new();
    for i in 1..=3 {
        let key = format!("k{i}");
        assert_eq!(commit_batch(&db, &mut model, &put_batch(&[(key.as_str(), "v")])), i);
    }
    let t = db.begin_write().unwrap();
    let _ = t.get(b"k1").unwrap();
    let _ = t.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap();
    assert_eq!(t.commit().unwrap(), 3, "reads only");
    let mut t = db.begin_write().unwrap();
    assert!(matches!(t.put(&vec![b'x'; MAX_KEY_LEN + 1], b"v"), Err(Error::InvalidArgument(_))));
    assert_eq!(t.commit().unwrap(), 3, "every write was rejected");
    assert_eq!(commit_batch(&db, &mut model, &put_batch(&[("k4", "v")])), 4, "no sequence was consumed");
    // A batch made only of an empty range is a commit; so is the transaction.
    let mut t = db.begin_write().unwrap();
    t.delete_range(b"q", b"q").unwrap();
    assert_eq!(t.commit().unwrap(), 5, "an empty range is a write");
    assert_state(&db.snapshot().unwrap(), &model, "after the empty range");
    assert_eq!(db.begin_write().unwrap().commit().unwrap(), 5);
    db.close().unwrap();
    let db: E = open(&disk.as_vfs(), &options(Durability::Sync)).unwrap();
    assert_eq!(db.begin_write().unwrap().commit().unwrap(), 5, "after a reopen it is the recovered sequence");
}

/// The write slot is exclusive between transactions.
pub fn begin_write_is_exclusive<E: TxnEngine>() {
    for end in [None, Some(End::Abort), Some(End::Drop)] {
        let ctx = format!("first transaction ends by {}", end.map_or("commit".into(), |e| format!("{e:?}")));
        let (_disk, db) = new_db::<E>(5);
        let db = Arc::new(db);
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let first = {
            let db = db.clone();
            spawn("first-txn", move || {
                let mut t = db.begin_write().unwrap();
                t.put(b"a", b"1").unwrap();
                held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                match end {
                    None => Some(t.commit().unwrap()),
                    Some(end) => {
                        end_txn(t, end);
                        None
                    }
                }
            })
        };
        held_rx.recv_timeout(WRITERS_TIMEOUT).unwrap_or_else(|_| panic!("{ctx}: the first transaction never began"));
        let (got_tx, got_rx) = mpsc::channel();
        let second = {
            let db = db.clone();
            spawn("second-txn", move || {
                let t = db.begin_write().unwrap();
                got_tx.send(t.get(b"a").unwrap()).unwrap();
                t.commit().unwrap()
            })
        };
        match got_rx.recv_timeout(WATCH) {
            Err(RecvTimeoutError::Timeout) => {}
            other => panic!("{ctx}: begin_write returned while another transaction was open: {other:?}"),
        }
        release_tx.send(()).unwrap();
        let seen = got_rx
            .recv_timeout(WRITERS_TIMEOUT)
            .unwrap_or_else(|_| panic!("{ctx}: begin_write never returned after the first transaction ended"));
        let committed = join(first);
        assert_eq!(
            seen,
            committed.map(|_| b"1".to_vec()),
            "{ctx}: the second transaction sees exactly what was committed"
        );
        assert_eq!(join(second), committed.unwrap_or(0), "{ctx}: the second transaction was empty");
    }
}

/// A blind commit waits for an open transaction (D-03).
pub fn blind_commit_waits_for_open_txn<E: TxnEngine>() {
    for commit_txn in [true, false] {
        let ctx = if commit_txn { "the transaction commits" } else { "the transaction aborts" };
        let (_disk, db) = new_db::<E>(6);
        let mut model = Model::new();
        commit_batch(&db, &mut model, &put_batch(&[("base", "1")]));
        let db = Arc::new(db);
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let txn = {
            let db = db.clone();
            spawn("txn", move || {
                let mut t = db.begin_write().unwrap();
                t.put(b"txn", b"1").unwrap();
                held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                if commit_txn {
                    Some(t.commit().unwrap())
                } else {
                    t.abort();
                    None
                }
            })
        };
        held_rx.recv_timeout(WRITERS_TIMEOUT).unwrap_or_else(|_| panic!("{ctx}: the transaction never began"));
        let (done_tx, done_rx) = mpsc::channel();
        let blind = {
            let db = db.clone();
            spawn("blind", move || {
                let seq = db.commit(&put_batch(&[("blind", "1")])).unwrap();
                done_tx.send(seq).unwrap();
            })
        };
        match done_rx.recv_timeout(WATCH) {
            Err(RecvTimeoutError::Timeout) => {}
            other => panic!("{ctx}: a blind commit returned while a transaction was open: {other:?}"),
        }
        release_tx.send(()).unwrap();
        let blind_seq = done_rx
            .recv_timeout(WRITERS_TIMEOUT)
            .unwrap_or_else(|_| panic!("{ctx}: the blind commit never returned after the transaction ended"));
        let txn_seq = join(txn);
        join(blind);
        match txn_seq {
            Some(seq) => {
                assert_eq!(seq, 2, "{ctx}: the transaction committed first");
                assert_eq!(blind_seq, 3, "{ctx}: the blind commit follows it");
                model.insert(b"txn".to_vec(), b"1".to_vec());
            }
            None => assert_eq!(blind_seq, 2, "{ctx}: nothing was committed before the blind commit"),
        }
        model.insert(b"blind".to_vec(), b"1".to_vec());
        assert_state(&db.snapshot().unwrap(), &model, ctx);
    }
}

/// Readers are not blocked by, and never see, an open transaction.
pub fn snapshot_does_not_see_uncommitted_txn<E: TxnEngine>() {
    let (_disk, db) = new_db::<E>(7);
    let mut model = Model::new();
    commit_batch(&db, &mut model, &put_batch(&[("a", "1")]));
    let before = db.snapshot().unwrap();
    let mut t = db.begin_write().unwrap();
    t.put(b"a", b"2").unwrap();
    t.put(b"b", b"3").unwrap();
    // The thread that holds the transaction can still read.
    let during = db.snapshot().unwrap();
    assert_eq!(during.seq(), 1);
    assert_state(&during, &model, "a snapshot taken during a transaction");
    assert_eq!(t.commit().unwrap(), 2);
    assert_state(&before, &model, "a snapshot taken before the transaction");
    assert_state(&during, &model, "a snapshot taken during the transaction, after its commit");
    let after = db.snapshot().unwrap();
    assert_eq!(after.seq(), 2);
    let mut committed = model.clone();
    committed.insert(b"a".to_vec(), b"2".to_vec());
    committed.insert(b"b".to_vec(), b"3".to_vec());
    assert_state(&after, &committed, "a snapshot taken after the commit");
    let mut t = db.begin_write().unwrap();
    t.put(b"c", b"4").unwrap();
    let during = db.snapshot().unwrap();
    t.abort();
    assert_state(&during, &committed, "a snapshot taken during an aborted transaction");
    assert_state(&db.snapshot().unwrap(), &committed, "after the abort");
}

/// Transactions and blind commits interleaved: each transaction starts on
/// the latest state.
pub fn txn_sees_latest_committed_state<E: TxnEngine>() {
    for_each_seed("txn_sees_latest_committed_state", seeds(8), |seed| {
        let mut rng = Rng::new(seed);
        let mut w = Workload::new(seed ^ 0x7A11);
        w.key_space = 80;
        let (_disk, db) = new_db::<E>(seed);
        let mut model = Model::new();
        let mut seq = 0;
        let probes: Vec<Vec<u8>> = (0..80).map(|i| format!("key-{i:08}").into_bytes()).collect();
        for round in 0..30 {
            let ctx = format!("seed {seed} round {round}");
            let b = w.batch(&model);
            if rng.chance(1, 2) {
                seq = commit_batch(&db, &mut model, &b);
                continue;
            }
            let mut t = db.begin_write().unwrap();
            check_txn(&t, &model, &probes, &mut rng, &format!("{ctx}: at begin"));
            apply_in_txn(&mut t, &b).unwrap();
            apply(&mut model, &b);
            check_txn(&t, &model, &probes, &mut rng, &format!("{ctx}: after its writes"));
            seq += 1;
            assert_eq!(t.commit().unwrap(), seq, "{ctx}: commit sequence");
            let snap = db.snapshot().unwrap();
            assert_eq!(snap.seq(), seq, "{ctx}");
            assert_state(&snap, &model, &ctx);
        }
    });
}

/// Limits are checked when the operation is made.
pub fn txn_limits_enforced<E: TxnEngine>() {
    let (_disk, db) = new_db::<E>(8);
    let long_key = vec![b'k'; MAX_KEY_LEN + 1];
    let long_value = vec![0u8; MAX_VALUE_LEN + 1];
    let invalid = |r: Result<()>, what: &str| {
        assert!(matches!(r, Err(Error::InvalidArgument(_))), "{what}: expected InvalidArgument, got {r:?}");
    };
    let mut t = db.begin_write().unwrap();
    invalid(t.put(&long_key, b"v"), "put with an oversized key");
    invalid(t.put(b"k", &long_value), "put with an oversized value");
    invalid(t.delete(&long_key), "delete with an oversized key");
    invalid(t.delete_range(b"b", b"a"), "delete_range with start > end");
    invalid(t.delete_range(&long_key, b"z"), "delete_range with an oversized start");
    invalid(t.delete_range(b"a", &long_key), "delete_range with an oversized end");
    assert_eq!(t.get(b"k").unwrap(), None, "a rejected put has no effect");
    assert!(t.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap().is_empty(), "nothing was written");
    // The limits themselves are fine, and the transaction is still usable.
    let max_key = vec![b'm'; MAX_KEY_LEN];
    let max_value = vec![7u8; MAX_VALUE_LEN];
    t.put(&max_key, &max_value).unwrap();
    t.put(b"ok", b"1").unwrap();
    assert_eq!(t.get(&max_key).unwrap().as_deref(), Some(max_value.as_slice()));
    assert_eq!(t.commit().unwrap(), 1);
    let mut model = Model::new();
    model.insert(max_key, max_value);
    model.insert(b"ok".to_vec(), b"1".to_vec());
    assert_state(&db.snapshot().unwrap(), &model, "after the commit");
    let mut t = db.begin_write().unwrap();
    invalid(t.put(&long_key, b"v"), "put with an oversized key, again");
    assert_eq!(t.commit().unwrap(), 1, "a transaction whose every write was rejected is empty");
}

/// Sixteen threads increment counters in transactions.
pub fn concurrent_counter_increments_no_lost_updates<E: TxnEngine>() {
    const THREADS: usize = 16;
    const INCREMENTS: u64 = 20;
    let ctx = "concurrent_counter_increments_no_lost_updates";
    let (_disk, db) = new_db::<E>(9);
    let mut model = Model::new();
    let mut zero = WriteBatch::new();
    zero.put("counter", 0u64.to_le_bytes().to_vec());
    commit_batch(&db, &mut model, &zero);
    let db = Arc::new(db);
    let runs = {
        let db = db.clone();
        on_threads(THREADS, ctx, move |i| {
            let own = format!("own/{i:02}").into_bytes();
            let mut seen = Vec::new();
            for _ in 0..INCREMENTS {
                let mut t = db.begin_write().unwrap();
                let shared = counter(t.get(b"counter").unwrap());
                let mine = counter(t.get(&own).unwrap());
                t.put(b"counter", &(shared + 1).to_le_bytes()).unwrap();
                t.put(&own, &(mine + 1).to_le_bytes()).unwrap();
                seen.push((shared, t.commit().unwrap()));
            }
            seen
        })
    };
    let total = THREADS as u64 * INCREMENTS;
    let mut values: Vec<u64> = Vec::new();
    for (i, seen) in runs.iter().enumerate() {
        for &(read, seq) in seen {
            // Nobody commits between the read and the commit of a
            // transaction, so it commits right after the `read` increments
            // before it (the zeroing commit was sequence 1).
            assert_eq!(
                seq,
                read + 2,
                "{ctx}: thread {i} read {read} and committed at sequence {seq}: another commit came between"
            );
            values.push(read);
        }
    }
    values.sort_unstable();
    assert_eq!(values, (0..total).collect::<Vec<_>>(), "{ctx}: every increment read a distinct value");
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.seq(), total + 1, "{ctx}");
    assert_eq!(counter(snap.get(b"counter").unwrap()), total, "{ctx}: lost updates");
    for i in 0..THREADS {
        let own = snap.get(format!("own/{i:02}").as_bytes()).unwrap();
        assert_eq!(counter(own), INCREMENTS, "{ctx}: thread {i}'s counter");
    }
}

fn account(i: usize) -> Vec<u8> {
    format!("acct/{i:02}").into_bytes()
}

/// The sum of the accounts in `rows` (a scan of the `acct/` prefix).
fn total_balance(rows: &[(Vec<u8>, Vec<u8>)], ctx: &str) -> u64 {
    assert_eq!(rows.len(), BANK_ACCOUNTS, "{ctx}: an account is missing");
    rows.iter().map(|(_, v)| counter(Some(v.clone()))).sum()
}

const BANK_ACCOUNTS: usize = 8;
const BANK_OPENING: u64 = 1000;

/// Concurrent transfers between accounts that read the balances (and the
/// total) in the transaction, with readers checking the total in snapshots.
pub fn bank_transfer_reads_in_txn<E: TxnEngine>() {
    const WRITERS: usize = 6;
    const TRANSFERS: usize = 30;
    let ctx = "bank_transfer_reads_in_txn";
    let (_disk, db) = new_db::<E>(10);
    let mut opening = WriteBatch::new();
    for i in 0..BANK_ACCOUNTS {
        opening.put(account(i), BANK_OPENING.to_le_bytes().to_vec());
    }
    db.commit(&opening).unwrap();
    let total = BANK_ACCOUNTS as u64 * BANK_OPENING;
    let db = Arc::new(db);
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..2)
        .map(|r| {
            let (db, stop) = (db.clone(), stop.clone());
            spawn(&format!("reader-{r}"), move || {
                let mut checks = 0u64;
                loop {
                    let finished = stop.load(Ordering::SeqCst);
                    let snap = db.snapshot().unwrap();
                    let rows = snap.scan(Bound::Included(b"acct/"), Bound::Excluded(b"acct0"), 100).unwrap();
                    assert_eq!(
                        total_balance(&rows, "a snapshot"),
                        total,
                        "a snapshot saw a transfer half done (seq {})",
                        snap.seq()
                    );
                    checks += 1;
                    if finished {
                        return checks;
                    }
                }
            })
        })
        .collect();
    let run = catch_unwind(AssertUnwindSafe(|| {
        let db = db.clone();
        on_threads(WRITERS, ctx, move |w| {
            let mut rng = Rng::new(0xBA9C ^ w as u64);
            let (mut committed, mut aborted) = (0u64, 0u64);
            for _ in 0..TRANSFERS {
                let from = rng.below(BANK_ACCOUNTS as u64) as usize;
                let to = (from + 1 + rng.below(BANK_ACCOUNTS as u64 - 1) as usize) % BANK_ACCOUNTS;
                let amount = rng.range(1, 400);
                let mut t = db.begin_write().unwrap();
                let rows = t.scan(Bound::Included(b"acct/"), Bound::Excluded(b"acct0"), 100).unwrap();
                assert_eq!(total_balance(&rows, "a transaction"), total, "a transaction saw a transfer half done");
                let (a, b) = (counter(t.get(&account(from)).unwrap()), counter(t.get(&account(to)).unwrap()));
                if a < amount {
                    t.abort();
                    aborted += 1;
                    continue;
                }
                t.put(&account(from), &(a - amount).to_le_bytes()).unwrap();
                t.put(&account(to), &(b + amount).to_le_bytes()).unwrap();
                t.commit().unwrap();
                committed += 1;
            }
            (committed, aborted)
        })
    }));
    stop.store(true, Ordering::SeqCst);
    let counts = run.unwrap_or_else(|panic| resume_unwind(panic));
    let checks: u64 = readers.into_iter().map(join).sum();
    assert!(checks > 0);
    let committed: u64 = counts.iter().map(|c| c.0).sum();
    let snap = db.snapshot().unwrap();
    let rows = snap.scan(Bound::Included(b"acct/"), Bound::Excluded(b"acct0"), 100).unwrap();
    assert_eq!(total_balance(&rows, ctx), total, "{ctx}: money was created or destroyed");
    assert_eq!(snap.seq(), 1 + committed, "{ctx}: aborted transfers must not consume sequences");
}

/// A panic in code that holds a transaction releases the write slot.
pub fn panic_inside_txn_aborts_cleanly<E: TxnEngine>() {
    let ctx = "panic_inside_txn_aborts_cleanly";
    let (_disk, db) = new_db::<E>(11);
    let mut model = Model::new();
    commit_batch(&db, &mut model, &put_batch(&[("a", "1")]));
    let db = Arc::new(db);
    let r = catch_unwind(AssertUnwindSafe(|| {
        let mut t = db.begin_write().unwrap();
        t.put(b"a", b"2").unwrap();
        t.put(b"b", b"2").unwrap();
        assert_eq!(t.get(b"b").unwrap().as_deref(), Some(&b"2"[..]));
        panic!("user code panics inside a transaction");
    }));
    assert!(r.is_err());
    let seq = call_with_timeout(&db, "begin_write after a panic inside a transaction", ctx, |db: &E| {
        let t = db.begin_write().unwrap();
        assert_eq!(t.get(b"a").unwrap().as_deref(), Some(&b"1"[..]), "the panicked transaction's write is visible");
        assert_eq!(t.get(b"b").unwrap(), None, "the panicked transaction's insert is visible");
        t.commit().unwrap()
    });
    assert_eq!(seq, 1, "{ctx}: the panicked transaction consumed a sequence");
    assert_state(&db.snapshot().unwrap(), &model, ctx);
    assert_eq!(commit_batch(&*db, &mut model, &put_batch(&[("c", "3")])), 2, "{ctx}: blind commit after the panic");
}

/// Range deletions among the writes of a transaction.
pub fn txn_range_delete_then_put<E: TxnEngine>() {
    let (_disk, db) = new_db::<E>(12);
    let mut model = Model::new();
    let letters = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let mut base = WriteBatch::new();
    for l in letters {
        base.put(l, format!("old-{l}"));
    }
    commit_batch(&db, &mut model, &base);
    let probes: Vec<Vec<u8>> = ["", "a", "b", "bb", "c", "cc", "d", "e", "f", "g", "h", "i", "j", "k", "z"]
        .iter()
        .map(|k| k.as_bytes().to_vec())
        .collect();
    let mut rng = Rng::new(12);
    let mut t = db.begin_write().unwrap();
    // Applies `b` to the transaction and the model, and compares them.
    fn step<T: Txn>(t: &mut T, model: &mut Model, b: &WriteBatch, probes: &[Vec<u8>], rng: &mut Rng, what: &str) {
        apply_in_txn(t, b).unwrap_or_else(|e| panic!("{what}: {e}"));
        apply(model, b);
        check_txn(t, model, probes, rng, what);
    }
    let mut b = WriteBatch::new();
    b.put("c", "new-c").put("cc", "new-cc");
    step(&mut t, &mut model, &b, &probes, &mut rng, "puts");
    let mut b = WriteBatch::new();
    b.delete_range("b", "e");
    step(&mut t, &mut model, &b, &probes, &mut rng, "a range over committed keys and written ones");
    for hidden in ["b", "c", "cc", "d"] {
        assert_eq!(t.get(hidden.as_bytes()).unwrap(), None, "{hidden} is inside the range");
    }
    assert_eq!(t.get(b"e").unwrap().as_deref(), Some(&b"old-e"[..]), "the end of a range is excluded");
    let mut b = WriteBatch::new();
    b.put("c", "again").delete("f").delete_range("h", "h");
    step(&mut t, &mut model, &b, &probes, &mut rng, "a put after the range, a delete, an empty range");
    assert_eq!(t.get(b"c").unwrap().as_deref(), Some(&b"again"[..]), "a key written after the range is visible");
    let rows = t.scan(Bound::Included(b"b"), Bound::Excluded(b"g"), 10).unwrap();
    let keys: Vec<&[u8]> = rows.iter().map(|(k, _)| k.as_slice()).collect();
    assert_eq!(keys, [&b"c"[..], b"e"], "scan across the range");
    let rows = t.scan(Bound::Excluded(b"c"), Bound::Unbounded, 1).unwrap();
    assert_eq!(rows, [(b"e".to_vec(), b"old-e".to_vec())], "a page that resumes before a hidden range");
    let mut b = WriteBatch::new();
    b.put("bb", "late").delete_range("", "zzz").put("z", "last");
    step(&mut t, &mut model, &b, &probes, &mut rng, "everything deleted, then a put");
    assert_eq!(model.len(), 1);
    assert_eq!(t.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap(), [(b"z".to_vec(), b"last".to_vec())]);
    assert_eq!(t.commit().unwrap(), 2);
    assert_state(&db.snapshot().unwrap(), &model, "after commit");
}

/// Every commit of scenario A (with range deletions in its batches) goes
/// through a transaction, with a crash at every mutating operation.
pub fn txn_commit_crash_at_every_point<E: TxnEngine>() {
    let cfg = SuiteConfig::with_range_deletes(40);
    for (kind, durability) in [
        (CrashKind::PowerLoss, Durability::Sync),
        (CrashKind::ProcessKill, Durability::Sync),
        (CrashKind::ProcessKill, Durability::Buffered),
    ] {
        crash::every_point_with_config::<TxnCommits<E>>(kind, durability, &cfg);
    }
}

/// Generates the transaction suite (module `shared_txn`) for an engine type
/// that implements [`TxnEngine`](kerno_core::contract::TxnEngine):
///
/// ```ignore
/// kerno_testkit::txn_test_suite!(MyEngine);
/// kerno_testkit::txn_test_suite!(kerno_testkit::emulated::Emulated<OtherEngine>);
/// ```
///
/// See the [module documentation](crate::harness::txn) for what each test
/// checks.
#[macro_export]
macro_rules! txn_test_suite {
    ($engine:ty) => {
        mod shared_txn {
            use super::*;
            use $crate::harness::txn as t;
            #[test]
            fn read_your_writes_get_and_scan() {
                t::read_your_writes_get_and_scan::<$engine>();
            }
            #[test]
            fn abort_leaves_no_trace_and_no_seq() {
                t::abort_leaves_no_trace_and_no_seq::<$engine>();
            }
            #[test]
            fn drop_is_abort() {
                t::drop_is_abort::<$engine>();
            }
            #[test]
            fn empty_txn_returns_current_seq() {
                t::empty_txn_returns_current_seq::<$engine>();
            }
            #[test]
            fn begin_write_is_exclusive() {
                t::begin_write_is_exclusive::<$engine>();
            }
            #[test]
            fn blind_commit_waits_for_open_txn() {
                t::blind_commit_waits_for_open_txn::<$engine>();
            }
            #[test]
            fn snapshot_does_not_see_uncommitted_txn() {
                t::snapshot_does_not_see_uncommitted_txn::<$engine>();
            }
            #[test]
            fn txn_sees_latest_committed_state() {
                t::txn_sees_latest_committed_state::<$engine>();
            }
            #[test]
            fn txn_limits_enforced() {
                t::txn_limits_enforced::<$engine>();
            }
            #[test]
            fn concurrent_counter_increments_no_lost_updates() {
                t::concurrent_counter_increments_no_lost_updates::<$engine>();
            }
            #[test]
            fn bank_transfer_reads_in_txn() {
                t::bank_transfer_reads_in_txn::<$engine>();
            }
            #[test]
            fn panic_inside_txn_aborts_cleanly() {
                t::panic_inside_txn_aborts_cleanly::<$engine>();
            }
            #[test]
            fn txn_range_delete_then_put() {
                t::txn_range_delete_then_put::<$engine>();
            }
            #[test]
            fn txn_commit_crash_at_every_point() {
                t::txn_commit_crash_at_every_point::<$engine>();
            }
        }
    };
}

/// Engines that break one rule of [`TxnEngine`] each, to show that the suite
/// fails for every rule (the suite is validated on `Emulated<LogEngine>`, which
/// follows them all).
#[cfg(test)]
mod mutants {
    use std::marker::PhantomData;

    use parking_lot::{Mutex, MutexGuard};

    use super::*;
    use crate::emulated::{Emulated, EmulatedTxn};
    use crate::reference::{LogEngine, LogSnapshot};

    /// Transactions without a read-your-writes view that lock (or not) the
    /// write slot: `TXN_GATE` makes `begin_write` exclusive, `BLIND_GATE`
    /// makes blind commits wait for open transactions.
    pub struct Naive<const TXN_GATE: bool, const BLIND_GATE: bool> {
        inner: LogEngine,
        gate: Mutex<()>,
    }

    impl<const T: bool, const B: bool> Engine for Naive<T, B> {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-naive-txn"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            Ok(Naive { inner: LogEngine::open(vfs, dir, opts)?, gate: Mutex::new(()) })
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            let _gate = B.then(|| self.gate.lock());
            self.inner.commit(batch)
        }

        fn snapshot(&self) -> Result<LogSnapshot> {
            self.inner.snapshot()
        }

        fn close(self) -> Result<()> {
            self.inner.close()
        }
    }

    impl<const T: bool, const B: bool> TxnEngine for Naive<T, B> {
        type Txn<'a>
            = NaiveTxn<'a, T, B>
        where
            Self: 'a;

        fn begin_write(&self) -> Result<NaiveTxn<'_, T, B>> {
            let gate = T.then(|| self.gate.lock());
            let snap = self.inner.snapshot()?;
            if !T {
                // Widen the read-to-commit window so concurrent writers
                // overlap on every run and the suite reliably sees the
                // lost update this mutant allows.
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(NaiveTxn { db: self, snap, batch: WriteBatch::new(), _gate: gate })
        }
    }

    pub struct NaiveTxn<'a, const T: bool, const B: bool> {
        db: &'a Naive<T, B>,
        snap: LogSnapshot,
        batch: WriteBatch,
        _gate: Option<MutexGuard<'a, ()>>,
    }

    impl<const T: bool, const B: bool> Txn for NaiveTxn<'_, T, B> {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            self.snap.get(key)
        }

        fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
            self.snap.scan(start, end, limit)
        }

        fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
            self.batch.put(key, value);
            Ok(())
        }

        fn delete(&mut self, key: &[u8]) -> Result<()> {
            self.batch.delete(key);
            Ok(())
        }

        fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()> {
            self.batch.delete_range(start, end);
            Ok(())
        }

        fn commit(self) -> Result<u64> {
            if self.batch.is_empty() {
                return Ok(self.snap.seq());
            }
            self.db.inner.commit(&self.batch)
        }
    }

    /// Which rule a [`Mutant`] breaks (none for [`Honest`]).
    pub trait Mode: Send + Sync + 'static {
        /// `get` and `scan` read the committed state only.
        const STALE_READS: bool = false;
        /// Dropping or aborting the transaction commits it.
        const COMMIT_ON_DROP: bool = false;
        /// A rejected `put` reports success.
        const SWALLOW_PUT_ERRORS: bool = false;
        /// A transaction without writes consumes a sequence.
        const EMPTY_CONSUMES: bool = false;
    }

    pub struct Honest;
    impl Mode for Honest {}
    pub struct StaleReads;
    impl Mode for StaleReads {
        const STALE_READS: bool = true;
    }
    pub struct CommitOnDrop;
    impl Mode for CommitOnDrop {
        const COMMIT_ON_DROP: bool = true;
    }
    pub struct SwallowPutErrors;
    impl Mode for SwallowPutErrors {
        const SWALLOW_PUT_ERRORS: bool = true;
    }
    pub struct EmptyConsumes;
    impl Mode for EmptyConsumes {
        const EMPTY_CONSUMES: bool = true;
    }

    /// `Emulated<LogEngine>` with one rule of [`Mode`] broken.
    pub struct Mutant<M>(Emulated<LogEngine>, PhantomData<M>);

    impl<M: Mode> Engine for Mutant<M> {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-txn"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            Ok(Mutant(Emulated::open(vfs, dir, opts)?, PhantomData))
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            self.0.commit(batch)
        }

        fn snapshot(&self) -> Result<LogSnapshot> {
            self.0.snapshot()
        }

        fn close(self) -> Result<()> {
            self.0.close()
        }
    }

    impl<M: Mode> TxnEngine for Mutant<M> {
        type Txn<'a>
            = MutantTxn<'a, M>
        where
            Self: 'a;

        fn begin_write(&self) -> Result<MutantTxn<'_, M>> {
            Ok(MutantTxn { db: &self.0, txn: Some(self.0.begin_write()?), writes: 0, mode: PhantomData })
        }
    }

    pub struct MutantTxn<'a, M: Mode> {
        db: &'a Emulated<LogEngine>,
        txn: Option<EmulatedTxn<'a, LogEngine>>,
        writes: usize,
        mode: PhantomData<M>,
    }

    impl<M: Mode> MutantTxn<'_, M> {
        fn txn(&self) -> &EmulatedTxn<'_, LogEngine> {
            self.txn.as_ref().expect("the transaction has not ended")
        }

        fn wrote(&mut self, r: Result<()>) -> Result<()> {
            self.writes += usize::from(r.is_ok());
            if M::SWALLOW_PUT_ERRORS { Ok(()) } else { r }
        }
    }

    impl<M: Mode> Txn for MutantTxn<'_, M> {
        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
            if M::STALE_READS { self.db.snapshot()?.get(key) } else { self.txn().get(key) }
        }

        fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
            if M::STALE_READS {
                self.db.snapshot()?.scan(start, end, limit)
            } else {
                self.txn().scan(start, end, limit)
            }
        }

        fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
            let r = self.txn.as_mut().expect("the transaction has not ended").put(key, value);
            self.wrote(r)
        }

        fn delete(&mut self, key: &[u8]) -> Result<()> {
            let r = self.txn.as_mut().expect("the transaction has not ended").delete(key);
            self.wrote(r)
        }

        fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()> {
            let r = self.txn.as_mut().expect("the transaction has not ended").delete_range(start, end);
            self.wrote(r)
        }

        fn commit(mut self) -> Result<u64> {
            let mut txn = self.txn.take().expect("the transaction has not ended");
            if M::EMPTY_CONSUMES && self.writes == 0 {
                // A key no test uses: the state does not change, a sequence is consumed.
                txn.delete(b"\0\0\0absent")?;
            }
            txn.commit()
        }
    }

    impl<M: Mode> Drop for MutantTxn<'_, M> {
        fn drop(&mut self) {
            if M::COMMIT_ON_DROP
                && let Some(txn) = self.txn.take()
            {
                let _ = txn.commit();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::mutants::{CommitOnDrop, EmptyConsumes, Honest, Mutant, Naive, StaleReads, SwallowPutErrors};
    use super::*;
    use crate::emulated::Emulated;
    use crate::harness::concurrency::mutants::NoFsync;
    use crate::reference::LogEngine;

    fn fails(f: impl FnOnce()) -> bool {
        catch_unwind(AssertUnwindSafe(f)).is_err()
    }

    /// The mutants are `Emulated<LogEngine>` with one rule broken: with none
    /// broken the same suite functions pass, so a failure below is the broken
    /// rule's.
    #[test]
    fn honest_mutant_passes_the_suite_functions_used_below() {
        type H = Mutant<Honest>;
        assert!(!fails(read_your_writes_get_and_scan::<H>));
        assert!(!fails(abort_leaves_no_trace_and_no_seq::<H>));
        assert!(!fails(drop_is_abort::<H>));
        assert!(!fails(panic_inside_txn_aborts_cleanly::<H>));
        assert!(!fails(txn_limits_enforced::<H>));
        assert!(!fails(empty_txn_returns_current_seq::<H>));
        assert!(!fails(txn_range_delete_then_put::<H>));
    }

    #[test]
    fn suite_rejects_reads_that_ignore_the_transaction_s_writes() {
        assert!(fails(read_your_writes_get_and_scan::<Mutant<StaleReads>>));
        assert!(fails(txn_range_delete_then_put::<Mutant<StaleReads>>));
    }

    #[test]
    fn suite_rejects_a_transaction_that_commits_when_it_ends_without_commit() {
        assert!(fails(abort_leaves_no_trace_and_no_seq::<Mutant<CommitOnDrop>>));
        assert!(fails(drop_is_abort::<Mutant<CommitOnDrop>>));
        assert!(fails(panic_inside_txn_aborts_cleanly::<Mutant<CommitOnDrop>>));
    }

    #[test]
    fn suite_rejects_limits_that_are_not_enforced() {
        assert!(fails(txn_limits_enforced::<Mutant<SwallowPutErrors>>));
    }

    #[test]
    fn suite_rejects_an_empty_transaction_that_consumes_a_sequence() {
        assert!(fails(empty_txn_returns_current_seq::<Mutant<EmptyConsumes>>));
    }

    #[test]
    fn suite_rejects_transactions_that_are_not_exclusive() {
        assert!(fails(begin_write_is_exclusive::<Naive<false, true>>));
        assert!(fails(concurrent_counter_increments_no_lost_updates::<Naive<false, true>>));
        assert!(fails(bank_transfer_reads_in_txn::<Naive<false, true>>));
    }

    /// Transactions exclusive among themselves are not enough: a blind commit
    /// must wait for them too.
    #[test]
    fn suite_rejects_a_blind_commit_that_does_not_wait_for_a_transaction() {
        assert!(!fails(begin_write_is_exclusive::<Naive<true, false>>));
        assert!(!fails(concurrent_counter_increments_no_lost_updates::<Naive<true, false>>));
        assert!(fails(blind_commit_waits_for_open_txn::<Naive<true, false>>));
    }

    #[test]
    fn suite_rejects_commits_that_are_not_durable() {
        assert!(fails(txn_commit_crash_at_every_point::<Emulated<NoFsync>>));
        assert!(!fails(txn_commit_crash_at_every_point::<Emulated<LogEngine>>));
    }

    #[test]
    fn txn_commits_is_a_plain_engine_over_the_transaction_path() {
        let (_disk, db) = new_db::<TxnCommits<Emulated<LogEngine>>>(1);
        let mut b = WriteBatch::new();
        b.put("a", "1").put("b", "2").delete_range("a", "b").put("a", "3");
        assert_eq!(db.commit(&b).unwrap(), 1);
        assert_eq!(db.commit(&WriteBatch::new()).unwrap(), 1, "an empty batch is not a commit");
        let mut model = Model::new();
        apply(&mut model, &b);
        assert_state(&db.snapshot().unwrap(), &model, "TxnCommits");
        let mut bad = WriteBatch::new();
        bad.put("c", "1").delete_range("z", "a");
        assert!(matches!(db.commit(&bad), Err(Error::InvalidArgument(_))));
        assert_eq!(db.snapshot().unwrap().seq(), 1, "a rejected batch writes nothing");
    }
}
