//! The single measurement of hybrid retrieval over the second frozen
//! evaluation set (E8-10: `crates/kerno-testkit/data/derived-eval-2`), against
//! criterion Q4 of acta 8 and the thresholds of acta 9 §5. Every decision below
//! was fixed before the first run; none changes after it (the report is
//! `docs/testing/hybrid-e8-10-report.md`).
//!
//! The harness is copied from `tests/derived_measure2.rs` (the E9-20 record),
//! not refactored: that file's line numbers are cited by
//! `docs/architecture/stage9-measurement.md` §4. The pieces copied are the MANIFEST
//! check, the frozen-file loading, the query translation, the scoring and the
//! `put!` macro. `derived_eval2::{nearest, arm_without_signal}` and the
//! `harness::derived` loader are reused.
//!
//! # Parameters (the library defaults, not chosen from E8-10)
//!
//! RRF `k = 60`, weights 1/1/1, 100 candidates per leg, cut 10. HNSW:
//! `AnnConfig::new(16, Cosine)` (M 16, ef_construction 200, ef_search 64, seed 0).
//!
//! # The keyword leg and its limit
//!
//! The frozen queries carry no text. A keyword leg is evaluated only through a
//! declared derivation rule: the query text is the label of the seed unit (for
//! example `s0000.d0`). That is close to a same-session oracle, so every arm
//! that uses it is secondary and informational; the primary arm uses only what
//! the frozen queries carry (a vector and a graph query).
//!
//! # Arms, cut at 10, all 100 queries
//!
//! * `graph`: the graph leg alone. This is the "without the vector signal" arm of
//!   Q4 as written. A single-leg fusion keeps the reference order.
//! * `ann`: the HNSW leg alone.
//! * `exact`: the exact `VectorSet` leg alone: the arm "without signal" of
//!   T-E10-S1 and S2.
//! * `fused`: HNSW + graph. **Primary.**
//! * `ranked`: `rank_assoc` with `RankMode::On`, default weights, no vectors:
//!   the second reading of S1 and S2, as in E9-20.
//! * `brute`: an independent brute-force cosine order (the reference of Q1).
//! * `keyword`: text only. `fused_kg`: text + graph. `fused_all`: text + HNSW +
//!   graph. **Secondary.**
//!
//! # Verdicts on the primary arm
//!
//! Q4 as written: `fused - graph >= -0.01` in nDCG@10, overall and in each of
//! the 4 classes. T-E10-S1: `fused - exact >= 0.02`. T-E10-S2: `fused - exact >=
//! -0.01` per class. T-E10-Q2: fused nDCG >= 0.80. T-E10-Q3: fused recall >=
//! 0.90. T-E10-Q5 over {graph, ann, exact, fused, ranked, brute} x 5 scopes.
//! T-E10-V1, V2 (exact <= 0.96), Q1 (exact against brute force) and R.
//!
//! The headline reads "criterion Q4 on E8-10: demonstrated" if and only if Q4 as
//! written (all five), S1, S2, V1, V2, Q1, Q5 and R hold. Q2 and Q3 are printed
//! as the floors of acta 9 §5 and do not enter the headline. A threshold is
//! printed with its figure and never asserted: what fails the test is what the
//! measurement depends on (the MANIFEST, the reopened database serving every
//! vector, the graph arm agreeing with the reference path, the exact arm
//! agreeing with the testkit's independent computation, the two engines
//! printing identical reports).
//!
//! ```text
//! cargo test -p kerno-units --all-features --release --test hybrid_measure -- --nocapture --exact hybrid_quality_over_the_second_frozen_set
//! ```

mod harness;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use harness::derived::{PermutedIds, T0, f32_vector, load, recipe, unit_id};
use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, Judgment};
use kerno_testkit::assoc_metrics::{ndcg_at, recall_at};
use kerno_testkit::derived_eval::{self, CUT, Class, DIM, Query};
use kerno_testkit::derived_eval2::{SATURATION_LIMIT, arm_without_signal, nearest};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::sha256;
use kerno_testkit::vfs::SimVfs;
use kerno_units::assoc_rank::{RankMode, RankOptions, VectorMetric};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;
type Res<T> = Result<T, Box<dyn std::error::Error>>;

const CORPUS: &str = include_str!("../../kerno-testkit/data/derived-eval-2/corpus.txt");
const VECTORS: &str = include_str!("../../kerno-testkit/data/derived-eval-2/vectors.txt");
const QUERIES: &str = include_str!("../../kerno-testkit/data/derived-eval-2/queries.txt");
const MANIFEST: &str = include_str!("../../kerno-testkit/data/derived-eval-2/MANIFEST");

const CLASSES: [Class; 4] = [Class::VectorNeighbourhood, Class::Temporal, Class::Provenance, Class::Combined];

/// Thresholds, as written in acta 8 (Q4) and acta 9 §5.
const V1_MIN_QUERIES_PER_CLASS: usize = 25;
const Q1_RECALL: f64 = 1.0;
const Q2_NDCG: f64 = 0.80;
const Q3_RECALL: f64 = 0.90;
const Q4_MIN_GAIN: f64 = -0.01;
const S1_MIN_GAIN: f64 = 0.02;
const S2_MIN_GAIN: f64 = -0.01;
/// T-E10-Q5: at most one position in ten may be unjudged.
const Q5_MAX_UNJUDGED_TENTHS: usize = 1;

const UNITS: usize = 2_000;
const QUERY_COUNT: usize = 100;

// ------------------------------------------------------------------ MANIFEST

/// Checks the frozen files against `MANIFEST`: the three files, in order, with
/// their SHA-256. Nothing is measured if this fails (T-E10-R).
fn check_manifest(manifest: &str, files: [(&str, &str); 3]) -> Result<(), String> {
    let entries = assoc_eval::parse_manifest(manifest).map_err(|e| format!("the MANIFEST does not parse: {e:?}"))?;
    if entries.len() != files.len() {
        return Err(format!("the MANIFEST lists {} files, expected {}", entries.len(), files.len()));
    }
    for ((digest, name), (want_name, text)) in entries.iter().zip(files) {
        if name != want_name {
            return Err(format!("the MANIFEST names {name} where {want_name} was expected"));
        }
        let got = sha256::hex(&sha256::digest(text.as_bytes()));
        if &got != digest {
            return Err(format!("MANIFEST: {name} does not match the frozen file"));
        }
    }
    Ok(())
}

fn frozen_files() -> [(&'static str, &'static str); 3] {
    [("corpus.txt", CORPUS), ("vectors.txt", VECTORS), ("queries.txt", QUERIES)]
}

// ---------------------------------------------------------------- thresholds

fn q4_met(gain: f64) -> bool {
    gain >= Q4_MIN_GAIN
}

fn s1_met(gain: f64) -> bool {
    gain >= S1_MIN_GAIN
}

fn s2_met(gain: f64) -> bool {
    gain >= S2_MIN_GAIN
}

/// Unjudged positions are at most one in ten (T-E10-Q5).
fn q5_met(positions: usize, unjudged: usize) -> bool {
    unjudged * 10 <= positions * Q5_MAX_UNJUDGED_TENTHS
}

/// The figures of the primary arm the thresholds are checked against.
struct Figures {
    /// Queries with a query vector, per class, in `CLASSES` order.
    vector_queries: [usize; 4],
    /// nDCG@10 of the exact arm, overall.
    exact_ndcg: f64,
    /// Smallest recall of the exact arm against brute force.
    q1_recall: f64,
    fused_ndcg: f64,
    fused_recall: f64,
    /// Fused minus the graph arm (Q4 as written): overall and per class.
    q4_gain: f64,
    q4_class_gains: [f64; 4],
    /// Fused minus the exact arm (S1, S2): overall and per class.
    s1_gain: f64,
    s2_class_gains: [f64; 4],
    /// `(positions, unjudged)` of every (arm, scope) pair of the Q5 table.
    q5_pairs: Vec<(usize, usize)>,
}

struct Verdict {
    code: String,
    met: bool,
    /// Whether the headline depends on it (Q2 and Q3 are floors of acta 9 §5,
    /// printed and not part of the headline).
    headline: bool,
}

/// Every threshold, in the order the report prints them.
fn verdicts(f: &Figures) -> Vec<Verdict> {
    let mut out = Vec::new();
    let mut add = |code: String, met: bool, headline: bool| out.push(Verdict { code, met, headline });
    add("Q4 all-100".into(), q4_met(f.q4_gain), true);
    for (class, gain) in CLASSES.iter().zip(f.q4_class_gains) {
        add(format!("Q4 {}", class.name()), q4_met(gain), true);
    }
    add("T-E10-S1".into(), s1_met(f.s1_gain), true);
    add("T-E10-S2".into(), f.s2_class_gains.iter().all(|g| s2_met(*g)), true);
    add("T-E10-Q2".into(), f.fused_ndcg >= Q2_NDCG, false);
    add("T-E10-Q3".into(), f.fused_recall >= Q3_RECALL, false);
    add("T-E10-Q5".into(), f.q5_pairs.iter().all(|(p, u)| q5_met(*p, *u)), true);
    add("T-E10-V1".into(), f.vector_queries.iter().all(|n| *n >= V1_MIN_QUERIES_PER_CLASS), true);
    add("T-E10-V2".into(), f.exact_ndcg <= SATURATION_LIMIT, true);
    add("T-E10-Q1".into(), f.q1_recall >= Q1_RECALL, true);
    out
}

/// "demonstrated" if and only if every verdict the headline depends on is met
/// (T-E10-R, the MANIFEST, is checked before anything runs).
fn headline(v: &[Verdict]) -> bool {
    v.iter().filter(|v| v.headline).all(|v| v.met)
}

// ------------------------------------------------------------------- metrics

/// The metrics of one arm on one query.
struct Scored {
    recall: f64,
    ndcg: f64,
    /// Positions of the cut the arm returned, and how many are unjudged.
    positions: usize,
    unjudged: usize,
}

/// The top `CUT` of `list` with the unjudged units left out, scored against
/// `expected`.
fn score(list: &[u32], expected: &[Judgment]) -> Scored {
    let cut = &list[..list.len().min(CUT)];
    let judged: BTreeSet<u32> = expected.iter().map(|j| j.index).collect();
    let kept: Vec<u32> = cut.iter().copied().filter(|u| judged.contains(u)).collect();
    Scored {
        recall: recall_at(&kept, expected, CUT),
        ndcg: ndcg_at(&kept, expected, CUT),
        positions: cut.len(),
        unjudged: cut.len() - kept.len(),
    }
}

fn judgments(q: &Query) -> Vec<Judgment> {
    q.expected.iter().map(|j| Judgment { index: j.unit, grade: j.grade }).collect()
}

fn mean(v: impl Iterator<Item = f64>) -> f64 {
    let (n, sum) = v.fold((0usize, 0.0), |(n, s), x| (n + 1, s + x));
    if n == 0 { 0.0 } else { sum / n as f64 }
}

fn verdict(met: bool) -> &'static str {
    if met { "met" } else { "not met" }
}

// ------------------------------------------------------------------- queries

fn kind_of(name: &str) -> ProvenanceKind {
    match name {
        "declared" => ProvenanceKind::Declared,
        "imported" => ProvenanceKind::Imported,
        "algorithmic" => ProvenanceKind::Algorithmic,
        "inferred" => ProvenanceKind::Inferred,
        other => panic!("unknown provenance class {other}"),
    }
}

/// A query of the frozen set as an `AssocQuery`: seed, direction, kinds,
/// depth, window and provenance classes come from the file; the limit and the
/// visit budget are the hard maxima of the crate, for every query.
fn assoc_query(units: &[UnitId], q: &Query) -> AssocQuery {
    let mut query =
        AssocQuery::new(vec![units[q.seed as usize]], u32::from(q.depth), MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS);
    query.direction = match q.direction {
        derived_eval::Direction::Out => AssocDirection::Out,
        derived_eval::Direction::In => AssocDirection::In,
        derived_eval::Direction::Both => AssocDirection::Both,
    };
    query.relation_types = Some(q.kinds.iter().map(|k| (*k).to_owned()).collect());
    query.window = q.window.map(|(lo, hi)| SeqWindow { lo: CommitSeq(lo), hi: CommitSeq(hi) });
    let kinds: BTreeSet<ProvenanceKind> = q.provenance.iter().map(|p| kind_of(p)).collect();
    query.provenance = ProvenanceFilter { kinds: (!kinds.is_empty()).then_some(kinds), actor: None, source: None };
    query
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    Graph,
    Ann,
    Exact,
    Fused,
    Ranked,
    Brute,
    Keyword,
    FusedKg,
    FusedAll,
}

/// The arms of the primary table, in the order of the Q5 check.
const PRIMARY_ARMS: [Arm; 6] = [Arm::Graph, Arm::Ann, Arm::Exact, Arm::Fused, Arm::Ranked, Arm::Brute];
const SECONDARY_ARMS: [Arm; 3] = [Arm::Keyword, Arm::FusedKg, Arm::FusedAll];

impl Arm {
    fn name(self) -> &'static str {
        match self {
            Arm::Graph => "graph",
            Arm::Ann => "ann",
            Arm::Exact => "exact",
            Arm::Fused => "fused",
            Arm::Ranked => "ranked",
            Arm::Brute => "brute",
            Arm::Keyword => "keyword",
            Arm::FusedKg => "fused_kg",
            Arm::FusedAll => "fused_all",
        }
    }
}

/// What each arm returned for one query, as corpus identifiers, indexed by
/// `Arm as usize`.
struct Run {
    id: String,
    class: Class,
    expected: Vec<Judgment>,
    lists: Vec<Vec<u32>>,
}

impl Run {
    fn list(&self, arm: Arm) -> &[u32] {
        &self.lists[arm as usize]
    }
}

/// `None` is all the queries.
fn in_scope(run: &Run, class: Option<Class>) -> bool {
    class.is_none_or(|c| run.class == c)
}

fn scope_name(class: Option<Class>) -> &'static str {
    class.map_or("all-100", |c| c.name())
}

fn scopes() -> impl Iterator<Item = Option<Class>> {
    std::iter::once(None).chain(CLASSES.map(Some))
}

/// `(queries, recall@10, nDCG@10)` means of an arm over a scope.
fn mean_of(runs: &[Run], arm: Arm, class: Option<Class>) -> (usize, f64, f64) {
    let scores: Vec<Scored> =
        runs.iter().filter(|r| in_scope(r, class)).map(|r| score(r.list(arm), &r.expected)).collect();
    (scores.len(), mean(scores.iter().map(|s| s.recall)), mean(scores.iter().map(|s| s.ndcg)))
}

/// `(positions, unjudged)` of an arm over a scope.
fn positions_of(runs: &[Run], arm: Arm, class: Option<Class>) -> (usize, usize) {
    runs.iter()
        .filter(|r| in_scope(r, class))
        .map(|r| score(r.list(arm), &r.expected))
        .fold((0, 0), |(p, u), s| (p + s.positions, u + s.unjudged))
}

/// nDCG@10 of `with` minus that of `without`: overall, then per class.
fn gains(runs: &[Run], with: Arm, without: Arm) -> (f64, [f64; 4]) {
    let diff = |class| mean_of(runs, with, class).2 - mean_of(runs, without, class).2;
    (diff(None), CLASSES.map(|c| diff(Some(c))))
}

// ------------------------------------------------------------------- setup

fn config(corpus: &derived_eval::Corpus) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(PermutedIds::new(corpus)));
    c.engine = Options::small_for_tests();
    c
}

struct Set {
    corpus: derived_eval::Corpus,
    vectors: Vec<[i32; DIM]>,
    queries: derived_eval::QuerySet,
}

/// Parses the frozen files, after the MANIFEST. Panics (refusing to measure)
/// if the MANIFEST does not match.
fn frozen_set() -> Set {
    if let Err(why) = check_manifest(MANIFEST, frozen_files()) {
        panic!("refusing to measure: {why}");
    }
    let corpus = derived_eval::parse_corpus(CORPUS).expect("corpus parses");
    let vectors = derived_eval::parse_vectors(VECTORS).expect("vectors parse");
    let queries = derived_eval::parse_queries(QUERIES).expect("queries parse");
    assert_eq!(corpus.units.len(), UNITS, "the frozen corpus has 2000 units");
    assert_eq!(vectors.len(), UNITS, "one vector per unit");
    assert_eq!(queries.queries.len(), QUERY_COUNT, "the frozen set has 100 queries");
    assert!(queries.queries.iter().all(|q| q.vector.is_some()), "every query of this set carries a vector");
    Set { corpus, vectors, queries }
}

/// Everything the report of one engine says, one line per entry.
type Lines = Vec<String>;

macro_rules! put {
    ($out:expr, $($arg:tt)*) => { $out.push(format!($($arg)*)) };
}

// ------------------------------------------------------------------- quality

fn quality<E: Target>(set: &Set) -> Res<Lines> {
    let mut out: Lines = Vec::new();
    let disk = SimVfs::new(91);
    let cfg = config(&set.corpus);

    // Load with every vector persisted, close, and measure what a reopened
    // database serves.
    let (dd, _) = DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg)?)?;
    let loaded = load(&dd, &set.corpus, &set.vectors, true)?;
    dd.close()?;

    let (dd, reopen) = DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg)?)?;
    assert_eq!(reopen.served, UNITS, "every persisted vector is served after reopening");
    assert!(reopen.demoted.is_empty() && reopen.findings.is_empty(), "the reopened database has no stale datum");
    let mut ann =
        AnnIndex::<E>::open(disk.as_vfs(), Path::new("/ann"), &cfg.engine, AnnConfig::new(DIM, AnnMetric::Cosine))?;
    let synced = dd.sync_ann(&mut ann, &recipe())?;
    assert!(synced.added == UNITS && synced.skipped == 0, "the index holds every vector: {synced:?}");
    let vector_set = dd.vector_set(DIM, VectorMetric::Cosine)?;
    assert_eq!(vector_set.len(), UNITS, "the exact set holds every vector");
    dd.close()?;
    put!(
        out,
        "load: {} units, {} relations, {} change sets; {} vectors persisted, reopened and served; {} indexed in HNSW",
        UNITS,
        set.corpus.relations.len(),
        set.corpus.changes.len(),
        reopen.served,
        ann.len()
    );

    let db = open_units::<E>(disk.as_vfs(), &cfg)?;
    let snap = db.snapshot()?;
    let corpus_id: HashMap<UnitId, u32> = loaded.units.iter().enumerate().map(|(i, u)| (*u, i as u32)).collect();
    assert_eq!(loaded.units[7], unit_id(7), "a unit's id is its corpus identifier");
    let label = |id: u32| set.corpus.units.iter().find(|u| u.id == id).map(|u| u.label.as_str());

    let mut runs: Vec<Run> = Vec::new();
    let mut seed_in_ann = 0usize;
    for q in &set.queries.queries {
        let query = assoc_query(&loaded.units, q);
        let simple = snap.reference_assoc_query(&query)?;
        let qv = q.vector.as_ref().ok_or("a query of this set has no vector")?;
        let qf = f32_vector(qv);
        let text = label(q.seed).ok_or("a seed has no label")?;

        let hybrid = |text: Option<&str>, ann_leg: bool, exact_leg: bool, graph: bool| -> Res<Vec<u32>> {
            let mut h = HybridQuery::new(CUT);
            h.text = text;
            if ann_leg {
                h.vector = Some(VectorLeg { index: &ann, query: &qf });
            }
            if exact_leg {
                h.vector = Some(VectorLeg { index: &vector_set, query: &qf });
            }
            h.graph = graph.then(|| query.clone());
            Ok(snap.hybrid_search(&h)?.iter().map(|hit| corpus_id[&hit.unit]).collect())
        };
        let as_ids = |units: &mut dyn Iterator<Item = UnitId>| -> Vec<u32> { units.map(|u| corpus_id[&u]).collect() };

        let ranked_hits = snap.rank_assoc(&simple, &RankOptions { mode: RankMode::On, ..RankOptions::default() })?;
        let mut lists = vec![Vec::new(); 9];
        lists[Arm::Graph as usize] = hybrid(None, false, false, true)?;
        lists[Arm::Ann as usize] = hybrid(None, true, false, false)?;
        lists[Arm::Exact as usize] = hybrid(None, false, true, false)?;
        lists[Arm::Fused as usize] = hybrid(None, true, false, true)?;
        lists[Arm::Ranked as usize] = as_ids(&mut ranked_hits.iter().take(CUT).map(|h| h.unit));
        lists[Arm::Brute as usize] = nearest(&set.vectors, qv).into_iter().take(CUT).collect();
        lists[Arm::Keyword as usize] = hybrid(Some(text), false, false, false)?;
        lists[Arm::FusedKg as usize] = hybrid(Some(text), false, false, true)?;
        lists[Arm::FusedAll as usize] = hybrid(Some(text), true, false, true)?;

        // The structural anchor: a single-leg fusion keeps the reference order.
        let reference = as_ids(&mut simple.hits.iter().take(CUT).map(|h| h.unit));
        assert_eq!(lists[Arm::Graph as usize], reference, "{}: the graph arm differs from the reference order", q.id);
        seed_in_ann += usize::from(lists[Arm::Ann as usize].contains(&q.seed));
        runs.push(Run { id: q.id.clone(), class: q.class, expected: judgments(q), lists });
    }
    put!(out, "queries: {} run through the hybrid query, the reference path and brute force", runs.len());

    // The independent anchor: the testkit's own computation of the arm without signal.
    let anchor = arm_without_signal(&set.vectors, &set.queries).0;
    let exact = mean_of(&runs, Arm::Exact, None);
    assert!((exact.2 - anchor).abs() < 1e-12, "the exact arm differs from the testkit's: {} vs {anchor}", exact.2);

    // ---- T-E10-V1: queries with a query vector, per class.
    put!(out, "T-E10-V1  queries that carry a query vector, by class (at least {V1_MIN_QUERIES_PER_CLASS} each)");
    let mut vector_queries = [0usize; 4];
    for (slot, class) in vector_queries.iter_mut().zip(CLASSES) {
        *slot = runs.iter().filter(|r| r.class == class).count();
        put!(out, "  {:<22} {:>3}", class.name(), *slot);
    }

    // ---- T-E10-Q1: the exact arm against the independent brute force.
    put!(out, "T-E10-Q1  exact arm against brute force (cosine), cut {CUT}");
    let overlap = |a: &[u32], b: &[u32]| a.iter().filter(|u| b.contains(u)).count() as f64 / CUT as f64;
    let q1: Vec<f64> = runs.iter().map(|r| overlap(r.list(Arm::Exact), r.list(Arm::Brute))).collect();
    let q1_recall = q1.iter().copied().fold(f64::INFINITY, f64::min);
    let same = runs.iter().filter(|r| r.list(Arm::Exact) == r.list(Arm::Brute)).count();
    put!(
        out,
        "  recall@{CUT} against the reference: smallest {q1_recall:.3}, mean {:.3} over {} queries; identical lists in the same order: {same}",
        mean(q1.iter().copied()),
        q1.len()
    );

    // ---- Informational: how close HNSW is to exact search.
    let hnsw: Vec<f64> = runs.iter().map(|r| overlap(r.list(Arm::Ann), r.list(Arm::Brute))).collect();
    let hnsw_same = runs.iter().filter(|r| r.list(Arm::Ann) == r.list(Arm::Brute)).count();
    put!(
        out,
        "HNSW recall@{CUT} against brute force: smallest {:.3}, mean {:.3}; identical lists: {hnsw_same} of {}",
        hnsw.iter().copied().fold(f64::INFINITY, f64::min),
        mean(hnsw.iter().copied()),
        hnsw.len()
    );
    put!(out, "seed unit present in the ann top {CUT}: {seed_in_ann} of {}", runs.len());

    // ---- T-E10-Q5: the validity of the judgments, per arm and per class.
    put!(
        out,
        "T-E10-Q5  unjudged units in the top {CUT}, per arm and per class (excluded from the metrics and counted here)"
    );
    let mut q5_pairs = Vec::new();
    for arm in PRIMARY_ARMS.into_iter().chain(SECONDARY_ARMS) {
        for class in scopes() {
            let (positions, unjudged) = positions_of(&runs, arm, class);
            if PRIMARY_ARMS.contains(&arm) {
                q5_pairs.push((positions, unjudged));
            }
            put!(
                out,
                "  {:<9} {:<22} {positions:>4} positions, {unjudged:>4} unjudged ({:.1} %) -> {}",
                arm.name(),
                scope_name(class),
                100.0 * unjudged as f64 / positions.max(1) as f64,
                verdict(q5_met(positions, unjudged))
            );
        }
    }

    // ---- Means.
    put!(out, "means (cut {CUT}; unjudged units excluded): arm, scope, queries, recall@10, nDCG@10");
    for arm in PRIMARY_ARMS.into_iter().chain(SECONDARY_ARMS) {
        for class in scopes() {
            let m = mean_of(&runs, arm, class);
            put!(out, "  {:<9} {:<22} {:>3} {:>8.4} {:>8.4}", arm.name(), scope_name(class), m.0, m.1, m.2);
        }
    }

    // ---- Primary verdicts.
    let fused = mean_of(&runs, Arm::Fused, None);
    let (q4_gain, q4_class_gains) = gains(&runs, Arm::Fused, Arm::Graph);
    let (s1_gain, s2_class_gains) = gains(&runs, Arm::Fused, Arm::Exact);
    let figures = Figures {
        vector_queries,
        exact_ndcg: exact.2,
        q1_recall,
        fused_ndcg: fused.2,
        fused_recall: fused.1,
        q4_gain,
        q4_class_gains,
        s1_gain,
        s2_class_gains,
        q5_pairs,
    };
    put!(out, "primary arm: fused = HNSW + graph (weights 1/1/1, k 60, 100 candidates per leg, cut {CUT})");
    put!(
        out,
        "  Q4 as written  fused - graph (without the vector signal) >= {Q4_MIN_GAIN:+.2}, overall and per class:"
    );
    put!(out, "    all-100                {q4_gain:+.4} -> {}", verdict(q4_met(q4_gain)));
    for (class, gain) in CLASSES.iter().zip(q4_class_gains) {
        put!(out, "    {:<22} {gain:+.4} -> {}", class.name(), verdict(q4_met(gain)));
    }
    put!(out, "  T-E10-S1  fused - exact, overall >= {S1_MIN_GAIN:.2}: {s1_gain:+.4} -> {}", verdict(s1_met(s1_gain)));
    for (class, gain) in CLASSES.iter().zip(s2_class_gains) {
        put!(
            out,
            "  T-E10-S2  {:<22} fused - exact >= {S2_MIN_GAIN:+.2}: {gain:+.4} -> {}",
            class.name(),
            verdict(s2_met(gain))
        );
    }
    put!(out, "  T-E10-Q2  fused nDCG@10 >= {Q2_NDCG:.2}: {:.4}", figures.fused_ndcg);
    put!(out, "  T-E10-Q3  fused recall@10 >= {Q3_RECALL:.2}: {:.4}", figures.fused_recall);
    put!(out, "  T-E10-V2  exact nDCG@10 <= {SATURATION_LIMIT:.2}: {:.4}", figures.exact_ndcg);
    let (gain_ranked, class_gains_ranked) = gains(&runs, Arm::Fused, Arm::Ranked);
    put!(out, "second reading of S1 and S2 (without signal = the ranked arm, no vectors), informational:");
    put!(out, "  T-E10-S1  overall >= {S1_MIN_GAIN:.2}: {gain_ranked:+.4} -> {}", verdict(s1_met(gain_ranked)));
    for (class, gain) in CLASSES.iter().zip(class_gains_ranked) {
        put!(out, "  T-E10-S2  {:<22} >= {S2_MIN_GAIN:+.2}: {gain:+.4} -> {}", class.name(), verdict(s2_met(gain)));
    }
    let primary = verdicts(&figures);
    put!(out, "verdicts, primary arm:");
    for v in &primary {
        put!(
            out,
            "  {}: {}{}",
            v.code,
            verdict(v.met),
            if v.headline { "" } else { " (floor of acta 9 §5, not in the headline)" }
        );
    }

    // ---- Secondary verdicts: the keyword leg through the declared derivation rule.
    put!(out, "secondary arms (keyword text = the seed unit's label, a declared derivation; informational):");
    let all = mean_of(&runs, Arm::FusedAll, None);
    let (kq4, kq4_classes) = gains(&runs, Arm::FusedAll, Arm::FusedKg);
    let (ks1, ks2) = gains(&runs, Arm::FusedAll, Arm::Exact);
    put!(
        out,
        "  Q4 as written  fused_all - fused_kg >= {Q4_MIN_GAIN:+.2}: all-100 {kq4:+.4} -> {}",
        verdict(q4_met(kq4))
    );
    for (class, gain) in CLASSES.iter().zip(kq4_classes) {
        put!(out, "    {:<22} {gain:+.4} -> {}", class.name(), verdict(q4_met(gain)));
    }
    put!(out, "  T-E10-S1  fused_all - exact >= {S1_MIN_GAIN:.2}: {ks1:+.4} -> {}", verdict(s1_met(ks1)));
    for (class, gain) in CLASSES.iter().zip(ks2) {
        put!(out, "  T-E10-S2  {:<22} >= {S2_MIN_GAIN:+.2}: {gain:+.4} -> {}", class.name(), verdict(s2_met(gain)));
    }
    put!(out, "  T-E10-Q2  fused_all nDCG@10 >= {Q2_NDCG:.2}: {:.4} -> {}", all.2, verdict(all.2 >= Q2_NDCG));
    put!(out, "  T-E10-Q3  fused_all recall@10 >= {Q3_RECALL:.2}: {:.4} -> {}", all.1, verdict(all.1 >= Q3_RECALL));
    let q5_secondary = SECONDARY_ARMS.iter().flat_map(|a| scopes().map(move |c| (*a, c))).all(|(a, c)| {
        let (p, u) = positions_of(&runs, a, c);
        q5_met(p, u)
    });
    put!(out, "  T-E10-Q5  over keyword, fused_kg, fused_all x 5 scopes: {}", verdict(q5_secondary));

    put!(out, "criterion Q4 on E8-10: {}", if headline(&primary) { "demonstrated" } else { "not demonstrated" });
    if let Some(r) = runs.first() {
        put!(out, "first query {} ({}): fused list {:?}", r.id, r.class.name(), r.list(Arm::Fused));
    }

    drop(snap);
    db.close()?;
    ann.close()?;
    Ok(out)
}

// -------------------------------------------------------------------- tests

/// Runs both engines, prints their reports, and checks that they agree.
#[test]
fn hybrid_quality_over_the_second_frozen_set() {
    let set = frozen_set();
    println!("T-E10-R  MANIFEST: corpus.txt, vectors.txt and queries.txt match");
    let mut reports: Vec<(&str, Lines)> = Vec::new();
    for (name, lines) in [("emulated_log", quality::<EmuLog>(&set)), ("kerno_db", quality::<KernoDb>(&set))] {
        let lines = lines.expect("quality measurement");
        println!("== engine {name}");
        for l in &lines {
            println!("{l}");
        }
        reports.push((name, lines));
    }
    assert_eq!(reports[0].1, reports[1].1, "the two engines report different figures");
    println!("engines: emulated_log and kerno_db report identical figures ({} lines)", reports[0].1.len());
}

#[test]
fn the_manifest_of_the_second_frozen_set_matches() {
    assert_eq!(check_manifest(MANIFEST, frozen_files()), Ok(()));
}

/// The refusal itself: a changed byte in any of the three files, a missing
/// entry or a swapped order is refused, each for its own reason.
#[test]
fn a_manifest_that_does_not_match_refuses_to_measure() {
    for which in 0..3 {
        let mut files = frozen_files();
        let mut text = files[which].1.to_owned();
        text.push('\n');
        let altered = Box::leak(text.into_boxed_str());
        files[which].1 = altered;
        let err = check_manifest(MANIFEST, files).unwrap_err();
        assert!(err.contains(files[which].0) && err.contains("does not match"), "{err}");
    }
    let two_lines: String = MANIFEST.lines().take(2).map(|l| format!("{l}\n")).collect();
    assert!(check_manifest(&two_lines, frozen_files()).unwrap_err().contains("lists 2 files"));
    let mut swapped = frozen_files();
    swapped.swap(0, 1);
    assert!(check_manifest(MANIFEST, swapped).unwrap_err().contains("where"));
}

/// Unjudged units leave the list; they are counted; the ideal stays whole.
#[test]
fn unjudged_units_are_excluded_and_counted() {
    let expected = [Judgment { index: 1, grade: 3 }, Judgment { index: 2, grade: 0 }, Judgment { index: 3, grade: 1 }];
    // 90 and 91 are unjudged: they must neither count as grade 0 nor shift the others' ranks.
    let s = score(&[90, 1, 91, 3], &expected);
    assert_eq!((s.positions, s.unjudged), (4, 2));
    let condensed = ndcg_at(&[1, 3], &expected, CUT);
    assert_eq!(s.ndcg, condensed);
    assert_eq!(s.recall, 1.0);
    // Only the cut counts: a unit after position 10 is not read.
    let long: Vec<u32> = (100..120).collect();
    assert_eq!(score(&long, &expected).positions, CUT);
}

fn passing() -> Figures {
    Figures {
        vector_queries: [25; 4],
        exact_ndcg: 0.42,
        q1_recall: 1.0,
        fused_ndcg: 0.90,
        fused_recall: 0.95,
        q4_gain: 0.0,
        q4_class_gains: [0.0; 4],
        s1_gain: 0.03,
        s2_class_gains: [0.0; 4],
        q5_pairs: vec![(100, 10)],
    }
}

fn failing(f: &Figures) -> Vec<String> {
    verdicts(f).into_iter().filter(|v| !v.met).map(|v| v.code).collect()
}

/// Each verdict flips at its own bound, as the actas write it, and only it;
/// Q4 failing flips the headline and Q2 failing does not.
#[test]
fn every_verdict_flips_at_its_own_bound() {
    let base = passing();
    assert!(
        verdicts(&base).iter().all(|v| v.met) && headline(&verdicts(&base)),
        "the reference figures meet everything"
    );
    let flip = |code: &str, edit: &dyn Fn(&mut Figures)| {
        let mut f = passing();
        edit(&mut f);
        assert_eq!(failing(&f), [code], "only {code} may fail");
        f
    };
    // Q4 at -0.0101 fails, at -0.01 it is inside; overall and in each class.
    let f = flip("Q4 all-100", &|f| f.q4_gain = -0.0101);
    assert!(!headline(&verdicts(&f)), "Q4 failing flips the headline");
    flip("Q4 temporal", &|f| f.q4_class_gains[1] = -0.0101);
    flip("T-E10-S1", &|f| f.s1_gain = 0.0199);
    flip("T-E10-S2", &|f| f.s2_class_gains[3] = -0.0101);
    let f = flip("T-E10-Q2", &|f| f.fused_ndcg = 0.7999);
    assert!(headline(&verdicts(&f)), "Q2 failing does not flip the headline");
    let f = flip("T-E10-Q3", &|f| f.fused_recall = 0.8999);
    assert!(headline(&verdicts(&f)), "Q3 failing does not flip the headline");
    flip("T-E10-Q5", &|f| f.q5_pairs.push((100, 11)));
    flip("T-E10-V1", &|f| f.vector_queries[2] = 24);
    flip("T-E10-V2", &|f| f.exact_ndcg = 0.9601);
    flip("T-E10-Q1", &|f| f.q1_recall = 0.9);
    // The bounds themselves are inside.
    let mut edge = passing();
    edge.q4_gain = Q4_MIN_GAIN;
    edge.q4_class_gains = [Q4_MIN_GAIN; 4];
    edge.s1_gain = S1_MIN_GAIN;
    edge.s2_class_gains = [S2_MIN_GAIN; 4];
    edge.fused_ndcg = Q2_NDCG;
    edge.fused_recall = Q3_RECALL;
    edge.exact_ndcg = SATURATION_LIMIT;
    edge.vector_queries = [V1_MIN_QUERIES_PER_CLASS; 4];
    assert!(failing(&edge).is_empty(), "{:?}", failing(&edge));
}
