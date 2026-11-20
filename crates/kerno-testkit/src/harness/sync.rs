//! `sync()` suite (ADR-0002, invariant I-17) for engines that implement
//! [`SyncEngine`] (see [`sync_test_suite!`](crate::sync_test_suite)).
//!
//! `sync()` returning `Ok(s)` promises that every commit `<= s` survives a
//! power loss, and `s` is at least every commit acknowledged before the
//! call; a failed `sync` poisons the handle.
//!
//! | Test | Failure model | Requirement |
//! |---|---|---|
//! | `sync_makes_buffered_commits_durable` | random commits and `sync` calls in `Buffered` mode, power loss at a random point, several rounds | `sync` returns the last acknowledged sequence; recovery: `synced <= s <= attempted` and exactly the model at `s` |
//! | `sync_failure_poisons` | the `fsync` of a `sync` fails (both `fsync` models), then a restart or a power loss | `sync` returns `Io`; then `commit` and `sync` return `Poisoned`; restart: `acked <= s`; power loss: `synced <= s` |
//! | `sync_on_poisoned_returns_poisoned` | a commit's write fails (`Sync` and `Buffered`) | `sync` returns `Poisoned`, and keeps doing so; after reopening it works again |
//! | `power_loss_at_every_io_point_buffered_with_sync` | scenario A with `sync` steps, power loss at every mutating operation (`Buffered`) | `durable <= s <= attempted`, where `durable` is the last `sync`, clean close or open |
//! | `scenario_c_power_loss_at_every_io_point_buffered_with_sync` | scenario C, writers also call `sync`, power loss at every mutating operation (`Buffered`) | every `sync` returns at least every sequence acknowledged to any writer before the call; no holes; `s >=` every sequence a successful `sync` returned |
//! | `scenario_c_io_failure_at_every_point_buffered_with_sync` | the same with one I/O error (both `fsync` models), then a restart or a power loss | restart: every acknowledged commit; power loss: every successful `sync` |
//!
//! Every test ends like the other sweeps: one more commit, a clean close, a
//! power loss, and a full model check. The calls that fail and the calls on
//! a poisoned handle run under [`call_with_timeout`], so an engine that
//! hangs there fails the test instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kerno_core::contract::{Options, Snapshot, SyncEngine};
use kerno_core::{Durability, Error, WriteBatch};

use super::concurrency::{SyncFn, call_with_timeout};
use super::crash::{Step, commits, every_point_c, failure_sweep_c, keeps_working, scenario};
use super::faults::After;
use super::{Model, Workload, apply, assert_snapshot_matches, for_each_seed, open, seeds};
use crate::rng::Rng;
use crate::vfs::{CrashKind, FsyncFailure, OpKind, SimVfs};

/// `SyncEngine::sync` as a [`SyncFn`], for the multi-writer runners.
pub fn sync_fn<E: SyncEngine>() -> SyncFn<E> {
    Arc::new(|db: &E| db.sync())
}

/// Name of the [`Step::Ext`] that [`sync_step`] builds.
pub const SYNC_STEP: &str = "sync";

/// A [`Step::Ext`] that calls `sync` and stores the sequence it returned in
/// `synced`.
pub fn sync_step<E: SyncEngine>(synced: &Arc<AtomicU64>) -> Step<E> {
    let cell = synced.clone();
    Step::Ext(
        SYNC_STEP,
        Arc::new(move |db: &E| {
            cell.store(db.sync()?, Ordering::SeqCst);
            Ok(())
        }),
    )
}

/// `steps` with a [`sync_step`] right after every [`Step::Open`] and after
/// every `every`-th commit.
pub fn with_sync_steps<E: SyncEngine>(steps: Vec<Step<E>>, every: usize, synced: &Arc<AtomicU64>) -> Vec<Step<E>> {
    let mut out = Vec::with_capacity(steps.len() + steps.len() / every + 4);
    let mut commits = 0usize;
    for step in steps {
        let sync_after = match &step {
            Step::Open => true,
            Step::Commit(_) => {
                commits += 1;
                commits.is_multiple_of(every)
            }
            _ => false,
        };
        out.push(step);
        if sync_after {
            out.push(sync_step(synced));
        }
    }
    out
}

fn options(durability: Durability) -> Options {
    let mut opts = Options::small_for_tests();
    opts.durability = durability;
    opts
}

/// The model after the first `s` commits of `history`.
fn model_at<'a>(history: impl IntoIterator<Item = &'a WriteBatch>, s: u64) -> Model {
    let mut model = Model::new();
    for b in history.into_iter().take(s as usize) {
        apply(&mut model, b);
    }
    model
}

fn small_batch(i: u64) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(format!("key-{i:04}"), vec![i as u8; 100]);
    b
}

/// Several rounds of: open (the recovered state must lie between the
/// durable floor and the last attempted commit), `sync` (which returns the
/// recovered sequence), random commits with random `sync` calls in
/// `Buffered` mode (each `sync` returns exactly the last acknowledged
/// sequence and raises the floor), and a power loss at a random point.
pub fn sync_makes_buffered_commits_durable<E: SyncEngine>() {
    const ROUNDS: usize = 4;
    const COMMITS_PER_ROUND: u64 = 60;
    for_each_seed("sync_makes_buffered_commits_durable", seeds(40), |seed| {
        let mut rng = Rng::new(seed);
        let mut w = Workload::new(seed ^ 0x5EED);
        w.key_space = rng.range(40, 400);
        let opts = options(Durability::Buffered);
        let mut disk = SimVfs::new(seed);
        if seed % 2 == 1 {
            disk.set_tear_unit(1);
        }
        // Every batch ever attempted, indexed by sequence - 1.
        let mut history: Vec<WriteBatch> = Vec::new();
        // Lowest sequence a recovery may return: the last `sync` that
        // returned before the power loss.
        let mut floor = 0u64;
        for round in 0..=ROUNDS {
            let ctx = format!("seed {seed} round {round}");
            let db: E = open(&disk.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
            let s = db.snapshot().unwrap().seq();
            assert!(
                floor <= s && s as usize <= history.len(),
                "{ctx}: recovered sequence {s} outside {floor}..={} (the floor is what sync returned)",
                history.len()
            );
            history.truncate(s as usize);
            let mut model = model_at(&history, s);
            assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &format!("{ctx}: after recovery"));
            assert_eq!(db.sync().unwrap(), s, "{ctx}: sync right after open must return the recovered sequence");
            floor = s;
            if round == ROUNDS {
                keeps_working(db, &disk, &opts, model, s, &mut rng, &ctx);
                return;
            }
            disk.crash_after(rng.range(1, COMMITS_PER_ROUND * 2), CrashKind::PowerLoss);
            for _ in 0..COMMITS_PER_ROUND {
                let b = w.batch(&model);
                history.push(b.clone());
                match db.commit(&b) {
                    Ok(seq) => {
                        assert_eq!(seq, history.len() as u64, "{ctx}: unexpected commit sequence");
                        apply(&mut model, &b);
                    }
                    Err(e) => {
                        assert!(disk.has_crashed(), "{ctx}: commit failed without a crash: {e}");
                        break;
                    }
                }
                if rng.chance(1, 5) {
                    match db.sync() {
                        Ok(synced) => {
                            let acked = history.len() as u64;
                            assert_eq!(synced, acked, "{ctx}: sync must return the last acknowledged sequence");
                            floor = synced;
                        }
                        Err(e) => {
                            assert!(disk.has_crashed(), "{ctx}: sync failed without a crash: {e}");
                            break;
                        }
                    }
                }
            }
            let image = match disk.take_crash_image() {
                Some(image) => image,
                None => disk.crash_now(CrashKind::PowerLoss),
            };
            drop(db);
            disk = image;
        }
    });
}

/// In `Buffered` mode, after a successful `sync` and more commits, the
/// `fsync` of the next `sync` fails: `sync` must return `Io`, and later
/// `commit` and `sync` calls `Poisoned`. Then the process restarts (every
/// acknowledged commit must be there) or the power fails (everything up to
/// the successful `sync` must be there), with failed syncs keeping their
/// writes pending or dropping them as Linux does.
pub fn sync_failure_poisons<E: SyncEngine>() {
    const SYNCED: u64 = 5;
    const ACKED: u64 = 10;
    for fsync in [FsyncFailure::KeepPending, FsyncFailure::LinuxDropPages] {
        for after in [After::Restart, After::PowerLoss] {
            for seed in 0..4 {
                let ctx = format!("failed fsync {fsync:?}, then {after:?}, seed {seed}");
                let disk = SimVfs::new(seed);
                if seed % 2 == 1 {
                    disk.set_tear_unit(1);
                }
                disk.set_fsync_failure(fsync);
                let opts = options(Durability::Buffered);
                let db: E = open(&disk.as_vfs(), &opts).unwrap();
                let history: Vec<WriteBatch> = (1..=ACKED).map(small_batch).collect();
                for (i, b) in history.iter().enumerate() {
                    assert_eq!(db.commit(b).unwrap(), i as u64 + 1, "{ctx}: commit");
                    if i as u64 + 1 == SYNCED {
                        assert_eq!(db.sync().unwrap(), SYNCED, "{ctx}: first sync");
                    }
                }
                let then = match after {
                    After::Restart => None,
                    After::PowerLoss => Some(CrashKind::PowerLoss),
                };
                disk.fail_when(|op| op.kind == OpKind::SyncData, 1, then);
                let db = Arc::new(db);
                let r = call_with_timeout(&db, "the sync whose fsync fails", &ctx, |db: &E| db.sync());
                assert!(disk.fault_injected(), "{ctx}: sync issued no sync_data with buffered commits pending");
                assert!(matches!(r, Err(Error::Io(_))), "{ctx}: a failed fsync must fail sync with Io, got {r:?}");
                if !disk.has_crashed() {
                    let c = call_with_timeout(&db, "a commit after a failed sync", &ctx, |db: &E| {
                        db.commit(&small_batch(ACKED + 1))
                    });
                    assert!(matches!(c, Err(Error::Poisoned(_))), "{ctx}: commit after a failed sync: {c:?}");
                    let s = call_with_timeout(&db, "a sync after a failed sync", &ctx, |db: &E| db.sync());
                    assert!(matches!(s, Err(Error::Poisoned(_))), "{ctx}: sync after a failed sync: {s:?}");
                }
                let (disk, lower) = match after {
                    After::Restart => {
                        disk.disarm();
                        (disk, ACKED)
                    }
                    After::PowerLoss => {
                        let image = match disk.take_crash_image() {
                            Some(image) => image,
                            None => disk.crash_now(CrashKind::PowerLoss),
                        };
                        (image, SYNCED)
                    }
                };
                drop(db);
                let db: E = open(&disk.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
                let snap = db.snapshot().unwrap();
                let s = snap.seq();
                assert!(lower <= s && s <= ACKED, "{ctx}: recovered sequence {s} outside {lower}..={ACKED}");
                let model = model_at(&history, s);
                let mut rng = Rng::new(seed);
                assert_snapshot_matches(&snap, &model, &mut rng, &ctx);
                drop(snap);
                keeps_working(db, &disk, &opts, model, s, &mut rng, &ctx);
            }
        }
    }
}

/// A failed write poisons the handle (`Sync` and `Buffered` mode): `sync`
/// returns `Poisoned` without I/O, and keeps doing so; after reopening,
/// `sync` works again and returns the recovered sequence.
pub fn sync_on_poisoned_returns_poisoned<E: SyncEngine>() {
    const ACKED: u64 = 3;
    for durability in [Durability::Sync, Durability::Buffered] {
        let ctx = format!("{durability:?}");
        let disk = SimVfs::new(3);
        let opts = options(durability);
        let db: E = open(&disk.as_vfs(), &opts).unwrap();
        let history: Vec<WriteBatch> = (1..=ACKED + 1).map(small_batch).collect();
        for (i, b) in history[..ACKED as usize].iter().enumerate() {
            assert_eq!(db.commit(b).unwrap(), i as u64 + 1, "{ctx}: commit");
        }
        disk.fail_when(|op| op.kind == OpKind::WriteAt, 1, None);
        let db = Arc::new(db);
        let failing = history[ACKED as usize].clone();
        let r = call_with_timeout(&db, "the commit whose write fails", &ctx, move |db: &E| db.commit(&failing));
        assert!(disk.fault_injected(), "{ctx}: the commit wrote nothing");
        assert!(matches!(r, Err(Error::Io(_))), "{ctx}: a commit whose write failed must return Io, got {r:?}");
        let ops = disk.mutating_ops();
        for attempt in 1..=2 {
            let s = call_with_timeout(&db, "a sync on a poisoned handle", &ctx, |db: &E| db.sync());
            assert!(matches!(s, Err(Error::Poisoned(_))), "{ctx}: sync #{attempt} on a poisoned handle: {s:?}");
        }
        let c = call_with_timeout(&db, "a commit on a poisoned handle", &ctx, |db: &E| db.commit(&small_batch(99)));
        assert!(matches!(c, Err(Error::Poisoned(_))), "{ctx}: commit on a poisoned handle: {c:?}");
        assert_eq!(disk.mutating_ops(), ops, "{ctx}: a poisoned handle must not write or sync");
        drop(db);
        let db: E = open(&disk.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: reopen failed: {e}"));
        let s = db.snapshot().unwrap().seq();
        assert!((ACKED..=ACKED + 1).contains(&s), "{ctx}: recovered sequence {s} outside {ACKED}..={}", ACKED + 1);
        assert_eq!(db.sync().unwrap(), s, "{ctx}: sync after reopening");
        let model = model_at(&history, s);
        let mut rng = Rng::new(3);
        assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &ctx);
        keeps_working(db, &disk, &opts, model, s, &mut rng, &ctx);
    }
}

/// Commit counts of a run of steps with `sync` steps.
struct SyncRun {
    acked: u64,
    attempted: u64,
    /// The highest sequence known durable: the last successful `sync`,
    /// clean close or open.
    durable: u64,
}

/// Runs `steps` until the device fails (every error must come from a
/// crash), tracking what is known durable. A successful [`sync_step`] must
/// have stored exactly the last acknowledged sequence in `synced`.
fn run_with_syncs<E: SyncEngine>(
    disk: &SimVfs,
    steps: &[Step<E>],
    opts: &Options,
    synced: &AtomicU64,
    ctx: &str,
) -> SyncRun {
    let vfs = disk.as_vfs();
    let mut db: Option<E> = None;
    let mut run = SyncRun { acked: 0, attempted: 0, durable: 0 };
    for step in steps {
        match step {
            Step::Open => match open::<E>(&vfs, opts) {
                Ok(d) => {
                    db = Some(d);
                    // Recovery makes what it recovered durable (the extended
                    // failure suite checks it too).
                    run.durable = run.acked;
                }
                Err(e) => {
                    assert!(disk.has_crashed(), "{ctx}: open failed without a crash: {e}");
                    break;
                }
            },
            Step::Commit(b) => {
                run.attempted += 1;
                match db.as_ref().unwrap().commit(b) {
                    Ok(seq) => {
                        assert_eq!(seq, run.attempted, "{ctx}: unexpected commit sequence");
                        run.acked = seq;
                    }
                    Err(e) => {
                        assert!(disk.has_crashed(), "{ctx}: commit failed without a crash: {e}");
                        break;
                    }
                }
            }
            Step::Close => match db.take().unwrap().close() {
                Ok(()) => run.durable = run.acked,
                Err(e) => {
                    assert!(disk.has_crashed(), "{ctx}: close failed without a crash: {e}");
                    break;
                }
            },
            Step::Drop => drop(db.take()),
            Step::Ext(name, f) => {
                let handle = db.as_ref().unwrap_or_else(|| panic!("{ctx}: step {name} needs an open handle"));
                if let Err(e) = f(handle) {
                    assert!(disk.has_crashed(), "{ctx}: {name} failed without a crash: {e}");
                    break;
                }
                if *name == SYNC_STEP {
                    let s = synced.load(Ordering::SeqCst);
                    assert_eq!(s, run.acked, "{ctx}: sync returned {s} with {} commits acknowledged", run.acked);
                    run.durable = s;
                }
            }
        }
    }
    drop(db);
    run
}

/// Power loss at every mutating operation of scenario A with a `sync` step
/// after every open and every 8th commit, in `Buffered` mode: the recovered
/// sequence `s` must satisfy `durable <= s <= attempted`, where `durable` is
/// the last successful `sync`, clean close or open, and the state must be
/// exactly the model at `s`.
pub fn power_loss_at_every_io_point_buffered_with_sync<E: SyncEngine>() {
    let opts = options(Durability::Buffered);
    let synced = Arc::new(AtomicU64::new(0));
    let steps = with_sync_steps(scenario::<E>(7), 8, &synced);
    let batches = commits(&steps);
    let dry = SimVfs::new(0);
    let run = run_with_syncs::<E>(&dry, &steps, &opts, &synced, "dry run");
    assert_eq!(run.acked as usize, batches.len(), "dry run must complete");
    let total = dry.mutating_ops() + 20;
    let only: Option<u64> = std::env::var("SPIKE_POINT").ok().and_then(|s| s.parse().ok());
    for point in 1..=total {
        if only.is_some_and(|p| p != point) {
            continue;
        }
        let ctx = format!("PowerLoss (Buffered, with sync) before mutating op #{point} of {total}");
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let disk = SimVfs::new(point);
            if point % 2 == 1 {
                disk.set_tear_unit(1);
            }
            disk.crash_after(point, CrashKind::PowerLoss);
            let run = run_with_syncs::<E>(&disk, &steps, &opts, &synced, &ctx);
            let image = match disk.take_crash_image() {
                Some(image) => image,
                None => disk.crash_now(CrashKind::PowerLoss),
            };
            let db: E = open(&image.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
            let snap = db.snapshot().unwrap();
            let s = snap.seq();
            assert!(
                run.durable <= s && s <= run.attempted,
                "{ctx}: recovered seq {s} outside {}..={} (the floor is the last sync, close or open)",
                run.durable,
                run.attempted
            );
            let model = model_at(batches.iter().copied(), s);
            let mut rng = Rng::new(point);
            assert_snapshot_matches(&snap, &model, &mut rng, &ctx);
            drop(snap);
            keeps_working(db, &image, &opts, model, s, &mut rng, &ctx);
        }));
        if let Err(e) = r {
            eprintln!("\n*** {ctx}; reproduce with SPIKE_POINT={point}\n");
            std::panic::resume_unwind(e);
        }
    }
}

/// Scenario C where the writers also call `sync`, power loss at every
/// mutating operation (`Buffered`): no holes, and every sequence a
/// successful `sync` returned survives.
pub fn scenario_c_power_loss_at_every_io_point_buffered_with_sync<E: SyncEngine>() {
    every_point_c::<E>(CrashKind::PowerLoss, Durability::Buffered, Some(&sync_fn::<E>()));
}

/// Scenario C where the writers also call `sync`, one I/O error at every
/// mutating operation (`Buffered`; failed syncs keep their writes pending,
/// then drop them as Linux does), then a restart (every acknowledged commit
/// survives) or a power loss (every successful `sync` survives).
pub fn scenario_c_io_failure_at_every_point_buffered_with_sync<E: SyncEngine>() {
    for fsync in [FsyncFailure::KeepPending, FsyncFailure::LinuxDropPages] {
        failure_sweep_c::<E>(Durability::Buffered, fsync, &[After::Restart, After::PowerLoss], Some(&sync_fn::<E>()));
    }
}

/// Generates the `sync()` suite (module `shared_sync`) for an engine type
/// that implements [`SyncEngine`]:
///
/// ```ignore
/// kerno_testkit::sync_test_suite!(MyEngine);
/// ```
///
/// See the [module documentation](crate::harness::sync) for what each test
/// checks.
#[macro_export]
macro_rules! sync_test_suite {
    ($engine:ty) => {
        mod shared_sync {
            use super::*;
            use $crate::harness::sync as s;
            #[test]
            fn sync_makes_buffered_commits_durable() {
                s::sync_makes_buffered_commits_durable::<$engine>();
            }
            #[test]
            fn sync_failure_poisons() {
                s::sync_failure_poisons::<$engine>();
            }
            #[test]
            fn sync_on_poisoned_returns_poisoned() {
                s::sync_on_poisoned_returns_poisoned::<$engine>();
            }
            #[test]
            fn power_loss_at_every_io_point_buffered_with_sync() {
                s::power_loss_at_every_io_point_buffered_with_sync::<$engine>();
            }
            #[test]
            fn scenario_c_power_loss_at_every_io_point_buffered_with_sync() {
                s::scenario_c_power_loss_at_every_io_point_buffered_with_sync::<$engine>();
            }
            #[test]
            fn scenario_c_io_failure_at_every_point_buffered_with_sync() {
                s::scenario_c_io_failure_at_every_point_buffered_with_sync::<$engine>();
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::super::concurrency::mutants::{NoFsync, OwnSeqSync, SwallowsSyncErrors, SyncOwnOnly};
    use super::*;
    use crate::reference::LogEngine;

    fn fails(f: impl FnOnce()) -> bool {
        catch_unwind(AssertUnwindSafe(f)).is_err()
    }

    #[test]
    fn sync_steps_follow_every_open_and_every_nth_commit() {
        let synced = Arc::new(AtomicU64::new(0));
        let steps = with_sync_steps(scenario::<LogEngine>(7), 8, &synced);
        let syncs: Vec<usize> = steps
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Step::Ext(name, _) if *name == SYNC_STEP))
            .map(|(i, _)| i)
            .collect();
        // Three opens and 55 commits: 3 + 55 / 8 sync steps.
        assert_eq!(syncs.len(), 3 + 6);
        for &i in &syncs {
            assert!(matches!(steps[i - 1], Step::Open | Step::Commit(_)), "a sync step follows an open or a commit");
        }
        assert_eq!(commits(&steps).len(), 55, "sync steps do not commit");
    }

    #[test]
    fn suite_rejects_a_sync_without_fsync() {
        assert!(fails(sync_makes_buffered_commits_durable::<NoFsync>));
    }

    #[test]
    fn suite_rejects_a_sweep_with_a_sync_without_fsync() {
        assert!(fails(power_loss_at_every_io_point_buffered_with_sync::<NoFsync>));
    }

    #[test]
    fn suite_rejects_a_swallowed_fsync_error() {
        assert!(fails(sync_failure_poisons::<SwallowsSyncErrors>));
    }

    #[test]
    fn scenario_c_rejects_a_sync_without_fsync() {
        assert!(fails(scenario_c_power_loss_at_every_io_point_buffered_with_sync::<NoFsync>));
    }

    /// A `sync` that only accounts for its caller's own commits passes every
    /// single-threaded check (there, the caller's commits are all the
    /// commits); only the cross-writer lower bound of the multi-writer runs
    /// catches it. Whether one run does depends on thread timing: another
    /// writer's commit must be acknowledged between a writer's commit and
    /// its `sync` (for `SyncOwnOnly`, after another writer's `sync`), which
    /// [`SYNC_WAIT`](super::super::concurrency::SYNC_WAIT) makes likely.
    /// On a 4-core machine `OwnSeqSync` failed 300 of 300 complete scenario
    /// C runs and `SyncOwnOnly` about two in three, so a whole sweep (dozens
    /// of runs) is all but certain to catch both.
    #[test]
    fn scenario_c_rejects_a_sync_that_ignores_other_writers() {
        assert!(!fails(sync_makes_buffered_commits_durable::<OwnSeqSync>));
        assert!(fails(scenario_c_power_loss_at_every_io_point_buffered_with_sync::<OwnSeqSync>));
        assert!(!fails(sync_makes_buffered_commits_durable::<SyncOwnOnly>));
        assert!(fails(scenario_c_power_loss_at_every_io_point_buffered_with_sync::<SyncOwnOnly>));
    }
}
