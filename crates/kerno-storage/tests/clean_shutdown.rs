//! The clean shutdown and the session protocol around it (v1.md W-9, S-1..S-6,
//! W-8, W-10): a closed database holds one `CLEAN` snapshot and no WAL, a
//! reopen replays nothing and costs two syncs, a flush costs three, a crash at
//! any point of a close loses nothing, and the MANIFEST does not grow without
//! bound.

mod common;

use kerno_core::contract::Engine;
use kerno_core::{Durability, WriteBatch};
use kerno_storage::Config;
use kerno_storage::format::manifest::Edit;
use kerno_testkit::harness::Model;
use kerno_testkit::vfs::{CountingVfs, CrashKind, OpKind, SimVfs};

use common::*;

#[test]
fn clean_close_manifest_is_single_clean_snapshot() {
    let (image, _) = closed_image();
    let (replay, records) = manifest_of(&image);
    assert_eq!(records.len(), 1, "a clean close leaves one record: {records:?}");
    assert!(replay.clean, "MAN-R4: the last record is a CLEAN snapshot");
    let Edit::Snapshot { clean, next_file, log_number, files, .. } = &records[0].1.edit else {
        panic!("record 0 is not a snapshot: {:?}", records[0].1.edit)
    };
    assert!(*clean);
    assert_eq!(log_number, next_file, "a CLEAN snapshot needs no WAL: log_number = next_file");
    assert!(!files.is_empty(), "the data is in tables");
    // LOCK, MANIFEST and the tables, nothing else: no WAL, no MANIFEST.tmp.
    let names: Vec<String> =
        image.files.keys().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    for n in &names {
        assert!(n == "LOCK" || n == "MANIFEST" || n.ends_with(".sst"), "unexpected file {n} after a clean close");
    }
    // Every table of the snapshot is on disk, and no other one.
    let mut listed: Vec<String> = files.iter().map(|f| format!("{:06}.sst", f.number)).collect();
    listed.sort();
    let mut on_disk: Vec<String> = names.into_iter().filter(|n| n.ends_with(".sst")).collect();
    on_disk.sort();
    assert_eq!(listed, on_disk);
}

#[test]
fn clean_reopen_replays_no_wal() {
    let (image, model) = closed_image();
    let (disk, db) = open_copy(&image, &config(Durability::Sync));
    let db = db.unwrap();
    assert_eq!(stat(&db, "last_open_replayed_bytes"), 0);
    assert_eq!(stat(&db, "last_open_clean"), 1);
    assert!(model_of(&db) == model);
    // The session opened with one appended record (S-4), not a new generation.
    let (replay, records) = manifest_of(&disk.dump());
    assert_eq!((replay.header.file_number, records.len()), (2, 2), "{records:?}");
    assert!(matches!(records[1].1.edit, Edit::SessionOpen { .. }));
    assert!(!replay.clean, "the first frame may be written: the state is no longer CLEAN (W-8)");
    // One live WAL, empty.
    let wals = files_with(&disk.dump(), "wal");
    assert_eq!(wals.len(), 1);
    assert_eq!(disk.dump().files[&wals[0]].len(), 96, "a header and no frame");
}

#[test]
fn clean_reopen_costs_two_syncs() {
    let (image, _) = closed_image();
    let (vfs, io) = CountingVfs::wrap(SimVfs::restore(&image).as_vfs());
    let before = io.snapshot();
    let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
    let d = io.snapshot().since(&before);
    // S-3 creates the WAL (`sync_data` of its header, `sync_dir`), S-4 appends
    // SESSION_OPEN (`sync_data`).
    assert_eq!((d.dir_syncs, d.file_syncs), (1, 2), "{d:?}");
    drop(db);
}

#[test]
fn flush_costs_three_syncs() {
    let disk = SimVfs::new(3);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 30);
    disk.start_trace();
    db.hooks().flush().unwrap();
    let trace = disk.trace();
    let syncs = |phase: &str| {
        trace
            .iter()
            .filter(|t| t.op.phase == phase && matches!(t.op.kind, OpKind::SyncData | OpKind::SyncDir))
            .map(|t| (t.op.kind, t.op.path.file_name().unwrap().to_string_lossy().into_owned()))
            .collect::<Vec<_>>()
    };
    // The table, its directory entry, the FLUSH record (the M0 also rewrote
    // MANIFEST.tmp and synced the directory again: four).
    let flush = syncs("flush");
    assert_eq!(flush.len(), 3, "{flush:?}");
    assert_eq!(flush[0].0, OpKind::SyncData);
    assert!(flush[0].1.ends_with(".sst"), "{flush:?}");
    assert_eq!(flush[1].0, OpKind::SyncDir);
    assert_eq!((flush[2].0, flush[2].1.as_str()), (OpKind::SyncData, "MANIFEST"));
    // Rotation costs the header sync and the directory sync of the new WAL.
    assert_eq!(syncs("rotate").len(), 2);
    let (replay, records) = manifest_of(&disk.dump());
    assert_eq!(replay.header.file_number, 1, "no new generation");
    assert!(matches!(records.last().unwrap().1.edit, Edit::Flush { .. }), "{:?}", records.last());
}

/// Crashes at every mutating operation of a close, in every way: nothing
/// that was committed is lost and nothing is reported as damage.
#[test]
fn crash_at_every_point_of_close() {
    let build = |seed: u64| {
        let disk = SimVfs::new(seed);
        if seed % 2 == 1 {
            disk.set_tear_unit(1);
        }
        let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
        let mut model = Model::new();
        fill(&db, &mut model, 0, 120);
        (disk, db, model)
    };
    let (disk, db, _) = build(0);
    let before = disk.mutating_ops();
    db.close().unwrap();
    let total = disk.mutating_ops() - before;
    assert!(total >= 8, "a close of a database with data does real work: {total}");
    for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill, CrashKind::ProcessKillKeepPending] {
        for point in 1..=total + 2 {
            let (disk, db, model) = build(point);
            disk.crash_after(point, kind);
            let closed = db.close();
            let image = match disk.take_crash_image() {
                Some(image) => image,
                None => {
                    assert!(closed.is_ok(), "{kind:?} #{point}: close failed without a crash");
                    disk.crash_now(kind)
                }
            };
            let ctx = format!("{kind:?} before op #{point} of the close");
            let db = open_in(&image.as_vfs(), &config(Durability::Sync)).unwrap_or_else(|e| panic!("{ctx}: {e}"));
            assert!(model_of(&db) == model, "{ctx}: the recovered state differs");
            // And it keeps working.
            let mut b = WriteBatch::new();
            b.put("after", "x");
            assert_eq!(db.commit(&b).unwrap(), 121, "{ctx}");
            db.close().unwrap_or_else(|e| panic!("{ctx}: second close: {e}"));
        }
    }
}

#[test]
fn reopen_invalidates_clean_before_first_frame() {
    let (image, _) = closed_image();
    let disk = SimVfs::restore(&image);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    disk.start_trace();
    let mut b = WriteBatch::new();
    b.put("first", "x");
    // Die right before the first frame is written.
    disk.crash_when(|op| op.phase == "commit" && op.kind == OpKind::WriteAt, 1, CrashKind::ProcessKill);
    assert!(db.commit(&b).is_err(), "the crash refused the write");
    let after = disk.take_crash_image().expect("the crash fired").dump();
    let (replay, _) = manifest_of(&after);
    assert!(!replay.clean, "SESSION_OPEN is durable before any frame (W-8): the state is not CLEAN");
    // So a frame that did reach the disk could never sit beside a CLEAN state:
    // run it again without the crash and look at the order of operations.
    let disk = SimVfs::restore(&image);
    disk.start_trace();
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    db.commit(&b).unwrap();
    let ops = disk.trace();
    let session_record = ops
        .iter()
        .position(|t| t.op.phase == "session" && t.op.kind == OpKind::SyncData && t.op.path.ends_with("MANIFEST"))
        .expect("SESSION_OPEN is synced");
    let first_frame = ops
        .iter()
        .position(|t| t.op.phase == "commit" && t.op.kind == OpKind::WriteAt)
        .expect("the commit writes a frame");
    assert!(session_record < first_frame, "SESSION_OPEN (#{session_record}) after the first frame (#{first_frame})");
}

#[test]
fn unknown_compat_bit_dropped_on_checkpoint() {
    let (mut image, model) = unclean_image();
    for ext in ["wal", "sst", "MANIFEST"] {
        for file in files_with(&image, ext) {
            patch_header(&mut image, &file, |h| or_u64(h, 16, 1 << 5));
        }
    }
    let (disk, db) = open_copy(&image, &config(Durability::Sync));
    let db = db.unwrap();
    assert!(model_of(&db) == model);
    // The open appended to the MANIFEST (or checkpointed it); a file that was
    // only appended to keeps the bit it had.
    let bit = |d: &SimVfs| {
        let image = d.dump();
        let bytes = &image.files[&path("MANIFEST")];
        le_u64(bytes, 16)
    };
    let after_open = bit(&disk);
    db.hooks().checkpoint().unwrap();
    let after_checkpoint = bit(&disk);
    assert_eq!(after_checkpoint, 0, "a new generation carries only the bits this writer implements (HDR-8)");
    assert!(after_open == 0 || after_open == 1 << 5, "{after_open:#x}");
    db.close().unwrap();
    let (_, reopened) = open_copy(&disk.dump(), &config(Durability::Sync));
    assert!(model_of(&reopened.unwrap()) == model);
}

/// Appending forever would make the MANIFEST (and its replay) grow without
/// bound: past `max(4 MiB, 4 x record 0)` of appended bytes the next change is
/// a checkpoint (W-10).
#[test]
fn manifest_checkpoint_bounds_file_size() {
    let disk = SimVfs::new(9);
    // 512-byte keys make every FileMeta over a kilobyte, so that a few
    // thousand flushes cross the 4 MiB floor.
    let cfg = Config { write_buffer_bytes: 4 << 10, ..config(Durability::Buffered) };
    let db = open_in(&disk.as_vfs(), &cfg).unwrap();
    let mut model = Model::new();
    let mut largest = 0u64;
    let mut i = 0u32;
    while stat(&db, "manifest_checkpoints") == 0 {
        i += 1;
        assert!(
            i < 20_000,
            "no checkpoint after {i} commits; the MANIFEST holds {} bytes",
            stat(&db, "manifest_bytes")
        );
        let key = format!("{i:0512}");
        put(&db, &mut model, &key, &[7u8; 4096]);
        largest = largest.max(stat(&db, "manifest_bytes"));
    }
    // The checkpoint came when the appended bytes passed the floor, and the
    // file started over. (Sampled after each commit, so the peak is a few
    // records short of the exact size at the trigger.)
    let floor = 4u64 << 20;
    assert!(largest > floor - (64 << 10) && largest < floor + (256 << 10), "the file peaked at {largest}");
    // (Record 0 now holds every table of the database.)
    assert!(stat(&db, "manifest_bytes") < largest / 2, "{} bytes after the checkpoint", stat(&db, "manifest_bytes"));
    let image = disk.dump();
    let (replay, _) = manifest_of(&image);
    assert!(replay.header.file_number >= 2, "a new generation");
    db.close().unwrap();
    let (_, reopened) = open_copy(&disk.dump(), &cfg);
    assert!(model_of(&reopened.unwrap()) == model);
}
