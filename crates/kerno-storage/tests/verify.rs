//! `verify()` and `inspect()` (v1.md §7.9 SST-V1..V4, §9.5 RO-1..RO-4): they
//! take the `LOCK`, find every kind of damage the format can show without
//! opening the database, and never write. Every check here runs on a copy of
//! the directory and compares it byte for byte afterwards (the `LOCK` file
//! aside).

mod common;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, Result};
use kerno_storage::format::bloom;
use kerno_storage::format::filter::Filter;
use kerno_storage::format::wal::{WalScanParams, scan_wal};
use kerno_storage::{FileType, Issue, VerifyOptions, VerifyReport, inspect, verify};
use kerno_testkit::harness::seeds;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{DirImage, HookAction, HookOpKind, HookPoint, HookVfs, SimVfs, assert_dir_unchanged};

use common::craft::*;
use common::*;

fn verify_in(vfs: Arc<dyn Vfs>, opts: &VerifyOptions) -> Result<VerifyReport> {
    verify(vfs, Path::new(DIR), opts)
}

/// Verifies a copy of `image`; the copy must come out byte-identical and
/// without a single mutating operation (I-18, RO-1).
fn check_with(image: &DirImage, opts: &VerifyOptions) -> Result<VerifyReport> {
    let disk = SimVfs::restore(image);
    let before = disk.dump();
    let ops = disk.mutating_ops();
    let report = verify_in(disk.as_vfs(), opts);
    assert_eq!(disk.mutating_ops(), ops, "verify did a mutating operation");
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    report
}

fn check(image: &DirImage) -> Result<VerifyReport> {
    check_with(image, &VerifyOptions::default())
}

/// The report of a verify that must not fail.
fn report_of(image: &DirImage) -> VerifyReport {
    check(image).unwrap_or_else(|e| panic!("verify failed: {e}"))
}

fn rules(r: &VerifyReport) -> Vec<&'static str> {
    r.issues.iter().map(|i| i.rule).collect()
}

fn issue<'a>(r: &'a VerifyReport, rule: &str) -> &'a Issue {
    r.issues.iter().find(|i| i.rule == rule).unwrap_or_else(|| panic!("no {rule} issue in {:?}", r.issues))
}

/// Verify names the damage or fails: anything but a clean report.
fn is_flagged(r: &Result<VerifyReport>) -> bool {
    r.as_ref().map_or(true, |r| !r.is_ok())
}

#[test]
fn verify_clean_db_ok() {
    let (image, model) = closed_image();
    let r = report_of(&image);
    assert!(r.is_ok(), "{:?}", r.issues);
    assert!(r.clean && r.wals.is_empty() && r.orphans.is_empty() && r.unknown.is_empty());
    assert_eq!(r.last_seq, model.len() as u64, "400 commits of distinct keys");
    let on_disk = files_with(&image, "sst").len() as u64;
    assert_eq!(r.levels.iter().map(|l| l.files).sum::<u64>(), on_disk);
    assert_eq!(r.levels.iter().map(|l| l.entries).sum::<u64>(), model.len() as u64);
    // The MANIFEST and every table, with the version and bits they carry.
    assert_eq!(r.files.len() as u64, 1 + on_disk);
    for f in &r.files {
        let h = f.header.expect("a valid header");
        assert_eq!((h.format_version, h.db_id), (1, DB_ID), "{}", f.name);
        assert_eq!(f.len, image.files[&path(&f.name)].len() as u64, "{}", f.name);
    }

    // A database that was not closed: live WAL, MANIFEST that is not CLEAN.
    let (image, _) = unclean_image();
    let r = report_of(&image);
    assert!(r.is_ok() && !r.clean, "{:?}", r.issues);
    assert_eq!(r.wals.len(), files_with(&image, "wal").len());
    let newest = r.wals.last().unwrap();
    assert!(newest.header_valid && newest.frames >= 8 && newest.commits >= 8, "{newest:?}");
    assert!(!newest.torn_tail);
}

#[test]
fn verify_reports_a_torn_tail_without_calling_it_damage() {
    let (mut image, _) = unclean_image();
    image.files.get_mut(&path("MANIFEST")).unwrap().extend_from_slice(&[0xAB; 37]);
    let wal = files_with(&image, "wal").pop().unwrap();
    image.files.get_mut(&wal).unwrap().extend_from_slice(&[0xCD; 21]);
    let r = report_of(&image);
    assert!(r.is_ok(), "a crash leaves these: {:?}", r.issues);
    assert!(r.manifest_torn_tail);
    assert!(r.wals.last().unwrap().torn_tail);
}

#[test]
fn verify_is_read_only() {
    let (closed, _) = closed_image();
    let (unclean, _) = unclean_image();
    // The worst shape a crash leaves: torn tails, MANIFEST.tmp, an orphan,
    // unknown files.
    let mut messy = unclean.clone();
    messy.files.get_mut(&path("MANIFEST")).unwrap().extend_from_slice(&[0xAB; 37]);
    let wal = files_with(&messy, "wal").pop().unwrap();
    messy.files.get_mut(&wal).unwrap().extend_from_slice(&[0xCD; 21]);
    messy.files.insert(path("MANIFEST.tmp"), b"half a checkpoint".to_vec());
    messy.files.insert(path("999999.sst"), messy.files[&files_with(&messy, "sst")[0]].clone());
    messy.files.insert(path("notes.txt"), b"not ours".to_vec());
    // Damaged ones.
    let mut bad_table = closed.clone();
    let sst = files_with(&bad_table, "sst")[0].clone();
    bad_table.files.get_mut(&sst).unwrap()[200] ^= 0x04;
    let mut bad_manifest = unclean.clone();
    bad_manifest.files.get_mut(&path("MANIFEST")).unwrap()[100] ^= 0x01;
    let mut short_table = closed.clone();
    short_table.files.get_mut(&sst).unwrap().truncate(300);
    let mut no_wal = unclean.clone();
    no_wal.files.remove(&files_with(&no_wal, "wal")[0]);
    for (name, image) in [
        ("closed", &closed),
        ("unclean", &unclean),
        ("messy", &messy),
        ("bad table", &bad_table),
        ("bad manifest", &bad_manifest),
        ("short table", &short_table),
        ("missing wal", &no_wal),
    ] {
        // `check` compares the bytes and counts the mutating operations.
        let r = check(image);
        assert_eq!(name == "closed" || name == "unclean" || name == "messy", !is_flagged(&r), "{name}: {r:?}");
    }
}

/// Every bit of every live file of a closed database is covered by a check:
/// a flip anywhere is an issue or an error, never a clean report. (After a
/// clean close the directory holds only the MANIFEST and the tables, so no
/// damage can be taken for a torn tail.)
#[test]
fn verify_detects_every_bit_flip() {
    let (image, _) = closed_image();
    let live: Vec<_> = files_with(&image, "MANIFEST").into_iter().chain(files_with(&image, "sst")).collect();
    let total: u64 = live.iter().map(|p| image.files[p].len() as u64).sum();
    let (mut detected, mut missed) = (0u64, Vec::new());
    for seed in seeds(150) {
        let mut rng = Rng::new(seed);
        let mut at = rng.below(total);
        let file = live
            .iter()
            .find(|p| {
                let len = image.files[*p].len() as u64;
                if at < len {
                    true
                } else {
                    at -= len;
                    false
                }
            })
            .unwrap();
        let mask = 1u8 << rng.below(8);
        let mut damaged = image.clone();
        damaged.files.get_mut(file).unwrap()[at as usize] ^= mask;
        if is_flagged(&check(&damaged)) {
            detected += 1;
        } else {
            missed.push(format!("{} byte {at} mask {mask:#04x} (seed {seed})", file.display()));
        }
    }
    println!("verify_detects_every_bit_flip: {detected} detected, {} missed", missed.len());
    assert!(missed.is_empty(), "flips that verify did not report: {missed:?}");
}

#[test]
fn verify_detects_truncation() {
    let (image, _) = closed_image();
    let live: Vec<_> = files_with(&image, "MANIFEST").into_iter().chain(files_with(&image, "sst")).collect();
    for file in &live {
        let len = image.files[file].len();
        for cut in [0, 1, 95, 96, 97, len / 2, len - 81, len - 80, len - 1] {
            let mut damaged = image.clone();
            damaged.files.get_mut(file).unwrap().truncate(cut);
            let r = check(&damaged);
            assert!(is_flagged(&r), "{} cut to {cut} of {len} bytes: {r:?}", file.display());
        }
    }
}

/// A table whose filter lacks one of its keys: the engine would answer `None`
/// for that key without any error. Only SST-V3 shows it.
#[test]
fn verify_detects_bloom_missing_key() {
    let mut t = Table::grid(1, 'k', 3, 40, 0);
    // A key the smaller filter really does not match (a bloom filter can
    // match by chance).
    let all: Vec<&_> = t.blocks.iter().flatten().collect();
    let (hole, filter) = all
        .iter()
        .find_map(|missing| {
            let hashes: Vec<u64> = all.iter().filter(|e| e.key != missing.key).map(|e| bloom::hash(&e.key)).collect();
            let f = Filter::build(&hashes);
            (!f.may_contain(bloom::hash(&missing.key))).then(|| (missing.key.clone(), f))
        })
        .expect("some key is absent from a filter built without it");
    t.filter = filter;
    let image = Dir::single(&t).image();
    let r = report_of(&image);
    assert_eq!(rules(&r), ["SST-V3"], "{:?}", r.issues);
    assert_eq!(r.issues[0].file, "000001.sst");
    // The silent wrong answer this rule exists for.
    let (_, db) = open_copy(&image, &config(Durability::Sync));
    let db = db.expect("a table with an incomplete filter opens: only verify can tell");
    assert_eq!(db.snapshot().unwrap().get(&hole).unwrap(), None);
}

#[test]
fn verify_detects_cross_block_disorder() {
    let mut t = Table::grid(1, 'k', 2, 40, 0);
    // The second block starts below the end of the first one (each block is
    // sorted, the index is increasing).
    let low = t.blocks[0][5].key.clone();
    t.blocks[1][0].key = low;
    t.refresh();
    let image = Dir::single(&t).image();
    let r = report_of(&image);
    assert_eq!(rules(&r), ["SST-R2"], "{:?}", r.issues);
    // The engine finds it as soon as it reads that block.
    let (_, db) = open_copy(&image, &config(Durability::Sync));
    let snap = db.expect("the open reads no data block").snapshot().unwrap();
    let scan = snap.scan(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded, usize::MAX);
    assert!(matches!(scan, Err(Error::Corruption(_))), "{scan:?}");
}

#[test]
fn verify_detects_entry_count_mismatch() {
    type Lie = fn(&mut kerno_storage::format::props::Props);
    let lies: [(&str, Lie); 4] = [
        ("point_entries", |p| p.point_entries += 1),
        ("point_deletes", |p| p.point_deletes += 1),
        ("raw_key_bytes", |p| p.raw_key_bytes += 1),
        ("raw_value_bytes", |p| p.raw_value_bytes -= 1),
    ];
    for (what, lie) in lies {
        let mut t = Table::grid(1, 'k', 3, 40, 0);
        lie(&mut t.props);
        let r = report_of(&Dir::single(&t).image());
        assert_eq!(rules(&r), ["SST-V1"], "{what}: {:?}", r.issues);
    }
    // The key range the properties declare must be the real one (SST-V2).
    let mut t = Table::grid(1, 'k', 3, 40, 0);
    t.props.smallest_key = b"a".to_vec();
    let r = report_of(&Dir::single(&t).image());
    assert_eq!(rules(&r), ["SST-V2"], "{:?}", r.issues);
}

#[test]
fn verify_reports_orphans_and_unknown_without_deleting() {
    let (mut image, model) = closed_image();
    let (replay, _) = manifest_of(&image);
    let used: Vec<u64> = image.files.keys().filter_map(|p| p.file_stem()?.to_str()?.parse().ok()).collect();
    let mut free = (1..replay.state.next_file).filter(|n| !used.contains(n));
    let (orphan_sst, orphan_wal) = (free.next().unwrap(), free.next().unwrap());
    let sst = files_with(&image, "sst").pop().unwrap();
    let copy = image.files[&sst].clone();
    let copy_len = copy.len() as u64;
    image.files.insert(path(&format!("{orphan_sst:06}.sst")), copy);
    image.files.insert(path(&format!("{orphan_wal:06}.wal")), wal_bytes(orphan_wal, replay.state.last_seq + 1, &[]));
    image.files.insert(path("MANIFEST.tmp"), b"stale".to_vec());
    image.files.insert(path("notes.txt"), b"not ours".to_vec());
    let r = report_of(&image);
    assert!(r.is_ok(), "orphans are not damage: {:?}", r.issues);
    let mut names: Vec<&str> = r.orphans.iter().map(|o| o.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, [format!("{orphan_sst:06}.sst"), format!("{orphan_wal:06}.wal")]);
    let table = r.orphans.iter().find(|o| o.name.ends_with(".sst")).unwrap();
    assert_eq!(table.len, copy_len);
    assert_eq!(r.unknown, ["notes.txt"]);
    assert!(r.manifest_tmp && r.vanished_orphans.is_empty());
    // The same directory opened read-write recovers it to the same model.
    let (_, db) = open_copy(&image, &config(Durability::Sync));
    assert!(model_of(&db.unwrap()) == model);
}

#[test]
fn verify_classifies_vanished_orphans() {
    let (mut image, _) = closed_image();
    let (replay, _) = manifest_of(&image);
    let used: Vec<u64> = image.files.keys().filter_map(|p| p.file_stem()?.to_str()?.parse().ok()).collect();
    let orphan = (1..replay.state.next_file).find(|n| !used.contains(n)).unwrap();
    let sst = files_with(&image, "sst").pop().unwrap();
    let orphan_name = format!("{orphan:06}.sst");
    image.files.insert(path(&orphan_name), image.files[&sst].clone());

    // RO-4: an outliving snapshot deletes an obsolete table while verify runs.
    let removing = |victim: String| {
        let disk = SimVfs::restore(&image);
        let inner = disk.as_vfs();
        let done = AtomicBool::new(false);
        let vfs = HookVfs::wrap(inner.clone(), move |op| {
            if op.kind == HookOpKind::Open
                && op.point == HookPoint::Before
                && op.path.file_name().is_some_and(|n| *n == *victim)
                && !done.swap(true, Ordering::SeqCst)
            {
                inner.remove_file(op.path).unwrap();
            }
            HookAction::Proceed
        });
        verify_in(vfs, &VerifyOptions::default())
    };

    // Not referenced: informational.
    let r = removing(orphan_name.clone()).unwrap();
    assert!(r.is_ok(), "{:?}", r.issues);
    assert_eq!(r.vanished_orphans, [orphan_name]);
    assert!(r.orphans.is_empty());

    // Referenced: the same event is damage.
    let referenced = format!("{:06}.sst", replay.state.levels.iter().flatten().next().unwrap().number);
    let r = removing(referenced.clone()).unwrap();
    let i = issue(&r, "REC-5");
    assert_eq!(i.file, referenced);
    assert!(r.vanished_orphans.is_empty());
}

#[test]
fn verify_refuses_when_locked() {
    let (image, _) = closed_image();
    let disk = SimVfs::restore(&image);
    let vfs = disk.as_vfs();
    let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
    assert!(matches!(verify_in(vfs.clone(), &VerifyOptions::default()), Err(Error::Locked)));
    assert!(matches!(inspect(vfs.clone(), Path::new(DIR)), Err(Error::Locked)));
    drop(db);
    assert!(verify_in(vfs.clone(), &VerifyOptions::default()).unwrap().is_ok());
    assert!(inspect(vfs.clone(), Path::new(DIR)).is_ok());
    // verify holds the lock only while it runs.
    assert!(open_in(&vfs, &config(Durability::Sync)).is_ok());
}

#[test]
fn verify_dry_runs_the_live_wal() {
    let (image, _) = unclean_image();
    let wal = files_with(&image, "wal").pop().unwrap();
    let number = file_number(&wal);
    let bytes = &image.files[&wal];
    let params = WalScanParams { file_number: number, db_id: DB_ID, expected_seq: le_u64(bytes, 64) };
    let mut frames = Vec::new();
    scan_wal(&mut &bytes[..], &params, |at, _| {
        frames.push(at);
        Ok(())
    })
    .map_err(|e| e.into_format())
    .unwrap();
    assert!(frames.len() >= 8);
    // Damage in a frame that later ones prove was durable is not a torn tail.
    let mut damaged = image.clone();
    damaged.files.get_mut(&wal).unwrap()[frames[2] as usize + 40] ^= 0x10;
    let r = report_of(&damaged);
    assert_eq!(rules(&r), ["WAL-R5"], "{:?}", r.issues);
    assert_eq!(r.issues[0].file, wal.file_name().unwrap().to_str().unwrap());
    // The same bytes are not read at all without check_wal.
    let r = check_with(&damaged, &VerifyOptions { check_wal: false, ..VerifyOptions::default() }).unwrap();
    assert!(r.is_ok() && r.wals.is_empty(), "{:?}", r.issues);
}

#[test]
fn verify_options_limit_what_is_read() {
    let (image, _) = closed_image();
    let sst = files_with(&image, "sst")[0].clone();
    let mut damaged = image.clone();
    // Inside the first data block: only reading the block can see it.
    damaged.files.get_mut(&sst).unwrap()[150] ^= 0x01;
    assert!(is_flagged(&check(&damaged)));
    let shallow = check_with(&damaged, &VerifyOptions { deep: false, ..VerifyOptions::default() }).unwrap();
    assert!(shallow.is_ok(), "{:?}", shallow.issues);
    // The open-time checks still run: the footer is not a data block.
    let mut damaged = image.clone();
    let len = damaged.files[&sst].len();
    damaged.files.get_mut(&sst).unwrap()[len - 5] ^= 0x01;
    let shallow = check_with(&damaged, &VerifyOptions { deep: false, ..VerifyOptions::default() }).unwrap();
    assert_eq!(rules(&shallow), ["SST-O2"]);
}

#[test]
fn verify_rejects_what_is_not_a_database() {
    let disk = SimVfs::new(1);
    let vfs = disk.as_vfs();
    // Nothing there: no directory is created, nothing is locked.
    let before = disk.dump();
    assert!(matches!(verify_in(vfs.clone(), &VerifyOptions::default()), Err(Error::InvalidArgument(_))));
    assert!(matches!(inspect(vfs.clone(), Path::new(DIR)), Err(Error::InvalidArgument(_))));
    assert_dir_unchanged(&before, &disk.dump(), &[]);
    // A directory of someone else's files.
    vfs.create_dir_all(Path::new(DIR)).unwrap();
    vfs.open(&path("notes.txt"), kerno_core::vfs::OpenMode::CreateNew).unwrap();
    let before = disk.dump();
    assert!(matches!(verify_in(vfs.clone(), &VerifyOptions::default()), Err(Error::InvalidArgument(_))));
    assert_dir_unchanged(&before, &disk.dump(), &[]);
    // Data files without a MANIFEST are a damaged database (REC-2): reported.
    let (mut image, _) = closed_image();
    image.files.remove(&path("MANIFEST"));
    let r = report_of(&image);
    assert_eq!(rules(&r), ["REC-2"]);
}

#[test]
fn inspect_reports_versions_and_bits() {
    let (mut image, _) = closed_image();
    let disk = SimVfs::restore(&image);
    let before = disk.dump();
    let ops = disk.mutating_ops();
    let info = inspect(disk.as_vfs(), Path::new(DIR)).unwrap();
    assert_eq!(disk.mutating_ops(), ops);
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    let ssts = files_with(&image, "sst");
    assert_eq!(info.files.len(), 1 + ssts.len());
    assert_eq!(info.files[0].name, "MANIFEST");
    assert_eq!(info.files[0].file_type, FileType::Manifest);
    for f in &info.files {
        let bytes = &image.files[&path(&f.name)];
        assert_eq!(f.len, bytes.len() as u64, "{}", f.name);
        let h = f.header.expect("valid header");
        // Version 1, the DRAFT bit (no database made now opens under the
        // frozen format) and nothing else.
        assert_eq!((h.format_version, h.compat, h.ro_compat, h.incompat), (1, 0, 0, 1 << 63), "{}", f.name);
        assert_eq!(h.db_id, DB_ID);
        let expected_number =
            if f.file_type == FileType::Manifest { le_u64(bytes, 56) } else { file_number(&path(&f.name)) };
        assert_eq!(h.file_number, expected_number);
        assert!(f.problem.is_none());
    }

    // compat and ro_compat bits are reported as they are.
    let sst = ssts[0].clone();
    patch_header(&mut image, &sst, |h| {
        or_u64(h, 16, 1 << 5);
        or_u64(h, 24, 1 << 3);
    });
    let disk = SimVfs::restore(&image);
    let info = inspect(disk.as_vfs(), Path::new(DIR)).unwrap();
    let f = info.files.iter().find(|f| f.name == sst.file_name().unwrap().to_str().unwrap()).unwrap();
    let h = f.header.unwrap();
    assert_eq!((h.compat, h.ro_compat), (1 << 5, 1 << 3));

    // A header that does not decode is listed with its rule, not an error.
    let mut damaged = image.clone();
    damaged.files.get_mut(&sst).unwrap()[10] ^= 0x01;
    let info = inspect(SimVfs::restore(&damaged).as_vfs(), Path::new(DIR)).unwrap();
    let f = info.files.iter().find(|f| f.header.is_none()).expect("one file without a header");
    assert!(f.problem.as_deref().is_some_and(|p| p.contains("HDR-")), "{:?}", f.problem);

    // A feature this engine cannot read is an error, as it is for open.
    for patch in [(|h: &mut [u8]| h[8] = 2) as fn(&mut [u8]), |h| or_u64(h, 32, 1 << 40), |h| or_u64(h, 32, 1)] {
        let mut incompatible = image.clone();
        patch_header(&mut incompatible, &sst, patch);
        let e = inspect(SimVfs::restore(&incompatible).as_vfs(), Path::new(DIR)).expect_err("incompatible");
        assert!(matches!(e, Error::Incompatible(_)), "{e:?}");
        let e = check(&incompatible).expect_err("verify refuses it too");
        assert!(matches!(e, Error::Incompatible(_)), "{e:?}");
    }
}
