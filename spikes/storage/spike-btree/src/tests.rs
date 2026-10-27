//! Engine-level tests specific to the B+ tree (the shared contract suites
//! live in `tests/shared.rs`). Most of them run [`BTreeEngine::verify`],
//! which walks every page and checks the page accounting and free list.

use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use spike_core::harness::{Model, Workload, apply, read_all};
use spike_core::rng::Rng;
use spike_core::vfs::{CrashKind, SimVfs, Vfs};
use spike_core::{Durability, Engine, Error, MAX_KEY_LEN, MAX_VALUE_LEN, Options, Snapshot, WriteBatch};

use crate::BTreeEngine;
use crate::page::{LEAF_ENTRY_OVERHEAD, MAX_INLINE_ENTRY, NODE_CAPACITY, OVERFLOW_CAPACITY, PAGE_SIZE, UNDERFULL};

const DIR: &str = "/db";
const DATA: &str = "/db/data.kbt";

fn opts() -> Options {
    Options::small_for_tests()
}

fn open(vfs: &Arc<dyn Vfs>) -> BTreeEngine {
    BTreeEngine::open(vfs.clone(), Path::new(DIR), &opts()).unwrap()
}

fn stat(db: &BTreeEngine, name: &str) -> u64 {
    db.stats().into_iter().find(|(k, _)| k == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

fn put(db: &BTreeEngine, k: &[u8], v: &[u8]) -> u64 {
    let mut b = WriteBatch::new();
    b.put(k, v);
    db.commit(&b).unwrap()
}

fn check(db: &BTreeEngine, model: &Model) {
    db.verify().unwrap();
    let got = read_all(&db.snapshot().unwrap()).unwrap();
    assert!(got == *model, "contents differ: {}", spike_core::harness::diff(model, &got));
}

/// Random histories with large batches, live snapshots and reopens, with a
/// full structural check after every commit.
#[test]
fn randomized_histories_keep_every_invariant() {
    for seed in 0..12u64 {
        let disk = SimVfs::new(seed);
        let vfs = disk.as_vfs();
        let mut o = opts();
        if seed % 3 == 2 {
            o.durability = Durability::Buffered;
        }
        let mut db = BTreeEngine::open(vfs.clone(), Path::new(DIR), &o).unwrap();
        let mut w = Workload::new(seed);
        w.key_space = [50, 400, 3000][seed as usize % 3];
        w.max_batch = [3, 40, 400][(seed as usize / 3) % 3];
        let mut rng = Rng::new(seed);
        let mut model = Model::new();
        let mut snaps: Vec<(crate::BTreeSnapshot, Model)> = Vec::new();
        for i in 0..250 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
            db.verify().unwrap_or_else(|e| panic!("seed {seed} commit {i}: {e}"));
            if rng.chance(1, 10) {
                snaps.push((db.snapshot().unwrap(), model.clone()));
            }
            if snaps.len() > 4 || (rng.chance(1, 15) && !snaps.is_empty()) {
                let (s, m) = snaps.remove(rng.below(snaps.len() as u64) as usize);
                assert!(read_all(&s).unwrap() == m, "seed {seed}: snapshot {} changed", s.seq());
            }
            if i % 60 == 59 {
                snaps.clear();
                if rng.chance(1, 2) {
                    db.close().unwrap();
                } else {
                    drop(db);
                }
                db = BTreeEngine::open(vfs.clone(), Path::new(DIR), &o).unwrap();
                assert_eq!(stat(&db, "free_pages_pending"), 0, "everything free is reusable after reopen");
                check(&db, &model);
            }
        }
        for (s, m) in &snaps {
            assert!(read_all(s).unwrap() == *m);
        }
        check(&db, &model);
    }
}

#[test]
fn leaf_split_and_merge_at_the_exact_page_boundary() {
    let vfs = SimVfs::new(1).as_vfs();
    let db = open(&vfs);
    // Four entries of exactly MAX_INLINE_ENTRY bytes fill a leaf completely.
    let vlen = MAX_INLINE_ENTRY - LEAF_ENTRY_OVERHEAD - 2;
    let key = |i: u32| format!("k{i}").into_bytes();
    let mut b = WriteBatch::new();
    for i in 0..4 {
        b.put(key(i), vec![i as u8; vlen]);
    }
    db.commit(&b).unwrap();
    assert_eq!(4 * MAX_INLINE_ENTRY, NODE_CAPACITY);
    assert_eq!(stat(&db, "height"), 1, "a page filled to the last byte must not split");
    assert_eq!(stat(&db, "overflow_pages_written"), 0, "values of exactly the threshold are inline");
    // One more byte anywhere forces a split into two leaves under a root.
    put(&db, b"k4", b"");
    assert_eq!(stat(&db, "height"), 2);
    let report = db.verify().unwrap();
    assert_eq!(report.tree_pages, 3);
    // Deleting it again merges the leaves and collapses the root.
    let mut b = WriteBatch::new();
    b.delete("k4").delete("k3").delete("k2");
    db.commit(&b).unwrap();
    assert_eq!(stat(&db, "height"), 1, "merged leaves and collapsed root");
    assert_eq!(db.verify().unwrap().tree_pages, 1);
    // A value one byte over the threshold goes to an overflow page.
    put(&db, b"k9", &vec![1u8; vlen + 1]);
    assert_eq!(stat(&db, "overflow_pages_written"), 1);
    let s = db.snapshot().unwrap();
    assert_eq!(s.get(b"k9").unwrap().unwrap(), vec![1u8; vlen + 1]);
    assert_eq!(s.get(b"k0").unwrap().unwrap(), vec![0u8; vlen]);
}

#[test]
fn bulk_load_and_bulk_delete_in_single_batches() {
    let vfs = SimVfs::new(2).as_vfs();
    let db = open(&vfs);
    let mut model = Model::new();
    let mut b = WriteBatch::new();
    for i in 0..30_000u32 {
        b.put(format!("bulk-{i:08}"), i.to_be_bytes().repeat(5));
    }
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    let h = stat(&db, "height");
    assert!(h >= 3, "30k keys must need at least three levels, got {h}");
    check(&db, &model);
    // Half-empty leaves are not underfull: nothing merges.
    let before = db.verify().unwrap().tree_pages;
    let mut b = WriteBatch::new();
    for i in (0..30_000u32).step_by(8) {
        for j in 1..4 {
            b.delete(format!("bulk-{:08}", i + j));
        }
    }
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    check(&db, &model);
    let half = db.verify().unwrap().tree_pages;
    assert!(half >= before - 2, "{before} -> {half}");
    // Keeping one key in eight leaves them underfull: they merge.
    let mut b = WriteBatch::new();
    for i in (0..30_000u32).step_by(8) {
        for j in 4..8 {
            b.delete(format!("bulk-{:08}", i + j));
        }
    }
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    check(&db, &model);
    let after = db.verify().unwrap().tree_pages;
    assert!(after < before / 2, "underfull leaves should merge: {before} -> {after} pages");
    // Delete everything: the tree becomes empty and every page is free.
    let mut b = WriteBatch::new();
    for k in model.keys() {
        b.delete(k.clone());
    }
    db.commit(&b).unwrap();
    model.clear();
    check(&db, &model);
    assert_eq!(stat(&db, "height"), 0);
    let r = db.verify().unwrap();
    assert_eq!(r.tree_pages, 0);
    assert_eq!(r.free_pages + r.freelist_pages + 2, r.page_count);
}

#[test]
fn deletes_merge_underfull_nodes() {
    let vfs = SimVfs::new(3).as_vfs();
    let db = open(&vfs);
    let mut model = Model::new();
    for chunk in 0..40u32 {
        let mut b = WriteBatch::new();
        for i in 0..100 {
            b.put(format!("m{:06}", chunk * 100 + i), vec![7u8; 60]);
        }
        db.commit(&b).unwrap();
        apply(&mut model, &b);
    }
    let full = db.verify().unwrap().tree_pages;
    // Random single-key deletes until only 5% are left.
    let mut rng = Rng::new(3);
    let mut keys: Vec<Vec<u8>> = model.keys().cloned().collect();
    while keys.len() > 200 {
        let k = keys.swap_remove(rng.below(keys.len() as u64) as usize);
        let mut b = WriteBatch::new();
        b.delete(k);
        db.commit(&b).unwrap();
        apply(&mut model, &b);
    }
    check(&db, &model);
    let r = db.verify().unwrap();
    // 200 entries of ~75 bytes need ~4 full pages; without merging the
    // tree would keep (almost) all of its original leaves.
    assert!(r.tree_pages * 4 < full, "{} tree pages left of {full}", r.tree_pages);
    let leaves_min = (200 * 75usize).div_ceil(PAGE_SIZE) as u64;
    assert!(r.tree_pages <= 4 * leaves_min * PAGE_SIZE as u64 / UNDERFULL as u64 + 2);
}

#[test]
fn each_page_is_copied_once_per_commit() {
    let vfs = SimVfs::new(4).as_vfs();
    let db = open(&vfs);
    let mut b = WriteBatch::new();
    for i in 0..5000u32 {
        b.put(format!("p{i:06}"), vec![1u8; 20]);
    }
    db.commit(&b).unwrap();
    let height = stat(&db, "height");
    assert!(height >= 2);
    // 20 overwrites that all land in the same leaf: one copy per level.
    let before = stat(&db, "tree_pages_written");
    let mut b = WriteBatch::new();
    for i in 1000..1020u32 {
        b.put(format!("p{i:06}"), vec![2u8; 20]);
    }
    db.commit(&b).unwrap();
    assert_eq!(stat(&db, "tree_pages_written") - before, height);
}

#[test]
fn overflow_chains_at_size_boundaries() {
    let vfs = SimVfs::new(5).as_vfs();
    let db = open(&vfs);
    let mut model = Model::new();
    let sizes = [OVERFLOW_CAPACITY - 1, OVERFLOW_CAPACITY, OVERFLOW_CAPACITY + 1, 3 * OVERFLOW_CAPACITY, MAX_VALUE_LEN];
    let mut rng = Rng::new(5);
    for (i, &n) in sizes.iter().enumerate() {
        let before = stat(&db, "overflow_pages_written");
        let mut b = WriteBatch::new();
        b.put(format!("v{i}"), rng.bytes(n));
        db.commit(&b).unwrap();
        apply(&mut model, &b);
        assert_eq!(stat(&db, "overflow_pages_written") - before, n.div_ceil(OVERFLOW_CAPACITY) as u64);
    }
    // A maximal key with a large value, then overwrite and delete large
    // values: their chains are freed.
    let mut b = WriteBatch::new();
    b.put(vec![b'K'; MAX_KEY_LEN], rng.bytes(10_000)).put("v4", b"small".to_vec()).delete("v3");
    db.commit(&b).unwrap();
    apply(&mut model, &b);
    check(&db, &model);
    let r = db.verify().unwrap();
    assert_eq!(r.overflow_pages, 1 + 1 + 2 + 3, "v0..v2 plus the 10 KB value");
    db.close().unwrap();
    let db = open(&vfs);
    check(&db, &model);
}

#[test]
fn corrupt_overflow_pointer_is_detected_without_looping() {
    for (offset_in_page, mask, what) in
        [(8usize, 0x01u8, "self loop"), (15, 0x80, "far beyond EOF"), (100, 0x04, "payload")]
    {
        let disk = SimVfs::new(6);
        let vfs = disk.as_vfs();
        let db = open(&vfs);
        // Fresh file: the chain gets pages 2, 3, 4 and the leaf page 5.
        put(&db, b"big", &vec![9u8; 3 * OVERFLOW_CAPACITY]);
        db.close().unwrap();
        // Page 2's next pointer is 3; flipping bit 0 makes it point at 2.
        disk.corrupt_byte(Path::new(DATA), 2 * PAGE_SIZE as u64 + offset_in_page as u64, mask);
        let db = open(&vfs);
        let s = db.snapshot().unwrap();
        assert!(matches!(s.get(b"big"), Err(Error::Corruption(_))), "{what}");
        assert!(s.scan(Bound::Unbounded, Bound::Unbounded, 10).is_err(), "{what}");
        // Overwriting the value must not free the wrong pages either.
        let mut b = WriteBatch::new();
        b.put("big", "x");
        assert!(db.commit(&b).is_err(), "{what}");
        assert!(matches!(db.commit(&b), Err(Error::Poisoned(_))));
    }
}

#[test]
fn free_list_survives_reopen_and_space_is_reused() {
    let vfs = SimVfs::new(7).as_vfs();
    let mut model = Model::new();
    let mut peak = 0;
    for round in 0..6 {
        let db = open(&vfs);
        check(&db, &model);
        let mut rng = Rng::new(round);
        for chunk in 0..10u32 {
            let mut b = WriteBatch::new();
            for i in 0..100 {
                let len = rng.range(10, 2000) as usize;
                b.put(format!("f{:05}", chunk * 100 + i), rng.bytes(len));
            }
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        let pages = stat(&db, "pages_total");
        if round == 0 {
            peak = pages;
        } else {
            assert!(pages <= peak + peak / 4, "round {round}: file grew to {pages} pages (first round {peak})");
        }
        for chunk in 0..10u32 {
            let mut b = WriteBatch::new();
            for i in 0..100 {
                b.delete(format!("f{:05}", chunk * 100 + i));
            }
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        let free_before = stat(&db, "free_pages");
        check(&db, &model);
        if round % 2 == 0 {
            db.close().unwrap();
        } else {
            drop(db);
        }
        let db = open(&vfs);
        assert_eq!(stat(&db, "free_pages"), free_before, "free pages lost or leaked across reopen");
        assert_eq!(stat(&db, "free_pages_reusable"), free_before);
        drop(db);
    }
}

#[test]
fn free_list_log_is_compacted() {
    let vfs = SimVfs::new(8).as_vfs();
    let db = open(&vfs);
    let mut model = Model::new();
    let mut w = Workload::new(8);
    w.key_space = 300;
    for _ in 0..3000 {
        let b = w.batch(&model);
        db.commit(&b).unwrap();
        apply(&mut model, &b);
    }
    assert!(stat(&db, "freelist_compactions") > 0);
    let pages = stat(&db, "freelist_log_pages");
    let min = stat(&db, "free_pages").div_ceil(508);
    assert!(pages <= 2 * min + 12, "free-list log of {pages} pages for {min} pages of free ids");
    check(&db, &model);
}

#[test]
fn snapshots_pin_freed_pages_until_dropped() {
    let vfs = SimVfs::new(9).as_vfs();
    let db = open(&vfs);
    for i in 0..500u32 {
        put(&db, format!("s{i:04}").as_bytes(), &[1u8; 50]);
    }
    let snap = db.snapshot().unwrap();
    let before = read_all(&snap).unwrap();
    for i in 0..500u32 {
        put(&db, format!("s{i:04}").as_bytes(), &[2u8; 50]);
    }
    let pending = stat(&db, "free_pages_pending");
    assert!(pending > 50, "pages of the old tree must stay pinned ({pending})");
    assert_eq!(stat(&db, "live_snapshots"), 1);
    assert!(read_all(&snap).unwrap() == before);
    drop(snap);
    put(&db, b"one-more", b"x");
    assert!(stat(&db, "free_pages_pending") < 10, "dropping the snapshot releases its pages");
    db.verify().unwrap();
}

#[test]
fn meta_slot_selection_after_damage() {
    // Clean close writes both slots: either one alone is enough.
    for slot in 0..2u64 {
        let disk = SimVfs::new(10);
        let vfs = disk.as_vfs();
        let db = open(&vfs);
        for i in 0..7u32 {
            put(&db, format!("a{i}").as_bytes(), b"v");
        }
        db.close().unwrap();
        disk.corrupt_byte(Path::new(DATA), slot * PAGE_SIZE as u64 + 20, 0x10);
        let db = open(&vfs);
        assert_eq!(db.snapshot().unwrap().seq(), 7, "slot {slot} damaged");
        db.verify().unwrap();
        // The next clean close repairs the damaged slot.
        db.close().unwrap();
        disk.corrupt_byte(Path::new(DATA), (1 - slot) * PAGE_SIZE as u64 + 20, 0x10);
        assert_eq!(open(&vfs).snapshot().unwrap().seq(), 7);
    }
    // Without a clean close the slots hold the last two commits; a torn
    // newest slot falls back to the previous commit, intact.
    let disk = SimVfs::new(11);
    let vfs = disk.as_vfs();
    let db = open(&vfs);
    let mut model = Model::new();
    let mut w = Workload::new(11);
    let mut models = Vec::new();
    for _ in 0..40 {
        let b = w.batch(&model);
        db.commit(&b).unwrap();
        apply(&mut model, &b);
        models.push(model.clone());
    }
    drop(db);
    // The initial metas fill both slots, commit 1 goes to slot 1, so commit
    // 40 lives in slot 0: tear it (damage its crc'd bytes).
    disk.corrupt_byte(Path::new(DATA), 30, 0xFF);
    let db = open(&vfs);
    let s = db.snapshot().unwrap();
    assert_eq!(s.seq(), 39);
    assert!(read_all(&s).unwrap() == models[38]);
    drop(s);
    db.verify().unwrap();
    drop(db);
    // Both slots damaged: an error, never a fresh empty database.
    disk.corrupt_byte(Path::new(DATA), PAGE_SIZE as u64 + 30, 0xFF);
    assert!(matches!(BTreeEngine::open(vfs.clone(), Path::new(DIR), &opts()), Err(Error::Corruption(_))));
}

#[test]
fn every_tree_page_bit_flip_is_detected() {
    let disk = SimVfs::new(12);
    let vfs = disk.as_vfs();
    let db = open(&vfs);
    let mut model = Model::new();
    let mut w = Workload::new(12);
    for _ in 0..30 {
        let b = w.batch(&model);
        db.commit(&b).unwrap();
        apply(&mut model, &b);
    }
    let pages = stat(&db, "pages_total");
    db.close().unwrap();
    let mut rng = Rng::new(12);
    let mut detected = 0;
    for page in 2..pages {
        let at = page * PAGE_SIZE as u64 + rng.below(PAGE_SIZE as u64);
        let mask = 1 << rng.below(8);
        disk.corrupt_byte(Path::new(DATA), at, mask);
        let outcome = BTreeEngine::open(vfs.clone(), Path::new(DIR), &opts()).and_then(|db| {
            let s = db.snapshot()?;
            let all = read_all(&s)?;
            for k in model.keys() {
                s.get(k)?;
            }
            Ok(all)
        });
        match outcome {
            Ok(all) => assert!(all == model, "flip in page {page} returned wrong data"),
            Err(_) => detected += 1,
        }
        disk.corrupt_byte(Path::new(DATA), at, mask);
    }
    assert!(detected > 0);
}

#[test]
fn failed_io_poisons_the_handle() {
    for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill] {
        for after in 1..12 {
            let disk = SimVfs::new(13);
            let vfs = disk.as_vfs();
            let db = open(&vfs);
            put(&db, b"a", b"1");
            disk.crash_after(after, kind);
            let mut results = Vec::new();
            for i in 0..3 {
                let mut b = WriteBatch::new();
                b.put(format!("k{i}"), vec![3u8; 9000]);
                results.push(db.commit(&b));
            }
            let first_err = results.iter().position(|r| r.is_err()).expect("the device died");
            assert!(results[first_err + 1..].iter().all(|r| matches!(r, Err(Error::Poisoned(_)))));
            assert!(matches!(db.close(), Err(Error::Poisoned(_))));
            // Recovery from the post-crash image works.
            let image = disk.take_crash_image().unwrap();
            let db = BTreeEngine::open(image.as_vfs(), Path::new(DIR), &opts()).unwrap();
            let s = db.snapshot().unwrap().seq();
            assert!(s > first_err as u64 && s <= 2 + first_err as u64, "recovered {s}");
            db.verify().unwrap();
        }
    }
}

#[test]
fn fsyncs_per_commit() {
    let vfs = SimVfs::new(14).as_vfs();
    let db = open(&vfs);
    put(&db, b"a", b"1");
    let before = stat(&db, "fsyncs");
    for i in 0..10u32 {
        put(&db, format!("k{i}").as_bytes(), b"v");
    }
    assert_eq!(stat(&db, "fsyncs") - before, 20, "two fsyncs per Sync commit");
    db.close().unwrap();

    let mut o = opts();
    o.durability = Durability::Buffered;
    let db = BTreeEngine::open(vfs.clone(), Path::new(DIR), &o).unwrap();
    for i in 0..10u32 {
        put(&db, format!("b{i}").as_bytes(), b"v");
    }
    assert_eq!(stat(&db, "fsyncs"), 0, "buffered commits do not fsync");
    db.close().unwrap();
    let db = open(&vfs);
    assert_eq!(db.snapshot().unwrap().get(b"b9").unwrap().as_deref(), Some(&b"v"[..]));
}

#[test]
fn lingering_snapshot_keeps_the_database_locked() {
    let vfs = SimVfs::new(15).as_vfs();
    let db = open(&vfs);
    put(&db, b"a", b"1");
    let snap = db.snapshot().unwrap();
    db.close().unwrap();
    assert!(matches!(BTreeEngine::open(vfs.clone(), Path::new(DIR), &opts()), Err(Error::Locked)));
    assert_eq!(snap.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
    drop(snap);
    assert_eq!(open(&vfs).snapshot().unwrap().seq(), 1);
}

#[test]
fn concurrent_snapshots_during_page_reuse() {
    let vfs = SimVfs::new(16).as_vfs();
    let db = Arc::new(open(&vfs));
    let mut b = WriteBatch::new();
    for i in 0..2000u32 {
        b.put(format!("c{i:05}"), (i as u64).to_le_bytes().to_vec());
    }
    db.commit(&b).unwrap();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (db, stop) = (db.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    let s = db.snapshot().unwrap();
                    // Every value encodes the commit that wrote it.
                    let rows = s.scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap();
                    assert_eq!(rows.len(), 2000);
                    let round = u64::from_le_bytes(rows[0].1[..8].try_into().unwrap()) >> 32;
                    assert!(rows.iter().all(|(_, v)| u64::from_le_bytes(v[..8].try_into().unwrap()) >> 32 == round));
                }
            })
        })
        .collect();
    for round in 1..200u64 {
        let mut b = WriteBatch::new();
        for i in 0..2000u64 {
            b.put(format!("c{i:05}"), ((round << 32) | i).to_le_bytes().to_vec());
        }
        db.commit(&b).unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for r in readers {
        r.join().unwrap();
    }
    db.verify().unwrap();
}

/// Buffered commits are not fsynced, so a power loss may leave a meta whose
/// pages never reached the disk, or pages of an older tree overwritten by
/// reuse. The tree is checksummed from the meta down, so the outcome must be
/// either the exact state of some commit at or after the last clean close,
/// or a detected error — never wrong data.
#[test]
fn buffered_power_loss_gives_a_prefix_or_a_detected_error() {
    let (mut prefix, mut at_open, mut on_read) = (0, 0, 0);
    for seed in 0..150u64 {
        let disk = SimVfs::new(seed);
        let vfs = disk.as_vfs();
        let mut o = opts();
        o.durability = Durability::Buffered;
        let mut w = Workload::new(seed);
        w.key_space = 200;
        let mut rng = Rng::new(seed);
        let mut models = vec![Model::new()];
        let db = BTreeEngine::open(vfs.clone(), Path::new(DIR), &o).unwrap();
        for _ in 0..rng.range(5, 60) {
            let mut m = models.last().unwrap().clone();
            let b = w.batch(&m);
            db.commit(&b).unwrap();
            apply(&mut m, &b);
            models.push(m);
        }
        db.close().unwrap();
        let closed = models.len() as u64 - 1;
        let db = BTreeEngine::open(vfs.clone(), Path::new(DIR), &o).unwrap();
        disk.crash_after(rng.range(1, 400), CrashKind::PowerLoss);
        for _ in 0..rng.range(1, 80) {
            // models[i] is the state after commit i, including the one in
            // flight when the device died.
            let mut m = models.last().unwrap().clone();
            let b = w.batch(&m);
            apply(&mut m, &b);
            models.push(m);
            if db.commit(&b).is_err() {
                break;
            }
        }
        let image =
            if disk.has_crashed() { disk.take_crash_image().unwrap() } else { disk.crash_now(CrashKind::PowerLoss) };
        drop(db);
        let Ok(db) = BTreeEngine::open(image.as_vfs(), Path::new(DIR), &o) else {
            // Typically: the newest meta landed but the free-list page or
            // the file extension it refers to did not.
            at_open += 1;
            continue;
        };
        let s = db.snapshot().unwrap();
        let seq = s.seq();
        assert!(seq >= closed && seq < models.len() as u64, "seed {seed}: recovered {seq}");
        match read_all(&s) {
            Err(_) => on_read += 1,
            Ok(all) => {
                assert!(all == models[seq as usize], "seed {seed}: wrong data at seq {seq}");
                prefix += 1;
            }
        }
    }
    eprintln!("buffered power loss: prefix {prefix}, detected at open {at_open}, detected on read {on_read}");
    assert!(prefix > 0);
}
