//! E7-04: the ranking of associative results, its diagnostic mode and the
//! exact vector signal (`UnitSnapshot::rank_assoc`, `VectorSet`) over
//! `Emulated<LogEngine>` and `KernoDb`.
//!
//! * `contributions_add_up_to_the_score_and_match_an_oracle` (I-41): over
//!   generated graphs, queries, weights and vectors, the score of each hit is
//!   the sum of its five contributions, each contribution is what an oracle
//!   written here from the raw relations and units says, and the order is
//!   (score descending, unit id ascending).
//! * `hand_vectors_*`: a small world whose every number was worked out by hand
//!   with the default weights, with and without vectors.
//! * `same_state_and_query_give_the_same_order`: repeated runs, and the
//!   reference, indexed and planner paths, give one ranking.
//! * `ties_*`, `ranking_off_*`, `every_vector_rejection_*`,
//!   `similarity_matches_an_independent_oracle`, `sentinels_*`.
//!
//! The expected values never come from `assoc_rank.rs`'s own helpers, and the
//! frozen evaluation set is not read.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Engine, Options, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;
use kerno_units::assoc_rank::*;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

// ---------------------------------------------------------------- fixtures

fn open<E: TxnEngine>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open")
}

fn new_unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: Vec::new(), payload: payload.to_vec() }
}

fn units<E: TxnEngine>(db: &UnitDb<E>, n: usize) -> Vec<UnitId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    let ids = (0..n).map(|i| cs.create(new_unit(&[i as u8])).expect("create")).collect();
    cs.commit().expect("commit");
    ids
}

/// One change set creating `rels` (from, to, type, provenance).
fn relate<E: TxnEngine>(db: &UnitDb<E>, rels: &[(UnitId, UnitId, &str, RelationProvenance)]) -> CommitSeq {
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    for (from, to, ty, p) in rels {
        cs.create_relation(*from, *to, ty, p.clone()).expect("create_relation");
    }
    cs.commit().expect("commit").commit_seq
}

fn touch<E: TxnEngine>(db: &UnitDb<E>, unit: UnitId, payload: &[u8]) -> CommitSeq {
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    cs.update(unit, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None)
        .expect("update");
    cs.commit().expect("commit").commit_seq
}

fn imported() -> RelationProvenance {
    RelationProvenance::Imported { source: b"wiki".to_vec() }
}

fn inferred() -> RelationProvenance {
    RelationProvenance::Inferred { origin: b"model".to_vec(), confidence_permille: 700 }
}

fn algorithmic() -> RelationProvenance {
    RelationProvenance::Algorithmic { origin: b"dedup".to_vec() }
}

const TYPES: [&str; 3] = ["cites", "mentions", "refines"];

fn random_provenance(rng: &mut Rng) -> RelationProvenance {
    match rng.below(4) {
        0 => RelationProvenance::Declared,
        1 => imported(),
        2 => algorithmic(),
        _ => inferred(),
    }
}

/// A graph of 8 to 14 units, relations over many change sets (self and
/// parallel ones included) and a few later updates, so that the head
/// revisions have different commit sequences.
fn world<E: TxnEngine>(rng: &mut Rng) -> (UnitDb<E>, Vec<UnitId>) {
    let db = open::<E>();
    let u = units(&db, 8 + rng.below(7) as usize);
    for _ in 0..4 + rng.below(5) {
        let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
        for _ in 0..rng.range(2, 7) {
            let (from, to) = (u[rng.below(u.len() as u64) as usize], u[rng.below(u.len() as u64) as usize]);
            let ty = TYPES[rng.below(TYPES.len() as u64) as usize];
            cs.create_relation(from, to, ty, random_provenance(rng)).expect("create_relation");
        }
        cs.commit().expect("commit");
        if rng.chance(1, 2) {
            touch(&db, u[rng.below(u.len() as u64) as usize], &[rng.below(256) as u8, 1]);
        }
    }
    (db, u)
}

fn random_query(rng: &mut Rng, u: &[UnitId]) -> AssocQuery {
    let seeds = (0..rng.range(1, 3)).map(|_| u[rng.below(u.len() as u64) as usize]).collect();
    let limit = [2usize, 5, 1000][rng.below(3) as usize];
    let mut q = AssocQuery::new(seeds, rng.range(1, 4) as u32, limit, 100_000);
    q.direction = [AssocDirection::Out, AssocDirection::In, AssocDirection::Both][rng.below(3) as usize];
    if rng.chance(1, 3) {
        q.relation_types = Some(TYPES.iter().filter(|_| rng.chance(2, 3)).map(|t| (*t).to_owned()).collect());
        if q.relation_types.as_ref().is_some_and(BTreeSet::is_empty) {
            q.relation_types = None;
        }
    }
    q
}

/// Eighths in `-8..=8`: every value, product and sum of a few of them is
/// exact in `f32` and `f64`, so the oracle below can work in integers.
fn eighths(rng: &mut Rng, dim: usize) -> Vec<i32> {
    (0..dim).map(|_| rng.range(0, 17) as i32 - 8).collect()
}

fn as_f32(v: &[i32]) -> Vec<f32> {
    v.iter().map(|&k| k as f32 / 8.0).collect()
}

fn random_weights(rng: &mut Rng) -> RankWeights {
    let w = |rng: &mut Rng| rng.range(0, 3000) as i32 - 500;
    RankWeights { hops: w(rng), relation_type: w(rng), recency: w(rng), provenance: w(rng), similarity: w(rng) }
}

fn opts<'a>(mode: RankMode, vectors: Option<(&'a VectorSet, &'a VectorQuery)>) -> RankOptions<'a> {
    RankOptions { mode, vectors: vectors.map(|(set, query)| VectorSignal { set, query }), ..RankOptions::default() }
}

fn order(hits: &[RankedHit]) -> Vec<UnitId> {
    hits.iter().map(|h| h.unit).collect()
}

// ------------------------------------------------- the I-41 oracle

/// What the five contributions of `hit` must be, worked out here from the raw
/// relations and units of the snapshot (not from the trace, not from the
/// crate's helpers).
fn oracle<E: Engine>(
    snap: &UnitSnapshot<E>,
    hit: &AssocHit,
    w: &RankWeights,
    preferred: &BTreeSet<String>,
    sim: Option<i64>,
) -> [i64; 5] {
    let rels: Vec<Relation> =
        hit.path.iter().map(|s| snap.get_relation(s.relation).unwrap().expect("relation")).collect();
    let hops = i64::from(w.hops) * (5 - i64::from(hit.distance));

    let preferred_steps = rels.iter().filter(|r| preferred.contains(&r.relation_type)).count() as i64;
    let rel_type = i64::from(w.relation_type) * preferred_steps / rels.len() as i64;

    let trust = |r: &Relation| match r.provenance {
        RelationProvenance::Declared => 3,
        RelationProvenance::Imported { .. } => 2,
        RelationProvenance::Algorithmic { .. } => 1,
        RelationProvenance::Inferred { .. } => 0,
    };
    let provenance = i64::from(w.provenance) * rels.iter().map(trust).min().unwrap();

    let head = snap.get_unit(hit.unit).unwrap().expect("unit").updated_seq.0;
    let newest = head.max(rels.last().unwrap().created_seq.0);
    let age = (snap.seq().0 - newest) as i64;
    let recency = i64::from(w.recency) * 64 / (64 + age);

    let similarity = sim.map_or(0, |s| i64::from(w.similarity) * s.clamp(-1_000_000, 1_000_000) / 1_000_000);
    [hops, rel_type, recency, provenance, similarity]
}

fn contributions_add_up_to_the_score_and_match_an_oracle<E: TxnEngine>() {
    let mut ranked_hits = 0;
    let mut with_vectors = 0;
    let mut with_negative = 0;
    for seed in 0..8 {
        let mut rng = Rng::new(0xE704 + seed);
        let (db, u) = world::<E>(&mut rng);
        // A vector for most units, of the same set for both metrics' runs.
        for metric in [VectorMetric::Cosine, VectorMetric::Dot] {
            let mut set = VectorSet::new(3, metric).unwrap();
            for &unit in &u {
                if rng.chance(4, 5) {
                    let _ = set.insert(unit, as_f32(&eighths(&mut rng, 3)));
                }
            }
            let probe = loop {
                let v = eighths(&mut rng, 3);
                if let Ok(q) = VectorQuery::new(metric, as_f32(&v)) {
                    break q;
                }
            };
            let snap = db.snapshot().unwrap();
            for _ in 0..25 {
                let q = random_query(&mut rng, &u);
                let result = snap.reference_assoc_query(&q).unwrap();
                let w = random_weights(&mut rng);
                let preferred: BTreeSet<String> =
                    TYPES.iter().filter(|_| rng.chance(1, 2)).map(|t| (*t).to_owned()).collect();
                let use_vectors = rng.chance(2, 3);
                let options = RankOptions {
                    mode: RankMode::Diagnostic,
                    weights: w,
                    preferred_types: preferred.clone(),
                    vectors: use_vectors.then_some(VectorSignal { set: &set, query: &probe }),
                };
                let ranked = snap.rank_assoc(&result, &options).unwrap();
                assert_eq!(ranked.len(), result.hits.len());
                for r in &ranked {
                    let hit = result.hits.iter().find(|h| h.unit == r.unit).expect("a ranked hit is a hit");
                    let sim = if use_vectors { set.similarity(r.unit, &probe).unwrap() } else { None };
                    let want = oracle(&snap, hit, &w, &preferred, sim);
                    let c = r.contributions.expect("diagnostic mode gives the contributions");
                    assert_eq!(
                        [c.hops, c.relation_type, c.recency, c.provenance, c.similarity],
                        want,
                        "contributions of {:?} at seed {seed}",
                        r.unit
                    );
                    // I-41: the score is exactly the sum.
                    assert_eq!(r.score, Some(want.iter().sum()), "score is the sum of the contributions");
                    assert_eq!(r.score, Some(c.sum()));
                    assert_eq!(r.similarity, sim);
                    assert_eq!(r.distance, hit.distance);
                    ranked_hits += 1;
                    with_vectors += u32::from(sim.is_some());
                    with_negative += u32::from(want.iter().any(|&x| x < 0));
                }
                for pair in ranked.windows(2) {
                    let key = |h: &RankedHit| (std::cmp::Reverse(h.score), h.unit);
                    assert!(key(&pair[0]) < key(&pair[1]), "order is (score descending, id ascending)");
                }
                // `On` ranks exactly like `Diagnostic` and shows no contributions.
                let plain = snap.rank_assoc(&result, &RankOptions { mode: RankMode::On, ..options.clone() }).unwrap();
                assert_eq!(order(&plain), order(&ranked));
                assert!(plain.iter().all(|h| h.contributions.is_none() && h.similarity.is_none() && h.score.is_some()));
            }
        }
    }
    assert!(ranked_hits > 400, "the run must rank many hits: {ranked_hits}");
    assert!(with_vectors > 100, "the run must use the vector signal: {with_vectors}");
    assert!(with_negative > 20, "the run must cover negative contributions: {with_negative}");
}

// ------------------------------------------------- hand vectors

/// `a` reaches `b` and `c` in one hop and `d` and `e` in two:
///
/// ```text
/// seq 1  units a b c d e            seq 2  a -cites-> b   (Declared)
/// seq 3  a -mentions-> c (Inferred) seq 4  b -cites-> d   (Declared)
/// seq 5  c -cites-> e (Imported)    seq 6  d updated
/// ```
fn hand_world<E: TxnEngine>() -> (UnitDb<E>, [UnitId; 5]) {
    let db = open::<E>();
    let ids = units(&db, 5);
    let [a, b, c, d, e] = [ids[0], ids[1], ids[2], ids[3], ids[4]];
    assert_eq!(relate(&db, &[(a, b, "cites", RelationProvenance::Declared)]), CommitSeq(2));
    assert_eq!(relate(&db, &[(a, c, "mentions", inferred())]), CommitSeq(3));
    assert_eq!(relate(&db, &[(b, d, "cites", RelationProvenance::Declared)]), CommitSeq(4));
    assert_eq!(relate(&db, &[(c, e, "cites", imported())]), CommitSeq(5));
    assert_eq!(touch(&db, d, b"d2"), CommitSeq(6));
    (db, [a, b, c, d, e])
}

fn hand_query(a: UnitId) -> AssocQuery {
    let mut q = AssocQuery::new(vec![a], 2, 100, 10_000);
    q.direction = AssocDirection::Out;
    q
}

fn cites() -> BTreeSet<String> {
    BTreeSet::from(["cites".to_owned()])
}

fn hand_vectors_without_a_vector_signal<E: TxnEngine>() {
    let (db, [a, b, c, d, e]) = hand_world::<E>();
    let snap = db.snapshot().unwrap();
    let result = snap.reference_assoc_query(&hand_query(a)).unwrap();
    assert_eq!(order_of(&result), vec![b, c, d, e], "the reference order: distance, then id");

    let options = RankOptions { mode: RankMode::Diagnostic, preferred_types: cites(), ..RankOptions::default() };
    let ranked = snap.rank_assoc(&result, &options).unwrap();
    let got: Vec<(UnitId, i64, [i64; 5])> = ranked
        .iter()
        .map(|h| {
            let c = h.contributions.unwrap();
            (h.unit, h.score.unwrap(), [c.hops, c.relation_type, c.recency, c.provenance, c.similarity])
        })
        .collect();
    // hops, type, recency, provenance, similarity; age = 6 - newest.
    let want = vec![
        (b, 5626, [4000, 500, 376, 750, 0]), // newest = seq 2 (age 4): 25600 / 68
        (d, 4650, [3000, 500, 400, 750, 0]), // newest = seq 6 (age 0)
        (c, 4382, [4000, 0, 382, 0, 0]),     // newest = seq 3 (age 3): 25600 / 67; Inferred
        (e, 3643, [3000, 250, 393, 0, 0]),   // newest = seq 5 (age 1): 25600 / 65; 1 of 2 steps, Inferred weakest
    ];
    assert_eq!(got, want);
    // Without preferred types the signal is 0 for everyone.
    let none = snap.rank_assoc(&result, &RankOptions { mode: RankMode::Diagnostic, ..RankOptions::default() }).unwrap();
    assert!(none.iter().all(|h| h.contributions.unwrap().relation_type == 0));
}

fn hand_vectors_with_cosine_and_dot<E: TxnEngine>() {
    let (db, [a, b, c, d, e]) = hand_world::<E>();
    let snap = db.snapshot().unwrap();
    let result = snap.reference_assoc_query(&hand_query(a)).unwrap();

    let mut set = VectorSet::new(2, VectorMetric::Cosine).unwrap();
    set.insert(d, vec![1.0, 0.0]).unwrap();
    set.insert(c, vec![0.0, 1.0]).unwrap();
    set.insert(b, vec![-3.0, 0.0]).unwrap();
    let query = VectorQuery::new(VectorMetric::Cosine, vec![2.0, 0.0]).unwrap();
    let options = RankOptions {
        mode: RankMode::Diagnostic,
        preferred_types: cites(),
        vectors: Some(VectorSignal { set: &set, query: &query }),
        ..RankOptions::default()
    };
    let ranked = snap.rank_assoc(&result, &options).unwrap();
    // The scores of the previous test, moved by +1500 (d, cos 1), 0 (c, cos 0),
    // -1500 (b, cos -1); e has no vector.
    let got: Vec<(UnitId, i64, i64, Option<i64>)> =
        ranked.iter().map(|h| (h.unit, h.score.unwrap(), h.contributions.unwrap().similarity, h.similarity)).collect();
    assert_eq!(
        got,
        vec![
            (d, 6150, 1500, Some(1_000_000)),
            (c, 4382, 0, Some(0)),
            (b, 4126, -1500, Some(-1_000_000)),
            (e, 3643, 0, None),
        ]
    );

    // Dot product: 0.25 stays, 2.0 saturates at 1.0.
    let mut set = VectorSet::new(2, VectorMetric::Dot).unwrap();
    set.insert(d, vec![2.0, 2.0]).unwrap();
    set.insert(b, vec![0.25, 0.25]).unwrap();
    let query = VectorQuery::new(VectorMetric::Dot, vec![0.5, 0.5]).unwrap();
    let options = RankOptions {
        mode: RankMode::Diagnostic,
        vectors: Some(VectorSignal { set: &set, query: &query }),
        ..RankOptions::default()
    };
    let ranked = snap.rank_assoc(&result, &options).unwrap();
    let sim = |unit: UnitId| ranked.iter().find(|h| h.unit == unit).unwrap();
    assert_eq!(sim(d).similarity, Some(2_000_000), "the exact dot product is reported unclamped");
    assert_eq!(sim(d).contributions.unwrap().similarity, 1500, "and clamped to 1.0 in the score");
    assert_eq!(sim(b).similarity, Some(250_000));
    assert_eq!(sim(b).contributions.unwrap().similarity, 375);
    assert_eq!(sim(c).similarity, None);
    assert_eq!(sim(e).contributions.unwrap().similarity, 0);

    // The default weights are the documented constants.
    let w = RankWeights::default();
    assert_eq!((w.hops, w.relation_type, w.recency, w.provenance, w.similarity), (1000, 500, 400, 250, 1500));
    assert_eq!((RECENCY_HALF_LIFE, SIMILARITY_SCALE), (64, 1_000_000));
}

// ------------------------------------------------- determinism

fn same_state_and_query_give_the_same_order<E: TxnEngine>() {
    let mut paths = 0;
    for seed in 0..6 {
        let mut rng = Rng::new(0xDE7E + seed);
        let (db, u) = world::<E>(&mut rng);
        let mut set = VectorSet::new(3, VectorMetric::Cosine).unwrap();
        for &unit in &u {
            let _ = set.insert(unit, as_f32(&eighths(&mut rng, 3)));
        }
        let probe = VectorQuery::new(VectorMetric::Cosine, vec![0.5, -0.25, 1.0]).unwrap();
        let snap = db.snapshot().unwrap();
        for _ in 0..30 {
            let q = random_query(&mut rng, &u);
            let by_reference = snap.reference_assoc_query(&q).unwrap();
            let by_index = snap.indexed_assoc_query(&q).unwrap();
            let by_planner = snap.assoc_query(&q).unwrap().result;
            let preferred = cites();
            let rank = |r: &AssocResult| {
                let options = RankOptions {
                    mode: RankMode::Diagnostic,
                    preferred_types: preferred.clone(),
                    vectors: Some(VectorSignal { set: &set, query: &probe }),
                    ..RankOptions::default()
                };
                snap.rank_assoc(r, &options).unwrap()
            };
            let first = rank(&by_reference);
            // Whole rankings, contributions included, are equal, on every path and run.
            for r in [&by_reference, &by_index, &by_planner, &by_reference, &by_index] {
                assert_eq!(rank(r), first);
                paths += 1;
            }
            // A snapshot taken again from the same state ranks the same.
            let again = db.snapshot().unwrap();
            let options =
                RankOptions { mode: RankMode::On, preferred_types: preferred.clone(), ..RankOptions::default() };
            assert_eq!(
                order(&again.rank_assoc(&by_reference, &options).unwrap()),
                order(&snap.rank_assoc(&by_reference, &options).unwrap())
            );
        }
    }
    assert!(paths > 500);
}

// ------------------------------------------------- ties and off

fn ties_are_broken_by_identifier<E: TxnEngine>() {
    // Four leaves of one seed, one change set, one type: every signal ties.
    let db = open::<E>();
    let u = units(&db, 5);
    relate(&db, &u[1..].iter().map(|&l| (u[0], l, "cites", RelationProvenance::Declared)).collect::<Vec<_>>());
    let snap = db.snapshot().unwrap();
    let mut q = AssocQuery::new(vec![u[0]], 1, 10, 100);
    q.direction = AssocDirection::Out;
    let result = snap.reference_assoc_query(&q).unwrap();
    let ranked = snap.rank_assoc(&result, &opts(RankMode::Diagnostic, None)).unwrap();
    assert_eq!(ranked.len(), 4);
    assert!(ranked.iter().all(|h| h.score == ranked[0].score), "all four tie");
    assert_eq!(order(&ranked), u[1..].to_vec(), "ties go in ascending identifier order");

    // With every weight 0 all scores tie, across distances: pure identifier
    // order, which differs from the reference order (distance first).
    let db = open::<E>();
    let u = units(&db, 3); // s < far < near
    let (s, far, near) = (u[0], u[1], u[2]);
    relate(
        &db,
        &[(s, near, "cites", RelationProvenance::Declared), (near, far, "cites", RelationProvenance::Declared)],
    );
    let snap = db.snapshot().unwrap();
    let mut q = AssocQuery::new(vec![s], 2, 10, 100);
    q.direction = AssocDirection::Out;
    let result = snap.reference_assoc_query(&q).unwrap();
    assert_eq!(result.hits.iter().map(|h| h.unit).collect::<Vec<_>>(), vec![near, far]);
    let zero = RankWeights { hops: 0, relation_type: 0, recency: 0, provenance: 0, similarity: 0 };
    let ranked =
        snap.rank_assoc(&result, &RankOptions { mode: RankMode::On, weights: zero, ..RankOptions::default() }).unwrap();
    assert_eq!(order(&ranked), vec![far, near]);
    assert!(ranked.iter().all(|h| h.score == Some(0)));
}

fn ranking_off_is_the_reference_order<E: TxnEngine>() {
    let mut reordered = 0;
    for seed in 0..5 {
        let mut rng = Rng::new(0x0FF + seed);
        let (db, u) = world::<E>(&mut rng);
        let snap = db.snapshot().unwrap();
        for _ in 0..40 {
            let q = random_query(&mut rng, &u);
            let by_reference = snap.reference_assoc_query(&q).unwrap();
            let by_index = snap.indexed_assoc_query(&q).unwrap();
            let want = order_of(&by_reference);
            // Off is the default.
            assert_eq!(RankOptions::default().mode, RankMode::Off);
            for result in [&by_reference, &by_index] {
                let off = snap.rank_assoc(result, &RankOptions::default()).unwrap();
                assert_eq!(order(&off), want, "ranking off keeps the order of the reference path");
                assert!(off.iter().zip(&result.hits).all(|(r, h)| r.distance == h.distance));
                assert!(off.iter().all(|h| h.score.is_none() && h.contributions.is_none() && h.similarity.is_none()));
            }
            // The reference order itself is (distance, id).
            let by_distance: Vec<(u32, UnitId)> = by_reference.hits.iter().map(|h| (h.distance, h.unit)).collect();
            assert!(by_distance.windows(2).all(|p| p[0] <= p[1]), "hops, then id");
            let on = snap.rank_assoc(&by_reference, &opts(RankMode::On, None)).unwrap();
            reordered += u32::from(order(&on) != want);
        }
    }
    assert!(reordered > 10, "ranking on must differ from the reference order somewhere: {reordered}");
}

fn order_of(r: &AssocResult) -> Vec<UnitId> {
    r.hits.iter().map(|h| h.unit).collect()
}

fn a_result_from_another_state_is_refused_unless_ranking_is_off<E: TxnEngine>() {
    let (db, [a, b, ..]) = hand_world::<E>();
    let old = db.snapshot().unwrap();
    let result = old.reference_assoc_query(&hand_query(a)).unwrap();
    touch(&db, b, b"b2");
    let new = db.snapshot().unwrap();
    for mode in [RankMode::On, RankMode::Diagnostic] {
        assert!(matches!(new.rank_assoc(&result, &opts(mode, None)), Err(RankError::NotFromThisSnapshot)));
    }
    // Off reads nothing: it is the result's own order.
    assert_eq!(order(&new.rank_assoc(&result, &RankOptions::default()).unwrap()), order_of(&result));
    // On the snapshot it ran on, all is well.
    old.rank_assoc(&result, &opts(RankMode::On, None)).unwrap();
}

// ------------------------------------------------- vectors

fn every_vector_rejection<E: TxnEngine>() {
    let u = UnitId([7; 16]);
    // The set.
    assert_eq!(VectorSet::new(0, VectorMetric::Dot).unwrap_err(), VectorError::ZeroDimension);
    for metric in [VectorMetric::Cosine, VectorMetric::Dot] {
        let mut set = VectorSet::new(3, metric).unwrap();
        assert_eq!(set.insert(u, vec![]).unwrap_err(), VectorError::Empty);
        assert_eq!(set.insert(u, vec![1.0, 2.0]).unwrap_err(), VectorError::DimensionMismatch { expected: 3, got: 2 });
        assert_eq!(set.insert(u, vec![1.0; 4]).unwrap_err(), VectorError::DimensionMismatch { expected: 3, got: 4 });
        assert_eq!(set.insert(u, vec![1.0, f32::NAN, 1.0]).unwrap_err(), VectorError::NonFinite { index: 1 });
        assert_eq!(set.insert(u, vec![1.0, 1.0, f32::INFINITY]).unwrap_err(), VectorError::NonFinite { index: 2 });
        assert_eq!(set.insert(u, vec![f32::NEG_INFINITY, 1.0, 1.0]).unwrap_err(), VectorError::NonFinite { index: 0 });
        assert!(set.is_empty(), "a rejected vector is not stored");
        // The query vector.
        assert_eq!(VectorQuery::new(metric, vec![]).unwrap_err(), VectorError::Empty);
        assert_eq!(VectorQuery::new(metric, vec![0.0, f32::NAN]).unwrap_err(), VectorError::NonFinite { index: 1 });
    }
    // A zero vector: rejected under cosine, a value under dot (and -0.0 is zero).
    let mut cos = VectorSet::new(2, VectorMetric::Cosine).unwrap();
    assert_eq!(cos.insert(u, vec![0.0, -0.0]).unwrap_err(), VectorError::ZeroVector);
    assert_eq!(VectorQuery::new(VectorMetric::Cosine, vec![0.0, 0.0]).unwrap_err(), VectorError::ZeroVector);
    cos.insert(u, vec![0.0, 1e-30]).unwrap();
    let mut dot = VectorSet::new(2, VectorMetric::Dot).unwrap();
    dot.insert(u, vec![0.0, 0.0]).unwrap();
    VectorQuery::new(VectorMetric::Dot, vec![0.0, 0.0]).unwrap();
    // The comparison: metric first, then dimension.
    let q3 = VectorQuery::new(VectorMetric::Cosine, vec![1.0, 1.0, 1.0]).unwrap();
    let qdot = VectorQuery::new(VectorMetric::Dot, vec![1.0, 1.0]).unwrap();
    assert_eq!(cos.check(&q3).unwrap_err(), VectorError::DimensionMismatch { expected: 2, got: 3 });
    assert_eq!(
        cos.check(&qdot).unwrap_err(),
        VectorError::MetricMismatch { set: VectorMetric::Cosine, query: VectorMetric::Dot }
    );
    let both = VectorQuery::new(VectorMetric::Dot, vec![1.0; 3]).unwrap();
    assert!(matches!(cos.check(&both), Err(VectorError::MetricMismatch { .. })));
    assert!(cos.search(&q3, 1).is_err() && cos.similarity(u, &qdot).is_err());

    // "Before any ranking": every mode refuses, even with nothing to rank.
    let db = open::<E>();
    let ids = units(&db, 2);
    relate(&db, &[(ids[0], ids[1], "cites", RelationProvenance::Declared)]);
    let snap = db.snapshot().unwrap();
    let mut q = AssocQuery::new(vec![ids[0]], 1, 5, 50);
    q.direction = AssocDirection::Out;
    let result = snap.reference_assoc_query(&q).unwrap();
    let empty = AssocResult { hits: Vec::new(), ending: AssocEnding::Exhausted, visited: 0 };
    for r in [&result, &empty] {
        for mode in [RankMode::Off, RankMode::On, RankMode::Diagnostic] {
            for (set, query) in [(&cos, &q3), (&cos, &qdot)] {
                let err = snap.rank_assoc(r, &opts(mode, Some((set, query)))).unwrap_err();
                assert!(matches!(err, RankError::Vector(_)), "{mode:?}: {err}");
            }
        }
    }
    let ok = VectorQuery::new(VectorMetric::Cosine, vec![1.0, 1.0]).unwrap();
    snap.rank_assoc(&result, &opts(RankMode::On, Some((&cos, &ok)))).unwrap();
}

/// `m` is `round(1e6 * d / sqrt(n))` up to a tie, checked in exact integers:
/// `m - 1/2 <= 1e6 d / sqrt(n) <= m + 1/2`.
fn is_rounded_cosine(d: i128, n: i128, m: i64) -> bool {
    if d == 0 {
        return m == 0;
    }
    if (d > 0) != (m >= 0) && m != 0 {
        return false;
    }
    let (d, m) = (d.abs(), i128::from(m.abs()));
    let v2 = 4 * 1_000_000_i128.pow(2) * d * d;
    let upper = (2 * m + 1).pow(2) * n;
    let lower = if m == 0 { i128::MIN } else { (2 * m - 1).pow(2) * n };
    lower <= v2 && v2 <= upper
}

/// The vectors never touch an engine, so this one runs once.
#[test]
fn similarity_matches_an_independent_oracle() {
    let mut rng = Rng::new(0x51);
    let ids: Vec<UnitId> = (1..=40u8).map(|i| UnitId([i; 16])).collect();
    let mut checked = 0;
    for metric in [VectorMetric::Cosine, VectorMetric::Dot] {
        for dim in [1usize, 2, 3, 7, 16] {
            let mut set = VectorSet::new(dim, metric).unwrap();
            let mut ints: Vec<(UnitId, Vec<i32>)> = Vec::new();
            for &id in &ids {
                let v = eighths(&mut rng, dim);
                if set.insert(id, as_f32(&v)).is_ok() {
                    ints.push((id, v));
                }
            }
            for _ in 0..6 {
                let (qi, query) = loop {
                    let v = eighths(&mut rng, dim);
                    if let Ok(q) = VectorQuery::new(metric, as_f32(&v)) {
                        break (v, q);
                    }
                };
                let dot = |a: &[i32], b: &[i32]| a.iter().zip(b).map(|(x, y)| i128::from(x * y)).sum::<i128>();
                let right = |unit: UnitId, got: i64| {
                    let v = &ints.iter().find(|(id, _)| *id == unit).unwrap().1;
                    match metric {
                        // Components are k/8: the dot is D/64 and 1e6 * D/64 = 15625 * D.
                        VectorMetric::Dot => i128::from(got) == 15_625 * dot(v, &qi),
                        VectorMetric::Cosine => is_rounded_cosine(dot(v, &qi), dot(v, v) * dot(&qi, &qi), got),
                    }
                };
                let all = set.search(&query, usize::MAX).unwrap();
                assert_eq!(all.len(), ints.len(), "brute force compares every vector, once");
                let distinct: BTreeSet<UnitId> = all.iter().map(|s| s.unit).collect();
                assert_eq!(distinct.len(), ints.len());
                for s in &all {
                    assert!(right(s.unit, s.similarity), "{metric:?} dim {dim}: {s:?} against the oracle");
                    assert_eq!(set.similarity(s.unit, &query).unwrap(), Some(s.similarity));
                    checked += 1;
                }
                // Sorted by (similarity descending, id ascending).
                for p in all.windows(2) {
                    assert!(
                        (std::cmp::Reverse(p[0].similarity), p[0].unit)
                            < (std::cmp::Reverse(p[1].similarity), p[1].unit)
                    );
                }
                // The top k is a prefix; a unit left out is no better than the last one in.
                for k in [0usize, 1, 3, 100] {
                    let top = set.search(&query, k).unwrap();
                    assert_eq!(top, all[..k.min(all.len())].to_vec());
                }
            }
            assert_eq!(
                set.similarity(UnitId([99; 16]), &VectorQuery::new(metric, as_f32(&vec![1; dim])).unwrap()).unwrap(),
                None
            );
        }
    }
    assert!(checked > 1000, "{checked}");

    // Known values, by hand.
    let mut set = VectorSet::new(2, VectorMetric::Cosine).unwrap();
    let (x, y, z) = (UnitId([1; 16]), UnitId([2; 16]), UnitId([3; 16]));
    set.insert(x, vec![3.0, 4.0]).unwrap();
    set.insert(y, vec![-4.0, 3.0]).unwrap();
    set.insert(z, vec![1.0, 0.0]).unwrap();
    let q = VectorQuery::new(VectorMetric::Cosine, vec![6.0, 8.0]).unwrap();
    let got: Vec<(UnitId, i64)> = set.search(&q, 3).unwrap().iter().map(|s| (s.unit, s.similarity)).collect();
    assert_eq!(got, vec![(x, 1_000_000), (z, 600_000), (y, 0)]);
    // The magnitude does not matter to the cosine, and it does to the dot product.
    let mut dot = VectorSet::new(2, VectorMetric::Dot).unwrap();
    dot.insert(x, vec![3.0, 4.0]).unwrap();
    let q = VectorQuery::new(VectorMetric::Dot, vec![6.0, 8.0]).unwrap();
    assert_eq!(dot.similarity(x, &q).unwrap(), Some(50_000_000));
    // Huge components saturate instead of wrapping.
    let mut dot = VectorSet::new(1, VectorMetric::Dot).unwrap();
    dot.insert(x, vec![f32::MAX]).unwrap();
    let q = VectorQuery::new(VectorMetric::Dot, vec![f32::MAX]).unwrap();
    assert_eq!(dot.similarity(x, &q).unwrap(), Some(i64::MAX));
    let mut cos = VectorSet::new(1, VectorMetric::Cosine).unwrap();
    cos.insert(x, vec![f32::MAX]).unwrap();
    let q = VectorQuery::new(VectorMetric::Cosine, vec![f32::MAX]).unwrap();
    assert_eq!(cos.similarity(x, &q).unwrap(), Some(1_000_000));
}

// ------------------------------------------------- I-37

const S_TYPE: &str = "SENTINEL-TYPE-5c1e";
const S_COMPONENT: f32 = 7654.322;

fn sentinels_no_literal_outside_the_diagnostic_output<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 3);
    relate(&db, &[(u[0], u[1], S_TYPE, RelationProvenance::Declared), (u[1], u[2], S_TYPE, imported())]);
    let snap = db.snapshot().unwrap();
    let mut q = AssocQuery::new(vec![u[0]], 2, 10, 100);
    q.direction = AssocDirection::Out;
    let result = snap.reference_assoc_query(&q).unwrap();
    let control = format!("{result:?}");
    assert!(control.contains(S_TYPE), "control: the result itself holds the type");

    let mut set = VectorSet::new(2, VectorMetric::Dot).unwrap();
    set.insert(u[1], vec![S_COMPONENT, 2.0]).unwrap();
    set.insert(u[2], vec![1.0, S_COMPONENT]).unwrap();
    let query = VectorQuery::new(VectorMetric::Dot, vec![S_COMPONENT, S_COMPONENT]).unwrap();
    let preferred = BTreeSet::from([S_TYPE.to_owned()]);
    let make = |mode| RankOptions {
        mode,
        preferred_types: preferred.clone(),
        vectors: Some(VectorSignal { set: &set, query: &query }),
        ..RankOptions::default()
    };

    // Everything the module prints other than the diagnostic output.
    let mut out = String::new();
    for mode in [RankMode::Off, RankMode::On] {
        let ranked = snap.rank_assoc(&result, &make(mode)).unwrap();
        out.push_str(&format!("{ranked:?}\n{:?}\n", make(mode)));
    }
    out.push_str(&format!("{set:?}\n{query:?}\n"));
    let mut scratch = set.clone();
    let bad = [
        VectorSet::new(0, VectorMetric::Dot).unwrap_err(),
        scratch.insert(u[0], vec![S_COMPONENT, f32::NAN]).unwrap_err(),
        scratch.insert(u[0], vec![S_COMPONENT; 3]).unwrap_err(),
        VectorQuery::new(VectorMetric::Dot, vec![S_COMPONENT, f32::INFINITY]).unwrap_err(),
        VectorQuery::new(VectorMetric::Cosine, vec![0.0]).unwrap_err(),
        set.check(&VectorQuery::new(VectorMetric::Cosine, vec![S_COMPONENT; 2]).unwrap()).unwrap_err(),
        set.check(&VectorQuery::new(VectorMetric::Dot, vec![S_COMPONENT; 5]).unwrap()).unwrap_err(),
    ];
    for e in bad {
        out.push_str(&format!("{e} | {e:?}\n"));
    }
    let wrong = VectorQuery::new(VectorMetric::Dot, vec![S_COMPONENT; 4]).unwrap();
    let e = snap.rank_assoc(&result, &opts(RankMode::On, Some((&set, &wrong)))).unwrap_err();
    out.push_str(&format!("{e} | {e:?}\n"));
    touch(&db, u[1], b"later");
    let e = db.snapshot().unwrap().rank_assoc(&result, &make(RankMode::On)).unwrap_err();
    out.push_str(&format!("{e} | {e:?}\n"));

    let text_forms = [S_TYPE.to_owned(), "7654.322".to_owned(), "7654.32".to_owned(), format!("{S_COMPONENT}")];
    let id_forms: Vec<String> = u.iter().flat_map(|id| [id.to_string(), format!("{:?}", id.0)]).collect();
    for needle in text_forms.iter() {
        assert!(!out.contains(needle.as_str()), "sentinel {needle:?} leaked:\n{out}");
    }
    // Errors name no unit either.
    assert!(out.contains("RankedHit"), "control: the output holds the hits");
    for e in out.lines().filter(|l| l.contains(" | ")) {
        for id in &id_forms {
            assert!(!e.contains(id.as_str()), "an error named a unit: {e}");
        }
    }

    // The diagnostic output holds integers: still no vector component or type.
    let diag = format!("{:?}", snap.rank_assoc(&result, &make(RankMode::Diagnostic)).unwrap());
    assert!(diag.contains("Contributions"), "control: the diagnostic output has the contributions");
    for needle in &text_forms {
        assert!(!diag.contains(needle.as_str()), "sentinel {needle:?} in the diagnostic output:\n{diag}");
    }

    // Ranking reports nothing to the observer.
    db.observer().set_level(ObservationLevel::Diagnostic).unwrap();
    let before = db.observer().records().len();
    snap.rank_assoc(&result, &make(RankMode::Diagnostic)).unwrap();
    assert_eq!(db.observer().records().len(), before, "ranking is not an observed execution");
}

macro_rules! engine_tests {
    ($m:ident, $E:ty) => {
        mod $m {
            use super::*;
            type E = $E;
            #[test]
            fn contributions_add_up_to_the_score_and_match_an_oracle() {
                super::contributions_add_up_to_the_score_and_match_an_oracle::<E>();
            }
            #[test]
            fn hand_vectors_without_a_vector_signal() {
                super::hand_vectors_without_a_vector_signal::<E>();
            }
            #[test]
            fn hand_vectors_with_cosine_and_dot() {
                super::hand_vectors_with_cosine_and_dot::<E>();
            }
            #[test]
            fn same_state_and_query_give_the_same_order() {
                super::same_state_and_query_give_the_same_order::<E>();
            }
            #[test]
            fn ties_are_broken_by_identifier() {
                super::ties_are_broken_by_identifier::<E>();
            }
            #[test]
            fn ranking_off_is_the_reference_order() {
                super::ranking_off_is_the_reference_order::<E>();
            }
            #[test]
            fn a_result_from_another_state_is_refused_unless_ranking_is_off() {
                super::a_result_from_another_state_is_refused_unless_ranking_is_off::<E>();
            }
            #[test]
            fn every_vector_rejection() {
                super::every_vector_rejection::<E>();
            }
            #[test]
            fn sentinels_no_literal_outside_the_diagnostic_output() {
                super::sentinels_no_literal_outside_the_diagnostic_output::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, EmuLog);
engine_tests!(kerno, KernoDb);
