//! Hostile files whose checksums are all valid (R-85). A bit flip breaks a
//! CRC before any structural rule is reached, so these are built by hand with
//! the format helpers and sealed again: the checks that must hold are the
//! structural ones (SST-O*, SST-R*, SST-V*, MAN-S*, WAL-C4, VAR-*), not the
//! checksums.
//!
//! Each case goes through every way into the directory (`verify`, `open`,
//! `get`, `scan`) and must, on every one of them:
//!
//! * not panic and not hang (the probe runs in a thread with a deadline);
//! * fail with `Corruption` and nothing else when it fails;
//! * not allocate more than the engine needs for an honest directory, or
//!   twice the size of the largest file, whichever is more: a length field
//!   that claims gigabytes must not be believed (see `ALLOC_FLOOR`);
//!
//! and `verify` must name the rule that is broken.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::ops::Bound;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::format::coding::{put_bytes, put_varint};
use kerno_storage::format::header::Header;
use kerno_storage::format::limits::{MAX_COMMIT_OPS, MAX_FILE_NUMBER, MAX_SEQ};
use kerno_storage::format::manifest::{KIND_SNAPSHOT, seal_record};
use kerno_storage::format::rangedel::Fragment;
use kerno_storage::format::sst::IndexEntry;
use kerno_storage::format::wal::seal_frame;
use kerno_storage::{KernoDb, VerifyOptions, VerifyReport, verify};
use kerno_testkit::vfs::{DirImage, SimVfs, assert_dir_unchanged};

use common::craft::*;
use common::*;

// ---- allocation tracking ----------------------------------------------------

/// Remembers, per thread, the largest single allocation requested.
struct Tracking;

thread_local! {
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    LARGEST.with(|m| m.set(m.get().max(size)));
}

// SAFETY: every call is forwarded unchanged to the system allocator; the only
// addition is a write to a `const`-initialised thread-local `Cell<usize>`,
// which does not allocate.
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

/// What the engine allocates for itself whatever the files hold (block cache
/// shards, memtable, write buffers), measured on an honest directory. A claim
/// in a hostile file that the engine believed would be orders of magnitude
/// above this.
const ALLOC_FLOOR: usize = 128 << 10;

// ---- probing a directory ----------------------------------------------------

struct Probe {
    verify: kerno_core::Result<VerifyReport>,
    /// What opening read-write and then reading everything said.
    reads: kerno_core::Result<()>,
    max_alloc: usize,
}

/// Keys the honest tables of these tests hold, and some they do not.
const PROBE_KEYS: [&[u8]; 4] = [b"k00000", b"k00017", b"k00119", b"zzz"];

fn open_and_read(disk: &SimVfs) -> kerno_core::Result<()> {
    let db = open_in(&disk.as_vfs(), &config(Durability::Sync))?;
    let snap = db.snapshot()?;
    for key in PROBE_KEYS {
        snap.get(key)?;
    }
    snap.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX)?;
    Ok(())
}

fn run_probe(image: &DirImage) -> Probe {
    LARGEST.with(|m| m.set(0));
    let disk = SimVfs::restore(image);
    let before = disk.dump();
    let verified = verify(disk.as_vfs(), Path::new(DIR), &VerifyOptions::default());
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    let reads = open_and_read(&SimVfs::restore(image));
    Probe { verify: verified, reads, max_alloc: LARGEST.with(Cell::get) }
}

/// [`run_probe`] in its own thread (its allocations are the ones counted),
/// with a deadline.
fn probe(image: &DirImage) -> Probe {
    let image = image.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(run_probe(&image));
    });
    rx.recv_timeout(Duration::from_secs(120)).expect("the probe hung or panicked")
}

fn largest_file(image: &DirImage) -> usize {
    image.files.values().map(Vec::len).max().unwrap_or(0)
}

fn assert_corruption(what: &str, r: &kerno_core::Result<()>) {
    if let Err(e) = r {
        assert!(matches!(e, Error::Corruption(_)), "{what}: expected Corruption, got {e:?}");
    }
}

/// Runs every surface over `image`. `verify_rule` is the rule `verify` must
/// report; `engine_rejects` says whether `open`, `get` or `scan` must fail
/// too (some damage is only visible to a full read).
#[track_caller]
fn crafted(case: &str, image: &DirImage, verify_rule: &str, engine_rejects: bool) {
    let p = probe(image);
    let report = p.verify.unwrap_or_else(|e| panic!("{case}: verify returned an error: {e}"));
    assert!(
        report.issues.iter().any(|i| i.rule == verify_rule),
        "{case}: expected {verify_rule}, got {:?}",
        report.issues
    );
    assert_corruption(case, &p.reads);
    if engine_rejects {
        assert!(p.reads.is_err(), "{case}: the engine read the file without complaint");
    }
    let limit = ALLOC_FLOOR.max(2 * largest_file(image));
    assert!(p.max_alloc <= limit, "{case}: one allocation of {} bytes, limit {limit}", p.max_alloc);
}

/// A directory whose only table is `t` as `bytes`, with the size in the
/// MANIFEST that of `bytes`.
fn image_with_bytes(t: &Table, bytes: Vec<u8>) -> DirImage {
    let mut dir = Dir::single(t);
    dir.tables[0].0.size = bytes.len() as u64;
    dir.tables[0].1 = bytes;
    dir.image()
}

/// An honest table: 20 blocks of 6 entries.
fn honest() -> Table {
    Table::grid(1, 'k', 20, 6, 0)
}

#[test]
fn control_honest_directory_passes_every_surface() {
    let t = honest();
    let image = Dir::single(&t).image();
    let p = probe(&image);
    let report = p.verify.unwrap();
    assert!(report.is_ok(), "{:?}", report.issues);
    p.reads.unwrap();
    println!("largest single allocation for an honest directory: {} bytes", p.max_alloc);
    assert!(p.max_alloc > 0, "the allocation counter did not count");
    assert!(p.max_alloc <= ALLOC_FLOOR, "{} bytes for an honest directory", p.max_alloc);
}

// ---- lengths and counts -----------------------------------------------------

fn varint(v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, v);
    out
}

#[test]
fn crafted_huge_lengths_and_counts() {
    let t = honest();
    let image_of = |t: &Table| Dir::single(t).image();

    // Index: a count no payload could hold.
    let mut c = t.clone();
    c.raw_index = Some(varint(1 << 62));
    crafted("index count", &image_of(&c), "SST-O4", true);

    // Index: one entry whose block claims to be 2^60 bytes.
    let mut c = t.clone();
    let mut index = varint(1);
    put_bytes(&mut index, b"k00119");
    index.extend(varint(96));
    index.extend(varint(1 << 60));
    c.raw_index = Some(index);
    crafted("index block length", &image_of(&c), "SST-O5", true);

    // Footer: a handle past the end of the file.
    let (mut bytes, _) = t.encode_layout(|_| {});
    patch_footer(&mut bytes, 1, |f| f.index.len = u64::MAX / 2);
    crafted("footer handle", &image_with_bytes(&t, bytes), "SST-O3", true);

    // Filter: nbits of 4 Gbit over a payload of a few bytes.
    let mut c = t.clone();
    let mut filter = vec![1, 7, 0, 0];
    filter.extend((u32::MAX - 7).to_le_bytes());
    filter.extend([0u8; 8]);
    c.raw_filter = Some(filter);
    crafted("filter nbits", &image_of(&c), "SST-O7", true);

    // Range block: 2^60 fragments in a few bytes.
    let mut c = t.clone();
    c.raw_rdel = Some(varint(1 << 60));
    crafted("range count", &image_of(&c), "SST-O4", true);

    // Data block: 2^32 - 1 entries in a block of a few hundred bytes, and a
    // key of 2^50 bytes.
    let (mut bytes, layout) = t.encode_layout(|_| {});
    patch_block(&mut bytes, 1, layout.index[3].handle, |payload| {
        let n = payload.len();
        payload[n - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
    });
    crafted("block count", &image_with_bytes(&t, bytes), "SST-R1", true);
    let mut c = Table::grid(1, 'k', 1, 1, 0);
    let mut payload = varint(1 << 50); // key_len
    payload.extend(varint(1)); // seq
    payload.push(2); // DELETE
    payload.extend(0u32.to_le_bytes());
    payload.extend(1u32.to_le_bytes());
    c.raw_blocks = Some(vec![payload]);
    crafted("key length", &image_of(&c), "VAR-3", true);

    // Properties: counts that no table could have. The MANIFEST is derived
    // from them, so only a full read finds out.
    let mut c = t.clone();
    c.props.point_entries = u64::MAX / 2;
    crafted("point_entries", &image_of(&c), "SST-V1", false);
    let mut c = t.clone();
    c.props.data_blocks = 1 << 60;
    crafted("data_blocks", &image_of(&c), "SST-O5", true);

    // WAL: more operations in a commit than the format allows, and fewer
    // bytes than the operations it announces.
    for (case, payload, rule) in [
        ("op_count above the limit", varint(MAX_COMMIT_OPS + 1), "WAL-C4"),
        ("op_count with no operations", varint(MAX_COMMIT_OPS), "WAL-C4"),
        ("key longer than the frame", [varint(1), vec![1], varint(1 << 40)].concat(), "VAR-3"),
    ] {
        crafted(case, &dirty_with_wal_frame(&payload, 1), rule, true);
    }
    // A frame announcing 65 536 commits in a few bytes (WAL-R3 allows that
    // many, the payload does not hold them).
    crafted("frame count", &dirty_with_wal_frame(&[0], 65_536), "WAL-C4", true);

    // MANIFEST: a field longer than the record.
    let mut payload = vec![10];
    payload.extend(varint(1 << 60));
    crafted("manifest field length", &with_manifest_payload(&payload), "VAR-3", true);
}

/// A directory holding no table and one WAL whose only frame has this payload
/// (sealed, so its CRC verifies) announcing `count` commits.
fn dirty_with_wal_frame(payload: &[u8], count: u32) -> DirImage {
    let mut dir = Dir::new(Vec::new());
    dir.next_file = 3;
    dir.log_number = 2;
    dir.clean = false;
    let mut image = dir.image();
    let mut wal = Header::wal(DB_ID, 2, 1).encode().to_vec();
    wal.extend(seal_frame(2, 1, 0, count, payload, false));
    image.files.insert(path("000002.wal"), wal);
    image
}

/// The honest directory with a MANIFEST whose only record is a snapshot with
/// this payload.
fn with_manifest_payload(payload: &[u8]) -> DirImage {
    let mut image = Dir::single(&honest()).image();
    let mut manifest = Header::manifest(DB_ID, 1, 1).encode().to_vec();
    manifest.extend(seal_record(1, 1, KIND_SNAPSHOT, payload));
    image.files.insert(path("MANIFEST"), manifest);
    image
}

// ---- table structure ----------------------------------------------------------

#[test]
fn crafted_index_overlapping_blocks() {
    let t = honest();
    for (case, tweak) in [
        (
            "second block at the first one's offset",
            (|i: &mut Vec<IndexEntry>| i[1].handle.offset = i[0].handle.offset) as fn(&mut Vec<IndexEntry>),
        ),
        ("a block one byte too long", |i| i[1].handle.len += 1),
        ("a block one byte too short", |i| i[1].handle.len -= 1),
        ("blocks swapped", |i| {
            let (a, b) = (i[0].handle, i[1].handle);
            i[0].handle = b;
            i[1].handle = a;
        }),
    ] {
        let (bytes, _) = t.encode_layout(tweak);
        crafted(case, &image_with_bytes(&t, bytes), "SST-O5", true);
    }
}

#[test]
fn crafted_cross_block_disorder() {
    let mut t = honest();
    // Each block is sorted and the index is increasing, but block 1 starts
    // below the end of block 0.
    t.blocks[1][0].key = t.blocks[0][2].key.clone();
    t.refresh();
    crafted("overlap", &Dir::single(&t).image(), "SST-R2", true);
    // A key repeated across two blocks.
    let mut t = honest();
    t.blocks[1][0].key = t.blocks[0].last().unwrap().key.clone();
    t.refresh();
    crafted("repeat", &Dir::single(&t).image(), "SST-R2", true);
}

#[test]
fn crafted_smallest_mismatch() {
    // The properties promise a smaller first key than there is: nothing on
    // the read path notices, `verify` does (SST-V2).
    let mut t = honest();
    t.props.smallest_key = b"a".to_vec();
    crafted("smallest below the first key", &Dir::single(&t).image(), "SST-V2", false);
    // A larger one: the first block starts below it.
    let mut t = honest();
    t.props.smallest_key = b"k00003".to_vec();
    crafted("smallest above the first key", &Dir::single(&t).image(), "SST-R2", true);
    // The MANIFEST and the file disagree (REC-5 / SST-O6).
    let t = honest();
    let mut dir = Dir::single(&t);
    dir.tables[0].0.smallest = b"a".to_vec();
    crafted("MANIFEST says another smallest", &dir.image(), "SST-O6", true);
    let mut dir = Dir::single(&t);
    dir.tables[0].0.largest = b"zzz".to_vec();
    crafted("MANIFEST says another largest", &dir.image(), "SST-O6", true);
}

#[test]
fn crafted_entries_mismatch() {
    let mut t = honest();
    t.props.point_entries += 1;
    crafted("one entry more than there is", &Dir::single(&t).image(), "SST-V1", false);
    let mut t = honest();
    t.props.point_entries -= 1;
    crafted("one entry fewer than there is", &Dir::single(&t).image(), "SST-V1", false);
    // The MANIFEST counts differently from the table.
    let mut dir = Dir::single(&honest());
    dir.tables[0].0.point_entries += 1;
    crafted("MANIFEST counts more", &dir.image(), "SST-O6", true);
}

#[test]
fn crafted_seq_above_last_seq() {
    // A table with sequences the MANIFEST says were never assigned.
    let t = honest();
    let mut dir = Dir::single(&t);
    dir.last_seq = dir.tables[0].0.largest_seq - 1;
    crafted("table above last_seq", &dir.image(), "MAN-S2", true);
    // The MANIFEST agrees with its last_seq but not with the table.
    let mut dir = Dir::single(&t);
    dir.tables[0].0.largest_seq -= 1;
    dir.last_seq = dir.tables[0].0.largest_seq;
    crafted("MANIFEST records a lower largest_seq", &dir.image(), "SST-O6", true);
    // An entry above the largest sequence its own table declares.
    let mut t = honest();
    let top = t.props.largest_seq;
    t.blocks[4][1].seq = top + 5;
    let mut dir = Dir::single(&t);
    t.props.largest_seq = top; // the properties still say `top`
    dir.tables[0].1 = t.encode();
    crafted("entry above the table's largest_seq", &dir.image(), "SST-R3", true);
    // Sequence 0 is not assigned to any commit.
    let mut t = honest();
    t.blocks[2][0].seq = 0;
    crafted("sequence 0", &Dir::single(&t).image(), "SST-R1", true);
}

/// The properties of `t` with the sequence range of its points alone.
fn points_only_seqs(t: &mut Table) {
    t.props.largest_seq = t.blocks.iter().flatten().map(|e| e.seq).max().unwrap();
}

/// The properties of `t` with the key range of its points alone.
fn points_only_keys(t: &mut Table) {
    t.props.smallest_key = t.first_key();
    t.props.largest_key = t.blocks.last().unwrap().last().unwrap().key.clone();
    t.props.largest_exclusive = false;
}

#[test]
fn crafted_overlapping_fragments() {
    let frag = |s: &str, e: &str, seq: u64| Fragment { start: s.as_bytes().to_vec(), end: e.as_bytes().to_vec(), seq };
    type Tweak = fn(&mut Table);
    for (case, fragments, rule, tweak) in [
        ("overlapping", vec![frag("k00010", "k00040", 5), frag("k00030", "k00050", 6)], "SST-F1", None),
        ("unordered", vec![frag("k00050", "k00060", 5), frag("k00010", "k00020", 6)], "SST-F1", None),
        ("empty", vec![frag("k00030", "k00030", 5)], "SST-F1", None),
        ("inverted", vec![frag("k00040", "k00030", 5)], "SST-F1", None),
        // The properties keep the extremes of the points alone, so the
        // fragment is the only thing out of range.
        (
            "sequence outside the table",
            vec![frag("k00010", "k00020", 1 << 40)],
            "SST-F1",
            Some(points_only_seqs as Tweak),
        ),
        ("outside the table's keys", vec![frag("a", "b", 5)], "SST-O7", Some(points_only_keys as Tweak)),
    ] {
        let mut t = honest();
        t.fragments = fragments;
        t.refresh();
        if let Some(tweak) = tweak {
            tweak(&mut t);
        }
        crafted(case, &Dir::single(&t).image(), rule, true);
    }
}

// ---- MANIFEST structure -------------------------------------------------------

#[test]
fn crafted_l0_seq_disorder() {
    // Two tables of L0 over the same keys. The first listed is the newer one
    // and must hold the higher sequences (MAN-S4); here it holds the lower.
    let newer = Table::grid(2, 'k', 4, 6, 0);
    let older = Table::grid(1, 'k', 4, 6, 100);
    let dir = Dir::new(vec![(newer.meta(0), newer.encode()), (older.meta(0), older.encode())]);
    crafted("L0 out of order", &dir.image(), "MAN-S4", true);
    // Control: the other way round is fine.
    let dir = Dir::new(vec![(older.meta(0), older.encode()), (newer.meta(0), newer.encode())]);
    let p = probe(&dir.image());
    assert!(p.verify.unwrap().is_ok());
}

#[test]
fn crafted_overlapping_levels() {
    let a = Table::grid(1, 'k', 4, 6, 0);
    let b = Table::grid(2, 'k', 4, 6, 100);
    let dir = Dir::new(vec![(a.meta(1), a.encode()), (b.meta(1), b.encode())]);
    crafted("two files of L1 over the same keys", &dir.image(), "MAN-S3", true);
}

#[test]
fn crafted_last_seq_at_max_then_commit() {
    // The last sequence that exists. Reading works; the next commit has no
    // sequence to take and fails cleanly without writing.
    let t = honest();
    let mut dir = Dir::single(&t);
    dir.last_seq = MAX_SEQ;
    let image = dir.image();
    let disk = SimVfs::restore(&image);
    let vfs = disk.as_vfs();
    let db: KernoDb =
        open_in(&vfs, &config(Durability::Sync)).unwrap_or_else(|e| panic!("open at last_seq = MAX_SEQ: {e}"));
    assert_eq!(db.snapshot().unwrap().seq(), MAX_SEQ);
    assert!(db.snapshot().unwrap().get(b"k00000").unwrap().is_some());
    let before = disk.dump();
    let mut b = WriteBatch::new();
    b.put("k00000", "changed");
    for _ in 0..2 {
        let e = db.commit(&b).expect_err("no sequence left");
        assert!(matches!(e, Error::InvalidArgument(_)), "{e:?}");
    }
    // The handle is healthy, and nothing reached the disk.
    assert!(db.snapshot().unwrap().get(b"k00000").unwrap().is_some());
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    drop(db);
    let p = probe(&image);
    assert!(p.verify.unwrap().is_ok());
    p.reads.unwrap();
}

#[test]
fn crafted_next_file_at_limit() {
    let t = honest();
    // The last number the format has is not usable by a writer: the open
    // refuses to create a file with it, and changes nothing.
    let mut dir = Dir::single(&t);
    dir.next_file = MAX_FILE_NUMBER;
    dir.log_number = MAX_FILE_NUMBER;
    let image = dir.image();
    let disk = SimVfs::restore(&image);
    let before = disk.dump();
    let e = open_in(&disk.as_vfs(), &config(Durability::Sync)).err().expect("no file number left");
    assert!(matches!(e, Error::InvalidArgument(_)), "{e:?}");
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    // It is a valid directory, so verify has no complaint.
    let r = verify(SimVfs::restore(&image).as_vfs(), Path::new(DIR), &VerifyOptions::default()).unwrap();
    assert!(r.is_ok(), "{:?}", r.issues);
    // One past it does not exist (MAN-S1).
    let mut dir = Dir::single(&t);
    dir.next_file = MAX_FILE_NUMBER + 1;
    dir.log_number = dir.next_file;
    crafted("next_file above the limit", &dir.image(), "MAN-S1", true);
    // A table numbered at or above next_file (MAN-S1).
    let mut dir = Dir::single(&t);
    dir.next_file = 1;
    dir.log_number = 1;
    crafted("table numbered next_file", &dir.image(), "MAN-S1", true);
}

#[test]
fn crafted_overlong_varints() {
    let t = honest();
    let image_of = |t: &Table| Dir::single(t).image();
    // The same value in more bytes than it needs (VAR-2): 1 as `81 00`.
    let overlong = |v: u64| {
        let mut out = varint(v);
        let last = out.len() - 1;
        out[last] |= 0x80;
        out.push(0);
        out
    };
    // Index count.
    let mut c = t.clone();
    let mut index = overlong(1);
    put_bytes(&mut index, b"k00119");
    index.extend(varint(96));
    index.extend(varint(100));
    c.raw_index = Some(index);
    crafted("index count", &image_of(&c), "VAR-2", true);
    // Properties: the first field's value.
    let mut c = t.clone();
    let mut props = vec![1u8, 2];
    props.extend(overlong(0));
    c.raw_props = Some(props);
    crafted("property value", &image_of(&c), "VAR-2", true);
    // Range block count.
    let mut c = t.clone();
    c.raw_rdel = Some(overlong(1));
    crafted("range count", &image_of(&c), "VAR-2", true);
    // A data block: the key length of its only entry.
    let mut c = Table::grid(1, 'k', 1, 1, 0);
    let mut payload = overlong(6);
    payload.extend(varint(1)); // seq
    payload.push(2); // DELETE
    payload.extend(b"k00000");
    payload.extend(0u32.to_le_bytes());
    payload.extend(1u32.to_le_bytes());
    c.blocks[0][0].value = None;
    c.raw_blocks = Some(vec![payload]);
    crafted("entry key length", &image_of(&c), "VAR-2", true);
    // A MANIFEST field: next_file.
    let mut payload = vec![1u8, 2];
    payload.extend(overlong(2));
    crafted("manifest value", &with_manifest_payload(&payload), "VAR-2", true);
    // A WAL commit's operation count.
    let mut payload = overlong(1);
    payload.extend([2, 1, b'k']); // DELETE "k"
    crafted("wal op_count", &dirty_with_wal_frame(&payload, 1), "VAR-2", true);
}
