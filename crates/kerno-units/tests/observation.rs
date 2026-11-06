//! E5-03: `ExecutionObservation` (etapa5-acta.md §3, ADR-0015) through the
//! real entrypoints, `UnitSnapshot::query` and `query_relations`.
//!
//! * `observing_does_not_change_the_data` — I-34, observation part: the same
//!   commits and reads on two databases, one with the observer `off` and one
//!   at `diagnostic`, leave the same answers, the same commit sequences and the
//!   same bytes in the engine. The observed run is also shown to have
//!   recorded, so the comparison is not between two silent runs.
//! * `the_default_is_off_and_keeps_nothing` — a fresh database observes
//!   nothing; only `set_level` turns it on.
//! * `queries_report_what_ran` — a record carries the path, rows, snapshot
//!   sequence and keyed fingerprint of the query that produced it.
//! * `concurrent_recording_loses_only_what_it_counts` — queries from several
//!   threads all return the right answer while recording; every execution is
//!   either observed or counted as dropped.

mod harness;

use std::sync::Arc;

use harness::units::{Target, dump, open_units};
use kerno_core::contract::{Options, Snapshot, TxnEngine};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: Target>() -> UnitDb<E> {
    open_units::<E>(SimVfs::new(1).as_vfs(), &config()).expect("open")
}

fn unit(t: &str, schema: u32, payload: u8) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: schema, metadata: Vec::new(), payload: vec![payload] }
}

fn type_filter(t: &str) -> UnitFilter {
    UnitFilter { unit_type: Some(t.into()), schema_version: None }
}

/// What a read returns, with no field the observer could have added.
#[derive(Debug, PartialEq)]
struct Answers {
    seq: CommitSeq,
    units: Vec<Vec<UnitId>>,
    paths: Vec<PlanPath>,
    relations: Vec<Vec<RelationId>>,
}

/// The same reads at every step: each filter and the relations of `unit`.
fn read_all<E: TxnEngine>(db: &UnitDb<E>, unit: Option<UnitId>) -> Answers {
    let snap = db.snapshot().expect("snapshot");
    let (mut units, mut paths) = (Vec::new(), Vec::new());
    for f in [UnitFilter::default(), type_filter("a"), type_filter("b"), type_filter("none")] {
        let out = snap.query(&f).expect("query");
        units.push(out.units.iter().map(|u| u.id).collect());
        paths.push(out.path);
    }
    let mut relations = Vec::new();
    if let Some(unit) = unit {
        for direction in [Direction::Out, Direction::In] {
            let q = RelationQuery { unit, direction, relation_type: None };
            let out = snap.query_relations(&q).expect("query_relations");
            relations.push(out.relations.iter().map(|r| r.id).collect());
        }
    }
    Answers { seq: snap.seq(), units, paths, relations }
}

/// One scripted run: commits interleaved with reads. Returns what every
/// commit returned and what every read answered.
fn run<E: Target>(db: &UnitDb<E>) -> (Vec<u64>, Vec<Answers>) {
    let (mut commits, mut reads) = (Vec::new(), Vec::new());
    let mut note = |c: CommitInfo, db: &UnitDb<E>, u: Option<UnitId>, reads: &mut Vec<Answers>| {
        commits.push(c.commit_seq.0);
        reads.push(read_all(db, u));
    };
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let ids: Vec<UnitId> =
        [("a", 1), ("a", 2), ("b", 1), ("c", 1)].iter().map(|(t, v)| cs.create(unit(t, *v, 1)).unwrap()).collect();
    note(cs.commit().unwrap(), db, None, &mut reads);

    db.rebuild_index().unwrap();
    reads.push(read_all(db, Some(ids[0])));

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(ids[0], ids[1], "cites", RelationProvenance::Declared).unwrap();
    cs.create_relation(ids[2], ids[0], "refines", RelationProvenance::Declared).unwrap();
    note(cs.commit().unwrap(), db, Some(ids[0]), &mut reads);

    // A write after the index was built leaves it stale: the planner falls back.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(ids[1], UnitUpdate { payload: vec![9], schema_version: None, metadata: None }, None).unwrap();
    cs.archive(ids[3], None).unwrap();
    note(cs.commit().unwrap(), db, Some(ids[0]), &mut reads);

    db.rebuild_index().unwrap();
    reads.push(read_all(db, Some(ids[0])));
    (commits, reads)
}

fn observing_does_not_change_the_data<E: Target>() {
    let (off, observed) = (open::<E>(), open::<E>());
    assert_eq!(off.observer().level(), ObservationLevel::Off);
    observed.observer().set_level(ObservationLevel::Diagnostic).unwrap();

    let (commits_off, reads_off) = run(&off);
    let (commits_obs, reads_obs) = run(&observed);

    assert_eq!(commits_off, commits_obs, "commit sequences");
    assert_eq!(reads_off, reads_obs, "answers");
    assert_eq!(dump(&off.engine().snapshot().unwrap()), dump(&observed.engine().snapshot().unwrap()), "engine bytes");
    assert_eq!(off.engine().snapshot().unwrap().seq(), observed.engine().snapshot().unwrap().seq());

    // The observed run did record, and the unobserved one did not.
    assert_eq!(off.observer().stats(), ObservationStats::default());
    assert!(off.observer().records().is_empty());
    let stats = observed.observer().stats();
    // read_all runs 4 queries (6 with a unit): once without one, four times with.
    assert_eq!(stats.observed, 4 + 4 * 6, "{stats:?}");
    assert_eq!((stats.dropped, stats.errors), (0, 0));
    assert_eq!(observed.observer().records().len() as u64, stats.observed.min(DEFAULT_MAX_RECORDS as u64));
}

fn the_default_is_off_and_keeps_nothing<E: Target>() {
    let db = open::<E>();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create(unit("a", 1, 1)).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.observer().level(), ObservationLevel::Off);
    db.snapshot().unwrap().query(&type_filter("a")).unwrap();
    assert_eq!(db.observer().stats(), ObservationStats::default());
    assert!(db.observer().records().is_empty());

    // Only an explicit call raises the level, and `Diagnostic` is no exception.
    db.observer().set_level(ObservationLevel::Aggregate).unwrap();
    db.snapshot().unwrap().query(&type_filter("a")).unwrap();
    assert_eq!((db.observer().stats().observed, db.observer().records().len()), (1, 0));
    db.observer().set_level(ObservationLevel::Diagnostic).unwrap();
    db.snapshot().unwrap().query(&type_filter("a")).unwrap();
    assert_eq!(db.observer().records()[0].fingerprint_text.as_deref(), Some("units[unit_type=?text]"));
}

fn queries_report_what_ran<E: Target>() {
    let db = open::<E>();
    db.observer().set_level(ObservationLevel::Fingerprint).unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let ids: Vec<UnitId> = (0..3).map(|i| cs.create(unit("a", 1, i)).unwrap()).collect();
    cs.create_relation(ids[0], ids[1], "cites", RelationProvenance::Declared).unwrap();
    let committed = cs.commit().unwrap();

    // No index yet: the reference path answers.
    let snap = db.snapshot().unwrap();
    let out = snap.query(&type_filter("a")).unwrap();
    assert_eq!(out.units.len(), 3);
    let rel = snap.query_relations(&RelationQuery { unit: ids[0], direction: Direction::Out, relation_type: None });
    assert_eq!(rel.unwrap().relations.len(), 1);
    // The same shape with another literal has the same fingerprint.
    snap.query(&type_filter("other")).unwrap();

    let records = db.observer().records();
    assert_eq!(records.len(), 3);
    for r in &records {
        assert_eq!(
            (r.commit_seq, r.outcome, r.fallback, r.fingerprint_text.clone()),
            (committed.commit_seq, Outcome::Ok, None, None)
        );
        assert_eq!(r.plan_version, PLANNER_VERSION);
    }
    assert_eq!((records[0].path, records[0].rows_returned), (Some(out.path), 3));
    assert_eq!((records[1].path, records[1].rows_returned), (Some(PlanPath::RelationsReference), 1));
    assert_eq!(records[2].rows_returned, 0);
    let kinds: Vec<QueryKind> = records.iter().map(|r| r.fingerprint.kind()).collect();
    assert_eq!(kinds, [QueryKind::Units, QueryKind::Relations, QueryKind::Units]);
    assert_eq!(records[0].fingerprint, records[2].fingerprint);
    assert_ne!(records[0].fingerprint, records[1].fingerprint);
    let ids: Vec<u64> = records.iter().map(|r| r.observation_id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
}

fn concurrent_recording_loses_only_what_it_counts<E: Target>() {
    const THREADS: usize = 4;
    const PER_THREAD: usize = 200;
    let db = open::<E>();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for i in 0..5 {
        cs.create(unit("a", 1, i)).unwrap();
    }
    cs.commit().unwrap();
    db.rebuild_index().unwrap();
    db.observer().set_level(ObservationLevel::Diagnostic).unwrap();
    db.observer().set_config(ObservationConfig { max_records: 64, ..ObservationConfig::default() }).unwrap();

    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| {
                let snap = db.snapshot().unwrap();
                for _ in 0..PER_THREAD {
                    assert_eq!(snap.query(&type_filter("a")).unwrap().units.len(), 5);
                }
            });
        }
    });

    let stats = db.observer().stats();
    assert_eq!(stats.observed + stats.dropped, (THREADS * PER_THREAD) as u64, "{stats:?}");
    assert!(stats.retained <= 64, "{stats:?}");
    assert_eq!(stats.retained as u64 + stats.evicted, stats.observed, "{stats:?}");
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn observing_does_not_change_the_data() {
                super::observing_does_not_change_the_data::<E>();
            }
            #[test]
            fn the_default_is_off_and_keeps_nothing() {
                super::the_default_is_off_and_keeps_nothing::<E>();
            }
            #[test]
            fn queries_report_what_ran() {
                super::queries_report_what_ran::<E>();
            }
            #[test]
            fn concurrent_recording_loses_only_what_it_counts() {
                super::concurrent_recording_loses_only_what_it_counts::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
