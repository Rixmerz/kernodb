//! Range deletion suite: what [`Op::DeleteRange`](kerno_core::Op::DeleteRange)
//! must do in every engine that implements it (see
//! [`range_delete_test_suite!`](crate::range_delete_test_suite), module
//! `shared_range`). Semantics: v1.md §11 (SEM-3, SEM-4) and the contract
//! documentation in `kerno_core::contract`.
//!
//! | Test | What it checks |
//! |---|---|
//! | `delete_range_bounds_and_degenerate` | `[start, end)`; the empty key and the largest key as bounds; `start == end` deletes nothing but is a commit; `start > end` and bounds over `MAX_KEY_LEN` are `InvalidArgument` with nothing applied and no sequence consumed |
//! | `order_within_batch_put_then_range` | a range deletes the keys put earlier in its batch |
//! | `order_within_batch_range_then_put` | a key put after a range in the same batch survives it (same sequence: the point wins) |
//! | `scan_across_tombstones_in_all_sources` | gets and paginated, bounded scans over ranges written in several generations (deep levels, recent files, the active write buffer), older snapshots unaffected |
//! | `snapshot_isolation_with_ranges_across_flush_and_compaction` | snapshots taken before narrow, wide and "delete everything" ranges keep their state while flushes and compactions run |
//! | `randomized_model_check_with_ranges` | the randomized model check with ranges in the workload |
//! | `range_survives_reopen` | ranges survive clean close, drop, process kill and (`Sync`) power loss, in `Sync` and `Buffered` mode |
//! | `space_is_reclaimed_after_range_deletes` | filling with fresh keys and range-deleting them, cycle after cycle, does not grow the files without bound: the points covered by the ranges are reclaimed, not only overwritten versions; it runs enough cycles that an engine that keeps them fails |
//!
//! The suite is validated first on the reference `LogEngine` (native) and
//! on `Emulated<LogEngine>` (`kerno-testkit/tests/reference_engine.rs`).

use std::ops::Bound;
use std::sync::Arc;

use kerno_core::contract::{Options, RangeDeleteEngine, Snapshot};
use kerno_core::{Durability, Error, MAX_KEY_LEN, WriteBatch};

use super::functional::randomized_model_check_with_config;
use super::{
    Model, SuiteConfig, Workload, WorkloadCfg, apply, assert_snapshot_matches, db_dir, diff, for_each_seed, open,
    read_all, seeds, show,
};
use crate::rng::Rng;
use crate::vfs::{CrashKind, SimVfs, Vfs, dir_size};

fn key(i: u64) -> Vec<u8> {
    format!("k{i:05}").into_bytes()
}

/// A recognizable value of about 200 bytes for key `i` in generation `g`.
fn val(i: u64, g: u32) -> Vec<u8> {
    let mut v = format!("g{g}-k{i:05}-").into_bytes();
    v.resize(200, b'.');
    v
}

fn options(durability: Durability) -> Options {
    Options { durability, ..Options::small_for_tests() }
}

/// Commits `b`, which must get sequence `*seq + 1`, and applies it to `model`.
fn commit<E: RangeDeleteEngine>(db: &E, model: &mut Model, seq: &mut u64, b: &WriteBatch) {
    let got = db.commit(b).unwrap_or_else(|e| panic!("commit {} failed: {e}", *seq + 1));
    *seq += 1;
    assert_eq!(got, *seq, "every non-empty batch gets the next sequence");
    apply(model, b);
}

fn range(start: impl Into<Vec<u8>>, end: impl Into<Vec<u8>>) -> WriteBatch {
    let mut b = WriteBatch::new();
    b.delete_range(start, end);
    b
}

/// The expected result of `scan(start, end, limit)`: empty for an inverted
/// or empty pair of bounds.
fn model_scan(model: &Model, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    if let (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) = (start, end)
        && (a > b || (a == b && !matches!((start, end), (Bound::Included(_), Bound::Included(_)))))
    {
        return Vec::new();
    }
    model.range::<[u8], _>((start, end)).take(limit).map(|(k, v)| (k.clone(), v.clone())).collect()
}

fn show_bound(b: Bound<&[u8]>) -> String {
    match b {
        Bound::Included(k) => format!("[{}", show(k)),
        Bound::Excluded(k) => format!("({}", show(k)),
        Bound::Unbounded => "..".into(),
    }
}

/// Exhaustive comparison of `snap` with `model`: full paginated scans with
/// several page sizes (each page restarts after the last key returned, often
/// inside a deleted range), `get` of every key in `probes`, and bounded
/// scans between every ordered pair of `bounds` with every kind of bound and
/// a few limits.
fn check<S: Snapshot>(snap: &S, model: &Model, probes: &[Vec<u8>], bounds: &[Vec<u8>], ctx: &str) {
    let all = read_all(snap).unwrap_or_else(|e| panic!("{ctx}: full scan failed: {e}"));
    assert!(all == *model, "{ctx}: full scan differs from the model: {}", diff(model, &all));
    for page in [1usize, 2, 3, 7, 4096] {
        let mut got = Vec::new();
        let mut last: Option<Vec<u8>> = None;
        loop {
            let start = last.as_deref().map_or(Bound::Unbounded, Bound::Excluded);
            let rows = snap.scan(start, Bound::Unbounded, page).unwrap();
            assert!(rows.len() <= page, "{ctx}: scan returned more than its limit");
            let done = rows.len() < page;
            last = rows.last().map(|(k, _)| k.clone()).or(last);
            got.extend(rows.into_iter().map(|(k, _)| k));
            if done {
                break;
            }
        }
        let want: Vec<_> = model.keys().cloned().collect();
        assert!(got == want, "{ctx}: paginated scan with pages of {page} differs from the model");
    }
    for k in probes {
        let got = snap.get(k).unwrap_or_else(|e| panic!("{ctx}: get failed: {e}"));
        assert!(got.as_ref() == model.get(k), "{ctx}: get({}) differs from the model", show(k));
    }
    let mut sorted: Vec<&[u8]> = bounds.iter().map(Vec::as_slice).collect();
    sorted.sort();
    sorted.dedup();
    let mut pairs = Vec::new();
    for (i, &a) in sorted.iter().enumerate() {
        pairs.push((Bound::Unbounded, Bound::Excluded(a)));
        pairs.push((Bound::Unbounded, Bound::Included(a)));
        pairs.push((Bound::Included(a), Bound::Unbounded));
        pairs.push((Bound::Excluded(a), Bound::Unbounded));
        pairs.push((Bound::Included(a), Bound::Included(a)));
        pairs.push((Bound::Included(a), Bound::Excluded(a)));
        for &b in &sorted[i + 1..] {
            pairs.push((Bound::Included(a), Bound::Excluded(b)));
            pairs.push((Bound::Included(a), Bound::Included(b)));
            pairs.push((Bound::Excluded(a), Bound::Excluded(b)));
            pairs.push((Bound::Excluded(a), Bound::Included(b)));
        }
    }
    for (start, end) in pairs {
        for limit in [1usize, 7, 50] {
            let got = snap.scan(start, end, limit).unwrap();
            let want = model_scan(model, start, end, limit);
            assert!(
                got == want,
                "{ctx}: scan {}, {}) limit {limit} returned {} rows, expected {}",
                show_bound(start),
                show_bound(end),
                got.len(),
                want.len()
            );
        }
    }
}

/// Reopens `db` three ways and checks the state and the sequence each time:
/// first a crash right after the last acknowledged commit (power loss in
/// `Sync` mode, process kill in `Buffered` mode: either way every
/// acknowledged commit must survive), then drop without `close` and reopen,
/// then a clean `close` and reopen. Returns the handle and the device it
/// lives on (the crash image).
#[allow(clippy::too_many_arguments)]
fn reopen_and_check<E: RangeDeleteEngine>(
    db: E,
    disk: SimVfs,
    opts: &Options,
    model: &Model,
    seq: u64,
    probes: &[Vec<u8>],
    bounds: &[Vec<u8>],
    ctx: &str,
) -> (E, SimVfs) {
    let kind = if opts.durability == Durability::Sync { CrashKind::PowerLoss } else { CrashKind::ProcessKill };
    let image = disk.crash_now(kind);
    drop(db);
    let vfs = image.as_vfs();
    let db: E = open(&vfs, opts).unwrap_or_else(|e| panic!("{ctx}: recovery after {kind:?}: {e}"));
    assert_eq!(db.snapshot().unwrap().seq(), seq, "{ctx}: sequence after {kind:?}");
    check(&db.snapshot().unwrap(), model, probes, bounds, &format!("{ctx}, after {kind:?}"));
    drop(db);
    let db: E = open(&vfs, opts).unwrap_or_else(|e| panic!("{ctx}: reopen after drop: {e}"));
    assert_eq!(db.snapshot().unwrap().seq(), seq, "{ctx}: sequence after drop and reopen");
    check(&db.snapshot().unwrap(), model, probes, bounds, &format!("{ctx}, after drop and reopen"));
    db.close().unwrap();
    let db: E = open(&vfs, opts).unwrap_or_else(|e| panic!("{ctx}: reopen after close: {e}"));
    assert_eq!(db.snapshot().unwrap().seq(), seq, "{ctx}: sequence after close and reopen");
    check(&db.snapshot().unwrap(), model, probes, bounds, &format!("{ctx}, after close and reopen"));
    (db, image)
}

pub fn delete_range_bounds_and_degenerate<E: RangeDeleteEngine>() {
    let disk = SimVfs::new(1);
    let opts = options(Durability::Sync);
    let db: E = open(&disk.as_vfs(), &opts).unwrap();
    let max_key = vec![0xffu8; MAX_KEY_LEN];
    let over = vec![b'z'; MAX_KEY_LEN + 1];
    let names: [&[u8]; 9] = [b"", b"a", b"b", b"b\0", b"ba", b"bb", b"c", b"d", b"e"];
    let mut keys: Vec<Vec<u8>> = names.iter().map(|k| k.to_vec()).collect();
    keys.push(max_key.clone());
    let mut probes = keys.clone();
    probes.extend([b"a\0".to_vec(), b"bc".to_vec(), b"zz".to_vec(), over.clone()]);
    let bounds = probes.clone();
    let mut model = Model::new();
    let mut seq = 0;
    let mut b = WriteBatch::new();
    for k in &keys {
        b.put(k.clone(), [b"v-".as_slice(), k.as_slice()].concat());
    }
    commit(&db, &mut model, &mut seq, &b);

    // `start` is included and `end` excluded.
    commit(&db, &mut model, &mut seq, &range("b", "bb"));
    for (k, present) in [("a", true), ("b", false), ("b\0", false), ("ba", false), ("bb", true)] {
        assert_eq!(model.contains_key(k.as_bytes()), present, "model sanity for {k:?}");
    }
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "after [b, bb)");
    // `start == end` deletes nothing, not even `start`, but is a commit.
    let before = model.clone();
    commit(&db, &mut model, &mut seq, &range("c", "c"));
    assert_eq!(model, before);
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "after [c, c)");
    // A range that covers no key is a commit that changes nothing.
    commit(&db, &mut model, &mut seq, &range("bc", "bz"));
    assert_eq!(model, before);
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "after [bc, bz)");
    // The empty key is the smallest key: `["", "a")` deletes exactly it.
    commit(&db, &mut model, &mut seq, &range("", "a"));
    assert!(!model.contains_key(&b""[..]) && model.contains_key(&b"a"[..]));
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "after [\"\", a)");
    // The largest key of MAX_KEY_LEN bytes works as an end bound (and, being
    // the largest key there is, no range can delete it).
    commit(&db, &mut model, &mut seq, &range("d", max_key.clone()));
    assert!(!model.contains_key(&b"e"[..]) && model.contains_key(&max_key));
    commit(&db, &mut model, &mut seq, &range(max_key.clone(), max_key.clone()));
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "after [d, max)");
    // A batch made only of empty ranges is still a commit.
    let mut b = range("a", "a");
    b.delete_range("zz", "zz").delete_range("", "");
    commit(&db, &mut model, &mut seq, &b);

    // Invalid batches: nothing applied, no sequence consumed, handle intact.
    let mut inverted = WriteBatch::new();
    inverted.put("new", "x").delete_range("z", "a");
    let mut long_end = WriteBatch::new();
    long_end.put("new", "x").delete_range("a", over.clone());
    let mut long_start = WriteBatch::new();
    long_start.delete_range(over.clone(), over.clone());
    for (bad, what) in
        [(&inverted, "start > end"), (&long_end, "end too long"), (&long_start, "empty range, bounds too long")]
    {
        let r = db.commit(bad);
        assert!(matches!(r, Err(Error::InvalidArgument(_))), "{what}: expected InvalidArgument, got {r:?}");
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), seq, "{what}: a rejected batch consumed a sequence");
        assert_eq!(snap.get(b"new").unwrap(), None, "{what}: a rejected batch was partially applied");
    }
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "after the rejected batches");
    let mut b = WriteBatch::new();
    b.put("bb", "after").delete_range("ba", "bb");
    commit(&db, &mut model, &mut seq, &b);
    reopen_and_check(db, disk, &opts, &model, seq, &probes, &bounds, "bounds");
}

pub fn order_within_batch_put_then_range<E: RangeDeleteEngine>() {
    let disk = SimVfs::new(2);
    let opts = options(Durability::Sync);
    let db: E = open(&disk.as_vfs(), &opts).unwrap();
    let name = |p: &str, i: u32| format!("{p}{i}").into_bytes();
    let mut probes: Vec<Vec<u8>> = Vec::new();
    for p in ["a", "b", "c", "d", "e", "f", "g"] {
        probes.extend((0..10).map(|i| name(p, i)));
    }
    let bounds = probes.clone();
    let mut model = Model::new();
    let mut seq = 0;
    let mut b = WriteBatch::new();
    for i in 1..=9 {
        b.put(name("a", i), "old");
    }
    commit(&db, &mut model, &mut seq, &b);
    let mut snaps = vec![(db.snapshot().unwrap(), model.clone())];

    // A key put earlier in the batch, inside the range: deleted with the
    // older keys there. Keys put outside the range stay.
    let mut b = WriteBatch::new();
    b.put("a5", "new").put("b1", "x").put("a0", "edge").delete_range("a3", "a7");
    commit(&db, &mut model, &mut seq, &b);
    assert!(!model.contains_key(&b"a5"[..]) && model.contains_key(&b"a7"[..]) && model.contains_key(&b"a0"[..]));
    snaps.push((db.snapshot().unwrap(), model.clone()));
    // Put, delete and put again, then a range that ends at a put key.
    let mut b = WriteBatch::new();
    b.put("c1", "1").delete("c1").put("c1", "2").put("c2", "2").put("c3", "3").delete_range("c2", "c3");
    commit(&db, &mut model, &mut seq, &b);
    assert_eq!(model.get(&b"c1"[..]).map(Vec::as_slice), Some(&b"2"[..]));
    assert!(!model.contains_key(&b"c2"[..]) && model.contains_key(&b"c3"[..]));
    // Overlapping ranges after the puts.
    let mut b = WriteBatch::new();
    for i in 1..=9 {
        b.put(name("d", i), "d");
    }
    b.delete_range("d2", "d5").delete_range("d4", "d7").put("e1", "e");
    commit(&db, &mut model, &mut seq, &b);
    snaps.push((db.snapshot().unwrap(), model.clone()));
    // A range over every key the batch puts, and over older keys.
    let mut b = WriteBatch::new();
    b.put("f1", "f").put("f2", "f").put("a9", "newer").delete_range("a8", "g");
    commit(&db, &mut model, &mut seq, &b);
    assert!(model.keys().all(|k| k.as_slice() < &b"a8"[..]));
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "put then range");
    for (snap, m) in &snaps {
        check(snap, m, &probes, &bounds, &format!("older snapshot at seq {}", snap.seq()));
    }
    drop(snaps);
    reopen_and_check(db, disk, &opts, &model, seq, &probes, &bounds, "put then range");
}

pub fn order_within_batch_range_then_put<E: RangeDeleteEngine>() {
    let disk = SimVfs::new(3);
    let opts = options(Durability::Sync);
    let db: E = open(&disk.as_vfs(), &opts).unwrap();
    let name = |p: &str, i: u32| format!("{p}{i}").into_bytes();
    let mut probes: Vec<Vec<u8>> = Vec::new();
    for p in ["e", "f", "g", "h"] {
        probes.extend((0..10).map(|i| name(p, i)));
    }
    let bounds = probes.clone();
    let mut model = Model::new();
    let mut seq = 0;
    let mut b = WriteBatch::new();
    for i in 0..=9 {
        b.put(name("e", i), "old").put(name("g", i), "old");
    }
    commit(&db, &mut model, &mut seq, &b);
    let mut snaps = vec![(db.snapshot().unwrap(), model.clone())];

    // A key put after the range in the same batch survives it: same
    // sequence, the point wins (SEM-3).
    let mut b = WriteBatch::new();
    b.delete_range("e1", "e9").put("e5", "new");
    commit(&db, &mut model, &mut seq, &b);
    assert_eq!(model.get(&b"e5"[..]).map(Vec::as_slice), Some(&b"new"[..]));
    assert!(!model.contains_key(&b"e4"[..]) && model.contains_key(&b"e9"[..]) && model.contains_key(&b"e0"[..]));
    snaps.push((db.snapshot().unwrap(), model.clone()));
    // Point operations after a range interleave with a second range.
    let mut b = WriteBatch::new();
    b.delete_range("f0", "f9").put("f3", "x").delete("f3").put("f4", "first").delete_range("f4", "f5");
    b.put("f4", "again").put("f6", "kept");
    commit(&db, &mut model, &mut seq, &b);
    assert_eq!(model.get(&b"f4"[..]).map(Vec::as_slice), Some(&b"again"[..]));
    assert!(!model.contains_key(&b"f3"[..]));
    // Overlapping ranges, then puts inside both and at the exclusive end.
    let mut b = WriteBatch::new();
    b.delete_range("g0", "g5").delete_range("g3", "g9").put("g4", "new").put("g9", "new");
    commit(&db, &mut model, &mut seq, &b);
    let g: Vec<_> = model.keys().filter(|k| k.starts_with(b"g")).cloned().collect();
    assert_eq!(g, [b"g4".to_vec(), b"g9".to_vec()]);
    snaps.push((db.snapshot().unwrap(), model.clone()));
    // A put of a new key after a range that covers it, alone in a range of
    // nothing else.
    let mut b = WriteBatch::new();
    b.delete_range("h", "i").put("h5", "new");
    commit(&db, &mut model, &mut seq, &b);
    // Deleting a range again over keys put after the previous one.
    commit(&db, &mut model, &mut seq, &range("e5", "e6"));
    assert!(!model.contains_key(&b"e5"[..]));
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "range then put");
    for (snap, m) in &snaps {
        check(snap, m, &probes, &bounds, &format!("older snapshot at seq {}", snap.seq()));
    }
    drop(snaps);
    reopen_and_check(db, disk, &opts, &model, seq, &probes, &bounds, "range then put");
}

pub fn scan_across_tombstones_in_all_sources<E: RangeDeleteEngine>() {
    const N: u64 = 3000;
    let disk = SimVfs::new(4);
    let vfs = disk.as_vfs();
    let opts = options(Durability::Sync);
    let mut db: E = open(&vfs, &opts).unwrap();
    let mut model = Model::new();
    let mut seq = 0;
    let mut probes: Vec<Vec<u8>> = (0..N + 10).map(key).collect();
    probes.extend([b"".to_vec(), b"k".to_vec(), b"z".to_vec()]);
    let mut bounds: Vec<Vec<u8>> = vec![b"".to_vec(), key(0), key(N - 1), key(N), b"z".to_vec()];

    // Generation 0: every key, about 660 KB. With the small test buffers
    // that is many flushes and compactions: it ends up in the deepest levels.
    for chunk in 0..N / 50 {
        let mut b = WriteBatch::new();
        for i in chunk * 50..(chunk + 1) * 50 {
            b.put(key(i), val(i, 0));
        }
        commit(&db, &mut model, &mut seq, &b);
    }
    // Generation 1: overwrites of every third key, interleaved with ranges
    // (overlapping, adjacent, one key, wide, over the tail) that delete some
    // of them again.
    let g1: [(u64, u64); 6] = [(100, 200), (150, 250), (250, 260), (1000, 1001), (2000, 2500), (2990, N + 10)];
    for (n, &(a, z)) in g1.iter().enumerate() {
        let n = n as u64;
        let mut b = WriteBatch::new();
        for i in (n * 500..(n + 1) * 500).step_by(3) {
            b.put(key(i), val(i, 1));
        }
        commit(&db, &mut model, &mut seq, &b);
        commit(&db, &mut model, &mut seq, &range(key(a), key(z)));
        bounds.extend([key(a), key(z)]);
    }
    commit(&db, &mut model, &mut seq, &range("", key(10)));
    bounds.push(key(10));
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "generation 1");
    db.close().unwrap();
    db = open(&vfs, &opts).unwrap();
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "generation 1 after reopen");

    // Generation 2: keys put again inside older ranges (newer than the
    // ranges: visible), a range over generation-1 overwrites, and enough
    // volume after it to write part of this generation to disk.
    let mut b = WriteBatch::new();
    for i in (120..240).step_by(2) {
        b.put(key(i), val(i, 2));
    }
    commit(&db, &mut model, &mut seq, &b);
    commit(&db, &mut model, &mut seq, &range(key(1500), key(1800)));
    for chunk in 0..6 {
        let mut b = WriteBatch::new();
        for i in 2600 + chunk * 50..2650 + chunk * 50 {
            b.put(key(i), val(i, 2));
        }
        commit(&db, &mut model, &mut seq, &b);
    }
    commit(&db, &mut model, &mut seq, &range(key(2700), key(2750)));
    bounds.extend([key(120), key(240), key(1500), key(1800), key(2700), key(2750)]);
    let old = (db.snapshot().unwrap(), model.clone());

    // Generation 3: small, so it stays in the active write buffer: ranges
    // over keys of every generation, and puts inside them.
    let mut b = WriteBatch::new();
    b.delete_range(key(130), key(140)).put(key(135), val(135, 3));
    b.delete_range(key(1490), key(1510)).put(key(1000), val(1000, 3)).delete_range(key(2595), key(2605));
    commit(&db, &mut model, &mut seq, &b);
    commit(&db, &mut model, &mut seq, &range(key(2999), key(3001)));
    bounds.extend([key(130), key(135), key(140), key(1490), key(1510), key(2595), key(2605), key(2999)]);
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "generation 3");
    check(&old.0, &old.1, &probes, &bounds, "snapshot before generation 3");
    drop(old);
    db.close().unwrap();
    let db: E = open(&vfs, &opts).unwrap();
    assert_eq!(db.snapshot().unwrap().seq(), seq);
    check(&db.snapshot().unwrap(), &model, &probes, &bounds, "generation 3 after reopen");
}

pub fn snapshot_isolation_with_ranges_across_flush_and_compaction<E: RangeDeleteEngine>() {
    for_each_seed("snapshot_isolation_with_ranges", seeds(4), |seed| {
        let vfs = SimVfs::new(seed).as_vfs();
        let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
        let mut model = Model::new();
        let mut w = Workload::with_config(seed, WorkloadCfg { range_delete_per_mille: 60 });
        w.key_space = 600;
        let mut rng = Rng::new(seed);
        let mut live: Vec<(E::Snapshot, Model)> = Vec::new();
        // Snapshots right before the wide ranges, kept to the end.
        let mut pinned: Vec<(E::Snapshot, Model)> = Vec::new();
        let mut seq = 0;
        for i in 0..900 {
            if i == 300 || i == 600 {
                pinned.push((db.snapshot().unwrap(), model.clone()));
            }
            let b = match i {
                // Everything the workload writes (every key is below 0xff).
                300 => range("", [0xff]),
                600 => range("key-00000100", "key-00000500"),
                _ => w.batch(&model),
            };
            commit(&db, &mut model, &mut seq, &b);
            if i % 25 == 0 {
                live.push((db.snapshot().unwrap(), model.clone()));
                if live.len() > 8 {
                    let idx = rng.below(live.len() as u64) as usize;
                    live.remove(idx);
                }
            }
            if i % 10 == 0 {
                let all: Vec<&(E::Snapshot, Model)> = live.iter().chain(&pinned).collect();
                let (snap, m) = all[rng.below(all.len() as u64) as usize];
                let got = read_all(snap).unwrap();
                assert!(got == *m, "seed {seed}: snapshot at seq {} changed: {}", snap.seq(), diff(m, &got));
            }
        }
        for (snap, m) in live.iter().chain(&pinned) {
            assert_snapshot_matches(snap, m, &mut rng, &format!("seed {seed}: snapshot at seq {}", snap.seq()));
        }
        assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &format!("seed {seed}: latest"));
    });
}

pub fn randomized_model_check_with_ranges<E: RangeDeleteEngine>() {
    randomized_model_check_with_config::<E>(&SuiteConfig::with_range_deletes(40));
}

pub fn range_survives_reopen<E: RangeDeleteEngine>() {
    for durability in [Durability::Sync, Durability::Buffered] {
        let ctx = format!("{durability:?}");
        let disk = SimVfs::new(5);
        let opts = options(durability);
        let db: E = open(&disk.as_vfs(), &opts).unwrap();
        let mut probes: Vec<Vec<u8>> = (0..610).map(key).collect();
        probes.push(b"z".to_vec());
        let mut bounds = vec![key(0), key(600)];
        let mut model = Model::new();
        let mut seq = 0;
        // About 120 KB: part of it is on disk when the ranges arrive.
        for chunk in 0..15 {
            let mut b = WriteBatch::new();
            for i in chunk * 40..(chunk + 1) * 40 {
                b.put(key(i), val(i, 0));
            }
            commit(&db, &mut model, &mut seq, &b);
        }
        commit(&db, &mut model, &mut seq, &range(key(100), key(300)));
        let mut b = WriteBatch::new();
        b.put(key(150), val(150, 1));
        commit(&db, &mut model, &mut seq, &b);
        let mut b = range(key(500), key(600));
        b.put(key(550), val(550, 1));
        commit(&db, &mut model, &mut seq, &b);
        bounds.extend([key(100), key(150), key(300), key(500), key(550)]);
        let (db, disk) = reopen_and_check(db, disk, &opts, &model, seq, &probes, &bounds, &ctx);

        // Ranges committed after recovery, over keys from before it.
        commit(&db, &mut model, &mut seq, &range(key(0), key(50)));
        commit(&db, &mut model, &mut seq, &range(key(140), key(160)));
        let mut b = range(key(300), key(400));
        b.put(key(350), val(350, 2));
        commit(&db, &mut model, &mut seq, &b);
        bounds.extend([key(50), key(140), key(160), key(350), key(400)]);
        let (db, disk) = reopen_and_check(db, disk, &opts, &model, seq, &probes, &bounds, &ctx);

        // A key put again inside a deleted range is visible; its neighbours
        // stay deleted.
        let mut b = WriteBatch::new();
        b.put(key(120), val(120, 3));
        commit(&db, &mut model, &mut seq, &b);
        assert!(model.contains_key(&key(120)) && !model.contains_key(&key(121)));
        reopen_and_check(db, disk, &opts, &model, seq, &probes, &bounds, &ctx);
    }
}

/// Filling the database and emptying it with range deletions, cycle after
/// cycle: after each refill the files must stay within
/// `4 * first fill + 1 MiB` on disk (the bound of
/// `functional::space_is_reclaimed`). Every cycle writes keys no earlier
/// cycle wrote (all inside `["c", "d")`), so no value is ever overwritten or
/// deleted by a point operation: only the range deletions make old values
/// garbage, and an engine that reclaims overwritten versions but keeps the
/// points that only a range covers grows like one that reclaims nothing.
/// A range deletion takes a few bytes, so such an engine keeps at least
/// every value byte written so far (random, hence incompressible). The test
/// runs at least six cycles (those of `functional::space_is_reclaimed`), and
/// then until those bytes alone have exceeded the bound in two cycles (64
/// cycles at most), so such an engine cannot pass.
pub fn space_is_reclaimed_after_range_deletes<E: RangeDeleteEngine>() {
    let vfs: Arc<dyn Vfs> = SimVfs::new(3).as_vfs();
    let dir = db_dir();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    let mut rng = Rng::new(3);
    let (mut peak_first, mut bound) = (0, u64::MAX);
    // Value bytes committed so far, and the cycles after which they exceed
    // `bound`.
    let (mut written, mut decisive) = (0u64, 0);
    let mut cycle = 0;
    while cycle < MIN_RECLAIM_CYCLES || decisive < 2 {
        assert!(
            cycle < MAX_RECLAIM_CYCLES,
            "the first fill used {peak_first} bytes: {MAX_RECLAIM_CYCLES} cycles cannot tell reclamation from growth"
        );
        // Fresh keys in every cycle (see above).
        let ckey = |i: u64| format!("c{cycle:02}-{i:06}").into_bytes();
        for chunk in 0..20 {
            let mut b = WriteBatch::new();
            for i in 0..100 {
                let len = rng.range(50, 400) as usize;
                b.put(ckey(chunk * 100 + i), rng.bytes(len));
                written += len as u64;
            }
            db.commit(&b).unwrap();
        }
        let size = dir_size(&*vfs, &dir).unwrap();
        if cycle == 0 {
            peak_first = size;
            bound = 4 * peak_first + (1 << 20);
        } else {
            assert!(
                size <= bound,
                "cycle {cycle}: {size} bytes on disk after refill, first fill used {peak_first} (bound {bound}, \
                 {written} value bytes written so far)"
            );
        }
        if written > bound {
            decisive += 1;
        }
        if cycle % 2 == 0 {
            // One range over everything.
            db.commit(&range("c", "d")).unwrap();
        } else {
            // One range per chunk.
            for chunk in 0..20 {
                db.commit(&range(ckey(chunk * 100), ckey((chunk + 1) * 100))).unwrap();
            }
        }
        assert!(read_all(&db.snapshot().unwrap()).unwrap().is_empty(), "cycle {cycle}: keys left after the ranges");
        cycle += 1;
    }
}

/// Fewest cycles of [`space_is_reclaimed_after_range_deletes`] (those of
/// `functional::space_is_reclaimed`).
const MIN_RECLAIM_CYCLES: u64 = 6;
/// Most cycles of [`space_is_reclaimed_after_range_deletes`]: an engine whose
/// first fill is so large that this many cycles cannot exceed the bound
/// fails the test rather than pass it untested.
const MAX_RECLAIM_CYCLES: u64 = 64;

/// Generates the range deletion suite (module `shared_range`) for an engine
/// type that implements [`RangeDeleteEngine`] (natively, or through
/// [`Emulated`](crate::emulated::Emulated)):
///
/// ```ignore
/// kerno_testkit::range_delete_test_suite!(MyEngine);
/// ```
///
/// See the [module documentation](crate::harness::range) for what each test
/// checks.
#[macro_export]
macro_rules! range_delete_test_suite {
    ($engine:ty) => {
        mod shared_range {
            use super::*;
            use $crate::harness::range as r;
            #[test]
            fn delete_range_bounds_and_degenerate() {
                r::delete_range_bounds_and_degenerate::<$engine>();
            }
            #[test]
            fn order_within_batch_put_then_range() {
                r::order_within_batch_put_then_range::<$engine>();
            }
            #[test]
            fn order_within_batch_range_then_put() {
                r::order_within_batch_range_then_put::<$engine>();
            }
            #[test]
            fn scan_across_tombstones_in_all_sources() {
                r::scan_across_tombstones_in_all_sources::<$engine>();
            }
            #[test]
            fn snapshot_isolation_with_ranges_across_flush_and_compaction() {
                r::snapshot_isolation_with_ranges_across_flush_and_compaction::<$engine>();
            }
            #[test]
            fn randomized_model_check_with_ranges() {
                r::randomized_model_check_with_ranges::<$engine>();
            }
            #[test]
            fn range_survives_reopen() {
                r::range_survives_reopen::<$engine>();
            }
            #[test]
            fn space_is_reclaimed_after_range_deletes() {
                r::space_is_reclaimed_after_range_deletes::<$engine>();
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use kerno_core::contract::Engine;
    use kerno_core::{Op, Result};

    use super::*;
    use crate::reference::LogEngine;
    use crate::vfs::{File, OpenMode};

    fn keys(model: &Model) -> Vec<String> {
        model.keys().map(|k| String::from_utf8(k.clone()).unwrap()).collect()
    }

    #[test]
    fn model_applies_ranges_in_batch_order() {
        let mut model = Model::new();
        let mut b = WriteBatch::new();
        for k in ["a", "b", "c", "d", "e"] {
            b.put(k, "0");
        }
        apply(&mut model, &b);
        let mut b = WriteBatch::new();
        b.put("bb", "1").delete_range("b", "d").put("c", "2").delete_range("e", "e").delete_range("", "a\0");
        apply(&mut model, &b);
        assert_eq!(keys(&model), ["c", "d", "e"]);
        assert_eq!(model[&b"c"[..]], b"2");
        // An inverted range (rejected by `validate`) leaves the model alone.
        let mut b = WriteBatch::new();
        b.delete_range("z", "a");
        apply(&mut model, &b);
        assert_eq!(keys(&model), ["c", "d", "e"]);
    }

    #[test]
    fn workload_range_deletes() {
        let cfg = WorkloadCfg { range_delete_per_mille: 200 };
        let mut w = Workload::with_config(9, cfg);
        let mut model = Model::new();
        let (mut ranges, mut empty, mut long, mut ops) = (0, 0, 0, 0);
        for _ in 0..2000 {
            let b = w.batch(&model);
            b.validate().unwrap();
            apply(&mut model, &b);
            for op in b.ops() {
                ops += 1;
                if let Op::DeleteRange { start, end } = op {
                    ranges += 1;
                    empty += usize::from(start == end);
                    long += usize::from(start.starts_with(b"long-"));
                    let family = if start.starts_with(b"key-") { &b"key-"[..] } else { &b"long-"[..] };
                    assert!(start.starts_with(family) && end.starts_with(family), "a range leaves its key family");
                }
            }
        }
        // About a fifth of the operations, with every shape represented.
        assert!(ranges * 6 > ops && ranges * 4 < ops, "{ranges} ranges in {ops} operations");
        assert!(empty > 0 && long > 0, "{empty} empty and {long} long-key ranges");
        // Without ranges, no operation is a range and the random stream is
        // the one `Workload::new` draws.
        let mut a = Workload::with_config(9, WorkloadCfg::default());
        let mut b = Workload::new(9);
        for _ in 0..200 {
            let (x, y) = (a.batch(&Model::new()), b.batch(&Model::new()));
            assert!(!x.has_range_deletes());
            assert_eq!(x, y);
        }
    }

    /// `LogEngine` plus a journal in the database directory that keeps the
    /// keys and values of every committed batch forever: an engine that
    /// reclaims no space at all.
    struct Leaky {
        db: LogEngine,
        journal: Arc<dyn File>,
        end: parking_lot::Mutex<u64>,
    }

    impl Engine for Leaky {
        type Snapshot = <LogEngine as Engine>::Snapshot;

        fn name() -> &'static str {
            "leaky"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            let db = LogEngine::open(vfs.clone(), dir, opts)?;
            let journal = vfs.open(&dir.join("JOURNAL"), OpenMode::CreateOrOpen)?;
            let end = parking_lot::Mutex::new(journal.len()?);
            Ok(Leaky { db, journal, end })
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            let seq = self.db.commit(batch)?;
            let mut buf = Vec::new();
            for op in batch.ops() {
                buf.extend_from_slice(op.key());
                if let Op::Put { value, .. } = op {
                    buf.extend_from_slice(value);
                }
            }
            let mut end = self.end.lock();
            self.journal.write_at(&buf, *end)?;
            *end += buf.len() as u64;
            Ok(seq)
        }

        fn snapshot(&self) -> Result<Self::Snapshot> {
            self.db.snapshot()
        }

        fn close(self) -> Result<()> {
            self.db.close()
        }
    }

    impl RangeDeleteEngine for Leaky {}

    #[test]
    fn space_check_fails_for_an_engine_that_reclaims_nothing() {
        let err = std::panic::catch_unwind(space_is_reclaimed_after_range_deletes::<Leaky>)
            .expect_err("an engine that keeps every byte passed the space check");
        let msg = err.downcast_ref::<String>().map_or("", String::as_str);
        assert!(msg.contains("bytes on disk after refill"), "unexpected failure: {msg}");
    }

    /// Live keys and the length of their values, and the values that only a
    /// range deletion covers (key and length).
    #[derive(Default)]
    struct Graves {
        live: BTreeMap<Vec<u8>, u64>,
        graves: BTreeMap<Vec<u8>, u64>,
    }

    /// `LogEngine` plus a file in the database directory that keeps the
    /// value bytes of every point a range deletion covered until that key is
    /// written again (put or point deletion). It models an LSM whose
    /// compaction drops the versions a newer point of the same key shadows,
    /// but never a point that only a range tombstone covers. The graves live
    /// in memory: enough for a test that never reopens.
    struct Graveyard {
        db: LogEngine,
        file: Arc<dyn File>,
        state: parking_lot::Mutex<Graves>,
    }

    impl Engine for Graveyard {
        type Snapshot = <LogEngine as Engine>::Snapshot;

        fn name() -> &'static str {
            "graveyard"
        }

        fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
            let db = LogEngine::open(vfs.clone(), dir, opts)?;
            let file = vfs.open(&dir.join("GRAVES"), OpenMode::CreateOrOpen)?;
            let live = read_all(&db.snapshot()?)?.into_iter().map(|(k, v)| (k, v.len() as u64)).collect();
            Ok(Graveyard { db, file, state: parking_lot::Mutex::new(Graves { live, graves: BTreeMap::new() }) })
        }

        fn commit(&self, batch: &WriteBatch) -> Result<u64> {
            let mut st = self.state.lock();
            let seq = self.db.commit(batch)?;
            let Graves { live, graves } = &mut *st;
            for op in batch.ops() {
                match op {
                    Op::Put { key, value } => {
                        graves.remove(key);
                        live.insert(key.clone(), value.len() as u64);
                    }
                    Op::Delete { key } => {
                        graves.remove(key);
                        live.remove(key);
                    }
                    Op::DeleteRange { start, end } if start < end => {
                        let covered: Vec<Vec<u8>> =
                            live.range(start.clone()..end.clone()).map(|(k, _)| k.clone()).collect();
                        for k in covered {
                            let len = live.remove(&k).expect("listed above");
                            graves.insert(k, len);
                        }
                    }
                    Op::DeleteRange { .. } => {}
                }
            }
            self.file.set_len(graves.values().sum())?;
            Ok(seq)
        }

        fn snapshot(&self) -> Result<Self::Snapshot> {
            self.db.snapshot()
        }

        fn close(self) -> Result<()> {
            self.db.close()
        }
    }

    impl RangeDeleteEngine for Graveyard {}

    /// The space check must not pass for an engine that reclaims overwritten
    /// and point-deleted versions but keeps the points covered only by a
    /// range: with the same keys in every cycle, each refill shadowed the
    /// range-deleted values and such an engine passed.
    #[test]
    fn space_check_fails_for_an_engine_that_keeps_range_deleted_points() {
        // Sanity: the graves are what makes it grow. Overwritten and
        // point-deleted values leave nothing behind; range-deleted ones do.
        let vfs: Arc<dyn Vfs> = SimVfs::new(1).as_vfs();
        let db: Graveyard = open(&vfs, &Options::small_for_tests()).unwrap();
        let mut b = WriteBatch::new();
        b.put("a", "1234").put("b", "12").put("c", "123");
        db.commit(&b).unwrap();
        let mut b = WriteBatch::new();
        b.put("a", "x").delete("b");
        db.commit(&b).unwrap();
        assert_eq!(db.file.len().unwrap(), 0);
        db.commit(&range("a", "c")).unwrap();
        assert_eq!(db.file.len().unwrap(), 1);
        let mut b = range("b", "d");
        b.put("c", "new");
        db.commit(&b).unwrap();
        assert_eq!(db.file.len().unwrap(), 1, "a point after the range in its batch shadows it");
        let mut b = WriteBatch::new();
        b.put("a", "back");
        db.commit(&b).unwrap();
        assert_eq!(db.file.len().unwrap(), 0);
        drop(db);

        let err = std::panic::catch_unwind(space_is_reclaimed_after_range_deletes::<Graveyard>)
            .expect_err("an engine that keeps the points covered by ranges passed the space check");
        let msg = err.downcast_ref::<String>().map_or("", String::as_str);
        assert!(msg.contains("bytes on disk after refill"), "unexpected failure: {msg}");
    }
}
