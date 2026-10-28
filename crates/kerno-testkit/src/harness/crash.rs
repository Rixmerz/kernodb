//! Crash-consistency suite.
//!
//! Each seed runs several rounds of: open (recovery) → verify → random
//! commits → crash at a random mutating I/O operation. Crashes can also hit
//! *during* recovery. After every recovery the database must be exactly the
//! state after some commit `s` (its reported sequence), and `s` must be
//! within what the durability mode allows:
//!
//! | mode              | allowed recovered sequence `s`                  |
//! |-------------------|--------------------------------------------------|
//! | Sync (any crash)  | last acknowledged, or the one in flight          |
//! | Buffered + kill   | last acknowledged, or the one in flight          |
//! | Buffered + power  | anything from the round's starting point upward  |
//!
//! The deterministic scenarios swept over every mutating operation live here
//! too: scenario A ([`scenario`], one thread) and scenario C
//! ([`run_scenario_c`]: [`SCENARIO_C_WRITERS`] writers ×
//! [`SCENARIO_C_COMMITS`] commits at once, with a slow `sync_data`), swept
//! with [`every_point_c`] (crashes) and [`failure_sweep_c`] (one I/O error).

use std::fmt;
use std::sync::Arc;

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{Durability, Error, Result, WriteBatch};

use super::concurrency::{
    Floor, SyncFn, WriterLog, call_with_timeout, check_recovered, run_writers, writer_batches, writers_disk,
};
use super::faults::After;
use super::{Model, SuiteConfig, Workload, apply, assert_snapshot_matches, for_each_seed, open, read_all, seeds};
use crate::rng::Rng;
use crate::vfs::{CrashKind, FsyncFailure, SimVfs};

const ROUNDS: usize = 5;
const COMMITS_PER_ROUND: u64 = 80;

pub fn run<E: Engine>(kind: CrashKind, durability: Durability) {
    run_with_config::<E>(kind, durability, &SuiteConfig::default());
}

/// [`run`] with the workload of `cfg`.
pub fn run_with_config<E: Engine>(kind: CrashKind, durability: Durability, cfg: &SuiteConfig) {
    let name = format!("crash {kind:?} {durability:?}");
    for_each_seed(&name, seeds(60), |seed| one_seed::<E>(seed, kind, durability, cfg));
}

fn one_seed<E: Engine>(seed: u64, kind: CrashKind, durability: Durability, cfg: &SuiteConfig) {
    let mut rng = Rng::new(seed);
    let mut w = Workload::with_config(seed ^ 0xC0FFEE, cfg.workload);
    // Small key spaces mean heavy overwrite churn, which is what drives
    // page reuse, flushes, compactions and log rewrites.
    w.key_space = rng.range(40, 800);
    let mut opts = Options::small_for_tests();
    opts.durability = durability;

    let mut disk = SimVfs::new(seed);
    if seed % 2 == 1 {
        // Odd seeds: writes can tear at any byte, not only at sectors.
        disk.set_tear_unit(1);
    }
    // History of every batch ever attempted, indexed by sequence - 1.
    let mut history: Vec<WriteBatch> = Vec::new();
    // State known to be recovered at the start of the round, and its seq.
    let mut base_model = Model::new();
    let mut base_seq = 0u64;
    // Bounds for the next recovered sequence.
    let mut min_seq = 0u64;
    let mut max_seq = 0u64;
    let mut ops_per_commit_estimate = 20u64;

    for round in 0..ROUNDS {
        let ctx = format!("seed {seed} round {round}");
        let vfs = disk.as_vfs();
        if rng.chance(1, 5) {
            // Crash during recovery itself.
            disk.crash_after(rng.range(1, 40), kind);
        }
        let db: E = match open(&vfs, &opts) {
            Ok(db) => db,
            Err(e) => {
                assert!(disk.has_crashed(), "{ctx}: open failed without a crash: {e}");
                disk = disk.take_crash_image().expect("crash image");
                continue;
            }
        };
        // Recovery finished before the trigger fired: cancel it, so that
        // background work started by `open` cannot pull the device away in
        // the middle of verification. (If it already fired in that window,
        // the `has_crashed` branch below handles it.) The workload trigger
        // armed further down still covers crashes during that background work.
        disk.disarm();
        let snap = match db.snapshot() {
            Ok(s) => s,
            Err(e) => {
                assert!(disk.has_crashed(), "{ctx}: snapshot failed without a crash: {e}");
                drop(db);
                disk = disk.take_crash_image().expect("crash image");
                continue;
            }
        };
        let s = snap.seq();
        assert!(
            s >= min_seq && s <= max_seq,
            "{ctx}: recovered sequence {s} outside the allowed range {min_seq}..={max_seq}"
        );
        // Rebuild the model at `s` from the last verified (durable) base.
        assert!(s >= base_seq, "{ctx}: recovered sequence {s} is older than verified base {base_seq}");
        let mut model = base_model.clone();
        for b in &history[base_seq as usize..s as usize] {
            apply(&mut model, b);
        }
        if !disk.has_crashed() {
            assert_snapshot_matches(&snap, &model, &mut rng, &format!("{ctx}: after recovery at seq {s}"));
        } else {
            // The crash fired in background work started by `open`; reads
            // that miss the cache now fail, so verify what can be read.
            if let Ok(all) = read_all(&snap) {
                assert!(all == model, "{ctx}: recovered state differs: {}", super::diff(&model, &all));
            }
        }
        drop(snap);
        history.truncate(s as usize);
        base_model = model.clone();
        base_seq = s;

        if !disk.has_crashed() {
            let budget = COMMITS_PER_ROUND * ops_per_commit_estimate;
            disk.crash_after(rng.range(1, budget.max(2)), kind);
        }
        let ops_before = disk.mutating_ops();
        let mut acked = s;
        let mut attempted = s;
        let mut commits_done = 0u64;
        for _ in 0..COMMITS_PER_ROUND {
            if disk.has_crashed() {
                break;
            }
            let b = w.batch(&model);
            history.push(b.clone());
            attempted += 1;
            match db.commit(&b) {
                Ok(seq) => {
                    assert_eq!(seq, attempted, "{ctx}: unexpected commit sequence");
                    acked = seq;
                    apply(&mut model, &b);
                    commits_done += 1;
                }
                Err(_) => break,
            }
        }
        if let Some(used) = disk.mutating_ops().saturating_sub(ops_before).checked_div(commits_done) {
            ops_per_commit_estimate = (ops_per_commit_estimate + used.max(1)) / 2 + 1;
        }
        let image = if disk.has_crashed() {
            drop(db);
            disk.take_crash_image().expect("crash image")
        } else {
            let image = disk.crash_now(kind);
            drop(db);
            image
        };
        disk = image;

        // Allowed range for the next recovery.
        min_seq = match (durability, kind) {
            (Durability::Buffered, CrashKind::PowerLoss) => base_seq,
            _ => acked,
        };
        max_seq = attempted;
    }
}

/// An engine-specific action of a [`Step::Ext`].
pub type ExtFn<E> = Arc<dyn Fn(&E) -> Result<()> + Send + Sync>;

/// One step of a deterministic scenario (see [`scenario`] and
/// [`run_scenario`]).
pub enum Step<E> {
    Open,
    Commit(WriteBatch),
    Close,
    /// Drop the handle without `close` (process exit without shutdown).
    Drop,
    /// An engine-specific action on the open handle (`sync`, a directed
    /// compaction, a forced checkpoint, ...), named for diagnostics. It must
    /// not commit: the runners count commits from [`Step::Commit`] only.
    Ext(&'static str, ExtFn<E>),
}

impl<E> Clone for Step<E> {
    fn clone(&self) -> Self {
        match self {
            Step::Open => Step::Open,
            Step::Commit(b) => Step::Commit(b.clone()),
            Step::Close => Step::Close,
            Step::Drop => Step::Drop,
            Step::Ext(name, f) => Step::Ext(name, f.clone()),
        }
    }
}

impl<E> fmt::Debug for Step<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Step::Open => f.write_str("Open"),
            Step::Commit(b) => write!(f, "Commit({} ops)", b.ops().len()),
            Step::Close => f.write_str("Close"),
            Step::Drop => f.write_str("Drop"),
            Step::Ext(name, _) => write!(f, "Ext({name})"),
        }
    }
}

/// The batches of every [`Step::Commit`], in order (batch `i` gets
/// sequence `i + 1`).
pub fn commits<E>(steps: &[Step<E>]) -> Vec<&WriteBatch> {
    steps.iter().filter_map(|s| if let Step::Commit(b) = s { Some(b) } else { None }).collect()
}

/// Scenario A: a fixed scenario touching every phase of an engine's life:
/// creation, commits (small, large values, deletes), clean close, reopen,
/// unclean drop, recovery and more commits. Deterministic for a given seed;
/// the sweeps use seed 7.
pub fn scenario<E>(seed: u64) -> Vec<Step<E>> {
    scenario_with_config(seed, &SuiteConfig::default())
}

/// [`scenario`] with the workload of `cfg` (the same steps; with the default
/// configuration, exactly scenario A).
pub fn scenario_with_config<E>(seed: u64, cfg: &SuiteConfig) -> Vec<Step<E>> {
    let mut w = Workload::with_config(seed, cfg.workload);
    w.key_space = 120;
    w.large_per_mille = 100;
    let mut model = Model::new();
    let mut steps = vec![Step::Open];
    let mut commits = |n: usize, steps: &mut Vec<Step<E>>| {
        for _ in 0..n {
            let b = w.batch(&model);
            apply(&mut model, &b);
            steps.push(Step::Commit(b));
        }
    };
    commits(25, &mut steps);
    steps.push(Step::Close);
    steps.push(Step::Open);
    commits(20, &mut steps);
    steps.push(Step::Drop);
    steps.push(Step::Open);
    commits(10, &mut steps);
    steps.push(Step::Close);
    steps
}

/// Runs the scenario until the device fails (every error must come from a
/// crash). Returns (acked, attempted) commit counts.
pub fn run_scenario<E: Engine>(disk: &SimVfs, steps: &[Step<E>], opts: &Options) -> (u64, u64) {
    let vfs = disk.as_vfs();
    let mut db: Option<E> = None;
    let (mut acked, mut attempted) = (0u64, 0u64);
    for step in steps {
        match step {
            Step::Open => match open::<E>(&vfs, opts) {
                Ok(d) => db = Some(d),
                Err(e) => {
                    assert!(disk.has_crashed(), "open failed without a crash: {e}");
                    break;
                }
            },
            Step::Commit(b) => {
                attempted += 1;
                match db.as_ref().unwrap().commit(b) {
                    Ok(seq) => {
                        assert_eq!(seq, attempted, "unexpected commit sequence");
                        acked = seq;
                    }
                    Err(e) => {
                        assert!(disk.has_crashed(), "commit failed without a crash: {e}");
                        break;
                    }
                }
            }
            Step::Close => {
                if let Err(e) = db.take().unwrap().close() {
                    assert!(disk.has_crashed(), "close failed without a crash: {e}");
                    break;
                }
            }
            Step::Drop => drop(db.take()),
            Step::Ext(name, f) => {
                let handle = db.as_ref().unwrap_or_else(|| panic!("step {name} needs an open handle"));
                if let Err(e) = f(handle) {
                    assert!(disk.has_crashed(), "{name} failed without a crash: {e}");
                    break;
                }
            }
        }
    }
    drop(db);
    (acked, attempted)
}

/// Crash at *every* mutating I/O operation of a fixed scenario (the
/// predefined failure points of the failure matrix), for both crash kinds,
/// and check the recovered state each time. The expected outcome is the
/// same at every point: the state after commit `s`, with
/// `acked <= s <= attempted`, followed by a working database.
///
/// `durability` must be `Sync` for power loss; `Buffered` is meaningful with
/// [`CrashKind::ProcessKill`] (whatever was acknowledged was handed to the
/// OS and survives a process crash).
pub fn every_point<E: Engine>(kind: CrashKind, durability: Durability) {
    every_point_with::<E>(kind, durability, &|_| {});
}

/// [`every_point`] on devices that `configure` sets up before each run (an
/// opt-in failure model such as per-directory durability). The crash images
/// inherit that configuration, so the final power loss uses it too.
pub fn every_point_with<E: Engine>(kind: CrashKind, durability: Durability, configure: &dyn Fn(&SimVfs)) {
    every_point_steps::<E>(kind, durability, configure, &scenario::<E>(7));
}

/// [`every_point`] over the scenario built with the workload of `cfg`.
pub fn every_point_with_config<E: Engine>(kind: CrashKind, durability: Durability, cfg: &SuiteConfig) {
    every_point_steps::<E>(kind, durability, &|_| {}, &scenario_with_config::<E>(7, cfg));
}

/// [`every_point_with`] over any scenario `steps` (see [`run_scenario`]).
pub fn every_point_steps<E: Engine>(
    kind: CrashKind,
    durability: Durability,
    configure: &dyn Fn(&SimVfs),
    steps: &[Step<E>],
) {
    assert!(
        durability == Durability::Sync || kind != CrashKind::PowerLoss,
        "Buffered commits only promise to survive process kills at every point"
    );
    let mut opts = Options::small_for_tests();
    opts.durability = durability;
    let batches = commits(steps);
    let dry = SimVfs::new(0);
    configure(&dry);
    let (acked, _) = run_scenario::<E>(&dry, steps, &opts);
    assert_eq!(acked as usize, batches.len(), "dry run must complete");
    // Background work makes the exact count vary a little; go past it.
    let total = dry.mutating_ops() + 20;
    let only: Option<u64> = std::env::var("SPIKE_POINT").ok().and_then(|s| s.parse().ok());
    for point in 1..=total {
        if only.is_some_and(|p| p != point) {
            continue;
        }
        let ctx = format!("{kind:?} ({durability:?}) crash before mutating op #{point} of {total}");
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let disk = SimVfs::new(point);
            if point % 2 == 1 {
                disk.set_tear_unit(1);
            }
            configure(&disk);
            disk.crash_after(point, kind);
            let (acked, attempted) = run_scenario::<E>(&disk, steps, &opts);
            let image = if disk.has_crashed() { disk.take_crash_image().unwrap() } else { disk.crash_now(kind) };
            let vfs = image.as_vfs();
            let db: E = open(&vfs, &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
            let snap = db.snapshot().unwrap();
            let s = snap.seq();
            assert!(acked <= s && s <= attempted, "{ctx}: recovered seq {s} outside {acked}..={attempted}");
            let mut model = Model::new();
            for b in &batches[..s as usize] {
                apply(&mut model, b);
            }
            let mut rng = Rng::new(point);
            assert_snapshot_matches(&snap, &model, &mut rng, &ctx);
            drop(snap);
            // The recovered database must keep working, and what it commits
            // and closes afterwards must survive a real power loss.
            let mut b = WriteBatch::new();
            b.put("after-recovery", "x");
            assert_eq!(db.commit(&b).unwrap(), s + 1, "{ctx}: commit after recovery");
            apply(&mut model, &b);
            db.close().unwrap();
            let after = image.crash_now(CrashKind::PowerLoss);
            let db: E = open(&after.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: reopen after power loss: {e}"));
            assert_snapshot_matches(
                &db.snapshot().unwrap(),
                &model,
                &mut rng,
                &format!("{ctx}, then power loss after a clean close"),
            );
        }));
        if let Err(e) = r {
            eprintln!("\n*** {ctx}; reproduce with SPIKE_POINT={point}\n");
            std::panic::resume_unwind(e);
        }
    }
}

/// The end of a sweep point: one more commit on the recovered handle, a
/// clean close and a power loss; everything must survive.
pub fn keeps_working<E: Engine>(
    db: E,
    disk: &SimVfs,
    opts: &Options,
    mut model: Model,
    s: u64,
    rng: &mut Rng,
    ctx: &str,
) {
    let mut b = WriteBatch::new();
    b.put("after-recovery", "x");
    let seq = db.commit(&b).unwrap_or_else(|e| panic!("{ctx}: commit after recovery: {e}"));
    assert_eq!(seq, s + 1, "{ctx}: commit after recovery");
    apply(&mut model, &b);
    db.close().unwrap_or_else(|e| panic!("{ctx}: close after recovery: {e}"));
    let image = disk.crash_now(CrashKind::PowerLoss);
    let db: E = open(&image.as_vfs(), opts).unwrap_or_else(|e| panic!("{ctx}: reopen after power loss: {e}"));
    assert_snapshot_matches(
        &db.snapshot().unwrap(),
        &model,
        rng,
        &format!("{ctx}, then power loss after a clean close"),
    );
}

/// Writer threads of scenario C.
pub const SCENARIO_C_WRITERS: usize = 4;
/// Commits per writer in scenario C.
pub const SCENARIO_C_COMMITS: u64 = 10;

/// Where the failures of a scenario C run may come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failures {
    /// A crash ([`SimVfs::crash_after`]): every error needs the device to
    /// be gone.
    Crash,
    /// One injected I/O error ([`SimVfs::fail_after`]): every error needs
    /// the fault to have fired and is `Io` or `Poisoned`; a failed open is
    /// retried once (the fault is one-shot), and after a failed commit or
    /// `sync` the handle must be poisoned.
    Fault,
}

/// What a run of scenario C did.
#[derive(Debug, Default)]
pub struct ScenarioCRun {
    /// Whether the database opened (and the writers ran).
    pub opened: bool,
    /// What each writer did (empty if the open failed).
    pub logs: Vec<WriterLog>,
    /// Whether the final clean close succeeded.
    pub closed: bool,
}

impl ScenarioCRun {
    /// What the recovered state must contain: every acknowledged commit,
    /// except after a power loss in `Buffered` mode before the clean close,
    /// where only what a successful `sync` made durable must survive.
    pub fn floor(&self, durability: Durability, power_loss: bool) -> Floor {
        let acked = durability == Durability::Sync || !power_loss || self.closed;
        let seq = self.logs.iter().map(WriterLog::synced_max).max().unwrap_or(0);
        Floor { acked, seq }
    }
}

/// The device scenario C runs on at a sweep point: seeded by the point,
/// byte-granular tearing on odd points, and `sync_data` slowed down by
/// [`SYNC_LATENCY`](super::concurrency::SYNC_LATENCY) so that commits queue
/// up behind each other.
pub fn scenario_c_disk(point: u64) -> SimVfs {
    writers_disk(point)
}

/// Scenario C ("group", phase1-design.md §6.4): open,
/// [`SCENARIO_C_WRITERS`] writers make [`SCENARIO_C_COMMITS`] commits each
/// at once ([`writer_batch`](super::concurrency::writer_batch); with `sync`,
/// the writers also call it after the commits that
/// [`syncs_after`](super::concurrency::syncs_after) picks), then a clean
/// close. The run stops at the first failure, which must come from
/// `failures`. Threads make the interleaving, and so which commit a given
/// operation belongs to, vary between runs; the checks hold for any of them.
/// The probe commit on the handle after an injected failure runs under
/// [`call_with_timeout`]: it must return `Poisoned`, not hang.
pub fn run_scenario_c<E: Engine>(
    disk: &SimVfs,
    opts: &Options,
    sync: Option<&SyncFn<E>>,
    failures: Failures,
    ctx: &str,
) -> ScenarioCRun {
    let vfs = disk.as_vfs();
    let mut run = ScenarioCRun::default();
    let allowed = |what: &str, e: &Error| match failures {
        Failures::Crash => assert!(disk.has_crashed(), "{ctx}: {what} failed without a crash: {e}"),
        Failures::Fault => {
            assert!(disk.fault_injected(), "{ctx}: {what} failed without a fault: {e}");
            assert!(
                matches!(e, Error::Io(_) | Error::Poisoned(_)),
                "{ctx}: {what} failed with an unexpected error kind: {e}"
            );
        }
    };
    let db: E = match open::<E>(&vfs, opts) {
        Ok(db) => db,
        Err(e) => {
            allowed("open", &e);
            if failures == Failures::Crash || disk.has_crashed() {
                return run;
            }
            match open::<E>(&vfs, opts) {
                Ok(db) => db,
                Err(_) if disk.has_crashed() => return run,
                Err(e) => panic!("{ctx}: second open failed: {e}"),
            }
        }
    };
    run.opened = true;
    let db = Arc::new(db);
    run.logs = run_writers(&db, SCENARIO_C_WRITERS, SCENARIO_C_COMMITS, &writer_batches(), sync, ctx);
    let mut failed = false;
    for (i, log) in run.logs.iter().enumerate() {
        if let Some(e) = &log.error {
            allowed(&format!("writer {i}"), e);
            failed = true;
        }
    }
    if failed && failures == Failures::Fault && !disk.has_crashed() {
        let r = call_with_timeout(&db, "the probe commit after a failure", ctx, |db: &E| {
            let mut probe = WriteBatch::new();
            probe.put("probe-after-failure", "x");
            db.commit(&probe)
        });
        assert!(
            matches!(r, Err(Error::Poisoned(_))),
            "{ctx}: after a failed commit or sync the handle must be poisoned, got {r:?}"
        );
    }
    let db = Arc::try_unwrap(db).unwrap_or_else(|_| panic!("{ctx}: a writer still holds the handle"));
    if failed {
        return run;
    }
    match db.close() {
        Ok(()) => run.closed = true,
        Err(e) => allowed("close", &e),
    }
    run
}

/// Runs `body(point, ctx)` for every point `1..=total` (or only
/// `SPIKE_POINT`), reporting the point on failure.
fn for_each_point(total: u64, describe: impl Fn(u64) -> String, body: impl Fn(u64, &str)) {
    let only: Option<u64> = std::env::var("SPIKE_POINT").ok().and_then(|s| s.parse().ok());
    for point in 1..=total {
        if only.is_some_and(|p| p != point) {
            continue;
        }
        let ctx = describe(point);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(point, &ctx)));
        if let Err(e) = r {
            eprintln!("\n*** {ctx}; reproduce with SPIKE_POINT={point} (the interleaving of the writers varies)\n");
            std::panic::resume_unwind(e);
        }
    }
}

/// Mutating operations of a complete scenario C run on an undisturbed
/// device, plus a margin: grouping makes the count vary between runs.
fn scenario_c_points<E: Engine>(opts: &Options, sync: Option<&SyncFn<E>>, failures: Failures) -> u64 {
    let dry = scenario_c_disk(0);
    let run = run_scenario_c::<E>(&dry, opts, sync, failures, "dry run");
    assert!(run.closed, "the dry run of scenario C must complete");
    dry.mutating_ops() + 20
}

/// Crash of `kind` at every mutating operation of scenario C, then
/// recovery, checked with [`check_recovered`] against
/// [`ScenarioCRun::floor`]. With [`CrashKind::ProcessKillKeepPending`] the
/// recovered state must also survive a power loss right after recovery
/// (recovery made it durable). Every point ends with one more commit, a
/// clean close and a power loss.
///
/// A power loss in `Buffered` mode is only meaningful with `sync`: what a
/// successful `sync` returned is then the floor.
pub fn every_point_c<E: Engine>(kind: CrashKind, durability: Durability, sync: Option<&SyncFn<E>>) {
    assert!(
        durability == Durability::Sync || kind != CrashKind::PowerLoss || sync.is_some(),
        "without sync, Buffered commits only promise to survive process kills"
    );
    let mut opts = Options::small_for_tests();
    opts.durability = durability;
    let total = scenario_c_points::<E>(&opts, sync, Failures::Crash);
    let with_sync = if sync.is_some() { " with sync" } else { "" };
    let describe =
        |point| format!("scenario C{with_sync}: {kind:?} ({durability:?}) before mutating op #{point} of {total}");
    for_each_point(total, describe, |point, ctx| {
        let disk = scenario_c_disk(point);
        disk.crash_after(point, kind);
        let run = run_scenario_c::<E>(&disk, &opts, sync, Failures::Crash, ctx);
        let image = match disk.take_crash_image() {
            Some(image) => image,
            None => disk.crash_now(kind),
        };
        let floor = run.floor(durability, kind == CrashKind::PowerLoss);
        let db: E = open(&image.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
        let snap = db.snapshot().unwrap();
        let mut rng = Rng::new(point);
        let rec = check_recovered(&snap, &Model::new(), 0, &run.logs, floor, &mut rng, ctx);
        drop(snap);
        let (db, image) = if kind == CrashKind::ProcessKillKeepPending {
            let after = image.crash_now(CrashKind::PowerLoss);
            drop(db);
            let db: E = open(&after.as_vfs(), &opts)
                .unwrap_or_else(|e| panic!("{ctx}: reopen after the power loss that followed recovery: {e}"));
            let snap = db.snapshot().unwrap();
            assert_eq!(snap.seq(), rec.seq, "{ctx}: recovery did not make the state it recovered durable");
            assert_snapshot_matches(
                &snap,
                &rec.model,
                &mut rng,
                &format!("{ctx}, then power loss right after recovery"),
            );
            drop(snap);
            (db, after)
        } else {
            (db, image)
        };
        keeps_working(db, &image, &opts, rec.model, rec.seq, &mut rng, ctx);
    });
}

/// One injected I/O error at every mutating operation of scenario C, with
/// failed syncs behaving as `fsync`, followed by each of `afters` (see
/// [`failure_sweep`](super::faults::failure_sweep)): the writers' errors are
/// `Io` or `Poisoned` and leave the handle poisoned, and the recovered state
/// passes [`check_recovered`] against [`ScenarioCRun::floor`] (after a
/// restart, every acknowledged commit; after a power loss in `Buffered`
/// mode, what a successful `sync` returned).
pub fn failure_sweep_c<E: Engine>(
    durability: Durability,
    fsync: FsyncFailure,
    afters: &[After],
    sync: Option<&SyncFn<E>>,
) {
    let mut opts = Options::small_for_tests();
    opts.durability = durability;
    let total = scenario_c_points::<E>(&opts, sync, Failures::Fault);
    let with_sync = if sync.is_some() { " with sync" } else { "" };
    for &after in afters {
        let describe = |point| {
            format!(
                "scenario C{with_sync}: I/O error at mutating op #{point} of {total} ({durability:?}, failed fsync \
                 {fsync:?}), then {after:?}"
            )
        };
        for_each_point(total, describe, |point, ctx| {
            let disk = scenario_c_disk(point);
            if fsync != FsyncFailure::KeepPending {
                disk.set_fsync_failure(fsync);
            }
            let then = match after {
                After::Restart => None,
                After::PowerLoss => Some(CrashKind::PowerLoss),
            };
            disk.fail_after(point, then);
            let run = run_scenario_c::<E>(&disk, &opts, sync, Failures::Fault, ctx);
            let disk = match after {
                After::Restart => {
                    // A fault point past the end must not fire during
                    // verification.
                    disk.disarm();
                    disk
                }
                After::PowerLoss => match disk.take_crash_image() {
                    Some(image) => image,
                    None => disk.crash_now(CrashKind::PowerLoss),
                },
            };
            let floor = run.floor(durability, after == After::PowerLoss);
            let db: E = open(&disk.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
            let snap = db.snapshot().unwrap();
            let mut rng = Rng::new(point);
            let rec = check_recovered(&snap, &Model::new(), 0, &run.logs, floor, &mut rng, ctx);
            drop(snap);
            keeps_working(db, &disk, &opts, rec.model, rec.seq, &mut rng, ctx);
        });
    }
}

#[cfg(test)]
mod tests {
    use kerno_core::Op;

    use super::*;

    /// crc32c over every step and every operation of a scenario.
    fn digest<E>(steps: &[Step<E>]) -> u32 {
        let mut crc = 0u32;
        for step in steps {
            let (tag, batch) = match step {
                Step::Open => (0, None),
                Step::Commit(b) => (1, Some(b)),
                Step::Close => (2, None),
                Step::Drop => (3, None),
                Step::Ext(..) => (4, None),
            };
            crc = crc32c::crc32c_append(crc, &[tag]);
            for op in batch.map(|b| b.ops()).unwrap_or(&[]) {
                match op {
                    Op::Put { key, value } => {
                        crc = crc32c::crc32c_append(crc, &[1]);
                        crc = crc32c::crc32c_append(crc, &(key.len() as u64).to_le_bytes());
                        crc = crc32c::crc32c_append(crc, key);
                        crc = crc32c::crc32c_append(crc, &(value.len() as u64).to_le_bytes());
                        crc = crc32c::crc32c_append(crc, value);
                    }
                    Op::Delete { key } => {
                        crc = crc32c::crc32c_append(crc, &[2]);
                        crc = crc32c::crc32c_append(crc, &(key.len() as u64).to_le_bytes());
                        crc = crc32c::crc32c_append(crc, key);
                    }
                    Op::DeleteRange { start, end } => {
                        crc = crc32c::crc32c_append(crc, &[3]);
                        for bound in [start, end] {
                            crc = crc32c::crc32c_append(crc, &(bound.len() as u64).to_le_bytes());
                            crc = crc32c::crc32c_append(crc, bound);
                        }
                    }
                }
            }
        }
        crc
    }

    /// Scenario A is what every `every_point` sweep and its point numbers
    /// rest on. The digest was computed with the same function from the code
    /// before `Step` became generic (`cd78fbc`).
    #[test]
    fn scenario_a_is_unchanged() {
        let steps = scenario::<()>(7);
        assert_eq!(steps.len(), 61);
        assert_eq!(commits(&steps).len(), 55);
        assert_eq!(digest(&steps), 3342322216);
    }

    /// The default configuration is scenario A; with range deletions the
    /// scenario keeps its steps and gains ranges.
    #[test]
    fn scenario_with_config_keeps_scenario_a_and_adds_ranges() {
        let default = scenario_with_config::<()>(7, &SuiteConfig::default());
        assert_eq!(digest(&default), 3342322216);
        let ranged = scenario_with_config::<()>(7, &SuiteConfig::with_range_deletes(100));
        assert_eq!(ranged.len(), 61);
        let batches = commits(&ranged);
        assert_eq!(batches.len(), 55);
        let ranges = batches.iter().flat_map(|b| b.ops()).filter(|op| matches!(op, Op::DeleteRange { .. })).count();
        assert!(ranges >= 10, "only {ranges} range deletions");
        assert!(batches.iter().all(|b| b.validate().is_ok()));
    }

    #[test]
    fn ext_steps_run_on_the_open_handle() {
        use std::sync::atomic::{AtomicU64, Ordering};

        use crate::reference::LogEngine;

        let seen = Arc::new(AtomicU64::new(0));
        let probe = seen.clone();
        let ext: ExtFn<LogEngine> = Arc::new(move |db: &LogEngine| {
            probe.store(db.snapshot()?.seq(), Ordering::SeqCst);
            Ok(())
        });
        let mut steps = scenario::<LogEngine>(7);
        // Right after the first 25 commits, before the clean close.
        steps.insert(26, Step::Ext("observe", ext.clone()));
        assert_eq!(format!("{:?}", steps[26].clone()), "Ext(observe)");
        let disk = SimVfs::new(1);
        let (acked, attempted) = run_scenario::<LogEngine>(&disk, &steps, &Options::small_for_tests());
        assert_eq!((acked, attempted), (55, 55));
        assert_eq!(seen.load(Ordering::SeqCst), 25);
        // A failing extension step stops the run like any other failure.
        let failing: ExtFn<LogEngine> = Arc::new(|_| Err(kerno_core::Error::Io(std::io::Error::other("boom"))));
        steps[26] = Step::Ext("fail", failing);
        let disk = SimVfs::new(1);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_scenario::<LogEngine>(&disk, &steps, &Options::small_for_tests())
        }));
        assert!(r.is_err(), "an extension error without a crash is a test failure");
    }
}
