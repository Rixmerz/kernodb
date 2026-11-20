//! Hybrid retrieval (`UnitSnapshot::hybrid_search`): the fusion arithmetic,
//! the ordering, the seed and inactive-unit rules, the refusals, the privacy of
//! the query and the agreement of the two engines.
//!
//! The fixture (3-dimensional vectors, cosine):
//!
//! | unit | payload | vector | relations |
//! |---|---|---|---|
//! | `a` | `rust borrow checker` | `[1, 0, 0]` | none |
//! | `b` | `garbage collector` | `[0, 1, 0]` | `b -> s` |
//! | `c` | `rust ownership` | `[0.9, 0.1, 0]` | none |
//! | `s` | `memory` | `[0, 0, 1]` | |
//! | `d` | `memory safety` | `[0, 0.2, 1]` | `d -> s` |

mod harness;

use std::cell::Cell;
use std::path::Path;
use std::sync::Arc;

use harness::derived::T0;
use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::assoc_rank::{VectorMetric, VectorSet};
use kerno_units::hybrid::Fusion;
use kerno_units::{
    AnnConfig, AnnIndex, AnnMetric, AssocQuery, ChangeMeta, CounterIds, FixedClock, HybridHit, HybridQuery,
    HybridWeights, NearestUnits, NewUnit, RelationProvenance, UnitConfig, UnitDb, UnitError, UnitId, UnitSnapshot,
};

const RUST: [f32; 3] = [1.0, 0.0, 0.0];

struct Fixture<E: Target> {
    db: UnitDb<E>,
    ann: AnnIndex<E>,
    a: UnitId,
    b: UnitId,
    c: UnitId,
    s: UnitId,
    d: UnitId,
}

fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn new_unit(payload: &str) -> NewUnit {
    NewUnit { unit_type: "note".into(), schema_version: 1, metadata: Vec::new(), payload: payload.as_bytes().to_vec() }
}

fn fixture<E: Target>() -> Fixture<E> {
    let disk = SimVfs::new(5);
    let db = open_units::<E>(disk.as_vfs(), &config()).expect("open the units");
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    let a = cs.create(new_unit("rust borrow checker")).expect("a");
    let b = cs.create(new_unit("garbage collector")).expect("b");
    let c = cs.create(new_unit("rust ownership")).expect("c");
    let s = cs.create(new_unit("memory")).expect("s");
    let d = cs.create(new_unit("memory safety")).expect("d");
    cs.create_relation(b, s, "about", RelationProvenance::Declared).expect("b -> s");
    cs.create_relation(d, s, "about", RelationProvenance::Declared).expect("d -> s");
    cs.commit().expect("commit");

    let cfg = AnnConfig { seed: 7, ..AnnConfig::new(3, AnnMetric::Cosine) };
    let mut ann = AnnIndex::<E>::open(disk.as_vfs(), Path::new("/ann"), &Options::small_for_tests(), cfg).expect("ann");
    for (unit, v) in
        [(a, [1.0, 0.0, 0.0]), (b, [0.0, 1.0, 0.0]), (c, [0.9, 0.1, 0.0]), (s, [0.0, 0.0, 1.0]), (d, [0.0, 0.2, 1.0])]
    {
        ann.insert(unit, [0; 32], &v).expect("insert");
    }
    Fixture { db, ann, a, b, c, s, d }
}

impl<E: Target> Fixture<E> {
    fn snap(&self) -> UnitSnapshot<E> {
        self.db.snapshot().expect("snapshot")
    }

    fn query<'q>(&'q self, text: Option<&'q str>, vector: Option<&'q [f32]>, graph: Option<UnitId>) -> HybridQuery<'q> {
        let mut q = HybridQuery::new(10);
        q.text = text;
        q.vector = vector.map(|query| kerno_units::VectorLeg { index: &self.ann, query });
        q.graph = graph.map(|s| AssocQuery::new(vec![s], 1, 10, 1000));
        q
    }
}

fn rrf(weight: u64, rank: u64) -> u64 {
    weight * 1_000_000_000 / (60 + rank)
}

fn all_legs<E: Target>(f: &Fixture<E>) -> Vec<HybridHit> {
    f.snap().hybrid_search(&f.query(Some("rust"), Some(&RUST), Some(f.s))).expect("hybrid")
}

fn invalid(e: UnitError) -> String {
    match e {
        UnitError::InvalidArgument(m) => m,
        other => panic!("expected InvalidArgument, got {other:?}"),
    }
}

#[test]
fn fused_scores_are_the_weighted_rrf_of_the_leg_ranks() {
    let f = fixture::<Emulated<LogEngine>>();
    let snap = f.snap();
    let hits = all_legs(&f);

    let keyword: Vec<UnitId> = snap.search_text("rust", 100).unwrap().into_iter().map(|h| h.unit).collect();
    let vector: Vec<UnitId> =
        f.ann.search(&RUST, 100).unwrap().into_iter().map(|h| h.unit).filter(|u| *u != f.s).collect();
    let graph: Vec<UnitId> = snap
        .assoc_query(&AssocQuery::new(vec![f.s], 1, 10, 1000))
        .unwrap()
        .result
        .hits
        .iter()
        .map(|h| h.unit)
        .collect();
    assert_eq!((keyword.len(), graph.len()), (2, 2), "the fixture's legs");

    for h in &hits {
        let pos = |leg: &[UnitId]| leg.iter().position(|u| *u == h.unit).map(|i| u32::try_from(i + 1).unwrap());
        assert_eq!((h.ranks.keyword, h.ranks.vector, h.ranks.graph), (pos(&keyword), pos(&vector), pos(&graph)));
        let want: u64 =
            [h.ranks.keyword, h.ranks.vector, h.ranks.graph].into_iter().flatten().map(|r| rrf(1, u64::from(r))).sum();
        assert_eq!(h.score, want, "score of {}", h.unit);
    }
    let a = hits.iter().find(|h| h.unit == f.a).expect("a is a hit");
    assert_eq!((a.ranks.keyword, a.ranks.vector, a.ranks.graph), (Some(2), Some(1), None));
    assert_eq!(a.score, 16_393_442 + 16_129_032);
    assert_eq!(a.score, 32_522_474);
    assert!(hits.iter().all(|h| h.unit != f.s), "the seed is not a hit");
    let got: Vec<UnitId> = hits.iter().map(|h| h.unit).collect();
    assert!(got.contains(&f.b) && got.contains(&f.c) && got.contains(&f.d));
}

#[test]
fn ties_are_broken_by_unit_id() {
    let f = fixture::<Emulated<LogEngine>>();
    // `garbage` finds only b (keyword rank 1); the vector leg's rank 1 is d:
    // two units with one leg at rank 1 each, so equal scores.
    let mut q = f.query(Some("garbage"), None, None);
    let only_d = VectorOf(f.d);
    q.vector = Some(kerno_units::VectorLeg { index: &only_d, query: &RUST });
    let hits = f.snap().hybrid_search(&q).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].score, hits[1].score);
    assert_eq!(hits[0].score, rrf(1, 1));
    assert!(hits[0].unit < hits[1].unit, "equal scores come in unit id order");
}

/// A vector leg that answers one unit, for a controlled rank.
struct VectorOf(UnitId);

impl NearestUnits for VectorOf {
    fn nearest(&self, _: &[f32], _: usize) -> Result<Vec<UnitId>, UnitError> {
        Ok(vec![self.0])
    }
}

#[test]
fn the_weight_multiplies_before_the_division() {
    let f = fixture::<Emulated<LogEngine>>();
    let mut q = f.query(Some("garbage"), None, None);
    q.weights = HybridWeights { keyword: 3, ..HybridWeights::default() };
    let hits = f.snap().hybrid_search(&q).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].unit, f.b);
    assert_eq!(hits[0].score, 49_180_327);
}

/// A vector leg that counts how often it is asked.
struct Counting(Cell<usize>, UnitId);

impl NearestUnits for Counting {
    fn nearest(&self, _: &[f32], _: usize) -> Result<Vec<UnitId>, UnitError> {
        self.0.set(self.0.get() + 1);
        Ok(vec![self.1])
    }
}

#[test]
fn a_leg_of_weight_zero_is_not_run() {
    let f = fixture::<Emulated<LogEngine>>();
    let counting = Counting(Cell::new(0), f.a);
    let mut q = f.query(Some("rust"), None, None);
    q.vector = Some(kerno_units::VectorLeg { index: &counting, query: &RUST });
    q.weights = HybridWeights { vector: 0, ..HybridWeights::default() };
    let hits = f.snap().hybrid_search(&q).unwrap();
    assert_eq!(counting.0.get(), 0, "a leg of weight 0 is not asked");
    assert!(hits.iter().all(|h| h.ranks.vector.is_none()));
    assert_eq!(hits.len(), 2);

    q.weights = HybridWeights::default();
    f.snap().hybrid_search(&q).unwrap();
    assert_eq!(counting.0.get(), 1);

    // A graph leg of weight 0 does not remove its seeds either.
    let mut q = f.query(Some("memory"), None, Some(f.s));
    q.weights = HybridWeights { graph: 0, ..HybridWeights::default() };
    let hits = f.snap().hybrid_search(&q).unwrap();
    assert_eq!(hits[0].unit, f.s);
    assert!(hits.iter().all(|h| h.ranks.graph.is_none()));
}

#[test]
fn graph_seeds_are_not_hits() {
    let f = fixture::<Emulated<LogEngine>>();
    let seed_vector = [0.0, 0.0, 1.0];

    let with = f.snap().hybrid_search(&f.query(Some("memory"), Some(&seed_vector), Some(f.s))).unwrap();
    assert!(with.iter().all(|h| h.unit != f.s));
    let d = with.iter().find(|h| h.unit == f.d).expect("d is a hit");
    assert_eq!((d.ranks.keyword, d.ranks.vector), (Some(1), Some(1)), "the next unit takes rank 1");

    let without = f.snap().hybrid_search(&f.query(Some("memory"), Some(&seed_vector), None)).unwrap();
    let s = without.iter().find(|h| h.unit == f.s).expect("s is a hit without a graph leg");
    assert_eq!((s.ranks.keyword, s.ranks.vector), (Some(1), Some(1)));
}

#[test]
fn vector_hits_of_inactive_units_are_dropped() {
    let f = fixture::<Emulated<LogEngine>>();
    let before = f.snap().hybrid_search(&f.query(None, Some(&RUST), None)).unwrap();
    assert_eq!(before[0].unit, f.a);
    assert_eq!(before[1].unit, f.c);

    let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(f.a, None).unwrap();
    cs.delete(f.c, None).unwrap();
    cs.commit().unwrap();
    assert!(f.ann.contains(&f.a) && f.ann.contains(&f.c), "the index is stale on purpose");

    let after = f.snap().hybrid_search(&f.query(None, Some(&RUST), None)).unwrap();
    assert!(after.iter().all(|h| h.unit != f.a && h.unit != f.c), "{after:?}");
    assert_eq!(after.len(), 3);
    let ranks: Vec<_> = after.iter().map(|h| h.ranks.vector).collect();
    assert_eq!(ranks, [Some(1), Some(2), Some(3)], "ranks count after the filter");
}

#[test]
fn invalid_queries_are_refused() {
    let f = fixture::<Emulated<LogEngine>>();
    let run = |q: &HybridQuery<'_>| invalid(f.snap().hybrid_search(q).unwrap_err());
    let text = || f.query(Some("rust"), None, None);

    let mut q = text();
    q.limit = 0;
    assert_eq!(run(&q), "hybrid query: limit 0 is outside 1..=1000");
    q.limit = 1001;
    assert_eq!(run(&q), "hybrid query: limit 1001 is outside 1..=1000");
    let mut q = text();
    q.candidates = 0;
    assert_eq!(run(&q), "hybrid query: candidates 0 is outside 1..=1000");
    q.candidates = 1001;
    assert_eq!(run(&q), "hybrid query: candidates 1001 is outside 1..=1000");
    for (name, weights) in [
        ("keyword", HybridWeights { keyword: 1001, ..HybridWeights::default() }),
        ("vector", HybridWeights { vector: 1001, ..HybridWeights::default() }),
        ("graph", HybridWeights { graph: 1001, ..HybridWeights::default() }),
    ] {
        let mut q = text();
        q.weights = weights;
        assert_eq!(run(&q), format!("hybrid query: weight 1001 of the {name} leg is above 1000"));
    }
    let none = HybridQuery::new(10);
    assert_eq!(run(&none), "hybrid query: no leg to run (give text, vector or graph with a weight above 0)");
    let mut off = text();
    off.weights = HybridWeights { keyword: 0, ..HybridWeights::default() };
    assert!(run(&off).starts_with("hybrid query: no leg to run"), "a leg of weight 0 is no leg");

    let mut q = text();
    q.graph = Some(AssocQuery::new(Vec::new(), 1, 10, 1000));
    assert_eq!(run(&q), "hybrid query: graph leg: an associative query needs at least one seed");

    let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(f.a, None).unwrap();
    cs.commit().unwrap();
    let message = run(&f.query(Some("rust"), None, Some(f.a)));
    assert_eq!(message, format!("hybrid query: graph leg: seed {} is not an active unit at this snapshot", f.a));

    let mut set = VectorSet::new(2, VectorMetric::Cosine).unwrap();
    set.insert(f.b, vec![1.0, 0.0]).unwrap();
    let mut q = HybridQuery::new(10);
    q.vector = Some(kerno_units::VectorLeg { index: &set, query: &RUST });
    assert!(run(&q).starts_with("vector leg: "), "a dimension mismatch is the vector leg's");

    let mut q = HybridQuery::new(10);
    q.vector = Some(kerno_units::VectorLeg { index: &f.ann, query: &[1.0, 0.0] });
    assert!(run(&q).contains("dimension 3"), "the index's refusal passes through");
}

#[test]
fn no_caller_literal_in_debug_or_errors() {
    let f = fixture::<Emulated<LogEngine>>();
    let vector = [0.123, 0.456];
    let mut q = f.query(Some("zebra-secret"), Some(&vector), Some(f.s));
    let shown = format!("{q:?}");
    assert!(!shown.contains("zebra") && !shown.contains("0.123") && !shown.contains("0.456"), "{shown}");
    assert!(shown.contains("text_bytes") && shown.contains("vector_dimension"), "{shown}");

    let error = f.snap().hybrid_search(&q).unwrap_err().to_string();
    assert!(!error.contains("zebra") && !error.contains("0.123"), "{error}");
    q.limit = 0;
    let error = f.snap().hybrid_search(&q).unwrap_err().to_string();
    assert!(!error.contains("zebra") && !error.contains("0.123"), "{error}");
}

#[test]
fn both_engines_answer_identically() {
    let log = all_legs(&fixture::<Emulated<LogEngine>>());
    let kerno = all_legs(&fixture::<KernoDb>());
    assert!(!log.is_empty());
    assert_eq!(log, kerno);
}

// ------------------------------------------------------------ graph-gated fusion

/// A vector leg that answers a fixed list, for controlled ranks.
struct VectorList(Vec<UnitId>);

impl NearestUnits for VectorList {
    fn nearest(&self, _: &[f32], _: usize) -> Result<Vec<UnitId>, UnitError> {
        Ok(self.0.clone())
    }
}

fn order(hits: &[HybridHit]) -> Vec<UnitId> {
    hits.iter().map(|h| h.unit).collect()
}

/// Runs `q` under both fusion modes.
fn both_modes<E: Target>(f: &Fixture<E>, q: &mut HybridQuery<'_>) -> (Vec<HybridHit>, Vec<HybridHit>) {
    q.fusion = Fusion::Rrf;
    let union = f.snap().hybrid_search(q).unwrap();
    q.fusion = Fusion::GraphGated;
    let gated = f.snap().hybrid_search(q).unwrap();
    (union, gated)
}

#[test]
fn the_default_fusion_is_the_union() {
    assert_eq!(Fusion::default(), Fusion::Rrf);
    assert_eq!(HybridQuery::new(10).fusion, Fusion::Rrf);
    // A query that never names a mode orders by score alone: the vector-only
    // `a` (weight 2) outscores the graph hits.
    let f = fixture::<Emulated<LogEngine>>();
    let only_a = VectorOf(f.a);
    let mut q = f.query(None, None, Some(f.s));
    q.vector = Some(kerno_units::VectorLeg { index: &only_a, query: &RUST });
    q.weights = HybridWeights { vector: 2, ..HybridWeights::default() };
    assert_eq!(order(&f.snap().hybrid_search(&q).unwrap()), [f.a, f.b, f.d]);
}

#[test]
fn the_gate_puts_the_walks_hits_before_every_other_unit() {
    let f = fixture::<Emulated<LogEngine>>();
    // `a` is found by the vector leg alone, at weight 2: 32_786_885 against 16_393_442
    // for the graph's first hit. The union puts it first; the gate puts it last.
    let only_a = VectorOf(f.a);
    let mut q = f.query(None, None, Some(f.s));
    q.vector = Some(kerno_units::VectorLeg { index: &only_a, query: &RUST });
    q.weights = HybridWeights { vector: 2, ..HybridWeights::default() };
    let (union, gated) = both_modes(&f, &mut q);
    assert_eq!(order(&union), [f.a, f.b, f.d]);
    assert_eq!(order(&gated), [f.b, f.d, f.a]);
    assert!(union[0].score > union[1].score, "the union ranks `a` above the walk's hits by score");

    // The gate changes the order and nothing else: same units, scores and ranks.
    let by_unit = |hits: &[HybridHit]| {
        let mut v = hits.to_vec();
        v.sort_by_key(|h| h.unit);
        v
    };
    assert_eq!(by_unit(&union), by_unit(&gated));

    // The limit cuts after the gate: with room for two, `a` is out.
    q.limit = 2;
    assert_eq!(order(&f.snap().hybrid_search(&q).unwrap()), [f.b, f.d]);
}

#[test]
fn inside_the_gate_the_other_legs_still_reorder() {
    let f = fixture::<Emulated<LogEngine>>();
    // The walk finds b (rank 1) then d (rank 2). The vector leg ranks a, d:
    // d earns 16_129_032 twice and overtakes b; a, outside the walk, stays last
    // although its vector score ties b's graph score.
    let list = VectorList(vec![f.a, f.d]);
    let mut q = f.query(None, None, Some(f.s));
    q.vector = Some(kerno_units::VectorLeg { index: &list, query: &RUST });
    let (union, gated) = both_modes(&f, &mut q);
    assert_eq!(order(&union), [f.d, f.a, f.b]);
    assert_eq!(order(&gated), [f.d, f.b, f.a]);
    assert_eq!(gated[0].score, rrf(1, 2) * 2);
    assert_eq!(gated[1].score, rrf(1, 1));

    // The graph leg alone keeps its own order: the reorder came from the vector.
    let alone = f.snap().hybrid_search(&f.query(None, None, Some(f.s))).unwrap();
    assert_eq!(order(&alone), [f.b, f.d]);
}

#[test]
fn the_gate_keeps_the_hop_order_of_the_walk() {
    let f = fixture::<Emulated<LogEngine>>();
    // e -> d: e is two hops from the seed s; b and d are one hop away.
    let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
    let e = cs.create(new_unit("far")).unwrap();
    cs.create_relation(e, f.d, "about", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();

    // The vector leg ranks e first at weight 2: 32_786_885 + its graph rank 3
    // (15_873_015) outscores both one-hop units.
    let only_e = VectorOf(e);
    let mut q = f.query(None, None, None);
    q.graph = Some(AssocQuery::new(vec![f.s], 2, 10, 1000));
    q.vector = Some(kerno_units::VectorLeg { index: &only_e, query: &RUST });
    q.weights = HybridWeights { vector: 2, ..HybridWeights::default() };
    let (union, gated) = both_modes(&f, &mut q);
    assert_eq!(order(&union), [e, f.b, f.d]);
    assert_eq!(order(&gated), [f.b, f.d, e], "a nearer hop leads whatever the other legs say");
    let far = gated.iter().find(|h| h.unit == e).unwrap();
    assert_eq!((far.ranks.graph, far.ranks.vector), (Some(3), Some(1)));
    assert_eq!(far.score, 15_873_015 + 32_786_885, "the gate does not change a score");
}

#[test]
fn without_a_walk_hit_the_gate_changes_nothing() {
    let f = fixture::<Emulated<LogEngine>>();
    // No graph leg.
    let mut q = f.query(Some("rust"), Some(&RUST), None);
    let (union, gated) = both_modes(&f, &mut q);
    assert!(union.len() >= 2);
    assert_eq!(union, gated);
    // A graph leg of weight 0 does not run.
    let mut q = f.query(Some("rust"), Some(&RUST), Some(f.s));
    q.weights = HybridWeights { graph: 0, ..HybridWeights::default() };
    let (union, gated) = both_modes(&f, &mut q);
    assert!(union.iter().all(|h| h.ranks.graph.is_none()));
    assert_eq!(union, gated);
    // A walk that finds nothing: `c` has no relation, so its seed has no hit.
    let mut q = f.query(Some("rust"), Some(&RUST), Some(f.c));
    let (union, gated) = both_modes(&f, &mut q);
    assert!(union.iter().all(|h| h.ranks.graph.is_none()) && union.len() >= 2);
    assert_eq!(union, gated);
}

#[test]
fn the_gate_keeps_the_rules_of_the_legs() {
    let f = fixture::<Emulated<LogEngine>>();
    // Seeds are not hits.
    let seed_vector = [0.0, 0.0, 1.0];
    let mut q = f.query(Some("memory"), Some(&seed_vector), Some(f.s));
    q.fusion = Fusion::GraphGated;
    let hits = f.snap().hybrid_search(&q).unwrap();
    assert!(hits.iter().all(|h| h.unit != f.s));
    let d = hits.iter().find(|h| h.unit == f.d).expect("d is a hit");
    assert_eq!((d.ranks.keyword, d.ranks.vector), (Some(1), Some(1)));

    // An archived unit the stale index still holds is dropped.
    let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(f.a, None).unwrap();
    cs.commit().unwrap();
    let mut q = f.query(None, Some(&RUST), Some(f.s));
    q.fusion = Fusion::GraphGated;
    let hits = f.snap().hybrid_search(&q).unwrap();
    assert!(hits.iter().all(|h| h.unit != f.a), "{hits:?}");
    assert!(hits.iter().take_while(|h| h.ranks.graph.is_some()).count() == 2, "the walk's hits lead: {hits:?}");

    // The query's text and vector stay out of its Debug; the mode is in it.
    let shown = format!("{q:?}");
    assert!(shown.contains("fusion: GraphGated"), "{shown}");
}

fn gated_run<E: Target>(f: &Fixture<E>) -> Vec<HybridHit> {
    let list = VectorList(vec![f.a, f.d]);
    let mut q = f.query(Some("rust"), None, Some(f.s));
    q.vector = Some(kerno_units::VectorLeg { index: &list, query: &RUST });
    q.fusion = Fusion::GraphGated;
    f.snap().hybrid_search(&q).unwrap()
}

#[test]
fn both_engines_answer_identically_when_gated() {
    let log = gated_run(&fixture::<Emulated<LogEngine>>());
    let kerno = gated_run(&fixture::<KernoDb>());
    assert!(log.len() >= 3);
    assert_eq!(log, kerno);
}
