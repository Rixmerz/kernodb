//! P5 (post-Stage-9), defect K2: live-unit counts per type
//! (`docs/file-format/units-v5.md`).
//!
//! A typed query through `IDX_TYPE` used to answer one unit short, with no
//! error, when an entry was lost, and a lost `IDX_PRIMARY` entry went unseen
//! once the `IDX_META` total was lowered to match. The count record per unit
//! type makes both reads compare what their scan saw with what the writers
//! recorded. These tests damage the indexes and the counts every way the
//! design promises to see (and the ways it declares it cannot), and require
//! the answer to be the reference's or an explicit `Corruption` / `Fallback`,
//! never short; they pin the record layout, the writer's upkeep through every
//! kind of write, the verifier rules, and the legacy path (a store written
//! before layer 5 opens, reads unchanged and gets a count when a write next
//! touches the type).
//!
//! Every database test runs on `emulated_log` and on `kerno_db`; the
//! committed legacy directory exists only for `kerno_db` (the other builds the
//! same content in memory).

#![cfg(feature = "test-support")]

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use harness::units::{Target, open_units, raw, seq};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{IndexMeta, Key, LayerMeta, MAX_LAYER_VERSION};
use kerno_units::type_count::{check_shape, record};
use kerno_units::*;

type EmuLog = kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg(first: u64) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(first)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: Target>() -> UnitDb<E> {
    open_units::<E>(SimVfs::new(5).as_vfs(), &cfg(1)).expect("open")
}

fn new_unit(t: &str, schema: u32, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: schema, metadata: vec![], payload: payload.to_vec() }
}

/// New units `(type, schema_version)` in one change set.
fn create_units<E: Target>(db: &UnitDb<E>, spec: &[(&str, u32)]) -> Vec<UnitId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let ids = spec.iter().enumerate().map(|(i, (t, v))| cs.create(new_unit(t, *v, &[i as u8])).unwrap()).collect();
    cs.commit().unwrap();
    ids
}

fn change<E: Target>(db: &UnitDb<E>, f: impl FnOnce(&mut ChangeSet<'_, E>)) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    f(&mut cs);
    cs.commit().unwrap();
}

type Patch = Vec<(Vec<u8>, Option<Vec<u8>>)>;

fn put_all<E: Target>(db: &UnitDb<E>, puts: Patch) {
    db.raw_commit(puts).unwrap();
}

fn undo_of<E: Target>(db: &UnitDb<E>, keys: impl IntoIterator<Item = Vec<u8>>) -> Patch {
    keys.into_iter()
        .map(|k| {
            let v = db.raw_get(&k).unwrap();
            (k, v)
        })
        .collect()
}

#[track_caller]
fn clean<E: Target>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().unwrap();
    assert!(r.is_clean(), "{ctx}: {:?}", r.findings);
}

fn rules<E: Target>(db: &UnitDb<E>) -> BTreeSet<String> {
    db.verify_units().unwrap().rules().into_iter().map(|r| r.to_string()).collect()
}

fn mark_key() -> Vec<u8> {
    Key::LayerMeta { name: "layer".into() }.encode().unwrap()
}

fn layer_version<E: Target>(db: &UnitDb<E>) -> u32 {
    let mark = raw(db).into_iter().find(|(k, _)| *k == mark_key()).expect("mark").1;
    LayerMeta::decode_upto(&mark, MAX_LAYER_VERSION).unwrap().layer_version
}

/// The stored `0x73` records, as `unit_type -> count`.
fn counts<E: Target>(db: &UnitDb<E>) -> BTreeMap<String, u64> {
    raw(db)
        .into_iter()
        .filter(|(k, _)| k[0] == 0x73)
        .map(|(k, v)| {
            let n = check_shape(&k, &v).expect("a count record that decodes");
            (String::from_utf8(k[2..].to_vec()).unwrap(), n)
        })
        .collect()
}

fn want(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
    pairs.iter().map(|(t, n)| ((*t).to_owned(), *n)).collect()
}

fn by_type(t: &str, v: Option<u32>) -> UnitFilter {
    UnitFilter { unit_type: Some(t.into()), schema_version: v }
}

fn is_corruption(e: &UnitError) -> bool {
    matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_)))
}

// ---------------------------------------------------------------- layout

/// The stored bytes, pinned: `73 ‖ len ‖ type` and `01 00 ‖ u64`; the mark is
/// 5; a type may hold a NUL and reach the 64 bytes of a unit type.
fn record_layout_is_pinned<E: Target>() {
    let db = open::<E>();
    let long = "t".repeat(64);
    create_units(&db, &[("doc", 1), ("doc", 1), ("img", 1), ("a\0b", 1), (&long, 1)]);
    let got: BTreeMap<Vec<u8>, Vec<u8>> = raw(&db).into_iter().filter(|(k, _)| k[0] == 0x73).collect();
    let value = |n: u8| vec![1, 0, 0, 0, 0, 0, 0, 0, 0, n];
    let mut long_key = vec![0x73, 64];
    long_key.extend_from_slice(long.as_bytes());
    assert_eq!(
        got,
        BTreeMap::from([
            (vec![0x73, 3, b'd', b'o', b'c'], value(2)),
            (vec![0x73, 3, b'i', b'm', b'g'], value(1)),
            (vec![0x73, 3, b'a', 0, b'b'], value(1)),
            (long_key, value(1)),
        ])
    );
    assert_eq!(layer_version(&db), 7);
    clean(&db, "layout");
}

// ------------------------------------------------------------- the writer

/// Every kind of write that moves a unit in or out of `Active`, and the ones
/// that must not: create +1, archive -1, unarchive +1, delete of an active
/// unit -1, delete of an archived one nothing; update (schema version
/// included), restore and a unit of another type leave a count alone. Zero is
/// stored.
fn counts_follow_every_lifecycle_change<E: Target>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("a", 1), ("b", 1)]);
    assert_eq!(counts(&db), want(&[("a", 2), ("b", 1)]));
    clean(&db, "created");

    change(&db, |cs| {
        cs.update(u[0], UnitUpdate { payload: b"x".to_vec(), schema_version: Some(9), metadata: None }, None).unwrap();
    });
    change(&db, |cs| {
        cs.restore(u[0], RevNo(1), None).unwrap();
    });
    assert_eq!(counts(&db), want(&[("a", 2), ("b", 1)]), "update and restore change no count");

    change(&db, |cs| {
        cs.archive(u[0], None).unwrap();
    });
    assert_eq!(counts(&db), want(&[("a", 1), ("b", 1)]));
    clean(&db, "archived");
    change(&db, |cs| {
        cs.unarchive(u[0], None).unwrap();
    });
    assert_eq!(counts(&db), want(&[("a", 2), ("b", 1)]));

    // Deleting an archived unit: it already left the count.
    change(&db, |cs| {
        cs.archive(u[1], None).unwrap();
    });
    change(&db, |cs| {
        cs.delete(u[1], None).unwrap();
    });
    assert_eq!(counts(&db), want(&[("a", 1), ("b", 1)]), "an archived unit is deleted without a second decrement");
    clean(&db, "archived then deleted");
    // Deleting an active one.
    change(&db, |cs| {
        cs.delete(u[0], None).unwrap();
    });
    assert_eq!(counts(&db), want(&[("a", 0), ("b", 1)]), "zero is stored, not removed");
    assert_eq!(layer_version(&db), 7);
    clean(&db, "all of a gone");
    // A type back from zero keeps counting from the record, with no scan to rebuild it.
    create_units(&db, &[("a", 1)]);
    assert_eq!(counts(&db), want(&[("a", 1), ("b", 1)]));
    // Several changes to one type in one change set compose.
    change(&db, |cs| {
        cs.create(new_unit("a", 1, b"1")).unwrap();
        cs.create(new_unit("a", 1, b"2")).unwrap();
        cs.archive(u[2], None).unwrap();
    });
    assert_eq!(counts(&db), want(&[("a", 3), ("b", 0)]));
    clean(&db, "composed");
    db.rebuild_index().unwrap();
    for t in ["a", "b"] {
        let snap = db.snapshot().unwrap();
        let f = by_type(t, None);
        assert_eq!(snap.indexed_scan_via(IndexPath::ByType, &f).unwrap(), snap.reference_filter(&f).unwrap());
    }
}

/// An operation that is refused, a change set that is dropped and one that
/// is closed by its limit leave no count behind (P1's review found orphan
/// counters after a refused creation): the counts after the commit are those
/// of the units that exist.
fn a_refused_or_aborted_write_leaves_no_count<E: Target>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("a", 1)]);
    change(&db, |cs| {
        cs.create_relation(u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    });
    let before = counts(&db);

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    assert!(matches!(cs.delete(u[0], None), Err(UnitError::UnitHasRelations { .. })), "a unit with relations");
    assert!(matches!(cs.archive(u[1], None), Err(UnitError::UnitHasRelations { .. })));
    let noop = UnitUpdate { payload: b"x".to_vec(), schema_version: None, metadata: None };
    assert!(matches!(cs.update(u[0], noop, Some(RevNo(9))), Err(UnitError::Conflict { .. })));
    assert!(matches!(cs.create(new_unit(&"x".repeat(65), 1, b"")), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(cs.delete(UnitId([9; 16]), None), Err(UnitError::NotFound { .. })));
    cs.create(new_unit("b", 1, b"ok")).unwrap();
    cs.commit().unwrap();
    assert_eq!(counts(&db), BTreeMap::from([("a".to_owned(), 2), ("b".to_owned(), 1)]));
    assert_eq!(before, want(&[("a", 2)]));
    clean(&db, "after refusals");

    // Dropped: nothing, and an `abort` too.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create(new_unit("c", 1, b"x")).unwrap();
    drop(cs);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create(new_unit("c", 1, b"y")).unwrap();
    cs.abort();
    assert_eq!(counts(&db), want(&[("a", 2), ("b", 1)]));
    clean(&db, "after aborts");

    // The change set limit refuses the second create before it counts it.
    let mut c = cfg(100);
    c.max_changeset_ops = 1;
    let limited = open_units::<E>(SimVfs::new(6).as_vfs(), &c).unwrap();
    let mut cs = limited.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create(new_unit("a", 1, b"1")).unwrap();
    assert!(matches!(cs.create(new_unit("a", 1, b"2")), Err(UnitError::ChangeSetTooLarge { .. })));
    cs.commit().unwrap();
    assert_eq!(counts(&limited), want(&[("a", 1)]));
    clean(&limited, "after the limit");
}

/// A database of counted units survives export and import: the target is
/// counted too (the import writes its units through the same upkeep) and
/// verifies clean, whatever lifecycle each unit ends in.
fn export_import_keeps_the_counts<E: Target>() {
    let src = open::<E>();
    let u = create_units(&src, &[("a", 1), ("a", 1), ("b", 1), ("c", 1)]);
    change(&src, |cs| {
        cs.archive(u[1], None).unwrap();
        cs.delete(u[3], None).unwrap();
    });
    change(&src, |cs| {
        cs.update(u[0], UnitUpdate { payload: b"v2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    });
    assert_eq!(counts(&src), want(&[("a", 1), ("b", 1), ("c", 0)]));
    let bytes = kerno_units::export::export(&src).unwrap();
    let target = open_units::<E>(SimVfs::new(7).as_vfs(), &cfg(100)).unwrap();
    kerno_units::import::import(&target, &bytes).unwrap();
    // `c` has no active unit to import, so the target never counts it: no record, not a zero.
    assert_eq!(counts(&target), want(&[("a", 1), ("b", 1)]));
    assert_eq!(layer_version(&target), 7);
    clean(&target, "imported");
    assert_eq!(kerno_units::export::export(&target).unwrap(), bytes);
}

/// Retention rewrites revisions and heads' `oldest_rev`, never the set of
/// active units: the counts stay and the store verifies.
fn retention_leaves_the_counts<E: Target>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("b", 1)]);
    for i in 0..4u8 {
        change(&db, |cs| {
            cs.update(u[0], UnitUpdate { payload: vec![i; 8], schema_version: None, metadata: None }, None).unwrap();
        });
    }
    let before = counts(&db);
    let report =
        db.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default()).unwrap();
    assert!(report.revisions_pruned > 0, "retention did something: {report:?}");
    assert_eq!(counts(&db), before);
    clean(&db, "after retention");
    db.rebuild_index().unwrap();
    let snap = db.snapshot().unwrap();
    let f = by_type("a", None);
    assert_eq!(snap.indexed_scan_via(IndexPath::ByType, &f).unwrap(), snap.reference_filter(&f).unwrap());
}

/// Reclaim and compaction rewrite files, not keys: the counts survive them.
#[test]
fn reclaim_and_compact_leave_the_counts() {
    let db = open::<KernoDb>();
    let u = create_units(&db, &[("a", 1), ("a", 1), ("b", 1)]);
    change(&db, |cs| {
        cs.archive(u[1], None).unwrap();
    });
    let before = counts(&db);
    assert_eq!(before, want(&[("a", 1), ("b", 1)]));
    db.reclaim().unwrap();
    assert_eq!(counts(&db), before);
    db.compact().unwrap();
    assert_eq!(counts(&db), before);
    clean(&db, "after reclaim and compact");
    db.rebuild_index().unwrap();
    let snap = db.snapshot().unwrap();
    let f = by_type("a", None);
    assert_eq!(snap.indexed_scan_via(IndexPath::ByType, &f).unwrap(), snap.reference_filter(&f).unwrap());
}

// ------------------------------------------------------------- the reader

#[derive(Clone, Copy, PartialEq, Debug)]
enum Read {
    Right,
    Short,
    Corrupt,
}

/// The five reads every damage case is held to: `IDX_TYPE` of `a` (all
/// versions, and version 1 alone), of `b`, and `IDX_PRIMARY` (every unit, and
/// type `b`: a read that skips every `a` entry still tallies them).
fn reads<E: Target>(db: &UnitDb<E>, name: &str) -> [Read; 5] {
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.index_status().unwrap().state, IndexState::Ready, "{name}: the patch kept the index ready");
    let cases = [
        (IndexPath::ByType, by_type("a", None)),
        (IndexPath::ByType, by_type("a", Some(1))),
        (IndexPath::ByType, by_type("b", None)),
        (IndexPath::Primary, UnitFilter::default()),
        (IndexPath::Primary, by_type("b", None)),
    ];
    cases.map(|(path, filter)| {
        let reference = snap.reference_filter(&filter).unwrap();
        match snap.indexed_scan_via(path, &filter) {
            Ok(units) if units == reference => Read::Right,
            Ok(units) => {
                assert!(units.len() < reference.len(), "{name} / {path:?} {filter:?}: short, not different");
                Read::Short
            }
            Err(e) => {
                assert!(is_corruption(&e), "{name} / {path:?} {filter:?}: {e}");
                Read::Corrupt
            }
        }
    })
}

fn type_key(unit_type: &str, schema_version: u32, unit: UnitId) -> Vec<u8> {
    Key::IndexType { unit_type: unit_type.into(), schema_version, unit }.encode().unwrap()
}

fn entry_key(unit: UnitId) -> Vec<u8> {
    Key::IndexPrimary { unit }.encode().unwrap()
}

fn meta_key() -> Vec<u8> {
    Key::IndexMeta.encode().unwrap()
}

fn meta_value(watermark: u64, entries: u64) -> Vec<u8> {
    IndexMeta { outcome: BuildOutcome::Succeeded, watermark: CommitSeq(watermark), entries }.encode().unwrap()
}

fn count_key(t: &str) -> Vec<u8> {
    record(t, 0).0
}

/// The damage matrix of K2. Units `a/1`, `b/1`, `a/2` (counts a = 2, b = 1),
/// index built. Each patch commits once (and so does its undo), so the
/// `IDX_META` watermark names the next sequence and the index stays `Ready`.
/// The rows below the line are the declared limits: damage the reads cannot
/// see because it agrees with itself, which `verify_units` reports.
fn damage_matrix<E: Target>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("b", 1), ("a", 2)]);
    assert_eq!(db.rebuild_index().unwrap().state, IndexState::Ready);
    clean(&db, "built");
    let u0 = u[0];
    use Read::{Corrupt, Right, Short};
    type Case = (&'static str, Box<dyn Fn(u64) -> Patch>, [Read; 5], &'static [&'static str]);
    let cases: Vec<Case> = vec![
        (
            "IDX_TYPE entry of a lost, count intact",
            Box::new(move |w| vec![(type_key("a", 1, u0), None), (meta_key(), Some(meta_value(w, 3)))]),
            [Corrupt, Corrupt, Right, Right, Right],
            &["UNIT-38"],
        ),
        (
            "count of a lowered",
            Box::new(move |w| vec![(record("a", 1).0, Some(record("a", 1).1)), (meta_key(), Some(meta_value(w, 3)))]),
            [Corrupt, Corrupt, Right, Corrupt, Corrupt],
            &["UNIT-47"],
        ),
        (
            "count of a raised",
            Box::new(move |w| vec![(record("a", 3).0, Some(record("a", 3).1)), (meta_key(), Some(meta_value(w, 3)))]),
            [Corrupt, Corrupt, Right, Corrupt, Corrupt],
            &["UNIT-47"],
        ),
        (
            "count of b lowered to zero",
            Box::new(move |w| vec![(record("b", 0).0, Some(record("b", 0).1)), (meta_key(), Some(meta_value(w, 3)))]),
            [Right, Right, Corrupt, Corrupt, Corrupt],
            &["UNIT-47"],
        ),
        (
            "IDX_PRIMARY entry lost, IDX_META total lowered to match (K2)",
            Box::new(move |w| vec![(entry_key(u0), None), (meta_key(), Some(meta_value(w, 2)))]),
            [Right, Right, Right, Corrupt, Corrupt],
            &["UNIT-37"],
        ),
        (
            "count of b lost, entries intact (a store from before layer 5)",
            Box::new(move |w| vec![(count_key("b"), None), (meta_key(), Some(meta_value(w, 3)))]),
            [Right, Right, Right, Right, Right],
            &[],
        ),
        // ---- declared limits: damage that agrees with itself
        (
            "IDX_TYPE entry and count of a both lost (double damage)",
            Box::new(move |w| {
                vec![(type_key("a", 1, u0), None), (count_key("a"), None), (meta_key(), Some(meta_value(w, 3)))]
            }),
            [Short, Short, Right, Right, Right],
            &["UNIT-38"],
        ),
        (
            "IDX_TYPE entry lost, count of a lowered to match",
            Box::new(move |w| {
                vec![
                    (type_key("a", 1, u0), None),
                    (record("a", 1).0, Some(record("a", 1).1)),
                    (meta_key(), Some(meta_value(w, 3))),
                ]
            }),
            [Short, Short, Right, Corrupt, Corrupt],
            &["UNIT-38", "UNIT-47"],
        ),
    ];
    for (name, patch, expected, verifier) in cases {
        let patch = patch(seq(&db) + 1);
        let undo = undo_of(&db, patch.iter().map(|(k, _)| k.clone()));
        put_all(&db, patch);
        let found = rules(&db);
        if verifier.is_empty() {
            assert!(found.is_empty(), "{name}: verify_units found {found:?}");
        } else {
            for rule in verifier {
                assert!(found.contains(*rule), "{name}: verify_units says {found:?}, not {rule}");
            }
        }
        assert_eq!(reads(&db, name), expected, "{name}");
        // The next sequence again: the undo is one commit too.
        let mut undo = undo;
        undo.retain(|(k, _)| *k != meta_key());
        undo.push((meta_key(), Some(meta_value(seq(&db) + 1, 3))));
        put_all(&db, undo);
        clean(&db, name);
        assert_eq!(reads(&db, name), [Right; 5], "{name}: undone");
    }
}

/// `query` on a damaged typed read answers by the reference path with a
/// `Fallback` `Corrupt`, and the answer is the reference's, complete.
fn query_falls_back_on_a_typed_mismatch<E: Target>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("b", 1), ("a", 2)]);
    db.rebuild_index().unwrap();
    let f = by_type("a", None);
    let healthy = db.snapshot().unwrap().query(&f).unwrap();
    assert_eq!((healthy.path, healthy.fallback), (PlanPath::IndexType, None));
    let w = seq(&db) + 1;
    put_all(&db, vec![(type_key("a", 1, u[0]), None), (meta_key(), Some(meta_value(w, 3)))]);
    let snap = db.snapshot().unwrap();
    let out = snap.query(&f).unwrap();
    assert_eq!(out.units, snap.reference_filter(&f).unwrap());
    assert_eq!(out.units.len(), 2, "complete");
    assert_eq!(out.path, PlanPath::UnitsReference);
    let fallback = out.fallback.expect("the index path was taken and abandoned");
    assert_eq!((fallback.abandoned, fallback.cause), (PlanPath::IndexType, FallbackCause::Corrupt));
}

// ------------------------------------------------------------ the verifier

/// UNIT-47 against the active heads, UNIT-48 against the mark, and the shapes
/// the record refuses.
fn verifier_rules_of_the_count_records<E: Target>() {
    let db = open::<E>();
    create_units(&db, &[("a", 1), ("a", 1)]);
    clean(&db, "healthy");
    let key = record("a", 0).0;
    let good = db.raw_get(&key).unwrap().unwrap();
    // The count against the heads: either way, and a count for a type with no unit.
    for n in [0, 1, 3] {
        put_all(&db, vec![(key.clone(), Some(record("a", n).1))]);
        assert_eq!(rules(&db), BTreeSet::from(["UNIT-47".to_owned()]), "count {n}");
    }
    put_all(&db, vec![(key.clone(), Some(good.clone()))]);
    clean(&db, "restored");
    put_all(&db, vec![(record("ghost", 0).0, Some(record("ghost", 2).1))]);
    assert_eq!(rules(&db), BTreeSet::from(["UNIT-47".to_owned()]), "a count of units that do not exist");
    put_all(&db, vec![(record("ghost", 0).0, Some(record("ghost", 0).1))]);
    clean(&db, "a zero for a type with no unit is sound");
    // The mark below 5 with records (UNIT-48), whatever it is lowered to.
    let mark = raw(&db).into_iter().find(|(k, _)| *k == mark_key()).unwrap().1;
    let current = LayerMeta::decode_upto(&mark, MAX_LAYER_VERSION).unwrap();
    for lowered in [1, 2, 3, 4] {
        let v = LayerMeta { layer_version: lowered, ..current.clone() }.encode().unwrap();
        put_all(&db, vec![(mark_key(), Some(v))]);
        assert_eq!(rules(&db), BTreeSet::from(["UNIT-48".to_owned()]), "mark {lowered}");
    }
    put_all(&db, vec![(mark_key(), Some(mark))]);
    clean(&db, "mark restored");
    // The shapes of the value.
    for (bad, rule) in [
        (vec![1, 0, 0, 0], "UNIT-9"),
        (vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0], "UNIT-10"),
        (vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 2], "UNIT-6"),
        (vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 2], "UNIT-7"),
        (vec![1, 1, 0, 0, 0, 0, 0, 0, 0, 2], "UNIT-8"),
    ] {
        put_all(&db, vec![(key.clone(), Some(bad))]);
        assert!(rules(&db).contains(rule), "{rule}: {:?}", rules(&db));
    }
    put_all(&db, vec![(key.clone(), Some(good))]);
    // The shapes of the key.
    for (bad, rule) in [(vec![0x73], "UNIT-3"), (vec![0x73, 5, b'a'], "UNIT-3"), (vec![0x73, 1, 0xff], "UNIT-12")] {
        put_all(&db, vec![(bad.clone(), Some(record("a", 2).1))]);
        assert!(rules(&db).contains(rule), "{rule}: {:?}", rules(&db));
        put_all(&db, vec![(bad, None)]);
    }
    clean(&db, "restored");
}

// ---------------------------------------------------------------- vectors

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex {s:?}");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn cells(line: &str) -> Option<Vec<String>> {
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|c| c.trim().trim_matches('`').to_owned()).collect())
}

/// Each row of section 6 of `docs/file-format/units-v5.md`: positives are what
/// the production writer stores, negatives give the rule that is named.
#[test]
fn vectors_of_the_document_match_the_codec_byte_for_byte() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/file-format/units-v5.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (mut ok, mut bad, mut marks_ok, mut marks_bad) = (0, 0, 0, 0);
    for line in text.lines() {
        let Some(c) = cells(line) else { continue };
        if c.len() != 4 || !c[0].chars().next().is_some_and(|ch| ch.is_ascii_lowercase()) || c[0] == "nombre" {
            continue;
        }
        let (name, result) = (c[0].as_str(), c[3].as_str());
        if name.starts_with("mark_") {
            let reader: u32 = c[2].parse().unwrap();
            match (result, LayerMeta::decode_upto(&unhex(&c[1]), reader)) {
                ("ok", Ok(m)) => {
                    assert_eq!(m.layer_version, 5, "{name}");
                    assert_eq!(m.encode().unwrap(), unhex(&c[1]), "{name}");
                    marks_ok += 1;
                }
                (rule, Err(e)) => {
                    assert_eq!(e.rule.id(), rule, "{name}");
                    marks_bad += 1;
                }
                (rule, Ok(_)) => panic!("{name}: decoded, wanted {rule}"),
            }
            continue;
        }
        let (key, value) = (unhex(&c[1]), unhex(&c[2]));
        match (result, check_shape(&key, &value)) {
            ("ok", Ok(count)) => {
                let ty = std::str::from_utf8(&key[2..]).unwrap();
                assert_eq!(record(ty, count), (key, value), "{name}: bytes the writer stores");
                ok += 1;
            }
            (rule, Err(e)) => {
                assert_eq!(e.rule.id(), rule, "{name}: {e}");
                bad += 1;
            }
            (rule, Ok(_)) => panic!("{name}: accepted, wanted {rule}"),
        }
    }
    assert_eq!((ok, bad, marks_ok, marks_bad), (4, 11, 1, 2));
}

// ---------------------------------------------------------------- fixture

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            macro_rules! each {
                ($f:ident) => {
                    #[test]
                    fn $f() {
                        super::$f::<$e>();
                    }
                };
            }
            each!(record_layout_is_pinned);
            each!(counts_follow_every_lifecycle_change);
            each!(a_refused_or_aborted_write_leaves_no_count);
            each!(export_import_keeps_the_counts);
            each!(retention_leaves_the_counts);
            each!(damage_matrix);
            each!(query_falls_back_on_a_typed_mismatch);
            each!(verifier_rules_of_the_count_records);
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
