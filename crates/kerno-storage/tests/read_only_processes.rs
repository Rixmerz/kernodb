//! One writer process and several readers (ADR "Concurrent read-only
//! handles" in `DESIGN.md`): while a child commits batches of ten keys, with a
//! small write buffer so that it flushes, compacts and deletes files all the
//! time, this process opens the directory read-only again and again and
//! refreshes a handle it keeps. Every state a reader sees must hold whole
//! batches, never go back, and a second writer must stay locked out.
//!
//! The writer is this same test binary run again as a child
//! (`kerno_testkit::process`). Linux only: `flock` through `StdVfs`.
#![cfg(target_os = "linux")]

use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::StdVfs;
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::{AccessMode, Config, KernoDb};
use kerno_testkit::harness::read_all;
use kerno_testkit::process::{child_args, child_role, next_report, report, spawn_child};

const DEADLINE: Duration = Duration::from_secs(120);
/// Batches the writer commits; each is `KEYS_PER_BATCH` new keys and one
/// overwrite of a hot key, so that compactions have something to drop.
const BATCHES: u64 = 2_500;
const KEYS_PER_BATCH: u64 = 10;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("kerno-ro-processes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn writer_config() -> Config {
    Config {
        durability: Durability::Buffered,
        write_buffer_bytes: 32 << 10,
        allow_unverified_platform: true,
        ..Config::default()
    }
}

fn reader_config() -> Config {
    Config { access: AccessMode::ReadOnly, allow_unverified_platform: true, ..Config::default() }
}

/// The child: commits `BATCHES` batches, says `started` after the first and
/// `done` after the last, and keeps the database open until stdin closes.
fn write(dir: &Path) {
    let db = KernoDb::open(StdVfs::shared(), dir, &writer_config()).expect("the writer opens");
    for n in 0..BATCHES {
        let mut b = WriteBatch::new();
        for k in 0..KEYS_PER_BATCH {
            b.put(format!("b{n:06}-{k}"), vec![(n % 251) as u8; 120]);
        }
        b.put("hot", n.to_le_bytes());
        db.commit(&b).expect("the writer commits");
        if n == 0 {
            report("started");
        }
    }
    report("done");
    let mut rest = Vec::new();
    std::io::stdin().read_to_end(&mut rest).expect("stdin of the writer");
    db.close().expect("the writer closes cleanly");
}

/// The batches a handle holds: the number of `b…` keys and the sequence of
/// the snapshot they were read at.
fn state_of(db: &KernoDb) -> (u64, u64) {
    let snap = db.snapshot().unwrap();
    let model = read_all(&snap).expect("a snapshot reads without error");
    let keys = model.keys().filter(|k| k.starts_with(b"b")).count() as u64;
    assert_eq!(keys % KEYS_PER_BATCH, 0, "a reader saw part of a batch ({keys} keys)");
    // The batch that wrote `hot` also wrote its ten keys, in the same commit.
    let hot = model.get(&b"hot"[..]).map_or(0, |v| u64::from_le_bytes(v[..8].try_into().unwrap()) + 1);
    assert_eq!(hot, keys / KEYS_PER_BATCH, "the hot key and the batch keys come from different commits");
    (keys, snap.seq())
}

#[test]
fn readers_see_whole_batches_beside_a_writer_that_flushes_and_compacts() {
    if let Some(role) = child_role() {
        assert_eq!(role, "write");
        write(Path::new(&child_args()[0]));
        return;
    }
    let tmp = TempDir::new("batches");
    let mut child = spawn_child(
        "readers_see_whole_batches_beside_a_writer_that_flushes_and_compacts",
        "write",
        &[tmp.0.to_str().expect("a UTF-8 temp dir")],
    );
    let mut out = BufReader::new(child.stdout.take().expect("piped stdout"));
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        while let Some(msg) = next_report(&mut out) {
            if tx.send(msg).is_err() {
                return;
            }
        }
        let mut sink = Vec::new();
        let _ = out.read_to_end(&mut sink);
    });
    let started = Instant::now();
    assert_eq!(rx.recv_timeout(DEADLINE).expect("the writer starts"), "started");

    let vfs = StdVfs::shared();
    let kept = KernoDb::open(vfs.clone(), &tmp.0, &reader_config()).expect("a reader beside the writer");
    let (mut kept_keys, mut kept_seq) = state_of(&kept);
    let mut distinct = std::collections::BTreeSet::from([kept_keys]);
    let mut finished = false;
    let mut opens = 0u64;
    while !finished {
        assert!(started.elapsed() < DEADLINE, "the writer did not finish");
        finished = matches!(rx.try_recv(), Ok(m) if m == "done");
        // A reader that opens now, and sees a state that holds whole batches.
        let fresh = KernoDb::open(vfs.clone(), &tmp.0, &reader_config()).expect("open beside a busy writer");
        distinct.insert(state_of(&fresh).0);
        opens += 1;
        // The reader kept from before moves forward and never back.
        let seq = kept.refresh().expect("refresh beside a busy writer");
        let (keys, snap_seq) = state_of(&kept);
        assert!(keys >= kept_keys && seq >= kept_seq && snap_seq == seq, "refresh went back or lost a commit");
        (kept_keys, kept_seq) = (keys, seq);
        distinct.insert(keys);
        // The directory has one writer.
        let e = KernoDb::open(vfs.clone(), &tmp.0, &Config { allow_unverified_platform: true, ..Config::default() })
            .err()
            .expect("a second writer must be refused");
        assert!(matches!(e, Error::Locked), "{e:?}");
    }
    kept.refresh().unwrap();
    assert_eq!(state_of(&kept).0, BATCHES * KEYS_PER_BATCH, "after `done` a refresh sees every batch");
    let fresh = KernoDb::open(vfs, &tmp.0, &reader_config()).unwrap();
    assert_eq!(state_of(&fresh).0, BATCHES * KEYS_PER_BATCH);
    assert!(opens > 1 && distinct.len() > 2, "readers never saw the writer at work: {opens} opens, {distinct:?}");
    drop((kept, fresh));
    drop(child.stdin.take());
    assert!(child.wait().expect("wait for the writer").success(), "the writer failed");
}
