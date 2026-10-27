//! Functional behaviour every candidate must have.

use std::ops::Bound;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::{Model, Workload, apply, assert_snapshot_matches, db_dir, for_each_seed, open, read_all, seeds};
use crate::engine::{Durability, Engine, MAX_KEY_LEN, MAX_VALUE_LEN, Options, Snapshot, WriteBatch};
use crate::error::Error;
use crate::rng::Rng;
use crate::vfs::{SimVfs, StdVfs, Vfs, dir_size};

fn sim() -> Arc<dyn Vfs> {
    SimVfs::new(7).as_vfs()
}

pub fn empty_database<E: Engine>() {
    let vfs = sim();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    let s = db.snapshot().unwrap();
    assert_eq!(s.seq(), 0);
    assert_eq!(s.get(b"anything").unwrap(), None);
    assert!(s.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap().is_empty());
    drop(s);
    db.close().unwrap();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    assert_eq!(db.snapshot().unwrap().seq(), 0);
}

pub fn put_get_overwrite_delete<E: Engine>() {
    let vfs = sim();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    let mut b = WriteBatch::new();
    b.put("a", "1").put("b", "2");
    db.commit(&b).unwrap();
    let s = db.snapshot().unwrap();
    assert_eq!(s.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
    assert_eq!(s.get(b"b").unwrap().as_deref(), Some(&b"2"[..]));
    assert_eq!(s.get(b"c").unwrap(), None);

    let mut b = WriteBatch::new();
    b.put("a", "11");
    db.commit(&b).unwrap();
    let mut b = WriteBatch::new();
    b.delete("b").delete("does-not-exist");
    db.commit(&b).unwrap();

    let s2 = db.snapshot().unwrap();
    assert_eq!(s2.get(b"a").unwrap().as_deref(), Some(&b"11"[..]));
    assert_eq!(s2.get(b"b").unwrap(), None);
    // The older snapshot is unaffected.
    assert_eq!(s.get(b"a").unwrap().as_deref(), Some(&b"1"[..]));
    assert_eq!(s.get(b"b").unwrap().as_deref(), Some(&b"2"[..]));
}

pub fn batch_semantics<E: Engine>() {
    let vfs = sim();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    let mut b = WriteBatch::new();
    b.put("k", "1").put("k", "2").delete("k").put("k", "3").put("gone", "x").delete("gone");
    let seq = db.commit(&b).unwrap();
    assert_eq!(seq, 1);
    let s = db.snapshot().unwrap();
    assert_eq!(s.get(b"k").unwrap().as_deref(), Some(&b"3"[..]));
    assert_eq!(s.get(b"gone").unwrap(), None);
    assert_eq!(db.commit(&WriteBatch::new()).unwrap(), 1, "empty batch is a no-op");
    let mut b = WriteBatch::new();
    b.put("", "empty key").put("empty value", "");
    assert_eq!(db.commit(&b).unwrap(), 2);
    let s = db.snapshot().unwrap();
    assert_eq!(s.get(b"").unwrap().as_deref(), Some(&b"empty key"[..]));
    assert_eq!(s.get(b"empty value").unwrap().as_deref(), Some(&b""[..]));
}

pub fn scan_bounds_and_pagination<E: Engine>() {
    let vfs = sim();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    let mut model = Model::new();
    let mut b = WriteBatch::new();
    for i in 0..1500u32 {
        let k = format!("k{:05}", i * 2).into_bytes();
        let v = i.to_le_bytes().to_vec();
        b.put(k, v);
        if b.len() == 100 {
            apply(&mut model, &b);
            db.commit(&b).unwrap();
            b = WriteBatch::new();
        }
    }
    let s = db.snapshot().unwrap();
    assert!(read_all(&s).unwrap() == model);
    let k = |i: u32| format!("k{:05}", i).into_bytes();
    let keys = |v: Vec<(Vec<u8>, Vec<u8>)>| v.into_iter().map(|(k, _)| k).collect::<Vec<_>>();
    let r = s.scan(Bound::Included(&k(10)), Bound::Included(&k(16)), 100).unwrap();
    assert_eq!(keys(r), vec![k(10), k(12), k(14), k(16)]);
    let r = s.scan(Bound::Excluded(&k(10)), Bound::Excluded(&k(16)), 100).unwrap();
    assert_eq!(keys(r), vec![k(12), k(14)]);
    let r = s.scan(Bound::Included(&k(11)), Bound::Excluded(&k(15)), 100).unwrap();
    assert_eq!(keys(r), vec![k(12), k(14)]);
    let r = s.scan(Bound::Unbounded, Bound::Excluded(&k(4)), 100).unwrap();
    assert_eq!(keys(r), vec![k(0), k(2)]);
    let r = s.scan(Bound::Excluded(&k(2994)), Bound::Unbounded, 100).unwrap();
    assert_eq!(keys(r), vec![k(2996), k(2998)]);
    let r = s.scan(Bound::Included(&k(100)), Bound::Unbounded, 3).unwrap();
    assert_eq!(keys(r), vec![k(100), k(102), k(104)]);
    assert!(s.scan(Bound::Included(&k(20)), Bound::Excluded(&k(20)), 10).unwrap().is_empty());
    assert!(s.scan(Bound::Included(&k(30)), Bound::Included(&k(20)), 10).unwrap().is_empty());
    assert!(s.scan(Bound::Unbounded, Bound::Unbounded, 0).unwrap().is_empty());
}

pub fn size_limits<E: Engine>() {
    let vfs = sim();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    let max_key = vec![b'k'; MAX_KEY_LEN];
    let big_value: Vec<u8> = (0..MAX_VALUE_LEN).map(|i| (i % 251) as u8).collect();
    let mut b = WriteBatch::new();
    b.put(max_key.clone(), big_value.clone());
    db.commit(&b).unwrap();

    let mut b = WriteBatch::new();
    b.put(vec![b'k'; MAX_KEY_LEN + 1], "x");
    assert!(matches!(db.commit(&b), Err(Error::InvalidArgument(_))));
    let mut b = WriteBatch::new();
    b.put("ok", "x").put("v", vec![0u8; MAX_VALUE_LEN + 1]);
    assert!(matches!(db.commit(&b), Err(Error::InvalidArgument(_))));
    let s = db.snapshot().unwrap();
    assert_eq!(s.seq(), 1, "rejected batches must not commit");
    assert_eq!(s.get(b"ok").unwrap(), None, "rejected batches must not be partially applied");
    assert!(s.get(&max_key).unwrap().unwrap() == big_value);
    drop(s);
    db.close().unwrap();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    assert!(db.snapshot().unwrap().get(&max_key).unwrap().unwrap() == big_value);
}

fn random_history<E: Engine>(db: &E, model: &mut Model, w: &mut Workload, commits: usize) {
    for _ in 0..commits {
        let b = w.batch(model);
        db.commit(&b).unwrap();
        apply(model, &b);
    }
}

pub fn reopen_after_clean_close<E: Engine>() {
    let vfs = sim();
    let mut model = Model::new();
    let mut w = Workload::new(11);
    let mut rng = Rng::new(11);
    for round in 0..4 {
        let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
        assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &format!("after reopen #{round}"));
        random_history(&db, &mut model, &mut w, 150);
        db.close().unwrap();
    }
}

pub fn reopen_after_drop_without_close<E: Engine>() {
    let vfs = sim();
    let mut model = Model::new();
    let mut w = Workload::new(12);
    let mut rng = Rng::new(12);
    for round in 0..4 {
        let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
        assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &format!("after reopen #{round}"));
        random_history(&db, &mut model, &mut w, 150);
        drop(db);
    }
}

pub fn commit_sequence<E: Engine>() {
    let vfs = sim();
    let mut expected = 0;
    for _ in 0..3 {
        let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
        assert_eq!(db.snapshot().unwrap().seq(), expected);
        for i in 0..50 {
            let mut b = WriteBatch::new();
            b.put(format!("k{i}"), "v");
            expected += 1;
            assert_eq!(db.commit(&b).unwrap(), expected);
            assert_eq!(db.snapshot().unwrap().seq(), expected);
        }
        db.close().unwrap();
    }
}

pub fn exclusive_open<E: Engine>() {
    let vfs = sim();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    assert!(matches!(open::<E>(&vfs, &Options::small_for_tests()), Err(Error::Locked)));
    db.close().unwrap();
    let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
    drop(db);
    let _db: E = open(&vfs, &Options::small_for_tests()).unwrap();
}

/// Many snapshots at different sequence numbers stay valid while the
/// writer churns (page reuse, flushes, compactions).
pub fn snapshot_isolation_under_churn<E: Engine>() {
    for_each_seed("snapshot_isolation_under_churn", seeds(4), |seed| {
        let vfs = sim();
        let db: E = open(&vfs, &Options::small_for_tests()).unwrap();
        let mut model = Model::new();
        let mut w = Workload::new(seed);
        w.key_space = 600;
        let mut rng = Rng::new(seed);
        let mut live: Vec<(E::Snapshot, Model)> = Vec::new();
        for i in 0..600 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
            if i % 25 == 0 {
                live.push((db.snapshot().unwrap(), model.clone()));
                if live.len() > 8 {
                    let idx = rng.below(live.len() as u64) as usize;
                    live.remove(idx);
                }
            }
            if i % 10 == 0 && !live.is_empty() {
                let idx = rng.below(live.len() as u64) as usize;
                let (snap, m) = &live[idx];
                let got = read_all(snap).unwrap();
                assert!(got == *m, "snapshot at seq {} changed: {}", snap.seq(), super::diff(m, &got));
            }
        }
        for (snap, m) in &live {
            assert_snapshot_matches(snap, m, &mut rng, &format!("snapshot at seq {}", snap.seq()));
        }
    });
}

/// Readers must never see half of a batch: the writer moves "money" between
/// accounts and readers check that the total is constant.
pub fn concurrent_readers<E: Engine>() {
    const ACCOUNTS: u64 = 16;
    const TOTAL: u64 = 1_000_000;
    let vfs = sim();
    let db: Arc<E> = Arc::new(open(&vfs, &Options::small_for_tests()).unwrap());
    let acct = |i: u64| format!("acct-{i:03}").into_bytes();
    let mut b = WriteBatch::new();
    for i in 0..ACCOUNTS {
        let v = if i == 0 { TOTAL } else { 0 };
        b.put(acct(i), v.to_le_bytes().to_vec());
    }
    db.commit(&b).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let checks = Arc::new(AtomicU64::new(0));
    let mut readers = Vec::new();
    for r in 0..4 {
        let db = db.clone();
        let stop = stop.clone();
        let checks = checks.clone();
        readers.push(std::thread::spawn(move || {
            let mut last_seq = 0;
            while !stop.load(Ordering::Relaxed) {
                let s = db.snapshot().unwrap();
                assert!(s.seq() >= last_seq, "reader {r}: snapshot sequence went backwards");
                last_seq = s.seq();
                let mut sum = 0;
                for i in 0..ACCOUNTS {
                    let v = s.get(&acct(i)).unwrap().expect("account exists");
                    sum += u64::from_le_bytes(v.try_into().unwrap());
                }
                assert_eq!(sum, TOTAL, "reader {r}: point reads saw a partial batch at seq {}", s.seq());
                let rows = s.scan(Bound::Included(b"acct-"), Bound::Excluded(b"acct."), 1000).unwrap();
                let sum2: u64 = rows.iter().map(|(_, v)| u64::from_le_bytes(v[..].try_into().unwrap())).sum();
                assert_eq!(sum2, TOTAL, "reader {r}: scan saw a partial batch at seq {}", s.seq());
                checks.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    let mut rng = Rng::new(99);
    let mut balances = vec![0u64; ACCOUNTS as usize];
    balances[0] = TOTAL;
    let mut w = Workload::new(5);
    let mut noise_model = Model::new();
    for _ in 0..1500 {
        let from = rng.below(ACCOUNTS) as usize;
        let to = rng.below(ACCOUNTS) as usize;
        let amount = rng.below(balances[from] + 1);
        balances[from] -= amount;
        balances[to] += amount;
        let mut b = w.batch(&noise_model);
        apply(&mut noise_model, &b);
        b.put(acct(from as u64), balances[from].to_le_bytes().to_vec());
        b.put(acct(to as u64), balances[to].to_le_bytes().to_vec());
        db.commit(&b).unwrap();
    }
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        r.join().expect("reader thread panicked");
    }
    assert!(checks.load(Ordering::Relaxed) > 0);
}

/// Long random history checked against an in-memory model, with clean and
/// unclean reopens in between.
pub fn randomized_model_check<E: Engine>() {
    for_each_seed("randomized_model_check", seeds(6), |seed| {
        let vfs = sim();
        let mut rng = Rng::new(seed);
        let mut w = Workload::new(seed);
        let mut model = Model::new();
        let mut opts = Options::small_for_tests();
        if seed % 2 == 1 {
            opts.durability = Durability::Buffered;
        }
        let mut db: E = open(&vfs, &opts).unwrap();
        for i in 0..1200 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
            if i % 50 == 0 {
                let s = db.snapshot().unwrap();
                let got = read_all(&s).unwrap();
                assert!(got == model, "after commit {i}: {}", super::diff(&model, &got));
            }
            if i % 300 == 299 {
                if rng.chance(1, 2) {
                    db.close().unwrap();
                } else {
                    drop(db);
                }
                db = open(&vfs, &opts).unwrap();
                assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, &format!("reopen after {i}"));
            }
        }
        assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, "final");
    });
}

/// Repeatedly filling and emptying the database must not grow the files
/// without bound.
pub fn space_is_reclaimed<E: Engine>() {
    let vfs = sim();
    let dir = db_dir();
    let opts = Options::small_for_tests();
    let db: E = open(&vfs, &opts).unwrap();
    let mut rng = Rng::new(3);
    let mut peak_first = 0;
    for cycle in 0..6 {
        for chunk in 0..20 {
            let mut b = WriteBatch::new();
            for i in 0..100 {
                let len = rng.range(50, 400) as usize;
                b.put(format!("c{:06}", chunk * 100 + i), rng.bytes(len));
            }
            db.commit(&b).unwrap();
        }
        let size = dir_size(&*vfs, &dir).unwrap();
        if cycle == 0 {
            peak_first = size;
        } else {
            assert!(
                size <= 4 * peak_first + (1 << 20),
                "cycle {cycle}: {size} bytes on disk after refill, first fill used {peak_first}"
            );
        }
        for chunk in 0..20 {
            let mut b = WriteBatch::new();
            for i in 0..100 {
                b.delete(format!("c{:06}", chunk * 100 + i));
            }
            db.commit(&b).unwrap();
        }
    }
    assert!(read_all(&db.snapshot().unwrap()).unwrap().is_empty());
}

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("kerno-spike-{tag}-{}-{nanos}", std::process::id()))
}

/// Same engine code on the real file system: basic durability and the
/// cross-handle lock.
pub fn real_filesystem_smoke<E: Engine>() {
    let dir = temp_dir(E::name());
    let vfs = StdVfs::shared();
    let opts = Options::small_for_tests();
    let mut model = Model::new();
    let mut w = Workload::new(77);
    let mut rng = Rng::new(77);
    {
        let db = E::open(vfs.clone(), &dir, &opts).unwrap();
        assert!(matches!(E::open(vfs.clone(), &dir, &opts), Err(Error::Locked)));
        random_history(&db, &mut model, &mut w, 300);
        db.close().unwrap();
    }
    {
        let db = E::open(vfs.clone(), &dir, &opts).unwrap();
        assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, "real fs reopen");
        random_history(&db, &mut model, &mut w, 100);
    }
    let db = E::open(vfs.clone(), &dir, &opts).unwrap();
    assert_snapshot_matches(&db.snapshot().unwrap(), &model, &mut rng, "real fs reopen after drop");
    db.close().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}
