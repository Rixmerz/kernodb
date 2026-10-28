//! Helpers shared by the test targets of the metadata log, recovery, read-only
//! open and deferred deletion (WP08): engine configurations, small databases
//! in known states, and readers of the files the engine writes.
//!
//! Every target includes this module whole, and none uses all of it.
#![allow(dead_code)]

pub mod craft;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kerno_core::contract::Engine;
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Result, WriteBatch};
use kerno_storage::format::header::{FileKind, HEADER_LEN, reseal_header};
use kerno_storage::format::manifest::{ManifestReplay, Record, replay_manifest};
use kerno_storage::format::wal::{CommitOp, Frame, canonicalize, encode_frame};
use kerno_storage::{Background, Config, KernoDb, TestConfig};
use kerno_testkit::harness::crash::{ExtFn, Step};
use kerno_testkit::harness::{Model, SuiteConfig, Workload, apply, read_all};
use kerno_testkit::vfs::{DirImage, SimVfs};

pub const DIR: &str = "/db";
pub const DB_ID: [u8; 16] = [0xd1; 16];

/// Flushes and compactions in the calling thread, a 16 KiB write buffer and a
/// close without a time limit: every sequence of operations is deterministic.
pub fn config(durability: Durability) -> Config {
    Config {
        durability,
        write_buffer_bytes: 16 << 10,
        cache_bytes: 1 << 20,
        test: TestConfig { background: Background::Inline, db_id: Some(DB_ID) },
        close_budget: kerno_storage::CloseBudget::Unbounded,
        ..Config::default()
    }
}

/// [`config`] with background threads.
pub fn threads_config(durability: Durability) -> Config {
    Config { test: TestConfig { background: Background::Threads, db_id: Some(DB_ID) }, ..config(durability) }
}

pub fn open_in(vfs: &Arc<dyn Vfs>, cfg: &Config) -> Result<KernoDb> {
    KernoDb::open(vfs.clone(), Path::new(DIR), cfg)
}

pub fn path(name: &str) -> PathBuf {
    Path::new(DIR).join(name)
}

pub fn put(db: &KernoDb, model: &mut Model, key: &str, value: &[u8]) -> u64 {
    let mut b = WriteBatch::new();
    b.put(key, value);
    let seq = db.commit(&b).unwrap();
    apply(model, &b);
    seq
}

/// `n` commits of one 200-byte value each, from key number `from`.
pub fn fill(db: &KernoDb, model: &mut Model, from: u32, n: u32) {
    for i in from..from + n {
        put(db, model, &format!("key-{i:05}"), &[i as u8; 200]);
    }
}

pub fn model_of(db: &KernoDb) -> Model {
    read_all(&db.snapshot().unwrap()).unwrap()
}

pub fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

/// A database with flushed tables, closed cleanly.
pub fn closed_image() -> (DirImage, Model) {
    let disk = SimVfs::new(1);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 400);
    db.close().unwrap();
    (disk.dump(), model)
}

/// A database with flushed tables and a live WAL of at least eight commits,
/// left without a clean close.
pub fn unclean_image() -> (DirImage, Model) {
    let disk = SimVfs::new(2);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 400);
    fill(&db, &mut model, 400, 8);
    drop(db);
    (disk.dump(), model)
}

/// The files of `image` with this extension (or the MANIFEST), by name.
pub fn files_with(image: &DirImage, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = image
        .files
        .keys()
        .filter(|p| match ext {
            "MANIFEST" => p.file_name().is_some_and(|n| n == "MANIFEST"),
            ext => p.extension().is_some_and(|e| e == ext),
        })
        .cloned()
        .collect();
    v.sort();
    v
}

pub fn file_number(p: &Path) -> u64 {
    p.file_stem().unwrap().to_str().unwrap().parse().unwrap()
}

pub fn le_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Patches the 96-byte header of `file` and recomputes its CRC.
pub fn patch_header(image: &mut DirImage, file: &Path, patch: impl FnOnce(&mut [u8])) {
    let bytes = image.files.get_mut(file).unwrap_or_else(|| panic!("no file {}", file.display()));
    patch(&mut bytes[..HEADER_LEN]);
    assert!(reseal_header(&mut bytes[..HEADER_LEN]));
}

pub fn or_u64(h: &mut [u8], at: usize, bits: u64) {
    let v = le_u64(h, at) | bits;
    h[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// The replay of the MANIFEST of `image` and the offset and contents of each
/// of its valid records.
pub fn manifest_of(image: &DirImage) -> (ManifestReplay, Vec<(u64, Record)>) {
    let bytes = image.files.get(&path("MANIFEST")).expect("no MANIFEST");
    let mut records = Vec::new();
    let replay = replay_manifest(&mut &bytes[..], |at, r| {
        records.push((at, r.clone()));
        Ok(())
    })
    .map_err(|e| e.into_format())
    .unwrap_or_else(|e| panic!("the MANIFEST does not replay: {e}"));
    (replay, records)
}

/// A WAL file of `number` whose frames hold `commits`, one per frame, the
/// first with sequence `base_seq`.
pub fn wal_bytes(number: u64, base_seq: u64, commits: &[WriteBatch]) -> Vec<u8> {
    let mut out = kerno_storage::format::header::Header::wal(DB_ID, number, base_seq).encode().to_vec();
    for (seq, b) in (base_seq..).zip(commits) {
        let canonical = canonicalize(b.ops().iter().map(|op| match op {
            kerno_core::Op::Put { key, value } => CommitOp::Put { key, value },
            kerno_core::Op::Delete { key } => CommitOp::Delete { key },
            kerno_core::Op::DeleteRange { start, end } => CommitOp::DeleteRange { start, end },
        }));
        let frame: Frame = Frame { first_seq: seq, synced_len: 0, commits: vec![canonical.to_owned_commit()] };
        out.extend(encode_frame(number, &frame, false));
    }
    out
}

/// Whether `kind` is the common header kind of a file name.
pub fn kind_of(name: &str) -> FileKind {
    if name.ends_with(".wal") {
        FileKind::Wal
    } else if name.ends_with(".sst") {
        FileKind::Sst
    } else {
        FileKind::Manifest
    }
}

/// Opens `image` read-write on a copy and returns the handle with its disk.
pub fn open_copy(image: &DirImage, cfg: &Config) -> (SimVfs, Result<KernoDb>) {
    let disk = SimVfs::restore(image);
    let r = open_in(&disk.as_vfs(), cfg);
    (disk, r)
}

/// Scenario B (creation, commits, clean close, reopen, more commits, a
/// dropped handle, recovery, a forced checkpoint, a clean close) with a tenth
/// of the operations range deletions, and a directed compaction (`compact`)
/// in the middle and another after the first reopen.
pub fn ranged_scenario<E: Engine>(seed: u64, compact: ExtFn<E>, checkpoint: ExtFn<E>) -> Vec<Step<E>> {
    let mut w = Workload::with_config(seed, SuiteConfig::with_range_deletes(100).workload);
    w.key_space = 120;
    w.large_per_mille = 100;
    let mut model = Model::new();
    let mut steps = vec![Step::Open];
    let mut commits = |n: usize, steps: &mut Vec<Step<E>>| {
        for _ in 0..n {
            let b = w.batch(&model);
            apply(&mut model, &b);
            steps.push(Step::Commit(b));
        }
    };
    commits(30, &mut steps);
    steps.push(Step::Ext("compact_range", compact.clone()));
    commits(10, &mut steps);
    steps.push(Step::Close);
    steps.push(Step::Open);
    commits(15, &mut steps);
    steps.push(Step::Ext("compact_range", compact));
    commits(5, &mut steps);
    steps.push(Step::Drop);
    steps.push(Step::Open);
    commits(10, &mut steps);
    steps.push(Step::Ext("checkpoint", checkpoint));
    commits(5, &mut steps);
    steps.push(Step::Close);
    steps
}
