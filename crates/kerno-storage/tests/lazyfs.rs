//! Scenario A on a LazyFS mount (phase1-design.md §6.6, R-84), driven by
//! `spikes/storage/run-lazyfs.sh`.
//!
//! LazyFS keeps its own page cache and writes it to the backing directory
//! only on `fsync`, so clearing that cache loses exactly what the engine
//! never asked to make durable, on a real file system and without killing
//! anything. The two tests below are the two sides of that run and are
//! `#[ignore]`: they need the mount, so only the script runs them
//! (`--ignored --exact`):
//!
//! 1. `lazyfs_scenario_a_write` runs the M0 scenario A on the mount with
//!    `StdVfs`: create and 25 commits and a clean close, a session of 20
//!    commits that is dropped without closing, a third session of 10 commits
//!    that is also left open. It records on the real file system (outside the
//!    mount) the highest sequence it was promised was durable, and the last one
//!    it committed.
//! 2. The script clears the LazyFS cache.
//! 3. `lazyfs_scenario_a_check` verifies and reopens the directory and
//!    requires the recovered sequence `s` to satisfy `durable <= s <= last` and
//!    the contents to be exactly the model after batches `1..=s`.
//!
//! The promise differs by mode. With `Sync` every acknowledged commit is
//! durable, so `durable == last`. With `Buffered` only what a `sync()` covered
//! is: the writer calls `sync()` after the 5th commit of the third session and
//! then commits 5 more without one.
//!
//! Environment: `KERNO_LAZYFS_DB` (database directory on the mount),
//! `KERNO_LAZYFS_STATE` (directory on the backing disk, outside the mount) and
//! `KERNO_LAZYFS_MODE` (`sync` or `buffered`, default `sync`). Linux only.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};

use kerno_core::contract::{Durability, Engine, Snapshot};
use kerno_core::vfs::StdVfs;
use kerno_storage::{KernoDb, VerifyOptions, verify};
use kerno_testkit::harness::{Model, apply, read_all};

#[path = "common/batch_stream.rs"]
mod batch_stream;
use batch_stream::{batch_for, config, mode_from, mode_name};

fn env_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{name} is not set: run these tests through spikes/storage/run-lazyfs.sh"))
}

fn durability() -> Durability {
    mode_from(&std::env::var("KERNO_LAZYFS_MODE").unwrap_or_else(|_| "sync".into()))
}

fn state_file(durability: Durability) -> PathBuf {
    env_path("KERNO_LAZYFS_STATE").join(format!("scenario-a-{}.state", mode_name(durability)))
}

/// Commits `batch_for(seq)` for each sequence in `seqs`, which must follow
/// the ones already committed.
fn commit_range(db: &KernoDb, seqs: std::ops::RangeInclusive<u64>) {
    for seq in seqs {
        let got = db.commit(&batch_for(seq)).expect("commit");
        assert_eq!(got, seq, "sequences are consecutive");
    }
}

#[test]
#[ignore = "needs a LazyFS mount: run spikes/storage/run-lazyfs.sh"]
fn lazyfs_scenario_a_write() {
    let dir = env_path("KERNO_LAZYFS_DB");
    let durability = durability();
    let vfs = StdVfs::shared();

    let db = KernoDb::open(vfs.clone(), &dir, &config(durability)).expect("create");
    commit_range(&db, 1..=25);
    db.close().expect("clean close");

    let db = KernoDb::open(vfs.clone(), &dir, &config(durability)).expect("second session");
    commit_range(&db, 26..=45);
    drop(db); // no close: the WAL is all there is

    let db = KernoDb::open(vfs, &dir, &config(durability)).expect("third session");
    commit_range(&db, 46..=50);
    // With `Buffered` only what the `sync()` below covers is promised.
    let synced = match durability {
        Durability::Sync => None,
        Durability::Buffered => Some(db.sync().expect("sync")),
    };
    if let Some(s) = synced {
        assert!(s >= 50, "sync covers every commit acknowledged before it");
    }
    commit_range(&db, 51..=55);
    let durable = synced.unwrap_or(55);
    drop(db);

    std::fs::write(state_file(durability), format!("durable={durable} last=55\n")).expect("record the promise");
    eprintln!("lazyfs {}: wrote 55 commits, durable {durable}", mode_name(durability));
}

/// `key=value` pairs of the state file.
fn read_state(path: &Path) -> (u64, u64) {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e} (did the write phase run?)", path.display()));
    let field = |name: &str| -> u64 {
        text.split_whitespace()
            .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("{}: no {name} in {text:?}", path.display()))
    };
    (field("durable"), field("last"))
}

#[test]
#[ignore = "needs a LazyFS mount: run spikes/storage/run-lazyfs.sh"]
fn lazyfs_scenario_a_check() {
    let dir = env_path("KERNO_LAZYFS_DB");
    let durability = durability();
    let (durable, last) = read_state(&state_file(durability));
    let vfs = StdVfs::shared();

    let report = verify(vfs.clone(), &dir, &VerifyOptions::default()).expect("verify after the cache was cleared");
    assert!(report.is_ok(), "verify after the cache was cleared: {:?}", report.issues);
    let db = KernoDb::open(vfs.clone(), &dir, &config(durability)).expect("reopen after the cache was cleared");
    let snap = db.snapshot().expect("snapshot");
    let s = snap.seq();
    assert!(s >= durable, "recovered sequence {s} is below the durable sequence {durable}: a promised commit was lost");
    assert!(s <= last, "recovered sequence {s} is above the last commit {last}");
    let mut model = Model::new();
    for seq in 1..=s {
        apply(&mut model, &batch_for(seq));
    }
    assert!(
        read_all(&snap).expect("read the recovered contents") == model,
        "contents differ from the model after batches 1..={s}"
    );
    drop(snap);
    db.close().expect("close");
    let report = verify(vfs, &dir, &VerifyOptions::default()).expect("verify after the close");
    assert!(report.is_ok(), "verify after the close: {:?}", report.issues);
    eprintln!("lazyfs {}: recovered sequence {s} (durable {durable}, last {last})", mode_name(durability));
}
