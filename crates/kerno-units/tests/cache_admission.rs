//! E6-04b: what the planner stores in the result cache (etapa6-medicion.md).
//!
//! One entry fits per query shape, so a query that differs only in its
//! literals replaces the entry that is there. The planner asks
//! `ResultCache::worth_storing` before it copies a result: an entry that
//! served nothing is replaced by a newcomer only on the newcomer's second
//! sighting. Answers never depend on that choice (G1); only what is copied.
//!
//! * `a_stream_of_queries_that_never_repeat_copies_nothing` — the worst case
//!   of the acta (§6): after the first, no read stores or evicts.
//! * `a_query_that_comes_twice_is_stored_and_then_served` — the second
//!   sighting of a newcomer stores it and the third read hits.
//! * `a_slot_that_served_is_replaced_at_once` — the shape of W1: a query read
//!   several times at one snapshot, then the next one.

mod harness;

use harness::cache_script::{open, seed};
use harness::units::Target;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

/// The same shape (`unit_type = ?`) with a different literal each time.
fn of_type(t: &str) -> UnitFilter {
    UnitFilter { unit_type: Some(t.into()), schema_version: None }
}

fn served<E: Target>(snap: &UnitSnapshot<E>, f: &UnitFilter) -> bool {
    snap.query(f).unwrap().path == PlanPath::ResultCache
}

fn a_stream_of_queries_that_never_repeat_copies_nothing<E: Target>() {
    let (off, db) = (open::<E>(), open::<E>());
    seed(&off);
    seed(&db);
    db.result_cache().set_enabled(true);
    let (s_off, snap) = (off.snapshot().unwrap(), db.snapshot().unwrap());

    for (n, t) in ["doc", "img", "t1", "t2", "t3", "t4"].into_iter().enumerate() {
        let got = snap.query(&of_type(t)).unwrap();
        assert_eq!(got.units, s_off.query(&of_type(t)).unwrap().units, "{t}: same rows as with the cache off");
        assert_ne!(got.path, PlanPath::ResultCache, "{t}: nothing repeats");
        let stats = db.result_cache().stats();
        assert_eq!(
            (stats.writes, stats.evicted, stats.entries),
            (1, 0, 1),
            "after {n} reads past the first: {stats:?}"
        );
    }
    let log = db.result_cache().records();
    assert!(
        !log.iter().any(|r| matches!(r.transition, Transition::Deleted { reason: DeleteReason::Replaced })),
        "no entry was replaced"
    );
}

fn a_query_that_comes_twice_is_stored_and_then_served<E: Target>() {
    let db = open::<E>();
    seed(&db);
    db.result_cache().set_enabled(true);
    let snap = db.snapshot().unwrap();

    assert!(!served(&snap, &of_type("doc")), "stored: the slot was empty");
    assert!(!served(&snap, &of_type("img")), "first sighting: declined");
    assert_eq!(db.result_cache().stats().writes, 1);
    assert!(!served(&snap, &of_type("img")), "second sighting: a miss that stores");
    assert_eq!(db.result_cache().stats().writes, 2);
    assert!(served(&snap, &of_type("img")), "third read: served");
    assert!(!served(&snap, &of_type("doc")), "doc was replaced");
}

fn a_slot_that_served_is_replaced_at_once<E: Target>() {
    let db = open::<E>();
    seed(&db);
    db.result_cache().set_enabled(true);
    let snap = db.snapshot().unwrap();

    assert!(!served(&snap, &of_type("doc")));
    assert!(served(&snap, &of_type("doc")), "the entry earns its keep");
    assert!(!served(&snap, &of_type("img")), "the first read of the newcomer misses");
    assert_eq!(db.result_cache().stats().writes, 2, "and is stored at once");
    assert!(served(&snap, &of_type("img")));
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn a_stream_of_queries_that_never_repeat_copies_nothing() {
                super::a_stream_of_queries_that_never_repeat_copies_nothing::<E>();
            }
            #[test]
            fn a_query_that_comes_twice_is_stored_and_then_served() {
                super::a_query_that_comes_twice_is_stored_and_then_served::<E>();
            }
            #[test]
            fn a_slot_that_served_is_replaced_at_once() {
                super::a_slot_that_served_is_replaced_at_once::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
