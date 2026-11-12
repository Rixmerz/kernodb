//! E8-03: staleness and rebuild of derived data (etapa8-acta.md §5.2,
//! ADR-0018, I-23, I-24, I-25, I-38).
//!
//! Every test runs on both engines. The compute function is a deterministic
//! function of the sources, supplied here; nothing depends on time.

mod harness;

use std::collections::BTreeMap;
use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: Target>(disk: &SimVfs) -> UnitDb<E> {
    open_units::<E>(disk.as_vfs(), &cfg()).expect("open")
}

/// The value is the version, the method and the payloads joined: it changes
/// with every field the descriptor names.
fn compute(d: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<u8>> {
    let mut out = format!("{}@{}:", d.method(), d.version()).into_bytes();
    for i in inputs {
        out.extend_from_slice(&i.payload);
        out.push(b'|');
    }
    Some(out)
}

fn never(_: &DerivedDescriptor, _: &[SourceInput]) -> Option<Vec<u8>> {
    None
}

fn params() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn make<E: TxnEngineAlias>(db: &DerivedDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn edit<E: TxnEngineAlias>(db: &DerivedDb<E>, id: UnitId, payload: &[u8]) -> CommitInfo {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap()
}

use kerno_core::contract::TxnEngine as TxnEngineAlias;

fn fresh<E: Target>() -> DerivedDb<E> {
    DerivedDb::open(open::<E>(&SimVfs::new(1)), Vec::new(), &compute).unwrap()
}

// ------------------------------------------------------------ staleness

fn a_change_to_a_source_marks_its_dependents_stale_in_the_same_changeset<E: Target>() {
    let db = fresh::<E>();
    let (a, b, c) = (make(&db, b"a"), make(&db, b"b"), make(&db, b"c"));
    let on_ab = db.build([a, b], "embed", 1, params(), &compute).unwrap();
    let on_a = db.build([a], "summary", 1, params(), &compute).unwrap();
    let on_c = db.build([c], "embed", 1, params(), &compute).unwrap();
    for id in [on_ab, on_a, on_c] {
        assert_eq!(db.state(&id), Some(DerivedState::Ready));
    }

    edit(&db, a, b"a2");
    // The state is already updated when the commit returns.
    assert_eq!(db.state(&on_ab), Some(DerivedState::Stale));
    assert_eq!(db.state(&on_a), Some(DerivedState::Stale));
    assert_eq!(db.state(&on_c), Some(DerivedState::Ready), "an unrelated datum is untouched");
    assert_eq!(db.value(&on_ab), None, "a stale datum is not served");
    assert!(db.value(&on_c).is_some());
    assert!(db.index_is_exact());

    // Deleting a source also changes it.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(c, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.state(&on_c), Some(DerivedState::Stale));
}

fn a_changeset_that_fails_or_is_dropped_marks_nothing<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let id = db.build([a], "embed", 1, params(), &compute).unwrap();

    // Stale write conflict: the expected revision is not the head.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let upd = UnitUpdate { payload: b"x".to_vec(), schema_version: None, metadata: None };
    assert!(cs.update(a, upd, Some(RevNo(99))).is_err());
    drop(cs);
    assert_eq!(db.state(&id), Some(DerivedState::Ready));

    // Aborted, and an empty change set that writes nothing.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let upd = UnitUpdate { payload: b"y".to_vec(), schema_version: None, metadata: None };
    cs.update(a, upd, None).unwrap();
    cs.abort();
    db.begin_changeset(ChangeMeta::default()).unwrap().commit().unwrap();
    assert_eq!(db.state(&id), Some(DerivedState::Ready));
    assert_eq!(db.get_unit(a).unwrap().unwrap().payload.as_deref(), Some(&b"a"[..]));
}

// ------------------------------------------------------------ rebuild

fn rebuilding_gives_what_building_anew_gives<E: Target>() {
    let db = fresh::<E>();
    let (a, b) = (make(&db, b"a"), make(&db, b"b"));
    let old = db.build([a, b], "embed", 1, params(), &compute).unwrap();
    edit(&db, a, b"a2");
    assert_eq!(db.state(&old), Some(DerivedState::Stale));

    let rebuilt = db.rebuild(&old, &compute).unwrap();
    assert_eq!(db.state(&rebuilt), Some(DerivedState::Ready));

    // The same data, built anew on an independent database with the same history.
    let other = fresh::<E>();
    let (a2, b2) = (make(&other, b"a"), make(&other, b"b"));
    edit(&other, a2, b"a2");
    // Ids are minted by a counter per database, so they are equal.
    assert_eq!((a, b), (a2, b2));
    let anew = other.build([a2, b2], "embed", 1, params(), &compute).unwrap();

    assert_eq!(rebuilt, anew, "same descriptor identity");
    assert_eq!(db.descriptor(&rebuilt), other.descriptor(&anew), "same input hash and sources");
    assert_eq!(db.value(&rebuilt), other.value(&anew), "same result");
    assert_eq!(db.value(&rebuilt).as_deref(), Some(&b"embed@1:a2|b|"[..]));
    assert!(db.index_is_exact());
    if rebuilt != old {
        assert_eq!(db.state(&old), None, "the replaced entry is gone, not left behind");
    }
}

fn rebuild_uses_only_the_sources_that_are_still_valid<E: Target>() {
    let db = fresh::<E>();
    let (a, b) = (make(&db, b"a"), make(&db, b"b"));
    let old = db.build([a, b], "embed", 1, params(), &compute).unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(b, None).unwrap();
    cs.commit().unwrap();

    let rebuilt = db.rebuild(&old, &compute).unwrap();
    let d = db.descriptor(&rebuilt).unwrap();
    assert_eq!(d.sources().map(|s| s.unit).collect::<Vec<_>>(), vec![a]);
    assert_eq!(db.value(&rebuilt).as_deref(), Some(&b"embed@1:a|"[..]));

    // With nothing valid left the datum is failed, never ready.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(a, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.state(&rebuilt), Some(DerivedState::Stale));
    assert!(matches!(db.rebuild(&rebuilt, &compute), Err(DerivedStaleError::NoValidSources(_))));
    assert_eq!(db.state(&rebuilt), Some(DerivedState::Failed));
    assert_eq!(db.value(&rebuilt), None);
}

fn a_failed_compute_leaves_the_datum_failed_and_a_later_rebuild_fixes_it<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let id = db.build([a], "embed", 1, params(), &compute).unwrap();
    edit(&db, a, b"a2");
    let err = db.rebuild(&id, &never).unwrap_err();
    let failed = match err {
        DerivedStaleError::ComputeFailed(f) => f,
        other => panic!("unexpected {other}"),
    };
    assert_eq!(db.state(&failed), Some(DerivedState::Failed));
    assert_eq!(db.value(&failed), None);
    // A ready datum cannot be rebuilt: it must be stale or failed first.
    let again = db.rebuild(&failed, &compute).unwrap();
    assert_eq!(db.state(&again), Some(DerivedState::Ready));
    assert!(matches!(
        db.rebuild(&again, &compute),
        Err(DerivedStaleError::Derived(DerivedError::IllegalTransition { .. }))
    ));
}

// ------------------------------------------------------------ method and version

fn a_new_version_creates_a_datum_and_supersedes_the_old_one<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let v1 = db.build([a], "embed", 1, params(), &compute).unwrap();
    let before = (db.descriptor(&v1).unwrap(), db.value(&v1).unwrap());

    let v2 = db.supersede(&v1, "embed", 2, params(), &compute).unwrap();
    assert_ne!(v1, v2);
    assert_eq!(db.state(&v1), Some(DerivedState::Superseded));
    assert_eq!(db.value(&v1), None, "a superseded datum is not served");
    assert_eq!(db.state(&v2), Some(DerivedState::Ready));
    assert_eq!(db.value(&v2).as_deref(), Some(&b"embed@2:a|"[..]));
    // The old entry is not rewritten.
    assert_eq!(db.descriptor(&v1).unwrap(), before.0);
    assert_eq!(db.descriptor(&v1).unwrap().version(), 1);

    // A new method too; and a superseded datum is final.
    let m = db.supersede(&v2, "cluster", 1, params(), &compute).unwrap();
    assert_eq!(db.state(&v2), Some(DerivedState::Superseded));
    assert_eq!(db.descriptor(&m).unwrap().method(), "cluster");
    assert!(matches!(
        db.supersede(&v1, "embed", 3, params(), &compute),
        Err(DerivedStaleError::Derived(DerivedError::IllegalTransition { .. }))
    ));
    // Superseding with the same identity is refused and changes nothing.
    assert!(matches!(
        db.supersede(&m, "cluster", 1, params(), &compute),
        Err(DerivedStaleError::Derived(DerivedError::Duplicate(_)))
    ));
    assert_eq!(db.state(&m), Some(DerivedState::Ready));

    // A change to the source leaves the superseded one superseded.
    edit(&db, a, b"a2");
    assert_eq!(db.state(&v1), Some(DerivedState::Superseded));
    assert_eq!(db.state(&m), Some(DerivedState::Stale));
    assert!(db.index_is_exact());
}

// ------------------------------------------------------------ reopening

fn reopening_rebuilds_the_registry_to_the_state_before_closing<E: Target>() {
    let disk = SimVfs::new(7);
    let db = DerivedDb::open(open::<E>(&disk), Vec::new(), &compute).unwrap();
    let (a, b, c) = (make(&db, b"a"), make(&db, b"b"), make(&db, b"c"));
    let ready = db.build([a, b], "embed", 1, params(), &compute).unwrap();
    let stale = db.build([c], "embed", 1, params(), &compute).unwrap();
    edit(&db, c, b"c2");
    assert_eq!(db.state(&ready), Some(DerivedState::Ready));
    assert_eq!(db.state(&stale), Some(DerivedState::Stale));

    let known: Vec<DerivedDescriptor> = [ready, stale].iter().map(|i| db.descriptor(i).unwrap()).collect();
    let value = db.value(&ready);
    db.close().unwrap();

    let db = DerivedDb::open(open::<E>(&disk), known.clone(), &compute).unwrap();
    assert_eq!(db.state(&ready), Some(DerivedState::Ready));
    assert_eq!(db.state(&stale), Some(DerivedState::Stale));
    assert_eq!(db.value(&ready), value, "the rebuilt value equals the one before closing");
    assert_eq!(db.value(&stale), None);
    assert!(db.index_is_exact());
    for d in &known {
        assert_eq!(db.descriptor(&d.id()).as_ref(), Some(d));
    }

    // The stale one is rebuilt after reopening like any other.
    let rebuilt = db.rebuild(&stale, &compute).unwrap();
    assert_eq!(db.state(&rebuilt), Some(DerivedState::Ready));
    db.close().unwrap();
}

fn a_datum_whose_input_changed_is_never_ready_after_reopening<E: Target>() {
    let disk = SimVfs::new(8);
    let db = DerivedDb::open(open::<E>(&disk), Vec::new(), &compute).unwrap();
    let a = make(&db, b"a");
    let first = db.build([a], "embed", 1, params(), &compute).unwrap();
    let old = db.descriptor(&first).unwrap();
    edit(&db, a, b"a2");
    let current = db.build([a], "embed", 2, params(), &compute).unwrap();
    let cur = db.descriptor(&current).unwrap();
    // Current sources, but another input hash than the data give.
    let forged_hash = DerivedDescriptor::new(cur.sources(), "forged", 1, params(), [9; 32]).unwrap();
    // The current input hash, but a source that is not the head revision.
    let old_source = DerivedDescriptor::new(old.sources(), "old-rev", 1, params(), *cur.input_hash()).unwrap();
    db.close().unwrap();

    let known = [cur.clone(), old.clone(), forged_hash.clone(), old_source.clone()];
    let db = DerivedDb::open(open::<E>(&disk), known, &compute).unwrap();
    assert_eq!(db.state(&current), Some(DerivedState::Ready), "the control is rebuilt");
    assert_eq!(db.state(&first), Some(DerivedState::Stale));
    assert_eq!(db.state(&forged_hash.id()), Some(DerivedState::Stale));
    assert_eq!(db.state(&old_source.id()), Some(DerivedState::Stale));
    for id in [first, forged_hash.id(), old_source.id()] {
        assert_eq!(db.value(&id), None, "a stale datum is not served");
    }
    db.close().unwrap();

    // A compute that fails on reopening leaves the datum failed, never ready.
    let db = DerivedDb::open(open::<E>(&disk), [cur.clone()], &never).unwrap();
    assert_eq!(db.state(&current), Some(DerivedState::Failed));
    assert_eq!(db.value(&current), None);
    db.close().unwrap();
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn a_change_to_a_source_marks_its_dependents_stale_in_the_same_changeset() {
                super::a_change_to_a_source_marks_its_dependents_stale_in_the_same_changeset::<E>();
            }
            #[test]
            fn a_changeset_that_fails_or_is_dropped_marks_nothing() {
                super::a_changeset_that_fails_or_is_dropped_marks_nothing::<E>();
            }
            #[test]
            fn rebuilding_gives_what_building_anew_gives() {
                super::rebuilding_gives_what_building_anew_gives::<E>();
            }
            #[test]
            fn rebuild_uses_only_the_sources_that_are_still_valid() {
                super::rebuild_uses_only_the_sources_that_are_still_valid::<E>();
            }
            #[test]
            fn a_failed_compute_leaves_the_datum_failed_and_a_later_rebuild_fixes_it() {
                super::a_failed_compute_leaves_the_datum_failed_and_a_later_rebuild_fixes_it::<E>();
            }
            #[test]
            fn a_new_version_creates_a_datum_and_supersedes_the_old_one() {
                super::a_new_version_creates_a_datum_and_supersedes_the_old_one::<E>();
            }
            #[test]
            fn reopening_rebuilds_the_registry_to_the_state_before_closing() {
                super::reopening_rebuilds_the_registry_to_the_state_before_closing::<E>();
            }
            #[test]
            fn a_datum_whose_input_changed_is_never_ready_after_reopening() {
                super::a_datum_whose_input_changed_is_never_ready_after_reopening::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
