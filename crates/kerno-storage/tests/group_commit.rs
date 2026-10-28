//! Group commit in `KernoDb` (phase1-design.md §5.6): what the shared
//! multi-writer suite cannot see, because it needs the engine's counters, the
//! file system under the engine, or a leader held at a precise point.
//!
//! Tests that need a group to form do not rely on timing: a `HookVfs` holds
//! the leader inside its WAL write or `fsync` (a [`Gate`]), the test queues
//! the other writers behind it and waits until the engine counts them
//! (`queued_bytes_max`), and only then releases the leader.

use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, Op, Result, WriteBatch};
use kerno_storage::format::wal::{CommitOp, canonicalize};
use kerno_storage::{Config, GroupCommitConfig, KernoDb};
use kerno_testkit::harness::concurrency::{BatchFn, run_writers, writer_batches, writers_disk};
use kerno_testkit::vfs::{CountingVfs, CrashKind, HookAction, HookOp, HookOpKind, HookPoint, HookVfs, SimVfs};

/// How long a test waits for any thread before it fails as hung.
const LIMIT: Duration = Duration::from_secs(60);

/// Logs that grow: the tests hold the leader at its WAL write and count the
/// I/O of the frames, which the zeros of a preallocated log would add to.
fn config(durability: Durability, group_commit: GroupCommitConfig) -> Config {
    Config { group_commit, wal_preallocate: false, ..Config::from(&Options { durability, ..Options::default() }) }
}

fn open(vfs: Arc<dyn Vfs>, cfg: &Config) -> Arc<KernoDb> {
    Arc::new(KernoDb::open(vfs, Path::new("/db"), cfg).unwrap())
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap_or_else(|| panic!("no stat {name}"))
}

/// Commit `i` of a test that needs every commit to have the same size: one
/// key, one 100-byte value.
fn batch(i: usize) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(format!("w{i:03}"), vec![i as u8; 100]);
    b
}

/// The bytes a commit counts against the queue limits (its encoding).
fn encoded_len(b: &WriteBatch) -> usize {
    canonicalize(b.ops().iter().map(|op| match op {
        Op::Put { key, value } => CommitOp::Put { key, value },
        Op::Delete { key } => CommitOp::Delete { key },
        Op::DeleteRange { start, end } => CommitOp::DeleteRange { start, end },
    }))
    .encoded_len()
}

fn is_wal(op: &HookOp<'_>) -> bool {
    op.path.extension().is_some_and(|e| e == "wal")
}

/// The WAL write of a frame (the header is written at offset 0).
fn is_frame_write(op: &HookOp<'_>) -> bool {
    op.kind == HookOpKind::WriteAt && op.point == HookPoint::Before && is_wal(op) && op.offset > 0
}

fn is_wal_sync(op: &HookOp<'_>) -> bool {
    op.kind == HookOpKind::SyncData && op.point == HookPoint::Before && is_wal(op)
}

/// Holds the thread that calls [`hold`](Gate::hold) until the test calls
/// [`release`](Gate::release).
#[derive(Default)]
struct Gate {
    /// (a thread is held, released)
    state: Mutex<(bool, bool)>,
    cv: Condvar,
}

impl Gate {
    fn hold(&self) {
        let mut st = self.state.lock().unwrap();
        st.0 = true;
        self.cv.notify_all();
        while !st.1 {
            let (next, timeout) = self.cv.wait_timeout(st, LIMIT).unwrap();
            assert!(!timeout.timed_out(), "the held thread was never released");
            st = next;
        }
    }

    fn wait_held(&self) {
        let mut st = self.state.lock().unwrap();
        while !st.0 {
            let (next, timeout) = self.cv.wait_timeout(st, LIMIT).unwrap();
            assert!(!timeout.timed_out(), "no thread reached the gate");
            st = next;
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.cv.notify_all();
    }
}

/// Waits (polling the engine, with a deadline) until `bytes` have been
/// queued at once: every writer started so far has joined the queue.
fn wait_queued(db: &KernoDb, bytes: usize) {
    let deadline = Instant::now() + LIMIT;
    while (stat(db, "queued_bytes_max") as usize) < bytes {
        assert!(Instant::now() < deadline, "writers did not queue: {} of {bytes} bytes", stat(db, "queued_bytes_max"));
        std::thread::yield_now();
    }
}

/// Commits `batches` on one thread each; results arrive by writer index.
struct Commits {
    rx: mpsc::Receiver<(usize, Result<u64>)>,
    tx: mpsc::Sender<(usize, Result<u64>)>,
    started: usize,
}

impl Commits {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Commits { rx, tx, started: 0 }
    }

    fn spawn(&mut self, db: &Arc<KernoDb>, i: usize) {
        let (db, tx) = (db.clone(), self.tx.clone());
        std::thread::Builder::new()
            .name(format!("commit-{i}"))
            .spawn(move || {
                let r = db.commit(&batch(i));
                drop(db);
                let _ = tx.send((i, r));
            })
            .unwrap();
        self.started += 1;
    }

    /// Every result still outstanding, by writer; fails the test if one
    /// does not arrive (a follower waiting for a leader that is gone).
    fn finish(&mut self) -> Vec<(usize, Result<u64>)> {
        let mut out: Vec<_> = (0..self.started)
            .map(|_| self.rx.recv_timeout(LIMIT).expect("a commit never returned: a follower hangs"))
            .collect();
        self.started = 0;
        out.sort_by_key(|(i, _)| *i);
        out
    }
}

/// 16 writers commit at once: every acknowledged sequence is unique, they
/// are exactly `1..=total`, and the groups they formed add up.
#[test]
fn seqs_unique_and_contiguous_under_16_writers() {
    const WRITERS: usize = 16;
    const COMMITS: u64 = 30;
    let disk = writers_disk(5);
    let db = open(disk.as_vfs(), &config(Durability::Sync, GroupCommitConfig::default()));
    let logs = run_writers(&db, WRITERS, COMMITS, &writer_batches(), None, "16 writers");
    let mut seqs: Vec<u64> = logs.iter().flat_map(|l| l.acked.iter().copied()).collect();
    for (i, log) in logs.iter().enumerate() {
        assert!(log.error.is_none(), "writer {i}: {:?}", log.error);
        assert!(log.acked.windows(2).all(|w| w[0] < w[1]), "writer {i}: sequences must increase");
    }
    seqs.sort_unstable();
    let total = WRITERS as u64 * COMMITS;
    assert_eq!(seqs, (1..=total).collect::<Vec<_>>());
    assert_eq!(db.snapshot().unwrap().seq(), total);
    assert_eq!(stat(&db, "commits"), total);
    assert_eq!(stat(&db, "durable_seq"), total, "Sync mode: everything published is durable");
    // Every commit is in exactly one group, and the frames say so.
    let (groups, widest) = (stat(&db, "commit_groups"), stat(&db, "group_commits_max"));
    assert!(
        (1..=total).contains(&groups) && widest >= 1 && widest * groups >= total,
        "{groups} groups, widest {widest}"
    );
    assert_eq!(stat(&db, "wal_syncs"), groups, "one fsync per group in Sync mode");
}

/// Eight writers against a slow `fsync`: groups form, and the WAL is synced
/// less than once per two commits (ADR-0001 condition 3; target T6).
#[test]
fn fsyncs_per_commit_below_half_with_8_writers() {
    const WRITERS: usize = 8;
    const COMMITS: u64 = 40;
    let disk = SimVfs::new(8);
    disk.set_sync_latency(Duration::from_millis(1));
    let (vfs, io) = CountingVfs::wrap(disk.as_vfs());
    let db = open(vfs, &config(Durability::Sync, GroupCommitConfig::default()));
    let before = io.snapshot();
    let logs = run_writers(&db, WRITERS, COMMITS, &writer_batches(), None, "8 writers");
    assert!(logs.iter().all(|l| l.error.is_none()));
    let total = WRITERS as u64 * COMMITS;
    let io = io.snapshot().since(&before);
    assert_eq!(io.dir_syncs, 0, "no memtable switch in this run");
    let per_commit = io.file_syncs as f64 / total as f64;
    assert!(per_commit <= 0.5, "{} fsyncs for {total} commits = {per_commit:.2} per commit", io.file_syncs);
    assert_eq!(io.writes, io.file_syncs, "one write and one fsync per group");
    assert_eq!(stat(&db, "wal_syncs"), io.file_syncs);
    assert!(stat(&db, "group_commits_max") > 1);
}

/// One writer: every commit is its own group, written and synced alone.
#[test]
fn single_writer_fast_path_one_frame_per_commit() {
    let disk = SimVfs::new(1);
    let (vfs, counters) = CountingVfs::wrap(disk.as_vfs());
    let db = open(vfs, &config(Durability::Sync, GroupCommitConfig::default()));
    let before = counters.snapshot();
    for i in 0..50 {
        assert_eq!(db.commit(&batch(i)).unwrap(), i as u64 + 1);
    }
    let io = counters.snapshot().since(&before);
    assert_eq!((io.writes, io.file_syncs), (50, 50), "one frame and one fsync per commit");
    assert_eq!((stat(&db, "commits"), stat(&db, "commit_groups"), stat(&db, "group_commits_max")), (50, 50, 1));
    assert_eq!(stat(&db, "wal_syncs"), 50);
    // An empty batch is not a commit: no group, no I/O, the current sequence.
    assert_eq!(db.commit(&WriteBatch::new()).unwrap(), 50);
    assert_eq!(counters.snapshot().since(&before), io);
    assert_eq!(stat(&db, "commit_groups"), 50);
}

/// What a run in which the second group fails left behind.
struct FailedGroup {
    /// The result of each writer (writer 0 led the first, successful group).
    results: Vec<Result<u64>>,
    /// The sequence recovered after reopening.
    recovered: u64,
    /// Which of the writers' keys the recovered database holds.
    present: Vec<usize>,
}

/// Writer 0 leads a group whose `fsync` is held; `followers` writers queue
/// behind it; the next `fsync` (the second group's) fails. Then the handle is
/// checked for poisoning and reopened after `crash`.
fn run_failed_group(followers: usize, max_group_commits: usize, crash: Option<CrashKind>) -> FailedGroup {
    let disk = SimVfs::new(21);
    disk.set_tear_unit(1);
    let gate = Arc::new(Gate::default());
    let syncs = Arc::new(AtomicUsize::new(0));
    let hook_gate = gate.clone();
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        if !is_wal_sync(op) {
            return HookAction::Proceed;
        }
        match syncs.fetch_add(1, Ordering::SeqCst) {
            0 => {
                hook_gate.hold();
                HookAction::Proceed
            }
            1 => HookAction::Fail(std::io::ErrorKind::Other),
            _ => HookAction::Proceed,
        }
    });
    let group_commit = GroupCommitConfig { max_group_commits, ..GroupCommitConfig::default() };
    let db = open(vfs, &config(Durability::Sync, group_commit));
    let mut commits = Commits::new();
    commits.spawn(&db, 0);
    gate.wait_held();
    for i in 1..=followers {
        commits.spawn(&db, i);
    }
    wait_queued(&db, (followers + 1) * encoded_len(&batch(0)));
    gate.release();
    let results: Vec<Result<u64>> = commits.finish().into_iter().map(|(_, r)| r).collect();

    // The handle is poisoned: nothing more is written or synced.
    let ops = disk.mutating_ops();
    assert!(matches!(db.commit(&batch(99)), Err(Error::Poisoned(_))));
    assert!(matches!(db.sync(), Err(Error::Poisoned(_))));
    assert_eq!(disk.mutating_ops(), ops);
    drop(db);

    let disk = match crash {
        Some(kind) => disk.crash_now(kind),
        None => {
            disk.disarm();
            disk
        }
    };
    let db = open(disk.as_vfs(), &config(Durability::Sync, GroupCommitConfig::default()));
    let snap = db.snapshot().unwrap();
    let present: Vec<usize> =
        (0..=followers).filter(|&i| snap.get(format!("w{i:03}").as_bytes()).unwrap().is_some()).collect();
    for &i in &present {
        assert_eq!(snap.get(format!("w{i:03}").as_bytes()).unwrap(), Some(vec![i as u8; 100]));
    }
    FailedGroup { results, recovered: snap.seq(), present }
}

fn kinds(results: &[Result<u64>]) -> (usize, usize, usize) {
    let count = |f: fn(&Result<u64>) -> bool| results.iter().filter(|r| f(r)).count();
    (count(|r| r.is_ok()), count(|r| matches!(r, Err(Error::Io(_)))), count(|r| matches!(r, Err(Error::Poisoned(_)))))
}

/// The `fsync` of a group of five fails: every member gets `Io`, the handle
/// is poisoned, and the frame is all there or not there at all after a
/// restart or a power loss (one CRC covers the whole group).
#[test]
fn failed_group_fsync_fails_all_members_and_poisons() {
    for crash in [None, Some(CrashKind::PowerLoss)] {
        let ctx = format!("after {crash:?}");
        let run = run_failed_group(5, 1024, crash);
        assert!(matches!(run.results[0], Ok(1)), "{ctx}: the first group succeeded: {:?}", run.results[0]);
        assert_eq!(kinds(&run.results[1..]), (0, 5, 0), "{ctx}: all five members of the failed group get Io");
        // Writer 0 was durable; the group frame is atomic.
        assert!(
            run.recovered == 1 || run.recovered == 6,
            "{ctx}: recovered sequence {} splits the group",
            run.recovered
        );
        assert_eq!(run.present.len() as u64, run.recovered, "{ctx}: {:?}", run.present);
        if crash.is_none() {
            assert_eq!(run.recovered, 6, "a restart keeps the written frame");
        }
    }
}

/// The group that fails holds two commits; the three queued behind it get
/// `Poisoned` without ever being written.
#[test]
fn queued_requests_get_poisoned_after_failure() {
    let run = run_failed_group(5, 2, None);
    assert!(matches!(run.results[0], Ok(1)));
    assert_eq!(kinds(&run.results[1..]), (0, 2, 3), "{:?}", run.results);
    // Only the failed group's frame reached the file: writer 0 and the two
    // commits that were in it.
    assert_eq!(run.recovered, 3);
    assert_eq!(run.present.len(), 3);
}

/// A panic in the leader (here a WAL write that panics) poisons the handle
/// and wakes the followers with an error: nothing hangs.
#[test]
fn leader_panic_poisons_and_wakes_followers() {
    let disk = SimVfs::new(3);
    let gate = Arc::new(Gate::default());
    let hook_gate = gate.clone();
    let armed = Arc::new(AtomicUsize::new(0));
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        if is_frame_write(op) && armed.fetch_add(1, Ordering::SeqCst) == 0 {
            hook_gate.hold();
            return HookAction::Panic;
        }
        HookAction::Proceed
    });
    let db = open(vfs, &config(Durability::Sync, GroupCommitConfig::default()));
    let mut commits = Commits::new();
    commits.spawn(&db, 0);
    gate.wait_held();
    for i in 1..=4 {
        commits.spawn(&db, i);
    }
    wait_queued(&db, 5 * encoded_len(&batch(0)));
    gate.release();
    let results: Vec<Result<u64>> = commits.finish().into_iter().map(|(_, r)| r).collect();
    assert!(matches!(&results[0], Err(Error::Io(e)) if e.to_string().contains("panicked")), "{:?}", results[0]);
    for r in &results[1..] {
        assert!(matches!(r, Err(Error::Poisoned(m)) if m.contains("panicked")), "{r:?}");
    }
    assert!(matches!(db.commit(&batch(9)), Err(Error::Poisoned(_))));
    assert!(matches!(db.sync(), Err(Error::Poisoned(_))));
    // The panic published nothing.
    assert!(matches!(db.snapshot(), Err(Error::Poisoned(_))));
    drop(db);
    let db = open(disk.as_vfs(), &config(Durability::Sync, GroupCommitConfig::default()));
    assert_eq!(db.snapshot().unwrap().seq(), 0);
}

/// In `Sync` mode a reader never sees a sequence whose `fsync` has not
/// finished (I-16): while the leader is held inside the `fsync`, the commit
/// is written, applied and invisible; it appears when the `fsync` returns.
#[test]
fn sync_mode_readers_never_see_undurable_seq() {
    let disk = SimVfs::new(4);
    let gate = Arc::new(Gate::default());
    let hook_gate = gate.clone();
    let syncs = Arc::new(AtomicUsize::new(0));
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        if is_wal_sync(op) && syncs.fetch_add(1, Ordering::SeqCst) == 1 {
            hook_gate.hold();
        }
        HookAction::Proceed
    });
    let db = open(vfs, &config(Durability::Sync, GroupCommitConfig::default()));
    assert_eq!(db.commit(&batch(0)).unwrap(), 1);
    let mut commits = Commits::new();
    commits.spawn(&db, 1);
    gate.wait_held();
    let held = db.snapshot().unwrap();
    assert_eq!(held.seq(), 1, "the commit being synced is not published");
    assert_eq!(held.get(b"w001").unwrap(), None);
    assert_eq!(stat(&db, "durable_seq"), 1);
    gate.release();
    assert!(matches!(commits.finish()[0].1, Ok(2)));
    let after = db.snapshot().unwrap();
    assert_eq!((after.seq(), after.get(b"w001").unwrap()), (2, Some(vec![1u8; 100])));
    assert_eq!(held.get(b"w001").unwrap(), None, "an old snapshot keeps its view");
    assert_eq!(stat(&db, "durable_seq"), 2);
}

/// A group is one frame with one CRC: after a power loss in `Buffered` mode
/// the recovered sequence is a frame boundary, never the middle of a group.
#[test]
fn group_frame_torn_by_power_loss_recovers_prefix() {
    // Commit 1 is synced; commit 2 is written alone (its leader is held while
    // three writers queue); commits 3..=5 are written as one group.
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..40 {
        let disk = SimVfs::new(seed);
        if seed % 2 == 1 {
            disk.set_tear_unit(1);
        }
        let gate = Arc::new(Gate::default());
        let hook_gate = gate.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
            if is_frame_write(op) && writes.fetch_add(1, Ordering::SeqCst) == 1 {
                hook_gate.hold();
            }
            HookAction::Proceed
        });
        let db = open(vfs, &config(Durability::Buffered, GroupCommitConfig::default()));
        assert_eq!(db.commit(&batch(0)).unwrap(), 1);
        assert_eq!(db.sync().unwrap(), 1);
        let mut commits = Commits::new();
        commits.spawn(&db, 1);
        gate.wait_held();
        for i in 2..=4 {
            commits.spawn(&db, i);
        }
        wait_queued(&db, 4 * encoded_len(&batch(0)));
        gate.release();
        assert!(commits.finish().iter().all(|(_, r)| r.is_ok()));
        assert_eq!(stat(&db, "commit_groups"), 3);
        assert_eq!(stat(&db, "group_commits_max"), 3);
        let image = disk.crash_now(CrashKind::PowerLoss);
        drop(db);
        let db = open(image.as_vfs(), &config(Durability::Buffered, GroupCommitConfig::default()));
        let snap = db.snapshot().unwrap();
        let s = snap.seq();
        assert!([1, 2, 5].contains(&s), "seed {seed}: recovered sequence {s} is inside a group");
        let present = (0..=4).filter(|i| snap.get(format!("w{i:03}").as_bytes()).unwrap().is_some()).count() as u64;
        assert_eq!(present, s, "seed {seed}: contents do not match the recovered sequence");
        seen.insert(s);
    }
    assert!(seen.len() >= 2, "the power loss never cut anything: {seen:?}");
}

/// 64 writers with large values against a small queue limit: the bytes
/// queued never exceed the limit by more than one commit, everything is
/// committed, and groups still form.
#[test]
fn queue_memory_bounded_under_64_writers() {
    const WRITERS: usize = 64;
    const COMMITS: u64 = 6;
    const VALUE: usize = 40 << 10;
    const MAX_QUEUED: usize = 160 << 10;
    let disk = writers_disk(6);
    let group_commit =
        GroupCommitConfig { max_group_bytes: 100 << 10, max_queued_bytes: MAX_QUEUED, ..Default::default() };
    let db = open(disk.as_vfs(), &config(Durability::Sync, group_commit));
    let big: BatchFn = Arc::new(move |i: usize, n: u64| {
        let mut b = WriteBatch::new();
        b.put(format!("w{i}/{n}"), vec![n as u8; VALUE]);
        b
    });
    let each = encoded_len(&big(0, 1));
    let logs = run_writers(&db, WRITERS, COMMITS, &big, None, "64 writers");
    assert!(logs.iter().all(|l| l.error.is_none() && l.acked.len() as u64 == COMMITS));
    let max = stat(&db, "queued_bytes_max") as usize;
    assert!(max <= MAX_QUEUED + each, "{max} bytes were queued at once, over {MAX_QUEUED} + one commit of {each}");
    assert!(max > each, "no commit ever waited behind another: the bound was not exercised");
    assert!(stat(&db, "group_commits_max") >= 2 && stat(&db, "commit_groups") < WRITERS as u64 * COMMITS);
    assert_eq!(db.snapshot().unwrap().seq(), WRITERS as u64 * COMMITS);
}

/// splitmix64 step: a cheap source of random numbers shared by the hook's
/// threads.
fn next_random(state: &AtomicU64) -> u64 {
    let mut z = state.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Starts parking_lot's deadlock detector on a thread: a deadlock between
/// the engine's locks ends the process with the threads involved, instead of
/// a test that never returns.
#[cfg(feature = "deadlock_detection")]
fn watch_for_deadlocks() {
    std::thread::Builder::new()
        .name("deadlock-detector".into())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_millis(200));
                let deadlocks = parking_lot::deadlock::check_deadlock();
                if deadlocks.is_empty() {
                    continue;
                }
                eprintln!("{} deadlock(s) detected", deadlocks.len());
                for (i, threads) in deadlocks.iter().enumerate() {
                    for t in threads {
                        eprintln!("deadlock #{i}, thread {:?}:\n{:?}", t.thread_id(), t.backtrace());
                    }
                }
                std::process::exit(101);
            }
        })
        .unwrap();
}

#[cfg(not(feature = "deadlock_detection"))]
fn watch_for_deadlocks() {}

/// 10^4 commits from four writers while the file system yields and sleeps at
/// random in front of the WAL's writes and syncs: leadership changes hands
/// at every interleaving the delays produce. Every commit is acknowledged
/// once with a contiguous sequence, and nothing deadlocks (the writers have a
/// deadline, and with `--features deadlock_detection` parking_lot watches the
/// engine's locks).
#[test]
fn stress_handoff_with_random_delays() {
    const WRITERS: usize = 4;
    watch_for_deadlocks();
    for (name, max_group_commits, commits) in [("default groups", 1024, 1500u64), ("groups of 3", 3, 1000)] {
        let disk = SimVfs::new(77);
        let random = Arc::new(AtomicU64::new(0xC0FFEE));
        let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
            let r = next_random(&random);
            if r.is_multiple_of(4) {
                std::thread::yield_now();
            }
            if is_wal(op)
                && matches!(op.kind, HookOpKind::WriteAt | HookOpKind::SyncData)
                && op.point == HookPoint::Before
            {
                match (r >> 8) % 16 {
                    0 => return HookAction::Delay(Duration::from_micros((r >> 16) % 300)),
                    1..=4 => std::thread::yield_now(),
                    _ => {}
                }
            }
            HookAction::Proceed
        });
        let group_commit = GroupCommitConfig { max_group_commits, ..Default::default() };
        let db = open(vfs, &config(Durability::Sync, group_commit));
        let logs = run_writers(&db, WRITERS, commits, &writer_batches(), None, name);
        let mut seqs: Vec<u64> = logs.iter().flat_map(|l| l.acked.iter().copied()).collect();
        assert!(logs.iter().all(|l| l.error.is_none()), "{name}");
        seqs.sort_unstable();
        let total = WRITERS as u64 * commits;
        assert_eq!(seqs, (1..=total).collect::<Vec<_>>(), "{name}");
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), total);
        for w in 0..WRITERS {
            assert_eq!(
                snap.get(format!("w{w}/last").as_bytes()).unwrap(),
                Some(commits.to_le_bytes().to_vec()),
                "{name}"
            );
        }
        assert!(stat(&db, "group_commits_max") <= max_group_commits as u64);
    }
}
