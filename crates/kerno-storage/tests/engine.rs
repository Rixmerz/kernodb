//! LSM-specific behaviour not covered by the shared suite: file lifecycle,
//! compaction effects, stats, poisoning and recovery edge cases.

use std::io;
use std::ops::{Bound, RangeBounds};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::vfs::{OpenMode, Vfs};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::KernoDb;
use kerno_storage::format::header::HEADER_LEN;
use kerno_testkit::harness::{Model, apply, read_all};
use kerno_testkit::vfs::{CrashKind, SimVfs, assert_dir_unchanged};

const DIR: &str = "/db";

fn open(vfs: &Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<KernoDb> {
    <KernoDb as Engine>::open(vfs.clone(), Path::new(DIR), opts)
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

fn files(vfs: &dyn Vfs) -> Vec<String> {
    let mut v = vfs.list(Path::new(DIR)).unwrap();
    v.sort();
    v
}

/// Like `kerno_core::vfs::dir_size`, but tolerates files that background
/// threads delete or rename between listing and opening.
fn dir_size(vfs: &dyn Vfs) -> u64 {
    let mut total = 0;
    for name in vfs.list(Path::new(DIR)).unwrap() {
        match vfs.open(&Path::new(DIR).join(name), OpenMode::Existing) {
            Ok(f) => total += f.len().unwrap_or(0),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => panic!("{e}"),
        }
    }
    total
}

/// Waits until background work has caught up (no immutable memtables, L0
/// depth below the compaction trigger).
fn settle(db: &KernoDb) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while stat(db, "immutable_memtables") > 0 || stat(db, "l0_depth") >= 4 {
        assert!(Instant::now() < deadline, "background work did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn fill(db: &KernoDb, model: &mut Model, round: u32, n: u32) {
    for chunk in 0..n / 50 {
        let mut b = WriteBatch::new();
        for i in 0..50 {
            let k = format!("key-{:06}", (chunk * 50 + i) * 7 % n);
            b.put(k, format!("value-{round}-{i}-{}", "x".repeat((i % 200) as usize)));
        }
        db.commit(&b).unwrap();
        apply(model, &b);
    }
}

#[test]
fn clean_close_leaves_only_manifest_and_tables() {
    let vfs = SimVfs::new(1).as_vfs();
    let opts = Options::small_for_tests();
    let db = open(&vfs, &opts).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 3000);
    db.close().unwrap();
    for name in files(&*vfs) {
        assert!(name == "LOCK" || name == "MANIFEST" || name.ends_with(".sst"), "unexpected file {name} after close");
    }
    let db = open(&vfs, &opts).unwrap();
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
}

#[test]
fn compaction_reclaims_space_and_keeps_data() {
    let vfs = SimVfs::new(2).as_vfs();
    let opts = Options::small_for_tests();
    let db = open(&vfs, &opts).unwrap();
    let mut model = Model::new();
    for round in 0..8 {
        fill(&db, &mut model, round, 2000);
    }
    settle(&db);
    assert!(stat(&db, "flushes") > 10);
    assert!(stat(&db, "compactions") > 0);
    assert!(stat(&db, "l1_files") + stat(&db, "l2_files") > 0, "data should have moved below L0");
    let live: usize = model.iter().map(|(k, v)| k.len() + v.len()).sum();
    let size = dir_size(&*vfs);
    assert!(size < 4 * live as u64 + (512 << 10), "{size} bytes on disk for {live} live bytes");
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);

    // Delete everything; tombstones hide all data, also after reopen.
    let mut b = WriteBatch::new();
    for k in model.keys() {
        b.delete(k.clone());
    }
    db.commit(&b).unwrap();
    assert!(read_all(&db.snapshot().unwrap()).unwrap().is_empty());
    db.close().unwrap();
    let db = open(&vfs, &opts).unwrap();
    assert!(read_all(&db.snapshot().unwrap()).unwrap().is_empty());
}

#[test]
fn old_snapshot_pins_files_until_dropped() {
    let vfs = SimVfs::new(3).as_vfs();
    let opts = Options::small_for_tests();
    let db = open(&vfs, &opts).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 2000);
    settle(&db);
    let old = db.snapshot().unwrap();
    let old_model = model.clone();
    for round in 1..6 {
        fill(&db, &mut model, round, 2000);
    }
    settle(&db);
    // The old snapshot still reads its own version, through compactions
    // that replaced every file it references.
    assert!(read_all(&old).unwrap() == old_model);
    assert_eq!(old.get(b"key-000007").unwrap(), old_model.get(&b"key-000007"[..]).cloned());
    let pinned = dir_size(&*vfs);
    let deleted_before = stat(&db, "obsolete_files_deleted");
    drop(old);
    // The files go to the deleter thread (W-13): wait for it.
    db.hooks().wait_idle().unwrap();
    assert!(stat(&db, "obsolete_files_deleted") > deleted_before, "dropping the snapshot should free files");
    assert!(dir_size(&*vfs) < pinned);
    assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
}

#[test]
fn scans_honour_bounds_across_sources() {
    let vfs = SimVfs::new(4).as_vfs();
    let opts = Options::small_for_tests();
    let db = open(&vfs, &opts).unwrap();
    let mut model = Model::new();
    for round in 0..4 {
        fill(&db, &mut model, round, 1500);
        // Deletes and overwrites spread over memtable, L0 and deeper levels.
        let mut b = WriteBatch::new();
        for i in (round as usize..1500).step_by(5) {
            b.delete(format!("key-{i:06}"));
        }
        db.commit(&b).unwrap();
        apply(&mut model, &b);
    }
    let s = db.snapshot().unwrap();
    let keys: Vec<Vec<u8>> = model.keys().cloned().collect();
    for (i, a) in keys.iter().enumerate().step_by(97) {
        let b = &keys[(i + 211).min(keys.len() - 1)];
        for (lo, hi) in [
            (Bound::Included(a.as_slice()), Bound::Included(b.as_slice())),
            (Bound::Excluded(a.as_slice()), Bound::Excluded(b.as_slice())),
            (Bound::Unbounded, Bound::Excluded(b.as_slice())),
            (Bound::Excluded(a.as_slice()), Bound::Unbounded),
            (Bound::Included(b.as_slice()), Bound::Included(a.as_slice())),
        ] {
            let got = s.scan(lo, hi, 1000).unwrap();
            let want: Vec<_> = model
                .iter()
                .filter(|(k, _)| (lo, hi).contains(k.as_slice()))
                .take(1000)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            assert!(got == want, "scan {lo:?}..{hi:?}");
        }
    }
}

#[test]
fn failed_io_poisons_until_reopen() {
    let disk = SimVfs::new(5);
    let vfs = disk.as_vfs();
    let opts = Options { durability: Durability::Buffered, ..Options::small_for_tests() };
    let db = open(&vfs, &opts).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 500);
    disk.crash_after(1, CrashKind::PowerLoss);
    // Keep writing until the failure surfaces (it may hit a background thread first).
    let mut b = WriteBatch::new();
    b.put("k", "v");
    let mut failed = false;
    for _ in 0..10_000 {
        if db.commit(&b).is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert!(matches!(db.commit(&b), Err(Error::Poisoned(_))));
    assert!(matches!(db.snapshot(), Err(Error::Poisoned(_))));
    assert!(matches!(db.close(), Err(Error::Poisoned(_))));
    let image = disk.take_crash_image().unwrap();
    let db = open(&image.as_vfs(), &opts).unwrap();
    let s = db.snapshot().unwrap();
    assert!(read_all(&s).unwrap().len() <= model.len() + 1);
}

/// A MANIFEST that is missing or damaged is `Corruption` (never "start
/// empty", never `Incompatible`), and the open changes nothing (REC-2, HDR-1,
/// HDR-3, MAN-R1).
#[test]
fn missing_or_damaged_manifest_is_corruption() {
    let disk = SimVfs::new(6);
    let vfs = disk.as_vfs();
    let opts = Options::small_for_tests();
    let db = open(&vfs, &opts).unwrap();
    fill(&db, &mut Model::new(), 0, 1000);
    db.close().unwrap();
    let manifest = Path::new(DIR).join("MANIFEST");
    let rejected = |what: &str, rule: &str| {
        let before = disk.dump();
        match open(&vfs, &opts) {
            Err(Error::Corruption(m)) => assert!(m.contains(rule), "{what}: expected {rule}, got {m}"),
            other => panic!("{what}: expected Corruption, got {:?}", other.err()),
        }
        assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    };

    vfs.rename(&manifest, &Path::new(DIR).join("MANIFEST.old")).unwrap();
    rejected("data files without MANIFEST", "the MANIFEST is missing");
    vfs.rename(&Path::new(DIR).join("MANIFEST.old"), &manifest).unwrap();

    let f = vfs.open(&manifest, OpenMode::Existing).unwrap();
    let size = f.len().unwrap();
    let mut intact = vec![0u8; size as usize];
    f.read_at(&mut intact, 0).unwrap();
    // Empty, shorter than the header, cut inside the first record.
    for (len, rule) in [(0, "HDR-1"), (HEADER_LEN as u64 - 1, "HDR-1"), (size - 1, "MAN-R1")] {
        f.set_len(len).unwrap();
        rejected(&format!("MANIFEST of {len} bytes"), rule);
        f.set_len(0).unwrap();
        f.write_at(&intact, 0).unwrap();
        f.sync_data().unwrap();
    }
    // A flipped bit in the header, and in the record.
    disk.corrupt_byte(&manifest, 40, 0x01);
    rejected("bit flip in the header", "HDR-3");
    disk.corrupt_byte(&manifest, 40, 0x01);
    disk.corrupt_byte(&manifest, size - 1, 0x01);
    rejected("bit flip in the record", "MAN-R1");
    disk.corrupt_byte(&manifest, size - 1, 0x01);
    assert!(open(&vfs, &opts).is_ok(), "the restored MANIFEST opens");
}

#[test]
fn stats_are_reported() {
    let vfs = SimVfs::new(7).as_vfs();
    let db = open(&vfs, &Options::small_for_tests()).unwrap();
    fill(&db, &mut Model::new(), 0, 2000);
    let s = db.snapshot().unwrap();
    for i in 0..200 {
        s.get(format!("absent-{i}").as_bytes()).unwrap();
        s.get(format!("key-{:06}", i).as_bytes()).unwrap();
    }
    let names: Vec<String> = db.stats().into_iter().map(|(n, _)| n).collect();
    for n in
        ["l0_files", "l6_bytes", "commits", "flushes", "compaction_bytes_written", "write_stalls", "bloom_negatives"]
    {
        assert!(names.iter().any(|x| x == n), "missing stat {n}");
    }
    assert_eq!(stat(&db, "commits"), 40);
    assert!(stat(&db, "bloom_checks") > 0);
    assert!(stat(&db, "block_cache_hits") + stat(&db, "block_cache_misses") > 0);
}
