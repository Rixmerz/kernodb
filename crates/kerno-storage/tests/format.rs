//! The engine writes and reads only format v1 (`docs/file-format/v1.md`):
//! headers and feature bits, versions, the M0 and foreign directories, the
//! frames of the WAL, the validation of every file at open, the limits and
//! the platform gate.
//!
//! Every test that expects a rejection checks that the rejected directory is
//! byte for byte what it was (the `LOCK` file aside).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, Op, Result, WriteBatch};
use kerno_storage::format::block::reseal_block;
use kerno_storage::format::features::{AEAD, DRAFT};
use kerno_storage::format::header::{
    FORMAT_VERSION, FileKind, HEADER_LEN, Header, HeaderExpect, decode_header, reseal_header,
};
use kerno_storage::format::limits::MAX_SEQ;
use kerno_storage::format::manifest::{Edit, Record, encode_manifest, replay_manifest, reseal_record};
use kerno_storage::format::sst::{FOOTER_LEN, SstExpect, decode_footer, open_sst};
use kerno_storage::format::wal::{CanonicalCommit, Frame, WalScan, WalScanParams, encode_frame, scan_wal};
use kerno_storage::{Background, Config, KernoDb, Limits, TestConfig};
use kerno_testkit::harness::{Model, apply, read_all};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::assert_dir_unchanged;
use kerno_testkit::vfs::{CountingVfs, CrashKind, DirImage, HookAction, HookOpKind, HookPoint, HookVfs, SimVfs};
use parking_lot::Mutex;

const DIR: &str = "/db";
const DB_ID: [u8; 16] = [0xd1; 16];

fn config(durability: Durability) -> Config {
    Config {
        durability,
        write_buffer_bytes: 16 << 10,
        test: TestConfig { background: Background::Inline, db_id: Some(DB_ID) },
        ..Config::default()
    }
}

fn open_in(vfs: &Arc<dyn Vfs>, cfg: &Config) -> Result<KernoDb> {
    KernoDb::open(vfs.clone(), Path::new(DIR), cfg)
}

fn path(name: &str) -> PathBuf {
    Path::new(DIR).join(name)
}

fn put(db: &KernoDb, model: &mut Model, key: &str, value: &[u8]) -> u64 {
    let mut b = WriteBatch::new();
    b.put(key, value);
    let seq = db.commit(&b).unwrap();
    apply(model, &b);
    seq
}

/// `n` commits of one 200-byte value each, from key number `from`.
fn fill(db: &KernoDb, model: &mut Model, from: u32, n: u32) {
    for i in from..from + n {
        put(db, model, &format!("key-{i:05}"), &[i as u8; 200]);
    }
}

fn model_of(db: &KernoDb) -> Model {
    read_all(&db.snapshot().unwrap()).unwrap()
}

/// A database with flushed tables, closed cleanly.
fn closed_image() -> (DirImage, Model) {
    let disk = SimVfs::new(1);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 400);
    db.close().unwrap();
    (disk.dump(), model)
}

/// A database with flushed tables and a live WAL of at least eight
/// commits, left without a clean close.
fn unclean_image() -> (DirImage, Model) {
    let disk = SimVfs::new(2);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 400);
    fill(&db, &mut model, 400, 8);
    drop(db);
    let image = disk.dump();
    assert!(!files_with(&image, "wal").is_empty());
    (image, model)
}

/// The files of `image` with this extension (or the MANIFEST), by name.
fn files_with(image: &DirImage, ext: &str) -> Vec<PathBuf> {
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

fn first_file(image: &DirImage, ext: &str) -> PathBuf {
    files_with(image, ext).into_iter().next().unwrap_or_else(|| panic!("no {ext} file"))
}

fn file_number(p: &Path) -> u64 {
    p.file_stem().unwrap().to_str().unwrap().parse().unwrap()
}

fn le_u64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Patches the 96-byte header of `file` and recomputes its CRC.
fn patch_header(image: &mut DirImage, file: &Path, patch: impl FnOnce(&mut [u8])) {
    let bytes = image.files.get_mut(file).unwrap_or_else(|| panic!("no file {}", file.display()));
    patch(&mut bytes[..HEADER_LEN]);
    assert!(reseal_header(&mut bytes[..HEADER_LEN]));
}

fn set_u32(h: &mut [u8], at: usize, v: u32) {
    h[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn or_u64(h: &mut [u8], at: usize, bits: u64) {
    let v = le_u64(h, at) | bits;
    h[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// Opens a copy of `image` and returns the error, after checking that the
/// directory is unchanged. `cfg` is the default configuration.
fn reject_untouched(image: &DirImage) -> Error {
    reject_untouched_with(image, &config(Durability::Sync))
}

fn reject_untouched_with(image: &DirImage, cfg: &Config) -> Error {
    let disk = SimVfs::restore(image);
    let before = disk.dump();
    let err = open_in(&disk.as_vfs(), cfg).err().expect("the open must fail");
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    err
}

fn assert_incompatible(e: &Error, needle: &str) {
    assert!(matches!(e, Error::Incompatible(m) if m.contains(needle)), "expected Incompatible({needle}), got {e:?}");
}

fn assert_corruption(e: &Error, needle: &str) {
    assert!(matches!(e, Error::Corruption(m) if m.contains(needle)), "expected Corruption({needle}), got {e:?}");
}

fn opens_with(image: &DirImage, model: &Model) -> (SimVfs, KernoDb) {
    let disk = SimVfs::restore(image);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap_or_else(|e| panic!("open failed: {e}"));
    assert!(&model_of(&db) == model, "the opened database differs from the model");
    (disk, db)
}

/// Scans a WAL held in memory the way recovery does.
fn scan(bytes: &[u8], number: u64) -> (WalScan, Vec<(u64, Frame)>) {
    let mut frames = Vec::new();
    let params = WalScanParams { file_number: number, db_id: DB_ID, expected_seq: le_u64(bytes, 64) };
    let scan = scan_wal(&mut &bytes[..], &params, |off, f| {
        frames.push((off, f));
        Ok(())
    })
    .map_err(|e| e.into_format())
    .unwrap();
    (scan, frames)
}

// ---------------------------------------------------------------- headers

#[test]
fn header_round_trip_all_kinds() {
    let (image, _) = unclean_image();
    for (ext, kind) in [("MANIFEST", FileKind::Manifest), ("wal", FileKind::Wal), ("sst", FileKind::Sst)] {
        let file = first_file(&image, ext);
        let bytes = &image.files[&file];
        let expect =
            HeaderExpect { db_id: Some(DB_ID), file_number: (kind != FileKind::Manifest).then(|| file_number(&file)) };
        let h = decode_header(&bytes[..HEADER_LEN], bytes.len() as u64, kind, &expect)
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert_eq!((h.kind, h.format_version, h.header_len), (kind, FORMAT_VERSION, HEADER_LEN as u32));
        assert_eq!(h.db_id, DB_ID);
        assert_eq!((h.compat, h.ro_compat), (0, 0));
        assert_eq!(h.encode()[..], bytes[..HEADER_LEN], "{}: re-encoding changes the bytes", file.display());
        match kind {
            FileKind::Manifest => assert!(h.file_number >= 2, "creation is generation 1, every session adds one"),
            FileKind::Wal => assert!(h.per_type >= 1, "base_seq"),
            FileKind::Sst => assert_eq!(h.per_type, 4096, "block size"),
        }
    }
    // Every kind round trips through the codec, whatever the engine wrote.
    for kind in [FileKind::Manifest, FileKind::Wal, FileKind::Sst] {
        let h = Header::new(kind, DB_ID, 7, if kind == FileKind::Sst { 4096 } else { 9 });
        let bytes = h.encode();
        let expect = HeaderExpect { db_id: Some(DB_ID), file_number: Some(7) };
        assert_eq!(decode_header(&bytes, bytes.len() as u64, kind, &expect).unwrap(), h);
    }
}

#[test]
fn draft_bit_written_on_every_file() {
    // Flushes, compactions and a live WAL: every kind of file and every
    // reason a table is written for.
    let disk = SimVfs::new(3);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 700);
    drop(db);
    let image = disk.dump();
    let mut seen = [0usize; 3];
    for (file, bytes) in &image.files {
        let kind = match file.extension().and_then(|e| e.to_str()) {
            Some("wal") => FileKind::Wal,
            Some("sst") => FileKind::Sst,
            _ if file.file_name().is_some_and(|n| n == "MANIFEST") => FileKind::Manifest,
            _ => continue, // LOCK holds no state
        };
        let h = decode_header(&bytes[..HEADER_LEN], bytes.len() as u64, kind, &HeaderExpect::default()).unwrap();
        assert_eq!(h.incompat, DRAFT, "{}", file.display());
        assert_eq!(h.db_id, DB_ID, "{}", file.display());
        seen[kind as usize] += 1;
    }
    assert!(seen.iter().all(|&n| n > 0), "files seen by kind: {seen:?}");
    // The same after a clean close, which rewrites the MANIFEST.
    let (closed, _) = closed_image();
    let manifest = &closed.files[&path("MANIFEST")];
    let h =
        decode_header(&manifest[..HEADER_LEN], manifest.len() as u64, FileKind::Manifest, &Default::default()).unwrap();
    assert_eq!(h.incompat, DRAFT);
}

#[test]
fn sst_creation_reason_records_why_the_table_was_written() {
    // Flushes and compactions while the database runs; a recovery flush
    // after a crash.
    let disk = SimVfs::new(4);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    fill(&db, &mut model, 0, 700);
    drop(db);
    let crashed = disk.dump();
    let before: Vec<PathBuf> = files_with(&crashed, "sst");
    let recovered = SimVfs::restore(&crashed);
    drop(open_in(&recovered.as_vfs(), &config(Durability::Sync)).unwrap());
    let after = recovered.dump();
    let reason = |image: &DirImage, file: &Path| {
        let bytes = &image.files[file];
        let number = file_number(file);
        let ex = SstExpect { file_number: number, db_id: DB_ID, meta: None, last_seq: MAX_SEQ };
        open_sst(bytes, &ex).unwrap().props.creation_reason
    };
    let mut reasons: Vec<u8> = before.iter().map(|f| reason(&crashed, f)).collect();
    reasons.sort_unstable();
    reasons.dedup();
    assert_eq!(reasons, [1, 2], "flush and compaction while running");
    let new: Vec<PathBuf> = files_with(&after, "sst").into_iter().filter(|f| !before.contains(f)).collect();
    assert_eq!(new.len(), 1, "recovery flushes the live WAL into one table");
    assert_eq!(reason(&after, &new[0]), 3, "recovery");
}

// ------------------------------------------------- directories we refuse

#[test]
fn m0_prototype_directory_is_incompatible_untouched() {
    // What spike-lsm leaves: a MANIFEST with the M0 magic, logs without a
    // header and tables whose magic sits in the footer.
    let mut image = DirImage::default();
    image.dirs.insert(DIR.into());
    let mut manifest = b"KRNOMAN1".to_vec();
    manifest.extend((0..120u8).map(|b| b.wrapping_mul(7)));
    image.files.insert(path("MANIFEST"), manifest);
    image.files.insert(path("000003.wal"), (0..500u32).map(|b| (b % 251) as u8).collect());
    let mut sst = vec![0x11; 300];
    sst.extend_from_slice(b"KRNOSST1");
    image.files.insert(path("000002.sst"), sst);
    for create in [true, false] {
        let cfg = Config { create_if_missing: create, ..config(Durability::Sync) };
        assert_incompatible(&reject_untouched_with(&image, &cfg), "M0 prototype format");
    }
    // A v1 database whose table is an M0 one.
    let (mut closed, _) = closed_image();
    let sst = first_file(&closed, "sst");
    closed.files.get_mut(&sst).unwrap()[..8].copy_from_slice(b"KRNOSST1");
    assert_incompatible(&reject_untouched(&closed), "M0 prototype format");
}

#[test]
fn foreign_directories_refused_untouched() {
    // The B+ tree prototype's file, the reference engine's log, and plain
    // other files: none of them is a database, so nothing is created, not
    // even the lock.
    for names in [&["data.kbt"][..], &["log"], &["notes.txt", "photo.jpg"], &["1.wal"], &["000000.sst"]] {
        let mut image = DirImage::default();
        image.dirs.insert(DIR.into());
        for n in names {
            image.files.insert(path(n), b"not ours".to_vec());
        }
        for create in [true, false] {
            let disk = SimVfs::restore(&image);
            let before = disk.dump();
            let cfg = Config { create_if_missing: create, ..config(Durability::Sync) };
            let e = open_in(&disk.as_vfs(), &cfg).err().expect("must fail");
            assert!(matches!(&e, Error::InvalidArgument(m) if m == "not a Kerno database"), "{names:?}: {e:?}");
            assert_dir_unchanged(&before, &disk.dump(), &[]);
        }
    }
    // A directory with only a lock and a stale MANIFEST.tmp is a crash
    // during creation: it is created again.
    let mut image = DirImage::default();
    image.dirs.insert(DIR.into());
    image.files.insert(path("LOCK"), Vec::new());
    image.files.insert(path("MANIFEST.tmp"), b"half a manifest".to_vec());
    let disk = SimVfs::restore(&image);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    assert_eq!(db.snapshot().unwrap().seq(), 0);
    drop(db);
    assert!(!disk.as_vfs().exists(&path("MANIFEST.tmp")).unwrap());
}

#[test]
fn data_files_without_a_manifest_are_corruption_not_a_foreign_directory() {
    let (mut image, _) = closed_image();
    image.files.remove(&path("MANIFEST"));
    for create in [true, false] {
        let cfg = Config { create_if_missing: create, ..config(Durability::Sync) };
        assert_corruption(&reject_untouched_with(&image, &cfg), "the MANIFEST is missing");
    }
}

// --------------------------------------------- versions and feature bits

/// Patches the header of the first file with this extension and expects an
/// `Incompatible` open that changes nothing.
fn incompatible_after(ext: &str, patch: impl FnOnce(&mut [u8]), needle: &str) {
    let (mut image, _) = unclean_image();
    let file = first_file(&image, ext);
    patch_header(&mut image, &file, patch);
    assert_incompatible(&reject_untouched(&image), needle);
}

fn future_version(h: &mut [u8]) {
    set_u32(h, 8, FORMAT_VERSION + 1);
}

fn unknown_incompat(h: &mut [u8]) {
    or_u64(h, 32, 1 << 17);
}

fn aead(h: &mut [u8]) {
    or_u64(h, 32, AEAD);
}

#[test]
fn future_version_rejected_untouched_wal() {
    incompatible_after("wal", future_version, "HDR-4");
}

#[test]
fn future_version_rejected_untouched_sst() {
    incompatible_after("sst", future_version, "HDR-4");
}

#[test]
fn future_version_rejected_untouched_manifest() {
    incompatible_after("MANIFEST", future_version, "HDR-4");
}

#[test]
fn unknown_incompat_rejected_untouched_wal() {
    incompatible_after("wal", unknown_incompat, "HDR-6");
}

#[test]
fn unknown_incompat_rejected_untouched_sst() {
    incompatible_after("sst", unknown_incompat, "HDR-6");
}

#[test]
fn unknown_incompat_rejected_untouched_manifest() {
    incompatible_after("MANIFEST", unknown_incompat, "HDR-6");
}

#[test]
fn aead_bit_rejected_untouched() {
    for ext in ["wal", "sst", "MANIFEST"] {
        incompatible_after(ext, aead, "encryption not supported");
    }
}

#[test]
fn a_damaged_header_is_corruption_even_if_it_claims_a_future_version() {
    // HDR-3 comes before HDR-4: random damage is never presented as an
    // unknown version.
    for ext in ["MANIFEST", "sst", "wal"] {
        let (mut image, _) = unclean_image();
        let file = first_file(&image, ext);
        set_u32(&mut image.files.get_mut(&file).unwrap()[..HEADER_LEN], 8, FORMAT_VERSION + 1); // no new CRC
        assert_corruption(&reject_untouched(&image), if ext == "wal" { "WAL-R6" } else { "HDR-3" });
    }
}

#[test]
fn nonzero_aead_slot_is_corruption() {
    // A table whose filter block has a non-zero slot behind a valid CRC.
    let (mut image, _) = closed_image();
    let sst = first_file(&image, "sst");
    let number = file_number(&sst);
    let bytes = image.files.get_mut(&sst).unwrap();
    let len = bytes.len();
    let footer = decode_footer(&bytes[len - FOOTER_LEN..], number, (len - FOOTER_LEN) as u64).unwrap();
    let (off, blen) = (footer.filter.offset as usize, footer.filter.len as usize);
    bytes[off + blen - 4 - 40 + 7] = 0x5a; // inside the slot, before the CRC
    reseal_block(&mut bytes[off..off + blen], number, off as u64);
    assert_corruption(&reject_untouched(&image), "AEAD-1");

    // A MANIFEST record with a non-zero slot behind a valid CRC.
    let (mut image, _) = closed_image();
    let bytes = image.files.get_mut(&path("MANIFEST")).unwrap();
    let generation = le_u64(bytes, 56);
    let payload_len = u32::from_le_bytes(bytes[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap()) as usize;
    let record_len = 24 + payload_len + 40;
    bytes[HEADER_LEN + 24 + payload_len + 3] = 0x5a;
    reseal_record(&mut bytes[HEADER_LEN..HEADER_LEN + record_len], generation);
    assert_corruption(&reject_untouched(&image), "AEAD-1");
}

#[test]
fn unknown_compat_bit_ignored() {
    let (mut image, model) = unclean_image();
    for ext in ["wal", "sst", "MANIFEST"] {
        for file in files_with(&image, ext) {
            patch_header(&mut image, &file, |h| or_u64(h, 16, 1 << 5));
        }
    }
    let (disk, db) = opens_with(&image, &model);
    let mut model = model;
    put(&db, &mut model, "after", b"x");
    db.close().unwrap();
    // The MANIFEST was rewritten and does not carry the bit (HDR-8).
    let image = disk.dump();
    let manifest = &image.files[&path("MANIFEST")];
    let h =
        decode_header(&manifest[..HEADER_LEN], manifest.len() as u64, FileKind::Manifest, &Default::default()).unwrap();
    assert_eq!(h.compat, 0);
    let (_, db) = opens_with(&image, &model);
    db.close().unwrap();
}

#[test]
fn unknown_ro_compat_is_refused_until_read_only_exists() {
    for ext in ["wal", "sst", "MANIFEST"] {
        let (mut image, _) = unclean_image();
        let file = first_file(&image, ext);
        patch_header(&mut image, &file, |h| or_u64(h, 24, 1 << 3));
        assert_incompatible(&reject_untouched(&image), "read-only open not yet supported");
    }
}

// -------------------------------------------------------------------- WAL

#[test]
fn wal_frame_synced_len_follows_sync_state() {
    for (durability, label) in [(Durability::Sync, "Sync"), (Durability::Buffered, "Buffered")] {
        let disk = SimVfs::new(5);
        let db = open_in(&disk.as_vfs(), &config(durability)).unwrap();
        let mut model = Model::new();
        fill(&db, &mut model, 0, 150); // more than one memtable: rotations happened
        drop(db);
        let image = disk.dump();
        let wal = first_file(&image, "wal");
        let (scan, frames) = scan(&image.files[&wal], file_number(&wal));
        assert!(frames.len() >= 5, "{label}: {} frames", frames.len());
        assert_eq!(scan.commits, frames.len() as u64, "one commit per frame");
        for (i, (offset, frame)) in frames.iter().enumerate() {
            assert_eq!(frame.commits.len(), 1, "{label}");
            match durability {
                // The first frame is written before anything of the file is
                // durable; every later one after its predecessor was synced.
                Durability::Sync if i == 0 => assert_eq!(frame.synced_len, 0, "{label}"),
                Durability::Sync => assert_eq!(frame.synced_len, *offset, "{label}: frame {i}"),
                // Nothing is synced since the file was created.
                Durability::Buffered => assert_eq!(frame.synced_len, 0, "{label}: frame {i}"),
            }
        }
    }
}

/// A WAL header broken by a crash during creation: nothing, a prefix of the
/// header, zeros, a flipped bit.
fn broken_headers(valid: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let mut flipped = valid[..HEADER_LEN].to_vec();
    flipped[20] ^= 0x10;
    vec![
        ("empty file", Vec::new()),
        ("half a header", valid[..50].to_vec()),
        ("zeros", vec![0; HEADER_LEN]),
        ("bit flip", flipped),
    ]
}

#[test]
fn torn_newest_wal_header_is_empty_log() {
    // After a clean close: a log created by the next session that crashed
    // before its header was durable.
    let (closed, model) = closed_image();
    let manifest = &closed.files[&path("MANIFEST")];
    let next = replay_manifest(&mut &manifest[..], |_, _| Ok(())).map_err(|e| e.into_format()).unwrap().state.next_file;
    let template = Header::wal(DB_ID, next, 1).encode().to_vec();
    for (label, bytes) in broken_headers(&template) {
        let mut image = closed.clone();
        image.files.insert(path(&format!("{next:06}.wal")), bytes);
        let (disk, db) = opens_with(&image, &model);
        db.close().unwrap();
        assert!(
            !files_with(&disk.dump(), "wal").iter().any(|f| file_number(f) == next),
            "{label}: the empty log is deleted by recovery"
        );
    }
    // After a crash: a live log with commits, then a newer one that never got
    // a durable header (REC-10c).
    let (unclean, model) = unclean_image();
    let live = first_file(&unclean, "wal");
    let newer = path(&format!("{:06}.wal", file_number(&live) + 1));
    for (label, bytes) in broken_headers(&Header::wal(DB_ID, file_number(&newer), 1).encode()) {
        let mut image = unclean.clone();
        image.files.insert(newer.clone(), bytes);
        let (_, db) = opens_with(&image, &model);
        assert_eq!(db.snapshot().unwrap().seq(), 408, "{label}: the commits of the older log survive");
    }
}

/// A newer log that continues `wal` after its `frames` valid frames, with one
/// commit of its own.
fn newer_log(image: &mut DirImage, wal: &Path, valid_frames: u64) -> PathBuf {
    let number = file_number(wal) + 1;
    let base = le_u64(&image.files[wal], 64) + valid_frames;
    let commit = CanonicalCommit { ranges: Vec::new(), points: vec![(b"zz".to_vec(), Some(b"v".to_vec()))] };
    let frame = Frame { first_seq: base, synced_len: 0, commits: vec![commit] };
    let mut bytes = Header::wal(DB_ID, number, base).encode().to_vec();
    bytes.extend(encode_frame(number, &frame, false));
    let file = path(&format!("{number:06}.wal"));
    image.files.insert(file.clone(), bytes);
    file
}

#[test]
fn a_torn_tail_is_accepted_only_in_the_newest_log_with_commits() {
    // Two live logs, the first complete: both replay (REC-10a/b).
    let (mut image, mut model) = unclean_image();
    let wal = first_file(&image, "wal");
    let frames = scan(&image.files[&wal], file_number(&wal)).1.len() as u64;
    newer_log(&mut image, &wal, frames);
    model.insert(b"zz".to_vec(), b"v".to_vec());
    let (_, db) = opens_with(&image, &model);
    assert_eq!(db.snapshot().unwrap().seq(), 400 + 8 + 1);
    drop(db);

    // The first log torn inside its last frame, followed by a log with
    // commits: the tear was not the end of the log.
    let (mut image, _) = unclean_image();
    let torn = image.files.get_mut(&wal).unwrap();
    torn.truncate(torn.len() - 10);
    newer_log(&mut image, &wal, frames - 1);
    assert_corruption(&reject_untouched(&image), "REC-10a");

    // The same tear with nothing after it is a normal crash.
    let (mut image, model) = unclean_image();
    let log = image.files.get_mut(&wal).unwrap();
    log.truncate(log.len() - 10);
    let mut model = model;
    model.remove(b"key-00407".as_slice());
    let (_, db) = opens_with(&image, &model);
    assert_eq!(db.snapshot().unwrap().seq(), 400 + 8 - 1);
}

#[test]
fn invalid_wal_header_followed_by_synced_frame_is_corruption() {
    let (mut image, _) = unclean_image();
    let wal = first_file(&image, "wal");
    // A header with a bad CRC, and one with a magic that is no file's.
    let mut other = image.clone();
    image.files.get_mut(&wal).unwrap()[20] ^= 0x10;
    assert_corruption(&reject_untouched(&image), "WAL-R6");
    other.files.get_mut(&wal).unwrap()[..8].copy_from_slice(b"garbage!");
    assert_corruption(&reject_untouched(&other), "WAL-R6");
}

#[test]
fn wal_base_seq_gap_is_corruption() {
    let (mut image, _) = unclean_image();
    let wal = first_file(&image, "wal");
    patch_header(&mut image, &wal, |h| {
        let base = le_u64(h, 64);
        h[64..72].copy_from_slice(&(base + 1).to_le_bytes());
    });
    assert_corruption(&reject_untouched(&image), "WAL-R1");
    // A first frame that skips a sequence is not the expected one either.
    let (mut image, _) = unclean_image();
    let bytes = image.files.get_mut(&wal).unwrap();
    bytes[HEADER_LEN + 8..HEADER_LEN + 16].copy_from_slice(&u64::MAX.to_le_bytes());
    // (the CRC no longer verifies: the frame ends the log, which is a torn
    // tail of an empty log, while the frames behind it were synced)
    assert_corruption(&reject_untouched(&image), "WAL-R5");
}

#[test]
fn wal_file_number_mismatch_is_corruption() {
    let (mut image, _) = unclean_image();
    let wal = first_file(&image, "wal");
    let bytes = image.files.remove(&wal).unwrap();
    image.files.insert(path(&format!("{:06}.wal", file_number(&wal) + 5)), bytes);
    assert_corruption(&reject_untouched(&image), "HDR-10");
}

#[test]
fn foreign_db_id_sst_is_corruption() {
    let (mut image, _) = unclean_image();
    let sst = first_file(&image, "sst");
    patch_header(&mut image, &sst, |h| h[40] ^= 0xff);
    assert_corruption(&reject_untouched(&image), "HDR-10");
    // And a log of another database.
    let (mut image, _) = unclean_image();
    let wal = first_file(&image, "wal");
    patch_header(&mut image, &wal, |h| h[41] ^= 0x01);
    assert_corruption(&reject_untouched(&image), "HDR-10");
}

#[test]
fn sst_block_moved_within_file_is_corruption() {
    // Equal-sized entries make equal-sized blocks.
    let disk = SimVfs::new(6);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    for i in 0..300 {
        put(&db, &mut model, &format!("k{i:05}"), &[b'v'; 100]);
    }
    db.close().unwrap();
    let mut image = disk.dump();
    let (sst, number, handles) = files_with(&image, "sst")
        .into_iter()
        .find_map(|f| {
            let number = file_number(&f);
            let ex = SstExpect { file_number: number, db_id: DB_ID, meta: None, last_seq: MAX_SEQ };
            let handles: Vec<_> = open_sst(&image.files[&f], &ex).ok()?.index.iter().map(|e| e.handle).collect();
            (handles.len() >= 3 && handles[0].len == handles[1].len).then_some((f, number, handles))
        })
        .expect("a table with two equal blocks");
    let _ = number;
    let (a, b) = (handles[0], handles[1]);
    let bytes = image.files.get_mut(&sst).unwrap();
    let block_a = bytes[a.offset as usize..(a.offset + a.len) as usize].to_vec();
    bytes[b.offset as usize..(b.offset + b.len) as usize].copy_from_slice(&block_a);
    // The tables' meta blocks are intact: the open succeeds, the read of the
    // moved block does not (its CRC covers the offset it was written at).
    let disk = SimVfs::restore(&image);
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
    let e = read_all(&db.snapshot().unwrap()).expect_err("reading the moved block must fail");
    assert_corruption(&e, "SST-R1");
}

#[test]
fn damage_to_a_listed_table_is_found_at_open() {
    // SST-O1: the size recorded in the MANIFEST; HDR-10 above; and the
    // table must exist.
    let (mut image, _) = closed_image();
    let sst = first_file(&image, "sst");
    image.files.get_mut(&sst).unwrap().push(0);
    assert_corruption(&reject_untouched(&image), "SST-O1");
    let (mut image, _) = closed_image();
    image.files.remove(&sst);
    assert_corruption(&reject_untouched(&image), "missing");
}

// ------------------------------------------------------- limits and seqs

#[test]
fn batch_over_limit_rejected_before_write() {
    let limits = Limits { max_key_len: 8, max_value_len: 16, max_batch_bytes: 60, max_batch_ops: 25 };
    let disk = SimVfs::new(7);
    let cfg = Config { limits, ..config(Durability::Sync) };
    let db = open_in(&disk.as_vfs(), &cfg).unwrap();
    let stat = |db: &KernoDb, name: &str| db.metrics().flatten().into_iter().find(|(n, _)| n == name).unwrap().1;
    let mut over: Vec<(&str, WriteBatch)> = Vec::new();
    let mut b = WriteBatch::new();
    b.put([b'k'; 9], "v");
    over.push(("key", b));
    let mut b = WriteBatch::new();
    b.delete([b'k'; 9]);
    over.push(("deleted key", b));
    let mut b = WriteBatch::new();
    b.put("k", [b'v'; 17]);
    over.push(("value", b));
    let mut b = WriteBatch::new();
    for i in 0..26u8 {
        b.put([i], "");
    }
    over.push(("operations", b));
    let mut b = WriteBatch::new();
    b.put([b'a'; 8], [b'v'; 16]).put([b'b'; 8], [b'v'; 16]).put([b'c'; 8], [b'v'; 16]);
    over.push(("payload bytes", b));
    // Small keys and values, but 20 operations encode to more than 60 bytes.
    let mut b = WriteBatch::new();
    for i in 0..20u8 {
        b.put([i], [i]);
    }
    over.push(("encoded bytes", b));
    let ops = disk.mutating_ops();
    let commits = stat(&db, "commits");
    for (what, b) in &over {
        let r = db.commit(b);
        assert!(matches!(&r, Err(Error::InvalidArgument(_))), "{what}: {r:?}");
        assert_eq!(disk.mutating_ops(), ops, "{what}: a rejected batch wrote something");
        assert_eq!(stat(&db, "commits"), commits, "{what}");
    }
    // Nothing was numbered or poisoned, and the limits themselves are fine.
    assert_eq!(db.snapshot().unwrap().seq(), 0);
    let mut exact = WriteBatch::new();
    exact.put([b'k'; 8], [b'v'; 16]).delete([b'd'; 8]);
    assert_eq!(db.commit(&exact).unwrap(), 1);
    db.close().unwrap();
}

#[test]
fn limits_above_the_format_maxima_are_rejected_at_open() {
    let base = Limits::default();
    for bad in [
        Limits { max_key_len: 0, ..base },
        Limits { max_key_len: (16 << 10) + 1, ..base },
        Limits { max_value_len: (64 << 20) + 1, ..base },
        Limits { max_batch_ops: (1 << 24) + 1, ..base },
        Limits { max_batch_bytes: (1 << 30) + 1, ..base },
        Limits { max_batch_ops: 0, ..base },
    ] {
        let disk = SimVfs::new(8);
        let cfg = Config { limits: bad, ..config(Durability::Sync) };
        assert!(matches!(open_in(&disk.as_vfs(), &cfg), Err(Error::InvalidArgument(_))), "{bad:?}");
        assert_eq!(disk.mutating_ops(), 0, "{bad:?}");
    }
    // The format's own maxima are accepted.
    let top =
        Limits { max_key_len: 16 << 10, max_value_len: 64 << 20, max_batch_bytes: 1 << 30, max_batch_ops: 1 << 24 };
    let disk = SimVfs::new(8);
    open_in(&disk.as_vfs(), &Config { limits: top, ..config(Durability::Sync) }).unwrap().close().unwrap();
}

/// A closed database with no tables whose last sequence is `last_seq`.
fn image_at(last_seq: u64) -> DirImage {
    let record = Record {
        edit_seq: 1,
        edit: Edit::Snapshot {
            next_file: 1,
            last_seq,
            log_number: 1,
            clean: true,
            prev_generation: 0,
            files: Vec::new(),
        },
        ignored: Vec::new(),
    };
    let mut image = DirImage::default();
    image.dirs.insert(DIR.into());
    image.files.insert(path("MANIFEST"), encode_manifest(&Header::manifest(DB_ID, 1, 1), &[record]));
    image
}

#[test]
fn exhausted_seq_is_error_not_wrap() {
    for last_seq in [MAX_SEQ - 1, MAX_SEQ] {
        let disk = SimVfs::restore(&image_at(last_seq));
        let vfs = disk.as_vfs();
        let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
        let mut model = Model::new();
        let mut last = last_seq;
        if last_seq < MAX_SEQ {
            assert_eq!(put(&db, &mut model, "last", b"v"), MAX_SEQ, "the last sequence is usable");
            last = MAX_SEQ;
        }
        let ops = disk.mutating_ops();
        for _ in 0..3 {
            let mut b = WriteBatch::new();
            b.put("more", "x");
            let r = db.commit(&b);
            assert!(matches!(&r, Err(Error::InvalidArgument(m)) if m == "commit sequence exhausted"), "{r:?}");
        }
        assert_eq!(disk.mutating_ops(), ops, "an exhausted sequence writes nothing");
        // Not poisoned: reads, an empty batch and the close still work.
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), last);
        assert_eq!(snap.get(b"more").unwrap(), None);
        assert_eq!(db.commit(&WriteBatch::new()).unwrap(), last);
        drop(snap);
        // The sequence does not wrap after a restart either, with the last
        // commit replayed from the log or read from a table.
        drop(db);
        let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
        assert!(model_of(&db) == model);
        assert_eq!(db.snapshot().unwrap().seq(), last);
        let mut b = WriteBatch::new();
        b.put("more", "x");
        assert!(matches!(db.commit(&b), Err(Error::InvalidArgument(_))));
        db.close().unwrap();
        let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
        assert_eq!(db.snapshot().unwrap().seq(), last);
        db.close().unwrap();
    }
}

#[test]
fn huge_canonical_file_number_is_ignored() {
    // Names that look canonical but fall outside 1..=2^48-1 are unknown
    // files (DIR-1): never opened, never deleted, never counted.
    let (mut image, model) = closed_image();
    let strangers =
        ["281474976710656.sst", "281474976710656.wal", "999999999999999999.sst", "000000.wal", "000000.sst"];
    for name in strangers {
        image.files.insert(path(name), b"not an engine file".to_vec());
    }
    let (disk, db) = opens_with(&image, &model);
    let mut model = model;
    fill(&db, &mut model, 1000, 100);
    db.close().unwrap();
    let after = disk.dump();
    for name in strangers {
        assert_eq!(after.files.get(&path(name)).map(Vec::as_slice), Some(&b"not an engine file"[..]), "{name}");
    }
    let engine_numbers =
        files_with(&after, "sst").into_iter().chain(files_with(&after, "wal")).map(|f| file_number(&f));
    assert!(engine_numbers.filter(|n| *n < (1 << 40)).count() > 0);
    let (_, db) = opens_with(&after, &model);
    db.close().unwrap();
}

#[test]
fn platform_gate_matches_target() {
    let sim = SimVfs::new(9);
    let verified = cfg!(target_os = "linux");
    let strict = Config { allow_unverified_platform: false, ..config(Durability::Sync) };
    match open_in(&sim.as_vfs(), &strict) {
        Ok(db) => {
            assert!(verified, "an unverified platform opened with Sync");
            db.close().unwrap();
        }
        Err(e) => {
            assert!(!verified, "the verified platform refused Sync: {e}");
            assert!(matches!(&e, Error::InvalidArgument(m) if m == "durability not verified on this platform"));
            assert_eq!(sim.mutating_ops(), 0, "a refused open writes nothing");
        }
    }
    // Opting in always opens; Buffered never needs the opt-in.
    let sim = SimVfs::new(9);
    let allowed = Config { allow_unverified_platform: true, ..config(Durability::Sync) };
    open_in(&sim.as_vfs(), &allowed).unwrap().close().unwrap();
    let sim = SimVfs::new(9);
    open_in(&sim.as_vfs(), &config(Durability::Buffered)).unwrap().close().unwrap();
}

// ------------------------------------------------ protocol and accounting

#[test]
fn sync_commit_costs_one_fsync() {
    let (vfs, io) = CountingVfs::wrap(SimVfs::new(10).as_vfs());
    let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
    let mut model = Model::new();
    for i in 0..5u32 {
        let before = io.snapshot();
        put(&db, &mut model, &format!("k{i}"), b"value");
        let d = io.snapshot().since(&before);
        assert_eq!((d.writes, d.file_syncs, d.dir_syncs), (1, 1, 0), "commit {i}");
    }
    db.close().unwrap();
    let (vfs, io) = CountingVfs::wrap(SimVfs::new(10).as_vfs());
    let db = open_in(&vfs, &config(Durability::Buffered)).unwrap();
    let before = io.snapshot();
    put(&db, &mut model, "k", b"value");
    let d = io.snapshot().since(&before);
    assert_eq!((d.writes, d.file_syncs, d.dir_syncs), (1, 0, 0), "a Buffered commit is not synced");
}

#[test]
fn wal_replay_reads_in_bounded_chunks() {
    let disk = SimVfs::new(11);
    // One WAL of about 4 MiB: no memtable rotation, 40 frames of 100 KiB.
    let cfg = Config { write_buffer_bytes: 64 << 20, ..config(Durability::Buffered) };
    let db = open_in(&disk.as_vfs(), &cfg).unwrap();
    let mut model = Model::new();
    for i in 0..40u8 {
        put(&db, &mut model, &format!("big-{i:02}"), &vec![i; 100 << 10]);
    }
    drop(db);
    let wal_size = disk.dump().files.iter().find(|(p, _)| p.extension().is_some_and(|e| e == "wal")).unwrap().1.len();
    assert!(wal_size > 4_000_000, "{wal_size}");
    let reads = Arc::new(Mutex::new(Vec::new()));
    let log = reads.clone();
    let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
        if op.kind == HookOpKind::ReadAt
            && op.point == HookPoint::Before
            && op.path.extension().is_some_and(|e| e == "wal")
        {
            log.lock().push(op.len);
        }
        HookAction::Proceed
    });
    let db = open_in(&vfs, &cfg).unwrap();
    assert!(model_of(&db) == model);
    let reads = reads.lock().clone();
    let largest = reads.iter().copied().max().unwrap();
    assert!(largest <= 1 << 20, "a replay read of {largest} bytes");
    assert!(reads.len() >= 4, "the 4 MiB log is read in several chunks: {reads:?}");
    let total: u64 = reads.iter().sum();
    assert!(total < wal_size as u64 + (2 << 20), "{total} bytes read to replay {wal_size}");
}

#[test]
fn canonical_batch_semantics_preserved() {
    // Batches with repeated and overwritten keys: the log holds the
    // canonical form (points in key order, one operation per key), and the
    // state is what applying the batch operation by operation gives, live
    // and after a restart replays the log.
    let mut rng = Rng::new(12);
    let disk = SimVfs::new(12);
    // One memtable, so one log holds every commit.
    let cfg = Config { write_buffer_bytes: 1 << 20, ..config(Durability::Sync) };
    let db = open_in(&disk.as_vfs(), &cfg).unwrap();
    let mut model = Model::new();
    let mut batches = Vec::new();
    for _ in 0..60 {
        let mut b = WriteBatch::new();
        for _ in 0..1 + rng.below(12) {
            let key = format!("k{}", rng.below(6));
            if rng.chance(1, 3) {
                b.delete(key);
            } else {
                let len = rng.below(20) as usize;
                b.put(key, rng.bytes(len));
            }
        }
        db.commit(&b).unwrap();
        apply(&mut model, &b);
        batches.push(b);
    }
    assert!(model_of(&db) == model);
    drop(db);
    let image = disk.dump();
    let wal = first_file(&image, "wal");
    let (_, frames) = scan(&image.files[&wal], file_number(&wal));
    assert_eq!(frames.len(), 60);
    for ((_, frame), batch) in frames.iter().zip(&batches) {
        let commit = &frame.commits[0];
        assert!(commit.ranges.is_empty());
        assert!(commit.points.windows(2).all(|w| w[0].0 < w[1].0), "points are in strictly increasing key order");
        // Per key, the last operation of the batch.
        let mut last: std::collections::BTreeMap<&[u8], Option<&[u8]>> = Default::default();
        for op in batch.ops() {
            match op {
                Op::Put { key, value } => last.insert(key, Some(value)),
                Op::Delete { key } => last.insert(key, None),
                Op::DeleteRange { .. } => unreachable!(),
            };
        }
        let logged: Vec<(&[u8], Option<&[u8]>)> = commit.points.iter().map(|(k, v)| (&k[..], v.as_deref())).collect();
        assert_eq!(logged, last.into_iter().collect::<Vec<_>>());
    }
    let (_, db) = opens_with(&image, &model);
    assert_eq!(db.snapshot().unwrap().seq(), 60);
}

// ------------------------------------------------------ clean shutdown

fn manifest_replay(image: &DirImage) -> kerno_storage::format::manifest::ManifestReplay {
    let manifest = &image.files[&path("MANIFEST")];
    replay_manifest(&mut &manifest[..], |_, _| Ok(())).map_err(|e| e.into_format()).unwrap()
}

#[test]
fn close_writes_the_clean_snapshot_after_the_last_compaction() {
    // `CLEAN` means "no log is needed", with `log_number = next_file` (W-9):
    // it comes after every flush and every compaction of the close, not
    // before a compaction that would move `next_file` past it. Look for
    // sizes where the close itself has to compact.
    let mut compacted_in_close = 0;
    for n in (30..300).step_by(7) {
        let disk = SimVfs::new(20);
        let db = open_in(&disk.as_vfs(), &config(Durability::Sync)).unwrap();
        let mut model = Model::new();
        fill(&db, &mut model, 0, n);
        // Open and not closed: the MANIFEST says "not clean".
        assert!(!manifest_replay(&disk.dump()).clean);
        let before = db.metrics().background.compactions + db.metrics().background.trivial_moves;
        let after = db.close_with_metrics().unwrap().background;
        compacted_in_close += usize::from(after.compactions + after.trivial_moves > before);
        let image = disk.dump();
        let replay = manifest_replay(&image);
        assert!(replay.clean, "{n} commits: the MANIFEST after a clean close is not CLEAN");
        assert_eq!(replay.state.log_number, replay.state.next_file, "{n} commits");
        assert_eq!(replay.records, 1, "one snapshot per generation");
        let (reopened, db) = opens_with(&image, &model);
        // A session opened after a clean close is not clean until it ends.
        assert!(!manifest_replay(&reopened.dump()).clean);
        drop(db);
    }
    assert!(compacted_in_close > 0, "no size made the close compact: the test does not reach its point");
}

// ---------------------------------------------------- creation protocol

#[test]
fn creation_syncs_the_parent_only_when_the_database_is_created() {
    struct Recorder(Mutex<Vec<PathBuf>>);
    let log = Arc::new(Recorder(Mutex::new(Vec::new())));
    let sink = log.clone();
    let vfs = HookVfs::wrap(SimVfs::new(13).as_vfs(), move |op| {
        if op.kind == HookOpKind::SyncDir && op.point == HookPoint::Before {
            sink.0.lock().push(op.path.to_path_buf());
        }
        HookAction::Proceed
    });
    let dir = Path::new("/data/apps/db");
    let parents = |log: &Recorder| log.0.lock().iter().filter(|p| !p.starts_with(dir)).cloned().collect::<Vec<_>>();
    KernoDb::open(vfs.clone(), dir, &config(Durability::Sync)).unwrap().close().unwrap();
    assert!(parents(&log).contains(&PathBuf::from("/data/apps")), "creating syncs the parent");
    log.0.lock().clear();
    for _ in 0..2 {
        KernoDb::open(vfs.clone(), dir, &config(Durability::Sync)).unwrap().close().unwrap();
    }
    assert_eq!(parents(&log), Vec::<PathBuf>::new(), "opening an existing database never syncs its parent");
}

#[test]
fn lock_comes_before_any_mutation() {
    let disk = SimVfs::new(14);
    let vfs = disk.as_vfs();
    let first = open_in(&vfs, &config(Durability::Sync)).unwrap();
    let before = disk.dump();
    let ops = disk.mutating_ops();
    for _ in 0..3 {
        let second = open_in(&vfs, &config(Durability::Sync));
        assert!(matches!(second, Err(Error::Locked)));
    }
    assert_eq!(disk.mutating_ops(), ops, "a second open found the lock taken and did nothing else");
    assert_dir_unchanged(&before, &disk.dump(), &[]);
    drop(first);
}

#[test]
fn a_crash_at_any_point_of_creation_leaves_a_directory_that_opens() {
    let measure = SimVfs::new(15);
    drop(open_in(&measure.as_vfs(), &config(Durability::Sync)).unwrap());
    let ops = measure.mutating_ops();
    assert!(ops >= 8, "creating writes a MANIFEST, a session MANIFEST and a log: {ops} operations");
    for point in 1..=ops + 1 {
        for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill] {
            let disk = SimVfs::new(15);
            disk.crash_after(point, kind);
            let first = open_in(&disk.as_vfs(), &config(Durability::Sync));
            drop(first);
            let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(kind));
            let ctx = format!("crash at operation {point} ({kind:?})");
            // Whatever is left is either a directory that is created again
            // or a database: both open, empty.
            let vfs = image.as_vfs();
            let db = open_in(&vfs, &config(Durability::Sync)).unwrap_or_else(|e| panic!("{ctx}: {e}"));
            assert_eq!(db.snapshot().unwrap().seq(), 0, "{ctx}");
            let mut model = Model::new();
            assert_eq!(put(&db, &mut model, "k", b"v"), 1, "{ctx}");
            db.close().unwrap();
            let db = open_in(&vfs, &config(Durability::Sync)).unwrap();
            assert!(model_of(&db) == model, "{ctx}");
        }
    }
}

#[test]
fn database_ids_are_injectable_and_otherwise_random() {
    let id_of = |disk: &SimVfs| {
        let image = disk.dump();
        let manifest = &image.files[&path("MANIFEST")];
        decode_header(&manifest[..HEADER_LEN], manifest.len() as u64, FileKind::Manifest, &Default::default())
            .unwrap()
            .db_id
    };
    let fixed = SimVfs::new(16);
    open_in(&fixed.as_vfs(), &config(Durability::Sync)).unwrap().close().unwrap();
    assert_eq!(id_of(&fixed), DB_ID);
    let random =
        Config { test: TestConfig { background: Background::Inline, db_id: None }, ..config(Durability::Sync) };
    let (a, b) = (SimVfs::new(16), SimVfs::new(16));
    open_in(&a.as_vfs(), &random).unwrap().close().unwrap();
    open_in(&b.as_vfs(), &random).unwrap().close().unwrap();
    assert_ne!(id_of(&a), id_of(&b));
    // The id never changes once the database exists.
    let id = id_of(&a);
    open_in(&a.as_vfs(), &random).unwrap().close().unwrap();
    assert_eq!(id_of(&a), id);
}
