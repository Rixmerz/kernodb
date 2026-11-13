//! ADR-0016 and ADR-0018 (D16 of etapa9-acta.md): a forget leaves no cached
//! answer servable. A forget is a commit, and a hit needs the exact commit
//! sequence of the entry, so the entry is invalidated by the commit itself.
//!
//! * `a_deleted_unit_is_not_served_from_the_cache` — the answer is cached and
//!   served (so the test is not vacuous), the unit is deleted, and the next
//!   read of a new snapshot comes from the reference path without the unit.
//! * `a_retention_purge_is_not_served_from_the_cache` — the same for the
//!   change set of a retention run.
//!
//! The deleted-unit test shows the behaviour (not served, forgotten unit absent);
//! other dependencies, such as the index watermark, also invalidate there. The
//! retention test is the one that isolates the commit sequence: it goes red
//! when the entry's commit sequence is forced constant.
//!
//! A snapshot taken *before* the forget still sees the unit, as every
//! snapshot does; it is served the entry of its own sequence, which is its
//! own state and not the forgotten one.

mod harness;

use harness::cache_script::{open, seed};
use harness::units::Target;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

fn everything() -> UnitFilter {
    UnitFilter::default()
}

/// Reads the same query twice so that the second read is a hit, and checks it.
fn warmed<E: Target>(db: &UnitDb<E>, f: &UnitFilter) -> Vec<Unit> {
    let snap = db.snapshot().unwrap();
    let first = snap.query(f).unwrap();
    assert_ne!(first.path, PlanPath::ResultCache, "the first read is a miss");
    let hit = snap.query(f).unwrap();
    assert_eq!(hit.path, PlanPath::ResultCache, "the second read is served from the cache");
    assert_eq!(hit.units, first.units);
    hit.units
}

fn a_deleted_unit_is_not_served_from_the_cache<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    db.result_cache().set_enabled(true);
    let f = everything();
    let before = warmed(&db, &f);
    assert!(before.iter().any(|u| u.id == ids[1]));
    let old = db.snapshot().unwrap();

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    // The unit has relations: they go first, as the layer requires.
    for r in db
        .snapshot()
        .unwrap()
        .query_relations(&RelationQuery { unit: ids[1], direction: Direction::Out, relation_type: None })
        .unwrap()
        .relations
    {
        cs.delete_relation(r.id).unwrap();
    }
    for r in db
        .snapshot()
        .unwrap()
        .query_relations(&RelationQuery { unit: ids[1], direction: Direction::In, relation_type: None })
        .unwrap()
        .relations
    {
        cs.delete_relation(r.id).unwrap();
    }
    cs.delete(ids[1], None).unwrap();
    cs.commit().unwrap();

    let after = db.snapshot().unwrap().query(&f).unwrap();
    assert_ne!(after.path, PlanPath::ResultCache, "the commit invalidated the entry");
    assert!(after.units.iter().all(|u| u.id != ids[1]), "the forgotten unit is not returned");
    // The snapshot from before keeps its own state.
    assert!(old.query(&f).unwrap().units.iter().any(|u| u.id == ids[1]));
}

fn a_retention_purge_is_not_served_from_the_cache<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    for n in 0..3u8 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let upd = UnitUpdate { payload: vec![n; 4], schema_version: None, metadata: None };
        cs.update(ids[0], upd, None).unwrap();
        cs.commit().unwrap();
    }
    db.result_cache().set_enabled(true);
    let f = everything();
    warmed(&db, &f);

    let policy = RetentionPolicy::all(RetentionRule::KeepLast(1));
    let report = db.run_retention(&policy, RetentionBudget::default()).unwrap();
    assert!(report.change_set.is_some(), "the run purged revisions in a commit");

    let after = db.snapshot().unwrap().query(&f).unwrap();
    assert_ne!(after.path, PlanPath::ResultCache, "the purge commit invalidated the entry");
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn a_deleted_unit_is_not_served_from_the_cache() {
                super::a_deleted_unit_is_not_served_from_the_cache::<E>();
            }
            #[test]
            fn a_retention_purge_is_not_served_from_the_cache() {
                super::a_retention_purge_is_not_served_from_the_cache::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
