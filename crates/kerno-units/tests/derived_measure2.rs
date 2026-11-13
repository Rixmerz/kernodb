//! E9-20: the single measurement of E8-10 over the second frozen evaluation
//! set (`crates/kerno-testkit/data/derived-eval-2`, E9-19). The thresholds are
//! those of acta 9 §5 (T-E10-V1, V2, R, Q1, Q2, Q3, S1, S2, Q5), fixed before
//! the set was measured (decision D22). The report of the committed run is
//! `docs/architecture/etapa9-medicion-salida.txt`; the write-up is
//! `docs/architecture/etapa9-medicion.md`. The ratios of time are in
//! `examples/derived_bench2.rs`.
//!
//! A threshold is printed with its figure and whether it is met. It does not
//! fail the test: a threshold that is not met is recorded, never adjusted.
//! What fails the test is what the measurement depends on: the `MANIFEST`, the
//! reopened database serving every persisted vector, the indexed path agreeing
//! with the reference path, and the two engines reporting identical figures.
//!
//! Arms, cut at 10, each over all the 100 queries (every query of this set
//! carries a query vector, so every arm exists for every query):
//!
//! * `simple`: the associative query in the reference order.
//! * `ranked`: the same result under `RankMode::On`, default weights, with the
//!   vector signal off.
//! * `vector`: the exact cosine search over the persisted vectors read back
//!   from the reopened database. This is the arm "without signal" of T-E10-V2
//!   (the one the freezing validated), and so the primary reading of S1 and S2.
//! * `hybrid`: `ranked` with the vector signal on, over the persisted vectors.
//!   This is the arm "with signal".
//! * `brute`: an independent brute-force cosine order over the integer vectors
//!   of the set, the reference of T-E10-Q1.
//!
//! The acta writes "without signal" in two places with two meanings (the
//! vector-only arm in T-E10-V2 and the freezing document; the arm with the
//! vector signal off in E8's Q4, which S1 and S2 replace). Both readings are
//! computed from the same run and reported with their own label; the primary
//! one is fixed here, before the run, as the vector-only arm.
//!
//! Unjudged units in the top 10 of an arm are excluded from the list before
//! the metrics are computed and counted per arm and per class. The ideal of
//! nDCG and the relevant total of recall stay those of the whole judgment list.
//!
//! ```text
//! cargo test -p kerno-units --release --test derived_measure2 -- --nocapture --exact quality_over_the_second_frozen_set
//! ```

mod harness;

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use harness::derived::{PermutedIds, T0, f32_vector, load, unit_id};
use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, Judgment};
use kerno_testkit::assoc_metrics::{ndcg_at, recall_at};
use kerno_testkit::derived_eval::{self, CUT, Class, DIM, Query};
use kerno_testkit::derived_eval2::SATURATION_LIMIT;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::sha256;
use kerno_testkit::vfs::SimVfs;
use kerno_units::assoc_rank::{RankMode, RankOptions, VectorMetric, VectorQuery, VectorSet, VectorSignal};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;
type Res<T> = Result<T, Box<dyn std::error::Error>>;

const CORPUS: &str = include_str!("../../kerno-testkit/data/derived-eval-2/corpus.txt");
const VECTORS: &str = include_str!("../../kerno-testkit/data/derived-eval-2/vectors.txt");
const QUERIES: &str = include_str!("../../kerno-testkit/data/derived-eval-2/queries.txt");
const MANIFEST: &str = include_str!("../../kerno-testkit/data/derived-eval-2/MANIFEST");

const CLASSES: [Class; 4] = [Class::VectorNeighbourhood, Class::Temporal, Class::Provenance, Class::Combined];

/// Thresholds of acta 9 §5, as written there.
const V1_MIN_QUERIES_PER_CLASS: usize = 25;
const Q1_RECALL: f64 = 1.0;
const Q2_NDCG: f64 = 0.80;
const Q3_RECALL: f64 = 0.90;
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

/// The figures the thresholds of §5 are checked against.
struct Figures {
    /// Queries with a query vector, per class, in `CLASSES` order.
    vector_queries: [usize; 4],
    /// nDCG@10 of the arm without signal (vector only), overall.
    without_signal_ndcg: f64,
    /// Smallest recall of the exact persisted search against brute force.
    q1_recall: f64,
    hybrid_ndcg: f64,
    hybrid_recall: f64,
    /// Hybrid minus the arm without signal: overall and per class.
    gain: f64,
    class_gains: [f64; 4],
    /// Worst unjudged fraction, as `(positions, unjudged)` of the (arm, class) pair.
    q5_pairs: Vec<(usize, usize)>,
}

/// Every threshold of §5 for one engine, in the order of the acta.
fn thresholds(f: &Figures) -> Vec<(&'static str, bool)> {
    vec![
        ("T-E10-V1", f.vector_queries.iter().all(|n| *n >= V1_MIN_QUERIES_PER_CLASS)),
        ("T-E10-V2", f.without_signal_ndcg <= SATURATION_LIMIT),
        ("T-E10-Q1", f.q1_recall >= Q1_RECALL),
        ("T-E10-Q2", f.hybrid_ndcg >= Q2_NDCG),
        ("T-E10-Q3", f.hybrid_recall >= Q3_RECALL),
        ("T-E10-S1", s1_met(f.gain)),
        ("T-E10-S2", f.class_gains.iter().all(|g| s2_met(*g))),
        ("T-E10-Q5", f.q5_pairs.iter().all(|(p, u)| q5_met(*p, *u))),
    ]
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

/// The brute-force reference of Q1: every vector of the set against the
/// query, cosine descending then identifier ascending, cut at `CUT`.
fn brute_force(vectors: &[[i32; DIM]], query: &[i32; DIM]) -> Vec<u32> {
    let mut all: Vec<(u32, f64)> =
        vectors.iter().enumerate().map(|(i, v)| (i as u32, derived_eval::cosine(v, query))).collect();
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    all.into_iter().take(CUT).map(|(i, _)| i).collect()
}

/// What each arm returned for one query, as corpus identifiers.
struct Run {
    id: String,
    class: Class,
    expected: Vec<Judgment>,
    simple: Vec<u32>,
    ranked: Vec<u32>,
    vector: Vec<u32>,
    hybrid: Vec<u32>,
    brute: Vec<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    Simple,
    Ranked,
    Vector,
    Hybrid,
    Brute,
}

const ARMS: [Arm; 5] = [Arm::Simple, Arm::Ranked, Arm::Vector, Arm::Hybrid, Arm::Brute];

impl Arm {
    fn name(self) -> &'static str {
        match self {
            Arm::Simple => "simple",
            Arm::Ranked => "ranked",
            Arm::Vector => "vector",
            Arm::Hybrid => "hybrid",
            Arm::Brute => "brute",
        }
    }

    fn list(self, run: &Run) -> &[u32] {
        match self {
            Arm::Simple => &run.simple,
            Arm::Ranked => &run.ranked,
            Arm::Vector => &run.vector,
            Arm::Hybrid => &run.hybrid,
            Arm::Brute => &run.brute,
        }
    }
}

/// `None` is all the queries.
fn in_scope(run: &Run, class: Option<Class>) -> bool {
    class.is_none_or(|c| run.class == c)
}

fn scope_name(class: Option<Class>) -> &'static str {
    class.map_or("all-100", |c| c.name())
}

/// `(queries, recall@10, nDCG@10)` means of an arm over a scope.
fn mean_of(runs: &[Run], arm: Arm, class: Option<Class>) -> (usize, f64, f64) {
    let scores: Vec<Scored> =
        runs.iter().filter(|r| in_scope(r, class)).map(|r| score(arm.list(r), &r.expected)).collect();
    (scores.len(), mean(scores.iter().map(|s| s.recall)), mean(scores.iter().map(|s| s.ndcg)))
}

/// `(positions, unjudged)` of an arm over a scope.
fn positions_of(runs: &[Run], arm: Arm, class: Option<Class>) -> (usize, usize) {
    runs.iter()
        .filter(|r| in_scope(r, class))
        .map(|r| score(arm.list(r), &r.expected))
        .fold((0, 0), |(p, u), s| (p + s.positions, u + s.unjudged))
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
    let mut vector_set = VectorSet::new(DIM, VectorMetric::Cosine)?;
    for (i, derived) in loaded.derived.iter().enumerate() {
        let id = derived.ok_or("a unit has no persisted vector")?;
        let desc = dd.descriptor(&id).ok_or("a persisted vector has no descriptor")?;
        assert_eq!(desc.sources().map(|s| s.unit).collect::<Vec<_>>(), [loaded.units[i]]);
        assert_eq!(dd.state(&id), Some(DerivedState::Ready));
        vector_set.insert(loaded.units[i], dd.vector(&id).ok_or("a ready vector is not served")?)?;
    }
    dd.close()?;
    put!(
        out,
        "load: {} units, {} relations, {} change sets; {} vectors persisted, reopened and served",
        UNITS,
        set.corpus.relations.len(),
        set.corpus.changes.len(),
        reopen.served
    );

    let db = open_units::<E>(disk.as_vfs(), &cfg)?;
    let snap = db.snapshot()?;
    let corpus_id: HashMap<UnitId, u32> = loaded.units.iter().enumerate().map(|(i, u)| (*u, i as u32)).collect();
    assert_eq!(loaded.units[7], unit_id(7), "a unit's id is its corpus identifier");

    let mut runs: Vec<Run> = Vec::new();
    let mut fallbacks = 0usize;
    for q in &set.queries.queries {
        let query = assoc_query(&loaded.units, q);
        let simple = snap.reference_assoc_query(&query)?;
        let indexed = snap.indexed_assoc_query(&query)?;
        assert_eq!(indexed, simple, "{}: the indexed path differs from the reference path (I-32)", q.id);
        let planned = snap.assoc_query(&query)?;
        assert_eq!(planned.result, simple, "{}: the planner's answer differs from the reference", q.id);
        fallbacks += usize::from(planned.fallback.is_some());

        let ranked_hits = snap.rank_assoc(&simple, &RankOptions { mode: RankMode::On, ..RankOptions::default() })?;
        let ids = |units: &mut dyn Iterator<Item = UnitId>| -> Vec<u32> { units.map(|u| corpus_id[&u]).collect() };
        let qv = q.vector.as_ref().ok_or("a query of this set has no vector")?;
        let vq = VectorQuery::new(VectorMetric::Cosine, f32_vector(qv))?;
        let exact = vector_set.search(&vq, CUT)?;
        let opts = RankOptions {
            mode: RankMode::On,
            vectors: Some(VectorSignal { set: &vector_set, query: &vq }),
            ..RankOptions::default()
        };
        let hybrid = snap.rank_assoc(&simple, &opts)?;
        runs.push(Run {
            id: q.id.clone(),
            class: q.class,
            expected: judgments(q),
            simple: ids(&mut simple.hits.iter().take(CUT).map(|h| h.unit)),
            ranked: ids(&mut ranked_hits.iter().take(CUT).map(|h| h.unit)),
            vector: ids(&mut exact.iter().map(|s| s.unit)),
            hybrid: ids(&mut hybrid.iter().take(CUT).map(|h| h.unit)),
            brute: brute_force(&set.vectors, qv),
        });
    }
    put!(
        out,
        "queries: {} translated and run on the reference, indexed and planner paths; planner fallbacks: {fallbacks}",
        runs.len()
    );

    // ---- T-E10-V1: queries with a query vector, per class.
    put!(out, "T-E10-V1  queries that carry a query vector, by class (at least {V1_MIN_QUERIES_PER_CLASS} each)");
    let mut vector_queries = [0usize; 4];
    for (slot, class) in vector_queries.iter_mut().zip(CLASSES) {
        *slot = runs.iter().filter(|r| r.class == class).count();
        put!(out, "  {:<22} {:>3}", class.name(), *slot);
    }

    // ---- T-E10-Q1: exact search of E8 against the independent brute force.
    put!(out, "T-E10-Q1  exact vector search over the persisted vectors against brute force (cosine), cut {CUT}");
    let mut overlaps = Vec::new();
    let mut same_order = 0usize;
    for r in &runs {
        let common = r.vector.iter().filter(|u| r.brute.contains(u)).count();
        overlaps.push(common as f64 / CUT as f64);
        same_order += usize::from(r.vector == r.brute);
        if r.vector != r.brute {
            put!(out, "  {}: overlap {common}/{CUT}, same order: {}", r.id, r.vector == r.brute);
        }
    }
    let q1_recall = overlaps.iter().copied().fold(f64::INFINITY, f64::min);
    put!(
        out,
        "  recall@{CUT} against the reference: smallest {q1_recall:.3}, mean {:.3} over {} queries; identical lists in the same order: {same_order}",
        mean(overlaps.iter().copied()),
        overlaps.len()
    );

    // ---- T-E10-Q5: the validity of the judgments, per arm and per class.
    put!(
        out,
        "T-E10-Q5  unjudged units in the top {CUT}, per arm and per class (excluded from the metrics and counted here)"
    );
    let mut q5_pairs = Vec::new();
    for arm in ARMS {
        for class in std::iter::once(None).chain(CLASSES.map(Some)) {
            let (positions, unjudged) = positions_of(&runs, arm, class);
            q5_pairs.push((positions, unjudged));
            put!(
                out,
                "  {:<7} {:<22} {positions:>4} positions, {unjudged:>4} unjudged ({:.1} %) -> {}",
                arm.name(),
                scope_name(class),
                100.0 * unjudged as f64 / positions.max(1) as f64,
                verdict(q5_met(positions, unjudged))
            );
        }
    }

    // ---- Means.
    put!(out, "means (cut {CUT}; unjudged units excluded): arm, scope, queries, recall@10, nDCG@10");
    for arm in ARMS {
        for class in std::iter::once(None).chain(CLASSES.map(Some)) {
            let m = mean_of(&runs, arm, class);
            put!(out, "  {:<7} {:<22} {:>3} {:>8.4} {:>8.4}", arm.name(), scope_name(class), m.0, m.1, m.2);
        }
    }

    // ---- T-E10-V2, Q2, Q3, S1, S2.
    let hybrid = mean_of(&runs, Arm::Hybrid, None);
    let without = mean_of(&runs, Arm::Vector, None);
    let ranked = mean_of(&runs, Arm::Ranked, None);
    let mut class_gains = [0.0; 4];
    let mut class_gains_ranked = [0.0; 4];
    for (i, class) in CLASSES.iter().enumerate() {
        let h = mean_of(&runs, Arm::Hybrid, Some(*class)).2;
        class_gains[i] = h - mean_of(&runs, Arm::Vector, Some(*class)).2;
        class_gains_ranked[i] = h - mean_of(&runs, Arm::Ranked, Some(*class)).2;
    }
    let figures = Figures {
        vector_queries,
        without_signal_ndcg: without.2,
        q1_recall,
        hybrid_ndcg: hybrid.2,
        hybrid_recall: hybrid.1,
        gain: hybrid.2 - without.2,
        class_gains,
        q5_pairs,
    };
    put!(out, "thresholds of acta 9 §5, primary reading (without signal = the vector-only arm):");
    let verdicts = thresholds(&figures);
    put!(
        out,
        "  T-E10-V2  nDCG@10 without signal, overall <= {SATURATION_LIMIT:.2}: {:.4}",
        figures.without_signal_ndcg
    );
    put!(out, "  T-E10-Q2  nDCG@10 with signal, overall >= {Q2_NDCG:.2}: {:.4}", figures.hybrid_ndcg);
    put!(out, "  T-E10-Q3  recall@10 with signal, overall >= {Q3_RECALL:.2}: {:.4}", figures.hybrid_recall);
    put!(out, "  T-E10-S1  nDCG@10 with signal - without signal, overall >= {S1_MIN_GAIN:.2}: {:+.4}", figures.gain);
    for (class, gain) in CLASSES.iter().zip(figures.class_gains) {
        put!(
            out,
            "  T-E10-S2  {:<22} with - without >= {S2_MIN_GAIN:+.2}: {gain:+.4} -> {}",
            class.name(),
            verdict(s2_met(gain))
        );
    }
    put!(out, "second reading (without signal = the ranked arm with the vector signal off), informational:");
    let gain_ranked = hybrid.2 - ranked.2;
    put!(out, "  T-E10-S1  overall >= {S1_MIN_GAIN:.2}: {gain_ranked:+.4} -> {}", verdict(s1_met(gain_ranked)));
    for (class, gain) in CLASSES.iter().zip(class_gains_ranked) {
        put!(out, "  T-E10-S2  {:<22} >= {S2_MIN_GAIN:+.2}: {gain:+.4} -> {}", class.name(), verdict(s2_met(gain)));
    }
    put!(out, "verdicts, primary reading:");
    for (code, met) in verdicts {
        put!(out, "  {code}: {}", verdict(met));
    }
    Ok(out)
}

// -------------------------------------------------------------------- tests

/// Runs both engines, prints their reports, and checks that they agree.
#[test]
fn quality_over_the_second_frozen_set() {
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
        without_signal_ndcg: 0.42,
        q1_recall: 1.0,
        hybrid_ndcg: 0.90,
        hybrid_recall: 0.95,
        gain: 0.03,
        class_gains: [0.0; 4],
        q5_pairs: vec![(100, 10)],
    }
}

fn met(f: &Figures, code: &str) -> bool {
    thresholds(f).into_iter().find(|(c, _)| *c == code).expect("a threshold of that code").1
}

/// Each threshold flips at its own bound, as the acta writes it, and only it.
#[test]
fn every_threshold_flips_at_its_own_bound() {
    let base = passing();
    assert!(thresholds(&base).iter().all(|(_, m)| *m), "the reference figures meet everything");
    let flip = |code: &str, edit: &dyn Fn(&mut Figures)| {
        let mut f = passing();
        edit(&mut f);
        let after = thresholds(&f);
        let failing: Vec<&str> = after.iter().filter(|(_, m)| !*m).map(|(c, _)| *c).collect();
        assert_eq!(failing, [code], "only {code} may fail");
    };
    flip("T-E10-V1", &|f| f.vector_queries[2] = 24);
    flip("T-E10-V2", &|f| f.without_signal_ndcg = 0.9601);
    flip("T-E10-Q1", &|f| f.q1_recall = 0.9);
    flip("T-E10-Q2", &|f| f.hybrid_ndcg = 0.7999);
    flip("T-E10-Q3", &|f| f.hybrid_recall = 0.8999);
    flip("T-E10-S1", &|f| f.gain = 0.0199);
    flip("T-E10-S2", &|f| f.class_gains[3] = -0.0101);
    flip("T-E10-Q5", &|f| f.q5_pairs.push((100, 11)));
    // The bounds themselves are inside.
    let mut edge = passing();
    edge.vector_queries = [25; 4];
    edge.without_signal_ndcg = SATURATION_LIMIT;
    edge.hybrid_ndcg = Q2_NDCG;
    edge.hybrid_recall = Q3_RECALL;
    edge.gain = S1_MIN_GAIN;
    edge.class_gains = [S2_MIN_GAIN; 4];
    edge.q5_pairs = vec![(100, 10)];
    assert!(
        ["T-E10-V1", "T-E10-V2", "T-E10-Q2", "T-E10-Q3", "T-E10-S1", "T-E10-S2", "T-E10-Q5"]
            .iter()
            .all(|c| met(&edge, c))
    );
}
