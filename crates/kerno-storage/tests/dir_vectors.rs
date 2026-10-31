//! Directory vectors (v1.md §13.2–§13.4): everything the engine writes to
//! disk in format v1, as committed, deterministic directories under
//! `tests/vectors/v1/dirs/<case>/`.
//!
//! * Each case is the raw files plus an `EXPECTED` text file: `seq=N`,
//!   `mode=rw|ro|reject:<Class>`, `clean=0|1` and one `hex(key)<TAB>hex(value)`
//!   line per live key.
//! * The generator (`regenerate_v1_vectors`, `#[ignore]`) writes them; the
//!   guard (`dir_vectors_are_reproducible`) regenerates every case in memory
//!   and compares bytes with the committed files, so any byte change of the
//!   format fails CI until it goes through the v1.md §15 process.
//! * The loaders copy each case into a `SimVfs` and into a `StdVfs`
//!   directory, open it read-only and read-write, compare the sequence and
//!   the contents, and run `verify()`.
//! * The damage tests flip every bit of every byte and truncate at every
//!   offset of the clean cases: each result is `Corruption` or `Io`, or the
//!   exact contents, and none may be harmless.
//!
//! Determinism: `Background::Inline`, a fixed `db_id`, a close budget in bytes
//! only, `Rng(2026)` workloads and crashes chosen by operation filters on a
//! seeded `SimVfs`. The compaction levers are pinned to the M0 values
//! (`CompactionConfig::m0`) so that a change of the engine's tuning defaults
//! cannot move the bytes of a vector.

mod common;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::{OpenMode, StdVfs, Vfs};
use kerno_core::{Durability, Error, MAX_VALUE_LEN, Op, WriteBatch};
use kerno_storage::format::features;
use kerno_storage::format::header::{HEADER_LEN, Header};
use kerno_storage::format::manifest::replay_manifest;
use kerno_storage::format::patch_header;
use kerno_storage::format::sst::{SstExpect, open_sst_from};
use kerno_storage::format::wal::{CommitOp, Frame, WalScanParams, canonicalize, encode_frame, scan_wal};
use kerno_storage::format::{End, ReadError};
use kerno_storage::{
    AccessMode, Background, CloseBudget, CompactionConfig, Config, KernoDb, TestConfig, VerifyOptions, verify,
};
use kerno_testkit::harness::{Model, apply, read_all};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, DirImage, HookAction, HookOp, HookOpKind, HookPoint, HookVfs, OpKind, SimVfs};

use common::{DIR, le_u64};

/// The `db_id` of every vector: bytes `00..0f` (v1.md §13.5).
const VECTOR_DB_ID: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// The seed of the workloads (`Rng(2026)`).
const WORKLOAD_SEED: u64 = 2026;

/// Name of the file that holds what a case must open to.
const EXPECTED: &str = "EXPECTED";

/// Raw files of a case by name (the `LOCK` file is not part of a vector).
type Files = BTreeMap<String, Vec<u8>>;

// ---------------------------------------------------------------------------
// Cases and the EXPECTED file

/// How a case is rejected, as the class of the error `open` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Corruption,
    Incompatible,
    InvalidArgument,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Corruption => "Corruption",
            Class::Incompatible => "Incompatible",
            Class::InvalidArgument => "InvalidArgument",
        }
    }

    fn of(e: &Error) -> Option<Class> {
        match e {
            Error::Corruption(_) => Some(Class::Corruption),
            Error::Incompatible(_) => Some(Class::Incompatible),
            Error::InvalidArgument(_) => Some(Class::InvalidArgument),
            _ => None,
        }
    }
}

/// What opening the directory must do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Opens read-write (and read-only).
    Rw,
    /// Opens, but only for reading, even when read-write is asked for.
    Ro,
    /// The open fails with this class and leaves the directory untouched.
    Reject(Class),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Expected {
    seq: u64,
    mode: Mode,
    /// The last MANIFEST record is a `CLEAN` snapshot.
    clean: bool,
    live: Model,
}

impl Expected {
    fn render(&self) -> String {
        let mode = match self.mode {
            Mode::Rw => "rw".to_string(),
            Mode::Ro => "ro".to_string(),
            Mode::Reject(c) => format!("reject:{}", c.name()),
        };
        let mut out = format!("seq={}\nmode={mode}\nclean={}\n", self.seq, u8::from(self.clean));
        for (k, v) in &self.live {
            writeln!(out, "{}\t{}", hex(k), hex(v)).unwrap();
        }
        out
    }

    fn parse(text: &str) -> Expected {
        let mut lines = text.lines();
        let mut field = |name: &str| {
            let line = lines.next().unwrap_or_else(|| panic!("EXPECTED: no {name} line"));
            line.strip_prefix(&format!("{name}="))
                .unwrap_or_else(|| panic!("EXPECTED: {line:?} is not {name}="))
                .to_string()
        };
        let seq = field("seq").parse().expect("EXPECTED: seq");
        let mode = match field("mode").as_str() {
            "rw" => Mode::Rw,
            "ro" => Mode::Ro,
            "reject:Corruption" => Mode::Reject(Class::Corruption),
            "reject:Incompatible" => Mode::Reject(Class::Incompatible),
            "reject:InvalidArgument" => Mode::Reject(Class::InvalidArgument),
            other => panic!("EXPECTED: unknown mode {other:?}"),
        };
        let clean = match field("clean").as_str() {
            "0" => false,
            "1" => true,
            other => panic!("EXPECTED: clean={other}"),
        };
        let live = lines
            .map(|l| {
                let (k, v) = l.split_once('\t').unwrap_or_else(|| panic!("EXPECTED: no tab in {l:?}"));
                (unhex(k), unhex(v))
            })
            .collect();
        Expected { seq, mode, clean, live }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex length");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex digit")).collect()
}

/// A directory vector: its raw files and what it must do.
#[derive(Clone)]
struct Case {
    name: String,
    files: Files,
    expected: Expected,
}

impl Case {
    /// The files as they are committed: the raw ones plus `EXPECTED`.
    fn committed_form(&self) -> Files {
        let mut files = self.files.clone();
        files.insert(EXPECTED.to_string(), self.expected.render().into_bytes());
        files
    }
}

/// A case as read back from `tests/vectors/v1/dirs`.
struct Committed {
    name: String,
    files: Files,
    expected: Expected,
}

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/v1/dirs")
}

fn read_committed() -> Vec<Committed> {
    let root = vectors_dir();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap_or_else(|e| panic!("{}: {e}", root.display())) {
        let entry = entry.unwrap();
        if !entry.file_type().unwrap().is_dir() {
            continue;
        }
        let mut files = Files::new();
        for f in std::fs::read_dir(entry.path()).unwrap() {
            let f = f.unwrap();
            files.insert(f.file_name().into_string().unwrap(), std::fs::read(f.path()).unwrap());
        }
        let text = files.remove(EXPECTED).unwrap_or_else(|| panic!("{}: no {EXPECTED}", entry.path().display()));
        let expected = Expected::parse(std::str::from_utf8(&text).expect("EXPECTED is text"));
        out.push(Committed { name: entry.file_name().into_string().unwrap(), files, expected });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ---------------------------------------------------------------------------
// The engine, configured so that its bytes do not depend on tuning defaults

const WRITE_BUFFER: usize = 4 << 10;

fn vector_config(durability: Durability) -> Config {
    Config {
        durability,
        write_buffer_bytes: WRITE_BUFFER,
        cache_bytes: 1 << 20,
        compaction: CompactionConfig::m0(WRITE_BUFFER),
        close_budget: CloseBudget::Bounded { max_bytes: 64 << 20, max_duration: None },
        wal_preallocate: false,
        test: TestConfig { background: Background::Inline, db_id: Some(VECTOR_DB_ID) },
        ..Config::default()
    }
}

/// [`vector_config`] with levels and files small enough for a few KiB of data
/// to reach three levels.
fn deep_config() -> Config {
    Config {
        compaction: CompactionConfig {
            level_base_bytes: Some(3 << 10),
            target_file_size: Some(1 << 10),
            ..CompactionConfig::m0(WRITE_BUFFER)
        },
        ..vector_config(Durability::Sync)
    }
}

fn db_path(name: &str) -> PathBuf {
    Path::new(DIR).join(name)
}

fn open_db(disk: &SimVfs, cfg: &Config) -> KernoDb {
    KernoDb::open(disk.as_vfs(), Path::new(DIR), cfg).unwrap_or_else(|e| panic!("open: {e}"))
}

/// The raw files of `image` by name, without `LOCK`.
fn files_of(image: &DirImage) -> Files {
    image
        .files
        .iter()
        .filter(|(p, _)| p.file_name().is_some_and(|n| n != "LOCK"))
        .map(|(p, bytes)| {
            assert_eq!(p.parent(), Some(Path::new(DIR)), "{} is not directly in the database directory", p.display());
            (p.file_name().unwrap().to_str().unwrap().to_string(), bytes.clone())
        })
        .collect()
}

fn image_of(files: &Files) -> DirImage {
    DirImage {
        dirs: [PathBuf::from(DIR)].into(),
        files: files.iter().map(|(n, b)| (db_path(n), b.clone())).collect(),
        unlinked_open: Vec::new(),
    }
}

fn put_batch(key: &[u8], value: &[u8]) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(key, value);
    b
}

/// A random commit over a small key space: mostly puts of 20..120 bytes, some
/// deletes. Every sequence number maps to one entry of `models`.
fn random_batch(rng: &mut Rng, keys: u64) -> WriteBatch {
    let mut b = WriteBatch::new();
    let key = format!("key-{:04}", rng.below(keys));
    if rng.chance(1, 6) {
        b.delete(key);
    } else {
        let len = rng.range(20, 120) as usize;
        b.put(key, rng.bytes(len));
    }
    b
}

/// A database that is being written: the model of every sequence so far.
struct Session {
    disk: SimVfs,
    db: KernoDb,
    /// `models[s]` is the state at sequence `s`.
    models: Vec<Model>,
    rng: Rng,
}

impl Session {
    fn new(disk: SimVfs, cfg: &Config) -> Session {
        let db = open_db(&disk, cfg);
        Session { disk, db, models: vec![Model::new()], rng: Rng::new(WORKLOAD_SEED) }
    }

    /// Resumes on a database that already holds `models`.
    fn resume(disk: SimVfs, cfg: &Config, models: Vec<Model>, rng: Rng) -> Session {
        let db = open_db(&disk, cfg);
        Session { disk, db, models, rng }
    }

    fn seq(&self) -> u64 {
        self.models.len() as u64 - 1
    }

    fn commit(&mut self, batch: &WriteBatch) -> kerno_core::Result<u64> {
        let mut next = self.models.last().unwrap().clone();
        apply(&mut next, batch);
        let r = self.db.commit(batch);
        match &r {
            Ok(seq) => {
                assert_eq!(*seq, self.models.len() as u64, "sequences are consecutive for one writer");
                self.models.push(next);
            }
            // The commit that failed may still have reached the disk: its
            // state is the last entry of `models`.
            Err(_) => self.models.push(next),
        }
        r
    }

    fn commit_ok(&mut self, batch: &WriteBatch) -> u64 {
        self.commit(batch).unwrap_or_else(|e| panic!("commit: {e}"))
    }

    /// `n` random commits over `keys` keys.
    fn run(&mut self, n: u32, keys: u64) {
        for _ in 0..n {
            let b = random_batch(&mut self.rng, keys);
            self.commit_ok(&b);
        }
    }

    /// Commits random batches until the armed crash fires; returns the last
    /// acknowledged sequence.
    fn run_until_crash(&mut self, keys: u64, limit: u32) -> u64 {
        let mut acked = self.seq();
        for _ in 0..limit {
            let b = random_batch(&mut self.rng, keys);
            match self.commit(&b) {
                Ok(seq) => acked = seq,
                Err(e) => {
                    assert!(self.disk.has_crashed(), "commit failed without a crash: {e}");
                    return acked;
                }
            }
        }
        panic!("no crash after {limit} commits");
    }

    fn model(&self) -> &Model {
        self.models.last().unwrap()
    }

    /// Closes cleanly: the case opens to everything committed.
    fn close(self, name: &str) -> Case {
        let (seq, live) = (self.seq(), self.model().clone());
        let seen = read_all(&self.db.snapshot().unwrap()).unwrap();
        assert!(seen == live, "{name}: the database differs from the model before the close");
        self.db.close().unwrap_or_else(|e| panic!("{name}: close: {e}"));
        let image = self.disk.dump();
        assert_clean(name, &image);
        Case {
            name: name.to_string(),
            files: files_of(&image),
            expected: Expected { seq, mode: Mode::Rw, clean: true, live },
        }
    }

    /// Dies without closing (the process is killed; what was written stays)
    /// and returns the image.
    fn kill(self) -> (DirImage, Vec<Model>) {
        let image = self.disk.crash_now(CrashKind::ProcessKill).dump();
        drop(self.db);
        (image, self.models)
    }
}

fn assert_clean(name: &str, image: &DirImage) {
    let manifest = &image.files[&db_path("MANIFEST")];
    let replay = replay_manifest(&mut &manifest[..], |_, _| Ok(())).map_err(|e| e.into_format()).unwrap();
    assert!(replay.clean, "{name}: the MANIFEST is not clean");
    assert!(
        image.files.keys().all(|p| p.extension().is_none_or(|e| e != "wal")),
        "{name}: a clean close leaves no WAL"
    );
}

/// What a read-only open of a crashed `image` recovers, which must be one of
/// the states the session went through and not older than `acked`.
fn recovered_case(name: &str, image: &DirImage, models: &[Model], acked: u64) -> Case {
    let disk = SimVfs::restore(image);
    let cfg = Config { access: AccessMode::ReadOnly, ..vector_config(Durability::Sync) };
    let db = open_db(&disk, &cfg);
    let snap = db.snapshot().unwrap();
    let seq = snap.seq();
    assert!(
        acked <= seq && (seq as usize) < models.len(),
        "{name}: recovered sequence {seq}, {acked} acknowledged, {} assigned",
        models.len() - 1
    );
    let live = read_all(&snap).unwrap();
    assert!(live == models[seq as usize], "{name}: the recovered contents are not the state at sequence {seq}");
    let manifest = &image.files[&db_path("MANIFEST")];
    let clean = replay_manifest(&mut &manifest[..], |_, _| Ok(())).map_err(|e| e.into_format()).unwrap().clean;
    Case { name: name.to_string(), files: files_of(image), expected: Expected { seq, mode: Mode::Rw, clean, live } }
}

fn newest_wal(image: &DirImage) -> (PathBuf, u64) {
    let wal = image.files.keys().filter(|p| p.extension().is_some_and(|e| e == "wal")).max().expect("a WAL").clone();
    let number = wal.file_stem().unwrap().to_str().unwrap().parse().unwrap();
    (wal, number)
}

fn scan_newest_wal(image: &DirImage) -> Result<kerno_storage::format::wal::WalScan, String> {
    let (wal, number) = newest_wal(image);
    let bytes = &image.files[&wal];
    if bytes.len() < HEADER_LEN {
        return Err(format!("{} bytes: no header", bytes.len()));
    }
    let params = WalScanParams { file_number: number, db_id: VECTOR_DB_ID, expected_seq: le_u64(bytes, 64) };
    scan_wal(&mut &bytes[..], &params, |_, _| Ok(())).map_err(|e| match e {
        ReadError::Format(f) => f.to_string(),
        ReadError::Source(never) => match never {},
    })
}

fn frames_of_newest_wal(image: &DirImage) -> Vec<(u64, kerno_storage::format::wal::Frame)> {
    let (wal, number) = newest_wal(image);
    let bytes = &image.files[&wal];
    let params = WalScanParams { file_number: number, db_id: VECTOR_DB_ID, expected_seq: le_u64(bytes, 64) };
    let mut frames = Vec::new();
    scan_wal(&mut &bytes[..], &params, |at, f| {
        frames.push((at, f));
        Ok(())
    })
    .map_err(|e| e.into_format())
    .unwrap();
    frames
}

fn manifest_end(image: &DirImage) -> End {
    let manifest = &image.files[&db_path("MANIFEST")];
    replay_manifest(&mut &manifest[..], |_, _| Ok(())).map_err(|e| e.into_format()).unwrap().end
}

// ---------------------------------------------------------------------------
// Positive cases

fn empty_clean() -> Case {
    Session::new(SimVfs::new(1), &vector_config(Durability::Sync)).close("empty-clean")
}

/// L0, L1 and L2 all hold tables.
fn basic_clean() -> Case {
    let mut s = Session::new(SimVfs::new(2), &deep_config());
    s.run(200, 60);
    let case = s.close("basic-clean");
    let per_level = tables_per_level(&case);
    assert!(per_level[..3].iter().all(|&n| n > 0), "basic-clean: tables per level {per_level:?}");
    case
}

/// How many tables each level holds, from a read-only open of the case.
fn tables_per_level(case: &Case) -> Vec<usize> {
    let disk = SimVfs::restore(&image_of(&case.files));
    let cfg = Config { access: AccessMode::ReadOnly, ..vector_config(Durability::Sync) };
    open_db(&disk, &cfg).level_tables().iter().map(Vec::len).collect()
}

fn tombstone_only_sst() -> Case {
    // A memtable is rotated by the commit that finds it full, so the commit
    // after the last put opens a memtable that receives only deletes, and the
    // commit after those rotates it into a table of nothing but tombstones.
    let full = |s: &Session| stat_of(&s.db, "memtable_bytes") >= WRITE_BUFFER as u64;
    let mut s = Session::new(SimVfs::new(3), &vector_config(Durability::Sync));
    let mut puts = 0u32;
    while !full(&s) {
        s.commit_ok(&put_batch(format!("key-{puts:04}").as_bytes(), &[puts as u8; 40]));
        puts += 1;
    }
    let mut deletes = 0u32;
    loop {
        let mut b = WriteBatch::new();
        b.delete(format!("key-{:04}", deletes % puts));
        s.commit_ok(&b);
        deletes += 1;
        if deletes > 1 && full(&s) {
            break;
        }
    }
    let mut b = WriteBatch::new();
    b.delete("key-0000");
    s.commit_ok(&b);
    s.commit_ok(&put_batch(b"key-9999", b"survivor"));
    let case = s.close("tombstone-only-sst");
    let only_tombstones = case
        .files
        .iter()
        .filter(|(n, _)| is_sst(n))
        .filter(|(n, bytes)| {
            let number = n.trim_end_matches(".sst").parse().unwrap();
            let expect = SstExpect { file_number: number, db_id: VECTOR_DB_ID, meta: None, last_seq: u64::MAX >> 1 };
            let meta = open_sst_from(&mut &bytes[..], &expect).map_err(|e| e.into_format()).unwrap();
            meta.props.point_entries > 0 && meta.props.point_deletes == meta.props.point_entries
        })
        .count();
    assert!(only_tombstones > 0, "tombstone-only-sst: no table holds only tombstones");
    case
}

fn range_delete_clean() -> Case {
    // Two range deletions, one with a put after it in the same batch; the
    // clean close flushes the memtable that holds them, so a table carries
    // the range fragments next to its points.
    let mut s = Session::new(SimVfs::new(5), &vector_config(Durability::Sync));
    for i in 0..60u32 {
        s.commit_ok(&put_batch(format!("key-{i:04}").as_bytes(), &[i as u8; 40]));
    }
    let mut b = WriteBatch::new();
    b.delete_range("key-0010", "key-0030").put("key-0020", "after the range, same batch");
    s.commit_ok(&b);
    let mut b = WriteBatch::new();
    b.delete_range("key-0045", "key-0050");
    s.commit_ok(&b);
    let case = s.close("range-delete-clean");
    let with_fragments = case
        .files
        .iter()
        .filter(|(n, _)| is_sst(n))
        .filter(|(n, bytes)| {
            let number = n.trim_end_matches(".sst").parse().unwrap();
            let expect = SstExpect { file_number: number, db_id: VECTOR_DB_ID, meta: None, last_seq: u64::MAX >> 1 };
            let meta = open_sst_from(&mut &bytes[..], &expect).map_err(|e| e.into_format()).unwrap();
            meta.props.range_fragments > 0
        })
        .count();
    assert!(with_fragments > 0, "range-delete-clean: no table holds range fragments");
    case
}

fn flush_split_l0() -> Case {
    let mut cfg = vector_config(Durability::Sync);
    cfg.compaction.flush_split = true;
    let mut s = Session::new(SimVfs::new(4), &cfg);
    s.run(70, 60);
    s.close("flush-split-l0")
}

/// The first case that `build` produces for a seed of `0..SEEDS`. A power loss
/// persists a random part of the writes that were pending when it hit, so only
/// some seeds leave the shape a vector is about; the search is deterministic.
fn first_seed(name: &str, build: impl Fn(u64) -> Option<Case>) -> Case {
    const SEEDS: u64 = 64;
    (0..SEEDS).find_map(build).unwrap_or_else(|| panic!("{name}: no seed of 0..{SEEDS} leaves the intended shape"))
}

fn is_wal_path(p: &Path) -> bool {
    p.extension().is_some_and(|e| e == "wal")
}

/// A crash right before the `fsync` of a frame: the frame, written but not
/// synced, is torn, and the newest WAL ends in a torn tail.
fn dirty_wal_torn_tail() -> Case {
    first_seed("dirty-wal-torn-tail", |seed| {
        let disk = SimVfs::new(seed);
        disk.set_tear_unit(1);
        let mut s = Session::new(disk, &vector_config(Durability::Sync));
        s.run(60, 40);
        s.disk.crash_when(|op| op.kind == OpKind::SyncData && is_wal_path(&op.path), 1, CrashKind::PowerLoss);
        let acked = s.run_until_crash(40, 100);
        let image = s.disk.take_crash_image().expect("the crash happened").dump();
        let scan = scan_newest_wal(&image).ok()?;
        (scan.end == End::Torn).then(|| recovered_case("dirty-wal-torn-tail", &image, &s.models, acked))
    })
}

/// A crash right before the first `fsync` of the next WAL, whose header is
/// still pending: the newest log has a broken header and the one before it
/// holds the live frames.
fn dirty_torn_newest_header() -> Case {
    first_seed("dirty-torn-newest-header", |seed| {
        let disk = SimVfs::new(seed);
        disk.set_tear_unit(1);
        let mut s = Session::new(disk, &vector_config(Durability::Sync));
        s.run(20, 40);
        let (first_wal, _) = newest_wal(&s.disk.dump());
        s.disk.crash_when(
            move |op| op.kind == OpKind::SyncData && is_wal_path(&op.path) && op.path != first_wal,
            1,
            CrashKind::PowerLoss,
        );
        let acked = s.run_until_crash(40, 200);
        let image = s.disk.take_crash_image().expect("the crash happened").dump();
        let (wal, _) = newest_wal(&image);
        let broken = scan_newest_wal(&image).is_err() || image.files[&wal].len() < HEADER_LEN;
        broken.then(|| recovered_case("dirty-torn-newest-header", &image, &s.models, acked))
    })
}

/// REC-10c: a torn WAL, then a session start that died after creating its
/// log and before the `SESSION_OPEN` that names it.
fn dirty_chained_session_start() -> Case {
    let torn = dirty_wal_torn_tail();
    let image = image_of(&torn.files);
    let disk = SimVfs::restore_seeded(&image, 7);
    disk.set_tear_unit(1);
    disk.crash_when(
        |op| op.phase == "session" && op.path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("MANIFEST")),
        1,
        CrashKind::PowerLoss,
    );
    let r = KernoDb::open(disk.as_vfs(), Path::new(DIR), &vector_config(Durability::Sync));
    assert!(r.is_err() && disk.has_crashed(), "dirty-chained-session-start: the open must die in the session start");
    let chained = disk.take_crash_image().expect("the crash happened").dump();
    let wals = chained.files.keys().filter(|p| p.extension().is_some_and(|e| e == "wal")).count();
    assert!(wals >= 2, "dirty-chained-session-start: a second WAL must exist ({wals})");
    // Whatever the first recovery found, the chained one finds the same.
    let models = vec![torn.expected.live.clone(); torn.expected.seq as usize + 1];
    let mut case = recovered_case("dirty-chained-session-start", &chained, &models, torn.expected.seq);
    assert_eq!(case.expected.seq, torn.expected.seq, "the chained crash changed the recovered state");
    case.name = "dirty-chained-session-start".into();
    case
}

/// A power loss right before the `fsync` of a `FLUSH` record: the record,
/// written but not synced, is torn.
fn dirty_manifest_torn_tail() -> Case {
    first_seed("dirty-manifest-torn-tail", |seed| {
        let disk = SimVfs::new(seed);
        disk.set_tear_unit(1);
        let mut s = Session::new(disk, &vector_config(Durability::Sync));
        s.run(10, 40);
        s.disk.crash_when(
            |op| op.phase == "flush" && op.path.ends_with("MANIFEST") && op.kind == OpKind::SyncData,
            1,
            CrashKind::PowerLoss,
        );
        let acked = s.run_until_crash(40, 200);
        let image = s.disk.take_crash_image().expect("the crash happened").dump();
        (manifest_end(&image) == End::Torn)
            .then(|| recovered_case("dirty-manifest-torn-tail", &image, &s.models, acked))
    })
}

/// A checkpoint (the close of the first session) followed by the edits of a
/// second session that was killed.
fn checkpoint_plus_edits() -> Case {
    let cfg = vector_config(Durability::Sync);
    let mut s = Session::new(SimVfs::new(9), &cfg);
    s.run(30, 40);
    let Session { disk, db, models, rng } = s;
    db.close().unwrap();
    let mut s = Session::resume(SimVfs::restore_seeded(&disk.dump(), 9), &cfg, models, rng);
    s.run(40, 40);
    let (live, seq) = (s.model().clone(), s.seq());
    let (killed, _) = s.kill();
    let manifest = &killed.files[&db_path("MANIFEST")];
    let mut records = 0;
    let replay = replay_manifest(&mut &manifest[..], |_, _| {
        records += 1;
        Ok(())
    })
    .map_err(|e| e.into_format())
    .unwrap();
    assert!(!replay.clean && records >= 3, "checkpoint-plus-edits: {records} records, clean {}", replay.clean);
    Case {
        name: "checkpoint-plus-edits".into(),
        files: files_of(&killed),
        expected: Expected { seq, mode: Mode::Rw, clean: false, live },
    }
}

/// A value of exactly `MAX_VALUE_LEN` bytes.
fn large_value_1mib() -> Case {
    let mut s = Session::new(SimVfs::new(10), &vector_config(Durability::Sync));
    s.commit_ok(&put_batch(b"a-small", b"before"));
    let value: Vec<u8> = (0..MAX_VALUE_LEN).map(|i| (i % 251) as u8).collect();
    s.commit_ok(&put_batch(b"big", &value));
    s.commit_ok(&put_batch(b"z-small", b"after"));
    s.close("large-value-1mib")
}

// ---- group commit ---------------------------------------------------------

const WAIT: Duration = Duration::from_secs(60);

/// Holds the thread that calls [`hold`](Gate::hold) until [`release`](Gate::release).
#[derive(Default)]
struct Gate {
    /// (a thread is held, released)
    state: Mutex<(bool, bool)>,
    cv: Condvar,
}

impl Gate {
    fn hold(&self) {
        let mut st = self.state.lock().unwrap();
        st.0 = true;
        self.cv.notify_all();
        while !st.1 {
            let (next, t) = self.cv.wait_timeout(st, WAIT).unwrap();
            assert!(!t.timed_out(), "the held thread was never released");
            st = next;
        }
    }

    fn wait_held(&self) {
        let mut st = self.state.lock().unwrap();
        while !st.0 {
            let (next, t) = self.cv.wait_timeout(st, WAIT).unwrap();
            assert!(!t.timed_out(), "no thread reached the gate");
            st = next;
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.cv.notify_all();
    }
}

fn is_wal_frame_write(op: &HookOp<'_>) -> bool {
    op.kind == HookOpKind::WriteAt
        && op.point == HookPoint::Before
        && op.path.extension().is_some_and(|e| e == "wal")
        && op.offset >= HEADER_LEN as u64
}

fn encoded_len(b: &WriteBatch) -> usize {
    canonicalize(b.ops().iter().map(|op| match op {
        Op::Put { key, value } => CommitOp::Put { key, value },
        Op::Delete { key } => CommitOp::Delete { key },
        Op::DeleteRange { start, end } => CommitOp::DeleteRange { start, end },
    }))
    .encoded_len()
}

fn stat_of(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

fn queued_bytes_max(db: &KernoDb) -> usize {
    stat_of(db, "queued_bytes_max") as usize
}

/// A group frame of three commits: writer 1 leads a frame alone while writers
/// 2, 3 and 4 queue behind it, one at a time and in this order, so that the
/// sequence numbers do not depend on thread scheduling; they form one frame.
fn dirty_group_frame() -> Case {
    let disk = SimVfs::new(11);
    let gate = Arc::new(Gate::default());
    let hold = gate.clone();
    let frames = Arc::new(AtomicUsize::new(0));
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        // The second frame of the log: its leader waits for the followers.
        if is_wal_frame_write(op) && frames.fetch_add(1, Ordering::SeqCst) == 1 {
            hold.hold();
        }
        HookAction::Proceed
    });
    let db = Arc::new(KernoDb::open(vfs, Path::new(DIR), &vector_config(Durability::Sync)).unwrap());
    let mut model = Model::new();
    let first = put_batch(b"w000", b"first");
    assert_eq!(db.commit(&first).unwrap(), 1);
    apply(&mut model, &first);

    let mut leader_batch = put_batch(b"w001", &[1; 100]);
    leader_batch.put(b"w001-b", [2; 30]);
    let mut delete = WriteBatch::new();
    delete.delete(b"w000");
    delete.put(b"w002", [3; 60]);
    let batches = [leader_batch, put_batch(b"w003", &[4; 80]), delete, put_batch(b"w004", &[5; 10])];
    let mut handles = Vec::new();
    let mut queued = 0;
    for (i, batch) in batches.iter().enumerate() {
        let (writer, b) = (db.clone(), batch.clone());
        handles.push(std::thread::spawn(move || writer.commit(&b)));
        queued += encoded_len(batch);
        if i == 0 {
            gate.wait_held();
        }
        let deadline = Instant::now() + WAIT;
        while queued_bytes_max(&db) < queued {
            assert!(Instant::now() < deadline, "writer {i} did not queue");
            std::thread::yield_now();
        }
    }
    gate.release();
    for (i, h) in handles.into_iter().enumerate() {
        let seq = h.join().unwrap().unwrap();
        assert_eq!(seq, 2 + i as u64, "commits are numbered in the order they queued");
        apply(&mut model, &batches[i]);
    }
    let stats: BTreeMap<_, _> = db.stats().into_iter().collect();
    assert_eq!((stats["commit_groups"], stats["group_commits_max"]), (3, 3), "dirty-group-frame: {stats:?}");
    let image = disk.crash_now(CrashKind::ProcessKill).dump();
    drop(db);
    let frames = frames_of_newest_wal(&image);
    assert_eq!(frames.iter().map(|(_, f)| f.commits.len()).collect::<Vec<_>>(), [1, 1, 3]);
    Case {
        name: "dirty-group-frame".into(),
        files: files_of(&image),
        expected: Expected { seq: 5, mode: Mode::Rw, clean: false, live: model },
    }
}

/// `Buffered` commits around an explicit `sync()`: the frames written after it
/// carry the synced prefix in `synced_len`, and the last ones are not synced.
fn dirty_buffered_synced_len() -> Case {
    let mut s = Session::new(SimVfs::new(12), &vector_config(Durability::Buffered));
    s.run(6, 20);
    assert_eq!(s.db.sync().unwrap(), 6);
    s.run(4, 20);
    let (image, models) = s.kill();
    let frames = frames_of_newest_wal(&image);
    assert!(
        frames.iter().any(|(_, f)| f.synced_len > HEADER_LEN as u64)
            && frames.iter().any(|(_, f)| f.synced_len == HEADER_LEN as u64),
        "dirty-buffered-synced-len: {:?}",
        frames.iter().map(|(_, f)| f.synced_len).collect::<Vec<_>>()
    );
    recovered_case("dirty-buffered-synced-len", &image, &models, 10)
}

/// A killed session whose only WAL holds exactly one `Sync` commit: the frame
/// carries `synced_len = HEADER_LEN` because the writer syncs the header
/// before it accepts the first frame (WAL-R6).
fn dirty_wal_one_frame() -> Case {
    let mut s = Session::new(SimVfs::new(17), &vector_config(Durability::Sync));
    s.commit_ok(&put_batch(b"only", b"frame"));
    let (image, models) = s.kill();
    let frames = frames_of_newest_wal(&image);
    assert_eq!(frames.len(), 1, "dirty-wal-one-frame");
    assert_eq!(frames[0].1.synced_len, HEADER_LEN as u64, "the header is synced before the first frame");
    recovered_case("dirty-wal-one-frame", &image, &models, 1)
}

// ---------------------------------------------------------------------------
// Negative cases, derived from positive ones

fn with_header_patch(
    base: &Case,
    name: &str,
    file_matches: impl Fn(&str) -> bool,
    class: Class,
    patch: impl Fn(&mut [u8]) + Copy,
) -> Case {
    let mut files = base.files.clone();
    let victim = files.keys().find(|n| file_matches(n)).unwrap_or_else(|| panic!("{name}: no file to patch")).clone();
    patch_header(files.get_mut(&victim).unwrap(), patch);
    reject(name, files, class)
}

fn reject(name: &str, files: Files, class: Class) -> Case {
    Case {
        name: name.to_string(),
        files,
        expected: Expected { seq: 0, mode: Mode::Reject(class), clean: false, live: Model::new() },
    }
}

fn is_manifest(n: &str) -> bool {
    n == "MANIFEST"
}
fn is_wal(n: &str) -> bool {
    n.ends_with(".wal")
}
fn is_sst(n: &str) -> bool {
    n.ends_with(".sst")
}

fn or_u64(h: &mut [u8], at: usize, bits: u64) {
    let v = le_u64(h, at) | bits;
    h[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

const COMPAT_AT: usize = 16;
const RO_COMPAT_AT: usize = 24;
const INCOMPAT_AT: usize = 32;
/// An `incompat` bit v1 does not define (v1.md §4.1 reserves 1..=62).
const UNKNOWN_INCOMPAT: u64 = 1 << 20;

fn negative_header_cases(clean: &Case, dirty: &Case) -> Vec<Case> {
    let mut out = Vec::new();
    for (suffix, matches) in [("man", is_manifest as fn(&str) -> bool), ("wal", is_wal), ("sst", is_sst)] {
        let base = if suffix == "wal" { dirty } else { clean };
        out.push(with_header_patch(base, &format!("future-version-{suffix}"), matches, Class::Incompatible, |h| {
            h[8..12].copy_from_slice(&2u32.to_le_bytes());
        }));
        out.push(with_header_patch(base, &format!("unknown-incompat-{suffix}"), matches, Class::Incompatible, |h| {
            or_u64(h, INCOMPAT_AT, UNKNOWN_INCOMPAT);
        }));
    }
    out.push(with_header_patch(clean, "aead-bit-set", is_sst, Class::Incompatible, |h| {
        or_u64(h, INCOMPAT_AT, features::AEAD);
    }));
    // A database made by the pre-freeze draft: the bit is refused by name.
    out.push(draft_v1(clean));
    out
}

/// Every file as the draft wrote it (`incompat` bit 63): `Incompatible`.
fn draft_v1(clean: &Case) -> Case {
    let mut files = clean.files.clone();
    for (name, bytes) in files.iter_mut() {
        if is_manifest(name) || is_wal(name) || is_sst(name) {
            patch_header(bytes, |h| or_u64(h, INCOMPAT_AT, features::DRAFT));
        }
    }
    reject("draft-v1", files, Class::Incompatible)
}

/// Opens read-only whatever the request says; the contents are those of `base`.
fn unknown_ro_compat(base: &Case) -> Case {
    let mut files = base.files.clone();
    patch_header(files.get_mut("MANIFEST").unwrap(), |h| or_u64(h, RO_COMPAT_AT, 1 << 7));
    Case { name: "unknown-ro-compat".into(), files, expected: Expected { mode: Mode::Ro, ..base.expected.clone() } }
}

/// Opens read-write; the unknown `compat` bit is ignored and does not survive
/// the checkpoint of the next close.
fn unknown_compat(base: &Case) -> Case {
    let mut files = base.files.clone();
    patch_header(files.get_mut("MANIFEST").unwrap(), |h| or_u64(h, COMPAT_AT, 1 << 5));
    Case { name: "unknown-compat".into(), files, expected: base.expected.clone() }
}

fn foreign_db_id_sst(clean: &Case) -> Case {
    with_header_patch(clean, "foreign-db-id-sst", is_sst, Class::Corruption, |h| h[40] ^= 0xff)
}

fn renamed_sst(clean: &Case) -> Case {
    let mut files = clean.files.clone();
    let name = files.keys().find(|n| is_sst(n)).unwrap().clone();
    let bytes = files.remove(&name).unwrap();
    files.insert("000099.sst".into(), bytes);
    reject("renamed-sst", files, Class::Corruption)
}

fn missing_log_wal(dirty: &Case) -> Case {
    let mut files = dirty.files.clone();
    let wal = files.keys().filter(|n| is_wal(n)).min().unwrap().clone();
    files.remove(&wal);
    reject("missing-log-wal", files, Class::Corruption)
}

/// After a clean close, a log numbered `next_file` or more may only be empty
/// (REC-7): this one has a frame.
fn bytes_after_clean(clean: &Case) -> Case {
    let manifest = &clean.files["MANIFEST"];
    let replay = replay_manifest(&mut &manifest[..], |_, _| Ok(())).map_err(|e| e.into_format()).unwrap();
    let (next_file, last_seq) = (replay.state.next_file, replay.state.last_seq);
    let mut files = clean.files.clone();
    files.insert(
        format!("{next_file:06}.wal"),
        wal_with_one_frame(next_file, last_seq + 1, &put_batch(b"late", b"commit")),
    );
    reject("bytes-after-clean", files, Class::Corruption)
}

/// A WAL of `number` with the commit `batch` as its only frame.
fn wal_with_one_frame(number: u64, seq: u64, batch: &WriteBatch) -> Vec<u8> {
    let canonical = canonicalize(batch.ops().iter().map(|op| match op {
        Op::Put { key, value } => CommitOp::Put { key, value },
        Op::Delete { key } => CommitOp::Delete { key },
        Op::DeleteRange { start, end } => CommitOp::DeleteRange { start, end },
    }));
    let mut out = Header::wal(VECTOR_DB_ID, number, seq).encode().to_vec();
    let frame = Frame { first_seq: seq, synced_len: 0, commits: vec![canonical.to_owned_commit()] };
    out.extend(encode_frame(number, &frame, false));
    out
}

/// A one-frame WAL whose header is damaged: the frame is valid and was written
/// after a durable header, so this is media damage, not a torn creation
/// (WAL-R6), and the acknowledged commit must not be dropped silently.
fn wal_one_frame_broken_header(one_frame: &Case) -> Case {
    let mut files = one_frame.files.clone();
    let wal = files.keys().filter(|n| is_wal(n)).max().unwrap().clone();
    files.get_mut(&wal).unwrap()[20] ^= 0x01;
    reject("wal-one-frame-broken-header", files, Class::Corruption)
}

/// Damage in the middle of the log followed by a frame that was written after
/// the damaged bytes were durable (WAL-R5).
fn wal_mid_damage_synced(dirty: &Case) -> Case {
    let mut files = dirty.files.clone();
    let wal = files.keys().filter(|n| is_wal(n)).max().unwrap().clone();
    let image = image_of(&files);
    let frames = frames_of_newest_wal(&image);
    assert!(frames.len() >= 3 && frames[2].1.synced_len > frames[1].0, "wal-mid-damage-synced needs synced frames");
    // A byte of the payload of the second frame.
    files.get_mut(&wal).unwrap()[frames[1].0 as usize + 40] ^= 0x01;
    reject("wal-mid-damage-synced", files, Class::Corruption)
}

// ---- other formats ---------------------------------------------------------

fn m0_spike_lsm() -> Case {
    use spike_lsm::LsmEngine;
    let disk = SimVfs::new(13);
    let opts = kerno_core::contract::Options::small_for_tests();
    let db = LsmEngine::open(disk.as_vfs(), Path::new(DIR), &opts).unwrap();
    for i in 0..20u32 {
        db.commit(&put_batch(format!("key-{i:04}").as_bytes(), &[i as u8; 50])).unwrap();
    }
    db.close().unwrap();
    reject("m0-spike-lsm", files_of(&disk.dump()), Class::Incompatible)
}

fn m0_spike_btree() -> Case {
    use spike_btree::BTreeEngine;
    let disk = SimVfs::new(14);
    let opts = kerno_core::contract::Options::small_for_tests();
    let db = BTreeEngine::open(disk.as_vfs(), Path::new(DIR), &opts).unwrap();
    for i in 0..20u32 {
        db.commit(&put_batch(format!("key-{i:04}").as_bytes(), &[i as u8; 50])).unwrap();
    }
    db.close().unwrap();
    reject("m0-spike-btree", files_of(&disk.dump()), Class::InvalidArgument)
}

fn reference_log() -> Case {
    use kerno_testkit::reference::LogEngine;
    let disk = SimVfs::new(15);
    let opts = kerno_core::contract::Options::small_for_tests();
    let db = LogEngine::open(disk.as_vfs(), Path::new(DIR), &opts).unwrap();
    for i in 0..20u32 {
        db.commit(&put_batch(format!("key-{i:04}").as_bytes(), &[i as u8; 50])).unwrap();
    }
    db.close().unwrap();
    reject("reference-log", files_of(&disk.dump()), Class::InvalidArgument)
}

/// Every case, in the order of v1.md §13.2.
fn all_cases() -> Vec<Case> {
    let basic = basic_clean();
    let dirty = dirty_wal_torn_tail();
    // The base of the WAL-R5 vector: a log with synced frames, killed.
    let synced = dirty_wal_with_synced_frames();
    let one_frame = dirty_wal_one_frame();
    let mut cases = vec![
        empty_clean(),
        basic.clone(),
        range_delete_clean(),
        tombstone_only_sst(),
        flush_split_l0(),
        dirty,
        dirty_torn_newest_header(),
        dirty_chained_session_start(),
        dirty_manifest_torn_tail(),
        checkpoint_plus_edits(),
        large_value_1mib(),
        dirty_group_frame(),
        dirty_buffered_synced_len(),
        one_frame.clone(),
    ];
    cases.extend(negative_header_cases(&basic, &synced));
    cases.push(unknown_ro_compat(&basic));
    cases.push(unknown_compat(&basic));
    cases.push(m0_spike_lsm());
    cases.push(m0_spike_btree());
    cases.push(reference_log());
    cases.push(foreign_db_id_sst(&basic));
    cases.push(renamed_sst(&basic));
    cases.push(missing_log_wal(&synced));
    cases.push(bytes_after_clean(&basic));
    cases.push(wal_mid_damage_synced(&synced));
    cases.push(wal_one_frame_broken_header(&one_frame));
    cases
}

/// A killed session with flushed tables and a log of at least four `Sync`
/// frames.
fn dirty_wal_with_synced_frames() -> Case {
    let mut s = Session::new(SimVfs::new(16), &vector_config(Durability::Sync));
    s.run(60, 40);
    while frames_of_newest_wal(&s.disk.dump()).len() < 4 {
        s.run(1, 40);
    }
    let acked = s.seq();
    let (image, models) = s.kill();
    recovered_case("dirty-synced-base", &image, &models, acked)
}

// ---------------------------------------------------------------------------
// Regeneration and the guard

/// The cases v1.md §13.2 lists
const REQUIRED: &[&str] = &[
    "empty-clean",
    "basic-clean",
    "range-delete-clean",
    "tombstone-only-sst",
    "flush-split-l0",
    "dirty-wal-torn-tail",
    "dirty-torn-newest-header",
    "dirty-chained-session-start",
    "dirty-manifest-torn-tail",
    "checkpoint-plus-edits",
    "large-value-1mib",
    "dirty-group-frame",
    "dirty-buffered-synced-len",
    "future-version-man",
    "future-version-wal",
    "future-version-sst",
    "unknown-incompat-man",
    "unknown-incompat-wal",
    "unknown-incompat-sst",
    "aead-bit-set",
    "draft-v1",
    "unknown-ro-compat",
    "unknown-compat",
    "m0-spike-lsm",
    "m0-spike-btree",
    "reference-log",
    "foreign-db-id-sst",
    "renamed-sst",
    "missing-log-wal",
    "bytes-after-clean",
    "wal-mid-damage-synced",
];

/// Writes every case under `tests/vectors/v1/dirs/`, replacing what is there.
/// Run it only for a deliberate change of the format (v1.md §15):
///
/// ```text
/// cargo test --release -p kerno-storage --test dir_vectors -- --ignored regenerate_v1_vectors
/// ```
#[test]
#[ignore = "rewrites the committed vectors; see v1.md §15"]
fn regenerate_v1_vectors() {
    let root = vectors_dir();
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    for case in all_cases() {
        let dir = root.join(&case.name);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in case.committed_form() {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
    }
}

/// The first difference between two sets of files, if any.
fn files_difference(want: &Files, got: &Files) -> Option<String> {
    for name in want.keys().chain(got.keys()) {
        match (want.get(name), got.get(name)) {
            (Some(_), None) => return Some(format!("{name} is missing")),
            (None, Some(_)) => return Some(format!("{name} is extra")),
            (Some(a), Some(b)) if a != b => {
                return Some(match a.iter().zip(b).position(|(x, y)| x != y) {
                    Some(at) => format!("{name}: byte {at} differs ({:#04x} vs {:#04x})", a[at], b[at]),
                    None => format!("{name}: length {} vs {}", a.len(), b.len()),
                });
            }
            _ => {}
        }
    }
    None
}

/// Regenerating every case in memory gives the committed bytes: any change of
/// a byte the engine writes fails here until the vectors are regenerated
/// deliberately (v1.md §13.3, §15).
#[test]
fn dir_vectors_are_reproducible() {
    let committed = read_committed();
    let generated = all_cases();
    let names = |v: Vec<&str>| {
        let mut v: Vec<String> = v.into_iter().map(String::from).collect();
        v.sort();
        v
    };
    assert_eq!(
        names(generated.iter().map(|c| c.name.as_str()).collect()),
        names(committed.iter().map(|c| c.name.as_str()).collect()),
        "the generated and the committed cases differ"
    );
    let missing: Vec<_> = REQUIRED.iter().filter(|n| !generated.iter().any(|c| c.name == **n)).collect();
    assert!(missing.is_empty(), "cases of v1.md §13.2 that are not generated: {missing:?}");
    for case in &generated {
        let on_disk = committed.iter().find(|c| c.name == case.name).unwrap();
        let mut on_disk_files = on_disk.files.clone();
        on_disk_files.insert(EXPECTED.to_string(), on_disk.expected.render().into_bytes());
        let want = case.committed_form();
        if let Some(d) = files_difference(&want, &read_raw(&case.name)) {
            panic!(
                "vector {} changed: {d}. A byte of format v1 moved: follow v1.md §15, then run \
                 `cargo test --release -p kerno-storage --test dir_vectors -- --ignored regenerate_v1_vectors`",
                case.name
            );
        }
        assert_eq!(on_disk_files, want, "{}: EXPECTED is not in its rendered form", case.name);
    }
}

/// The committed files of one case exactly as they are on disk.
fn read_raw(name: &str) -> Files {
    let dir = vectors_dir().join(name);
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|f| {
            let f = f.unwrap();
            (f.file_name().into_string().unwrap(), std::fs::read(f.path()).unwrap())
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Loaders: a case copied into a SimVfs and into a StdVfs directory

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Sim,
    Std,
}

const TARGETS: [Target; 2] = [Target::Sim, Target::Std];

/// A case in a directory of some `Vfs`; the directory of a `StdVfs` is
/// removed when this is dropped.
struct Loaded {
    vfs: Arc<dyn Vfs>,
    dir: PathBuf,
    remove_on_drop: bool,
}

impl Drop for Loaded {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// Copies `files` into a new directory: write, `sync_data`, then `sync_dir`.
fn load(target: Target, files: &Files) -> Loaded {
    let (vfs, dir, remove_on_drop) = match target {
        Target::Sim => (SimVfs::new(0).as_vfs(), PathBuf::from(DIR), false),
        Target::Std => {
            let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("kerno-dir-vectors-{}-{n}", std::process::id()));
            (StdVfs::shared(), dir, true)
        }
    };
    let loaded = Loaded { vfs, dir, remove_on_drop };
    let vfs = &loaded.vfs;
    vfs.create_dir_all(&loaded.dir).unwrap();
    for (name, bytes) in files {
        let file = vfs.open(&loaded.dir.join(name), OpenMode::CreateNew).unwrap();
        file.write_at(bytes, 0).unwrap();
        file.sync_data().unwrap();
    }
    vfs.sync_dir(&loaded.dir).unwrap();
    loaded
}

impl Loaded {
    /// The files now in the directory, without `LOCK`.
    fn files(&self) -> Files {
        let mut out = Files::new();
        for name in self.vfs.list(&self.dir).unwrap() {
            if name == "LOCK" {
                continue;
            }
            let f = self.vfs.open(&self.dir.join(&name), OpenMode::Existing).unwrap();
            let mut bytes = vec![0; f.len().unwrap() as usize];
            f.read_at(&mut bytes, 0).unwrap();
            out.insert(name, bytes);
        }
        out
    }

    fn open(&self, access: AccessMode) -> kerno_core::Result<KernoDb> {
        KernoDb::open(self.vfs.clone(), &self.dir, &Config { access, ..vector_config(Durability::Sync) })
    }

    fn verify(&self) -> kerno_core::Result<kerno_storage::VerifyReport> {
        verify(self.vfs.clone(), &self.dir, &VerifyOptions::default())
    }
}

fn is_positive(case: &Committed) -> bool {
    !matches!(case.expected.mode, Mode::Reject(_))
}

/// The handle shows the sequence and exactly the contents of the case.
fn assert_opens_to(ctx: &str, db: &KernoDb, expected: &Expected) {
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.seq(), expected.seq, "{ctx}: sequence");
    let seen = read_all(&snap).unwrap();
    assert!(seen == expected.live, "{ctx}: {}", kerno_testkit::harness::diff(&expected.live, &seen));
    for (k, v) in &expected.live {
        assert_eq!(snap.get(k).unwrap().as_ref(), Some(v), "{ctx}: get({})", hex(k));
    }
    assert_eq!(snap.get(b"no-such-key").unwrap(), None, "{ctx}: a key that was never written");
}

fn assert_rejected(ctx: &str, r: kerno_core::Result<impl Sized>, class: Class) {
    match r {
        Ok(_) => panic!("{ctx}: opened, expected {}", class.name()),
        Err(e) => assert_eq!(Class::of(&e), Some(class), "{ctx}: expected {}, got {e:?}", class.name()),
    }
}

fn assert_untouched(ctx: &str, before: &Files, loaded: &Loaded) {
    if let Some(d) = files_difference(before, &loaded.files()) {
        panic!("{ctx}: the directory changed: {d}");
    }
}

fn open_exact(target: Target) {
    let cases = read_committed();
    assert!(cases.len() >= REQUIRED.len());
    for case in &cases {
        let ctx = format!("{} on {target:?}", case.name);
        let loaded = load(target, &case.files);
        match case.expected.mode {
            Mode::Reject(class) => {
                assert_rejected(&ctx, loaded.open(AccessMode::ReadWrite), class);
                assert_untouched(&ctx, &case.files, &loaded);
            }
            Mode::Rw => {
                let db = loaded.open(AccessMode::ReadWrite).unwrap_or_else(|e| panic!("{ctx}: {e}"));
                assert!(db.is_read_only().is_none(), "{ctx}: opened read-only");
                assert_opens_to(&ctx, &db, &case.expected);
                // The recovered database takes writes, and they survive a close.
                db.commit(&put_batch(b"zz-probe", b"written after the open")).unwrap();
                db.close().unwrap_or_else(|e| panic!("{ctx}: close: {e}"));
                let db = loaded.open(AccessMode::ReadWrite).unwrap_or_else(|e| panic!("{ctx}: reopen: {e}"));
                let mut live = case.expected.live.clone();
                live.insert(b"zz-probe".to_vec(), b"written after the open".to_vec());
                assert_opens_to(&ctx, &db, &Expected { seq: case.expected.seq + 1, live, ..case.expected.clone() });
                drop(db);
                let report = loaded.verify().unwrap();
                assert!(report.is_ok(), "{ctx}: verify after the writes: {:?}", report.issues);
            }
            Mode::Ro => {
                let db = loaded.open(AccessMode::ReadWrite).unwrap_or_else(|e| panic!("{ctx}: {e}"));
                assert!(
                    matches!(db.is_read_only(), Some(kerno_storage::ReadOnlyReason::UnknownRoCompat { .. })),
                    "{ctx}: {:?}",
                    db.is_read_only()
                );
                assert_opens_to(&ctx, &db, &case.expected);
                let r = db.commit(&put_batch(b"k", b"v"));
                assert!(matches!(r, Err(Error::ReadOnly(_))), "{ctx}: a write through a read-only handle: {r:?}");
                drop(db);
                assert_untouched(&ctx, &case.files, &loaded);
            }
        }
        if case.name == "unknown-compat" {
            // The unknown `compat` bit goes away with the checkpoint of the close.
            let db = loaded.open(AccessMode::ReadWrite).unwrap();
            db.close().unwrap();
            let manifest = &loaded.files()["MANIFEST"];
            assert_eq!(le_u64(manifest, COMPAT_AT), 0, "{ctx}: the bit survived the checkpoint");
        }
    }
}

#[test]
fn dir_vectors_open_exact_sim() {
    open_exact(Target::Sim);
}

#[test]
fn dir_vectors_open_exact_std() {
    open_exact(Target::Std);
}

/// Every case that opens, opens read-only with the same sequence and contents,
/// and the open writes nothing (RO-1).
#[test]
fn dir_vectors_open_read_only() {
    for target in TARGETS {
        for case in read_committed().iter().filter(|c| is_positive(c)) {
            let ctx = format!("{} on {target:?}", case.name);
            let loaded = load(target, &case.files);
            let db = loaded.open(AccessMode::ReadOnly).unwrap_or_else(|e| panic!("{ctx}: {e}"));
            assert_eq!(db.is_read_only(), Some(kerno_storage::ReadOnlyReason::Requested), "{ctx}");
            assert_opens_to(&ctx, &db, &case.expected);
            let r = db.commit(&put_batch(b"k", b"v"));
            assert!(matches!(r, Err(Error::ReadOnly(_))), "{ctx}: a write through a read-only handle: {r:?}");
            drop(db);
            assert_untouched(&ctx, &case.files, &loaded);
        }
    }
}

/// `verify()` finds nothing wrong in a case that opens, reports its state, and
/// agrees with the open about the cases that do not.
#[test]
fn dir_vectors_verify_clean() {
    for target in TARGETS {
        for case in read_committed() {
            let ctx = format!("{} on {target:?}", case.name);
            let loaded = load(target, &case.files);
            let report = loaded.verify();
            match case.expected.mode {
                Mode::Rw | Mode::Ro => {
                    let report = report.unwrap_or_else(|e| panic!("{ctx}: {e}"));
                    assert!(report.is_ok(), "{ctx}: {:?}", report.issues);
                    assert_eq!(report.clean, case.expected.clean, "{ctx}: clean");
                }
                Mode::Reject(Class::Corruption) => {
                    let report = report.unwrap_or_else(|e| panic!("{ctx}: {e}"));
                    assert!(!report.is_ok(), "{ctx}: verify found nothing in a corrupt database");
                }
                Mode::Reject(class) => assert_rejected(&ctx, report, class),
            }
            assert_untouched(&ctx, &case.files, &loaded);
        }
    }
}

/// A rejected directory is left byte for byte as it was, by a read-write open,
/// a read-only open and `verify()` (`LOCK` aside).
#[test]
fn dir_vectors_negative_untouched() {
    let mut rejected = 0;
    for target in TARGETS {
        for case in read_committed() {
            let Mode::Reject(class) = case.expected.mode else { continue };
            rejected += 1;
            let ctx = format!("{} on {target:?}", case.name);
            let loaded = load(target, &case.files);
            for access in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
                assert_rejected(&format!("{ctx} ({access:?})"), loaded.open(access), class);
                assert_untouched(&ctx, &case.files, &loaded);
            }
            let _ = loaded.verify();
            assert_untouched(&ctx, &case.files, &loaded);
        }
    }
    assert!(rejected >= 2 * 15, "{rejected} negative runs");
}

// ---------------------------------------------------------------------------
// Exhaustive damage (v1.md §13.4)

/// Files up to this size get every bit flipped and every truncation; a larger
/// file gets them in its first and last bytes (headers, footers, meta blocks)
/// and one every `STRIDE` bytes of the rest.
const EXHAUSTIVE_LIMIT: usize = 64 << 10;
const HEAD: usize = 512;
const TAIL: usize = 4096;
const STRIDE: usize = 1021;

fn probed_offsets(len: usize) -> Vec<usize> {
    if len <= EXHAUSTIVE_LIMIT {
        return (0..len).collect();
    }
    (0..len).filter(|&i| i < HEAD || i >= len - TAIL || i % STRIDE == 0).collect()
}

#[derive(Debug)]
enum Outcome {
    /// `Corruption` or `Io`.
    Reported,
    /// The exact contents of the case, no error.
    Exact,
    /// Anything else: a wrong answer or the wrong class of error.
    Wrong(String),
}

fn outcome(image: &DirImage, expected: &Expected) -> Outcome {
    let disk = SimVfs::restore(image);
    let cfg = Config { access: AccessMode::ReadOnly, ..vector_config(Durability::Sync) };
    let db = match KernoDb::open(disk.as_vfs(), Path::new(DIR), &cfg) {
        Ok(db) => db,
        Err(e) => return reported_or_wrong(e),
    };
    let snap = match db.snapshot() {
        Ok(s) => s,
        Err(e) => return reported_or_wrong(e),
    };
    match read_all(&snap) {
        Err(e) => reported_or_wrong(e),
        Ok(seen) if snap.seq() == expected.seq && seen == expected.live => Outcome::Exact,
        Ok(seen) => Outcome::Wrong(format!(
            "sequence {} (expected {}): {}",
            snap.seq(),
            expected.seq,
            kerno_testkit::harness::diff(&expected.live, &seen)
        )),
    }
}

fn reported_or_wrong(e: Error) -> Outcome {
    match e {
        Error::Corruption(_) | Error::Io(_) => Outcome::Reported,
        other => Outcome::Wrong(format!("{other:?}")),
    }
}

/// What a sweep over one case found.
#[derive(Default)]
struct Sweep {
    runs: u64,
    reported: u64,
    harmless: Vec<String>,
    wrong: Vec<String>,
}

impl Sweep {
    fn record(&mut self, what: impl FnOnce() -> String, outcome: Outcome) {
        self.runs += 1;
        match outcome {
            Outcome::Reported => self.reported += 1,
            Outcome::Exact => self.harmless.push(what()),
            Outcome::Wrong(why) => self.wrong.push(format!("{}: {why}", what())),
        }
    }
}

/// The cases the damage tests cover: those that open and were closed cleanly.
fn clean_cases() -> Vec<Committed> {
    let cases: Vec<_> =
        read_committed().into_iter().filter(|c| c.expected.mode == Mode::Rw && c.expected.clean).collect();
    assert!(cases.len() >= 6, "{} clean cases", cases.len());
    cases
}

/// Runs `sweep` on every clean case, one thread per case, and checks that
/// nothing was wrong and nothing was harmless.
fn damage_test(what: &str, sweep: impl Fn(&Committed) -> Sweep + Sync) {
    let cases = clean_cases();
    let sweeps: Vec<(String, Sweep)> = std::thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|case| {
                let sweep = &sweep;
                scope.spawn(move || (case.name.clone(), sweep(case)))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut failures = Vec::new();
    for (name, s) in &sweeps {
        println!(
            "{what} {name}: {} runs, {} reported, {} harmless, {} wrong",
            s.runs,
            s.reported,
            s.harmless.len(),
            s.wrong.len()
        );
        failures.extend(s.harmless.iter().take(5).map(|h| format!("{name}: harmless: {h}")));
        failures.extend(s.wrong.iter().take(5).map(|w| format!("{name}: wrong: {w}")));
        assert!(s.runs > 0, "{name}: nothing ran");
    }
    assert!(failures.is_empty(), "{what}: {} problems, for example:\n{}", failures.len(), failures.join("\n"));
}

/// Flipping any bit of any byte of a clean database is reported or gives the
/// exact contents, and never the latter: no byte is uncovered.
#[test]
fn dir_vectors_exhaustive_bit_flips() {
    damage_test("bit flips", |case| {
        let mut image = image_of(&case.files);
        let mut sweep = Sweep::default();
        for (name, bytes) in &case.files {
            for offset in probed_offsets(bytes.len()) {
                for bit in 0..8 {
                    let path = db_path(name);
                    image.files.get_mut(&path).unwrap()[offset] ^= 1 << bit;
                    let o = outcome(&image, &case.expected);
                    image.files.get_mut(&path).unwrap()[offset] ^= 1 << bit;
                    sweep.record(|| format!("bit {bit} of byte {offset} of {name}"), o);
                }
            }
        }
        sweep
    });
}

/// Cutting any file of a clean database at any offset is reported.
#[test]
fn dir_vectors_truncate_every_offset() {
    damage_test("truncations", |case| {
        let mut image = image_of(&case.files);
        let mut sweep = Sweep::default();
        for (name, bytes) in &case.files {
            for len in probed_offsets(bytes.len()) {
                let path = db_path(name);
                image.files.insert(path.clone(), bytes[..len].to_vec());
                let o = outcome(&image, &case.expected);
                sweep.record(|| format!("{name} cut at {len} of {}", bytes.len()), o);
            }
            image.files.insert(db_path(name), bytes.clone());
        }
        sweep
    });
}
