//! E6-03: the result cache is reversible (etapa6-acta.md §3 and §4, ADR-0016
//! G6, I-34 for optimizations, I-36).
//!
//! The learned state is derived data kept outside every transaction, so
//! turning it on or off, invalidating it or deleting it must leave what the
//! user committed exactly as it was. "As it was" is checked three ways at
//! once: the engine's key/value pairs, the engine's `commit_seq`, and the
//! logical state read through the public API (answers, revisions, change
//! sets, index status).
//!
//! * `every_operation_on_the_cache_leaves_the_data_as_it_was` — enable,
//!   disable, per-query switches, delete, key change, bound change and clear,
//!   each applied to a warm cache and compared before and after.
//! * `invalidating_by_each_dependency_leaves_the_data_as_it_was` — a lookup
//!   whose current versions differ in each declared dependency drops the
//!   entry and touches nothing else.
//! * `a_cache_in_use_never_changes_what_a_commit_writes` — two databases run
//!   the same script, one with the cache on and its entries invalidated by
//!   every write; their engines are equal after every round.
//! * `with_the_cache_off_no_optimization_takes_part` — the default: a whole
//!   script of reads and writes leaves every counter and the log at zero.
//! * `a_cache_switched_off_after_use_stops_taking_part` — the same once the
//!   cache has served: the counters stop where the switch found them.

mod harness;

use std::time::Duration;

use harness::cache_script::{open, queries, relation_queries, seed, write};
use harness::units::{Dump, Target, raw, seq, verified};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

/// What the user can read through the public API, whichever path answered.
#[derive(Debug, PartialEq)]
struct Logical {
    units: Vec<Vec<Unit>>,
    relations: Vec<Vec<Relation>>,
    histories: Vec<Vec<Revision>>,
    change_sets: Vec<ChangeSetInfo>,
    index: IndexStatus,
}

/// Everything the data consists of: the engine's pairs and sequence, and the
/// logical state.
#[derive(Debug, PartialEq)]
struct Frozen {
    bytes: Dump,
    commit_seq: u64,
    logical: Logical,
}

fn freeze<E: Target>(db: &UnitDb<E>, ids: &[UnitId]) -> Frozen {
    let snap = db.snapshot().unwrap();
    let logical = Logical {
        units: queries().iter().map(|f| snap.query(f).unwrap().units).collect(),
        relations: relation_queries(ids).iter().map(|q| snap.query_relations(q).unwrap().relations).collect(),
        histories: ids.iter().map(|id| snap.history(*id, HistoryQuery::oldest(100)).unwrap()).collect(),
        change_sets: snap.change_sets_since(CommitSeq(0), 1000).unwrap(),
        index: snap.index_status().unwrap(),
    };
    assert_eq!(snap.seq().0, seq(db), "the snapshot is at the engine's sequence");
    Frozen { bytes: raw(db), commit_seq: seq(db), logical }
}

/// A database with history (several revisions, relations, an index built at
/// the last commit) so that every kind of committed data exists.
fn populated<E: Target>() -> (UnitDb<E>, Vec<UnitId>) {
    let db = open::<E>();
    let ids = seed(&db);
    for round in 0..5 {
        write(&db, &ids, round); // the fifth rebuilds the index
    }
    (db, ids)
}

/// Reads every query twice at one snapshot, so that the second read of each
/// can hit.
fn read_twice<E: Target>(db: &UnitDb<E>, ids: &[UnitId]) {
    let snap = db.snapshot().unwrap();
    for _ in 0..2 {
        for f in queries() {
            snap.query(&f).unwrap();
        }
        for q in relation_queries(ids) {
            snap.query_relations(&q).unwrap();
        }
    }
}

fn warm<E: Target>(db: &UnitDb<E>, ids: &[UnitId]) {
    db.observer().set_level(ObservationLevel::Fingerprint).unwrap(); // evidence for an entry
    db.result_cache().set_enabled(true);
    read_twice(db, ids);
    let stats = db.result_cache().stats();
    assert!(stats.entries > 0 && stats.hits > 0, "not vacuous: the cache holds entries and has served: {stats:?}");
}

type Operation<E> = (&'static str, bool, Box<dyn Fn(&UnitDb<E>)>);

/// Each operation of ADR-0016 G6, with whether it starts from a warm cache
/// (the enable operation starts from the default, off).
fn operations<E: Target>() -> Vec<Operation<E>> {
    let fp = |db: &UnitDb<E>| db.result_cache().fingerprint_of(&CachedQuery::Units(queries()[1].clone())).unwrap();
    vec![
        ("enable", false, Box::new(|db| db.result_cache().set_enabled(true))),
        ("disable", true, Box::new(|db| db.result_cache().set_enabled(false))),
        ("disable a query", true, Box::new(move |db| db.result_cache().disable_query(fp(db)))),
        (
            "enable a query again",
            true,
            Box::new(move |db| {
                db.result_cache().disable_query(fp(db));
                db.result_cache().enable_query(fp(db));
            }),
        ),
        (
            "delete an entry",
            true,
            Box::new(move |db| {
                assert!(db.result_cache().delete(&fp(db)), "there was an entry to delete");
            }),
        ),
        (
            "replace the fingerprint key",
            true,
            Box::new(|db| db.result_cache().set_key(FingerprintKey::from_bytes([7; 32]))),
        ),
        (
            "shrink the bounds",
            true,
            Box::new(|db| {
                let config = CacheConfig { max_entries: 1, ..CacheConfig::default() };
                db.result_cache().set_config(config).unwrap();
            }),
        ),
        ("clear", true, Box::new(|db| db.result_cache().clear())),
    ]
}

fn every_operation_on_the_cache_leaves_the_data_as_it_was<E: Target>() {
    for (name, starts_warm, operation) in operations::<E>() {
        let (db, ids) = populated::<E>();
        if starts_warm {
            warm(&db, &ids);
        }
        let before = freeze(&db, &ids);
        let entries = db.result_cache().stats().entries;

        operation(&db);
        // Before the freeze below reads through the cache and fills it again.
        let stats = db.result_cache().stats();
        let after = freeze(&db, &ids);
        assert_eq!(before.commit_seq, after.commit_seq, "{name}: commit_seq");
        assert_eq!(before.bytes, after.bytes, "{name}: engine bytes");
        assert_eq!(before.logical, after.logical, "{name}: logical state");
        assert_eq!(before, after, "{name}");

        // The operation did act on the learned state, so the equality above
        // is not about an operation that did nothing.
        match name {
            "enable" => assert!(stats.enabled),
            "disable" => assert!(!stats.enabled && stats.entries == 0, "{name}: {stats:?}"),
            "delete an entry" | "shrink the bounds" => assert!(stats.entries < entries, "{name}: {stats:?}"),
            "clear" | "replace the fingerprint key" => assert_eq!(stats.entries, 0, "{name}"),
            _ => {}
        }

        // Reading after the operation (rebuilding, serving or refusing) is
        // not a write either, and the store is still consistent.
        read_twice(&db, &ids);
        assert_eq!(freeze(&db, &ids), after, "{name}: reading afterwards");
        verified(&db, name);
    }
}

fn invalidating_by_each_dependency_leaves_the_data_as_it_was<E: Target>() {
    // Spelled out, not `Dependency::ALL`: a test that walks the list it guards
    // cannot notice a dependency missing from it.
    let each = [
        Dependency::SchemaVersion,
        Dependency::PlannerVersion,
        Dependency::IndexState,
        Dependency::IndexWatermark,
        Dependency::CommitSeq,
    ];
    assert_eq!(each.as_slice(), Dependency::ALL.as_slice(), "every declared dependency is covered, in order");
    for dependency in each {
        let (db, ids) = populated::<E>();
        let before = freeze(&db, &ids);
        db.result_cache().set_enabled(true);
        let snap = db.snapshot().unwrap();
        let status = snap.index_status().unwrap();
        assert!(status.watermark.is_some(), "an index was built");
        let query = CachedQuery::Units(queries()[1].clone());
        let cache = db.result_cache();
        let fp = cache.fingerprint_of(&query).unwrap();
        let deps = Dependencies::current(snap.seq(), Some(status));
        let rows = snap.query(&queries()[1]).unwrap().units;
        let evidence = Evidence { observations: 1, window: Duration::ZERO };
        cache
            .create(CacheEntry::new(fp, query.clone(), deps, CachedResult::Units(rows), evidence, 1_000).unwrap())
            .unwrap();
        cache.activate(&fp).unwrap();

        // The versions in force now differ from the entry's in this one.
        let mut now = deps;
        match dependency {
            Dependency::SchemaVersion => now.schema_version += 1,
            Dependency::PlannerVersion => now.planner_version += 1,
            Dependency::IndexState => now.index = Some(IndexStatus { state: IndexState::Stale, ..status }),
            Dependency::IndexWatermark => {
                now.index = Some(IndexStatus { watermark: status.watermark.map(|w| CommitSeq(w.0 + 1)), ..status });
            }
            Dependency::CommitSeq => now.commit_seq = CommitSeq(deps.commit_seq.0 + 1),
        }
        let looked = cache.lookup(&fp, &query, &now);
        assert!(matches!(looked, Lookup::Invalidated(d) if d == dependency), "{dependency}: not invalidated by it");
        assert_eq!(cache.stats().entries, 0, "{dependency}: the entry is gone");
        assert!(
            cache.records().iter().any(|r| r.transition == Transition::Invalidated { dependency }),
            "{dependency}: the invalidation is on the record"
        );

        assert_eq!(freeze(&db, &ids), before, "{dependency}: invalidating touched the data");
    }
}

fn a_cache_in_use_never_changes_what_a_commit_writes<E: Target>() {
    let (plain, with_cache) = (open::<E>(), open::<E>());
    let ids = seed(&plain);
    assert_eq!(seed(&with_cache), ids, "identical databases");
    with_cache.observer().set_level(ObservationLevel::Fingerprint).unwrap(); // evidence for an entry
    with_cache.result_cache().set_enabled(true);

    let mut invalidated = false;
    for round in 0..10 {
        // Fill the cache, so that the write below finds entries to invalidate.
        read_twice(&with_cache, &ids);
        write(&plain, &ids, round);
        write(&with_cache, &ids, round);
        // The next read finds every entry invalid and replaces it.
        read_twice(&with_cache, &ids);
        invalidated |= with_cache.result_cache().stats().invalidations > 0;

        assert_eq!(seq(&with_cache), seq(&plain), "round {round}: commit_seq");
        assert_eq!(raw(&with_cache), raw(&plain), "round {round}: engine bytes");
        // Clearing in the middle of the script changes nothing either.
        if round % 3 == 2 {
            with_cache.result_cache().clear();
            assert_eq!(raw(&with_cache), raw(&plain), "round {round}: engine bytes after clear");
        }
    }
    assert!(invalidated, "not vacuous: writes invalidated entries of the cache");
    assert_eq!(freeze(&with_cache, &ids), freeze(&plain, &ids));
    verified(&with_cache, "cache in use");
}

/// What one read gave and how.
struct Seen {
    answer: (Vec<Unit>, Vec<Relation>),
    path: PlanPath,
    fallback: Option<Fallback>,
    cache_source: bool,
}

/// Reads and writes of the whole script; the cache is whatever the caller set.
fn run_script<E: Target>(db: &UnitDb<E>, ids: &[UnitId]) -> Vec<Seen> {
    let mut seen = Vec::new();
    for round in 0..10 {
        let snap = db.snapshot().unwrap();
        for _ in 0..2 {
            for f in queries() {
                let o = snap.query(&f).unwrap();
                let cache_source = matches!(o.source, QuerySource::Cache { .. });
                seen.push(Seen { answer: (o.units, Vec::new()), path: o.path, fallback: o.fallback, cache_source });
            }
            for q in relation_queries(ids) {
                let o = snap.query_relations(&q).unwrap();
                let answer = (Vec::new(), o.relations);
                seen.push(Seen { answer, path: o.path, fallback: o.fallback, cache_source: false });
            }
        }
        write(db, ids, round);
    }
    seen
}

fn answers(seen: &[Seen]) -> Vec<&(Vec<Unit>, Vec<Relation>)> {
    seen.iter().map(|s| &s.answer).collect()
}

fn none_took_part(seen: &[Seen]) {
    assert!(!seen.is_empty());
    for s in seen {
        assert_ne!(s.path, PlanPath::ResultCache);
        assert!(!s.cache_source);
        assert!(s.fallback.is_none(), "nothing was abandoned: {:?}", s.fallback);
    }
}

fn with_the_cache_off_no_optimization_takes_part<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    assert!(!db.result_cache().enabled(), "off by default");

    none_took_part(&run_script(&db, &ids));
    // The counters of uses, reads and writes of the cache stay at zero, and
    // so does its log: no optimization took part.
    assert_eq!(db.result_cache().stats(), CacheStats::default());
    assert!(db.result_cache().records().is_empty());

    // Asked directly, a cache that is off reads nothing and says so.
    let cache = db.result_cache();
    let query = CachedQuery::Units(queries()[1].clone());
    let fp = cache.fingerprint_of(&query).unwrap();
    let snap = db.snapshot().unwrap();
    let now = Dependencies::current(snap.seq(), Some(snap.index_status().unwrap()));
    assert!(matches!(cache.lookup(&fp, &query, &now), Lookup::Disabled));
    assert_eq!(cache.stats(), CacheStats::default(), "no counter of use moved");
    assert!(cache.records().is_empty());
    verified(&db, "cache off");
}

fn a_cache_switched_off_after_use_stops_taking_part<E: Target>() {
    let (never, db) = (open::<E>(), open::<E>());
    let ids = seed(&never);
    assert_eq!(seed(&db), ids);
    warm(&db, &ids);
    let baseline = run_script(&never, &ids);

    // Globally.
    db.result_cache().set_enabled(false);
    let frozen = db.result_cache().stats();
    assert_eq!((frozen.entries, frozen.enabled), (0, false));
    let used = db.result_cache().records().len();
    let outcomes = run_script(&db, &ids);
    assert_eq!(db.result_cache().stats(), frozen, "no counter moved while it was off");
    assert_eq!(db.result_cache().records().len(), used, "nothing was recorded while it was off");
    none_took_part(&outcomes);
    assert_eq!(answers(&outcomes), answers(&baseline), "the semantics are the reference's");
    assert_eq!(raw(&db), raw(&never));

    // Per query: the switched-off one stops taking part; the others still do.
    let (db, never) = (open::<E>(), open::<E>());
    seed(&db);
    seed(&never);
    warm(&db, &ids);
    let f = &queries()[1];
    let fp = db.result_cache().fingerprint_of(&CachedQuery::Units(f.clone())).unwrap();
    db.result_cache().disable_query(fp);
    let snap = db.snapshot().unwrap();
    let before = db.result_cache().stats();
    for _ in 0..3 {
        let got = snap.query(f).unwrap();
        assert_ne!(got.path, PlanPath::ResultCache);
        assert_eq!(got.units, never.snapshot().unwrap().query(f).unwrap().units);
    }
    let after = db.result_cache().stats();
    assert_eq!((after.hits, after.reads, after.writes), (before.hits, before.reads, before.writes));
    // Asked directly, it is the switch that answers, not an empty slot.
    let query = CachedQuery::Units(f.clone());
    let now = Dependencies::current(snap.seq(), Some(snap.index_status().unwrap()));
    let records = db.result_cache().records().len();
    assert!(matches!(db.result_cache().lookup(&fp, &query, &now), Lookup::Disabled));
    assert_eq!(db.result_cache().records().len(), records, "a disabled query leaves no miss on the record");
    assert_eq!(snap.query(&queries()[0]).unwrap().path, PlanPath::ResultCache, "the other queries still hit");
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn every_operation_on_the_cache_leaves_the_data_as_it_was() {
                super::every_operation_on_the_cache_leaves_the_data_as_it_was::<E>();
            }
            #[test]
            fn invalidating_by_each_dependency_leaves_the_data_as_it_was() {
                super::invalidating_by_each_dependency_leaves_the_data_as_it_was::<E>();
            }
            #[test]
            fn a_cache_in_use_never_changes_what_a_commit_writes() {
                super::a_cache_in_use_never_changes_what_a_commit_writes::<E>();
            }
            #[test]
            fn with_the_cache_off_no_optimization_takes_part() {
                super::with_the_cache_off_no_optimization_takes_part::<E>();
            }
            #[test]
            fn a_cache_switched_off_after_use_stops_taking_part() {
                super::a_cache_switched_off_after_use_stops_taking_part::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
