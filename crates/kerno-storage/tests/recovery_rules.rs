//! The recovery decision table (v1.md §9, REC-1..REC-13) and the MANIFEST
//! rules it leans on (MAN-R1..R4): which damage is a torn tail and which is
//! corruption, what a clean state may find beside it, and that a crash at any
//! step of a session start is recoverable (REC-10c).
//!
//! Every test that expects a rejection checks that the rejected directory is
//! byte for byte what it was (the `LOCK` file aside).

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::{OpenMode, Vfs};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::format::header::Header;
use kerno_storage::format::manifest::{Edit, Record, encode_manifest, seal_record};
use kerno_storage::format::wal::{WalScanParams, scan_wal};
use kerno_storage::{Config, KernoDb};
use kerno_testkit::harness::{Model, apply};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{
    CrashKind, DirImage, HookAction, HookOpKind, HookPoint, HookVfs, OpKind, SimVfs, assert_dir_unchanged,
};
use parking_lot::Mutex;

use common::*;

/// Opens a copy of `image` read-write; the open must fail and leave the
/// directory as it was.
fn reject(image: &DirImage) -> Error {
    let disk = SimVfs::restore(image);
    let before = disk.dump();
    let err = open_in(&disk.as_vfs(), &config(Durability::Sync)).err().expect("the open must fail");
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    err
}

fn assert_corruption(e: &Error, needle: &str) {
    assert!(matches!(e, Error::Corruption(m) if m.contains(needle)), "expected Corruption({needle}), got {e:?}");
}

/// Opens a copy of `image`, which must work, and checks the model.
fn opens_with(image: &DirImage, model: &Model) -> (SimVfs, KernoDb) {
    let (disk, db) = open_copy(image, &config(Durability::Sync));
    let db = db.unwrap_or_else(|e| panic!("open failed: {e}"));
    assert!(&model_of(&db) == model, "the opened database differs from the model");
    (disk, db)
}

fn put_batch(key: &str, value: &str) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.put(key, value);
    b
}

/// The sequence after the last valid frame of a WAL file, from its bytes.
fn next_seq_of(bytes: &[u8], number: u64) -> u64 {
    let params = WalScanParams { file_number: number, db_id: DB_ID, expected_seq: le_u64(bytes, 64) };
    scan_wal(&mut &bytes[..], &params, |_, _| Ok(())).map_err(|e| e.into_format()).unwrap().next_seq
}

fn newest_wal(image: &DirImage) -> (PathBuf, u64) {
    let wal = files_with(image, "wal").pop().expect("a live WAL");
    let n = file_number(&wal);
    (wal, n)
}

/// A torn tail after the last record: bytes of a record that never finished.
fn with_torn_manifest_tail(image: &DirImage) -> DirImage {
    let mut image = image.clone();
    let m = image.files.get_mut(&path("MANIFEST")).unwrap();
    let (_, records) = {
        let tmp = DirImage { files: [(path("MANIFEST"), m.clone())].into(), ..Default::default() };
        manifest_of(&tmp)
    };
    // The first half of a copy of the last record: a plausible interrupted write.
    let (last_at, _) = records.last().unwrap();
    let half = m[*last_at as usize..*last_at as usize + 40].to_vec();
    m.extend_from_slice(&half);
    image
}

#[test]
fn manifest_torn_tail_accepted_after_crash() {
    let (image, model) = unclean_image();
    for tail in [vec![0xAB; 37], vec![0u8; 100], vec![0xFF]] {
        let mut torn = image.clone();
        torn.files.get_mut(&path("MANIFEST")).unwrap().extend_from_slice(&tail);
        let (_, db) = opens_with(&torn, &model);
        drop(db);
    }
    let (_, db) = opens_with(&with_torn_manifest_tail(&image), &model);
    drop(db);
    // The torn tail of a record that really was being written when the
    // machine died: every crash point of a sweep of the MANIFEST ops lands
    // here (see manifest_log.rs); one by hand: power loss in the middle of
    // the append of the first FLUSH record.
    let disk = SimVfs::new(5);
    disk.set_tear_unit(1);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut m = Model::new();
    disk.crash_when(
        |op| op.phase == "flush" && op.path.ends_with("MANIFEST") && op.kind == OpKind::WriteAt,
        1,
        CrashKind::PowerLoss,
    );
    let mut i = 0u64;
    let mut acked = 0;
    while !disk.has_crashed() {
        assert!(i < 200, "no flush");
        let mut b = WriteBatch::new();
        b.put(format!("k{i}"), vec![1u8; 700]);
        match db.commit(&b) {
            Ok(seq) => {
                apply(&mut m, &b);
                acked = seq;
            }
            Err(e) => assert!(disk.has_crashed(), "{e}"),
        }
        i += 1;
    }
    let image = disk.take_crash_image().unwrap();
    let db = open_in(&image.as_vfs(), &config(Durability::Sync)).unwrap();
    let seq = db.snapshot().unwrap().seq();
    // The commit whose flush was cut off was acknowledged by its WAL sync.
    assert!(seq >= acked && seq <= acked + 1, "{seq} after {acked} acknowledged commits");
    assert!(model_of(&db).len() as u64 >= acked.min(1));
}

#[test]
fn manifest_mid_damage_is_corruption() {
    let (image, _) = unclean_image();
    let (_, records) = manifest_of(&image);
    assert!(records.len() >= 4, "the image needs several records: {}", records.len());
    let len = image.files[&path("MANIFEST")].len() as u64;
    for (i, (at, _)) in records.iter().enumerate().take(records.len() - 1) {
        let end = records[i + 1].0;
        // The head, the middle and the last byte of the record.
        for off in [*at, at + 3, at + 8, at + 17, (at + end) / 2, end - 45, end - 1] {
            assert!(off < len);
            for mask in [0x01u8, 0x80] {
                let mut damaged = image.clone();
                damaged.files.get_mut(&path("MANIFEST")).unwrap()[off as usize] ^= mask;
                let e = reject(&damaged);
                assert!(matches!(e, Error::Corruption(_)), "record {i}, byte {off}, mask {mask:#x}: {e:?}");
            }
        }
    }
}

/// A database whose last MANIFEST record is the first `FLUSH`, with commits
/// in the log of the new memtable (so the old WAL is gone).
fn after_first_flush() -> (DirImage, Model) {
    let disk = SimVfs::new(4);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    let mut i = 0;
    while stat(&db, "flushes") == 0 {
        assert!(i < 200, "no flush");
        put(&db, &mut model, &format!("key-{i:04}"), &[i as u8; 700]);
        i += 1;
    }
    fill(&db, &mut model, 1000, 5);
    drop(db);
    (disk.dump(), model)
}

#[test]
fn damaged_last_flush_record_with_deleted_wal_is_corruption() {
    let (image, model) = after_first_flush();
    let (replay, records) = manifest_of(&image);
    let (at, last) = records.last().unwrap();
    assert!(matches!(last.edit, Edit::Flush { .. }), "the image ends with a FLUSH: {last:?}");
    drop(replay);
    // Intact, it opens.
    drop(opens_with(&image, &model));
    // With the last record damaged the tail is torn, the MANIFEST rolls back
    // to before the flush, and the log that state needs is gone (the FLUSH
    // was durable, so its log was deleted): recovery must not take that for
    // a smaller database.
    let mut damaged = image.clone();
    damaged.files.get_mut(&path("MANIFEST")).unwrap()[*at as usize + 30] ^= 0x04;
    assert_corruption(&reject(&damaged), "REC-9");
    // Truncating the record away is the same.
    let mut cut = image.clone();
    cut.files.get_mut(&path("MANIFEST")).unwrap().truncate(*at as usize);
    assert_corruption(&reject(&cut), "REC-9");
}

#[test]
fn required_wal_missing_is_corruption() {
    let (mut image, model) = unclean_image();
    let (replay, _) = manifest_of(&image);
    let oldest = path(&format!("{:06}.wal", replay.state.log_number));
    assert!(image.files.contains_key(&oldest), "log_number names a WAL that exists");
    drop(opens_with(&image, &model));
    image.files.remove(&oldest);
    assert_corruption(&reject(&image), "REC-9");
}

#[test]
fn clean_state_wal_with_frames_beyond_next_file_is_corruption() {
    let (mut image, model) = closed_image();
    let (replay, _) = manifest_of(&image);
    let (next_file, last_seq) = (replay.state.next_file, replay.state.last_seq);
    // The commit of a session whose SESSION_OPEN was lost: after a clean
    // close, a log numbered `next_file` or more may only be empty (W-8).
    let wal = wal_bytes(next_file, last_seq + 1, &[put_batch("late", "commit")]);
    image.files.insert(path(&format!("{next_file:06}.wal")), wal);
    assert_corruption(&reject(&image), "REC-7");
    // The same log without the frame is an orphan.
    image.files.insert(path(&format!("{next_file:06}.wal")), wal_bytes(next_file, last_seq + 1, &[]));
    drop(opens_with(&image, &model));
}

#[test]
fn clean_state_sst_beyond_next_file_is_corruption() {
    let (mut image, _) = closed_image();
    let (replay, _) = manifest_of(&image);
    let next_file = replay.state.next_file;
    let sst = files_with(&image, "sst").pop().unwrap();
    let bytes = image.files[&sst].clone();
    image.files.insert(path(&format!("{:06}.sst", next_file + 3)), bytes);
    assert_corruption(&reject(&image), "REC-6");
}

#[test]
fn frameless_wal_after_clean_is_orphan() {
    let (image, model) = closed_image();
    let (replay, _) = manifest_of(&image);
    let (next_file, last_seq) = (replay.state.next_file, replay.state.last_seq);
    let valid = wal_bytes(next_file, last_seq + 1, &[]);
    let mut flipped = valid.clone();
    flipped[20] ^= 0x10;
    let variants: [(&str, Vec<u8>); 5] = [
        ("header only", valid.clone()),
        ("empty file", Vec::new()),
        ("half a header", valid[..50].to_vec()),
        ("zeros", vec![0; 96]),
        ("bit flip in the header", flipped),
    ];
    for (what, bytes) in variants {
        let mut image = image.clone();
        let name = format!("{next_file:06}.wal");
        image.files.insert(path(&name), bytes);
        let (disk, db) = opens_with(&image, &model);
        assert!(db.recovery_orphans().contains(&name), "{what}: {:?}", db.recovery_orphans());
        // The open deleted it and went on with a log of its own.
        assert!(!disk.dump().files.contains_key(&path(&name)), "{what}: still there");
        let mut m = model.clone();
        put(&db, &mut m, "new", b"x");
        db.close().unwrap_or_else(|e| panic!("{what}: {e}"));
    }
}

#[test]
fn writer_checkpoints_instead_of_appending_after_torn_tail() {
    let (image, model) = unclean_image();
    let (before, records_before) = manifest_of(&image);
    // Intact tail: the session appends one record to the same generation.
    let (disk, db) = opens_with(&image, &model);
    assert_eq!((stat(&db, "manifest_records"), stat(&db, "manifest_checkpoints")), (1, 0));
    let (replay, records) = manifest_of(&disk.dump());
    assert_eq!(replay.header.file_number, before.header.file_number);
    assert_eq!(records.len(), records_before.len() + 1);
    drop(db);
    // Torn tail: nothing is appended after it (W-7); a new generation holds
    // one record, and the garbage is gone.
    let torn = with_torn_manifest_tail(&image);
    let (disk, db) = opens_with(&torn, &model);
    assert_eq!((stat(&db, "manifest_records"), stat(&db, "manifest_checkpoints")), (0, 1));
    let (replay, records) = manifest_of(&disk.dump());
    assert_eq!(replay.header.file_number, before.header.file_number + 1);
    assert_eq!(records.len(), 1);
    let Edit::Snapshot { clean, log_number, .. } = &records[0].1.edit else { panic!("{:?}", records[0]) };
    assert!(!clean);
    assert!(disk.dump().files.contains_key(&path(&format!("{log_number:06}.wal"))), "the snapshot names the new WAL");
    assert_eq!(replay.end, kerno_storage::format::End::Clean, "no torn bytes left");
    db.close().unwrap();
}

#[test]
fn non_newest_wal_torn_is_corruption() {
    let (mut image, model) = unclean_image();
    let (a, a_number) = newest_wal(&image);
    let a_bytes = image.files[&a].clone();
    let b_number = a_number + 50;
    let b_batches = [put_batch("b-one", "1"), put_batch("b-two", "2")];
    let b = wal_bytes(b_number, next_seq_of(&a_bytes, a_number), &b_batches);
    image.files.insert(path(&format!("{b_number:06}.wal")), b);
    let mut model_ab = model.clone();
    for batch in &b_batches {
        apply(&mut model_ab, batch);
    }
    // A chain of two intact logs replays both.
    drop(opens_with(&image, &model_ab));
    // A torn tail on the older one: the newer log proves its frames were
    // durable (REC-10a).
    for tail in [vec![0x5A; 30], vec![0u8, 0, 0, 1], vec![0xFF]] {
        let mut garbage = image.clone();
        garbage.files.get_mut(&a).unwrap().extend_from_slice(&tail);
        assert_corruption(&reject(&garbage), "REC-10a");
    }
    // A cut that takes frames off it also breaks the chain (the newer log
    // starts where the older one no longer ends).
    for cut in [1usize, 10, 40] {
        let mut torn = image.clone();
        let bytes = torn.files.get_mut(&a).unwrap();
        bytes.truncate(bytes.len() - cut);
        assert!(matches!(reject(&torn), Error::Corruption(_)));
    }
    // A broken header on the older one, likewise.
    let mut broken = image.clone();
    broken.files.get_mut(&a).unwrap()[..96].fill(0);
    assert!(matches!(reject(&broken), Error::Corruption(_)));
    // Torn newest log with frames (REC-10b) is fine.
    let mut torn_b = image.clone();
    let bytes = torn_b.files.get_mut(&path(&format!("{b_number:06}.wal"))).unwrap();
    bytes.truncate(bytes.len() - 5);
    let mut model_a_b1 = model;
    apply(&mut model_a_b1, &b_batches[0]);
    drop(opens_with(&torn_b, &model_a_b1));
}

/// The engine's own protocol makes a crash between S-3 (the new log exists)
/// and S-4 (its `SESSION_OPEN` is durable) recoverable (REC-10c): a dirty
/// close with a torn tail, a crash in the middle of the session start, a
/// reopen, another crash, a reopen: never `Corruption`, and the state is
/// always the one the first crash left.
#[test]
fn chained_crash_during_session_start_after_torn_wal() {
    for durability in [Durability::Sync, Durability::Buffered] {
        for seed in 0..60u64 {
            let ctx = format!("{durability:?} seed {seed}");
            let disk = SimVfs::new(seed);
            disk.set_tear_unit(if seed % 2 == 0 { 1 } else { 512 });
            let cfg = config(durability);
            let db = open_in(&disk.as_vfs(), &cfg).unwrap();
            let mut rng = Rng::new(seed);
            let mut batches = Vec::new();
            let mut acked = 0;
            for i in 0..rng.range(20, 80) {
                let b = put_batch(&format!("key-{i:04}"), &"v".repeat(rng.range(10, 900) as usize));
                db.commit(&b).unwrap();
                batches.push(b);
                acked += 1;
            }
            // Die in the middle of the write of a frame: a torn WAL tail.
            disk.crash_when(|op| op.phase == "commit" && op.kind == OpKind::WriteAt, 1, CrashKind::PowerLoss);
            let b = put_batch("in-flight", &"x".repeat(rng.range(50, 600) as usize));
            batches.push(b.clone());
            assert!(db.commit(&b).is_err());
            let attempted = acked + 1;
            let mut image = disk.take_crash_image().expect("crashed");
            // What recovery makes of that image, on a copy.
            let s1 = {
                let copy = SimVfs::restore(&image.dump());
                let db = open_in(&copy.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: first recovery: {e}"));
                db.snapshot().unwrap().seq()
            };
            let lower = if durability == Durability::Sync { acked } else { 0 };
            assert!(lower <= s1 && s1 <= attempted, "{ctx}: recovered {s1} outside {lower}..={attempted}");
            // Two crashes during the session start of the following opens: the
            // first at the beginning of S-4 (after S-2, S-3), the second at
            // its end.
            for round in 0..2 {
                let disk = image.clone();
                disk.set_tear_unit(if (seed + round) % 2 == 0 { 1 } else { 512 });
                let nth = 1 + round;
                disk.crash_when(
                    |op| {
                        op.phase == "session"
                            && op.path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("MANIFEST"))
                    },
                    nth,
                    CrashKind::PowerLoss,
                );
                let r = open_in(&disk.as_vfs(), &cfg);
                match r {
                    Ok(db) => drop(db),
                    Err(e) => assert!(disk.has_crashed(), "{ctx} round {round}: open failed without a crash: {e}"),
                }
                image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(CrashKind::PowerLoss));
            }
            let db = open_in(&image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: final open: {e}"));
            let s = db.snapshot().unwrap().seq();
            assert_eq!(s, s1, "{ctx}: the chained crashes changed the recovered state");
            let mut model = Model::new();
            for b in &batches[..s as usize] {
                apply(&mut model, b);
            }
            assert!(model_of(&db) == model, "{ctx}: the state is not the model at {s}");
        }
    }
}

/// S-2b: the frames recovery replays may be unsynced (`Buffered`, a process
/// that died with its page cache intact), and the log that S-3 creates claims
/// through its `base_seq` that they exist. A power loss before `SESSION_OPEN`
/// is durable must therefore find them: syncing the old log first keeps the
/// chain whole (without it the new log is a gap, REC-10c reports corruption or
/// commits vanish).
#[test]
fn recovery_makes_replayed_frames_durable_before_the_new_wal() {
    for seed in 0..40u64 {
        let ctx = format!("seed {seed}");
        let disk = SimVfs::new(seed);
        disk.set_tear_unit(if seed % 2 == 0 { 1 } else { 512 });
        let cfg = config(Durability::Buffered);
        let db = open_in(&disk.as_vfs(), &cfg).unwrap();
        let mut model = Model::new();
        let mut rng = Rng::new(seed);
        let n = rng.range(5, 25) as u32;
        for i in 0..n {
            put(&db, &mut model, &format!("key-{i:04}"), &vec![i as u8; rng.range(20, 600) as usize]);
        }
        // The process dies; nothing was synced after the log was created.
        let kill = disk.crash_now(CrashKind::ProcessKillKeepPending);
        drop(db);
        // Recovery replays the pending frames, and the power goes out as it
        // starts writing the MANIFEST.
        kill.crash_when(
            |op| {
                op.phase == "session"
                    && op.path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("MANIFEST"))
            },
            1,
            CrashKind::PowerLoss,
        );
        assert!(open_in(&kill.as_vfs(), &cfg).is_err(), "{ctx}: the power loss did not fire");
        let image = kill.take_crash_image().unwrap();
        let db = open_in(&image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: {e}"));
        assert_eq!(db.snapshot().unwrap().seq(), u64::from(n), "{ctx}: recovery lost commits it had replayed");
        assert!(model_of(&db) == model, "{ctx}");
    }
}

/// Damage or truncation anywhere in the MANIFEST of a database that was not
/// closed: the open either reports corruption or yields exactly the state
/// before the damage. Never a smaller database (K4).
#[test]
fn manifest_damage_after_unclean_close_never_rolls_back() {
    // A session with flushes and compactions, then a crash: the last records
    // delete things (a WAL, compaction inputs), so every rollback is caught.
    let disk = SimVfs::new(8);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    let mut rng = Rng::new(8);
    for i in 0..160 {
        put(&db, &mut model, &format!("key-{:05}", rng.below(300)), &[i as u8; 150]);
    }
    drop(db);
    let (_, corrupt_a) = sweep_manifest_damage(&disk.dump(), &model);
    // A session that wrote nothing: its SESSION_OPEN is the only record after
    // a CLEAN snapshot, and rolling it back lands on a state that holds
    // everything (the new, empty log is an orphan, REC-7).
    let (closed, closed_model) = closed_image();
    let disk = SimVfs::restore(&closed);
    drop(open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap());
    let (ok_b, corrupt_b) = sweep_manifest_damage(&disk.dump(), &closed_model);
    assert!(corrupt_a > 100, "the first image never reported corruption ({corrupt_a})");
    assert!(ok_b > 0, "no damage of the second image was recoverable ({ok_b} opens, {corrupt_b} corruptions)");
}

/// Damages (every byte, two masks) and truncates (every length) the MANIFEST
/// of `image`; every open must be `Corruption` or yield `model`. Returns how
/// many opens succeeded and how many reported corruption.
fn sweep_manifest_damage(image: &DirImage, model: &Model) -> (u64, u64) {
    let manifest = path("MANIFEST");
    let len = image.files[&manifest].len();
    let (_, records) = manifest_of(image);
    assert!(records.len() >= 2, "{} records", records.len());
    let (mut ok, mut corrupt) = (0u64, 0u64);
    let mut check = |damaged: &DirImage, what: String| {
        let disk = SimVfs::restore(damaged);
        match open_in(&disk.as_vfs(), &config(Durability::Sync)) {
            Ok(db) => {
                assert!(&model_of(&db) == model, "{what}: the open succeeded with a different state");
                ok += 1;
            }
            Err(Error::Corruption(_)) => corrupt += 1,
            Err(e) => panic!("{what}: {e}"),
        }
    };
    for off in 0..len {
        for mask in [0x01u8, 0x80] {
            let mut damaged = image.clone();
            damaged.files.get_mut(&manifest).unwrap()[off] ^= mask;
            check(&damaged, format!("flip {mask:#x} at {off} of {len}"));
        }
        let mut cut = image.clone();
        cut.files.get_mut(&manifest).unwrap().truncate(off);
        check(&cut, format!("truncation to {off} of {len}"));
    }
    // Record boundaries, one by one.
    for (at, _) in &records {
        let mut cut = image.clone();
        cut.files.get_mut(&manifest).unwrap().truncate(*at as usize);
        check(&cut, format!("cut at the record boundary {at}"));
    }
    (ok, corrupt)
}

/// The MANIFEST of an engine that recovered in chunks replays under the full
/// validation (MAN-S4: overlapping L0 files in sequence order), and a MANIFEST
/// that breaks the order is refused.
#[test]
fn l0_seq_order_validated() {
    // Overlapping tables: keys in random order.
    let disk = SimVfs::new(10);
    let cfg = Config { write_buffer_bytes: 1 << 20, ..config(Durability::Sync) };
    let db = open_in(&disk.as_vfs(), &cfg).unwrap();
    let mut model = Model::new();
    let mut rng = Rng::new(10);
    for i in 0..120 {
        put(&db, &mut model, &format!("key-{:05}", rng.below(2000)), &[i as u8; 400]);
    }
    drop(db);
    // Recovery in chunks of 16 KiB puts several overlapping tables at the head
    // of L0; the record that lists them replays under MAN-S4.
    let small = Config { write_buffer_bytes: 16 << 10, ..config(Durability::Sync) };
    let (disk2, db) = open_copy(&disk.dump(), &small);
    let db = db.unwrap();
    assert!(stat(&db, "flushes") >= 3, "{}", stat(&db, "flushes"));
    drop(db);
    let (replay, records) = manifest_of(&disk2.dump());
    let Edit::SessionOpen { files, .. } = &records.last().unwrap().1.edit else { panic!("{:?}", records.last()) };
    assert!(files.len() >= 3);
    assert!(files.windows(2).all(|w| w[0].smallest_seq > w[1].largest_seq), "newest first");
    // A snapshot that lists the same tables oldest first breaks MAN-S4.
    let mut bad = disk2.dump();
    let mut l0 = files.clone();
    l0.reverse();
    let state = &replay.state;
    let g = replay.header.file_number;
    let mut all = l0;
    all.extend(state.levels[1..].iter().flatten().cloned());
    let snapshot = Record {
        edit_seq: replay.next_edit_seq,
        edit: Edit::Snapshot {
            next_file: state.next_file,
            last_seq: state.last_seq,
            log_number: state.log_number,
            clean: false,
            prev_generation: g,
            files: all,
        },
        ignored: Vec::new(),
    };
    let header = Header::manifest(DB_ID, g + 1, replay.next_edit_seq);
    bad.files.insert(path("MANIFEST"), encode_manifest(&header, &[snapshot]));
    let e = reject(&bad);
    assert_corruption(&e, "MAN-S4");
}

/// S-2 writes an L0 table each time the replayed memtable reaches the write
/// buffer, while the log is still being read: recovery memory is bounded by
/// the buffer, not by the log.
#[test]
fn recovery_flush_is_chunked_by_write_buffer() {
    let disk = SimVfs::new(11);
    // An 8 MiB buffer keeps everything in one WAL of over 4 MiB: more than
    // the 1 MiB the replay reads at a time.
    let big = Config { write_buffer_bytes: 8 << 20, ..config(Durability::Buffered) };
    let db = open_in(&disk.as_vfs(), &big).unwrap();
    let mut model = Model::new();
    for i in 0..2200u32 {
        put(&db, &mut model, &format!("key-{i:05}"), &[i as u8; 2000]);
    }
    drop(db);
    let wal_len: u64 =
        disk.files().iter().filter(|(p, _)| p.extension().is_some_and(|e| e == "wal")).map(|f| f.1).sum();
    assert!(wal_len > 4_000_000, "{wal_len}");
    // Recover it with a 16 KiB buffer, watching the order of reads and creates.
    let events = Arc::new(Mutex::new(Vec::new()));
    let log = events.clone();
    let vfs: Arc<dyn Vfs> = HookVfs::wrap(disk.as_vfs(), move |op| {
        if op.point == HookPoint::Before {
            let ext = op.path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_string();
            match (op.kind, op.mode) {
                (HookOpKind::ReadAt, _) if ext == "wal" => log.lock().push("read-wal"),
                (HookOpKind::Open, Some(OpenMode::CreateNew)) if ext == "sst" => log.lock().push("create-sst"),
                _ => {}
            }
        }
        HookAction::Proceed
    });
    let small = Config { write_buffer_bytes: 16 << 10, ..config(Durability::Sync) };
    let db = open_in(&vfs, &small).unwrap();
    assert!(model_of(&db) == model);
    let events = events.lock().clone();
    let last_read = events.iter().rposition(|e| *e == "read-wal").unwrap();
    let first_create = events.iter().position(|e| *e == "create-sst").unwrap();
    assert!(first_create < last_read, "the first table was written only after the whole log was read");
    let flushes = stat(&db, "flushes");
    let expected = wal_len / (16 << 10);
    assert!(flushes >= expected / 2 && flushes >= 10, "{flushes} tables for {wal_len} bytes of log");
    // No table is much larger than the buffer.
    for t in db.level_tables().iter().flatten() {
        assert!(t.size < 3 * (16 << 10), "table {} holds {} bytes", t.number, t.size);
    }
}

/// Tags below 1024 that a reader does not know are corruption; tags from
/// 1024 up are ignored and a checkpoint does not keep them (§8.3).
#[test]
fn unknown_manifest_tag_rules() {
    let (image, model) = unclean_image();
    let (replay, records) = manifest_of(&image);
    let st = &replay.state;
    let noop = |ignored| Record {
        edit_seq: replay.next_edit_seq,
        edit: Edit::Flush {
            next_file: st.next_file,
            last_seq: st.last_seq,
            log_number: st.log_number,
            reason: None,
            files: Vec::new(),
        },
        ignored,
    };
    let generation = replay.header.file_number;
    // A tag >= 1024 in an appended record: ignored.
    let mut with_ignored = image.clone();
    with_ignored.files.get_mut(&path("MANIFEST")).unwrap().extend(noop(vec![(2000, vec![1, 2, 3])]).encode(generation));
    let (disk, db) = opens_with(&with_ignored, &model);
    drop(db);
    let (replay2, _) = manifest_of(&disk.dump());
    // The open went on from the same state: what it recovered from the log
    // went to tables, and the ignored field changed nothing.
    assert_eq!(replay2.state.last_seq, 408, "every commit of the image");
    assert!(replay.state.last_seq < 408);
    assert_eq!(replay2.state.levels[1..], replay.state.levels[1..]);
    // A tag below 1024 that the format does not define (here 500, a field
    // of no record type): corruption, whatever else the record says.
    let mut payload = noop(Vec::new()).encode_payload();
    payload.push(0xF4); // tag 500 as a varint
    payload.push(0x03);
    payload.push(0x01); // length 1
    payload.push(0x00);
    let record = seal_record(generation, replay.next_edit_seq, 3, &payload);
    let mut unknown = image.clone();
    unknown.files.get_mut(&path("MANIFEST")).unwrap().extend(record);
    // The record is intact (its CRC holds), so it is not a torn tail.
    assert_corruption(&reject(&unknown), "MAN-R2");
    let _ = records;
}
