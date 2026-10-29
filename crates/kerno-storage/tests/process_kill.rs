//! `kill -9` of a real writer process (phase1-design.md §6.6, §7.1 item 4).
//!
//! A child process (this same test binary run again, `kerno_testkit::process`)
//! opens the database on the real file system with `StdVfs` and commits a
//! deterministic stream of batches, reporting each acknowledged sequence on
//! its stdout. The parent sends SIGKILL at a random moment, reopens the
//! directory, and requires:
//!
//! * the recovered sequence `s` is at least the last sequence the child
//!   reported (an acknowledged commit survives a process kill in both
//!   durability modes: the bytes are in the page cache once `write` returns),
//! * the recovered contents are exactly the model after batches `1..=s`
//!   (nothing lost inside the prefix, nothing half-applied, no data from a
//!   commit after `s`),
//! * `verify()` accepts the directory before it is opened.
//!
//! Every iteration continues from the state the previous one recovered, so
//! the chain also covers a child that starts on a directory its predecessor
//! left dirty. The database uses tiny buffers, so kills land in flushes and
//! compactions as well as in commits.
//!
//! `KERNO_KILL_ITERS` sets the iterations per test (default 20; the weekly
//! job and the Phase 1 exit run use 200). Linux only.
#![cfg(target_os = "linux")]

use std::io::BufReader;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use kerno_core::contract::{Durability, Engine, Snapshot};
use kerno_core::vfs::StdVfs;
use kerno_storage::{KernoDb, VerifyOptions, verify};
use kerno_testkit::harness::{Model, apply, read_all};
use kerno_testkit::process::{child_args, child_role, next_report, report, spawn_child};
use kerno_testkit::rng::Rng;

#[path = "common/batch_stream.rs"]
mod batch_stream;
use batch_stream::{batch_for, config, mode_from, mode_name};

/// Long enough for a loaded machine to start a process and open a database.
const DEADLINE: Duration = Duration::from_secs(60);
/// Safety stop for a child nobody kills.
const MAX_COMMITS: u64 = 1 << 20;

/// The two tests of this file never run at the same time. A `fork` in one
/// test thread copies the descriptors the other thread has open, among them
/// the `LOCK` file of a `verify()` or an open in progress, and `flock` is
/// released only when every copy is gone: the other test's reopen would see
/// `Locked` until the forked child reaches `exec`.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// A directory in the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("kerno-process-kill-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // a leftover of an earlier run that died
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The child: opens the database, then commits `batch_for(seq)` for each next
/// sequence and reports it once `commit` returned, until it is killed.
fn writer(dir: &Path, durability: Durability) {
    let db = KernoDb::open(StdVfs::shared(), dir, &config(durability)).expect("the writer opens the database");
    let start = db.snapshot().expect("snapshot of the recovered state").seq();
    report(&format!("open {start}"));
    for seq in start + 1..=start + MAX_COMMITS {
        let got = db.commit(&batch_for(seq)).expect("the writer commits");
        assert_eq!(got, seq, "sequences are consecutive");
        report(&format!("acked {seq}"));
    }
    // Not reached when the parent does its job; leaves the handle unclosed
    // like the kill would.
    std::mem::forget(db);
}

/// What the parent reads from a running child.
struct Progress {
    /// Highest sequence the child reported as acknowledged.
    acked: AtomicU64,
    /// The child opened the database and said so.
    opened: AtomicBool,
}

fn kill_iterations() -> u64 {
    std::env::var("KERNO_KILL_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(20)
}

/// Checks the reopened database against the model and returns its sequence.
fn check_recovered(db: &KernoDb, model: &mut Model, model_seq: &mut u64, acked: u64, label: &str) -> u64 {
    let snap = db.snapshot().expect("snapshot after recovery");
    let s = snap.seq();
    assert!(s >= acked, "{label}: recovered sequence {s} is below the last acknowledged commit {acked}");
    assert!(s >= *model_seq, "{label}: recovered sequence {s} went back from {}", *model_seq);
    for seq in *model_seq + 1..=s {
        apply(model, &batch_for(seq));
    }
    *model_seq = s;
    let got = read_all(&snap).expect("read the recovered contents");
    assert_eq!(got.len(), model.len(), "{label}: number of live keys after recovering sequence {s}");
    assert!(got == *model, "{label}: contents differ from the model after batches 1..={s}");
    s
}

fn kill_loop(test: &str, durability: Durability) {
    let _serial = ONE_AT_A_TIME.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mode = mode_name(durability);
    let tmp = TempDir::new(mode);
    let vfs = StdVfs::shared();
    // The directory is created and closed before the first child, so that a
    // kill very early in a child's life finds a complete database.
    KernoDb::open(vfs.clone(), &tmp.0, &config(durability)).expect("create the database").close().expect("close");

    let iterations = kill_iterations();
    let mut model = Model::new();
    let mut model_seq = 0u64;
    let mut kills_after_progress = 0u64;
    for iter in 0..iterations {
        let label = format!("{mode} iteration {iter}");
        let mut rng = Rng::new(0x4B49_4C4C ^ iter);
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
                        progress.acked.fetch_max(n.parse().expect("a sequence number"), Ordering::SeqCst);
                    }
                }
            })
        };

        // Three kills in four come at a random moment after the child
        // started committing; the rest at a random moment from the spawn,
        // which lands in process start-up, in the open or in recovery.
        if rng.chance(3, 4) {
            let started = Instant::now();
            while progress.acked.load(Ordering::SeqCst) <= model_seq {
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
        reader.join().expect("the reader thread ends when the child's stdout closes");
        assert_eq!(status.signal(), Some(9), "{label}: the writer must die by SIGKILL, not on its own ({status:?})");
        let acked = progress.acked.load(Ordering::SeqCst);
        assert!(acked == 0 || progress.opened.load(Ordering::SeqCst), "{label}: commits acknowledged before open");

        let report = verify(vfs.clone(), &tmp.0, &VerifyOptions::default()).expect("verify after the kill");
        assert!(report.is_ok(), "{label}: verify after the kill: {:?}", report.issues);
        let db = KernoDb::open(vfs.clone(), &tmp.0, &config(durability)).expect("reopen after the kill");
        check_recovered(&db, &mut model, &mut model_seq, acked, &label);
        // Alternate a clean close with dropping the handle, so that the next
        // child starts on a clean directory half the time and on a dirty one
        // the other half.
        if iter % 2 == 0 {
            db.close().expect("close after the check");
        }
    }

    let db = KernoDb::open(vfs.clone(), &tmp.0, &config(durability)).expect("final reopen");
    let s = check_recovered(&db, &mut model, &mut model_seq, 0, "final");
    db.close().expect("final close");
    let report = verify(vfs, &tmp.0, &VerifyOptions::default()).expect("final verify");
    assert!(report.is_ok(), "final verify: {:?}", report.issues);
    assert!(s > 0, "{iterations} kills and not one commit survived: the test proved nothing");
    eprintln!(
        "process_kill {mode}: {iterations} kills ({kills_after_progress} after progress), recovered sequence {s}"
    );
}

#[test]
fn kill_dash_9_writer_recovers_acked_prefix_sync() {
    if let Some(role) = child_role() {
        assert_eq!(role, "writer");
        let args = child_args();
        writer(Path::new(&args[0]), mode_from(&args[1]));
        return;
    }
    kill_loop("kill_dash_9_writer_recovers_acked_prefix_sync", Durability::Sync);
}

#[test]
fn kill_dash_9_writer_recovers_acked_prefix_buffered() {
    if let Some(role) = child_role() {
        assert_eq!(role, "writer");
        let args = child_args();
        writer(Path::new(&args[0]), mode_from(&args[1]));
        return;
    }
    kill_loop("kill_dash_9_writer_recovers_acked_prefix_buffered", Durability::Buffered);
}
