//! Single I/O failure with the device still alive (invariant I-10).
//!
//! For every mutating I/O operation of the fixed scenario used by
//! [`crash::every_point`](super::crash::every_point), that one operation
//! fails with an I/O error while everything else keeps working. The engine
//! must:
//!
//! * report the failure from the call that depends on it (or from a later
//!   call, if it happened in background work), and never acknowledge a
//!   commit whose data could not be made durable;
//! * refuse further commits on that handle (poisoned) until reopened;
//! * recover to the state of some commit `s` with `acked <= s <= attempted`
//!   both when the process restarts on the same, still-working device and
//!   when the machine loses power right after the failure (before any
//!   later sync could make up for the failed one — this is what catches an
//!   engine that ignores a failed `fsync`);
//! * keep working (and stay durable) after that recovery.
//!
//! Failures of operations whose effect the engine can safely ignore (e.g.
//! deleting an obsolete file) may go unreported, as long as every
//! guarantee above still holds.
//!
//! [`failure_sweep`] runs the same sweep in `Buffered` mode and under
//! Linux `fsync` semantics ([`FsyncFailure::LinuxDropPages`]: the writes of a
//! failed sync are gone for good, so recovery must not trust what it can
//! still read but never persisted). In `Buffered` mode a power loss may lose
//! acknowledged commits, but never the state the last successful open
//! recovered (which recovery must have made durable).

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{Durability, Error, WriteBatch};

use super::crash::{OpFilter, Step, commits, scenario, scenario_with_config};
use super::{Model, SuiteConfig, apply, assert_snapshot_matches, open};
use crate::rng::Rng;
use crate::vfs::{CrashKind, FsyncFailure, SimVfs};

/// What happens right after the injected failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    /// The process restarts; the device and its caches are intact.
    Restart,
    /// The machine loses power right after the failure was observed.
    PowerLoss,
}

/// Commit counts of one faulty run.
struct Outcome {
    acked: u64,
    attempted: u64,
    /// `acked` when the last successful open returned.
    opened: u64,
}

/// Runs the scenario with one injected fault.
fn run_with_fault<E: Engine>(disk: &SimVfs, steps: &[Step<E>], opts: &Options, ctx: &str) -> Outcome {
    let vfs = disk.as_vfs();
    let mut db: Option<E> = None;
    let mut out = Outcome { acked: 0, attempted: 0, opened: 0 };
    // A failed commit or extension step must leave the handle poisoned.
    let check_poisoned = |db: &E, what: &str, e: &Error| {
        assert!(disk.fault_injected(), "{ctx}: {what} failed without a fault: {e}");
        if disk.has_crashed() {
            return;
        }
        let mut probe = WriteBatch::new();
        probe.put("probe-after-failure", "x");
        let r = db.commit(&probe);
        assert!(
            matches!(r, Err(Error::Poisoned(_))),
            "{ctx}: after an I/O failure the handle must be poisoned, got {r:?}"
        );
    };
    for step in steps {
        match step {
            Step::Open => match open::<E>(&vfs, opts) {
                Ok(d) => {
                    db = Some(d);
                    out.opened = out.acked;
                }
                Err(e) => {
                    assert!(disk.fault_injected(), "{ctx}: open failed without a fault: {e}");
                    if disk.has_crashed() {
                        return out;
                    }
                    // The fault is one-shot: opening again must work (unless
                    // the power-loss variant pulls the plug meanwhile).
                    match open::<E>(&vfs, opts) {
                        Ok(d) => {
                            db = Some(d);
                            out.opened = out.acked;
                        }
                        Err(_) if disk.has_crashed() => return out,
                        Err(e) => panic!("{ctx}: second open failed: {e}"),
                    }
                }
            },
            Step::Commit(b) => {
                out.attempted += 1;
                match db.as_ref().unwrap().commit(b) {
                    Ok(seq) => {
                        assert_eq!(seq, out.attempted, "{ctx}: unexpected commit sequence");
                        out.acked = seq;
                    }
                    Err(e) => {
                        check_poisoned(db.as_ref().unwrap(), "commit", &e);
                        return out;
                    }
                }
            }
            Step::Close => {
                if let Err(e) = db.take().unwrap().close() {
                    // (The fault may have hit earlier background work.)
                    assert!(disk.fault_injected(), "{ctx}: close failed without a fault: {e}");
                    return out;
                }
            }
            Step::Drop => drop(db.take()),
            Step::Ext(name, f) => {
                let handle = db.as_ref().unwrap_or_else(|| panic!("{ctx}: step {name} needs an open handle"));
                if let Err(e) = f(handle) {
                    check_poisoned(handle, name, &e);
                    return out;
                }
            }
        }
    }
    out
}

/// The original sweep: `Sync` mode, failed syncs keep their writes pending,
/// then restart and power loss.
pub fn single_failure_at_every_point<E: Engine>() {
    failure_sweep::<E>(Durability::Sync, FsyncFailure::KeepPending, &[After::Restart, After::PowerLoss]);
}

/// [`single_failure_at_every_point`] over the scenario built with the
/// workload of `cfg`.
pub fn single_failure_at_every_point_with_config<E: Engine>(cfg: &SuiteConfig) {
    let steps = scenario_with_config::<E>(7, cfg);
    failure_sweep_steps::<E>(Durability::Sync, FsyncFailure::KeepPending, &[After::Restart, After::PowerLoss], &steps);
}

/// One injected I/O failure at every mutating operation of scenario A, in
/// `durability` mode, with failed syncs behaving as `fsync`, followed by
/// each of `afters`. The recovered sequence `s` must satisfy
/// `lower <= s <= attempted`, where `lower` is the last acknowledged commit,
/// except for a power loss in `Buffered` mode, where it is the state the last
/// successful open recovered.
pub fn failure_sweep<E: Engine>(durability: Durability, fsync: FsyncFailure, afters: &[After]) {
    failure_sweep_steps::<E>(durability, fsync, afters, &scenario::<E>(7));
}

/// [`failure_sweep`] over any scenario `steps`.
pub fn failure_sweep_steps<E: Engine>(
    durability: Durability,
    fsync: FsyncFailure,
    afters: &[After],
    steps: &[Step<E>],
) {
    failure_sweep_filtered::<E>(durability, fsync, afters, steps, None);
}

/// [`failure_sweep_steps`] restricted to the operations `filter` accepts: the
/// error hits each such operation in turn ([`SimVfs::fail_when`]).
pub fn failure_sweep_matching<E: Engine>(
    durability: Durability,
    fsync: FsyncFailure,
    afters: &[After],
    steps: &[Step<E>],
    filter: OpFilter,
) {
    failure_sweep_filtered::<E>(durability, fsync, afters, steps, Some(filter));
}

fn failure_sweep_filtered<E: Engine>(
    durability: Durability,
    fsync: FsyncFailure,
    afters: &[After],
    steps: &[Step<E>],
    filter: Option<OpFilter>,
) {
    let mut opts = Options::small_for_tests();
    opts.durability = durability;
    let batches = commits(steps);
    let dry = SimVfs::new(0);
    dry.start_trace();
    let out = run_with_fault::<E>(&dry, steps, &opts, "dry run");
    assert_eq!(out.acked as usize, batches.len(), "dry run must complete");
    let total = match &filter {
        Some(f) => dry.trace().iter().filter(|t| f(&t.op)).count() as u64 + 3,
        None => dry.mutating_ops() + 20,
    };
    let only: Option<u64> = std::env::var("SPIKE_POINT").ok().and_then(|s| s.parse().ok());
    let mode = match (durability, fsync) {
        (Durability::Sync, FsyncFailure::KeepPending) => String::new(),
        _ => format!(" ({durability:?}, failed fsync {fsync:?})"),
    };
    for &after in afters {
        for point in 1..=total {
            if only.is_some_and(|p| p != point) {
                continue;
            }
            let ctx = format!("I/O error at mutating op #{point} of {total}{mode}, then {after:?}");
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let disk = SimVfs::new(point);
                if point % 2 == 1 {
                    disk.set_tear_unit(1);
                }
                if fsync != FsyncFailure::KeepPending {
                    disk.set_fsync_failure(fsync);
                }
                let then = match after {
                    After::Restart => None,
                    After::PowerLoss => Some(CrashKind::PowerLoss),
                };
                match &filter {
                    Some(f) => {
                        let f = f.clone();
                        disk.fail_when(move |op| f(op), point, then);
                    }
                    None => disk.fail_after(point, then),
                }
                let out = run_with_fault::<E>(&disk, steps, &opts, &ctx);
                let disk = match after {
                    After::Restart => {
                        // A fault point past the end of the scenario must not
                        // fire during verification.
                        disk.disarm();
                        disk
                    }
                    After::PowerLoss => match disk.take_crash_image() {
                        Some(image) => image,
                        None => disk.crash_now(CrashKind::PowerLoss),
                    },
                };
                let lower = match (after, durability) {
                    (After::PowerLoss, Durability::Buffered) => out.opened,
                    _ => out.acked,
                };
                let vfs = disk.as_vfs();
                let db: E = open(&vfs, &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
                let s = db.snapshot().unwrap().seq();
                assert!(
                    lower <= s && s <= out.attempted,
                    "{ctx}: recovered seq {s} outside {lower}..={}",
                    out.attempted
                );
                let mut model = Model::new();
                for b in &batches[..s as usize] {
                    apply(&mut model, b);
                }
                let mut rng = Rng::new(point);
                assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &ctx);
                let mut b = WriteBatch::new();
                b.put("after-recovery", "x");
                assert_eq!(db.commit(&b).unwrap(), s + 1, "{ctx}: commit after recovery");
                apply(&mut model, &b);
                db.close().unwrap();
                // Everything acknowledged since recovery must survive power loss.
                let image = disk.crash_now(CrashKind::PowerLoss);
                let db: E = open(&image.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: final reopen: {e}"));
                assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &format!("{ctx}, after power loss"));
            }));
            if let Err(e) = r {
                eprintln!("\n*** {ctx}; reproduce with SPIKE_POINT={point}\n");
                std::panic::resume_unwind(e);
            }
        }
    }
}
