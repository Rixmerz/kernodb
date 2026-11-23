//! The write overlay of `WriteTxn`: an append-only buffer sorted once at
//! commit. Reads before and after the commit must match a model in which the
//! last write of a key wins.

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::Path;

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_storage::{Config, KernoDb};
use kerno_core::vfs::Vfs;
use kerno_testkit::vfs::SimVfs;

fn open() -> KernoDb {
    let disk = SimVfs::new(1);
    let cfg = Config::from(&Options::small_for_tests());
    KernoDb::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap()
}

fn key(i: usize) -> Vec<u8> {
    format!("k{i:07}").into_bytes()
}

#[test]
fn the_last_write_of_a_key_wins_before_and_after_commit() {
    let db = open();
    let mut t = db.begin_write().unwrap();
    t.put(b"a", b"1").unwrap();
    t.put(b"b", b"1").unwrap();
    t.put(b"a", b"2").unwrap();
    t.delete(b"b").unwrap();
    t.put(b"c", b"1").unwrap();
    t.delete(b"c").unwrap();
    t.put(b"c", b"3").unwrap();
    // Before the commit: gets and a scan see the last write of each key.
    assert_eq!(t.get(b"a").unwrap(), Some(b"2".to_vec()));
    assert_eq!(t.get(b"b").unwrap(), None);
    assert_eq!(t.get(b"c").unwrap(), Some(b"3".to_vec()));
    let rows = t.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap();
    assert_eq!(rows, vec![(b"a".to_vec(), b"2".to_vec()), (b"c".to_vec(), b"3".to_vec())]);
    // A write after a read still wins.
    t.put(b"a", b"4").unwrap();
    assert_eq!(t.get(b"a").unwrap(), Some(b"4".to_vec()));
    t.commit().unwrap();
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.get(b"a").unwrap(), Some(b"4".to_vec()));
    assert_eq!(snap.get(b"b").unwrap(), None);
    assert_eq!(snap.get(b"c").unwrap(), Some(b"3".to_vec()));
}

#[test]
fn a_range_delete_hides_earlier_writes_but_not_later_ones() {
    let db = open();
    let mut t = db.begin_write().unwrap();
    t.put(b"a", b"1").unwrap();
    t.put(b"b", b"1").unwrap();
    t.delete_range(b"a", b"c").unwrap();
    t.put(b"b", b"2").unwrap();
    assert_eq!(t.get(b"a").unwrap(), None);
    assert_eq!(t.get(b"b").unwrap(), Some(b"2".to_vec()));
    t.commit().unwrap();
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.get(b"a").unwrap(), None);
    assert_eq!(snap.get(b"b").unwrap(), Some(b"2".to_vec()));
}

#[test]
fn a_read_between_writes_does_not_change_the_commit() {
    // More writes than the buffer scan limit, with reads in between that fold it.
    let db = open();
    let mut model = BTreeMap::new();
    let mut t = db.begin_write().unwrap();
    for i in 0..500usize {
        let k = key((i * 7) % 120);
        let v = format!("v{i}").into_bytes();
        t.put(&k, &v).unwrap();
        model.insert(k.clone(), v.clone());
        if i % 50 == 0 {
            assert_eq!(t.get(&k).unwrap(), Some(v));
        }
        if i % 97 == 0 {
            t.delete(&k).unwrap();
            model.remove(&k);
            assert_eq!(t.get(&k).unwrap(), None);
        }
    }
    let want: Vec<_> = model.into_iter().collect();
    assert_eq!(t.scan(Bound::Unbounded, Bound::Unbounded, 1000).unwrap(), want);
    t.commit().unwrap();
    assert_eq!(db.snapshot().unwrap().scan(Bound::Unbounded, Bound::Unbounded, 1000).unwrap(), want);
}

fn batch_of(n: usize) {
    let db = open();
    let mut t = db.begin_write().unwrap();
    // Every key is written twice, in reverse order the second time, so the
    // commit has to sort and to keep the second value.
    for i in 0..n {
        t.put_owned(key(i), b"first".to_vec()).unwrap();
    }
    for i in (0..n).rev() {
        t.put_owned(key(i), format!("v{i}").into_bytes()).unwrap();
    }
    assert_eq!(t.get(&key(n / 2)).unwrap(), Some(format!("v{}", n / 2).into_bytes()));
    t.commit().unwrap();
    let snap = db.snapshot().unwrap();
    let rows = snap.scan(Bound::Unbounded, Bound::Unbounded, n + 1).unwrap();
    assert_eq!(rows.len(), n);
    for (i, (k, v)) in rows.iter().enumerate() {
        assert_eq!((k, v), (&key(i), &format!("v{i}").into_bytes()));
    }
}

#[test]
fn a_batch_of_1_record() {
    batch_of(1);
}

#[test]
fn a_batch_of_1000_records() {
    batch_of(1000);
}

#[test]
fn a_batch_of_100000_records() {
    batch_of(100_000);
}
