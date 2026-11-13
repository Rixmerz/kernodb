//! E6-02: the result cache as one more candidate of the planner
//! (etapa6-acta.md §3, ADR-0016 G1 to G7, I-32, I-33, I-35, I-36).
//!
//! * `cache_on_off_and_invalidated_give_identical_results` — G1. The same
//!   script of reads and interleaved writes runs on three identical
//!   databases: cache off, cache on, and cache on with an old snapshot read
//!   after every write. Every answer is equal to the one with the cache off;
//!   the run is not vacuous (hits happen between writes, invalidations after).
//! * `a_failed_check_discards_the_entry_and_the_reference_path_answers` — G2
//!   and G5. Forged entries whose rows break `check_units` / `check_relations`
//!   are never returned: the reference path answers, the cause is recorded.
//! * `turning_the_cache_off_restores_the_reference_behaviour` — G6, I-36 and
//!   the default: off, no entry is read or written and nothing is recorded.
//! * `observations_never_act_as_authority` — G3. What the observer holds, at
//!   any level, neither creates, serves nor withholds an entry.
//! * `every_use_of_the_cache_is_a_transition` — G7. Hit, miss, store,
//!   invalidation and fallback each leave their record, with no literal.

mod harness;

use harness::cache_script::{open, queries, relation_queries, seed, write};
use harness::units::Target;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_units::*;
use std::sync::Arc;
use std::time::Duration;

type EmuLog = Emulated<LogEngine>;

/// What a read gave, without the path that gave it: that is what must not
/// depend on the cache.
type Answer = (Vec<Unit>, Vec<Relation>);

fn read_all<E: Target>(snap: &UnitSnapshot<E>, ids: &[UnitId]) -> Vec<(Answer, bool)> {
    let mut out = Vec::new();
    for f in queries() {
        let o = snap.query(&f).unwrap();
        out.push(((o.units, Vec::new()), o.path == PlanPath::ResultCache));
    }
    for q in relation_queries(ids) {
        let o = snap.query_relations(&q).unwrap();
        out.push(((Vec::new(), o.relations), o.path == PlanPath::ResultCache));
    }
    out
}

/// The cache on, and the observer at the level an entry's evidence needs.
fn cache_on<E: Target>(db: &UnitDb<E>) {
    db.observer().set_level(ObservationLevel::Fingerprint).unwrap();
    cache_on_only(db);
}

/// The cache on, the observer as it is.
fn cache_on_only<E: Target>(db: &UnitDb<E>) {
    db.result_cache().set_enabled(true);
}

const OBS_T0: Timestamp = Timestamp(5_000_000_000);

fn count(db: &UnitDb<impl Target>, f: impl Fn(&Transition) -> bool) -> usize {
    db.result_cache().records().iter().filter(|r| f(&r.transition)).count()
}

fn cache_on_off_and_invalidated_give_identical_results<E: Target>() {
    let (off, on, held) = (open::<E>(), open::<E>(), open::<E>());
    let ids = seed(&off);
    assert_eq!(seed(&on), ids, "identical databases");
    assert_eq!(seed(&held), ids);
    cache_on(&on);
    cache_on(&held);

    let mut hits = 0;
    let mut old: Option<UnitSnapshot<E>> = None;
    let mut old_expected: Option<Vec<(Answer, bool)>> = None;
    for round in 0..10 {
        let (s_off, s_on, s_held) = (off.snapshot().unwrap(), on.snapshot().unwrap(), held.snapshot().unwrap());
        let expected = read_all(&s_off, &ids);
        assert!(expected.iter().all(|(_, cached)| !cached), "the cache is off: no cached answer");
        // Twice at one snapshot: the second read is the one that can hit.
        for pass in 0..2 {
            let got = read_all(&s_on, &ids);
            for (i, ((a, cached), (b, _))) in got.iter().zip(&expected).enumerate() {
                assert_eq!(a, b, "round {round} pass {pass} query {i}: cache on differs from cache off");
                hits += usize::from(*cached);
            }
        }
        // The held database reads an old snapshot after the write below moved
        // `commit_seq`: it must answer as of its own sequence, not as of the
        // entries the newer snapshot stored.
        let before = read_all(&s_held, &ids);
        assert_eq!(before.iter().map(|x| &x.0).collect::<Vec<_>>(), expected.iter().map(|x| &x.0).collect::<Vec<_>>());
        if let (Some(snap), Some(exp)) = (&old, &old_expected) {
            let again = read_all(snap, &ids);
            assert_eq!(
                again.iter().map(|x| &x.0).collect::<Vec<_>>(),
                exp.iter().map(|x| &x.0).collect::<Vec<_>>(),
                "round {round}: an old snapshot is answered as of its own sequence"
            );
        }
        old = Some(s_held);
        old_expected = Some(expected);

        for db in [&off, &on, &held] {
            write(db, &ids, round);
        }
    }

    assert!(hits > 0, "not vacuous: repeated reads at one sequence hit the cache");
    let invalidated = count(&on, |t| matches!(t, Transition::Invalidated { .. }));
    assert!(invalidated > 0, "not vacuous: the interleaved writes invalidated entries");
    assert!(
        on.result_cache()
            .records()
            .iter()
            .any(|r| r.transition == Transition::Invalidated { dependency: Dependency::CommitSeq }),
        "a write invalidates through commit_seq"
    );
    assert_eq!(count(&off, |_| true), 0, "the cache off records nothing");
}

/// A forged entry for `query` holding `result`, valid for `snap` in every
/// dependency: only its rows are wrong.
fn plant<E: Target>(
    db: &UnitDb<E>,
    snap: &UnitSnapshot<E>,
    query: CachedQuery,
    result: CachedResult,
    index: bool,
) -> Fingerprint {
    let cache = db.result_cache();
    let fp = cache.fingerprint_of(&query).unwrap();
    let index = index.then(|| snap.index_status().unwrap());
    let deps = Dependencies::current(snap.seq(), index);
    let ev = Evidence { observations: 1, window: std::time::Duration::ZERO };
    cache.create(CacheEntry::new(fp, query, deps, result, ev, 1_000).unwrap()).unwrap();
    cache.activate(&fp).unwrap();
    fp
}

fn a_failed_check_discards_the_entry_and_the_reference_path_answers<E: Target>() {
    let reference = open::<E>();
    let db = open::<E>();
    let ids = seed(&reference);
    seed(&db);
    cache_on(&db);
    let (snap_ref, snap) = (reference.snapshot().unwrap(), db.snapshot().unwrap());

    let docs = UnitFilter { unit_type: Some("doc".into()), schema_version: None };
    let right = snap_ref.query(&docs).unwrap().units;
    assert!(right.len() >= 2);
    let imgs = snap_ref.query(&UnitFilter { unit_type: Some("img".into()), schema_version: None }).unwrap().units;

    let reversed: Vec<Unit> = right.iter().rev().cloned().collect();
    let mut archived = right.clone();
    archived[0].lifecycle = Lifecycle::Archived;
    let forged =
        [(imgs.clone(), Violation::NotMatching), (reversed, Violation::OutOfOrder), (archived, Violation::NotActive)];
    for (rows, violation) in forged {
        let fp = plant(&db, &snap, CachedQuery::Units(docs.clone()), CachedResult::Units(rows.clone()), true);
        let got = snap.query(&docs).unwrap();
        assert_eq!(got.units, right, "{violation:?}: the forged rows are not returned");
        assert_eq!(got.path, PlanPath::UnitsReference, "{violation:?}");
        assert_eq!(
            got.fallback,
            Some(Fallback { abandoned: PlanPath::ResultCache, cause: FallbackCause::Invalid(violation) })
        );
        let log = db.result_cache().records();
        assert!(
            log.iter().any(|r| r.fingerprint == Some(fp)
                && r.transition == Transition::Fallback { cause: CacheFallbackCause::Discrepancy }),
            "{violation:?}: the fallback is on the record with its cause"
        );
        assert!(
            log.iter().any(|r| r.transition == Transition::Deleted { reason: DeleteReason::Discrepancy }),
            "{violation:?}: the entry is discarded"
        );
        // The reference answer replaced it: the next read is served, and right.
        let next = snap.query(&docs).unwrap();
        assert_eq!((next.path, next.units.as_slice()), (PlanPath::ResultCache, right.as_slice()));
    }

    // The same for relations.
    let q = RelationQuery { unit: ids[1], direction: Direction::Out, relation_type: None };
    let rows = snap_ref.query_relations(&q).unwrap().relations;
    let other = snap_ref
        .query_relations(&RelationQuery { unit: ids[2], direction: Direction::Out, relation_type: None })
        .unwrap()
        .relations;
    assert!(!rows.is_empty() && !other.is_empty());
    plant(&db, &snap, CachedQuery::Relations(q.clone()), CachedResult::Relations(other), false);
    let got = snap.query_relations(&q).unwrap();
    assert_eq!(got.relations, rows);
    assert_eq!(got.path, PlanPath::RelationsReference);
    assert_eq!(
        got.fallback,
        Some(Fallback { abandoned: PlanPath::ResultCache, cause: FallbackCause::Invalid(Violation::NotMatching) })
    );
}

fn turning_the_cache_off_restores_the_reference_behaviour<E: Target>() {
    let (never, db) = (open::<E>(), open::<E>());
    let ids = seed(&never);
    seed(&db);
    assert!(!db.result_cache().enabled(), "off by default");

    // Off: the full outcome (rows, source, path, fallback) is what it was
    // before the cache existed, and nothing is stored or recorded.
    let outcomes = |db: &UnitDb<E>| {
        let s = db.snapshot().unwrap();
        let units: Vec<_> = queries().iter().map(|f| s.query(f).unwrap()).collect();
        let rels: Vec<_> = relation_queries(&ids[..1]).iter().map(|q| s.query_relations(q).unwrap()).collect();
        (units, rels)
    };
    let baseline = outcomes(&never);
    assert_eq!(outcomes(&db), baseline);
    assert_eq!(outcomes(&db), baseline);
    assert!(db.result_cache().records().is_empty());
    assert_eq!(db.result_cache().stats().entries, 0);

    // On: it fills and serves.
    cache_on(&db);
    outcomes(&db);
    let (units, rels) = outcomes(&db);
    // One entry per query shape: queries that differ only in their literals
    // share a slot and take turns, so not every read hits, but some do.
    let served = |os: &[PlanPath]| os.iter().filter(|p| **p == PlanPath::ResultCache).count();
    assert!(served(&units.iter().map(|o| o.path).collect::<Vec<_>>()) >= 3);
    assert!(served(&rels.iter().map(|o| o.path).collect::<Vec<_>>()) == rels.len());
    assert!(db.result_cache().stats().entries > 0);
    let seq = db.snapshot().unwrap().seq();

    // Off again: back to the baseline, byte for byte, and the entries are gone.
    db.result_cache().set_enabled(false);
    assert_eq!(db.result_cache().stats().entries, 0);
    assert_eq!(outcomes(&db), baseline);
    assert_eq!(db.snapshot().unwrap().seq(), seq, "switching and reading never commit");
    let used = count(&db, |t| *t == Transition::Used);
    outcomes(&db);
    assert_eq!(count(&db, |t| *t == Transition::Used), used, "off: no entry participates");

    // A write while off, then on: no entry of before survives, so the first
    // read is a miss and answers as the reference does.
    write(&db, &ids, 0);
    write(&never, &ids, 0);
    cache_on(&db);
    assert_eq!(db.result_cache().stats().entries, 0);
    let first = outcomes(&db);
    assert!(first.0.iter().all(|o| o.path != PlanPath::ResultCache));
    assert_eq!(
        first.0.iter().map(|o| &o.units).collect::<Vec<_>>(),
        outcomes(&never).0.iter().map(|o| &o.units).collect::<Vec<_>>()
    );

    // The per-query switch: one query is reference again, the others serve.
    let f = &queries()[1];
    let fp = db.result_cache().fingerprint_of(&CachedQuery::Units(f.clone())).unwrap();
    db.result_cache().disable_query(fp);
    let s = db.snapshot().unwrap();
    assert_ne!(s.query(f).unwrap().path, PlanPath::ResultCache);
    assert_eq!(s.query(f).unwrap().units, never.snapshot().unwrap().query(f).unwrap().units);
    assert_eq!(s.query(&queries()[0]).unwrap().path, PlanPath::ResultCache);
}

fn observations_never_act_as_authority<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    let f = queries()[1].clone();
    let q = CachedQuery::Units(f.clone());
    let clock = Arc::new(FixedClock::new(OBS_T0));
    db.observer().set_clock(clock.clone());

    // Many observations of this very query, with the cache off, are not an
    // entry: turning the cache on, the first read is a miss with no `Used`.
    db.observer().set_level(ObservationLevel::Diagnostic).unwrap();
    let snap = db.snapshot().unwrap();
    let mut last = None;
    for i in 0..20 {
        clock.set(Timestamp(OBS_T0.0 + i * 1_000));
        last = Some(snap.query(&f).unwrap());
    }
    let rows = last.unwrap();
    assert!(db.observer().stats().observed >= 20, "the observer holds plenty of evidence");
    cache_on_only(&db);
    clock.set(Timestamp(OBS_T0.0 + 25_000));
    let first = snap.query(&f).unwrap();
    assert_ne!(first.path, PlanPath::ResultCache, "observations alone do not serve");
    assert_eq!(first.units, rows.units);
    assert_eq!(count(&db, |t| *t == Transition::Used), 0);
    assert!(count(&db, |t| matches!(t, Transition::Missed { cause: MissCause::NoEntry })) > 0);

    // The evidence of the entry is what the observer held about the shape:
    // the 20 earlier executions and the one that concluded, over the time
    // from the first of them to now (S2).
    let fp = db.result_cache().fingerprint_of(&q).unwrap();
    let created: Vec<_> = db
        .result_cache()
        .records()
        .into_iter()
        .filter(|r| r.fingerprint == Some(fp))
        .filter_map(|r| match r.transition {
            Transition::Created { evidence, cost_ns } => Some((evidence, cost_ns)),
            _ => None,
        })
        .collect();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].0, Evidence { observations: 21, window: Duration::from_micros(25_000) });
    assert!(created[0].1 > 100, "a measured cost, not a placeholder: {} ns", created[0].1);

    // What the observer holds decides whether an entry is created, never
    // whether one is served: with records or without, the entry serves
    // while its dependencies match. An observer that holds nothing about the
    // shape (level below `Fingerprint`) gives no evidence, so no entry is
    // created or activated: a second read is not served either (S2).
    for level in [
        ObservationLevel::Diagnostic,
        ObservationLevel::Fingerprint,
        ObservationLevel::Aggregate,
        ObservationLevel::Off,
    ] {
        db.result_cache().clear();
        db.observer().set_level(level).unwrap();
        db.observer().clear();
        let snap = db.snapshot().unwrap();
        let a = snap.query(&f).unwrap();
        let b = snap.query(&f).unwrap();
        assert_ne!(a.path, PlanPath::ResultCache, "{level:?}: the first read misses");
        assert_eq!(a.units, b.units);
        if level >= ObservationLevel::Fingerprint {
            assert_eq!(b.path, PlanPath::ResultCache, "{level:?}: the second read hits");
            db.observer().clear();
            assert_eq!(
                snap.query(&f).unwrap().path,
                PlanPath::ResultCache,
                "{level:?}: clearing the observer keeps entries"
            );
        } else {
            assert_ne!(b.path, PlanPath::ResultCache, "{level:?}: no evidence, no entry to serve");
            let stats = db.result_cache().stats();
            assert_eq!((stats.entries, stats.writes, stats.hits), (0, 0, 0), "{level:?}");
            assert_eq!(
                count(&db, |t| matches!(t, Transition::Created { .. } | Transition::Activated | Transition::Used)),
                0,
                "{level:?}: nothing created, activated or used without evidence"
            );
        }
    }

    // Observed hits do not keep an entry alive across a write: what the
    // observer saw at an earlier sequence is no dependency of the entry.
    db.observer().set_level(ObservationLevel::Diagnostic).unwrap();
    let snap = db.snapshot().unwrap();
    snap.query(&f).unwrap();
    assert_eq!(snap.query(&f).unwrap().path, PlanPath::ResultCache);
    write(&db, &ids, 0);
    let later = db.snapshot().unwrap();
    let after = later.query(&f).unwrap();
    assert_ne!(after.path, PlanPath::ResultCache, "a write invalidates however much was observed");
    // The index goes stale with the write, and is compared before the sequence.
    assert!(count(&db, |t| matches!(t, Transition::Invalidated { .. })) > 0);
}

/// The entry of a relation query is created on the observer's evidence too.
fn relation_evidence_comes_from_the_observer<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    let rq = relation_queries(&ids)[0].clone();
    let fp = db.result_cache().fingerprint_of(&CachedQuery::Relations(rq.clone())).unwrap();
    let clock = Arc::new(FixedClock::new(OBS_T0));
    db.observer().set_clock(clock.clone());
    db.observer().set_level(ObservationLevel::Fingerprint).unwrap();
    let snap = db.snapshot().unwrap();
    for i in 0..3 {
        clock.set(Timestamp(OBS_T0.0 + i * 10));
        snap.query_relations(&rq).unwrap();
    }
    // A query of another shape is not evidence for this one.
    snap.query(&queries()[1]).unwrap();
    cache_on_only(&db);
    clock.set(Timestamp(OBS_T0.0 + 100));
    snap.query_relations(&rq).unwrap();
    let evidence = db.result_cache().records().into_iter().find_map(|r| match r.transition {
        Transition::Created { evidence, cost_ns } if r.fingerprint == Some(fp) => Some((evidence, cost_ns)),
        _ => None,
    });
    let (evidence, cost_ns) = evidence.expect("an entry was created");
    assert_eq!(evidence, Evidence { observations: 4, window: Duration::from_micros(100) });
    assert!(cost_ns > 100, "{cost_ns}");
    assert_eq!(snap.query_relations(&rq).unwrap().path, PlanPath::ResultCache);

    // No evidence (observer off): nothing is created or activated.
    let off = open::<E>();
    let ids = seed(&off);
    cache_on_only(&off);
    let rq = relation_queries(&ids)[0].clone();
    let snap = off.snapshot().unwrap();
    snap.query_relations(&rq).unwrap();
    assert_ne!(snap.query_relations(&rq).unwrap().path, PlanPath::ResultCache);
    assert_eq!(off.result_cache().stats().entries, 0);
}

/// What the answer's `path` and `source` say must be what the plan said.
fn assert_plan_matches<E: Target>(plan: &Plan, outcome: &QueryOutcome) {
    let served = outcome.path == PlanPath::ResultCache && matches!(outcome.source, QuerySource::Cache { .. });
    assert_eq!(plan.cache().learned_state_used(), served, "{plan:?} against {outcome:?}");
    if served {
        assert_eq!(plan.answering_path(), PlanPath::ResultCache);
    } else if outcome.fallback.is_none() {
        assert_eq!(plan.answering_path(), outcome.path, "{plan:?} against {outcome:?}");
    }
}

fn the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    let key = FingerprintKey::from_bytes([3; 32]);
    let f = queries()[1].clone();
    let rq = relation_queries(&ids)[0].clone();

    // Cache off (the default): not consulted, no learned state; the plan holds
    // no cache candidate and the answer is the planner's.
    let snap = db.snapshot().unwrap();
    let plan = snap.explain_filter(&key, &f).unwrap();
    assert_eq!(*plan.cache(), CacheView::Off);
    assert!(!plan.cache().consulted() && !plan.cache().learned_state_used());
    assert_eq!(plan.cache().cost_ns(), None);
    assert_plan_matches::<E>(&plan, &snap.query(&f).unwrap());
    let rplan = snap.explain_relations(&key, &rq).unwrap();
    assert_eq!(*rplan.cache(), CacheView::Off);
    assert!(db.result_cache().records().is_empty() && db.result_cache().stats().entries == 0);

    // On, nothing stored yet: consulted, a miss, no learned state.
    cache_on(&db);
    let plan = snap.explain_filter(&key, &f).unwrap();
    assert_eq!(*plan.cache(), CacheView::Miss(MissCause::NoEntry));
    assert!(plan.cache().consulted() && !plan.cache().learned_state_used());
    let rplan = snap.explain_relations(&key, &rq).unwrap();
    assert_eq!(*rplan.cache(), CacheView::Miss(MissCause::NoEntry));
    // Explaining changes nothing: no record, no counter, no entry.
    let before = (db.result_cache().records(), db.result_cache().stats());
    for _ in 0..3 {
        snap.explain_filter(&key, &f).unwrap();
        snap.explain_relations(&key, &rq).unwrap();
    }
    assert_eq!((db.result_cache().records(), db.result_cache().stats()), before);

    // The first read stores; then the plan says the entry answers, with the
    // evidence and the cost it was created on, and the answer agrees.
    let miss = snap.query(&f).unwrap();
    assert_plan_matches::<E>(&plan, &miss);
    let plan = snap.explain_filter(&key, &f).unwrap();
    let CacheView::Hit { evidence, cost_ns } = *plan.cache() else { panic!("{plan:?}") };
    assert!(plan.cache().consulted() && plan.cache().learned_state_used());
    assert_eq!((plan.cache().evidence(), plan.cache().cost_ns()), (Some(evidence), Some(cost_ns)));
    assert!(evidence.observations >= 1 && cost_ns > 0);
    let record = db
        .result_cache()
        .records()
        .into_iter()
        .find_map(|r| match r.transition {
            Transition::Created { evidence, cost_ns } => Some((evidence, cost_ns)),
            _ => None,
        })
        .unwrap();
    assert_eq!(record, (evidence, cost_ns), "the plan shows what the creation record says");
    assert_eq!(plan.answering_path(), PlanPath::ResultCache);
    assert!(plan.candidates().iter().all(|c| c.path != PlanPath::ResultCache), "never a candidate");
    let hit = snap.query(&f).unwrap();
    assert_plan_matches::<E>(&plan, &hit);
    assert_eq!(hit.units, miss.units);
    snap.query_relations(&rq).unwrap();
    let rplan = snap.explain_relations(&key, &rq).unwrap();
    assert!(matches!(*rplan.cache(), CacheView::Hit { .. }), "{rplan:?}");
    assert_eq!(snap.query_relations(&rq).unwrap().path, PlanPath::ResultCache);

    // Another query of the same shape (other literal) does not get the entry.
    let other = queries()[2].clone();
    let plan = snap.explain_filter(&key, &other).unwrap();
    assert_eq!(*plan.cache(), CacheView::Miss(MissCause::OtherQuery));
    assert_plan_matches::<E>(&plan, &snap.query(&other).unwrap());

    // A write moves the dependencies: the plan names the one that changed and
    // says no learned state answers; the answer is the planner's.
    write(&db, &ids, 0);
    let later = db.snapshot().unwrap();
    // `other` owns the entry of its shape (it replaced the first one above).
    let plan = later.explain_filter(&key, &other).unwrap();
    assert!(matches!(plan.cache(), CacheView::Invalidated(_)), "{plan:?}");
    assert!(plan.cache().consulted() && !plan.cache().learned_state_used());
    assert_plan_matches::<E>(&plan, &later.query(&other).unwrap());

    // Off for this one query: not consulted, and nothing else changes.
    let g = queries()[3].clone();
    later.query(&g).unwrap();
    let fp = db.result_cache().fingerprint_of(&CachedQuery::Units(g.clone())).unwrap();
    assert!(matches!(later.explain_filter(&key, &g).unwrap().cache(), CacheView::Hit { .. }));
    db.result_cache().disable_query(fp);
    let plan = later.explain_filter(&key, &g).unwrap();
    assert_eq!(*plan.cache(), CacheView::Off);
    assert_plan_matches::<E>(&plan, &later.query(&g).unwrap());
}

/// A plan does not announce an entry whose rows the checks would refuse.
fn the_plan_does_not_announce_an_entry_that_fails_its_check<E: Target>() {
    let db = open::<E>();
    seed(&db);
    cache_on(&db);
    let snap = db.snapshot().unwrap();
    let key = FingerprintKey::from_bytes([3; 32]);
    let docs = queries()[1].clone();
    let imgs = snap.query(&queries()[2]).unwrap().units;
    assert!(!imgs.is_empty());
    plant(&db, &snap, CachedQuery::Units(docs.clone()), CachedResult::Units(imgs), true);
    let plan = snap.explain_filter(&key, &docs).unwrap();
    assert!(matches!(plan.cache(), CacheView::Discrepancy(Violation::NotMatching)), "{plan:?}");
    assert!(plan.cache().consulted() && !plan.cache().learned_state_used());
    assert_eq!(plan.cache().cost_ns(), None);
    let outcome = snap.query(&docs).unwrap();
    assert_plan_matches::<E>(&plan, &outcome);
    assert_eq!(outcome.fallback.map(|f| f.abandoned), Some(PlanPath::ResultCache));
}

fn replacing_the_key_leaves_a_record_of_the_wipe<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    cache_on(&db);
    let snap = db.snapshot().unwrap();
    for f in queries().iter().take(3) {
        snap.query(f).unwrap();
    }
    snap.query_relations(&relation_queries(&ids)[0]).unwrap();
    let entries = db.result_cache().stats().entries as u64;
    assert!(entries >= 3);
    let seq = db.snapshot().unwrap().seq();
    db.result_cache().set_key(FingerprintKey::from_bytes([7; 32]));
    let records = db.result_cache().records();
    assert_eq!(records.len(), 1, "the new log holds the record of the wipe: {records:?}");
    assert_eq!(records[0].transition, Transition::KeyReplaced { entries });
    assert_eq!(records[0].fingerprint, None);
    assert_eq!(db.result_cache().stats().entries, 0);
    // The data and the sequence are untouched (I-34).
    assert_eq!(db.snapshot().unwrap().seq(), seq);
    assert_eq!(snap.query(&queries()[1]).unwrap().units.len(), 3);
}

fn every_use_of_the_cache_is_a_transition<E: Target>() {
    let db = open::<E>();
    let ids = seed(&db);
    cache_on(&db);
    let f = queries()[1].clone();
    let kinds = |db: &UnitDb<E>| db.result_cache().records().into_iter().map(|r| r.transition).collect::<Vec<_>>();

    let snap = db.snapshot().unwrap();
    snap.query(&f).unwrap(); // miss, store
    let k = kinds(&db);
    assert!(k.contains(&Transition::Missed { cause: MissCause::NoEntry }), "{k:?}");
    assert!(k.iter().any(|t| matches!(t, Transition::Created { .. })) && k.contains(&Transition::Activated));
    snap.query(&f).unwrap(); // hit
    assert!(kinds(&db).contains(&Transition::Used));
    write(&db, &ids, 0);
    db.snapshot().unwrap().query(&f).unwrap(); // invalidation, fallback, store
    let k = kinds(&db);
    // The index goes stale before the sequence is compared: that is the one named.
    assert!(k.iter().any(|t| matches!(t, Transition::Invalidated { .. })), "{k:?}");
    assert!(k.contains(&Transition::Fallback { cause: CacheFallbackCause::Invalidated }), "{k:?}");

    // Another query of the same shape (other literal) shares the fingerprint
    // but not the entry: it is a miss of its own, not a discrepancy.
    let now = db.snapshot().unwrap();
    let other = queries()[2].clone();
    assert_eq!(
        db.result_cache().fingerprint_of(&CachedQuery::Units(other.clone())),
        db.result_cache().fingerprint_of(&CachedQuery::Units(f.clone())),
        "same shape, same fingerprint"
    );
    let got = now.query(&other).unwrap();
    assert!(got.fallback.is_none() && got.path != PlanPath::ResultCache, "{got:?}");
    assert!(got.units.iter().all(|u| u.unit_type == "img") && !got.units.is_empty());
    assert!(kinds(&db).contains(&Transition::Missed { cause: MissCause::OtherQuery }));
    assert!(!kinds(&db).contains(&Transition::Fallback { cause: CacheFallbackCause::Discrepancy }));

    // No record names a filter, a unit or a row: only the fingerprint.
    for r in db.result_cache().records() {
        let text = format!("{r:?}");
        for needle in ["doc", "img", "cites"] {
            assert!(!text.contains(needle), "{needle} in {text}");
        }
    }
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn cache_on_off_and_invalidated_give_identical_results() {
                super::cache_on_off_and_invalidated_give_identical_results::<E>();
            }
            #[test]
            fn a_failed_check_discards_the_entry_and_the_reference_path_answers() {
                super::a_failed_check_discards_the_entry_and_the_reference_path_answers::<E>();
            }
            #[test]
            fn turning_the_cache_off_restores_the_reference_behaviour() {
                super::turning_the_cache_off_restores_the_reference_behaviour::<E>();
            }
            #[test]
            fn observations_never_act_as_authority() {
                super::observations_never_act_as_authority::<E>();
            }
            #[test]
            fn relation_evidence_comes_from_the_observer() {
                super::relation_evidence_comes_from_the_observer::<E>();
            }
            #[test]
            fn the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers() {
                super::the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers::<E>();
            }
            #[test]
            fn the_plan_does_not_announce_an_entry_that_fails_its_check() {
                super::the_plan_does_not_announce_an_entry_that_fails_its_check::<E>();
            }
            #[test]
            fn replacing_the_key_leaves_a_record_of_the_wipe() {
                super::replacing_the_key_leaves_a_record_of_the_wipe::<E>();
            }
            #[test]
            fn every_use_of_the_cache_is_a_transition() {
                super::every_use_of_the_cache_is_a_transition::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
