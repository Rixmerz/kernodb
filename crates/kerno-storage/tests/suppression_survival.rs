//! E9A-01 (docs/architecture/etapa9-adelanto-acta.md §3.2): what survives a
//! delete in `kerno-storage`.
//!
//! **This file is an observation of the current behaviour, not a contract.**
//! It measures which files of the database directory still contain the
//! sentinel of a deleted value and of its key at each stage; it does not fix
//! a deletion policy and it must not be read as one. ADR-0009 is still
//! *Pendiente* (its criteria, dated 2026-11-11, are in
//! docs/adr/0009-supresion-retencion-y-backups.md). If the engine's behaviour
//! changes, this file may need to change with it; that is not a regression of
//! a guarantee, because the engine makes none (DESIGN.md, "Qué puede
//! sobrevivir a `compact_range`"; threat model §6.5).
//!
//! # Predictions (acta §3.2, fixed before the first run; never changed)
//!
//! - **P1.** Tras `delete` sin flush, el valor sigue en el WAL.
//! - **P2.** Tras `FLUSH`, `compact_range` que cubre la clave y cierre limpio,
//!   el valor no aparece en ningún archivo vivo del directorio.
//! - **P3.** La lápida conserva la clave en algún archivo vivo hasta que la
//!   compactación la descarta.
//! - **P4.** Con `wal_preallocate = true`, un WAL volcado queda con ceros y el
//!   valor no aparece en él.
//!
//! (English gloss, for the code that follows: P1 a deleted value is still in
//! the WAL before any flush; P2 after flush, a covering `compact_range` and a
//! clean close, no live file holds the value; P3 the tombstone keeps the key
//! in some live file until compaction discards it; P4 with a preallocated
//! WAL, a flushed log is left as zeros and the value is not in it.)
//!
//! Each prediction is evaluated by `verdict`, which prints `HOLDS` or
//! `CONTRADICTED` with the observed files. A contradicted prediction is not
//! corrected to pass and no `src/` code is changed for it: the assertion then
//! pins the observed behaviour and a comment names the contradiction.
//!
//! # Results (written after the first run; the predictions above are untouched)
//!
//! First run, `cargo test -p kerno-storage --all-features --release --test
//! suppression_survival -- --nocapture`: **P1, P2, P3 and P4 all HOLD; none is
//! contradicted.** Observed on the default configuration:
//!
//! - Before any flush the value and the key are in the live `NNNNNN.wal`
//!   (P1), for `delete` and for `delete_range` (and with `wal_preallocate`).
//! - With the values flushed first, after the flush that follows the deletes
//!   the value is still in the older table and the key is in the tombstone's
//!   table and in the MANIFEST (the MANIFEST names file extents); after
//!   `compact_range` no live file holds either (P2, P3). The reopening adds
//!   nothing back.
//! - With a snapshot taken before the deletes, `compact_range` reports
//!   `pinned_files = 3`; those three tables stay on disk, the one with the
//!   sentinels still holds value and key, and all three go when the snapshot
//!   is dropped. The new MANIFEST names no key even then.
//! - With `wal_preallocate`, the flushed log is `WAL.spare`, all zeros after
//!   the 96-byte header, and holds no value (P4).
//! - Not exercised: a point tombstone left in the last level by a trivial
//!   move, a background deleter thread, and a long-lived handle.
//!
//! # Mutation record
//!
//! Applied to a **copy** of the tree (the working tree was not touched; `git
//! diff` shows only this file as new):
//!
//! - `compaction.rs`, the merge loop: `if cover.covering(it.key()) >
//!   it.seq()` changed to `if false && ...` (covered points are no longer
//!   discarded) and `keep_fragments` changed to keep every fragment, so reads
//!   stay correct. Result: all three tests red; P2 is `CONTRADICTED` with the
//!   range value still in a live `.sst`. The `delete` group is unaffected,
//!   because a point tombstone shadows its value inside the merge, not through
//!   the cover cursor.
//! - Only the first edit (fragments dropped as usual): red earlier, at the
//!   read check, `after compact_range: unexpected key zz-r00-KSENT-RNG`.
//! - Inversions of assertions in this file, one at a time, each restored: P1,
//!   P2, P3 and P4, the snapshot-unchanged check (I-2), the range-fragment
//!   count (C-3), `pinned_files` against the kept tables (C-4) and the
//!   value left after the snapshot is dropped; each turned exactly its own
//!   test red.
//!
//! # Method
//!
//! The database runs on `SimVfs` with the engine inline (flushes and
//! compactions in the calling thread), like the other `kerno-storage` suites,
//! so every stage is deterministic. The "directory" is the set of regular
//! files `SimVfs::dump` returns: `MANIFEST`, `MANIFEST.tmp`, `WAL.spare`, the
//! `NNNNNN.wal` and the `NNNNNN.sst`. Sentinels are ASCII, so their byte form
//! and their ASCII form are the same string. Freed blocks, memory and caches
//! are not in the directory and are not measured (ADR-0009, criterion C-1).
//!
//! Two keys of sentinels are deleted in every run: keys of the `delete` group
//! contain `KSENT-DEL` and values `VSENT-DEL`; keys of the `delete_range`
//! group contain `KSENT-RNG` and values `VSENT-RNG`. The range bounds
//! (`zz-r`, `zz-s`) deliberately do not contain the key sentinel: a range
//! tombstone records its bounds, and measuring whether the *keys* survive
//! needs bounds that are not the keys.

use std::ops::Bound;
use std::path::Path;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::{Durability, WriteBatch};
use kerno_storage::format::header::HEADER_LEN;
use kerno_storage::{Background, CloseBudget, Config, KernoDb, TestConfig};
use kerno_testkit::harness::{Model, apply, diff, read_all};
use kerno_testkit::vfs::SimVfs;

const DIR: &str = "/db";
const SENTINELS_PER_GROUP: u32 = 8;

const DEL_VAL: usize = 0;
const DEL_KEY: usize = 1;
const RNG_VAL: usize = 2;
const RNG_KEY: usize = 3;
/// What is searched for in the files, by index: value and key of the `delete`
/// group, value and key of the `delete_range` group.
const SENTINELS: [(&str, &[u8]); 4] = [
    ("delete value", b"VSENT-DEL"),
    ("delete key", b"KSENT-DEL"),
    ("range value", b"VSENT-RNG"),
    ("range key", b"KSENT-RNG"),
];

/// A small engine config copied from `tests/common`: inline background work,
/// a 16 KiB write buffer and a close without a time limit.
fn config(wal_preallocate: bool) -> Config {
    Config {
        durability: Durability::Sync,
        write_buffer_bytes: 16 << 10,
        cache_bytes: 1 << 20,
        test: TestConfig { background: Background::Inline, db_id: Some([0xe9; 16]) },
        close_budget: CloseBudget::Unbounded,
        wal_preallocate,
        ..Config::default()
    }
}

fn open(disk: &SimVfs, cfg: &Config) -> KernoDb {
    KernoDb::open(disk.as_vfs(), Path::new(DIR), cfg).unwrap()
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

fn put(db: &KernoDb, model: &mut Model, key: &str, value: &[u8]) {
    let mut b = WriteBatch::new();
    b.put(key, value);
    db.commit(&b).unwrap();
    apply(model, &b);
}

fn commit(db: &KernoDb, model: &mut Model, b: &WriteBatch) {
    db.commit(b).unwrap();
    apply(model, b);
}

fn del_key(i: u32) -> String {
    format!("zz-d{i:02}-KSENT-DEL")
}

fn rng_key(i: u32) -> String {
    format!("zz-r{i:02}-KSENT-RNG")
}

/// The sentinel, then padding that does not contain any sentinel.
fn value_of(sentinel: &[u8]) -> Vec<u8> {
    [sentinel, b"-", &[0x5a; 150][..]].concat()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// What one stage of one run looked like on disk.
struct Stage {
    name: &'static str,
    /// `(file name, length)` of every regular file.
    files: Vec<(String, usize)>,
    /// Per sentinel (see `SENTINELS`), the files that hold it. A file that
    /// was unlinked but is still open is named `(unlinked, open)`.
    holders: [Vec<String>; 4],
    /// For `WAL.spare`, whether everything after the header is zero.
    spare_is_zeros: Option<bool>,
}

impl Stage {
    fn observe(disk: &SimVfs, name: &'static str) -> Stage {
        let image = disk.dump();
        let mut files = Vec::new();
        let mut holders: [Vec<String>; 4] = Default::default();
        let mut spare_is_zeros = None;
        for (path, bytes) in &image.files {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            files.push((file.clone(), bytes.len()));
            for (i, (_, sentinel)) in SENTINELS.iter().enumerate() {
                if contains(bytes, sentinel) {
                    holders[i].push(file.clone());
                }
            }
            if file == "WAL.spare" {
                spare_is_zeros = Some(bytes.len() > HEADER_LEN && bytes[HEADER_LEN..].iter().all(|b| *b == 0));
            }
        }
        for bytes in &image.unlinked_open {
            for (i, (_, sentinel)) in SENTINELS.iter().enumerate() {
                if contains(bytes, sentinel) {
                    holders[i].push("(unlinked, open)".to_string());
                }
            }
        }
        let stage = Stage { name, files, holders, spare_is_zeros };
        stage.print();
        stage
    }

    fn print(&self) {
        println!("[survival] stage: {}", self.name);
        let listing: Vec<String> = self.files.iter().map(|(n, l)| format!("{n}={l}")).collect();
        println!("[survival]   files: {}", listing.join(" "));
        if let Some(zeros) = self.spare_is_zeros {
            println!("[survival]   WAL.spare is zeros after the header: {zeros}");
        }
        for (i, (label, _)) in SENTINELS.iter().enumerate() {
            println!("[survival]   {label:<12} in {:?}", self.holders[i]);
        }
    }

    /// The files of kind `kind` (a name test) that hold sentinel `which`.
    fn held_by(&self, which: usize, kind: fn(&str) -> bool) -> Vec<&str> {
        self.holders[which].iter().map(String::as_str).filter(|f| kind(f)).collect()
    }

    fn anywhere(&self, which: usize) -> bool {
        !self.holders[which].is_empty()
    }
}

fn is_wal(name: &str) -> bool {
    name.ends_with(".wal")
}
fn is_log_file(name: &str) -> bool {
    name.ends_with(".wal") || name == "WAL.spare"
}
fn is_sst(name: &str) -> bool {
    name.ends_with(".sst")
}
fn is_manifest(name: &str) -> bool {
    name.starts_with("MANIFEST")
}

/// Prints the verdict of a prediction and returns whether it held.
fn verdict(prediction: &str, holds: bool, observed: &str) -> bool {
    if holds {
        println!("[survival] {prediction}: HOLDS ({observed})");
    } else {
        println!("[survival] {prediction}: CONTRADICTED ({observed})");
    }
    holds
}

#[derive(Clone, Copy)]
struct Plan {
    wal_preallocate: bool,
    /// Flush the sentinel puts to a table before deleting them.
    flush_before_delete: bool,
    /// Keep a snapshot taken before the deletes alive through `compact_range`.
    hold_snapshot: bool,
}

#[derive(Default)]
struct Pinned {
    pinned_files: u64,
    pinned_bytes: u64,
    files_rewritten: u64,
    /// Tables that existed before `compact_range` and still exist after it.
    kept_tables: Vec<String>,
    retained_bytes: u64,
    /// Whether the held snapshot read exactly what it read before.
    snapshot_unchanged: bool,
}

struct Outcome {
    stages: Vec<Stage>,
    pinned: Pinned,
    wal_recycled: u64,
    /// `range_fragments` of the current version just before and just after
    /// `compact_range`.
    fragments: (u64, u64),
}

impl Outcome {
    fn at(&self, name: &str) -> &Stage {
        self.stages.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("no stage {name}"))
    }
}

/// Reads everything the handle shows to a new snapshot and compares it with
/// the model (ADR-0009 I-1: nothing deleted is visible after the commit).
fn assert_visible_state(db: &KernoDb, model: &Model, ctx: &str) {
    let snap = db.snapshot().unwrap();
    let all = read_all(&snap).unwrap();
    assert!(all == *model, "{ctx}: {}", diff(model, &all));
    for i in 0..SENTINELS_PER_GROUP {
        assert_eq!(snap.get(del_key(i).as_bytes()).unwrap(), None, "{ctx}: deleted key {i} is readable");
        assert_eq!(snap.get(rng_key(i).as_bytes()).unwrap(), None, "{ctx}: range-deleted key {i} is readable");
    }
}

fn table_names(disk: &SimVfs) -> Vec<String> {
    let mut v: Vec<String> = disk
        .dump()
        .files
        .keys()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .filter(|n| is_sst(n))
        .collect();
    v.sort();
    v
}

/// Writes, deletes and compacts as `plan` says and observes the directory at
/// each stage. Stage names are fixed so the tests can pick them.
fn run(seed: u64, plan: Plan) -> Outcome {
    println!(
        "[survival] run: wal_preallocate={} flush_before_delete={} hold_snapshot={}",
        plan.wal_preallocate, plan.flush_before_delete, plan.hold_snapshot
    );
    let disk = SimVfs::new(seed);
    let cfg = config(plan.wal_preallocate);
    let db = open(&disk, &cfg);
    let mut model = Model::new();
    let mut stages = Vec::new();
    let mut pinned = Pinned::default();

    // Ordinary data first, enough for several tables, then a flush so the
    // sentinel commits start on an empty memtable and log.
    for i in 0..120u32 {
        put(&db, &mut model, &format!("key-{i:05}"), &[i as u8; 200]);
    }
    db.hooks().flush().unwrap();

    for i in 0..SENTINELS_PER_GROUP {
        put(&db, &mut model, &del_key(i), &value_of(SENTINELS[DEL_VAL].1));
        put(&db, &mut model, &rng_key(i), &value_of(SENTINELS[RNG_VAL].1));
    }
    if plan.flush_before_delete {
        db.hooks().flush().unwrap();
        stages.push(Stage::observe(&disk, "puts flushed"));
    } else {
        stages.push(Stage::observe(&disk, "puts in the log"));
    }
    let wal_recycled = stat(&db, "wal_recycled");
    let rotations = stat(&db, "memtable_rotations");

    let held = plan.hold_snapshot.then(|| {
        let snap = db.snapshot().unwrap();
        let before = snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap();
        (snap, before)
    });

    // The deletes: one `delete` per key of the first group, one range over the
    // second.
    for i in 0..SENTINELS_PER_GROUP {
        let mut b = WriteBatch::new();
        b.delete(del_key(i));
        commit(&db, &mut model, &b);
    }
    let mut b = WriteBatch::new();
    b.delete_range("zz-r", "zz-s");
    commit(&db, &mut model, &b);
    assert_eq!(stat(&db, "memtable_rotations"), rotations, "the deletes must stay in the log they were written to");
    assert_visible_state(&db, &model, "after the deletes");
    if let Some((snap, before)) = &held {
        let now = snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap();
        pinned.snapshot_unchanged = now == *before; // ADR-0009 I-2
    }
    stages.push(Stage::observe(&disk, "deleted, not flushed"));

    db.hooks().flush().unwrap();
    assert_visible_state(&db, &model, "after the flush");
    stages.push(Stage::observe(&disk, "flushed"));

    let tables_before = table_names(&disk);
    let fragments_before = stat(&db, "range_fragments");
    let report = db.compact_range(Bound::Included(b"zz-"), Bound::Excluded(b"zz.")).unwrap();
    db.hooks().wait_idle().unwrap();
    assert_visible_state(&db, &model, "after compact_range");
    if let Some((snap, before)) = &held {
        let now = snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap();
        pinned.snapshot_unchanged = pinned.snapshot_unchanged && now == *before;
    }
    let tables_after = table_names(&disk);
    let fragments = (fragments_before, stat(&db, "range_fragments"));
    println!("[survival]   range fragments in the current version: {} before, {} after", fragments.0, fragments.1);
    pinned.pinned_files = report.pinned_files;
    pinned.pinned_bytes = report.pinned_bytes;
    pinned.files_rewritten = report.files_rewritten;
    pinned.kept_tables = tables_before.into_iter().filter(|t| tables_after.contains(t)).collect();
    pinned.retained_bytes = db.metrics().snapshots.retained_bytes;
    println!(
        "[survival]   compact_range: rewritten={} pinned_files={} pinned_bytes={} kept_tables={:?} retained_bytes={}",
        pinned.files_rewritten, pinned.pinned_files, pinned.pinned_bytes, pinned.kept_tables, pinned.retained_bytes
    );
    stages.push(Stage::observe(&disk, "compact_range"));

    if let Some((snap, _)) = held {
        drop(snap);
        db.hooks().wait_idle().unwrap();
        stages.push(Stage::observe(&disk, "snapshot dropped"));
    }

    db.close().unwrap();
    stages.push(Stage::observe(&disk, "closed"));

    let db = open(&disk, &cfg);
    assert_visible_state(&db, &model, "after reopening");
    stages.push(Stage::observe(&disk, "reopened"));
    db.close().unwrap();

    Outcome { stages, pinned, wal_recycled, fragments }
}

/// P1, P2 and P3 on the default configuration (`wal_preallocate = false`),
/// with the values deleted in the log they were written to (P1) and with the
/// values flushed to a table before the delete (P2, P3).
#[test]
fn observation_of_current_behaviour_default_log() {
    // Values and deletes in the same log, no flush in between.
    let unflushed = run(9001, Plan { wal_preallocate: false, flush_before_delete: false, hold_snapshot: false });
    let s = unflushed.at("deleted, not flushed");
    let p1 = verdict(
        "P1 (delete: value still in the log, no flush)",
        !s.held_by(DEL_VAL, is_wal).is_empty(),
        &format!("delete value in {:?}", s.holders[DEL_VAL]),
    );
    let p1_range = verdict(
        "P1 (delete_range: value still in the log, no flush)",
        !s.held_by(RNG_VAL, is_wal).is_empty(),
        &format!("range value in {:?}", s.holders[RNG_VAL]),
    );
    assert!(p1 && p1_range);

    // Values in a table first, deletes in a newer log and table.
    let flushed = run(9002, Plan { wal_preallocate: false, flush_before_delete: true, hold_snapshot: false });
    let before = flushed.at("flushed");
    let closed = flushed.at("closed");
    let reopened = flushed.at("reopened");
    let p2 = verdict(
        "P2 (value in no live file after flush, compact_range, clean close)",
        !closed.anywhere(DEL_VAL) && !closed.anywhere(RNG_VAL),
        &format!("delete value in {:?}, range value in {:?}", closed.holders[DEL_VAL], closed.holders[RNG_VAL]),
    );
    println!(
        "[survival] before compact_range the values were in {:?} / {:?}",
        before.holders[DEL_VAL], before.holders[RNG_VAL]
    );
    assert!(p2);
    assert!(!reopened.anywhere(DEL_VAL) && !reopened.anywhere(RNG_VAL), "reopening put a value back on disk");

    let p3 = verdict(
        "P3 (tombstone keeps the key in a live file until compaction discards it)",
        !before.held_by(DEL_KEY, is_sst).is_empty() && !closed.anywhere(DEL_KEY),
        &format!("before: {:?}, after close: {:?}", before.holders[DEL_KEY], closed.holders[DEL_KEY]),
    );
    assert!(p3);

    // ADR-0009 C-2 and C-3, observed in the same run: the checkpoint of
    // `compact_range` leaves no key in the MANIFEST (before it, the MANIFEST
    // named the keys as file extents), and no range fragment is left in the
    // current version.
    let compacted = flushed.at("compact_range");
    println!(
        "[survival] C-2: MANIFEST names a key before compact_range: {:?}, after: {:?}",
        before.held_by(DEL_KEY, is_manifest),
        compacted.held_by(DEL_KEY, is_manifest)
    );
    assert!(!before.held_by(RNG_KEY, is_manifest).is_empty(), "test setup: the MANIFEST names keys before");
    for key in [DEL_KEY, RNG_KEY] {
        assert!(compacted.held_by(key, is_manifest).is_empty());
    }
    println!("[survival] C-3: range fragments {} -> {}", flushed.fragments.0, flushed.fragments.1);
    assert!(flushed.fragments.0 > 0, "test setup: the range tombstone is in a table before compact_range");
    assert_eq!(flushed.fragments.1, 0);
}

/// P4 (and P1 to P3 again) with `wal_preallocate = true`: a flushed log is
/// overwritten with zeros and kept as the spare for the next one.
#[test]
fn observation_of_current_behaviour_preallocated_log() {
    let unflushed = run(9003, Plan { wal_preallocate: true, flush_before_delete: false, hold_snapshot: false });
    let s = unflushed.at("deleted, not flushed");
    assert!(verdict(
        "P1 with wal_preallocate",
        !s.held_by(DEL_VAL, is_wal).is_empty() && !s.held_by(RNG_VAL, is_wal).is_empty(),
        &format!("delete value in {:?}, range value in {:?}", s.holders[DEL_VAL], s.holders[RNG_VAL]),
    ),);

    let flushed = run(9004, Plan { wal_preallocate: true, flush_before_delete: true, hold_snapshot: false });
    assert!(flushed.wal_recycled > 0, "test setup: the flushed log must have been recycled");
    let stage = flushed.at("puts flushed");
    let p4 = verdict(
        "P4 (flushed log is zeros and holds no value)",
        stage.spare_is_zeros == Some(true)
            && stage.held_by(DEL_VAL, is_log_file).is_empty()
            && stage.held_by(RNG_VAL, is_log_file).is_empty(),
        &format!(
            "spare is zeros: {:?}; delete value in {:?}; range value in {:?}",
            stage.spare_is_zeros, stage.holders[DEL_VAL], stage.holders[RNG_VAL]
        ),
    );
    assert!(p4);

    let closed = flushed.at("closed");
    assert!(verdict(
        "P2 with wal_preallocate",
        !closed.anywhere(DEL_VAL) && !closed.anywhere(RNG_VAL),
        &format!("delete value in {:?}, range value in {:?}", closed.holders[DEL_VAL], closed.holders[RNG_VAL]),
    ),);
    let before = flushed.at("flushed");
    assert!(verdict(
        "P3 with wal_preallocate",
        !before.held_by(DEL_KEY, is_sst).is_empty() && !closed.anywhere(DEL_KEY),
        &format!("before: {:?}, after close: {:?}", before.holders[DEL_KEY], closed.holders[DEL_KEY]),
    ),);
}

/// ADR-0009 I-2 and C-4: a snapshot taken before the delete keeps reading the
/// old value, the tables it holds stay on disk with the value, and they go
/// when the snapshot is dropped.
#[test]
fn observation_of_current_behaviour_snapshot_pins_tables() {
    let out = run(9005, Plan { wal_preallocate: false, flush_before_delete: true, hold_snapshot: true });
    assert!(out.pinned.snapshot_unchanged, "I-2: the snapshot saw the delete before it was dropped");
    let p = &out.pinned;
    assert!(p.pinned_files > 0 && p.pinned_bytes > 0, "test setup: compact_range must report pinned tables");
    let held = out.at("compact_range");
    let ssts_with_value: Vec<&str> = held.held_by(DEL_VAL, is_sst);
    assert!(!ssts_with_value.is_empty(), "the pinned tables should still hold the value");
    // C-4: the tables that keep the sentinel are exactly among the tables the
    // report counts as retained.
    assert!(
        ssts_with_value.iter().all(|t| p.kept_tables.iter().any(|k| k == t)),
        "{ssts_with_value:?} hold the value but only {:?} survived",
        p.kept_tables
    );
    assert_eq!(p.kept_tables.len() as u64, p.pinned_files, "{:?}", p.kept_tables);
    assert!(p.retained_bytes > 0, "snapshot_retained_bytes does not show the held tables");
    let dropped = out.at("snapshot dropped");
    assert!(!dropped.anywhere(DEL_VAL) && !dropped.anywhere(RNG_VAL), "the value outlived the snapshot");
    // C-2 with a snapshot: the checkpoint names no key even though the pinned
    // table (a live file) still holds them.
    assert!(held.held_by(DEL_KEY, is_manifest).is_empty() && held.held_by(RNG_KEY, is_manifest).is_empty());
    let closed = out.at("closed");
    assert!(!closed.anywhere(DEL_KEY) && !closed.anywhere(RNG_KEY));
}
