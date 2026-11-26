//! Read-only open (v1.md §9.5, RO-2, RO-3, HDR-7): it takes no lock (the RO-1
//! exclusive `LOCK` is replaced by `DESIGN.md`, "Concurrent read-only
//! handles"), classifies the directory and replays the live WALs in memory,
//! and changes nothing else, whatever state it finds; every write returns
//! `ReadOnly`; `refresh` moves it to newer commits. An unknown `ro_compat` bit makes a read-write request open the
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
        // It holds the LOCK like any read-write open: a writer is refused, a plain reader is not.
        assert!(matches!(open_in(&disk.as_vfs(), &config(Durability::Sync)), Err(Error::Locked)), "{ext}: lock");
        open_in(&disk.as_vfs(), &read_only(Durability::Sync)).expect("a reader beside it");
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

/// A read-only open takes no lock: it opens beside a live writer and beside
/// other readers, sees what the writer has committed so far, and does not stop
/// a writer from opening later. Writers stay exclusive against each other.
#[test]
fn read_only_takes_no_lock() {
    let (image, model) = unclean_image();
    let disk = SimVfs::restore(&image);
    let vfs = disk.as_vfs();
    let rw = open_in(&vfs, &config(Durability::Sync)).unwrap();
    let ro1 = open_in(&vfs, &read_only(Durability::Sync)).expect("a reader beside the writer");
    let ro2 = open_in(&vfs, &read_only(Durability::Sync)).expect("two readers");
    assert!(model_of(&ro1) == model && model_of(&ro2) == model);
    assert!(matches!(open_in(&vfs, &config(Durability::Sync)), Err(Error::Locked)), "writers stay exclusive");
    drop(rw);
    let rw = open_in(&vfs, &config(Durability::Sync)).expect("readers hold nothing a writer needs");
    drop((ro1, ro2));
    drop(rw);
}

/// A reader sees one committed state and moves to newer ones only on
/// `refresh`; a snapshot taken before keeps reading the state it was taken in,
/// also after the writer has flushed, compacted and deleted the tables it used.
#[test]
fn read_only_refreshes_to_newer_commits() {
    let (image, mut model) = unclean_image();
    let disk = SimVfs::restore(&image);
    let vfs = disk.as_vfs();
    let rw = open_in(&vfs, &config(Durability::Sync)).unwrap();
    let ro = open_in(&vfs, &read_only(Durability::Sync)).unwrap();
    let old_model = model.clone();
    let old = ro.snapshot().unwrap();
    // Nothing new: refresh answers the same sequence and changes nothing.
    let seq0 = ro.refresh().unwrap();
    assert!(model_of(&ro) == model);
    put(&rw, &mut model, "key-00001", b"newer");
    assert!(model_of(&ro) == old_model, "a commit is not visible before refresh");
    let seq1 = ro.refresh().unwrap();
    assert!(seq1 > seq0 && model_of(&ro) == model, "refresh sees the commit in the live WAL");
    // Enough writes to flush, compact and delete the tables the reader holds.
    fill(&rw, &mut model, 1000, 600);
    rw.compact_range(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded).unwrap();
    let seq2 = ro.refresh().unwrap();
    assert!(seq2 > seq1 && model_of(&ro) == model, "refresh after a flush and a compaction");
    assert_eq!(old.get(b"key-00001").unwrap(), old_model.get(&b"key-00001"[..]).cloned(), "the old snapshot reads on");
    assert!(matches!(ro.commit(&WriteBatch::new()), Err(Error::ReadOnly(_))), "still read-only after a refresh");
}

#[test]
fn only_a_read_only_handle_refreshes() {
    let (image, _) = closed_image();
    let disk = SimVfs::restore(&image);
    let rw = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    assert!(matches!(rw.refresh(), Err(Error::InvalidArgument(m)) if m.contains("AccessMode::ReadOnly")));
    drop(rw);
    // A handle that is read-only for an unknown ro_compat bit holds the LOCK:
    // nothing changes under it, so it does not refresh either.
    let (mut image, _) = unclean_image();
    let file = files_with(&image, "sst").into_iter().next().unwrap();
    patch_header(&mut image, &file, |h| or_u64(h, 24, 1 << 3));
    let disk = SimVfs::restore(&image);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    assert!(db.is_read_only().is_some());
    assert!(matches!(db.refresh(), Err(Error::InvalidArgument(_))));
}

/// A reader that keeps its handle while the directory is emptied and filled
/// with another, intact database must not adopt it: its tables, cache and
/// snapshots belong to the first one, so `refresh` says `Corruption`, and the
/// handle goes on reading the database it opened.
#[test]
fn refresh_refuses_a_directory_that_now_holds_another_database() {
    use kerno_core::vfs::OpenMode;
    let (image, model) = closed_image();
    let other = {
        let disk = SimVfs::new(3);
        let mut cfg = config(Durability::Sync);
        cfg.test.db_id = Some([0x5A; 16]);
        let db = open_in(&disk.as_vfs(), &cfg).unwrap();
        fill(&db, &mut kerno_testkit::harness::Model::new(), 0, 50);
        db.close().unwrap();
        disk.dump()
    };
    let id = |i: &kerno_testkit::vfs::DirImage| manifest_of(i).0.header.db_id;
    assert_ne!(id(&image), id(&other), "two databases get two ids");
    let disk = SimVfs::restore(&image);
    let vfs = disk.as_vfs();
    let ro = open_in(&vfs, &read_only(Durability::Sync)).unwrap();
    let dir = std::path::Path::new(DIR);
    for name in vfs.list(dir).unwrap().into_iter().filter(|n| n != "LOCK") {
        vfs.remove_file(&dir.join(name)).unwrap();
    }
    for (file, bytes) in other.files.iter().filter(|(p, _)| p.file_name().is_some_and(|n| n != "LOCK")) {
        vfs.open(file, OpenMode::CreateNew).unwrap().write_at(bytes, 0).unwrap();
    }
    let err = ro.refresh().expect_err("another database");
    assert!(matches!(&err, Error::Corruption(m) if m.contains("another database")), "{err:?}");
    assert!(model_of(&ro) == model, "the handle still reads the database it opened");
}

/// The writer flushes and compacts while the reader is opening: the tables the
/// reader listed are gone when it opens them. The open reads again and ends
/// with the writer's final state instead of failing.
#[test]
fn read_only_open_reads_again_when_the_writer_changes_the_files() {
    use kerno_testkit::vfs::{HookAction, HookOpKind, HookPoint, HookVfs};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    let (image, model) = unclean_image();
    let disk = SimVfs::restore(&image);
    let writer = Arc::new(open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap());
    let model = Arc::new(Mutex::new(model));
    let fired = Arc::new(AtomicUsize::new(0));
    let lists = Arc::new(AtomicUsize::new(0));
    let hook = {
        let (writer, model, fired, lists) = (writer.clone(), model.clone(), fired.clone(), lists.clone());
        move |op: &kerno_testkit::vfs::HookOp<'_>| {
            if op.point == HookPoint::Before && op.kind == HookOpKind::List {
                lists.fetch_add(1, Ordering::SeqCst);
            }
            let is_sst = op.path.extension().is_some_and(|e| e == "sst");
            // The first time the reader opens a table, the writer replaces them all.
            if op.point == HookPoint::Before
                && op.kind == HookOpKind::Open
                && is_sst
                && fired.fetch_add(1, Ordering::SeqCst) == 0
            {
                fill(&writer, &mut model.lock().unwrap(), 2000, 600);
                writer.compact_range(std::ops::Bound::Unbounded, std::ops::Bound::Unbounded).unwrap();
            }
            HookAction::Proceed
        }
    };
    let reader_vfs = HookVfs::wrap(disk.as_vfs(), hook);
    let ro = open_in(&reader_vfs, &read_only(Durability::Sync)).expect("the open reads again");
    assert!(fired.load(Ordering::SeqCst) > 1, "the open went back to the tables after the writer changed them");
    assert!(model_of(&ro) == *model.lock().unwrap(), "one consistent, newer state");
    drop(ro);
}

/// Damage that does not change between two reads is reported at once, as
/// before: the retry is for a writer's changes, never a way to skip damage.
#[test]
fn read_only_open_reports_damage_without_retrying() {
    use kerno_testkit::vfs::{HookAction, HookOpKind, HookPoint, HookVfs};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let (mut image, _) = closed_image();
    let sst = files_with(&image, "sst").pop().unwrap();
    let last = image.files[&sst].len() - 1;
    image.files.get_mut(&sst).unwrap()[last] ^= 0xFF; // the footer
    let disk = SimVfs::restore(&image);
    let lists = Arc::new(AtomicUsize::new(0));
    let counter = lists.clone();
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        if op.point == HookPoint::Before && op.kind == HookOpKind::List {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        HookAction::Proceed
    });
    let err = open_in(&vfs, &read_only(Durability::Sync)).err().expect("a damaged table");
    assert!(matches!(err, Error::Corruption(_)), "{err:?}");
    // open's own listing, which is also the read's "before" and the one the read works from,
    // then the listing after the error.
    assert_eq!(lists.load(Ordering::SeqCst), 2, "damage is not read again");
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
