//! E4-01: the reference query path (`reference_get`, `reference_scan`,
//! `reference_by_type`) over `Emulated<LogEngine>` and `KernoDb`.
//!
//! The expectation is tracked by the test itself (a map of what is active),
//! not read back from the layer.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Options, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn open<E: TxnEngine>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open")
}

fn new_unit(t: &str, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: 1, metadata: Vec::new(), payload: payload.to_vec() }
}

/// `(unit_type, payload)` of every unit the test expects to be active.
type Expected = BTreeMap<UnitId, (String, Vec<u8>)>;

#[track_caller]
fn check_scan<E: kerno_core::contract::Engine>(snap: &UnitSnapshot<E>, want: &Expected) {
    let all = snap.reference_scan(|_| true).expect("scan");
    let got: Vec<_> = all.iter().map(|u| (u.id, u.unit_type.clone(), u.payload.clone().unwrap())).collect();
    let exp: Vec<_> = want.iter().map(|(id, (t, p))| (*id, t.clone(), p.clone())).collect();
    assert_eq!(got, exp, "reference_scan(all)");
    assert!(all.windows(2).all(|w| w[0].id < w[1].id), "unit id order");
    for t in ["a", "b", "none"] {
        let by: Vec<_> = snap.reference_by_type(t).expect("by_type").iter().map(|u| u.id).collect();
        let exp: Vec<_> = want.iter().filter(|(_, (ut, _))| ut == t).map(|(id, _)| *id).collect();
        assert_eq!(by, exp, "reference_by_type({t})");
    }
    for (id, (t, p)) in want {
        let u = snap.reference_get(*id).expect("get").expect("active unit");
        assert_eq!((&u.unit_type, u.payload.as_ref()), (t, Some(p)));
    }
}

fn empty<E: TxnEngine>() {
    let db = open::<E>();
    let snap = db.snapshot().expect("snapshot");
    assert!(snap.reference_scan(|_| true).unwrap().is_empty());
    assert!(snap.reference_by_type("a").unwrap().is_empty());
    assert_eq!(snap.reference_get(UnitId([7; 16])).unwrap(), None);
}

fn many_and_excluded<E: TxnEngine>() {
    let db = open::<E>();
    let mut want = Expected::new();
    let mut ids = Vec::new();
    // More than one scan page (256 heads) of units, in several change sets.
    for chunk in 0..3u32 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
        for i in 0..100u32 {
            let n = chunk * 100 + i;
            let t = if n % 2 == 0 { "a" } else { "b" };
            let p = n.to_le_bytes().to_vec();
            let id = cs.create(new_unit(t, &p)).expect("create");
            ids.push((id, n));
            want.insert(id, (t.to_owned(), p));
        }
        cs.commit().expect("commit");
    }
    let snap = db.snapshot().expect("snapshot");
    assert_eq!(want.len(), 300);
    check_scan(&snap, &want);
    // Predicate on a field other than the type.
    let hit = snap.reference_scan(|u| u.payload.as_deref() == Some(&7u32.to_le_bytes()[..])).unwrap();
    assert_eq!(hit.len(), 1);

    // Archive every 3rd, delete every 5th (not already archived), unarchive
    // every 6th archived one back, update every 7th active one.
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    for (id, n) in &ids {
        if n % 3 == 0 {
            cs.archive(*id, None).unwrap();
            want.remove(id);
        } else if n % 5 == 0 {
            cs.delete(*id, None).unwrap();
            want.remove(id);
        } else if n % 7 == 0 {
            let upd = UnitUpdate { payload: b"upd".to_vec(), schema_version: None, metadata: None };
            cs.update(*id, upd, None).unwrap();
            want.get_mut(id).unwrap().1 = b"upd".to_vec();
        }
    }
    cs.commit().expect("commit");
    // One revision per unit per change set: unarchive in a second one.
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    for (id, n) in ids.iter().filter(|(_, n)| n % 6 == 0) {
        cs.unarchive(*id, None).unwrap();
        want.insert(*id, (if n % 2 == 0 { "a" } else { "b" }.to_owned(), n.to_le_bytes().to_vec()));
    }
    cs.commit().expect("commit");
    let after = db.snapshot().expect("snapshot");
    assert!(want.len() < 300 && !want.is_empty());
    check_scan(&after, &want);
    // Excluded units: get_unit still sees them, the reference path does not.
    let (gone, _) = ids.iter().find(|(id, n)| n % 3 == 0 && n % 6 != 0 && !want.contains_key(id)).unwrap();
    assert!(db.get_unit(*gone).unwrap().is_some());
    assert_eq!(after.reference_get(*gone).unwrap(), None);
    let (deleted, _) = ids.iter().find(|(_, n)| n % 5 == 0 && n % 3 != 0).unwrap();
    assert_eq!(after.reference_get(*deleted).unwrap(), None);
}

fn snapshot_isolation<E: TxnEngine>() {
    let db = open::<E>();
    let (a, _) = db.create_unit(new_unit("a", b"one"), ChangeMeta::default()).unwrap();
    let (b, _) = db.create_unit(new_unit("a", b"two"), ChangeMeta::default()).unwrap();
    let snap = db.snapshot().unwrap();
    let mut before = Expected::new();
    before.insert(a, ("a".into(), b"one".to_vec()));
    before.insert(b, ("a".into(), b"two".to_vec()));
    check_scan(&snap, &before);

    // Writes after the snapshot: a new unit, an update, an archive.
    let (c, _) = db.create_unit(new_unit("a", b"three"), ChangeMeta::default()).unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(a, UnitUpdate { payload: b"changed".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.archive(b, None).unwrap();
    cs.commit().unwrap();

    check_scan(&snap, &before);
    assert_eq!(snap.reference_get(c).unwrap(), None);
    let mut now = Expected::new();
    now.insert(a, ("a".into(), b"changed".to_vec()));
    now.insert(c, ("a".into(), b"three".to_vec()));
    check_scan(&db.snapshot().unwrap(), &now);
}

macro_rules! engine_tests {
    ($m:ident, $E:ty) => {
        mod $m {
            type E = $E;
            use super::*;
            #[test]
            fn empty() {
                super::empty::<E>();
            }
            #[test]
            fn many_and_excluded() {
                super::many_and_excluded::<E>();
            }
            #[test]
            fn snapshot_isolation() {
                super::snapshot_isolation::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, EmuLog);
engine_tests!(kerno, KernoDb);
