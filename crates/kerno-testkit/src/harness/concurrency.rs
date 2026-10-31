//! Multi-writer suite: the guarantees that only concurrent commits can break,
//! which is where group commit lives (see
//! [`group_commit_test_suite!`](crate::group_commit_test_suite)).
//!
//! Writers commit to disjoint key prefixes ([`writer_batch`]): commit `n`
//! (from 1) of writer `i` puts `w{i}/{n}` and sets `w{i}/last` to `n`, so a
//! recovered state tells how many commits `r_i` of each writer survived.
//! After any crash the recovered state must be the state after some commit
//! `s` ([`check_recovered`]):
//!
//! * per writer, a prefix with no holes, `w{i}/1..=r_i`, and
//!   `acked_i <= r_i <= attempted_i` wherever acknowledged commits must
//!   survive (`Sync` mode, a process crash, a restart);
//! * no holes across writers: `s - base = sum(r_i)`, and an acknowledged
//!   commit with sequence `q` is present exactly when `q <= s`.
//!
//! | Test | Requirement |
//! |---|---|
//! | `concurrent_commits_unique_contiguous_seqs` | 8 writers × 25 commits (`Sync` and `Buffered`): the acknowledged sequences are exactly `base+1..=base+200` and increase for each writer, the final state applies the batches in sequence order, and every snapshot a concurrent reader takes is exactly the state after its sequence |
//! | `multi_writer_power_loss_prefix_no_holes` | random writers and commits, power loss at a random point (`Sync`): the rules above with `acked_i <= r_i` |
//! | `no_reader_sees_undurable_data` | readers observe snapshots while writers commit (`Sync`), then power loss: everything a reader saw before it survives (invariant I-16) |
//! | `scenario_c_*` | scenario C ([`crash::run_scenario_c`]) under a crash or one I/O error at every mutating operation |
//!
//! When the writers also call `sync`, every successful `sync` must return
//! at least the sequence of every commit acknowledged to *any* writer before
//! the call (not only the caller's own), and never less than an earlier
//! `sync` of the same writer.
//!
//! Every multi-writer phase fails instead of hanging if it takes longer than
//! [`WRITERS_TIMEOUT`] (a follower waiting forever for a leader that failed
//! or panicked), and so does every call the suites make on a handle after a
//! failure ([`call_with_timeout`]); the readers' wait gives up as soon as
//! every reader has died.

use std::collections::BTreeMap;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{Durability, Error, Result, WriteBatch};

use super::crash::{self, keeps_working};
use super::faults::After;
use super::{Model, apply, assert_snapshot_matches, for_each_seed, open, seeds};
use crate::rng::Rng;
use crate::vfs::{CrashKind, FsyncFailure, SimVfs};

/// `sync_data` latency of the multi-writer tests
/// ([`SimVfs::set_sync_latency`]): long enough for other writers to queue up
/// behind a commit that is syncing, so that a group-commit engine forms
/// groups.
pub const SYNC_LATENCY: Duration = Duration::from_micros(250);

/// How long a writer waits between a commit and the `sync` that follows it
/// where [`sync_waits_first`] says so: several [`SYNC_LATENCY`]s, so that
/// the other writers commit and sync meanwhile.
pub const SYNC_WAIT: Duration = Duration::from_millis(1);

/// How long one multi-writer phase may take before the test fails as hung.
pub const WRITERS_TIMEOUT: Duration = Duration::from_secs(120);

/// A `sync`-like action the writers of a run call now and then (see
/// [`syncs_after`]); the `sync()` suite passes `SyncEngine::sync`.
pub type SyncFn<E> = Arc<dyn Fn(&E) -> Result<u64> + Send + Sync>;

/// Builds commit `n` (from 1) of writer `i`.
pub type BatchFn = Arc<dyn Fn(usize, u64) -> WriteBatch + Send + Sync>;

/// The key commit `n` of `writer` inserts.
pub fn writer_key(writer: usize, n: u64) -> Vec<u8> {
    format!("w{writer}/{n:06}").into_bytes()
}

/// The key that holds the number of `writer`'s last commit.
pub fn writer_last_key(writer: usize) -> Vec<u8> {
    format!("w{writer}/last").into_bytes()
}

/// The value commit `n` of `writer` stores: deterministic, mostly small, and
/// 3 KiB for every seventh commit (so that an engine with small test buffers
/// rotates them in the middle of a run).
pub fn writer_value(writer: usize, n: u64) -> Vec<u8> {
    let mut rng = Rng::new(((writer as u64) << 32) ^ n ^ 0x5C3A_C0DE);
    let len = if n % 7 == 3 { 3 << 10 } else { rng.range(16, 400) as usize };
    rng.bytes(len)
}

/// Commit `n` (from 1) of `writer`: inserts [`writer_key`] and sets
/// [`writer_last_key`] to `n` (little-endian `u64`).
pub fn writer_batch(writer: usize, n: u64) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(writer_key(writer, n), writer_value(writer, n));
    b.put(writer_last_key(writer), n.to_le_bytes().to_vec());
    b
}

/// Whether writer `writer` calls `sync` right after its commit `n`: about one
/// commit in four, at a different phase for each writer.
pub fn syncs_after(writer: usize, n: u64) -> bool {
    (n + writer as u64).is_multiple_of(4)
}

/// Whether writer `writer` waits [`SYNC_WAIT`] between its commit `n` and
/// the `sync` that follows it: every other `sync` of [`syncs_after`]. The
/// other writers meanwhile commit and sync, so the `sync` arrives after
/// commits acknowledged to other writers since the last `fsync` (where a
/// `sync` that only covers its caller's own commits, or returns a stale
/// durable sequence, is caught); the other half follows its commit at once.
pub fn sync_waits_first(writer: usize, n: u64) -> bool {
    (n + writer as u64) % 8 == 4
}

/// What one writer thread of a multi-writer run did.
#[derive(Debug, Default)]
pub struct WriterLog {
    /// The sequence of every acknowledged commit, in order: commit `n` of the
    /// writer got `acked[n - 1]`.
    pub acked: Vec<u64>,
    /// Commits attempted: the acknowledged ones plus the one that failed, if
    /// any.
    pub attempted: u64,
    /// Every sequence a successful `sync` returned, in order.
    pub synced: Vec<u64>,
    /// The error that stopped the writer (a failed commit or `sync`).
    pub error: Option<Error>,
}

impl WriterLog {
    /// The highest sequence a successful `sync` of this writer returned.
    pub fn synced_max(&self) -> u64 {
        self.synced.iter().copied().max().unwrap_or(0)
    }
}

/// Writer threads started by [`spawn_writers`].
pub struct Writers {
    rx: mpsc::Receiver<(usize, std::thread::Result<WriterLog>)>,
    handles: Vec<JoinHandle<()>>,
    deadline: Instant,
}

impl Writers {
    /// Waits for every writer (at most until [`WRITERS_TIMEOUT`] after they
    /// were spawned) and returns their logs, by writer. Re-raises the panic
    /// of a writer; panics with `ctx` if they hang.
    pub fn join(self, ctx: &str) -> Vec<WriterLog> {
        let mut logs: Vec<Option<WriterLog>> = self.handles.iter().map(|_| None).collect();
        for _ in 0..logs.len() {
            let left = self.deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok((i, Ok(log))) => logs[i] = Some(log),
                Ok((_, Err(panic))) => std::panic::resume_unwind(panic),
                Err(_) => panic!(
                    "{ctx}: the writers did not finish within {WRITERS_TIMEOUT:?}: a commit or sync hangs \
                     (a follower waiting for a failed or panicked leader?)"
                ),
            }
        }
        for h in self.handles {
            h.join().expect("writer threads report their panics through the channel");
        }
        logs.into_iter().map(|log| log.expect("one log per writer")).collect()
    }
}

/// Starts `writers` threads (named `writer-{i}`) that begin at the same time:
/// writer `i` makes commits `1..=commits` (`batch(i, n)`), calls `sync` after
/// the commits [`syncs_after`] picks (if given; first waiting where
/// [`sync_waits_first`] says so), and stops at its first error. Each thread
/// drops its handle on `db` before it reports.
///
/// The writers share the highest sequence acknowledged so far: a successful
/// `sync` that returns less than what was acknowledged to any writer before
/// the call panics the writer (and so [`Writers::join`]), as does one that
/// returns less than an earlier `sync` of the same writer.
pub fn spawn_writers<E: Engine>(
    db: &Arc<E>,
    writers: usize,
    commits: u64,
    batch: &BatchFn,
    sync: Option<&SyncFn<E>>,
) -> Writers {
    let (tx, rx) = mpsc::channel();
    let start = Arc::new(Barrier::new(writers));
    let max_acked = Arc::new(AtomicU64::new(0));
    let mut handles = Vec::with_capacity(writers);
    for i in 0..writers {
        let db = db.clone();
        let batch = batch.clone();
        let sync = sync.cloned();
        let start = start.clone();
        let max_acked = max_acked.clone();
        let tx = tx.clone();
        let handle = std::thread::Builder::new()
            .name(format!("writer-{i}"))
            .spawn(move || {
                let log = catch_unwind(AssertUnwindSafe(|| {
                    start.wait();
                    one_writer(&*db, i, commits, &*batch, sync.as_deref(), &max_acked)
                }));
                drop(db);
                let _ = tx.send((i, log));
            })
            .expect("spawn a writer thread");
        handles.push(handle);
    }
    Writers { rx, handles, deadline: Instant::now() + WRITERS_TIMEOUT }
}

/// [`spawn_writers`] and [`Writers::join`].
pub fn run_writers<E: Engine>(
    db: &Arc<E>,
    writers: usize,
    commits: u64,
    batch: &BatchFn,
    sync: Option<&SyncFn<E>>,
    ctx: &str,
) -> Vec<WriterLog> {
    spawn_writers(db, writers, commits, batch, sync).join(ctx)
}

/// Calls `f` on `db` in another thread and waits at most
/// [`WRITERS_TIMEOUT`] for it to return, for the calls the suites make on a
/// handle after a failure (a request queued on a poisoned handle that no
/// leader will ever serve): the test then fails with `ctx` and `what`
/// instead of hanging. Re-raises a panic of `f`. When this returns, the
/// thread has ended and dropped its clone of `db`.
pub fn call_with_timeout<E, T>(db: &Arc<E>, what: &str, ctx: &str, f: impl FnOnce(&E) -> T + Send + 'static) -> T
where
    E: Send + Sync + 'static,
    T: Send + 'static,
{
    call_within(db, WRITERS_TIMEOUT, what, ctx, f)
}

fn call_within<E, T>(
    db: &Arc<E>,
    timeout: Duration,
    what: &str,
    ctx: &str,
    f: impl FnOnce(&E) -> T + Send + 'static,
) -> T
where
    E: Send + Sync + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let db = db.clone();
    let handle = std::thread::Builder::new()
        .name("timed-call".into())
        .spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(|| f(&*db)));
            drop(db);
            let _ = tx.send(r);
        })
        .expect("spawn a thread for a timed call");
    match rx.recv_timeout(timeout) {
        Ok(r) => {
            handle.join().expect("the call reports its panic through the channel");
            r.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        }
        Err(_) => panic!(
            "{ctx}: {what} did not return within {timeout:?}: a call on a failed or poisoned handle hangs \
             (a request queued behind a failed or panicked leader?)"
        ),
    }
}

type SyncRef<'a, E> = &'a (dyn Fn(&E) -> Result<u64> + Send + Sync);

fn one_writer<E: Engine>(
    db: &E,
    writer: usize,
    commits: u64,
    batch: &(dyn Fn(usize, u64) -> WriteBatch + Send + Sync),
    sync: Option<SyncRef<'_, E>>,
    max_acked: &AtomicU64,
) -> WriterLog {
    let mut log = WriterLog::default();
    for n in 1..=commits {
        log.attempted += 1;
        match db.commit(&batch(writer, n)) {
            Ok(seq) => {
                if let Some(&prev) = log.acked.last() {
                    assert!(
                        seq > prev,
                        "writer {writer}: commit {n} got sequence {seq}, not above its previous {prev}"
                    );
                }
                log.acked.push(seq);
                max_acked.fetch_max(seq, Ordering::SeqCst);
            }
            Err(e) => {
                log.error = Some(e);
                break;
            }
        }
        if let Some(sync) = sync
            && syncs_after(writer, n)
        {
            if sync_waits_first(writer, n) {
                std::thread::sleep(SYNC_WAIT);
            }
            // Every sequence in `max_acked` was acknowledged to its writer
            // before this load, so before the call: a lower bound for `s`
            // (it includes this writer's own last commit).
            let before = max_acked.load(Ordering::SeqCst);
            match sync(db) {
                Ok(s) => {
                    assert!(
                        s >= before,
                        "writer {writer}: sync returned {s} although sequence {before} was acknowledged before the call"
                    );
                    let prev = log.synced_max();
                    assert!(s >= prev, "writer {writer}: sync returned {s} after an earlier sync returned {prev}");
                    log.synced.push(s);
                }
                Err(e) => {
                    log.error = Some(e);
                    break;
                }
            }
        }
    }
    log
}

/// [`writer_batch`] as a [`BatchFn`].
pub fn writer_batches() -> BatchFn {
    Arc::new(writer_batch)
}

/// What a state recovered after a multi-writer run must contain at least.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Floor {
    /// Every acknowledged commit must survive (`Sync` mode, a process crash,
    /// a restart on a working device, or a run that closed cleanly).
    pub acked: bool,
    /// The recovered sequence must be at least this (a successful `sync`).
    pub seq: u64,
}

/// A state recovered after a multi-writer run, as [`check_recovered`]
/// verified it.
#[derive(Clone, Debug)]
pub struct Recovered {
    /// The recovered sequence.
    pub seq: u64,
    /// Commits of each writer that survived (`r_i`).
    pub per_writer: Vec<u64>,
    /// The recovered contents.
    pub model: Model,
}

/// Checks the state recovered after a run of [`writer_batch`] writers that
/// started from `base` (the state after sequence `base_seq`): per-writer
/// prefixes within `logs`' bounds, no holes across writers, at least
/// `floor`, and contents equal to the model (see the
/// [module documentation](self)).
pub fn check_recovered<S: Snapshot>(
    snap: &S,
    base: &Model,
    base_seq: u64,
    logs: &[WriterLog],
    floor: Floor,
    rng: &mut Rng,
    ctx: &str,
) -> Recovered {
    let s = snap.seq();
    assert!(s >= base_seq, "{ctx}: recovered sequence {s} is older than the base {base_seq}");
    assert!(
        s >= floor.seq,
        "{ctx}: recovered sequence {s} is below {}, which a successful sync made durable",
        floor.seq
    );
    let mut model = base.clone();
    let mut per_writer = Vec::with_capacity(logs.len());
    for (i, log) in logs.iter().enumerate() {
        let r = match snap.get(&writer_last_key(i)) {
            Ok(None) => 0,
            Ok(Some(v)) => u64::from_le_bytes(
                v.as_slice().try_into().unwrap_or_else(|_| panic!("{ctx}: w{i}/last holds {} bytes", v.len())),
            ),
            Err(e) => panic!("{ctx}: get(w{i}/last) failed: {e}"),
        };
        assert!(r <= log.attempted, "{ctx}: writer {i}: {r} commits recovered, only {} attempted", log.attempted);
        let acked = log.acked.len() as u64;
        if floor.acked {
            assert!(r >= acked, "{ctx}: writer {i}: {r} commits recovered, but {acked} were acknowledged");
        }
        for (k, &q) in log.acked.iter().enumerate() {
            let n = k as u64 + 1;
            assert!(
                (q <= s) == (n <= r),
                "{ctx}: writer {i}: its commit {n} (sequence {q}) is {} although the recovered sequence is {s}",
                if n <= r { "present" } else { "missing" }
            );
        }
        for n in 1..=r {
            apply(&mut model, &writer_batch(i, n));
        }
        per_writer.push(r);
    }
    let survived: u64 = per_writer.iter().sum();
    assert_eq!(
        s - base_seq,
        survived,
        "{ctx}: recovered sequence {s} after base {base_seq}, but {survived} writer commits survived: a hole or an extra commit"
    );
    assert_snapshot_matches(snap, &model, rng, ctx);
    Recovered { seq: s, per_writer, model }
}

fn options(durability: Durability) -> Options {
    let mut opts = Options::small_for_tests();
    opts.durability = durability;
    opts
}

/// A simulated device for a multi-writer test: seeded, byte-granular tearing
/// on odd seeds, and [`SYNC_LATENCY`].
pub fn writers_disk(seed: u64) -> SimVfs {
    let disk = SimVfs::new(seed);
    if seed % 2 == 1 {
        disk.set_tear_unit(1);
    }
    disk.set_sync_latency(SYNC_LATENCY);
    disk
}

/// Sets a flag when dropped, also while unwinding: stops reader threads when
/// a test fails, so that they do not spin for the rest of the test binary.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// What a reader saw in one snapshot.
#[derive(Clone, Debug)]
struct Observation {
    seq: u64,
    /// `w{i}/last` of every writer (0 if absent).
    lasts: Vec<u64>,
    /// The shared key, if the test uses one.
    shared: Option<Vec<u8>>,
}

fn observe<S: Snapshot>(snap: &S, writers: usize, shared: Option<&[u8]>) -> Result<Observation> {
    let mut lasts = Vec::with_capacity(writers);
    for i in 0..writers {
        let n = snap.get(&writer_last_key(i))?.map_or(0, |v| u64::from_le_bytes(v.as_slice().try_into().unwrap()));
        lasts.push(n);
    }
    let shared = match shared {
        Some(k) => snap.get(k)?,
        None => None,
    };
    Ok(Observation { seq: snap.seq(), lasts, shared })
}

/// Reader threads that take snapshots in a loop and keep one observation per
/// new sequence while `keep()` says so (checked after the observation), and
/// call `kept(seq)` right after keeping one. Every reader makes its first
/// observation before [`Readers::spawn`] returns, so that the readers are
/// running before the writers start and never finish with nothing seen
/// (however late the scheduler runs them).
struct Readers {
    stop: StopOnDrop,
    handles: Vec<JoinHandle<Vec<Observation>>>,
}

impl Readers {
    fn spawn<E: Engine>(
        db: &Arc<E>,
        count: usize,
        writers: usize,
        shared: Option<&'static [u8]>,
        keep: impl Fn() -> bool + Send + Sync + 'static,
        kept: impl Fn(u64) + Send + Sync + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let keep = Arc::new(keep);
        let kept = Arc::new(kept);
        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let mut handles = Vec::with_capacity(count);
        for r in 0..count {
            let db = db.clone();
            let stop = stop.clone();
            let keep = keep.clone();
            let kept = kept.clone();
            let mut ready = Some(ready_tx.clone());
            let handle = std::thread::Builder::new()
                .name(format!("reader-{r}"))
                .spawn(move || {
                    let mut seen: Vec<Observation> = Vec::new();
                    loop {
                        let obs = db.snapshot().and_then(|snap| observe(&snap, writers, shared));
                        match obs {
                            Ok(obs) => {
                                let last = seen.last().map_or(0, |o| o.seq);
                                assert!(
                                    obs.seq >= last,
                                    "reader {r}: snapshot sequence went back from {last} to {}",
                                    obs.seq
                                );
                                if obs.seq != last && keep() {
                                    let seq = obs.seq;
                                    seen.push(obs);
                                    kept(seq);
                                }
                            }
                            Err(e) => assert!(!keep(), "reader {r}: a read failed without a crash: {e}"),
                        }
                        if let Some(ready) = ready.take() {
                            let _ = ready.send(());
                        }
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::yield_now();
                    }
                    seen
                })
                .expect("spawn a reader thread");
            handles.push(handle);
        }
        drop(ready_tx);
        let readers = Readers { stop: StopOnDrop(stop), handles };
        for _ in 0..count {
            // A reader that panicked dropped its sender without sending:
            // stop waiting, `finish` re-raises its panic.
            if ready_rx.recv_timeout(WRITERS_TIMEOUT).is_err() {
                break;
            }
        }
        readers
    }

    /// Whether every reader thread has ended. A reader runs until
    /// [`Readers::finish`] stops it, so before that this means that every
    /// reader panicked (and `finish` re-raises the panic).
    fn all_finished(&self) -> bool {
        self.handles.iter().all(JoinHandle::is_finished)
    }

    /// Stops the readers and returns every observation they kept.
    fn finish(self) -> Vec<Observation> {
        self.stop.0.store(true, Ordering::SeqCst);
        let mut all = Vec::new();
        for h in self.handles {
            match h.join() {
                Ok(seen) => all.extend(seen),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        all
    }
}

/// 8 writers × 25 commits at once, in `Sync` and `Buffered` mode, with two
/// concurrent readers. Every batch also overwrites one shared key with the
/// writer and commit number, so the final state and every snapshot reveal
/// the order in which batches were applied: it must be the sequence order.
pub fn concurrent_commits_unique_contiguous_seqs<E: Engine>() {
    const WRITERS: usize = 8;
    const COMMITS: u64 = 25;
    const BASE: u64 = 3;
    const SHARED: &[u8] = b"shared/last";
    let tag = |i: usize, n: u64| format!("w{i}/{n}").into_bytes();
    let base_tag = |k: u64| format!("base/{k}").into_bytes();
    for durability in [Durability::Sync, Durability::Buffered] {
        let ctx = format!("concurrent commits ({durability:?})");
        let disk = writers_disk(11);
        let opts = options(durability);
        let db: E = open(&disk.as_vfs(), &opts).unwrap();
        let mut model = Model::new();
        for k in 1..=BASE {
            let mut b = WriteBatch::new();
            b.put(SHARED, base_tag(k)).put(base_tag(k), "x");
            assert_eq!(db.commit(&b).unwrap(), k, "{ctx}: base commit");
            apply(&mut model, &b);
        }
        let db = Arc::new(db);
        let readers = Readers::spawn(&db, 2, WRITERS, Some(SHARED), || true, |_| {});
        let batch: BatchFn = Arc::new(move |i, n| {
            let mut b = writer_batch(i, n);
            b.put(SHARED, tag(i, n));
            b
        });
        let logs = run_writers(&db, WRITERS, COMMITS, &batch, None, &ctx);
        let observations = readers.finish();

        let mut by_seq: BTreeMap<u64, (usize, u64)> = BTreeMap::new();
        for (i, log) in logs.iter().enumerate() {
            if let Some(e) = &log.error {
                panic!("{ctx}: writer {i} failed: {e}");
            }
            assert_eq!(log.acked.len() as u64, COMMITS, "{ctx}: writer {i}");
            for (k, &q) in log.acked.iter().enumerate() {
                if let Some((j, m)) = by_seq.insert(q, (i, k as u64 + 1)) {
                    panic!("{ctx}: sequence {q} was returned to writer {i} and to writer {j} (commit {m})");
                }
            }
        }
        let total = WRITERS as u64 * COMMITS;
        let first = by_seq.keys().next().copied();
        let last = by_seq.keys().next_back().copied();
        assert!(
            by_seq.len() as u64 == total && first == Some(BASE + 1) && last == Some(BASE + total),
            "{ctx}: the acknowledged sequences are not exactly {}..={} (first {first:?}, last {last:?})",
            BASE + 1,
            BASE + total
        );
        for &(i, n) in by_seq.values() {
            apply(&mut model, &batch(i, n));
        }
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), BASE + total, "{ctx}: final sequence");
        let mut rng = Rng::new(11);
        assert_snapshot_matches(&snap, &model, &mut rng, &format!("{ctx}: final state in sequence order"));
        drop(snap);

        assert!(!observations.is_empty(), "{ctx}: the readers observed nothing");
        for obs in &observations {
            let want_shared = match by_seq.get(&obs.seq) {
                Some(&(i, n)) => tag(i, n),
                None => {
                    assert!((1..=BASE).contains(&obs.seq), "{ctx}: a reader saw unknown sequence {}", obs.seq);
                    base_tag(obs.seq)
                }
            };
            assert!(
                obs.shared.as_deref() == Some(want_shared.as_slice()),
                "{ctx}: the snapshot at sequence {} shows the shared key of another commit",
                obs.seq
            );
            for (j, log) in logs.iter().enumerate() {
                let want = log.acked.partition_point(|&q| q <= obs.seq) as u64;
                assert_eq!(
                    obs.lasts[j], want,
                    "{ctx}: the snapshot at sequence {} shows {} commits of writer {j}, not {want}",
                    obs.seq, obs.lasts[j]
                );
            }
        }

        let db = Arc::try_unwrap(db).unwrap_or_else(|_| panic!("{ctx}: another handle is still alive"));
        db.close().unwrap();
        let image = disk.crash_now(CrashKind::PowerLoss);
        let db: E = open(&image.as_vfs(), &opts).unwrap();
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), BASE + total, "{ctx}: sequence after a clean close and a power loss");
        assert_snapshot_matches(&snap, &model, &mut rng, &format!("{ctx}: after a clean close and a power loss"));
    }
}

/// Single-threaded base commits, then random writers × commits at once
/// (`Sync`), with a power loss at a random point: per writer
/// `acked_i <= r_i <= attempted_i`, `s - base = sum(r_i)`, no holes, and
/// the recovered database keeps working.
pub fn multi_writer_power_loss_prefix_no_holes<E: Engine>() {
    for_each_seed("multi_writer_power_loss_prefix_no_holes", seeds(24), |seed| {
        let ctx = format!("seed {seed}");
        let mut rng = Rng::new(seed);
        let disk = writers_disk(seed);
        let opts = options(Durability::Sync);
        let db: E = open(&disk.as_vfs(), &opts).unwrap();
        let mut base = Model::new();
        let base_seq = rng.range(0, 4);
        for k in 1..=base_seq {
            let mut b = WriteBatch::new();
            let len = rng.range(1, 300) as usize;
            b.put(format!("base/{k}"), rng.bytes(len));
            assert_eq!(db.commit(&b).unwrap(), k, "{ctx}: base commit");
            apply(&mut base, &b);
        }
        let writers = rng.range(2, 8) as usize;
        let commits = rng.range(4, 16);
        // About two mutating operations per commit (write and sync) for an
        // engine without group commit; a crash point past the end of the run
        // means a power loss once everything was acknowledged.
        let budget = writers as u64 * commits * 2 + 8;
        disk.crash_after(rng.range(1, budget), CrashKind::PowerLoss);
        let db = Arc::new(db);
        let logs = run_writers(&db, writers, commits, &writer_batches(), None, &ctx);
        for (i, log) in logs.iter().enumerate() {
            if let Some(e) = &log.error {
                assert!(disk.has_crashed(), "{ctx}: writer {i} failed without a crash: {e}");
            }
        }
        let image = match disk.take_crash_image() {
            Some(image) => image,
            None => disk.crash_now(CrashKind::PowerLoss),
        };
        drop(db);
        let db: E = open(&image.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
        let snap = db.snapshot().unwrap();
        let floor = Floor { acked: true, seq: 0 };
        let rec = check_recovered(&snap, &base, base_seq, &logs, floor, &mut rng, &ctx);
        drop(snap);
        keeps_working(db, &image, &opts, rec.model, rec.seq, &mut rng, &ctx);
    });
}

/// Writers commit (`Sync`) while readers take snapshots; the first reader to
/// see a writer's commit arms a power loss at the very next mutating
/// operation: if that commit was published before its `fsync` completed,
/// this is when it can still be lost. Every observation made before the
/// power loss must survive it: the recovered sequence is at least the
/// observed one, and so is every writer's recovered count (invariant I-16:
/// in `Sync` mode a commit is published only after its `fsync`).
pub fn no_reader_sees_undurable_data<E: Engine>() {
    no_reader_sees_undurable_data_on::<E>(seeds(16));
}

fn no_reader_sees_undurable_data_on<E: Engine>(seeds: Range<u64>) {
    const WRITERS: usize = 4;
    const COMMITS: u64 = 12;
    for_each_seed("no_reader_sees_undurable_data", seeds, |seed| {
        let ctx = format!("seed {seed}");
        let mut rng = Rng::new(seed);
        let disk = writers_disk(seed);
        let opts = options(Durability::Sync);
        let db: E = open(&disk.as_vfs(), &opts).unwrap();
        let mut base = Model::new();
        let mut b = WriteBatch::new();
        b.put("base", "x");
        assert_eq!(db.commit(&b).unwrap(), 1, "{ctx}: base commit");
        apply(&mut base, &b);
        let db = Arc::new(db);
        // An observation counts only if the device was still alive after
        // it was made: then it was made before the power loss. The reader
        // arms the power loss itself, without waiting for another thread to
        // be scheduled (meanwhile the commit's `fsync` could finish).
        let alive = disk.clone();
        let target = disk.clone();
        let armed = Arc::new(AtomicBool::new(false));
        let arm = armed.clone();
        let readers = Readers::spawn(
            &db,
            2,
            WRITERS,
            None,
            move || !alive.has_crashed(),
            move |seq| {
                if seq > 1 && !arm.swap(true, Ordering::SeqCst) {
                    target.crash_after(1, CrashKind::PowerLoss);
                }
            },
        );
        let logs = run_writers(&db, WRITERS, COMMITS, &writer_batches(), None, &ctx);
        // If no reader saw a writer's commit while they were committing,
        // they see the final state now, unless every reader died: then
        // `finish` re-raises the reader's panic at once.
        let deadline = Instant::now() + WRITERS_TIMEOUT;
        while !armed.load(Ordering::SeqCst) && !readers.all_finished() {
            assert!(Instant::now() < deadline, "{ctx}: no reader saw a writer's commit");
            std::thread::yield_now();
        }
        let observations = readers.finish();
        for (i, log) in logs.iter().enumerate() {
            if let Some(e) = &log.error {
                assert!(disk.has_crashed(), "{ctx}: writer {i} failed without a crash: {e}");
            }
        }
        let image = match disk.take_crash_image() {
            Some(image) => image,
            None => disk.crash_now(CrashKind::PowerLoss),
        };
        drop(db);
        let db: E = open(&image.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
        let snap = db.snapshot().unwrap();
        let floor = Floor { acked: true, seq: 0 };
        let rec = check_recovered(&snap, &base, 1, &logs, floor, &mut rng, &ctx);
        drop(snap);
        assert!(observations.iter().any(|o| o.seq > 1), "{ctx}: no observation of a writer's commit was kept");
        for obs in &observations {
            assert!(
                obs.seq <= rec.seq,
                "{ctx}: a reader saw sequence {} before the power loss, but only {} was recovered",
                obs.seq,
                rec.seq
            );
            for (i, (&seen, &kept)) in obs.lasts.iter().zip(&rec.per_writer).enumerate() {
                assert!(
                    seen <= kept,
                    "{ctx}: a reader saw {seen} commits of writer {i} before the power loss, only {kept} survived"
                );
            }
        }
        keeps_working(db, &image, &opts, rec.model, rec.seq, &mut rng, &ctx);
    });
}

/// Scenario C, power loss at every mutating operation (`Sync`).
pub fn scenario_c_power_loss_at_every_io_point<E: Engine>() {
    crash::every_point_c::<E>(CrashKind::PowerLoss, Durability::Sync, None);
}

/// Scenario C, process kill at every mutating operation (`Sync`).
pub fn scenario_c_process_kill_at_every_io_point<E: Engine>() {
    crash::every_point_c::<E>(CrashKind::ProcessKill, Durability::Sync, None);
}

/// Scenario C, process kill at every mutating operation (`Buffered`).
pub fn scenario_c_process_kill_at_every_io_point_buffered<E: Engine>() {
    crash::every_point_c::<E>(CrashKind::ProcessKill, Durability::Buffered, None);
}

/// Scenario C, a process kill that keeps unsynced writes pending at every
/// mutating operation (`Sync`), then a power loss right after recovery.
pub fn scenario_c_power_loss_after_kill_keep_pending_sync<E: Engine>() {
    crash::every_point_c::<E>(CrashKind::ProcessKillKeepPending, Durability::Sync, None);
}

/// Scenario C, a process kill that keeps unsynced writes pending at every
/// mutating operation (`Buffered`), then a power loss right after recovery.
pub fn scenario_c_power_loss_after_kill_keep_pending_buffered<E: Engine>() {
    crash::every_point_c::<E>(CrashKind::ProcessKillKeepPending, Durability::Buffered, None);
}

/// Scenario C, one I/O error at every mutating operation (`Sync`, failed
/// syncs keep their writes pending), then a restart or a power loss.
pub fn scenario_c_io_failure_at_every_point<E: Engine>() {
    crash::failure_sweep_c::<E>(Durability::Sync, FsyncFailure::KeepPending, &[After::Restart, After::PowerLoss], None);
}

/// Scenario C, one I/O error at every mutating operation (`Buffered`, failed
/// syncs keep their writes pending), then a restart.
pub fn scenario_c_io_failure_at_every_point_buffered_restart<E: Engine>() {
    crash::failure_sweep_c::<E>(Durability::Buffered, FsyncFailure::KeepPending, &[After::Restart], None);
}

/// Scenario C, one I/O error at every mutating operation with Linux `fsync`
/// semantics (`Sync`), then a restart or a power loss.
pub fn scenario_c_linux_fsync_drop_faults_sync<E: Engine>() {
    crash::failure_sweep_c::<E>(
        Durability::Sync,
        FsyncFailure::LinuxDropPages,
        &[After::Restart, After::PowerLoss],
        None,
    );
}

/// Scenario C, one I/O error at every mutating operation with Linux `fsync`
/// semantics (`Buffered`), then a restart.
pub fn scenario_c_linux_fsync_drop_faults_buffered_restart<E: Engine>() {
    crash::failure_sweep_c::<E>(Durability::Buffered, FsyncFailure::LinuxDropPages, &[After::Restart], None);
}

/// Generates the multi-writer suite (module `shared_concurrency`) for an
/// engine type:
///
/// ```ignore
/// kerno_testkit::group_commit_test_suite!(MyEngine);
/// ```
///
/// See the [module documentation](crate::harness::concurrency) for what each
/// test checks. Only [`Engine`](crate::Engine) is required; an engine
/// without group commit (one `fsync` per commit) must pass it too.
#[macro_export]
macro_rules! group_commit_test_suite {
    ($engine:ty) => {
        mod shared_concurrency {
            use super::*;
            use $crate::harness::concurrency as g;
            #[test]
            fn concurrent_commits_unique_contiguous_seqs() {
                g::concurrent_commits_unique_contiguous_seqs::<$engine>();
            }
            #[test]
            fn multi_writer_power_loss_prefix_no_holes() {
                g::multi_writer_power_loss_prefix_no_holes::<$engine>();
            }
            #[test]
            fn no_reader_sees_undurable_data() {
                g::no_reader_sees_undurable_data::<$engine>();
            }
            #[test]
            fn scenario_c_power_loss_at_every_io_point() {
                g::scenario_c_power_loss_at_every_io_point::<$engine>();
            }
            #[test]
            fn scenario_c_process_kill_at_every_io_point() {
                g::scenario_c_process_kill_at_every_io_point::<$engine>();
            }
            #[test]
            fn scenario_c_process_kill_at_every_io_point_buffered() {
                g::scenario_c_process_kill_at_every_io_point_buffered::<$engine>();
            }
            #[test]
            fn scenario_c_power_loss_after_kill_keep_pending_sync() {
                g::scenario_c_power_loss_after_kill_keep_pending_sync::<$engine>();
            }
            #[test]
            fn scenario_c_power_loss_after_kill_keep_pending_buffered() {
                g::scenario_c_power_loss_after_kill_keep_pending_buffered::<$engine>();
            }
            #[test]
            fn scenario_c_io_failure_at_every_point() {
                g::scenario_c_io_failure_at_every_point::<$engine>();
            }
            #[test]
            fn scenario_c_io_failure_at_every_point_buffered_restart() {
                g::scenario_c_io_failure_at_every_point_buffered_restart::<$engine>();
            }
            #[test]
            fn scenario_c_linux_fsync_drop_faults_sync() {
                g::scenario_c_linux_fsync_drop_faults_sync::<$engine>();
            }
            #[test]
            fn scenario_c_linux_fsync_drop_faults_buffered_restart() {
                g::scenario_c_linux_fsync_drop_faults_buffered_restart::<$engine>();
            }
        }
    };
}

/// Deliberately broken engines that the suites must reject: evidence that
/// the checks have teeth, not only that the reference passes them.
#[cfg(test)]
pub(crate) mod mutants {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread::ThreadId;

    use kerno_core::contract::{Engine, Options, Snapshot, SyncEngine};
    use kerno_core::vfs::Vfs;
    use kerno_core::{Durability, Error, Result, WriteBatch};
    use parking_lot::Mutex;

    use crate::reference::{LogEngine, LogSnapshot};

    /// Acknowledges and publishes commits without any `fsync` (`LogEngine`
    /// in `Buffered` mode whatever the options say), and `sync` does no I/O.
    pub struct NoFsync(LogEngine);

    impl Engine for NoFsync {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-no-fsync"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            let opts = Options { durability: Durability::Buffered, ..opts.clone() };
            LogEngine::open(vfs, dir, &opts).map(NoFsync)
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

    impl SyncEngine for NoFsync {
        fn sync(&self) -> Result<u64> {
            Ok(self.0.snapshot()?.seq())
        }
    }

    /// Acknowledges only durable commits, but publishes each one to readers
    /// before its `fsync` (invariant I-16 broken): `LogEngine` in `Buffered`
    /// mode plus a `sync` before `commit` returns.
    pub struct PublishBeforeFsync(LogEngine);

    impl Engine for PublishBeforeFsync {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-publish-before-fsync"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            let opts = Options { durability: Durability::Buffered, ..opts.clone() };
            LogEngine::open(vfs, dir, &opts).map(PublishBeforeFsync)
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            let seq = self.0.commit(batch)?;
            self.0.sync()?;
            Ok(seq)
        }

        fn snapshot(&self) -> Result<LogSnapshot> {
            self.0.snapshot()
        }

        fn close(self) -> Result<()> {
            self.0.close()
        }
    }

    /// `sync` reports success even when its `fsync` failed.
    pub struct SwallowsSyncErrors(LogEngine);

    impl Engine for SwallowsSyncErrors {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-swallows-sync-errors"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            LogEngine::open(vfs, dir, opts).map(SwallowsSyncErrors)
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

    impl SyncEngine for SwallowsSyncErrors {
        fn sync(&self) -> Result<u64> {
            let _ = self.0.sync();
            Ok(self.0.snapshot()?.seq())
        }
    }

    /// `LogEngine` that remembers the last sequence acknowledged to each
    /// thread, for the mutants whose `sync` only looks at its caller.
    pub struct PerThread {
        inner: LogEngine,
        /// The sequence recovered by `open` (durable).
        recovered: u64,
        last: Mutex<HashMap<ThreadId, u64>>,
    }

    impl PerThread {
        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            let inner = LogEngine::open(vfs, dir, opts)?;
            let recovered = inner.snapshot()?.seq();
            Ok(PerThread { inner, recovered, last: Mutex::new(HashMap::new()) })
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            let seq = self.inner.commit(batch)?;
            self.last.lock().insert(std::thread::current().id(), seq);
            Ok(seq)
        }

        /// The last sequence acknowledged to the calling thread, or the
        /// recovered one.
        fn own(&self) -> u64 {
            let own = self.last.lock().get(&std::thread::current().id()).copied().unwrap_or(0);
            own.max(self.recovered)
        }
    }

    /// `sync` makes everything durable (a real `fsync`) but returns the
    /// calling thread's own last acknowledged sequence instead of the
    /// engine's: below what other writers had acknowledged before the call.
    pub struct OwnSeqSync(PerThread);

    impl Engine for OwnSeqSync {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-own-seq-sync"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            PerThread::open(vfs, dir, opts).map(OwnSeqSync)
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            self.0.commit(batch)
        }

        fn snapshot(&self) -> Result<LogSnapshot> {
            self.0.inner.snapshot()
        }

        fn close(self) -> Result<()> {
            self.0.inner.close()
        }
    }

    impl SyncEngine for OwnSeqSync {
        fn sync(&self) -> Result<u64> {
            self.0.inner.sync()?;
            Ok(self.0.own())
        }
    }

    /// `sync` skips the `fsync` and returns the durable sequence of the last
    /// `sync` that ran one whenever the calling thread's own last commit is
    /// already covered: commits acknowledged to other threads before the
    /// call are neither made durable nor counted (a group-commit `sync`
    /// that returns a stale durable sequence).
    pub struct SyncOwnOnly {
        engine: PerThread,
        durable: Mutex<u64>,
    }

    impl Engine for SyncOwnOnly {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-sync-own-only"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            let engine = PerThread::open(vfs, dir, opts)?;
            let durable = Mutex::new(engine.recovered);
            Ok(SyncOwnOnly { engine, durable })
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            self.engine.commit(batch)
        }

        fn snapshot(&self) -> Result<LogSnapshot> {
            self.engine.inner.snapshot()
        }

        fn close(self) -> Result<()> {
            self.engine.inner.close()
        }
    }

    impl SyncEngine for SyncOwnOnly {
        fn sync(&self) -> Result<u64> {
            let own = self.engine.own();
            let mut durable = self.durable.lock();
            if own > *durable {
                *durable = self.engine.inner.sync()?;
            }
            Ok(*durable)
        }
    }

    /// Every `snapshot` after the first two of a handle fails with
    /// `Corruption`.
    pub struct ReadsFail {
        inner: LogEngine,
        snapshots: AtomicU64,
    }

    impl Engine for ReadsFail {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-reads-fail"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            let inner = LogEngine::open(vfs, dir, opts)?;
            Ok(ReadsFail { inner, snapshots: AtomicU64::new(0) })
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            self.inner.commit(batch)
        }

        fn snapshot(&self) -> Result<LogSnapshot> {
            if self.snapshots.fetch_add(1, Ordering::SeqCst) >= 2 {
                return Err(Error::Corruption("mutant: every read after the second fails".into()));
            }
            self.inner.snapshot()
        }

        fn close(self) -> Result<()> {
            self.inner.close()
        }
    }

    /// Hands out sequence 12 twice (commit 13 is reported as 12).
    pub struct DuplicateSeq(LogEngine);

    impl Engine for DuplicateSeq {
        type Snapshot = LogSnapshot;

        fn name() -> &'static str {
            "mutant-duplicate-seq"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            LogEngine::open(vfs, dir, opts).map(DuplicateSeq)
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            let seq = self.0.commit(batch)?;
            Ok(if seq == 13 { 12 } else { seq })
        }

        fn snapshot(&self) -> Result<LogSnapshot> {
            self.0.snapshot()
        }

        fn close(self) -> Result<()> {
            self.0.close()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use kerno_core::contract::SyncEngine;

    use super::mutants::{DuplicateSeq, NoFsync, PublishBeforeFsync, ReadsFail};
    use super::*;
    use crate::reference::LogEngine;

    fn fails(f: impl FnOnce()) -> bool {
        catch_unwind(AssertUnwindSafe(f)).is_err()
    }

    /// The message of the panic `f` raises (`None` if it returns).
    fn panic_message(f: impl FnOnce()) -> Option<String> {
        let panic = catch_unwind(AssertUnwindSafe(f)).err()?;
        Some(match panic.downcast::<String>() {
            Ok(msg) => *msg,
            Err(panic) => panic.downcast_ref::<&str>().map_or_else(|| "<not a string>".into(), |m| m.to_string()),
        })
    }

    /// The checker accepts a prefix and rejects each way a recovered state
    /// can be wrong.
    #[test]
    fn check_recovered_accepts_a_prefix_and_rejects_holes() {
        // Writer 0's commits 1 and 2 got sequences 1 and 3; writer 1's commit
        // 1 got sequence 2.
        let disk = SimVfs::new(1);
        let db = LogEngine::open(disk.as_vfs(), Path::new("/db"), &Options::small_for_tests()).unwrap();
        for (w, n) in [(0, 1), (1, 1), (0, 2)] {
            db.commit(&writer_batch(w, n)).unwrap();
        }
        let snap = db.snapshot().unwrap();
        let log = |acked: &[u64], attempted| WriterLog { acked: acked.to_vec(), attempted, ..WriterLog::default() };
        let check = |logs: &[WriterLog], base_seq: u64, floor: Floor| {
            catch_unwind(AssertUnwindSafe(|| {
                check_recovered(&snap, &Model::new(), base_seq, logs, floor, &mut Rng::new(1), "self-test").per_writer
            }))
            .ok()
        };
        let acked = Floor { acked: true, seq: 0 };
        let unacked = Floor { acked: false, seq: 0 };
        assert_eq!(check(&[log(&[1, 3], 2), log(&[2], 1)], 0, acked), Some(vec![2, 1]));
        // Writer 1's commit 2 was in flight: it may be missing.
        assert_eq!(check(&[log(&[1, 3], 2), log(&[2], 2)], 0, acked), Some(vec![2, 1]));
        // An acknowledged commit (writer 1's commit 2, sequence 4) is missing...
        assert_eq!(check(&[log(&[1, 3], 2), log(&[2, 4], 2)], 0, acked), None);
        // ... which is allowed only where acknowledged commits may be lost.
        assert_eq!(check(&[log(&[1, 3], 2), log(&[2, 4], 2)], 0, unacked), Some(vec![2, 1]));
        // A hole: writer 0's commit 2 (sequence 4) survived, but the
        // recovered sequence is 3, so commit 3 (writer 1's) is missing.
        assert_eq!(check(&[log(&[1, 4], 2), log(&[2], 2)], 0, unacked), None);
        // More commits recovered than attempted.
        assert_eq!(check(&[log(&[1], 1), log(&[2], 1)], 0, unacked), None);
        // The recovered sequence does not match the commits that survived.
        assert_eq!(check(&[log(&[2, 4], 2), log(&[3], 1)], 1, unacked), None);
        // Below what a successful sync returned.
        assert_eq!(check(&[log(&[1, 3], 2), log(&[2], 1)], 0, Floor { acked: true, seq: 4 }), None);
    }

    #[test]
    fn suite_rejects_acknowledged_commits_that_were_never_synced() {
        assert!(fails(multi_writer_power_loss_prefix_no_holes::<NoFsync>));
    }

    #[test]
    fn suite_rejects_publication_before_fsync() {
        // Every acknowledged commit of this mutant is durable, so only the
        // readers' observations can catch it.
        assert!(!fails(multi_writer_power_loss_prefix_no_holes::<PublishBeforeFsync>));
        // Whether one seed catches it depends on thread timing (the reader
        // must see the commit while its `fsync` is in progress, and the
        // power loss must then drop the write); one of 64 seeds is all but
        // certain to.
        assert!(fails(|| no_reader_sees_undurable_data_on::<PublishBeforeFsync>(0..64)));
        assert!(fails(no_reader_sees_undurable_data::<NoFsync>));
    }

    #[test]
    fn suite_rejects_a_duplicate_sequence() {
        assert!(fails(concurrent_commits_unique_contiguous_seqs::<DuplicateSeq>));
    }

    #[test]
    fn scenario_c_rejects_lost_acknowledged_commits() {
        assert!(fails(scenario_c_power_loss_at_every_io_point::<NoFsync>));
    }

    #[test]
    fn writers_report_sequences_and_syncs() {
        let disk = writers_disk(2);
        let db = Arc::new(LogEngine::open(disk.as_vfs(), Path::new("/db"), &Options::small_for_tests()).unwrap());
        let sync: SyncFn<LogEngine> = Arc::new(|db: &LogEngine| db.sync());
        let logs = run_writers(&db, 3, 8, &writer_batches(), Some(&sync), "self-test");
        let mut all: Vec<u64> = logs.iter().flat_map(|l| l.acked.iter().copied()).collect();
        all.sort_unstable();
        assert_eq!(all, (1..=24).collect::<Vec<u64>>());
        for (i, log) in logs.iter().enumerate() {
            assert!(log.error.is_none(), "writer {i}");
            assert_eq!(log.attempted, 8, "writer {i}");
            let expected = (1..=8).filter(|&n| syncs_after(i, n)).count();
            assert_eq!(log.synced.len(), expected, "writer {i}");
            assert!(log.synced_max() <= 24, "writer {i}");
        }
    }

    /// Readers that die (here: every read after the second fails) fail the
    /// test at once with their own panic, instead of after
    /// [`WRITERS_TIMEOUT`] with a message that blames the timing.
    #[test]
    fn dead_readers_fail_the_test_at_once() {
        let start = Instant::now();
        let msg = panic_message(|| no_reader_sees_undurable_data_on::<ReadsFail>(0..1))
            .expect("readers whose reads fail must fail the test");
        let took = start.elapsed();
        assert!(msg.contains("a read failed without a crash"), "the readers' panic must be reported, not {msg:?}");
        assert!(took < WRITERS_TIMEOUT / 4, "the test waited {took:?} for readers that were dead");
    }

    #[test]
    fn a_hung_call_fails_the_test_instead_of_hanging() {
        let db = Arc::new(5u64);
        assert_eq!(call_with_timeout(&db, "a call", "self-test", |n| n + 1), 6);
        assert_eq!(Arc::strong_count(&db), 1, "the thread has dropped its clone of the handle");
        let msg = panic_message(|| {
            call_with_timeout(&db, "a panicking call", "self-test", |_| -> u64 { panic!("boom") });
        });
        assert_eq!(msg.as_deref(), Some("boom"), "the call's own panic is re-raised");
        // A call that never returns (until `release` is dropped, so that the
        // thread does not outlive the test).
        let (release, wait) = mpsc::channel::<()>();
        let start = Instant::now();
        let msg = panic_message(|| {
            call_within(&db, Duration::from_millis(50), "a hung call", "self-test", move |_| {
                let _ = wait.recv();
            });
        })
        .expect("a hung call must fail the test");
        assert!(msg.contains("self-test: a hung call did not return within"), "{msg}");
        assert!(start.elapsed() < WRITERS_TIMEOUT / 4);
        drop(release);
    }

    #[test]
    fn a_hung_writer_fails_the_test_instead_of_hanging() {
        // Writers that never finish: `join` must give up at the deadline.
        let (_keep, rx) = mpsc::channel::<(usize, std::thread::Result<WriterLog>)>();
        let writers = Writers { rx, handles: Vec::new(), deadline: Instant::now() };
        // No handles means nothing to wait for; with one missing report it
        // must time out.
        assert!(writers.join("self-test").is_empty());
        let (_keep, rx) = mpsc::channel::<(usize, std::thread::Result<WriterLog>)>();
        let idle = std::thread::spawn(|| {});
        let writers = Writers { rx, handles: vec![idle], deadline: Instant::now() + Duration::from_millis(50) };
        assert!(fails(|| {
            writers.join("self-test");
        }));
    }
}
