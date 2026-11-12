//! E8-02: the in-memory registry of derived data (etapa8-acta.md §3 and §5,
//! ADR-0018, I-25, I-38).
//!
//! Most tests need no database: the registry is plain data. The one that
//! takes its sources from real commits runs on both engines.

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

const ALL: [DerivedState; 5] =
    [DerivedState::Ready, DerivedState::Building, DerivedState::Stale, DerivedState::Failed, DerivedState::Superseded];

fn rev(unit: u8, rev: u64) -> RevisionId {
    RevisionId { unit: UnitId([unit; 16]), rev: RevNo(rev) }
}

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn desc(sources: &[RevisionId], method: &str, version: u32) -> DerivedDescriptor {
    DerivedDescriptor::new(sources.iter().copied(), method, version, params(&[("dim", "8")]), [1; 32]).unwrap()
}

#[test]
fn same_sources_method_version_and_params_have_the_same_identity() {
    let a = desc(&[rev(1, 1), rev(2, 3)], "embed", 1);
    // Source order and repeats do not matter; the input hash is not identity.
    let b = DerivedDescriptor::new([rev(2, 3), rev(1, 1), rev(1, 1)], "embed", 1, params(&[("dim", "8")]), [9; 32])
        .unwrap();
    assert_eq!(a.id(), b.id());
    assert_ne!(a, b, "the descriptors differ in their input hash");
}

#[test]
fn each_identity_field_changes_the_identity() {
    let base = desc(&[rev(1, 1)], "embed", 1);
    let others = [
        desc(&[rev(1, 2)], "embed", 1),
        desc(&[rev(2, 1)], "embed", 1),
        desc(&[rev(1, 1), rev(2, 1)], "embed", 1),
        desc(&[rev(1, 1)], "summary", 1),
        desc(&[rev(1, 1)], "embed", 2),
        DerivedDescriptor::new([rev(1, 1)], "embed", 1, params(&[("dim", "16")]), [1; 32]).unwrap(),
        DerivedDescriptor::new([rev(1, 1)], "embed", 1, params(&[]), [1; 32]).unwrap(),
    ];
    for o in &others {
        assert_ne!(base.id(), o.id());
    }
}

#[test]
fn a_descriptor_needs_a_source_and_a_method() {
    assert_eq!(DerivedDescriptor::new([], "embed", 1, params(&[]), [0; 32]).unwrap_err(), DerivedError::EmptySources);
    assert_eq!(
        DerivedDescriptor::new([rev(1, 1)], "", 1, params(&[]), [0; 32]).unwrap_err(),
        DerivedError::EmptyMethod
    );
}

#[test]
fn the_state_machine_allows_exactly_the_documented_moves() {
    use DerivedState::*;
    let allowed = [
        (Building, Ready),
        (Building, Failed),
        (Building, Stale),
        (Building, Superseded),
        (Ready, Stale),
        (Ready, Superseded),
        (Stale, Building),
        (Stale, Superseded),
        (Failed, Building),
        (Failed, Superseded),
    ];
    for from in ALL {
        for to in ALL {
            assert_eq!(from.can_become(to), allowed.contains(&(from, to)), "{from:?} -> {to:?}");
        }
    }
}

/// Drives a fresh entry into `state` along allowed moves.
fn entry_in(reg: &mut DerivedRegistry, d: DerivedDescriptor, state: DerivedState) -> DerivedId {
    use DerivedState::*;
    let id = reg.register(d).unwrap();
    let path: &[DerivedState] = match state {
        Building => &[],
        Ready => &[Ready],
        Stale => &[Ready, Stale],
        Failed => &[Failed],
        Superseded => &[Superseded],
    };
    for s in path {
        reg.transition(&id, *s).unwrap();
    }
    assert_eq!(reg.state(&id), Some(state));
    id
}

#[test]
fn a_move_the_machine_forbids_is_an_error_and_changes_nothing() {
    for from in ALL {
        for to in ALL {
            if from.can_become(to) {
                continue;
            }
            let mut reg = DerivedRegistry::new();
            let id = entry_in(&mut reg, desc(&[rev(1, 1)], "embed", 1), from);
            assert_eq!(reg.transition(&id, to), Err(DerivedError::IllegalTransition { id, from, to }));
            assert_eq!(reg.state(&id), Some(from));
            assert!(reg.index_is_exact());
        }
    }
}

#[test]
fn an_allowed_move_changes_the_state() {
    for from in ALL {
        for to in ALL {
            if !from.can_become(to) {
                continue;
            }
            let mut reg = DerivedRegistry::new();
            let id = entry_in(&mut reg, desc(&[rev(1, 1)], "embed", 1), from);
            reg.transition(&id, to).unwrap();
            assert_eq!(reg.state(&id), Some(to));
        }
    }
}

#[test]
fn a_new_entry_is_building_and_never_ready() {
    let mut reg = DerivedRegistry::new();
    let id = reg.register(desc(&[rev(1, 1)], "embed", 1)).unwrap();
    assert_eq!(reg.state(&id), Some(DerivedState::Building));
}

#[test]
fn registering_twice_is_refused_and_keeps_the_first() {
    let mut reg = DerivedRegistry::new();
    let id = reg.register(desc(&[rev(1, 1)], "embed", 1)).unwrap();
    reg.transition(&id, DerivedState::Ready).unwrap();
    let again = DerivedDescriptor::new([rev(1, 1)], "embed", 1, params(&[("dim", "8")]), [7; 32]).unwrap();
    assert_eq!(reg.register(again), Err(DerivedError::Duplicate(id)));
    assert_eq!(reg.len(), 1);
    assert_eq!(reg.state(&id), Some(DerivedState::Ready));
    assert_eq!(reg.descriptor(&id).unwrap().input_hash(), &[1; 32]);
    assert!(reg.index_is_exact());
}

#[test]
fn an_unknown_id_is_an_error() {
    let mut reg = DerivedRegistry::new();
    let id = desc(&[rev(1, 1)], "embed", 1).id();
    assert_eq!(reg.transition(&id, DerivedState::Ready), Err(DerivedError::Unknown(id)));
    assert_eq!(reg.remove(&id), Err(DerivedError::Unknown(id)));
    assert_eq!(reg.state(&id), None);
}

#[test]
fn the_inverse_index_follows_register_transition_and_remove() {
    let mut reg = DerivedRegistry::new();
    let a = reg.register(desc(&[rev(1, 1), rev(2, 1)], "embed", 1)).unwrap();
    let b = reg.register(desc(&[rev(1, 1)], "summary", 1)).unwrap();
    let c = reg.register(desc(&[rev(1, 2)], "embed", 1)).unwrap();
    assert!(reg.index_is_exact());

    let sorted = |mut v: Vec<DerivedId>| {
        v.sort();
        v
    };
    assert_eq!(sorted(reg.dependents_of(rev(1, 1))), sorted(vec![a, b]));
    assert_eq!(reg.dependents_of(rev(2, 1)), vec![a]);
    assert_eq!(reg.dependents_of(rev(1, 2)), vec![c]);
    assert!(reg.dependents_of(rev(1, 3)).is_empty());
    assert_eq!(sorted(reg.dependents_of_unit(UnitId([1; 16]))), sorted(vec![a, b, c]));
    assert_eq!(reg.dependents_of_unit(UnitId([2; 16])), vec![a]);
    assert!(reg.dependents_of_unit(UnitId([3; 16])).is_empty());

    // A state change does not touch the index: a superseded entry is still
    // registered, and still depends on its sources.
    reg.transition(&a, DerivedState::Superseded).unwrap();
    assert!(reg.index_is_exact());
    assert_eq!(reg.dependents_of(rev(2, 1)), vec![a]);

    // Removing leaves exactly the others, and no empty slot behind.
    let removed = reg.remove(&a).unwrap();
    assert_eq!(removed.id(), a);
    assert!(reg.index_is_exact());
    assert!(reg.dependents_of(rev(2, 1)).is_empty());
    assert_eq!(reg.dependents_of(rev(1, 1)), vec![b]);
    assert_eq!(reg.dependents_of_unit(UnitId([2; 16])), Vec::<DerivedId>::new());

    reg.remove(&b).unwrap();
    reg.remove(&c).unwrap();
    assert!(reg.is_empty());
    assert!(reg.index_is_exact());
    assert!(reg.dependents_of_unit(UnitId([1; 16])).is_empty());
}

#[test]
fn a_changed_method_or_version_is_a_new_entry_beside_the_old_one() {
    let mut reg = DerivedRegistry::new();
    let old = reg.register(desc(&[rev(1, 1)], "embed", 1)).unwrap();
    reg.transition(&old, DerivedState::Ready).unwrap();
    let new = reg.register(desc(&[rev(1, 1)], "embed", 2)).unwrap();
    assert_ne!(old, new);
    reg.transition(&old, DerivedState::Superseded).unwrap();
    assert_eq!(reg.state(&old), Some(DerivedState::Superseded));
    assert_eq!(reg.state(&new), Some(DerivedState::Building));
    assert_eq!(reg.dependents_of(rev(1, 1)).len(), 2);
}

// ------------------------------------------------------- real revisions, both engines

fn open<E: Target>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    open_units::<E>(SimVfs::new(1).as_vfs(), &c).expect("open")
}

fn registry_follows_the_revisions_a_database_commits<E: Target>() {
    let db = open::<E>();
    let new = |p: &[u8]| NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: p.to_vec() };
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let a = cs.create(new(b"a")).unwrap();
    let b = cs.create(new(b"b")).unwrap();
    let first = cs.commit().unwrap();
    assert_eq!(first.revisions.len(), 2);

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let upd = UnitUpdate { payload: b"a2".to_vec(), schema_version: None, metadata: None };
    cs.update(a, upd, None).unwrap();
    let second = cs.commit().unwrap();
    assert_eq!(second.revisions.len(), 1);

    let mut reg = DerivedRegistry::new();
    let d = DerivedDescriptor::new(first.revisions.iter().copied(), "embed", 1, params(&[]), [3; 32]).unwrap();
    let id = reg.register(d).unwrap();
    for r in &first.revisions {
        assert_eq!(reg.dependents_of(*r), vec![id]);
    }
    // The new revision of `a` is not a source of the old derived datum, but
    // the unit still is: a unit-level lookup finds it.
    assert!(reg.dependents_of(second.revisions[0]).is_empty());
    assert_eq!(reg.dependents_of_unit(a), vec![id]);
    assert_eq!(reg.dependents_of_unit(b), vec![id]);
    assert!(reg.index_is_exact());
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn registry_follows_the_revisions_a_database_commits() {
                super::registry_follows_the_revisions_a_database_commits::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
