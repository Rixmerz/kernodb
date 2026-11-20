//! Media damage on a database that is in use, and on one that just survived a
//! crash (R-67): the engine either reports the damage or returns exactly what
//! was committed. It never returns data that was not.
//!
//! The bytes are flipped in the simulated disk after the engine wrote them
//! (`corrupt_byte_live`, which also reaches the pending writes), so the flip
//! survives any crash that follows.

mod common;

use std::path::PathBuf;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::KernoDb;
use kerno_storage::format::wal::{WalScanParams, scan_wal};
use kerno_testkit::harness::{Model, apply, read_all, seeds};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, DirImage, SimVfs};

use common::*;

/// Reads everything `snap` can show: the whole scan, then every key of
/// `model` and one that is not in it. `Ok(())` means all of it matched
/// `model`; an error is what the engine reported.
fn read_everything(snap: &impl Snapshot, model: &Model, what: &str) -> Result<(), Error> {
    let seen = read_all(snap)?;
    assert!(
        &seen == model,
        "{what}: the scan returned data that was not committed: {}",
        kerno_testkit::harness::diff(model, &seen)
    );
    for (k, v) in model {
        let got = snap.get(k)?;
        assert_eq!(got.as_ref(), Some(v), "{what}: get({k:?}) returned wrong data");
    }
    assert_eq!(snap.get(b"no-such-key")?, None, "{what}: a key that was never written");
    Ok(())
}

fn assert_detected(what: &str, e: &Error) {
    assert!(matches!(e, Error::Corruption(_)), "{what}: damage must be reported as Corruption, got {e:?}");
}

/// Opens `image` and reads everything: `true` if all of it matched `model`,
/// `false` if the damage was reported, in the open or in a read.
fn open_and_read(image: &DirImage, model: &Model, what: &str) -> bool {
    let disk = SimVfs::restore(image);
    let db = match open_in(&disk.as_vfs(), &config(Durability::Sync)) {
        Ok(db) => db,
        Err(e) => {
            assert_detected(what, &e);
            return false;
        }
    };
    match read_everything(&db.snapshot().unwrap(), model, what) {
        Ok(()) => true,
        Err(e) => {
            assert_detected(what, &e);
            false
        }
    }
}

/// A byte of a table flipped while the database is open and has read nothing
/// of it yet: the read that reaches the byte fails its checksum, a byte the
/// engine already holds in memory (the index, the filter) is harmless, and
/// nothing in between returns wrong data. A reopen afterwards sees the same.
#[test]
fn damage_during_use_is_detected_or_harmless() {
    let (image, model) = closed_image();
    let tables = files_with(&image, "sst");
    assert!(tables.len() >= 3, "the image needs several tables: {}", tables.len());
    let (mut detected, mut harmless) = (0u64, 0u64);
    for seed in seeds(150) {
        let mut rng = Rng::new(seed);
        let file = &tables[rng.below(tables.len() as u64) as usize];
        let offset = rng.below(image.files[file].len() as u64);
        let mask = 1u8 << rng.below(8);
        let what = format!("seed {seed}: bit {mask:#04x} at {offset} of {}", file.display());

        let disk = SimVfs::restore(&image);
        let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
        disk.corrupt_byte_live(file, offset, mask);
        let snap = db.snapshot().unwrap();
        match read_everything(&snap, &model, &what) {
            Ok(()) => harmless += 1,
            Err(e) => {
                assert_detected(&what, &e);
                detected += 1;
            }
        }
        // The same bytes under a new handle.
        drop(snap);
        drop(db);
        open_and_read(&disk.dump(), &model, &what);
    }
    println!("damage during use: {detected} reported, {harmless} harmless");
    assert!(detected > 0, "no flip was ever reported: the test does not reach the data blocks");
    assert!(harmless > 0, "every flip was reported: the test never flipped a byte the engine holds in memory");
}

// ---- after a crash --------------------------------------------------------------

/// `n` single-key commits over 300 keys, `seed` deciding which; `models[s]` is
/// the state at sequence `s`. Stops at the first error (the crash). The
/// commit that failed may still have reached the disk, so its state is the
/// last entry of `models`.
fn drive(db: &KernoDb, seed: u64, n: u32) -> (u64, Vec<Model>) {
    let mut rng = Rng::new(seed);
    let mut models = vec![Model::new()];
    let mut acked = 0;
    for i in 0..n {
        let mut b = WriteBatch::new();
        let key = format!("key-{:05}", rng.below(300));
        if rng.chance(1, 8) {
            b.delete(key);
        } else {
            b.put(key, vec![i as u8; 150]);
        }
        let mut next = models.last().unwrap().clone();
        apply(&mut next, &b);
        match db.commit(&b) {
            Ok(seq) => {
                assert_eq!(seq, models.len() as u64, "sequences are consecutive for one writer");
                models.push(next);
                acked = seq;
            }
            Err(_) => {
                models.push(next);
                break;
            }
        }
    }
    (acked, models)
}

/// The byte range of the newest frame of the newest WAL that has frames, and
/// that WAL's file: damage there is indistinguishable from a torn tail (the
/// log is accepted up to the frame before it), so it may cost the last
/// acknowledged commit. With fewer than two frames the same holds for the
/// whole file, header included (a broken header is only proven damage by a
/// later frame that claims the log was durable: WAL-R6).
fn newest_wal_tail(image: &DirImage) -> Option<(PathBuf, u64)> {
    for wal in files_with(image, "wal").into_iter().rev() {
        let bytes = &image.files[&wal];
        if bytes.len() < 96 {
            continue; // a log whose creation the crash cut short
        }
        let params = WalScanParams { file_number: file_number(&wal), db_id: DB_ID, expected_seq: le_u64(bytes, 64) };
        let (mut frames, mut last) = (0u64, 0u64);
        let scan = scan_wal(&mut &bytes[..], &params, |at, _| {
            frames += 1;
            last = at;
            Ok(())
        });
        if scan.is_ok() && frames > 0 {
            return Some((wal, if frames >= 2 { last } else { 0 }));
        }
    }
    None
}

/// Crash at a random point of a session that flushes and compacts, flip one
/// bit of one file, recover: the open reports the damage, or the database
/// holds exactly one of the states the session went through (never a mixture,
/// never a value that was not written), and not one older than the last
/// acknowledged commit unless the bit is in the last frame of the newest log.
#[test]
fn damage_after_unclean_crash_never_returns_wrong_data() {
    const COMMITS: u32 = 220;
    // How many mutating operations the whole session has, to pick crash points.
    let total_ops = {
        let disk = SimVfs::new(0);
        let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
        drive(&db, 0, COMMITS);
        drop(db);
        disk.mutating_ops()
    };
    let (mut reported, mut recovered, mut lost_tail) = (0u64, 0u64, 0u64);
    for seed in seeds(150) {
        let mut rng = Rng::new(seed);
        let kind = if rng.chance(1, 2) { CrashKind::PowerLoss } else { CrashKind::ProcessKill };
        let disk = SimVfs::new(seed);
        disk.crash_after(rng.range(3, total_ops), kind);
        let (acked, models) = match open_in(&disk.as_vfs(), &config(Durability::Sync)) {
            Ok(db) => drive(&db, seed, COMMITS),
            Err(_) => continue, // crashed while creating the database
        };
        let Some(crashed) = disk.take_crash_image() else { continue };
        let image = crashed.dump();

        let tail = newest_wal_tail(&image);
        // Half of the flips go to the log that was being written: it is where
        // an acknowledged commit can be lost, and a seed in ten would reach it
        // by chance.
        let victims: Vec<&PathBuf> =
            image.files.keys().filter(|p| p.file_name().is_some_and(|n| n != "LOCK")).collect();
        let victim = match &tail {
            Some((wal, _)) if rng.chance(1, 2) => wal.clone(),
            _ => victims[rng.below(victims.len() as u64) as usize].clone(),
        };
        let offset = rng.below(image.files[&victim].len().max(1) as u64);
        let mask = 1u8 << rng.below(8);
        let what = format!(
            "seed {seed}: {kind:?} after {acked} acknowledged commits, bit {mask:#04x} at {offset} of {}",
            victim.display()
        );
        let in_last_frame = tail.as_ref().is_some_and(|(wal, from)| *wal == victim && offset >= *from);
        if offset >= image.files[&victim].len() as u64 {
            continue; // an empty file has no byte to flip
        }
        let mut damaged = image.clone();
        damaged.files.get_mut(&victim).unwrap()[offset as usize] ^= mask;

        let disk = SimVfs::restore(&damaged);
        let db = match open_in(&disk.as_vfs(), &config(Durability::Sync)) {
            Ok(db) => db,
            Err(e) => {
                assert_detected(&what, &e);
                reported += 1;
                continue;
            }
        };
        let snap = db.snapshot().unwrap();
        let seq = snap.seq();
        assert!((seq as usize) < models.len(), "{what}: sequence {seq} was never assigned (last {})", models.len() - 1);
        match read_everything(&snap, &models[seq as usize], &what) {
            Ok(()) => {
                recovered += 1;
                if seq < acked {
                    assert!(in_last_frame, "{what}: recovered sequence {seq} but {acked} were acknowledged");
                    lost_tail += 1;
                }
            }
            Err(e) => {
                assert_detected(&what, &e);
                reported += 1;
            }
        }
    }
    println!(
        "damage after a crash: {reported} reported, {recovered} recovered ({lost_tail} lost the damaged last frame)"
    );
    assert!(
        reported > 0 && recovered > 0,
        "{reported} reported, {recovered} recovered: the sweep did not reach both outcomes"
    );
}
