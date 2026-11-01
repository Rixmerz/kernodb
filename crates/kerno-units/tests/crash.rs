//! E3-08: crash sweeps of the units layer (etapa3-design.md §8.2), built on
//! `harness::units`. After every recovery the store must be byte for byte
//! the one after a whole prefix of change sets (I-21), `verify_units` must
//! find nothing, and the database must keep working.
//!
//! * `changeset_every_point` — a crash before every mutating I/O operation of
//!   a scenario per seed (change sets, clean close, unclean drop), both crash
//!   kinds and both durability modes.
//! * `model_crash` — random crashes in rounds (also during recovery), each
//!   round continuing on the recovered image.
//! * `recovery_reentrant` — a crash at every mutating operation of the
//!   recovery of a database whose change sets are only in the WAL; the second
//!   recovery is identical (I-11).
//! * `fsync_failure` — every `sync_data` of a scenario fails in turn, with
//!   both failure models: `Io`, then `Poisoned`; after reopening the change
//!   set is whole or absent (I-10).
//! * `enospc` — a full disk at every mutating operation, and a space limit
//!   that leaves room for a WAL header (96 B) but not for the change set.
//! * `torn_write` — power loss tears the WAL frame of a large change set; the
//!   frame is dropped whole.
//! * `wal_independence` — flushes, WAL rotation and recycling never change a
//!   unit's history (I-20).
//!
//! `SPIKE_SEEDS=<n>` sets the seeds of every suite, `SPIKE_SEED=<n>` reruns
//! one, `SPIKE_POINT=<n>` one point of a sweep.

mod harness;

use std::collections::BTreeSet;

use harness::units::*;
use kerno_core::Durability;
use kerno_storage::KernoDb;
use kerno_storage::format::header::HEADER_LEN;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::harness::{for_each_seed, seeds};
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, FsyncFailure, OpKind, SimVfs};
use kerno_units::*;

/// Runs `body(point)` for every point `1..=total` (or only `SPIKE_POINT`),
/// reporting the point on failure.
fn for_each_point(name: &str, total: u64, body: impl Fn(u64)) {
    let only: Option<u64> = std::env::var("SPIKE_POINT").ok().and_then(|s| s.parse().ok());
    for point in 1..=total {
        if only.is_some_and(|p| p != point) {
            continue;
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(point)));
        if let Err(e) = r {
            eprintln!("\n*** {name}: point {point} of {total}; reproduce with SPIKE_POINT={point}\n");
            std::panic::resume_unwind(e);
        }
    }
}

/// A sweep device: seeded by the point, byte-granular tearing on odd points
/// (as `crash::every_point`).
fn sweep_disk(seed: u64, point: u64) -> SimVfs {
    let disk = SimVfs::new(seed.wrapping_mul(1_000_003) ^ point);
    if point % 2 == 1 {
        disk.set_tear_unit(1);
    }
    disk
}

// ------------------------------------------------------------ changeset_every_point

/// 12 change sets with a clean close and an unclean drop in between.
const SCENARIO: [Step; 5] = [Step::ChangeSet(5), Step::Close, Step::ChangeSet(4), Step::Drop, Step::ChangeSet(3)];
const SCENARIO_CHANGE_SETS: u64 = 12;

/// What a scenario run did.
struct Run {
    acked: u64,
    attempted: u64,
    /// Last sequence made durable by a clean close (what a power loss cannot
    /// take back in `Buffered`).
    closed: u64,
}

/// Runs the scenario until the device fails (every error must come from a
/// crash or an injected fault) and closes at the end.
fn run_scenario<E: Target>(disk: &SimVfs, cfg: &UnitConfig, ids: &StepIds, seed: u64) -> Run {
    let mut run = Run { acked: 0, attempted: 0, closed: 0 };
    let Ok(mut db) = open_units::<E>(disk.as_vfs(), cfg) else { return run };
    for step in SCENARIO {
        match step {
            Step::ChangeSet(n) => {
                let (acked, err) = run_steps(&db, ids, seed, run.acked, run.acked + n);
                run.acked = acked;
                run.attempted = acked;
                if err.is_some() {
                    run.attempted = acked + 1;
                    return run;
                }
            }
            Step::Close | Step::Drop => {
                if step == Step::Close {
                    if db.close().is_err() {
                        return run;
                    }
                    run.closed = run.acked;
                } else {
                    drop(db);
                }
                db = match open_units::<E>(disk.as_vfs(), cfg) {
                    Ok(db) => db,
                    Err(_) => return run,
                };
            }
        }
    }
    if db.close().is_ok() {
        run.closed = run.acked;
    }
    run
}

fn changeset_every_point<E: Target>(kind: CrashKind, durability: Durability) {
    let name = format!("changeset_every_point {} {kind:?} {durability:?}", E::NAME);
    for_each_seed(&name, seeds(4), |seed| {
        let ids = StepIds::new();
        let cfg = unit_config(&ids, durability);
        let oracle = std::cell::RefCell::new(Oracle::new(seed));
        let dry = SimVfs::new(0);
        let run = run_scenario::<E>(&dry, &cfg, &ids, seed);
        assert_eq!(run.acked, SCENARIO_CHANGE_SETS, "the dry run must complete");
        let total = dry.mutating_ops() + 5;
        eprintln!("{name} seed {seed}: {total} points");
        for_each_point(&format!("{name} seed {seed}"), total, |point| {
            let ctx = format!("{name} seed {seed}: crash before mutating op #{point} of {total}");
            let oracle = &mut *oracle.borrow_mut();
            let disk = sweep_disk(seed, point);
            disk.crash_after(point, kind);
            let run = run_scenario::<E>(&disk, &cfg, &ids, seed);
            let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(kind));
            let lo =
                if kind == CrashKind::PowerLoss && durability == Durability::Buffered { run.closed } else { run.acked };
            let db = open_units::<E>(image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: recovery: {e}"));
            let s = check_store(&db, oracle, lo, run.attempted, &ctx);
            keeps_working(db, &image, &cfg, &ids, oracle, s, &ctx);
        });
    });
}

// ------------------------------------------------------------ model_crash

const ROUNDS: usize = 5;
const CHANGE_SETS_PER_ROUND: u64 = 20;

fn model_crash<E: Target>(kind: CrashKind, durability: Durability) {
    let name = format!("model_crash {} {kind:?} {durability:?}", E::NAME);
    for_each_seed(&name, seeds(16), |seed| model_crash_seed::<E>(seed, kind, durability));
}

fn model_crash_seed<E: Target>(seed: u64, kind: CrashKind, durability: Durability) {
    let mut rng = Rng::new(seed ^ 0x5EED_CAFE);
    let ids = StepIds::new();
    let cfg = unit_config(&ids, durability);
    let mut oracle = Oracle::new(seed);
    let mut disk = SimVfs::new(seed);
    if seed % 2 == 1 {
        disk.set_tear_unit(1);
    }
    // Bounds of the next recovered sequence, and the last verified one.
    let (mut lo, mut hi, mut base) = (0u64, 0u64, 0u64);
    let mut ops_per_change_set = 10u64;
    for round in 0..ROUNDS {
        let ctx = format!("model_crash {} seed {seed} round {round}", E::NAME);
        if rng.chance(1, 5) {
            // Crash during recovery itself.
            disk.crash_after(rng.range(1, 40), kind);
        }
        let db = match open_units::<E>(disk.as_vfs(), &cfg) {
            Ok(db) => db,
            Err(e) => {
                assert!(disk.has_crashed(), "{ctx}: open failed without a crash: {e}");
                disk = disk.take_crash_image().expect("crash image");
                continue;
            }
        };
        disk.disarm();
        assert!(!disk.has_crashed(), "{ctx}: inline engines do no work after open");
        let s = check_store(&db, &mut oracle, lo, hi, &format!("{ctx}: recovery"));
        assert!(s >= base, "{ctx}: recovered {s}, older than the verified {base}");
        base = s;
        disk.crash_after(rng.range(1, CHANGE_SETS_PER_ROUND * ops_per_change_set), kind);
        let before = disk.mutating_ops();
        let (acked, err) = run_steps(&db, &ids, seed, s, s + CHANGE_SETS_PER_ROUND);
        if let Some(e) = &err {
            assert!(disk.has_crashed(), "{ctx}: change set failed without a crash: {e}");
        }
        if acked > s {
            ops_per_change_set = ((disk.mutating_ops() - before) / (acked - s)).max(1) + 1;
        }
        let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(kind));
        drop(db);
        disk = image;
        lo = if kind == CrashKind::PowerLoss && durability == Durability::Buffered { base } else { acked };
        hi = if err.is_some() { acked + 1 } else { acked };
    }
    let ctx = format!("model_crash {} seed {seed}, final", E::NAME);
    let db = open_units::<E>(disk.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: recovery: {e}"));
    let s = check_store(&db, &mut oracle, lo, hi, &ctx);
    keeps_working(db, &disk, &cfg, &ids, &mut oracle, s, &ctx);
}

// ------------------------------------------------------------ recovery_reentrant

/// A database whose last `n` change sets are only in the WAL, crashed with
/// `kind`; returns the image.
fn wal_only_image<E: Target>(seed: u64, n: u64, cfg: &UnitConfig, ids: &StepIds, kind: CrashKind) -> SimVfs {
    let disk = SimVfs::new(seed);
    let db = open_units::<E>(disk.as_vfs(), cfg).expect("open");
    let (acked, err) = run_steps(&db, ids, seed, 0, n);
    assert!(err.is_none() && acked == n);
    let image = disk.crash_now(kind);
    drop(db);
    image
}

fn recovery_reentrant<E: Target>(kind: CrashKind) {
    let name = format!("recovery_reentrant {} {kind:?}", E::NAME);
    for_each_seed(&name, seeds(4), |seed| {
        let ids = StepIds::new();
        let cfg = unit_config(&ids, Durability::Sync);
        let mut oracle = Oracle::new(seed);
        // Fewer than LARGE_EVERY change sets: nothing is flushed, every
        // change set is in the WAL.
        let n = LARGE_EVERY - 1;
        let base = wal_only_image::<E>(seed, n, &cfg, &ids, kind).dump();
        // The undisturbed recovery: its operations and its result.
        let reference = SimVfs::restore(&base);
        let db = open_units::<E>(reference.as_vfs(), &cfg).expect("recovery");
        let ops = reference.mutating_ops();
        check_store(&db, &mut oracle, n, n, &format!("{name} seed {seed}: undisturbed recovery"));
        let want = raw(&db);
        drop(db);
        assert!(ops > 0 || E::NAME != "kerno", "{name}: recovery wrote nothing; the sweep would prove nothing");
        let oracle = std::cell::RefCell::new(oracle);
        for_each_point(&format!("{name} seed {seed}"), ops + 2, |point| {
            let ctx = format!("{name} seed {seed}: crash before recovery op #{point} of {ops}");
            let oracle = &mut *oracle.borrow_mut();
            let disk = SimVfs::restore_seeded(&base, point);
            if point % 2 == 1 {
                disk.set_tear_unit(1);
            }
            disk.crash_after(point, kind);
            let first = open_units::<E>(disk.as_vfs(), &cfg);
            let image = match disk.take_crash_image() {
                Some(image) => image,
                None => {
                    drop(first.unwrap_or_else(|e| panic!("{ctx}: recovery failed without a crash: {e}")));
                    disk.crash_now(kind)
                }
            };
            let db = open_units::<E>(image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: second recovery: {e}"));
            check_store(&db, oracle, n, n, &ctx);
            assert!(raw(&db) == want, "{ctx}: the second recovery differs from the undisturbed one");
            keeps_working(db, &image, &cfg, &ids, oracle, n, &ctx);
        });
    });
}

// ------------------------------------------------------------ fsync_failure

/// Scenario of the fault sweeps: open, `n` change sets, clean close. Returns
/// (acked, attempted, first error, whether the handle was poisoned after it).
fn fault_run(disk: &SimVfs, cfg: &UnitConfig, ids: &StepIds, seed: u64, n: u64) -> (u64, u64, Option<UnitError>) {
    let db = match open_units::<KernoDb>(disk.as_vfs(), cfg) {
        Ok(db) => db,
        Err(e) => return (0, 0, Some(e)),
    };
    let (acked, err) = run_steps(&db, ids, seed, 0, n);
    match err {
        Some(e) => {
            // Every later change set is refused without I/O.
            let next = run_script(&db, ids, acked + 1, &Script::Changes(vec![]));
            assert!(matches!(&next, Err(e) if is_poisoned(e)), "a change set after {e} must be Poisoned, got {next:?}");
            assert!(
                matches!(db.begin_changeset(ChangeMeta::default()), Err(ref e) if is_poisoned(e)),
                "begin_changeset after {e} must be Poisoned"
            );
            (acked, acked + 1, Some(e))
        }
        None => match db.close() {
            Ok(()) => (acked, acked, None),
            Err(e) => (acked, acked, Some(e)),
        },
    }
}

const FAULT_CHANGE_SETS: u64 = 10;

fn count_ops(cfg: &UnitConfig, ids: &StepIds, seed: u64, pred: impl Fn(&kerno_testkit::vfs::OpInfo) -> bool) -> u64 {
    let dry = SimVfs::new(0);
    dry.start_trace();
    let (acked, _, err) = fault_run(&dry, cfg, ids, seed, FAULT_CHANGE_SETS);
    assert!(err.is_none() && acked == FAULT_CHANGE_SETS, "dry run: {err:?}");
    dry.trace().iter().filter(|t| pred(&t.op)).count() as u64
}

fn fsync_failure(mode: FsyncFailure) {
    let name = format!("fsync_failure {mode:?}");
    for_each_seed(&name, seeds(2), |seed| {
        let ids = StepIds::new();
        let cfg = unit_config(&ids, Durability::Sync);
        let oracle = std::cell::RefCell::new(Oracle::new(seed));
        let syncs = count_ops(&cfg, &ids, seed, |op| op.kind == OpKind::SyncData);
        assert!(syncs >= FAULT_CHANGE_SETS, "one sync per Sync change set at least");
        for_each_point(&format!("{name} seed {seed}"), syncs, |nth| {
            let ctx = format!("{name} seed {seed}: sync_data #{nth} of {syncs} fails");
            let oracle = &mut *oracle.borrow_mut();
            let disk = sweep_disk(seed, nth);
            disk.set_fsync_failure(mode);
            disk.fail_when(|op| op.kind == OpKind::SyncData, nth, None);
            let (acked, attempted, err) = fault_run(&disk, &cfg, &ids, seed, FAULT_CHANGE_SETS);
            assert!(disk.fault_injected(), "{ctx}: the fault did not fire");
            let err = err.unwrap_or_else(|| panic!("{ctx}: a failed sync_data went unreported"));
            assert!(is_io(&err), "{ctx}: the failed sync must return Io, got {err}");
            // Both crash kinds, alternating.
            let kind = if nth % 2 == 0 { CrashKind::PowerLoss } else { CrashKind::ProcessKill };
            let image = disk.crash_now(kind);
            let db = open_units::<KernoDb>(image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: reopen: {e}"));
            let s = check_store(&db, oracle, acked, attempted, &format!("{ctx}, {kind:?}"));
            keeps_working(db, &image, &cfg, &ids, oracle, s, &ctx);
        });
    });
}

// ------------------------------------------------------------ enospc

fn is_disk_full(e: &UnitError) -> bool {
    matches!(e, UnitError::Storage(kerno_core::Error::Io(io)) if io.kind() == std::io::ErrorKind::StorageFull)
}

/// After a full disk: lift the limit, reopen the same device (the process
/// went on) and check.
fn after_full_disk(
    disk: &SimVfs,
    cfg: &UnitConfig,
    ids: &StepIds,
    oracle: &mut Oracle,
    acked: u64,
    hi: u64,
    ctx: &str,
) {
    disk.set_space_limit(None);
    let db = open_units::<KernoDb>(disk.as_vfs(), cfg).unwrap_or_else(|e| panic!("{ctx}: reopen with space: {e}"));
    let s = check_store(&db, oracle, acked, hi, ctx);
    keeps_working(db, disk, cfg, ids, oracle, s, ctx);
}

fn enospc_every_point() {
    let name = "enospc fill_disk_after";
    for_each_seed(name, seeds(2), |seed| {
        let ids = StepIds::new();
        let cfg = unit_config(&ids, Durability::Sync);
        let oracle = std::cell::RefCell::new(Oracle::new(seed));
        let total = count_ops(&cfg, &ids, seed, |_| true);
        for_each_point(&format!("{name} seed {seed}"), total, |point| {
            let ctx = format!("{name} seed {seed}: disk full before mutating op #{point} of {total}");
            let oracle = &mut *oracle.borrow_mut();
            let disk = sweep_disk(seed, point);
            disk.fill_disk_after(point);
            let (acked, attempted, err) = fault_run(&disk, &cfg, &ids, seed, FAULT_CHANGE_SETS);
            // An operation that only shrinks or syncs does not need space:
            // the run may then complete.
            if let Some(e) = &err {
                assert!(is_disk_full(e), "{ctx}: a full disk must return Io(StorageFull), got {e}");
            }
            after_full_disk(&disk, &cfg, &ids, oracle, acked, attempted, &ctx);
        });
    });
}

/// A space limit that leaves room for a new WAL's 96-byte header (nota #11,
/// design §8.2) and `slack` bytes more, set after `before` change sets: the
/// next change sets fail cleanly, never half there.
fn enospc_limit() {
    let name = "enospc set_space_limit";
    for_each_seed(name, seeds(2), |seed| {
        let ids = StepIds::new();
        let cfg = unit_config(&ids, Durability::Sync);
        let mut oracle = Oracle::new(seed);
        let mut full = 0;
        for before in [0u64, 3, LARGE_EVERY - 1, 9] {
            for slack in [0u64, 64, 512, 4_096, 20_000] {
                let ctx = format!("{name} seed {seed}: limit after {before} change sets, header + {slack} B");
                let disk = SimVfs::new(seed ^ slack);
                let db = open_units::<KernoDb>(disk.as_vfs(), &cfg).expect("open");
                let (acked, err) = run_steps(&db, &ids, seed, 0, before);
                assert!(err.is_none() && acked == before);
                disk.set_space_limit(Some(disk.space_used() + HEADER_LEN as u64 + slack));
                let (acked, err) = run_steps(&db, &ids, seed, before, before + 6);
                let hi = match &err {
                    Some(e) => {
                        assert!(is_disk_full(e), "{ctx}: a full disk must return Io(StorageFull), got {e}");
                        full += 1;
                        let next = run_script(&db, &ids, acked + 1, &Script::Changes(vec![]));
                        assert!(matches!(&next, Err(e) if is_poisoned(e)), "{ctx}: then Poisoned, got {next:?}");
                        acked + 1
                    }
                    None => acked,
                };
                drop(db);
                after_full_disk(&disk, &cfg, &ids, &mut oracle, acked, hi, &ctx);
            }
        }
        assert!(full > 0, "{name}: the disk never filled up; the test proved nothing");
    });
}

// ------------------------------------------------------------ torn_write

/// Power loss right after the WAL write of the large change set
/// `LARGE_EVERY` (before its `sync_data`): the pending frame is torn at a
/// random sector (or byte, on odd seeds) and recovery must drop it whole.
fn torn_write() {
    let name = "torn_write";
    let ids = StepIds::new();
    let cfg = unit_config(&ids, Durability::Sync);
    let outcomes = std::cell::RefCell::new([0u64; 2]);
    for_each_seed(name, seeds(16), |seed| {
        let mut oracle = Oracle::new(seed);
        // Where the frame of change set LARGE_EVERY is written.
        let dry = SimVfs::new(0);
        let db = open_units::<KernoDb>(dry.as_vfs(), &cfg).expect("open");
        let (acked, err) = run_steps(&db, &ids, seed, 0, LARGE_EVERY - 1);
        assert!(err.is_none() && acked == LARGE_EVERY - 1);
        dry.start_trace();
        let (acked, err) = run_steps(&db, &ids, seed, LARGE_EVERY - 1, LARGE_EVERY);
        assert!(err.is_none() && acked == LARGE_EVERY);
        let trace = dry.trace();
        let frame = trace
            .iter()
            .filter(|t| t.op.kind == OpKind::WriteAt && t.op.path.extension().is_some_and(|e| e == "wal"))
            .max_by_key(|t| t.op.len)
            .expect("the change set writes the WAL");
        assert!(frame.op.len > 8_000, "the frame of the large change set spans many sectors");
        let after_frame = frame.n - trace[0].n + 1;
        drop(db);

        for unit in [512, 1] {
            let ctx = format!("{name} seed {seed}, tear unit {unit}");
            let disk = SimVfs::new(seed);
            disk.set_tear_unit(unit);
            let db = open_units::<KernoDb>(disk.as_vfs(), &cfg).expect("open");
            let (acked, err) = run_steps(&db, &ids, seed, 0, LARGE_EVERY - 1);
            assert!(err.is_none() && acked == LARGE_EVERY - 1);
            disk.crash_after(after_frame + 1, CrashKind::PowerLoss);
            let (acked, err) = run_steps(&db, &ids, seed, LARGE_EVERY - 1, LARGE_EVERY);
            assert!(err.is_some() && acked == LARGE_EVERY - 1, "{ctx}: the large change set must not be acknowledged");
            let image = disk.take_crash_image().expect("crash image");
            drop(db);
            let db = open_units::<KernoDb>(image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: recovery: {e}"));
            let s = check_store(&db, &mut oracle, LARGE_EVERY - 1, LARGE_EVERY, &ctx);
            outcomes.borrow_mut()[(s - (LARGE_EVERY - 1)) as usize] += 1;
            keeps_working(db, &image, &cfg, &ids, &mut oracle, s, &ctx);
        }
    });
    let [dropped, whole] = outcomes.into_inner();
    eprintln!("{name}: frame dropped {dropped} times, survived whole {whole} times");
    assert!(dropped > 0, "{name}: no power loss dropped the frame; the test proved nothing");
}

// ------------------------------------------------------------ wal_independence

/// Every revision of `ids` as `get_revision` returns it (payload verified),
/// plus each `history` page.
fn histories(db: &UnitDb<KernoDb>, units: &[UnitId]) -> Vec<(Vec<Revision>, Vec<Revision>)> {
    units
        .iter()
        .map(|&id| {
            let page = db.history(id, HistoryQuery::oldest(1_000)).expect("history");
            let full = page.iter().map(|r| db.get_revision(id, r.id.rev).expect("get_revision")).collect();
            (page, full)
        })
        .collect()
}

fn wal_files(disk: &SimVfs) -> BTreeSet<String> {
    disk.files()
        .into_iter()
        .filter(|(p, _)| p.extension().is_some_and(|e| e == "wal"))
        .map(|(p, _)| p.display().to_string())
        .collect()
}

/// I-20: the history written while it lived only in the WAL is identical
/// after the memtable is flushed and that WAL deleted or recycled, after a
/// clean reopen, after a power loss, and after `compact`. With
/// `wal_preallocate` the engine recycles flushed WALs (nota #27).
fn wal_independence(preallocate: bool) {
    let ids = StepIds::new();
    let cfg = unit_config(&ids, Durability::Sync);
    let mut ecfg = kerno_config(&cfg.engine);
    ecfg.wal_preallocate = preallocate;
    let open = |disk: &SimVfs| {
        let engine = KernoDb::open(disk.as_vfs(), std::path::Path::new(DIR), &ecfg).expect("open the engine");
        UnitDb::from_engine(engine, &cfg).expect("open the layer")
    };
    let disk = SimVfs::new(1);
    let db = open(&disk);
    // Phase 1: a small history on four units, all of it in the first WAL.
    let new = |t: &str, p: &[u8]| NewUnit {
        unit_type: t.into(),
        schema_version: 1,
        metadata: b"m".to_vec(),
        payload: p.to_vec(),
    };
    let upd = |p: &[u8]| UnitUpdate { payload: p.to_vec(), schema_version: None, metadata: None };
    let mut units = Vec::new();
    ids.start_step(1);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for i in 0..4u8 {
        units.push(cs.create(new("doc", &[i; 100])).unwrap());
    }
    cs.commit().unwrap();
    for round in 1..=3u8 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(units[0], upd(&[round; 200]), None).unwrap();
        match round {
            1 => cs.archive(units[1], None).unwrap(),
            2 => cs.unarchive(units[1], None).unwrap(),
            _ => cs.delete(units[2], None).unwrap(),
        };
        cs.restore(units[3], RevNo(1), None).unwrap();
        cs.commit().unwrap();
    }
    let want = histories(&db, &units);
    let first_wals = wal_files(&disk);
    assert!(!first_wals.is_empty());
    // Phase 2: other units only, with payloads that fill memtables, until
    // every WAL of phase 1 is gone (flushed and deleted or recycled).
    let mut rng = Rng::new(9);
    let mut n = 0u64;
    while !wal_files(&disk).is_disjoint(&first_wals) {
        n += 1;
        assert!(n < 500, "the first WAL was never retired");
        ids.start_step(100 + n);
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.create(new("img", &rng.bytes(3_000))).unwrap();
        cs.commit().unwrap();
        assert_eq!(histories(&db, &units), want, "history changed during flushes ({n} change sets)");
    }
    // A few more rotations, then every way of reading it back.
    for i in 0..30 {
        ids.start_step(1_000 + i);
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.create(new("img", &rng.bytes(3_000))).unwrap();
        cs.commit().unwrap();
    }
    assert_eq!(histories(&db, &units), want, "history after WAL rotation");
    verified(&db, "after WAL rotation");
    db.compact().expect("compact");
    assert_eq!(histories(&db, &units), want, "history after compact");
    db.close().unwrap();
    let db = open(&disk);
    assert_eq!(histories(&db, &units), want, "history after a clean reopen");
    verified(&db, "after a clean reopen");
    let image = disk.crash_now(CrashKind::PowerLoss);
    drop(db);
    let db = open(&image);
    assert_eq!(histories(&db, &units), want, "history after a power loss");
    verified(&db, "after a power loss");
}

// ------------------------------------------------------------ tests

macro_rules! sweeps {
    ($m:ident, $e:ty) => {
        mod $m {
            use super::*;
            type E = $e;

            #[test]
            fn changeset_every_point_power_loss_sync() {
                super::changeset_every_point::<E>(CrashKind::PowerLoss, Durability::Sync);
            }
            #[test]
            fn changeset_every_point_power_loss_buffered() {
                super::changeset_every_point::<E>(CrashKind::PowerLoss, Durability::Buffered);
            }
            #[test]
            fn changeset_every_point_process_kill_sync() {
                super::changeset_every_point::<E>(CrashKind::ProcessKill, Durability::Sync);
            }
            #[test]
            fn changeset_every_point_process_kill_buffered() {
                super::changeset_every_point::<E>(CrashKind::ProcessKill, Durability::Buffered);
            }
            #[test]
            fn model_crash_power_loss_sync() {
                super::model_crash::<E>(CrashKind::PowerLoss, Durability::Sync);
            }
            #[test]
            fn model_crash_power_loss_buffered() {
                super::model_crash::<E>(CrashKind::PowerLoss, Durability::Buffered);
            }
            #[test]
            fn model_crash_process_kill_sync() {
                super::model_crash::<E>(CrashKind::ProcessKill, Durability::Sync);
            }
            #[test]
            fn model_crash_process_kill_buffered() {
                super::model_crash::<E>(CrashKind::ProcessKill, Durability::Buffered);
            }
            #[test]
            fn recovery_reentrant_power_loss() {
                super::recovery_reentrant::<E>(CrashKind::PowerLoss);
            }
            #[test]
            fn recovery_reentrant_process_kill() {
                super::recovery_reentrant::<E>(CrashKind::ProcessKill);
            }
        }
    };
}

sweeps!(emulated_log, Emulated<LogEngine>);
sweeps!(kerno, KernoDb);

#[test]
fn fsync_failure_keep_pending() {
    fsync_failure(FsyncFailure::KeepPending);
}

#[test]
fn fsync_failure_linux_drop_pages() {
    fsync_failure(FsyncFailure::LinuxDropPages);
}

#[test]
fn enospc_fill_disk_every_point() {
    enospc_every_point();
}

#[test]
fn enospc_space_limit_after_wal_header() {
    enospc_limit();
}

#[test]
fn torn_write_large_change_set() {
    torn_write();
}

#[test]
fn wal_independence_deleted_wals() {
    wal_independence(false);
}

#[test]
fn wal_independence_recycled_wals() {
    wal_independence(true);
}
