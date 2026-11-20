//! Post-Stage-9 defects K4 and K5 (`docs/architecture/stage9-k-e3-7.md` §4).
//!
//! `change_set` / `change_sets_since` used to return members that disagree
//! with their `REV_META`, and `get_relation` used to return a relation whose
//! adjacency entries, endpoints or `created_seq` were damaged. Each test
//! damages one stored key by hand, requires the read to answer
//! `Storage(Corruption)`, undoes the damage and requires the read to answer
//! again. Every test runs on `emulated_log` and on `kerno_db`.

#![cfg(feature = "test-support")]

mod harness;

use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{Adjacency, CsMember, Key, RelationRecord};
use kerno_units::*;

type EmuLog = kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn open<E: Target>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    open_units::<E>(SimVfs::new(5).as_vfs(), &c).expect("open")
}

fn new_unit(payload: u8) -> NewUnit {
    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: vec![payload] }
}

fn is_corruption<T>(r: &Result<T, UnitError>) -> bool {
    matches!(r, Err(UnitError::Storage(kerno_core::Error::Corruption(_))))
}

/// Damages `key` (`None` deletes it), runs `read` and requires corruption,
/// then restores the key and requires `read` to be right again.
#[track_caller]
fn damaged_read_is_corruption<E: Target, T: std::fmt::Debug>(
    db: &UnitDb<E>,
    name: &str,
    key: Vec<u8>,
    damage: Option<Vec<u8>>,
    read: impl Fn() -> Result<T, UnitError>,
) {
    let original = db.raw_get(&key).unwrap().expect("the key exists before the damage");
    db.raw_commit(vec![(key.clone(), damage)]).unwrap();
    let got = read();
    assert!(is_corruption(&got), "{name}: {got:?}");
    db.raw_commit(vec![(key, Some(original))]).unwrap();
    read().unwrap_or_else(|e| panic!("{name}: the read fails after the undo: {e}"));
}

// ------------------------------------------------------------------ K4

/// Change set 1 creates a unit (member `(u, 1, Create)`), change set 2
/// updates it (member `(u, 2, Update)`).
fn change_set_members_are_checked<E: Target>() {
    let db = open::<E>();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let u = cs.create(new_unit(1)).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(u, UnitUpdate { payload: vec![2], schema_version: None, metadata: None }, None).unwrap();
    let second = cs.commit().unwrap().change_set;
    assert_eq!(db.change_set(second).unwrap().expect("exists").members.len(), 1);

    let member = Key::CsMember { seq: second, ordinal: 0 }.encode().unwrap();
    let bytes = |unit, rev, kind| Some(CsMember { unit, rev_no: RevNo(rev), kind }.encode().unwrap());
    let cases = [
        ("kind differs from REV_META", bytes(u, 2, RevisionKind::Archive)),
        ("names a revision of another change set", bytes(u, 1, RevisionKind::Update)),
        ("names a revision above the head", bytes(u, 9, RevisionKind::Update)),
        ("names an unknown unit", bytes(UnitId([0xEE; 16]), 2, RevisionKind::Update)),
    ];
    for (name, damage) in cases {
        damaged_read_is_corruption(&db, name, member.clone(), damage.clone(), || db.change_set(second));
        damaged_read_is_corruption(&db, name, member.clone(), damage, || db.change_sets_since(CommitSeq(0), 10));
    }
    // The REV_META the member names is gone.
    let meta = Key::RevMeta { unit: u, rev: RevNo(2) }.encode().unwrap();
    damaged_read_is_corruption(&db, "REV_META missing", meta, None, || db.change_set(second));
}

// ------------------------------------------------------------------ K5

fn get_relation_checks_created_seq_and_both_ends<E: Target>() {
    let db = open::<E>();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let (a, b) = (cs.create(new_unit(1)).unwrap(), cs.create(new_unit(2)).unwrap());
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let r = cs.create_relation(a, b, "cites", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let snap = db.snapshot().unwrap();
    let rel = snap.get_relation(r).unwrap().expect("exists");
    assert_eq!((rel.from, rel.to), (a, b));
    drop(snap);

    let rel_key = Key::Relation { edge: r }.encode().unwrap();
    let record = RelationRecord::decode(&db.raw_get(&rel_key).unwrap().unwrap()).unwrap();
    let future = RelationRecord { created_seq: CommitSeq(10_000), ..record }.encode().unwrap();
    let out = Key::RelationOut { unit: a, relation_type: "cites".into(), edge: r }.encode().unwrap();
    let inn = Key::RelationIn { unit: b, relation_type: "cites".into(), edge: r }.encode().unwrap();
    let wrong_end = Adjacency { other: a }.encode();
    let read = || db.snapshot()?.get_relation(r);
    let cases = [
        ("created_seq after the snapshot", rel_key, Some(future)),
        ("REL_OUT missing", out.clone(), None),
        ("REL_IN missing", inn.clone(), None),
        ("REL_OUT names another unit", out, Some(wrong_end.clone())),
        ("REL_IN names another unit", inn, Some(Adjacency { other: b }.encode())),
        ("source unit missing", Key::UnitHead { unit: a }.encode().unwrap(), None),
        ("target unit missing", Key::UnitHead { unit: b }.encode().unwrap(), None),
    ];
    for (name, key, damage) in cases {
        damaged_read_is_corruption(&db, name, key, damage, read);
    }
}

macro_rules! engines {
    ($($m:ident: $E:ty),*) => {$(
        mod $m {
            type E = $E;
            use super::*;
            #[test]
            fn change_set_members_are_checked() {
                super::change_set_members_are_checked::<E>();
            }
            #[test]
            fn get_relation_checks_created_seq_and_both_ends() {
                super::get_relation_checks_created_seq_and_both_ends::<E>();
            }
        }
    )*};
}

engines!(emulated_log: EmuLog, kerno: KernoDb);
