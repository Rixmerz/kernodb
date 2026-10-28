//! The MANIFEST as an edit log under crashes and I/O errors: scenario B (a
//! clean close, a dropped handle, a forced checkpoint) is swept over every
//! operation that touches `MANIFEST` or `MANIFEST.tmp`, and over every
//! operation of a checkpoint, in every crash model and with an error in each
//! of them (v1.md W-3, W-7, W-10; phase1-design.md §6.4).

use std::sync::Arc;

use kerno_core::Durability;
use kerno_storage::testing::{Inline, Profiled};
use kerno_testkit::harness::crash::{OpFilter, Step, count_matching, every_matching_point, scenario_b};
use kerno_testkit::harness::faults::{After, failure_sweep_matching};
use kerno_testkit::harness::matrix::{FileClass, file_class};
use kerno_testkit::vfs::{CrashKind, FsyncFailure};

type Db = Profiled<Inline>;

fn steps() -> Vec<Step<Db>> {
    scenario_b::<Db>(7, Arc::new(|db: &Db| db.hooks().checkpoint()))
}

/// Every operation on the MANIFEST: appends, and the files of a checkpoint.
fn any_manifest_op() -> OpFilter {
    Arc::new(|op| matches!(file_class(op), FileClass::Manifest | FileClass::ManifestTmp))
}

/// Every operation of a checkpoint (W-10), the directory sync included.
fn checkpoint_op() -> OpFilter {
    Arc::new(|op| op.phase == "checkpoint")
}

fn opts(durability: Durability) -> kerno_core::contract::Options {
    let mut o = kerno_core::contract::Options::small_for_tests();
    o.durability = durability;
    o
}

/// Crash models and durability modes that make sense together: a power loss
/// only promises what was synced (`Sync`); a process kill promises what was
/// handed to the OS (`Buffered` too).
const CRASHES: [(CrashKind, Durability); 5] = [
    (CrashKind::PowerLoss, Durability::Sync),
    (CrashKind::ProcessKill, Durability::Sync),
    (CrashKind::ProcessKill, Durability::Buffered),
    (CrashKind::ProcessKillKeepPending, Durability::Sync),
    (CrashKind::ProcessKillKeepPending, Durability::Buffered),
];

#[test]
fn manifest_append_crash_at_every_point() {
    let steps = steps();
    let n = count_matching::<Db>(&steps, &opts(Durability::Sync), &|_| {}, &any_manifest_op());
    assert!(n > 40, "scenario B touches the MANIFEST {n} times");
    for (kind, durability) in CRASHES {
        every_matching_point::<Db>(kind, durability, &steps, any_manifest_op());
    }
}

#[test]
fn checkpoint_crash_at_every_point() {
    let steps = steps();
    let n = count_matching::<Db>(&steps, &opts(Durability::Sync), &|_| {}, &checkpoint_op());
    // The forced checkpoint and the two CLEAN ones of the closes: each is a
    // temporary file, its bytes, a sync, a rename and a directory sync.
    assert!(n >= 15, "scenario B checkpoints {n} operations' worth");
    for (kind, durability) in CRASHES {
        every_matching_point::<Db>(kind, durability, &steps, checkpoint_op());
    }
}

#[test]
fn manifest_eio_at_every_point() {
    let steps = steps();
    let both = [After::Restart, After::PowerLoss];
    for (durability, fsync, afters) in [
        (Durability::Sync, FsyncFailure::KeepPending, &both[..]),
        (Durability::Sync, FsyncFailure::LinuxDropPages, &both[..]),
        (Durability::Buffered, FsyncFailure::KeepPending, &both[..]),
        (Durability::Buffered, FsyncFailure::LinuxDropPages, &both[..1]),
    ] {
        failure_sweep_matching::<Db>(durability, fsync, afters, &steps, any_manifest_op());
    }
}
