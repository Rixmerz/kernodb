//! E3-08: `kill -9` of a real process committing change sets
//! (etapa3-design.md §8.3), like `kerno-storage/tests/process_kill.rs`.
//!
//! A child process (this test binary run again, `kerno_testkit::process`)
//! opens `UnitDb<KernoDb>` on the real file system with `StdVfs` and commits
//! the workload of `harness::units` (seed `SEED`) one change set at a time,
//! reporting each acknowledged sequence. The parent sends SIGKILL at a random
//! moment, checks the directory with the engine's `verify()`, reopens it and
//! requires:
//!
//! * the recovered sequence `s` is at least the last one the child reported
//!   and at most one more (the change set in flight);
//! * the store is byte for byte the one after exactly `s` change sets of the
//!   oracle (I-21: no change set is ever half there);
//! * `verify_units` finds nothing.
//!
//! Every iteration continues from the directory the previous one left, half
//! of the time after a clean close. The engine runs with its background
//! threads and tiny buffers, so kills also land in flushes and compactions.
//!
//! `KERNO_KILL_ITERS` sets the iterations per test (default 20; the weekly
//! job uses 200). Linux only.
#![cfg(target_os = "linux")]

mod harness;

use std::io::BufReader;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use harness::units::*;
use kerno_core::Durability;
use kerno_core::vfs::StdVfs;
use kerno_storage::{Config, KernoDb, VerifyOptions, verify};
use kerno_testkit::process::{child_args, child_role, next_report, report, spawn_child};
use kerno_testkit::rng::Rng;
use kerno_units::UnitDb;

/// Workload seed of the child and of the oracle.
const SEED: u64 = 0x0E30_8000;
/// Long enough for a loaded machine to start a process and open a database.
const DEADLINE: Duration = Duration::from_secs(60);
/// Safety stop for a child nobody kills.
const MAX_CHANGE_SETS: u64 = 1 << 20;

/// The two tests never run at once (a `fork` in one would copy the other's
/// `LOCK` descriptor; see the engine's `process_kill.rs`).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("kerno-units-kill-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mode_name(d: Durability) -> &'static str {
    match d {
        Durability::Sync => "sync",
        Durability::Buffered => "buffered",
    }
}

fn open_at(dir: &Path, ids: &StepIds, durability: Durability) -> UnitDb<KernoDb> {
    let cfg = unit_config(ids, durability);
    let engine = KernoDb::open(StdVfs::shared(), dir, &Config::from(&cfg.engine)).expect("open the engine");
    UnitDb::from_engine(engine, &cfg).expect("open the layer")
}

/// The child: commits the workload from the recovered sequence on, reporting
/// each acknowledged change set, until it is killed.
fn writer(dir: &Path, durability: Durability) {
    let ids = StepIds::new();
    let db = open_at(dir, &ids, durability);
    let start = seq(&db);
    report(&format!("open {start}"));
    for k in start + 1..=start + MAX_CHANGE_SETS {
        let (acked, err) = run_steps(&db, &ids, SEED, k - 1, k);
        assert!(err.is_none() && acked == k, "the writer commits: {err:?}");
        report(&format!("acked {k}"));
    }
    std::mem::forget(db);
}

struct Progress {
    acked: AtomicU64,
    opened: AtomicBool,
}

fn kill_iterations() -> u64 {
    std::env::var("KERNO_KILL_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20)
}

fn kill_loop(test: &str, durability: Durability) {
    let _serial = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let mode = mode_name(durability);
    let tmp = TempDir::new(mode);
    let ids = StepIds::new();
    open_at(&tmp.0, &ids, durability).close().expect("create the database");
    let mut oracle = Oracle::new(SEED);
    let mut floor = 0u64;
    let iterations = kill_iterations();
    let mut kills_after_progress = 0;
    for iter in 0..iterations {
        let label = format!("units {mode} iteration {iter}");
        let mut rng = Rng::new(0x554B_494C ^ iter);
        let mut child = spawn_child(test, "writer", &[tmp.0.to_str().expect("a UTF-8 temp dir"), mode]);
        let mut out = BufReader::new(child.stdout.take().expect("piped stdout"));
        let progress = Arc::new(Progress { acked: AtomicU64::new(0), opened: AtomicBool::new(false) });
        let reader = {
            let progress = progress.clone();
            thread::spawn(move || {
                while let Some(msg) = next_report(&mut out) {
                    if msg.starts_with("open ") {
                        progress.opened.store(true, Ordering::SeqCst);
                    } else if let Some(n) = msg.strip_prefix("acked ") {
                        progress.acked.fetch_max(n.parse().expect("a sequence"), Ordering::SeqCst);
                    }
                }
            })
        };
        if rng.chance(3, 4) {
            let started = Instant::now();
            while progress.acked.load(Ordering::SeqCst) <= floor {
                assert!(started.elapsed() < DEADLINE, "{label}: the writer committed nothing within {DEADLINE:?}");
                thread::sleep(Duration::from_micros(200));
            }
            kills_after_progress += 1;
            thread::sleep(Duration::from_micros(rng.below(40_000)));
        } else {
            thread::sleep(Duration::from_micros(rng.below(60_000)));
        }
        child.kill().expect("SIGKILL the writer");
        let status = child.wait().expect("wait for the writer");
        reader.join().expect("the reader ends with the child's stdout");
        assert_eq!(status.signal(), Some(9), "{label}: the writer must die by SIGKILL ({status:?})");
        let acked = progress.acked.load(Ordering::SeqCst).max(floor);

        let r = verify(StdVfs::shared(), &tmp.0, &VerifyOptions::default()).expect("verify after the kill");
        assert!(r.is_ok(), "{label}: engine verify: {:?}", r.issues);
        let db = open_at(&tmp.0, &ids, durability);
        // An acknowledged change set survives a process kill in both modes;
        // the one in flight may or may not.
        floor = check_store(&db, &mut oracle, acked, acked + 1, &label);
        oracle.forget_below(floor);
        if iter % 2 == 0 {
            db.close().expect("close after the check");
        }
    }
    let db = open_at(&tmp.0, &ids, durability);
    let s = check_store(&db, &mut oracle, floor, floor, "final");
    db.close().expect("final close");
    assert!(s > 0, "{iterations} kills and not one change set survived: the test proved nothing");
    eprintln!("units process_kill {mode}: {iterations} kills ({kills_after_progress} after progress), recovered {s}");
}

#[test]
fn kill_dash_9_changeset_writer_sync() {
    if let Some(role) = child_role() {
        assert_eq!(role, "writer");
        let args = child_args();
        writer(Path::new(&args[0]), if args[1] == "sync" { Durability::Sync } else { Durability::Buffered });
        return;
    }
    kill_loop("kill_dash_9_changeset_writer_sync", Durability::Sync);
}

#[test]
fn kill_dash_9_changeset_writer_buffered() {
    if let Some(role) = child_role() {
        assert_eq!(role, "writer");
        let args = child_args();
        writer(Path::new(&args[0]), if args[1] == "sync" { Durability::Sync } else { Durability::Buffered });
        return;
    }
    kill_loop("kill_dash_9_changeset_writer_buffered", Durability::Buffered);
}
