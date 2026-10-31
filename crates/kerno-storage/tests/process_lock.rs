//! The exclusive lock across real processes (I-07, RO-1): a second process
//! gets `Locked` from every way into the directory while the first one holds
//! it, and the lock goes away with the process, killed or not (phase1-design.md
//! §6.6). Linux only: `flock` through `StdVfs`.
//!
//! The holder is this same test binary run again as a child
//! (`kerno_testkit::process`), so there is nothing else to build.
#![cfg(target_os = "linux")]

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::StdVfs;
use kerno_core::{Error, WriteBatch};
use kerno_storage::{AccessMode, Config, KernoDb, VerifyOptions, inspect, verify};
use kerno_testkit::process::{child_args, child_role, next_report, report, spawn_child};

/// Long enough for a loaded CI machine to start a process and open a database.
const DEADLINE: Duration = Duration::from_secs(60);

/// A directory in the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("kerno-process-lock-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir); // a leftover of an earlier run that died
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_only() -> Config {
    Config { access: AccessMode::ReadOnly, ..Config::default() }
}

/// The child: opens the database read-write on the real file system, commits
/// a key, says `ready`, and keeps the handle until its stdin closes.
fn hold(dir: &Path) {
    let db = KernoDb::open(StdVfs::shared(), dir, &Config::default()).expect("the holder opens the database");
    let mut b = WriteBatch::new();
    b.put("held", "by the child");
    db.commit(&b).expect("the holder commits");
    report("ready");
    let mut rest = Vec::new();
    std::io::stdin().read_to_end(&mut rest).expect("stdin of the holder");
    db.close().expect("the holder closes cleanly");
}

/// Starts the holder and returns it once it reports `ready`. Panics, with the
/// child killed, if it does not within [`DEADLINE`].
fn start_holder(test: &str, dir: &Path) -> Child {
    let mut child = spawn_child(test, "hold", &[dir.to_str().expect("a UTF-8 temp dir")]);
    let mut out = BufReader::new(child.stdout.take().expect("piped stdout"));
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(next_report(&mut out));
        // Keep reading: the child's stdout must never fill up and block it.
        let mut sink = Vec::new();
        let _ = out.read_to_end(&mut sink);
    });
    match rx.recv_timeout(DEADLINE) {
        Ok(Some(msg)) if msg == "ready" => child,
        other => {
            let _ = child.kill();
            panic!("the holder did not become ready: {other:?}");
        }
    }
}

/// Every way into the directory: all of them must say `Locked`, none may
/// change anything.
fn assert_locked_everywhere(dir: &Path) {
    let vfs = StdVfs::shared();
    let e = KernoDb::open(vfs.clone(), dir, &Config::default()).err().expect("read-write open must fail");
    assert!(matches!(e, Error::Locked), "read-write open: {e:?}");
    let e = KernoDb::open(vfs.clone(), dir, &read_only()).err().expect("read-only open must fail");
    assert!(matches!(e, Error::Locked), "read-only open: {e:?}");
    let e = verify(vfs.clone(), dir, &VerifyOptions::default()).expect_err("verify must fail");
    assert!(matches!(e, Error::Locked), "verify: {e:?}");
    let e = inspect(vfs, dir).expect_err("inspect must fail");
    assert!(matches!(e, Error::Locked), "inspect: {e:?}");
}

/// The database opens, holds what the holder committed, and verifies.
fn assert_free_and_intact(dir: &Path) {
    let vfs = StdVfs::shared();
    let report = verify(vfs.clone(), dir, &VerifyOptions::default()).expect("verify after the holder is gone");
    assert!(report.is_ok(), "{:?}", report.issues);
    inspect(vfs.clone(), dir).expect("inspect after the holder is gone");
    let db = KernoDb::open(vfs, dir, &Config::default()).expect("open after the holder is gone");
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.get(b"held").unwrap().as_deref(), Some(&b"by the child"[..]));
    db.close().unwrap();
}

#[test]
fn second_process_gets_locked() {
    if let Some(role) = child_role() {
        assert_eq!(role, "hold");
        hold(Path::new(&child_args()[0]));
        return;
    }
    let tmp = TempDir::new("second");
    let mut child = start_holder("second_process_gets_locked", &tmp.0);
    assert_locked_everywhere(&tmp.0);
    // Still locked a moment later: the first answers were not luck.
    thread::sleep(Duration::from_millis(100));
    assert_locked_everywhere(&tmp.0);
    // The holder closes cleanly when its stdin does, and the lock is free.
    drop(child.stdin.take());
    assert!(child.wait().expect("wait for the holder").success(), "the holder failed");
    assert_free_and_intact(&tmp.0);
}

#[test]
fn lock_released_when_holder_killed() {
    if let Some(role) = child_role() {
        assert_eq!(role, "hold");
        hold(Path::new(&child_args()[0]));
        return;
    }
    let tmp = TempDir::new("killed");
    let mut child = start_holder("lock_released_when_holder_killed", &tmp.0);
    assert_locked_everywhere(&tmp.0);
    // SIGKILL: no destructor, no close, the WAL is left as it is.
    child.kill().expect("kill the holder");
    child.wait().expect("wait for the holder");
    // The kernel released the lock with the process; the database recovers.
    assert_free_and_intact(&tmp.0);
}
