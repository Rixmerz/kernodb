//! Extended failure suite: the failure models the M0 harness did not cover,
//! swept over every mutating operation of scenario A (see
//! [`extended_failure_suite!`](crate::extended_failure_suite)).
//!
//! | Test | Failure model | Requirement after recovery |
//! |---|---|---|
//! | `power_loss_after_kill_keep_pending_{sync,buffered}` | process kill that keeps unsynced writes pending, then an immediate power loss | `acked <= s <= attempted` after the kill; exactly `s` again after the power loss (recovery made its state durable) |
//! | `linux_fsync_drop_faults_sync` | one I/O error; a failed `fsync` drops its writes (Linux) | restart and power loss: `acked <= s <= attempted` |
//! | `linux_fsync_drop_faults_buffered_restart` | the same in `Buffered` mode | restart: `acked <= s <= attempted` |
//! | `single_io_failure_at_every_point_buffered` | one I/O error in `Buffered` mode | restart: `acked <= s`; power loss (if the engine keeps a prefix): `opened <= s` |
//! | `per_dir_durability_power_loss_sync` | power loss where `sync_dir` only persists its own directory | `acked <= s <= attempted` |
//! | `enospc_at_every_point` | the disk becomes full at that operation | errors are `Io` or `Poisoned`; after space returns, `acked <= s <= attempted` |
//!
//! Every test ends like the M0 sweeps: one more commit, a clean close, a
//! power loss, and a full model check.

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{Durability, Error, WriteBatch};

use super::crash::{Step, commits, every_point_with, run_scenario, scenario};
use super::faults::{After, failure_sweep};
use super::{Model, apply, assert_snapshot_matches, open};
use crate::rng::Rng;
use crate::vfs::{CrashKind, DirDurability, FsyncFailure, SimVfs};

/// The model after the first `s` commits.
fn model_at(batches: &[&WriteBatch], s: u64) -> Model {
    let mut model = Model::new();
    for b in &batches[..s as usize] {
        apply(&mut model, b);
    }
    model
}

fn options(durability: Durability) -> Options {
    let mut opts = Options::small_for_tests();
    opts.durability = durability;
    opts
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
            eprintln!("\n*** {ctx}; reproduce with SPIKE_POINT={point}\n");
            std::panic::resume_unwind(e);
        }
    }
}

/// The device every sweep point starts from: seeded by the point, with
/// byte-granular tearing on odd points.
fn sweep_disk(point: u64) -> SimVfs {
    let disk = SimVfs::new(point);
    if point % 2 == 1 {
        disk.set_tear_unit(1);
    }
    disk
}

/// One more commit, a clean close and a power loss: everything must survive.
fn commit_close_power_loss<E: Engine>(
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
    assert_eq!(db.commit(&b).unwrap(), s + 1, "{ctx}: commit after recovery");
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

/// Crash with [`CrashKind::ProcessKillKeepPending`] at every point of
/// scenario A, recover (`acked <= s <= attempted`), then lose power right
/// away, without any further commit: the state is exactly `s` again, because
/// recovery must have made what it recovered durable.
pub fn power_loss_after_kill_keep_pending<E: Engine>(durability: Durability) {
    let opts = options(durability);
    let steps = scenario::<E>(7);
    let batches = commits(&steps);
    let dry = SimVfs::new(0);
    let (acked, _) = run_scenario::<E>(&dry, &steps, &opts);
    assert_eq!(acked as usize, batches.len(), "dry run must complete");
    let total = dry.mutating_ops() + 20;
    let describe = |point| format!("ProcessKillKeepPending ({durability:?}) before mutating op #{point} of {total}");
    for_each_point(total, describe, |point, ctx| {
        let disk = sweep_disk(point);
        disk.crash_after(point, CrashKind::ProcessKillKeepPending);
        let (acked, attempted) = run_scenario::<E>(&disk, &steps, &opts);
        let image = match disk.take_crash_image() {
            Some(image) => image,
            None => disk.crash_now(CrashKind::ProcessKillKeepPending),
        };
        let db: E = open(&image.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
        let snap = db.snapshot().unwrap();
        let s = snap.seq();
        assert!(acked <= s && s <= attempted, "{ctx}: recovered seq {s} outside {acked}..={attempted}");
        let model = model_at(&batches, s);
        let mut rng = Rng::new(point);
        assert_snapshot_matches(&snap, &model, &mut rng, ctx);
        drop(snap);
        let after = image.crash_now(CrashKind::PowerLoss);
        drop(db);
        let db: E = open(&after.as_vfs(), &opts)
            .unwrap_or_else(|e| panic!("{ctx}: reopen after the power loss that followed recovery: {e}"));
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), s, "{ctx}: recovery did not make the state it recovered durable");
        assert_snapshot_matches(&snap, &model, &mut rng, &format!("{ctx}, then power loss right after recovery"));
        drop(snap);
        commit_close_power_loss(db, &after, &opts, model, s, &mut rng, ctx);
    });
}

/// One I/O error at every point in `Sync` mode, with Linux `fsync` failure
/// semantics; restart and power loss.
pub fn linux_fsync_drop_faults_sync<E: Engine>() {
    failure_sweep::<E>(Durability::Sync, FsyncFailure::LinuxDropPages, &[After::Restart, After::PowerLoss]);
}

/// One I/O error at every point in `Buffered` mode, with Linux `fsync`
/// failure semantics; restart only (after a failed `fsync`, `Buffered`
/// commits since the last successful sync may be lost to a power loss).
pub fn linux_fsync_drop_faults_buffered_restart<E: Engine>() {
    failure_sweep::<E>(Durability::Buffered, FsyncFailure::LinuxDropPages, &[After::Restart]);
}

/// One I/O error at every point in `Buffered` mode; restart, and power loss
/// if the engine keeps a consistent prefix of buffered commits.
pub fn single_io_failure_at_every_point_buffered<E: Engine>(buffered_power_loss_prefix: bool) {
    let afters: &[After] =
        if buffered_power_loss_prefix { &[After::Restart, After::PowerLoss] } else { &[After::Restart] };
    failure_sweep::<E>(Durability::Buffered, FsyncFailure::KeepPending, afters);
}

/// Power loss at every point in `Sync` mode, where `sync_dir` only persists
/// the entries of the directory synced and a new directory needs its parent
/// synced.
pub fn per_dir_durability_power_loss_sync<E: Engine>() {
    every_point_with::<E>(CrashKind::PowerLoss, Durability::Sync, &|disk| {
        disk.set_dir_durability(DirDurability::PerDir)
    });
}

/// Runs the scenario until the first error, which must be `Io` or
/// `Poisoned`; a failed commit or extension step must leave the handle
/// poisoned. Returns (acked, attempted).
fn run_until_error<E: Engine>(disk: &SimVfs, steps: &[Step<E>], opts: &Options, ctx: &str) -> (u64, u64) {
    let vfs = disk.as_vfs();
    let mut db: Option<E> = None;
    let (mut acked, mut attempted) = (0u64, 0u64);
    let check = |what: &str, e: &Error| {
        assert!(
            matches!(e, Error::Io(_) | Error::Poisoned(_)),
            "{ctx}: {what} failed on a full disk with an unexpected error kind: {e}"
        );
    };
    let check_poisoned = |db: &E| {
        let mut probe = WriteBatch::new();
        probe.put("probe-after-failure", "x");
        let r = db.commit(&probe);
        assert!(matches!(r, Err(Error::Poisoned(_))), "{ctx}: after ENOSPC the handle must be poisoned, got {r:?}");
    };
    for step in steps {
        match step {
            Step::Open => match open::<E>(&vfs, opts) {
                Ok(d) => db = Some(d),
                Err(e) => {
                    check("open", &e);
                    break;
                }
            },
            Step::Commit(b) => {
                attempted += 1;
                let handle = db.as_ref().unwrap();
                match handle.commit(b) {
                    Ok(seq) => {
                        assert_eq!(seq, attempted, "{ctx}: unexpected commit sequence");
                        acked = seq;
                    }
                    Err(e) => {
                        check("commit", &e);
                        check_poisoned(handle);
                        break;
                    }
                }
            }
            Step::Close => {
                if let Err(e) = db.take().unwrap().close() {
                    check("close", &e);
                    break;
                }
            }
            Step::Drop => drop(db.take()),
            Step::Ext(name, f) => {
                let handle = db.as_ref().unwrap_or_else(|| panic!("{ctx}: step {name} needs an open handle"));
                if let Err(e) = f(handle) {
                    check(name, &e);
                    check_poisoned(handle);
                    break;
                }
            }
        }
    }
    drop(db);
    (acked, attempted)
}

/// The disk becomes full at every point of scenario A (`Sync` mode). Every
/// error must be `Io` or `Poisoned`. Then space returns, the process
/// restarts on the same device, and the state must be some `s` with
/// `acked <= s <= attempted`, and keep working.
pub fn enospc_at_every_point<E: Engine>() {
    let opts = options(Durability::Sync);
    let steps = scenario::<E>(7);
    let batches = commits(&steps);
    let dry = SimVfs::new(0);
    let (acked, _) = run_until_error::<E>(&dry, &steps, &opts, "dry run");
    assert_eq!(acked as usize, batches.len(), "dry run must complete");
    let total = dry.mutating_ops() + 20;
    let describe = |point| format!("disk full from mutating op #{point} of {total}");
    for_each_point(total, describe, |point, ctx| {
        let disk = sweep_disk(point);
        disk.fill_disk_after(point);
        let (acked, attempted) = run_until_error::<E>(&disk, &steps, &opts, ctx);
        // Space returns (and a fill point past the end never fires).
        disk.set_space_limit(None);
        let db: E = open(&disk.as_vfs(), &opts).unwrap_or_else(|e| panic!("{ctx}: recovery failed: {e}"));
        let snap = db.snapshot().unwrap();
        let s = snap.seq();
        assert!(acked <= s && s <= attempted, "{ctx}: recovered seq {s} outside {acked}..={attempted}");
        let model = model_at(&batches, s);
        let mut rng = Rng::new(point);
        assert_snapshot_matches(&snap, &model, &mut rng, ctx);
        drop(snap);
        commit_close_power_loss(db, &disk, &opts, model, s, &mut rng, ctx);
    });
}

/// Generates the extended failure suite (module `shared_ext_failure`) for an
/// engine type:
///
/// ```ignore
/// kerno_testkit::extended_failure_suite!(MyEngine, buffered_power_loss_prefix = true);
/// ```
///
/// `buffered_power_loss_prefix` has the meaning it has in
/// [`engine_test_suite!`](crate::engine_test_suite): whether a power loss in
/// `Buffered` mode leaves a consistent prefix of commits. See the
/// [module documentation](crate::harness::ext) for what each test checks.
#[macro_export]
macro_rules! extended_failure_suite {
    ($engine:ty, buffered_power_loss_prefix = $bpl:expr) => {
        mod shared_ext_failure {
            use super::*;
            use $crate::Durability;
            use $crate::harness::ext as x;
            #[test]
            fn power_loss_after_kill_keep_pending_sync() {
                x::power_loss_after_kill_keep_pending::<$engine>(Durability::Sync);
            }
            #[test]
            fn power_loss_after_kill_keep_pending_buffered() {
                x::power_loss_after_kill_keep_pending::<$engine>(Durability::Buffered);
            }
            #[test]
            fn linux_fsync_drop_faults_sync() {
                x::linux_fsync_drop_faults_sync::<$engine>();
            }
            #[test]
            fn linux_fsync_drop_faults_buffered_restart() {
                x::linux_fsync_drop_faults_buffered_restart::<$engine>();
            }
            #[test]
            fn single_io_failure_at_every_point_buffered() {
                x::single_io_failure_at_every_point_buffered::<$engine>($bpl);
            }
            #[test]
            fn per_dir_durability_power_loss_sync() {
                x::per_dir_durability_power_loss_sync::<$engine>();
            }
            #[test]
            fn enospc_at_every_point() {
                x::enospc_at_every_point::<$engine>();
            }
        }
    };
}
