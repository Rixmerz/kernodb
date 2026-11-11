//! E5-01: the plan type and its explanation (etapa5-acta.md §3, I-23).
//!
//! * `explanation_lists_every_candidate_and_the_reason` — both kinds of query
//!   list all their candidates in table order, with the chosen one and why.
//! * `cheapest_usable_wins_and_ties_go_to_the_first` — the choice is the
//!   lowest cost among the usable, the earliest on a tie; an unusable
//!   candidate is never chosen however cheap.
//! * `index_not_ready_falls_back_to_the_reference_path` — each non-`Ready`
//!   state gives a `Fallback` plan on the reference path (I-23).
//! * `type_index_needs_a_type_filter` — `NoTypeFilter` is a legal reason
//!   only for `IDX_TYPE` and only with a ready index.
//! * `refuses_*` — a plan that would explain falsely is not built.
//! * `plan_holds_no_literal` — the plan of a query with sentinel literals
//!   shows none of them.
//! * `plan_follows_the_real_index_status` — over a real database, the status
//!   the snapshot reports (`Stale`, then `Ready`) is the one the plan records
//!   and the fallback follows it.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

fn key() -> FingerprintKey {
    FingerprintKey::from_bytes([9; 32])
}

fn units_fp(unit_type: Option<&str>) -> Fingerprint {
    NormalizedQuery::of_filter(&UnitFilter { unit_type: unit_type.map(Into::into), schema_version: None })
        .fingerprint(&key())
}

fn relations_fp() -> Fingerprint {
    let q = RelationQuery { unit: UnitId([1; 16]), direction: Direction::Out, relation_type: None };
    NormalizedQuery::of_relations(&q).fingerprint(&key())
}

fn status(state: IndexState) -> IndexStatus {
    IndexStatus { state, watermark: Some(CommitSeq(4)) }
}

fn cand(path: PlanPath, cost: u64, usability: Usability) -> Candidate {
    Candidate { path, cost: Cost(cost), usability }
}

const USABLE: Usability = Usability::Usable;

/// The three unit candidates with these costs, all usable.
fn units(reference: u64, primary: u64, by_type: u64) -> Vec<Candidate> {
    vec![
        cand(PlanPath::UnitsReference, reference, USABLE),
        cand(PlanPath::IndexPrimary, primary, USABLE),
        cand(PlanPath::IndexType, by_type, USABLE),
    ]
}

fn not_ready(state: IndexState) -> Vec<Candidate> {
    let no = Usability::Unusable(Unusable::IndexNotReady(state));
    vec![
        cand(PlanPath::UnitsReference, 1000, USABLE),
        cand(PlanPath::IndexPrimary, 10, no),
        cand(PlanPath::IndexType, 1, no),
    ]
}

fn relations(reference: u64, adjacency: u64) -> Vec<Candidate> {
    vec![cand(PlanPath::RelationsReference, reference, USABLE), cand(PlanPath::Adjacency, adjacency, USABLE)]
}

fn unit_plan(candidates: Vec<Candidate>, state: IndexState) -> Result<Plan, PlanError> {
    Plan::new(units_fp(Some("t")), Some(status(state)), candidates)
}

#[test]
fn explanation_lists_every_candidate_and_the_reason() {
    let plan = unit_plan(units(1000, 100, 7), IndexState::Ready).unwrap();
    assert_eq!(plan.candidates(), units(1000, 100, 7));
    assert_eq!(plan.chosen(), &cand(PlanPath::IndexType, 7, USABLE));
    assert_eq!(plan.reason(), Reason::LowestCost);
    assert_eq!(plan.index(), Some(status(IndexState::Ready)));
    assert_eq!(plan.fingerprint(), &units_fp(Some("t")));

    let plan = Plan::new(relations_fp(), None, relations(500, 3)).unwrap();
    assert_eq!(plan.candidates(), relations(500, 3));
    assert_eq!(plan.chosen().path, PlanPath::Adjacency);
    assert_eq!(plan.reason(), Reason::LowestCost);
    assert_eq!(plan.index(), None);
}

#[test]
fn cheapest_usable_wins_and_ties_go_to_the_first() {
    let chosen = |c| unit_plan(c, IndexState::Ready).unwrap().chosen().path;
    assert_eq!(chosen(units(5, 50, 500)), PlanPath::UnitsReference);
    assert_eq!(chosen(units(50, 5, 500)), PlanPath::IndexPrimary);
    assert_eq!(chosen(units(50, 500, 5)), PlanPath::IndexType);
    assert_eq!(chosen(units(9, 9, 9)), PlanPath::UnitsReference);
    assert_eq!(chosen(units(50, 9, 9)), PlanPath::IndexPrimary);
    let relation = |c| Plan::new(relations_fp(), None, c).unwrap().chosen().path;
    assert_eq!(relation(relations(4, 4)), PlanPath::RelationsReference);
    assert_eq!(relation(relations(5, 4)), PlanPath::Adjacency);

    // IDX_TYPE is the cheapest on paper but has no type filter to answer.
    let mut c = units(100, 50, 1);
    c[2].usability = Usability::Unusable(Unusable::NoTypeFilter);
    assert_eq!(chosen(c), PlanPath::IndexPrimary);
}

#[test]
fn index_not_ready_falls_back_to_the_reference_path() {
    for state in [IndexState::Stale, IndexState::Building, IndexState::Failed] {
        // The unusable index paths are cheaper: they still may not be chosen.
        let plan = unit_plan(not_ready(state), state).unwrap();
        assert_eq!(plan.chosen().path, PlanPath::UnitsReference, "{state:?}");
        assert_eq!(plan.reason(), Reason::Fallback { index: state });
        assert_eq!(plan.index().map(|s| s.state), Some(state));
        assert_eq!(plan.candidates().len(), 3);
    }
}

#[test]
fn type_index_needs_a_type_filter() {
    let mut c = units(100, 50, 1);
    c[2].usability = Usability::Unusable(Unusable::NoTypeFilter);
    let plan = unit_plan(c.clone(), IndexState::Ready).unwrap();
    assert_eq!(plan.candidates()[2].usability, Usability::Unusable(Unusable::NoTypeFilter));
    assert_eq!(plan.reason(), Reason::LowestCost);

    // Not for the primary index...
    c[1].usability = Usability::Unusable(Unusable::NoTypeFilter);
    c[2].usability = USABLE;
    assert!(matches!(unit_plan(c, IndexState::Ready), Err(PlanError::Freshness { path: PlanPath::IndexPrimary, .. })));
    // ...and not as the reason of a stale index.
    let mut c = not_ready(IndexState::Stale);
    c[2].usability = Usability::Unusable(Unusable::NoTypeFilter);
    assert!(matches!(unit_plan(c, IndexState::Stale), Err(PlanError::Freshness { path: PlanPath::IndexType, .. })));
}

#[test]
fn refuses_candidates_that_are_not_exactly_the_kinds() {
    let paths = |e: PlanError| match e {
        PlanError::Candidates { expected, found } => (expected, found),
        other => panic!("not a Candidates error: {other:?}"),
    };
    let mut missing = units(1, 2, 3);
    missing.pop();
    let (expected, found) = paths(unit_plan(missing, IndexState::Ready).unwrap_err());
    assert_eq!(expected, [PlanPath::UnitsReference, PlanPath::IndexPrimary, PlanPath::IndexType]);
    assert_eq!(found, [PlanPath::UnitsReference, PlanPath::IndexPrimary]);

    let mut reordered = units(1, 2, 3);
    reordered.swap(0, 1);
    assert!(matches!(unit_plan(reordered, IndexState::Ready), Err(PlanError::Candidates { .. })));
    let mut twice = units(1, 2, 3);
    twice.push(twice[0]);
    assert!(matches!(unit_plan(twice, IndexState::Ready), Err(PlanError::Candidates { .. })));
    // The candidates of the other kind of query.
    assert!(matches!(unit_plan(relations(1, 2), IndexState::Ready), Err(PlanError::Candidates { .. })));
    assert!(matches!(Plan::new(relations_fp(), None, units(1, 2, 3)), Err(PlanError::Candidates { .. })));
}

#[test]
fn refuses_a_wrong_index_status() {
    assert_eq!(Plan::new(units_fp(None), None, units(1, 2, 3)), Err(PlanError::IndexStatus { kind: QueryKind::Units }));
    assert_eq!(
        Plan::new(relations_fp(), Some(status(IndexState::Ready)), relations(1, 2)),
        Err(PlanError::IndexStatus { kind: QueryKind::Relations })
    );
}

#[test]
fn refuses_a_stale_index_declared_usable() {
    for state in [IndexState::Stale, IndexState::Building, IndexState::Failed] {
        // Either index path usable while the index is not ready.
        for usable in [1, 2] {
            let mut c = not_ready(state);
            c[usable].usability = USABLE;
            assert!(
                matches!(unit_plan(c, state), Err(PlanError::Freshness { status: s, .. }) if s.state == state),
                "{state:?} / candidate {usable}"
            );
        }
        // An unusable reason that names another state than the status.
        let other = if state == IndexState::Stale { IndexState::Failed } else { IndexState::Stale };
        assert!(matches!(unit_plan(not_ready(other), state), Err(PlanError::Freshness { .. })));
    }
    // And a ready index declared not ready.
    assert!(matches!(
        unit_plan(not_ready(IndexState::Stale), IndexState::Ready),
        Err(PlanError::Freshness { path: PlanPath::IndexPrimary, .. })
    ));
}

#[test]
fn refuses_an_unusable_reference_path() {
    let no = Usability::Unusable(Unusable::NoTypeFilter);
    let mut c = units(1, 2, 3);
    c[0].usability = no;
    assert_eq!(unit_plan(c, IndexState::Ready), Err(PlanError::FallbackUnusable { path: PlanPath::UnitsReference }));
    let mut c = relations(1, 2);
    c[1].usability = no;
    assert_eq!(Plan::new(relations_fp(), None, c), Err(PlanError::FallbackUnusable { path: PlanPath::Adjacency }));
}

fn assoc_fp() -> Fingerprint {
    let q = AssocQuery::new(vec![UnitId([1; 16])], 2, 10, 100);
    NormalizedQuery::of_assoc(&q).fingerprint(&key())
}

fn assoc(reference: u64, adjacency: u64, usability: Usability) -> Vec<Candidate> {
    vec![cand(PlanPath::AssocReference, reference, USABLE), cand(PlanPath::AssocAdjacency, adjacency, usability)]
}

#[test]
fn associative_plans_have_their_own_candidates_and_choice() {
    let plan = Plan::new(assoc_fp(), None, assoc(18, 8, USABLE)).unwrap();
    assert_eq!(plan.fingerprint().kind(), QueryKind::Associative);
    assert_eq!((plan.chosen().path, plan.reason(), plan.index()), (PlanPath::AssocAdjacency, Reason::LowestCost, None));
    // A tie goes to the reference path, first in table order.
    let tie = Plan::new(assoc_fp(), None, assoc(8, 8, USABLE)).unwrap();
    assert_eq!(tie.chosen().path, PlanPath::AssocReference);

    // The adjacency path may be unusable, and then it is never chosen however cheap; the plan says why.
    for why in [Unusable::AdjacencyAbsent, Unusable::AdjacencyInconsistent] {
        let plan = Plan::new(assoc_fp(), None, assoc(1000, 1, Usability::Unusable(why))).unwrap();
        assert_eq!(plan.chosen().path, PlanPath::AssocReference, "{why:?}");
        assert_eq!(plan.reason(), Reason::AdjacencyUnusable(why));
    }
    // Only for the reasons that are about the adjacency index.
    for no in [Unusable::NoTypeFilter, Unusable::IndexNotReady(IndexState::Stale)] {
        assert_eq!(
            Plan::new(assoc_fp(), None, assoc(1, 2, Usability::Unusable(no))),
            Err(PlanError::FallbackUnusable { path: PlanPath::AssocAdjacency }),
            "{no:?}"
        );
    }
    // The reference path is always usable, and a relation query's adjacency never is unusable for these reasons.
    let mut c = assoc(1, 2, USABLE);
    c[0].usability = Usability::Unusable(Unusable::AdjacencyAbsent);
    assert_eq!(Plan::new(assoc_fp(), None, c), Err(PlanError::FallbackUnusable { path: PlanPath::AssocReference }));
    let mut c = relations(1, 2);
    c[1].usability = Usability::Unusable(Unusable::AdjacencyAbsent);
    assert_eq!(Plan::new(relations_fp(), None, c), Err(PlanError::FallbackUnusable { path: PlanPath::Adjacency }));
    // Exactly its two candidates, in order; no index status.
    assert!(matches!(Plan::new(assoc_fp(), None, relations(1, 2)), Err(PlanError::Candidates { .. })));
    assert!(matches!(Plan::new(assoc_fp(), None, units(1, 2, 3)), Err(PlanError::Candidates { .. })));
    let reversed: Vec<_> = assoc(1, 2, USABLE).into_iter().rev().collect();
    assert!(matches!(Plan::new(assoc_fp(), None, reversed), Err(PlanError::Candidates { .. })));
    assert_eq!(
        Plan::new(assoc_fp(), Some(status(IndexState::Ready)), assoc(1, 2, USABLE)),
        Err(PlanError::IndexStatus { kind: QueryKind::Associative })
    );
    assert_eq!(PlanPath::candidates_of(QueryKind::Associative), [PlanPath::AssocReference, PlanPath::AssocAdjacency]);
}

#[test]
fn plan_holds_no_literal() {
    let q = NormalizedQuery::of_filter(&UnitFilter {
        unit_type: Some("SENTINEL-type-7f3a".into()),
        schema_version: Some(987_654),
    });
    let plan = Plan::new(q.fingerprint(&key()), Some(status(IndexState::Ready)), units(100, 50, 5)).unwrap();
    let shown = format!("{plan:?}");
    assert!(!shown.contains("SENTINEL") && !shown.contains("987654"), "{shown}");
    assert!(shown.contains(&plan.fingerprint().to_string()));

    // Nor does the plan of an associative query: seeds, types, actor, source, window.
    let mut q = AssocQuery::new(vec![UnitId([0x5E; 16])], 2, 10, 100);
    q.relation_types = Some(["SENTINEL-type-7f3a".to_owned()].into());
    q.window = Some(SeqWindow { lo: CommitSeq(987_654), hi: CommitSeq(987_655) });
    q.provenance.actor = Some(b"SENTINEL-actor".to_vec());
    q.provenance.source = Some(b"SENTINEL-source".to_vec());
    let plan = Plan::new(NormalizedQuery::of_assoc(&q).fingerprint(&key()), None, assoc(18, 8, USABLE)).unwrap();
    let shown = format!("{plan:?}");
    for needle in ["SENTINEL", "987654", "987655", "5e5e", "94, 94"] {
        assert!(!shown.contains(needle), "{needle}: {shown}");
    }
}

type EmuLog = Emulated<LogEngine>;

fn open() -> UnitDb<EmuLog> {
    let mut c = UnitConfig::with_sources(
        Arc::new(FixedClock::new(Timestamp(1_700_000_000_000_000))),
        Arc::new(CounterIds::new(1)),
    );
    c.engine = Options::small_for_tests();
    UnitDb::<EmuLog>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open")
}

/// The plan a snapshot's index status allows, with fixed costs.
fn plan_for(db: &UnitDb<EmuLog>, fingerprint: Fingerprint) -> Plan {
    let index = db.snapshot().unwrap().index_status().unwrap();
    let candidates = if index.state == IndexState::Ready { units(1000, 100, 7) } else { not_ready(index.state) };
    Plan::new(fingerprint, Some(index), candidates).unwrap()
}

#[test]
fn plan_follows_the_real_index_status() {
    let db = open();
    let meta = ChangeMeta::default();
    let mut cs = db.begin_changeset(meta.clone()).unwrap();
    cs.create(NewUnit { unit_type: "t".into(), schema_version: 1, metadata: Vec::new(), payload: b"p".to_vec() })
        .unwrap();
    cs.commit().unwrap();

    let stale = plan_for(&db, units_fp(Some("t")));
    assert_eq!(stale.index(), Some(IndexStatus { state: IndexState::Stale, watermark: None }));
    assert_eq!(stale.chosen().path, PlanPath::UnitsReference);
    assert_eq!(stale.reason(), Reason::Fallback { index: IndexState::Stale });

    let built = db.rebuild_index().unwrap();
    let ready = plan_for(&db, units_fp(Some("t")));
    assert_eq!(ready.index(), Some(built));
    assert_eq!(ready.chosen().path, PlanPath::IndexType);
    assert_eq!(ready.reason(), Reason::LowestCost);
}
