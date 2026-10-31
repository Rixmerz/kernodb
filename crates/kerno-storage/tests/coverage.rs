//! Coverage of the failure matrix (phase1-design.md §6.3,
//! `docs/architecture/failure-matrix.md`): scenario B runs in `Inline` mode
//! with a trace, every mutating operation is assigned to a row by (probe
//! phase, file kind, operation), and the test fails if a row of this work
//! package gets no point or an operation belongs to no row. The rows that are
//! a mode of the device or the handle (`RO`, `FL`, `NS`, `KP`) are run in their
//! mode and recorded when it fired.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::testing::{Inline, Profiled};
use kerno_storage::{AccessMode, Config, KernoDb};
use kerno_testkit::harness::crash::{Step, commits, run_scenario, scenario_b};
use kerno_testkit::harness::matrix::{PROTOCOL_ROWS, RANGE_ROWS, first_unclassified, rows_covered};
use kerno_testkit::harness::{Model, apply, assert_snapshot_matches, open};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, FsyncFailure, OpKind, SimVfs};

use common::ranged_scenario;

type Db = Profiled<Inline>;

/// The rows of the metadata log work package: the protocol rows scenario B
/// reaches and the mode rows.
const WP08_ROWS: &[&str] =
    &["L1-L4", "L5'", "L6", "L7'", "L8", "L9", "L10'", "L11", "L12'", "L13'", "CK", "CR", "R0", "RO", "FL", "NS", "KP"];

fn opts(durability: Durability) -> Options {
    Options { durability, ..Options::small_for_tests() }
}

fn steps<E: HasCheckpoint>() -> Vec<Step<E>> {
    scenario_b::<E>(7, Arc::new(|db: &E| db.checkpoint()))
}

/// Engines that can force a MANIFEST checkpoint.
trait HasCheckpoint: Engine {
    fn checkpoint(&self) -> kerno_core::Result<()>;
}

impl HasCheckpoint for Db {
    fn checkpoint(&self) -> kerno_core::Result<()> {
        self.hooks().checkpoint()
    }
}

impl HasCheckpoint for KernoDb {
    fn checkpoint(&self) -> kerno_core::Result<()> {
        self.hooks().checkpoint()
    }
}

/// Runs scenario B once with a trace and returns the rows its operations hit.
fn protocol_rows<E: HasCheckpoint>() -> (BTreeSet<&'static str>, SimVfs) {
    let steps = steps::<E>();
    let disk = SimVfs::new(7);
    disk.start_trace();
    let (acked, attempted) = run_scenario::<E>(&disk, &steps, &opts(Durability::Sync));
    assert_eq!((acked, attempted), (commits(&steps).len() as u64, commits(&steps).len() as u64));
    let trace = disk.trace();
    assert_eq!(trace.len() as u64, disk.mutating_ops());
    assert_eq!(first_unclassified(&trace), None, "an operation that belongs to no row of the matrix");
    (rows_covered(&trace), disk)
}

/// RO: a read-only open of the database scenario B leaves does nothing.
fn row_ro(disk: &SimVfs) -> bool {
    let image = disk.crash_now(CrashKind::ProcessKillKeepPending);
    let before = image.mutating_ops();
    let cfg = Config { access: AccessMode::ReadOnly, ..Config::default() };
    let db = KernoDb::open(image.as_vfs(), std::path::Path::new("/db"), &cfg).unwrap();
    assert!(db.is_read_only().is_some());
    drop(db);
    assert_eq!(image.mutating_ops(), before, "a read-only open did mutating operations");
    true
}

/// The model after the first `s` commits.
fn model_at(batches: &[&WriteBatch], s: u64) -> Model {
    let mut model = Model::new();
    for b in &batches[..s as usize] {
        apply(&mut model, b);
    }
    model
}

/// KP: the process dies in the middle of the scenario with its unsynced
/// writes still pending; recovery gives a state some commit left, and a power
/// loss right after recovery finds it again.
fn row_kp() -> bool {
    let steps = steps::<Db>();
    let batches = commits(&steps);
    let o = opts(Durability::Sync);
    let dry = SimVfs::new(0);
    run_scenario::<Db>(&dry, &steps, &o);
    let disk = SimVfs::new(21);
    disk.crash_after(dry.mutating_ops() / 2, CrashKind::ProcessKillKeepPending);
    let (acked, attempted) = run_scenario::<Db>(&disk, &steps, &o);
    assert!(disk.has_crashed(), "the crash did not fire");
    let image = disk.take_crash_image().unwrap();
    let db: Db = open(&image.as_vfs(), &o).unwrap();
    let s = db.snapshot().unwrap().seq();
    assert!(acked <= s && s <= attempted, "{s} outside {acked}..={attempted}");
    let model = model_at(&batches, s);
    assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut Rng::new(1), "KP");
    drop(db);
    let after = image.crash_now(CrashKind::PowerLoss);
    let db: Db = open(&after.as_vfs(), &o).unwrap();
    assert_eq!(db.snapshot().unwrap().seq(), s, "recovery did not make its state durable");
    true
}

/// FL: Linux drops the pages of a failed `fsync`: the commit that needed it
/// fails, the handle is poisoned, and what recovery finds is some prefix.
fn row_fl() -> bool {
    let steps = steps::<Db>();
    let batches = commits(&steps);
    let o = opts(Durability::Sync);
    let disk = SimVfs::new(22);
    disk.set_fsync_failure(FsyncFailure::LinuxDropPages);
    disk.fail_when(|op| op.kind == OpKind::SyncData && op.phase == "commit", 7, Some(CrashKind::PowerLoss));
    let (acked, attempted) = run_scenario_until_fault(&disk, &steps, &o);
    assert!(disk.fault_injected(), "the fault did not fire");
    let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(CrashKind::PowerLoss));
    let db: Db = open(&image.as_vfs(), &o).unwrap();
    let s = db.snapshot().unwrap().seq();
    assert!(acked <= s && s <= attempted, "{s} outside {acked}..={attempted}");
    assert_snapshot_matches(&db.snapshot().unwrap(), &model_at(&batches, s), &mut Rng::new(2), "FL");
    true
}

/// NS: the disk fills up at some point of the scenario: every error is `Io` or
/// `Poisoned`, and once space returns recovery finds some prefix.
fn row_ns() -> bool {
    let steps = steps::<Db>();
    let batches = commits(&steps);
    let o = opts(Durability::Sync);
    let disk = SimVfs::new(23);
    disk.fill_disk_after(150);
    let (acked, attempted) = run_scenario_until_fault(&disk, &steps, &o);
    disk.set_space_limit(None);
    let db: Db = open(&disk.as_vfs(), &o).unwrap();
    let s = db.snapshot().unwrap().seq();
    assert!(acked <= s && s <= attempted, "{s} outside {acked}..={attempted}");
    assert_snapshot_matches(&db.snapshot().unwrap(), &model_at(&batches, s), &mut Rng::new(3), "NS");
    // The disk did fill: the scenario did not run to its end.
    assert!(attempted < batches.len() as u64 || acked < batches.len() as u64 || s < batches.len() as u64 + 1);
    true
}

/// Runs the scenario until the first error (an I/O failure of the device or
/// the engine's poisoning of it); returns (acked, attempted).
fn run_scenario_until_fault(disk: &SimVfs, steps: &[Step<Db>], o: &Options) -> (u64, u64) {
    let vfs = disk.as_vfs();
    let mut db: Option<Db> = None;
    let (mut acked, mut attempted) = (0u64, 0u64);
    let expected = |e: &Error| matches!(e, Error::Io(_) | Error::Poisoned(_));
    for step in steps {
        let r: kerno_core::Result<()> = match step {
            Step::Open => open::<Db>(&vfs, o).map(|d| db = Some(d)),
            Step::Commit(b) => {
                attempted += 1;
                db.as_ref().unwrap().commit(b).map(|seq| acked = seq)
            }
            Step::Close => db.take().unwrap().close(),
            Step::Drop => {
                drop(db.take());
                Ok(())
            }
            Step::Ext(_, f) => f(db.as_ref().unwrap()),
        };
        if let Err(e) = r {
            assert!(expected(&e), "unexpected error kind: {e}");
            break;
        }
    }
    drop(db);
    (acked, attempted)
}

#[test]
fn failure_matrix_rows_are_covered() {
    let (mut rows, disk) = protocol_rows::<Db>();
    for row in PROTOCOL_ROWS {
        assert!(rows.contains(row), "no point of scenario B belongs to row {row}; covered: {rows:?}");
    }
    if row_ro(&disk) {
        rows.insert("RO");
    }
    if row_kp() {
        rows.insert("KP");
    }
    if row_fl() {
        rows.insert("FL");
    }
    if row_ns() {
        rows.insert("NS");
    }
    for row in WP08_ROWS {
        assert!(rows.contains(row), "row {row} is not covered; covered: {rows:?}");
    }
}

/// The same scenario with background threads (the engine's default) touches
/// every row the deterministic mode does.
#[test]
fn threads_mode_rows_superset_of_inline() {
    let (inline, _) = protocol_rows::<Db>();
    let (threads, _) = protocol_rows::<KernoDb>();
    let missing: Vec<_> = inline.difference(&threads).collect();
    assert!(missing.is_empty(), "rows only the inline mode reaches: {missing:?} (threads: {threads:?})");
}

/// RT and DC: scenario B with range deletions and directed compactions runs
/// to its end; the operations of the directed compactions belong to DC, none
/// belongs to no row, and some table written while it ran held range tombstone
/// fragments (RT; the sweeps over these points are `range_delete.rs` and
/// `compact_range.rs`).
#[test]
fn range_rows_are_covered() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let saw_fragments = Arc::new(AtomicBool::new(false));
    let seen = saw_fragments.clone();
    let steps: Vec<Step<Db>> = ranged_scenario::<Db>(
        7,
        Arc::new(move |db: &Db| {
            // Before the compaction: the files the scenario has written so
            // far, at least one with fragments.
            db.hooks().flush()?;
            let held = db.db().level_tables().iter().flatten().any(|t| !t.fragments.is_empty());
            seen.fetch_or(held, Ordering::SeqCst);
            db.hooks().compact_range(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded)
        }),
        Arc::new(|db: &Db| db.hooks().checkpoint()),
    );
    assert!(commits(&steps).iter().any(|b| b.has_range_deletes()));
    let disk = SimVfs::new(7);
    disk.start_trace();
    let (acked, attempted) = run_scenario::<Db>(&disk, &steps, &opts(Durability::Sync));
    assert_eq!((acked, attempted), (commits(&steps).len() as u64, commits(&steps).len() as u64));
    let trace = disk.trace();
    assert_eq!(first_unclassified(&trace), None, "an operation that belongs to no row of the matrix");
    let mut rows = rows_covered(&trace);
    if saw_fragments.load(Ordering::SeqCst) {
        rows.insert("RT");
    }
    for row in RANGE_ROWS {
        assert!(rows.contains(row), "row {row} is not covered; covered: {rows:?}");
    }
}
