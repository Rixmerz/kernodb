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

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{Error, WriteBatch};

use super::crash::{Step, scenario};
use super::{Model, apply, assert_snapshot_matches, open};
use crate::rng::Rng;
use crate::vfs::{CrashKind, SimVfs};

#[derive(Clone, Copy, Debug)]
enum After {
    /// The process restarts; the device and its caches are intact.
    Restart,
    /// The machine loses power right after the failure was observed.
    PowerLoss,
}

/// Runs the scenario with one injected fault. Returns (acked, attempted).
fn run_with_fault<E: Engine>(disk: &SimVfs, steps: &[Step], opts: &Options, ctx: &str) -> (u64, u64) {
    let vfs = disk.as_vfs();
    let mut db: Option<E> = None;
    let (mut acked, mut attempted) = (0u64, 0u64);
    for step in steps {
        match step {
            Step::Open => match open::<E>(&vfs, opts) {
                Ok(d) => db = Some(d),
                Err(e) => {
                    assert!(disk.fault_injected(), "{ctx}: open failed without a fault: {e}");
                    if disk.has_crashed() {
                        return (acked, attempted);
                    }
                    // The fault is one-shot: opening again must work (unless
                    // the power-loss variant pulls the plug meanwhile).
                    match open::<E>(&vfs, opts) {
                        Ok(d) => db = Some(d),
                        Err(_) if disk.has_crashed() => return (acked, attempted),
                        Err(e) => panic!("{ctx}: second open failed: {e}"),
                    }
                }
            },
            Step::Commit(b) => {
                attempted += 1;
                match db.as_ref().unwrap().commit(b) {
                    Ok(seq) => {
                        assert_eq!(seq, attempted, "{ctx}: unexpected commit sequence");
                        acked = seq;
                    }
                    Err(e) => {
                        assert!(disk.fault_injected(), "{ctx}: commit failed without a fault: {e}");
                        if disk.has_crashed() {
                            return (acked, attempted);
                        }
                        let mut probe = WriteBatch::new();
                        probe.put("probe-after-failure", "x");
                        let r = db.as_ref().unwrap().commit(&probe);
                        assert!(
                            matches!(r, Err(Error::Poisoned(_))),
                            "{ctx}: after an I/O failure the handle must be poisoned, got {r:?}"
                        );
                        return (acked, attempted);
                    }
                }
            }
            Step::Close => {
                if let Err(e) = db.take().unwrap().close() {
                    // (The fault may have hit earlier background work.)
                    assert!(disk.fault_injected(), "{ctx}: close failed without a fault: {e}");
                    return (acked, attempted);
                }
            }
            Step::Drop => drop(db.take()),
        }
    }
    (acked, attempted)
}

pub fn single_failure_at_every_point<E: Engine>() {
    let opts = Options::small_for_tests();
    let steps = scenario(7);
    let batches: Vec<&WriteBatch> =
        steps.iter().filter_map(|s| if let Step::Commit(b) = s { Some(b) } else { None }).collect();
    let dry = SimVfs::new(0);
    let (acked, _) = run_with_fault::<E>(&dry, &steps, &opts, "dry run");
    assert_eq!(acked as usize, batches.len(), "dry run must complete");
    let total = dry.mutating_ops() + 20;
    let only: Option<u64> = std::env::var("SPIKE_POINT").ok().and_then(|s| s.parse().ok());
    for after in [After::Restart, After::PowerLoss] {
        for point in 1..=total {
            if only.is_some_and(|p| p != point) {
                continue;
            }
            let ctx = format!("I/O error at mutating op #{point} of {total}, then {after:?}");
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let disk = SimVfs::new(point);
                if point % 2 == 1 {
                    disk.set_tear_unit(1);
                }
                let then = match after {
                    After::Restart => None,
                    After::PowerLoss => Some(CrashKind::PowerLoss),
                };
                disk.fail_after(point, then);
                let (acked, attempted) = run_with_fault::<E>(&disk, &steps, &opts, &ctx);
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
                let vfs = disk.as_vfs();
                let db: E = open(&vfs, &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
                let s = db.snapshot().unwrap().seq();
                assert!(acked <= s && s <= attempted, "{ctx}: recovered seq {s} outside {acked}..={attempted}");
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
