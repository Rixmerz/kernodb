//! Read-only open (v1.md §9.5, RO-1..RO-3, HDR-7): it takes the exclusive
//! `LOCK`, classifies the directory and replays the live WALs in memory, and
//! changes nothing else, whatever state it finds; every write returns
//! `ReadOnly`. An unknown `ro_compat` bit makes a read-write request open the
//! same way.

mod common;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::{AccessMode, Config, ReadOnlyReason};
use kerno_testkit::vfs::{SimVfs, assert_dir_unchanged};

use common::*;

fn read_only(durability: Durability) -> Config {
    Config { access: AccessMode::ReadOnly, ..config(durability) }
}

#[test]
fn read_only_open_writes_nothing_clean() {
    let (image, model) = closed_image();
    let disk = SimVfs::restore(&image);
    let before = disk.dump();
    let ops = disk.mutating_ops();
    let db = open_in(&disk.as_vfs(), &read_only(Durability::Sync)).unwrap();
    assert_eq!(db.is_read_only(), Some(ReadOnlyReason::Requested));
    assert!(model_of(&db) == model);
    assert_eq!(stat(&db, "last_open_clean"), 1);
    let report = db.close().unwrap();
    assert_eq!((report.flushed_bytes, report.compacted_bytes), (0, 0));
    assert_eq!(disk.mutating_ops(), ops, "a read-only open and close did mutating operations");
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
}

/// A directory in the worst shape a crash leaves: a torn MANIFEST tail, a WAL
/// with a torn tail, a `MANIFEST.tmp`, an orphan table, unknown files.
#[test]
fn read_only_open_writes_nothing_dirty_torn() {
    let (mut image, model) = unclean_image();
    let manifest = path("MANIFEST");
    image.files.get_mut(&manifest).unwrap().extend_from_slice(&[0xAB; 37]);
    let wal = files_with(&image, "wal").pop().unwrap();
    image.files.get_mut(&wal).unwrap().extend_from_slice(&[0xCD; 21]);
    image.files.insert(path("MANIFEST.tmp"), b"half a checkpoint".to_vec());
    let sst = files_with(&image, "sst").pop().unwrap();
    let orphan = image.files[&sst].clone();
    image.files.insert(path("999999.sst"), orphan);
    image.files.insert(path("notes.txt"), b"not ours".to_vec());
    let disk = SimVfs::restore(&image);
    let before = disk.dump();
    let ops = disk.mutating_ops();
    disk.start_trace();
    let db = open_in(&disk.as_vfs(), &read_only(Durability::Sync)).unwrap();
    assert!(model_of(&db) == model, "the commits of the live WAL are visible in memory");
    drop(db);
    assert_eq!(disk.mutating_ops(), ops, "{:?}", disk.trace());
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    // The same directory opened read-write recovers it to the same state.
    let (_, rw) = open_copy(&image, &config(Durability::Sync));
    assert!(model_of(&rw.unwrap()) == model);
}

#[test]
fn read_only_mutations_return_read_only() {
    let (image, model) = unclean_image();
    let disk = SimVfs::restore(&image);
    let db = open_in(&disk.as_vfs(), &read_only(Durability::Sync)).unwrap();
    let mut b = WriteBatch::new();
    b.put("key-00001", "changed").delete("key-00002");
    for _ in 0..2 {
        assert!(matches!(db.commit(&b), Err(Error::ReadOnly(_))), "commit on a read-only handle");
    }
    assert!(matches!(db.commit(&WriteBatch::new()), Err(Error::ReadOnly(_))), "even an empty batch");
    assert!(matches!(db.hooks().flush(), Err(Error::ReadOnly(_))));
    assert!(matches!(db.hooks().compact_once(), Err(Error::ReadOnly(_))));
    assert!(matches!(db.hooks().checkpoint(), Err(Error::ReadOnly(_))));
    // Reads still work, the handle is not poisoned and nothing changed.
    assert!(db.hooks().wait_idle().is_ok());
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.get(b"key-00001").unwrap(), model.get(&b"key-00001"[..]).cloned());
    assert!(model_of(&db) == model);
    let before = disk.dump();
    db.close().unwrap();
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
}

#[test]
fn unknown_ro_compat_opens_read_only() {
    for ext in ["wal", "sst", "MANIFEST"] {
        let (mut image, model) = unclean_image();
        let file = files_with(&image, ext).into_iter().next().unwrap();
        patch_header(&mut image, &file, |h| or_u64(h, 24, 1 << 3));
        let disk = SimVfs::restore(&image);
        let before = disk.dump();
        // Read-write was asked for.
        let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(db.is_read_only(), Some(ReadOnlyReason::UnknownRoCompat { file: name, bits: 1 << 3 }), "{ext}");
        assert!(model_of(&db) == model, "{ext}: reads work");
        let mut b = WriteBatch::new();
        b.put("x", "y");
        assert!(matches!(db.commit(&b), Err(Error::ReadOnly(m)) if m.contains("ro_compat")), "{ext}");
        drop(db);
        assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    }
    // Several unknown bits, and the bits that are known stay known.
    let (mut image, _) = unclean_image();
    let file = files_with(&image, "sst").into_iter().next().unwrap();
    patch_header(&mut image, &file, |h| or_u64(h, 24, (1 << 63) | (1 << 7)));
    let (_, db) = open_copy(&image, &config(Durability::Sync));
    assert!(
        matches!(db.unwrap().is_read_only(), Some(ReadOnlyReason::UnknownRoCompat { bits, .. }) if bits == (1 << 63) | (1 << 7))
    );
}

#[test]
fn read_only_takes_exclusive_lock() {
    let (image, _) = closed_image();
    let disk = SimVfs::restore(&image);
    let vfs = disk.as_vfs();
    // A read-write handle excludes a read-only one, and the other way round.
    let rw = open_in(&vfs, &config(Durability::Sync)).unwrap();
    assert!(matches!(open_in(&vfs, &read_only(Durability::Sync)), Err(Error::Locked)));
    drop(rw);
    let ro = open_in(&vfs, &read_only(Durability::Sync)).unwrap();
    assert!(matches!(open_in(&vfs, &config(Durability::Sync)), Err(Error::Locked)));
    assert!(matches!(open_in(&vfs, &read_only(Durability::Sync)), Err(Error::Locked)));
    drop(ro);
    assert!(open_in(&vfs, &config(Durability::Sync)).is_ok(), "released with the handle");
}

#[test]
fn read_only_reports_orphans_without_deleting() {
    let (mut image, model) = closed_image();
    let (replay, _) = manifest_of(&image);
    let (next_file, last_seq) = (replay.state.next_file, replay.state.last_seq);
    // A number below `next_file` that no file uses: REC-8 (a table nothing
    // references), REC-8 again for a log, and REC-7 for a frameless log the
    // next session created before it died.
    let used: Vec<u64> = image.files.keys().filter_map(|p| p.file_stem()?.to_str()?.parse().ok()).collect();
    let mut free = (1..next_file).filter(|n| !used.contains(n));
    let (orphan_sst, orphan_wal) = (free.next().unwrap(), free.next().unwrap());
    let sst = files_with(&image, "sst").pop().unwrap();
    let copy = image.files[&sst].clone();
    image.files.insert(path(&format!("{orphan_sst:06}.sst")), copy);
    image.files.insert(path(&format!("{orphan_wal:06}.wal")), wal_bytes(orphan_wal, last_seq + 1, &[]));
    let next_session_wal = format!("{next_file:06}.wal");
    image.files.insert(path(&next_session_wal), wal_bytes(next_file, last_seq + 1, &[]));
    image.files.insert(path("MANIFEST.tmp"), b"stale".to_vec());
    let orphan_sst = format!("{orphan_sst:06}.sst");
    let orphan_wal = format!("{orphan_wal:06}.wal");
    let disk = SimVfs::restore(&image);
    let db = open_in(&disk.as_vfs(), &read_only(Durability::Sync)).unwrap();
    let mut orphans = db.recovery_orphans().to_vec();
    orphans.sort();
    for name in [&orphan_sst, &orphan_wal, &next_session_wal] {
        assert!(orphans.contains(name), "{name} is not reported: {orphans:?}");
    }
    assert!(model_of(&db) == model);
    drop(db);
    assert_dir_unchanged(&image, &disk.dump(), &["LOCK"]);
    let vfs = disk.as_vfs();
    for name in [&orphan_sst, &orphan_wal, &next_session_wal, &"MANIFEST.tmp".to_string()] {
        assert!(vfs.exists(&path(name)).unwrap(), "{name} was deleted by a read-only open");
    }
    // A read-write open deletes them.
    let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
    assert!(db.recovery_orphans().contains(&orphan_sst));
    for name in [&orphan_sst, &orphan_wal, &next_session_wal, &"MANIFEST.tmp".to_string()] {
        assert!(!vfs.exists(&path(name)).unwrap(), "{name} survived a read-write open");
    }
}
