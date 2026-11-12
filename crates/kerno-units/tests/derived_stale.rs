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
use kerno_testkit::vfs::{OpKind, SimVfs};
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
    // The refusal left nothing behind: the datum it would have made is new.
    let v3 = db.build([a], "embed", 3, params(), &compute).unwrap();
    assert_eq!(db.state(&v3), Some(DerivedState::Ready));
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

// ------------------------------------------------------------ edge cases

fn rebuild_refuses_an_identity_that_already_exists_and_keeps_the_old_entry<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let old = db.build([a], "embed", 1, params(), &compute).unwrap();
    edit(&db, a, b"a2");
    // The same data, built anew while `old` is still stale: the identity a
    // rebuild of `old` would get is taken.
    let anew = db.build([a], "embed", 1, params(), &compute).unwrap();
    assert_ne!(old, anew);

    let before = db.descriptor(&old);
    assert!(
        matches!(db.rebuild(&old, &compute), Err(DerivedStaleError::Derived(DerivedError::Duplicate(d))) if d == anew)
    );
    assert_eq!(db.state(&old), Some(DerivedState::Stale), "the refused rebuild did not lose the entry");
    assert_eq!(db.descriptor(&old), before);
    assert_eq!(db.state(&anew), Some(DerivedState::Ready));
    assert_eq!(db.value(&anew).as_deref(), Some(&b"embed@1:a2|"[..]));
}

fn superseding_with_no_valid_source_is_refused_and_changes_nothing<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let v1 = db.build([a], "embed", 1, params(), &compute).unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(a, None).unwrap();
    cs.commit().unwrap();

    assert!(
        matches!(db.supersede(&v1, "embed", 2, params(), &compute), Err(DerivedStaleError::NoValidSources(i)) if i == v1)
    );
    assert_eq!(db.state(&v1), Some(DerivedState::Stale), "the earlier datum is not superseded");
}

fn a_datum_left_building_by_a_panic_goes_stale_with_its_source<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let seen = std::cell::Cell::new(None);
    let panics = |d: &DerivedDescriptor, _: &[SourceInput]| -> Option<Vec<u8>> {
        seen.set(Some(d.id()));
        panic!("the compute function failed")
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| db.build([a], "embed", 1, params(), &panics)));
    assert!(r.is_err());
    let id = seen.get().expect("compute ran");
    // The registry lock was poisoned by the panic and is still usable.
    assert_eq!(db.state(&id), Some(DerivedState::Building));

    edit(&db, a, b"a2");
    assert_eq!(db.state(&id), Some(DerivedState::Stale));
    let rebuilt = db.rebuild(&id, &compute).unwrap();
    assert_eq!(db.state(&rebuilt), Some(DerivedState::Ready));
}

fn a_failed_datum_stays_failed_when_its_source_changes<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let failed = match db.build([a], "embed", 1, params(), &never).unwrap_err() {
        DerivedStaleError::ComputeFailed(f) => f,
        other => panic!("unexpected {other}"),
    };
    edit(&db, a, b"a2");
    assert_eq!(db.state(&failed), Some(DerivedState::Failed));
}

fn build_normalises_the_unit_set_and_refuses_a_missing_source<E: Target>() {
    let db = fresh::<E>();
    let (a, b) = (make(&db, b"a"), make(&db, b"b"));
    let first = db.build([a, b], "embed", 1, params(), &compute).unwrap();
    // The same set in another order, with a repeat, is the same datum.
    assert!(matches!(
        db.build([b, a, a], "embed", 1, params(), &compute),
        Err(DerivedStaleError::Derived(DerivedError::Duplicate(d))) if d == first
    ));
    assert_eq!(db.state(&first), Some(DerivedState::Ready));

    let nobody = UnitId([0xEE; 16]);
    assert!(
        matches!(db.build([a, nobody], "x", 1, params(), &compute), Err(DerivedStaleError::MissingSource(u)) if u == nobody)
    );
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(b, None).unwrap();
    cs.commit().unwrap();
    assert!(matches!(db.build([a, b], "x", 1, params(), &compute), Err(DerivedStaleError::MissingSource(u)) if u == b));
}

// ------------------------------------------------------------ method, version and params

fn kv(k: &str, v: &str) -> BTreeMap<String, String> {
    BTreeMap::from([(k.to_string(), v.to_string())])
}

fn rebuild_keeps_method_version_and_params<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let old = db.build([a], "cluster", 7, kv("k", "5"), &compute).unwrap();
    edit(&db, a, b"a2");
    let rebuilt = db.rebuild(&old, &compute).unwrap();
    let d = db.descriptor(&rebuilt).unwrap();
    assert_eq!((d.method(), d.version(), d.params()), ("cluster", 7, &kv("k", "5")));
    assert_eq!(db.value(&rebuilt).as_deref(), Some(&b"cluster@7:a2|"[..]));
    // The identity is the one a new build with the same method, version and
    // params gets.
    let other = fresh::<E>();
    let a2 = make(&other, b"a");
    edit(&other, a2, b"a2");
    assert_eq!(other.build([a2], "cluster", 7, kv("k", "5"), &compute).unwrap(), rebuilt);
}

fn build_distinguishes_params_and_method<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let base = db.build([a], "embed", 1, params(), &compute).unwrap();
    // Differing only in params, and only in method, is another datum, not a duplicate.
    let with_params = db.build([a], "embed", 1, kv("k", "v"), &compute).unwrap();
    let with_method = db.build([a], "summary", 1, params(), &compute).unwrap();
    assert_ne!(base, with_params);
    assert_ne!(base, with_method);
    assert_ne!(with_params, with_method);
    assert_eq!(db.descriptor(&with_params).unwrap().params(), &kv("k", "v"));
    assert_eq!(db.descriptor(&with_method).unwrap().method(), "summary");
    for id in [base, with_params, with_method] {
        assert_eq!(db.state(&id), Some(DerivedState::Ready));
    }
}

fn supersede_distinguishes_params_and_method<E: Target>() {
    // Only the params differ: the new datum is not the old one.
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let v1 = db.build([a], "embed", 1, params(), &compute).unwrap();
    let p = db.supersede(&v1, "embed", 1, kv("k", "v"), &compute).unwrap();
    assert_ne!(v1, p);
    assert_eq!(db.state(&v1), Some(DerivedState::Superseded));
    assert_eq!(db.descriptor(&p).unwrap().params(), &kv("k", "v"));
    assert_eq!(db.state(&p), Some(DerivedState::Ready));

    // Only the method differs.
    let db = fresh::<E>();
    let a = make(&db, b"a");
    let v1 = db.build([a], "embed", 1, params(), &compute).unwrap();
    let m = db.supersede(&v1, "summary", 1, params(), &compute).unwrap();
    assert_ne!(v1, m);
    assert_eq!(db.state(&v1), Some(DerivedState::Superseded));
    assert_eq!(db.descriptor(&m).unwrap().method(), "summary");
    assert_eq!(db.state(&m), Some(DerivedState::Ready));
}

fn a_first_build_normalises_the_unit_set<E: Target>() {
    let one = fresh::<E>();
    let (a, b) = (make(&one, b"a"), make(&one, b"b"));
    let ordered = one.build([a, b], "embed", 1, params(), &compute).unwrap();
    let other = fresh::<E>();
    let (a2, b2) = (make(&other, b"a"), make(&other, b"b"));
    assert_eq!((a, b), (a2, b2));
    // Another order and a repeat, on the first build: the same datum.
    let shuffled = other.build([b2, a2, b2, a2], "embed", 1, params(), &compute).unwrap();
    assert_eq!(ordered, shuffled, "same identity");
    assert_eq!(one.descriptor(&ordered), other.descriptor(&shuffled), "same input hash and sources");
    assert_eq!(one.value(&ordered), other.value(&shuffled));
    assert_eq!(other.value(&shuffled).as_deref(), Some(&b"embed@1:a|b|"[..]));
}

// ------------------------------------------------------------ several units

fn one_changeset_changing_several_units_marks_every_dependent_stale<E: Target>() {
    let db = fresh::<E>();
    let (a, b, c) = (make(&db, b"a"), make(&db, b"b"), make(&db, b"c"));
    let on_a = db.build([a], "embed", 1, params(), &compute).unwrap();
    let on_b = db.build([b], "embed", 1, params(), &compute).unwrap();
    let on_ab = db.build([a, b], "summary", 1, params(), &compute).unwrap();
    let on_c = db.build([c], "embed", 1, params(), &compute).unwrap();

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for (u, p) in [(a, &b"a2"[..]), (b, &b"b2"[..])] {
        cs.update(u, UnitUpdate { payload: p.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    }
    cs.commit().unwrap();
    for id in [on_a, on_b, on_ab] {
        assert_eq!(db.state(&id), Some(DerivedState::Stale));
        assert_eq!(db.value(&id), None);
    }
    assert_eq!(db.state(&on_c), Some(DerivedState::Ready));
}

/// A commit that really fails marks nothing: the data sync of the commit is
/// made to fail on the simulated disk, so the engine returns an error from
/// `commit()` and the derived datum must still be ready.
fn a_commit_that_fails_marks_nothing<E: Target>() {
    let disk = SimVfs::new(21);
    let db = DerivedDb::open(open::<E>(&disk), Vec::new(), &compute).unwrap();
    let (a, b) = (make(&db, b"a"), make(&db, b"b"));
    let on_a = db.build([a], "embed", 1, params(), &compute).unwrap();
    let on_b = db.build([b], "embed", 1, params(), &compute).unwrap();

    let update = |p: &[u8]| UnitUpdate { payload: p.to_vec(), schema_version: None, metadata: None };
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(a, update(b"a2"), None).unwrap();
    disk.fail_when(|op| op.kind == OpKind::SyncData, 1, None);
    let result = cs.commit();
    assert!(disk.fault_injected(), "the fault did not fire");
    assert!(result.is_err(), "a failed data sync went unreported");

    assert_eq!(db.state(&on_a), Some(DerivedState::Ready));
    assert_eq!(db.state(&on_b), Some(DerivedState::Ready));
    assert_eq!(db.value(&on_a).as_deref(), Some(&b"embed@1:a|"[..]));
}

fn a_dependent_on_a_later_revision_goes_stale_with_the_next_change<E: Target>() {
    let db = fresh::<E>();
    let a = make(&db, b"a");
    edit(&db, a, b"a2");
    edit(&db, a, b"a3");
    // Built over revision 3, not the first revision.
    let id = db.build([a], "embed", 1, params(), &compute).unwrap();
    assert_eq!(db.state(&id), Some(DerivedState::Ready));
    edit(&db, a, b"a4");
    assert_eq!(db.state(&id), Some(DerivedState::Stale));
    let rebuilt = db.rebuild(&id, &compute).unwrap();
    assert_eq!(db.value(&rebuilt).as_deref(), Some(&b"embed@1:a4|"[..]));
}

// ------------------------------------------------------------ open

fn open_refuses_a_duplicate_descriptor_in_the_known_list<E: Target>() {
    let disk = SimVfs::new(9);
    let db = DerivedDb::open(open::<E>(&disk), Vec::new(), &compute).unwrap();
    let a = make(&db, b"a");
    let id = db.build([a], "embed", 1, params(), &compute).unwrap();
    let d = db.descriptor(&id).unwrap();
    db.close().unwrap();

    match DerivedDb::open(open::<E>(&disk), [d.clone(), d], &compute) {
        Err(DerivedStaleError::Derived(DerivedError::Duplicate(dup))) => assert_eq!(dup, id),
        Err(other) => panic!("unexpected {other}"),
        Ok(_) => panic!("a duplicate descriptor was accepted"),
    }
}

// ------------------------------------------------------------ input hash

fn source(unit: u8, rev: u64, payload: &[u8], digest: u8) -> SourceInput {
    SourceInput {
        id: RevisionId { unit: UnitId([unit; 16]), rev: RevNo(rev) },
        content_hash: ContentHash { alg: HashAlg::Blake3, digest: [digest; 32] },
        payload: payload.to_vec(),
    }
}

#[test]
fn the_input_hash_is_the_documented_encoding_of_unit_and_revision_per_source() {
    let inputs = [source(1, 3, b"x", 0), source(2, 5, b"y", 0)];
    let mut h = blake3::Hasher::new();
    h.update(b"kerno.derived.input.v1");
    h.update(&2u64.to_le_bytes());
    for (u, r) in [(1u8, 3u64), (2, 5)] {
        h.update(&[u; 16]);
        h.update(&r.to_le_bytes());
    }
    assert_eq!(input_hash(&inputs), *h.finalize().as_bytes());
}

#[test]
fn the_input_hash_tells_apart_revision_unit_order_and_count_but_not_the_payload() {
    let base = input_hash(&[source(1, 3, b"x", 0), source(2, 5, b"y", 0)]);
    let others = [
        input_hash(&[source(1, 4, b"x", 0), source(2, 5, b"y", 0)]),
        input_hash(&[source(1, 3, b"x", 0), source(2, 6, b"y", 0)]),
        input_hash(&[source(9, 3, b"x", 0), source(2, 5, b"y", 0)]),
        input_hash(&[source(2, 5, b"y", 0), source(1, 3, b"x", 0)]),
        input_hash(&[source(1, 3, b"x", 0)]),
        input_hash(&[]),
    ];
    for (i, o) in others.iter().enumerate() {
        assert_ne!(&base, o, "variant {i}");
    }
    // A revision is immutable and names its content: payload and content hash
    // carry nothing more.
    assert_eq!(base, input_hash(&[source(1, 3, b"other", 7), source(2, 5, b"y", 8)]));
}

#[test]
fn a_source_input_shows_no_content_when_debug_printed() {
    let text = format!("{:?}", source(1, 3, b"SECRET-PAYLOAD", 0));
    assert!(!text.contains("SECRET"), "{text}");
    assert!(text.contains("payload_len"), "{text}");
}

#[test]
fn every_error_says_what_it_is_without_naming_content() {
    let id = DerivedId([7; 32]);
    let inner = DerivedError::Unknown(id);
    let unit = UnitError::ChangeSetClosed;
    let cases = [
        (DerivedStaleError::Derived(inner.clone()), inner.to_string()),
        (DerivedStaleError::Unit(UnitError::ChangeSetClosed), unit.to_string()),
        (DerivedStaleError::MissingSource(UnitId([1; 16])), "a source unit does not exist or is deleted".into()),
        (DerivedStaleError::NoValidSources(id), "no source of the derived datum is valid any more".into()),
        (DerivedStaleError::ComputeFailed(id), "the derived value could not be computed".into()),
    ];
    for (e, text) in &cases {
        assert_eq!(&e.to_string(), text);
    }
    assert_eq!(cases[0].1, "no derived datum has this identity");
    assert_eq!(cases[1].1, "the change set is closed");
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
            #[test]
            fn rebuild_refuses_an_identity_that_already_exists_and_keeps_the_old_entry() {
                super::rebuild_refuses_an_identity_that_already_exists_and_keeps_the_old_entry::<E>();
            }
            #[test]
            fn superseding_with_no_valid_source_is_refused_and_changes_nothing() {
                super::superseding_with_no_valid_source_is_refused_and_changes_nothing::<E>();
            }
            #[test]
            fn a_datum_left_building_by_a_panic_goes_stale_with_its_source() {
                super::a_datum_left_building_by_a_panic_goes_stale_with_its_source::<E>();
            }
            #[test]
            fn build_normalises_the_unit_set_and_refuses_a_missing_source() {
                super::build_normalises_the_unit_set_and_refuses_a_missing_source::<E>();
            }
            #[test]
            fn a_failed_datum_stays_failed_when_its_source_changes() {
                super::a_failed_datum_stays_failed_when_its_source_changes::<E>();
            }
            #[test]
            fn rebuild_keeps_method_version_and_params() {
                super::rebuild_keeps_method_version_and_params::<E>();
            }
            #[test]
            fn build_distinguishes_params_and_method() {
                super::build_distinguishes_params_and_method::<E>();
            }
            #[test]
            fn supersede_distinguishes_params_and_method() {
                super::supersede_distinguishes_params_and_method::<E>();
            }
            #[test]
            fn a_first_build_normalises_the_unit_set() {
                super::a_first_build_normalises_the_unit_set::<E>();
            }
            #[test]
            fn a_commit_that_fails_marks_nothing() {
                super::a_commit_that_fails_marks_nothing::<E>();
            }
            #[test]
            fn one_changeset_changing_several_units_marks_every_dependent_stale() {
                super::one_changeset_changing_several_units_marks_every_dependent_stale::<E>();
            }
            #[test]
            fn open_refuses_a_duplicate_descriptor_in_the_known_list() {
                super::open_refuses_a_duplicate_descriptor_in_the_known_list::<E>();
            }
            #[test]
            fn a_dependent_on_a_later_revision_goes_stale_with_the_next_change() {
                super::a_dependent_on_a_later_revision_goes_stale_with_the_next_change::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
