//! E8-07: quality (acta §6.2, Q1 to Q5) and the equalities of §6.3 (P3, P4)
//! over the frozen derived-data evaluation set. The report of the committed
//! run is `docs/architecture/etapa8-medicion-salida.txt`; the write-up is
//! `docs/architecture/etapa8-medicion.md`. P1 and P2 (ratios of time) are in
//! `examples/derived_bench.rs`.
//!
//! The thresholds are those of the acta, fixed before any measurement of this
//! set (decision D4). They are printed with their number and whether they are
//! met. They do not fail the test: a threshold that is not met is recorded,
//! never adjusted. What does fail the test is what the measurement depends on
//! (the MANIFEST, the reopened database serving every persisted vector, the
//! indexed path agreeing with the reference path) and the two equalities P3
//! and P4, which are checked after the report is printed.
//!
//! Arms, fixed before the first run, cut at 10, over the 100 queries:
//!
//! * `simple`: the associative query in the reference order, "distance, then
//!   unit id" (`reference_assoc_query`). Unit ids are the permuted corpus
//!   identifiers, so the tie-break favours no structure.
//! * `ranked`: the same result under `RankMode::On`, default weights, no
//!   vectors. This is the arm "without the vector signal" of Q4.
//! * `vector` (only the 25 queries that carry a query vector): the exact
//!   cosine search over the persisted vectors, read back from the reopened
//!   database.
//! * `hybrid` (the same 25): the `ranked` arm with the vector signal on
//!   (`RankOptions::vectors`), over the persisted vectors. This is the arm
//!   "with the vector signal" of Q2, Q3 and Q4.
//! * `brute` (the same 25): an independent brute-force cosine order over the
//!   integer vectors of the set (`derived_eval::cosine`), the reference of Q1.
//!
//! Unjudged units in the top 10 of an arm are excluded from the list before
//! the metrics are computed (the list is condensed, never extended beyond the
//! cut) and counted per arm (acta §6.1). The ideal ranking of nDCG and the
//! relevant total of recall stay those of the whole judgment list.
//!
//! ```text
//! cargo test -p kerno-units --release --test derived_measure -- --nocapture --exact quality_over_the_frozen_set
//! ```

mod harness;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use harness::derived::{CORPUS, MANIFEST, PermutedIds, QUERIES, T0, VECTORS, f32_vector, load, recipe, unit_id};
use harness::units::{Target, open_units};
use kerno_core::contract::{Options, Snapshot};
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, Judgment};
use kerno_testkit::assoc_metrics::{ndcg_at, recall_at};
use kerno_testkit::derived_eval::{self, CUT, Class, DIM, Query};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::sha256;
use kerno_testkit::vfs::SimVfs;
use kerno_units::assoc_rank::{RankMode, RankOptions, VectorMetric, VectorQuery, VectorSet, VectorSignal};
use kerno_units::vector_store::VectorDescriptor;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;
type Res<T> = Result<T, Box<dyn std::error::Error>>;

const CLASSES: [Class; 4] = [Class::VectorNeighbourhood, Class::Temporal, Class::Provenance, Class::Combined];

/// Thresholds of acta §6.2, as written there.
const Q1_RECALL: f64 = 1.0;
const Q2_NDCG: f64 = 0.80;
const Q3_RECALL: f64 = 0.90;
const Q4_SLACK: f64 = 0.01;
/// Q5: at most one position in ten may be unjudged.
const Q5_MAX_UNJUDGED_TENTHS: usize = 1;

const UNITS: usize = 2_000;
const QUERY_COUNT: usize = 100;
const VECTOR_QUERIES: usize = 25;

// ------------------------------------------------------------------ MANIFEST

/// Checks the frozen files against `MANIFEST`: the three files, in order, with
/// their SHA-256. Nothing is measured if this fails.
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
/// visit budget are the hard maxima of the crate, for every query. A step is
/// the commit sequence of its change set (checked when the corpus is loaded).
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
    /// `vector`, `hybrid` and `brute`: only for a query with a query vector.
    vector: Option<(Vec<u32>, Vec<u32>, Vec<u32>)>,
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

    /// The top of the arm for `run`, if the arm exists for it.
    fn list(self, run: &Run) -> Option<&[u32]> {
        match self {
            Arm::Simple => Some(&run.simple),
            Arm::Ranked => Some(&run.ranked),
            Arm::Vector => run.vector.as_ref().map(|v| v.0.as_slice()),
            Arm::Hybrid => run.vector.as_ref().map(|v| v.1.as_slice()),
            Arm::Brute => run.vector.as_ref().map(|v| v.2.as_slice()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    All,
    WithVector,
    Class(Class),
}

const SCOPES: [Scope; 6] = [
    Scope::All,
    Scope::WithVector,
    Scope::Class(Class::VectorNeighbourhood),
    Scope::Class(Class::Temporal),
    Scope::Class(Class::Provenance),
    Scope::Class(Class::Combined),
];

impl Scope {
    fn name(self) -> &'static str {
        match self {
            Scope::All => "all-100",
            Scope::WithVector => "with-vector-25",
            Scope::Class(c) => c.name(),
        }
    }

    fn holds(self, run: &Run) -> bool {
        match self {
            Scope::All => true,
            Scope::WithVector => run.vector.is_some(),
            Scope::Class(c) => run.class == c,
        }
    }

    /// The scope an arm exists on.
    fn of_arm(arm: Arm) -> Scope {
        if matches!(arm, Arm::Simple | Arm::Ranked) { Scope::All } else { Scope::WithVector }
    }
}

/// `(queries, recall@10, nDCG@10)` means of an arm over a scope.
fn mean_of(runs: &[Run], arm: Arm, scope: Scope) -> (usize, f64, f64) {
    let scores: Vec<Scored> =
        runs.iter().filter(|r| scope.holds(r)).filter_map(|r| arm.list(r).map(|l| score(l, &r.expected))).collect();
    (scores.len(), mean(scores.iter().map(|s| s.recall)), mean(scores.iter().map(|s| s.ndcg)))
}

/// `(positions, unjudged, queries)` of an arm over a scope.
fn positions_of(runs: &[Run], arm: Arm, scope: Scope) -> (usize, usize, usize) {
    runs.iter()
        .filter(|r| scope.holds(r))
        .filter_map(|r| arm.list(r).map(|l| score(l, &r.expected)))
        .fold((0, 0, 0), |(p, u, n), s| (p + s.positions, u + s.unjudged, n + 1))
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
    assert_eq!(queries.queries.iter().filter(|q| q.vector.is_some()).count(), VECTOR_QUERIES);
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
    let disk = SimVfs::new(81);
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
        let vector = match &q.vector {
            None => None,
            Some(qv) => {
                let vq = VectorQuery::new(VectorMetric::Cosine, f32_vector(qv))?;
                let exact = vector_set.search(&vq, CUT)?;
                let opts = RankOptions {
                    mode: RankMode::On,
                    vectors: Some(VectorSignal { set: &vector_set, query: &vq }),
                    ..RankOptions::default()
                };
                let hybrid = snap.rank_assoc(&simple, &opts)?;
                Some((
                    ids(&mut exact.iter().map(|s| s.unit)),
                    ids(&mut hybrid.iter().take(CUT).map(|h| h.unit)),
                    brute_force(&set.vectors, qv),
                ))
            }
        };
        runs.push(Run {
            id: q.id.clone(),
            class: q.class,
            expected: judgments(q),
            simple: ids(&mut simple.hits.iter().take(CUT).map(|h| h.unit)),
            ranked: ids(&mut ranked_hits.iter().take(CUT).map(|h| h.unit)),
            vector,
        });
    }
    put!(
        out,
        "queries: {} translated and run on the reference, indexed and planner paths; planner fallbacks: {fallbacks}",
        runs.len()
    );

    let with_vector: Vec<&Run> = runs.iter().filter(|r| r.vector.is_some()).collect();
    let count_by_class = |class: Class| with_vector.iter().filter(|r| r.class == class).count();
    put!(out, "queries that carry a query vector, by class (the only ones the vector signal can be computed on):");
    for class in CLASSES {
        let all = runs.iter().filter(|r| r.class == class).count();
        put!(out, "  {:<22} {:>3} of {:>3}", class.name(), count_by_class(class), all);
    }

    // ---- Q1: exact search of E8 against the independent brute force.
    put!(out, "Q1  exact vector search over the persisted vectors against brute force (cosine), cut {CUT}");
    let mut overlaps = Vec::new();
    let mut same_order = 0usize;
    for r in &with_vector {
        let (exact, _, brute) = r.vector.as_ref().ok_or("no vector arm")?;
        let common = exact.iter().filter(|u| brute.contains(u)).count();
        overlaps.push(common as f64 / CUT as f64);
        same_order += usize::from(exact == brute);
        if exact != brute {
            put!(out, "  {}: overlap {common}/{CUT}, same order: {}", r.id, exact == brute);
        }
    }
    let q1 = mean(overlaps.iter().copied());
    put!(
        out,
        "  recall@{CUT} against the reference, mean over {} queries: {q1:.3}; identical lists in the same order: {same_order}",
        overlaps.len()
    );
    let q1_met = overlaps.iter().all(|o| *o >= Q1_RECALL);
    put!(
        out,
        "Q1  recall@10 = {Q1_RECALL:.3}, computed on the {} queries with a vector: {:.3} -> {}",
        overlaps.len(),
        q1,
        verdict(q1_met)
    );

    // ---- Q5 first: the validity of the set, per arm.
    put!(out, "Q5  unjudged units in the top {CUT}, per arm (excluded from the metrics and counted here)");
    let mut q5_met = true;
    for arm in ARMS {
        let (positions, unjudged, queries) = positions_of(&runs, arm, Scope::of_arm(arm));
        let met = unjudged * 10 <= positions * Q5_MAX_UNJUDGED_TENTHS;
        q5_met &= met;
        put!(
            out,
            "  {:<7} {queries:>3} queries, {positions:>4} positions, {unjudged:>4} unjudged ({:.1} %) -> {}",
            arm.name(),
            100.0 * unjudged as f64 / positions.max(1) as f64,
            verdict(met)
        );
    }
    for arm in [Arm::Simple, Arm::Ranked] {
        for class in CLASSES {
            let (p, u, n) = positions_of(&runs, arm, Scope::Class(class));
            put!(out, "    {:<7} {:<22} {n:>3} queries, {p:>4} positions, {u:>4} unjudged", arm.name(), class.name());
        }
    }
    put!(out, "Q5  at most 10 % unjudged in the top {CUT} of every arm: {}", verdict(q5_met));
    if !q5_met {
        put!(
            out,
            "Q5  not met: the set is incomplete for that arm and Q2 to Q4 are not evaluated (acta §6.1: judge more, then measure)"
        );
    }

    // ---- Means.
    put!(out, "means (cut {CUT}; unjudged units excluded): arm, scope, queries, recall@10, nDCG@10");
    for arm in ARMS {
        for scope in SCOPES {
            let m = mean_of(&runs, arm, scope);
            if m.0 > 0 {
                put!(out, "  {:<7} {:<22} {:>3} {:>8.3} {:>8.3}", arm.name(), scope.name(), m.0, m.1, m.2);
            }
        }
    }

    // ---- Q2, Q3, Q4 (only when Q5 holds).
    let hybrid = mean_of(&runs, Arm::Hybrid, Scope::WithVector);
    let ranked = mean_of(&runs, Arm::Ranked, Scope::WithVector);
    let simple = mean_of(&runs, Arm::Simple, Scope::WithVector);
    if q5_met {
        put!(
            out,
            "Q2  nDCG@10 with the vector signal (hybrid), overall >= {Q2_NDCG:.2}, computed on the {} queries with a vector: {:.3} -> {}",
            hybrid.0,
            hybrid.2,
            verdict(hybrid.2 >= Q2_NDCG)
        );
        put!(
            out,
            "Q3  recall@10 with the vector signal (hybrid), overall >= {Q3_RECALL:.2}, computed on the {} queries with a vector: {:.3} -> {}",
            hybrid.0,
            hybrid.1,
            verdict(hybrid.1 >= Q3_RECALL)
        );
        let bar = ranked.2 - Q4_SLACK;
        let overall_met = hybrid.2 >= bar;
        put!(
            out,
            "Q4  nDCG@10 with the signal >= without it - {Q4_SLACK:.2} (without = the ranked arm with the signal off):"
        );
        put!(
            out,
            "      overall ({} queries with a vector): {:.3} vs {:.3} - {Q4_SLACK:.2} = {bar:.3} -> {}",
            hybrid.0,
            hybrid.2,
            ranked.2,
            verdict(overall_met)
        );
        put!(
            out,
            "      vector_neighbourhood (the same {} queries): {:.3} vs {bar:.3} -> {}",
            hybrid.0,
            hybrid.2,
            verdict(overall_met)
        );
        for class in CLASSES.iter().skip(1) {
            put!(
                out,
                "      {:<22} not measurable: {} of {} queries carry a query vector, so the signal cannot be applied",
                class.name(),
                count_by_class(*class),
                runs.iter().filter(|r| r.class == *class).count()
            );
        }
        let q4_all = overall_met;
        put!(
            out,
            "Q4  {}",
            if q4_all {
                "met on the scopes that can be computed; NOT closed: 3 of 4 classes are not measurable"
            } else {
                "not met on the scopes that can be computed; 3 of 4 classes are in addition not measurable"
            }
        );
        let alt_bar = simple.2 - Q4_SLACK;
        put!(
            out,
            "Q4  secondary reading (without = the simple order), overall: {:.3} vs {:.3} - {Q4_SLACK:.2} = {alt_bar:.3} -> {}",
            hybrid.2,
            simple.2,
            verdict(hybrid.2 >= alt_bar)
        );
    } else {
        put!(out, "Q2, Q3, Q4: not evaluated because Q5 is not met");
    }
    put!(
        out,
        "Q6  not measured: it applies only if the ranking is proposed as the default later, and is not a closure criterion of Stage 8"
    );
    Ok(out)
}

// ------------------------------------------------- P3 and P4: the equalities

/// The edit applied to a source: a new payload, so a new revision.
fn edited(label: &str) -> Vec<u8> {
    format!("edited:{label}").into_bytes()
}

fn raw_get<E: Target>(db: &UnitDb<E>, key: &[u8]) -> Option<Vec<u8>> {
    db.engine().snapshot().expect("snapshot").get(key).expect("get")
}

/// The three persisted records of a datum: descriptor, vector, `0x62` entry.
fn records<E: Target>(db: &UnitDb<E>, id: DerivedId) -> Res<[Vec<u8>; 3]> {
    let desc = raw_get(db, &DerivedKey::Desc(id).encode()).ok_or("no descriptor")?;
    let source = VectorDescriptor::decode(&desc).map_err(|e| format!("descriptor: {e:?}"))?.source;
    let vector = raw_get(db, &DerivedKey::Vec(id).encode()).ok_or("no vector")?;
    let entry = raw_get(db, &DerivedKey::Src { unit: source.unit, rev: source.rev, id }.encode()).ok_or("no entry")?;
    Ok([desc, vector, entry])
}

/// P3 and P4 on one engine. Returns the lines of the report that do not depend
/// on time, the lines that do, and whether both equalities hold.
fn equalities<E: Target>(set: &Set) -> Res<(Lines, Lines, bool)> {
    let mut out: Lines = Vec::new();
    let mut timing: Lines = Vec::new();
    let cfg = config(&set.corpus);
    let label_of: HashMap<u32, String> = set.corpus.units.iter().map(|u| (u.id, u.label.clone())).collect();
    // The units whose source changes: every seventh identifier from 3.
    let changed: Vec<u32> = (0..UNITS as u32).filter(|i| i % 7 == 3).collect();
    let changed_set: BTreeSet<u32> = changed.iter().copied().collect();

    // The vector of a revision: the seeded vector of the unit, moved by the
    // length of the payload, so a new revision gives a new vector.
    let base: HashMap<UnitId, Vec<f32>> =
        (0..UNITS).map(|i| (unit_id(i as u32), f32_vector(&set.vectors[i]))).collect();
    let vcompute = |_: &DerivedDescriptor, inputs: &[SourceInput]| -> Option<Vec<f32>> {
        let first = inputs.first()?;
        let mut v = base.get(&first.id.unit)?.clone();
        v[0] += first.payload.len() as f32;
        Some(v)
    };
    let concat = |_: &DerivedDescriptor, inputs: &[SourceInput]| -> Option<Vec<u8>> {
        Some(inputs.iter().flat_map(|i| i.payload.iter().copied()).collect())
    };
    let edit_all = |dd: &DerivedDb<E>, loaded_units: &[UnitId]| -> Res<()> {
        for group in changed.chunks(10) {
            let mut cs = dd.begin_changeset(ChangeMeta::default())?;
            for i in group {
                let upd = UnitUpdate { payload: edited(&label_of[i]), schema_version: None, metadata: None };
                cs.update(loaded_units[*i as usize], upd, None)?;
            }
            cs.commit()?;
        }
        Ok(())
    };

    // Treatment: every unit has a persisted vector; pairs of units have an
    // in-memory datum with two sources.
    let disk = SimVfs::new(82);
    let (dd, _) = DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg)?)?;
    let loaded = load(&dd, &set.corpus, &set.vectors, true)?;
    let mut pairs: Vec<(DerivedId, u32, u32)> = Vec::new();
    for k in (0..UNITS as u32).step_by(5) {
        let (a, b) = (k, (k + 1) % UNITS as u32);
        let units = [loaded.units[a as usize], loaded.units[b as usize]];
        let params = BTreeMap::from([("pair".to_owned(), "concat".to_owned())]);
        pairs.push((dd.build(units, "pair-concat", 1, params, &concat)?, a, b));
    }
    let persisted: Vec<DerivedId> = loaded.derived.iter().map(|d| d.expect("persisted")).collect();
    edit_all(&dd, &loaded.units)?;

    // P3: the stale entries are exactly the derived data of the sources that changed.
    let mut expected: BTreeSet<DerivedId> =
        (0..UNITS as u32).filter(|i| changed_set.contains(i)).map(|i| persisted[i as usize]).collect();
    let expected_persisted = expected.clone();
    expected.extend(pairs.iter().filter(|(_, a, b)| changed_set.contains(a) || changed_set.contains(b)).map(|p| p.0));
    let everything: Vec<DerivedId> = persisted.iter().copied().chain(pairs.iter().map(|p| p.0)).collect();
    let stale: BTreeSet<DerivedId> =
        everything.iter().copied().filter(|id| dd.state(id) == Some(DerivedState::Stale)).collect();
    let others_ready =
        everything.iter().filter(|id| !stale.contains(id)).all(|id| dd.state(id) == Some(DerivedState::Ready));
    let p3_live = stale == expected && others_ready;
    put!(
        out,
        "P3  after {} source changes in {} change sets: {} derived data, {} expected stale, {} stale, the rest ready: {others_ready}",
        changed.len(),
        changed.len().div_ceil(10),
        everything.len(),
        expected.len(),
        stale.len()
    );
    dd.close()?;

    // The same after reopening. The change set that edited a source also
    // wrote the stale state of its persisted data, so they load as stale;
    // `demoted` lists only data persisted as ready that open found stale
    // (none here), so the stale set is read from the registry.
    let (dd, reopen) = DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg)?)?;
    let stale_after: BTreeSet<DerivedId> =
        persisted.iter().copied().filter(|id| dd.state(id) == Some(DerivedState::Stale)).collect();
    let p3_reopened = stale_after == expected_persisted
        && reopen.served == UNITS - changed.len()
        && reopen.demoted.is_empty()
        && persisted.iter().filter(|id| !stale_after.contains(id)).all(|id| dd.state(id) == Some(DerivedState::Ready));
    put!(
        out,
        "P3  after reopening: {} persisted data stale, {} expected, {} served ready, {} demoted by open",
        stale_after.len(),
        expected_persisted.len(),
        reopen.served,
        reopen.demoted.len()
    );
    let p3 = p3_live && p3_reopened;
    put!(out, "P3  the stale entries are exactly the derived data of the changed sources: {}", verdict(p3));

    // P4: rebuild every stale vector; the control builds the same data over
    // the same history without ever having had the old ones.
    let started = Instant::now();
    let mut rebuilt: BTreeMap<u32, DerivedId> = BTreeMap::new();
    for i in &changed {
        let new = dd.rebuild_vector(&persisted[*i as usize], ChangeMeta::default(), &vcompute)?;
        rebuilt.insert(*i, new);
    }
    timing.push(format!(
        "P4  rebuild of {} stale vectors, one change set each: {:.1} ms in total",
        changed.len(),
        started.elapsed().as_secs_f64() * 1e3
    ));
    let still_stale = persisted.iter().filter(|id| dd.state(id) == Some(DerivedState::Stale)).count();
    dd.close()?;
    let treated = open_units::<E>(disk.as_vfs(), &cfg)?;

    let control_disk = SimVfs::new(83);
    let (control, _) = DerivedDb::open_persisted(open_units::<E>(control_disk.as_vfs(), &config(&set.corpus))?)?;
    let control_loaded = load(&control, &set.corpus, &set.vectors, false)?;
    assert_eq!(control_loaded.units, loaded.units, "the two histories mint the same unit ids");
    edit_all(&control, &control_loaded.units)?;
    let started = Instant::now();
    let mut fresh: BTreeMap<u32, DerivedId> = BTreeMap::new();
    for i in &changed {
        fresh.insert(
            *i,
            control.build_vector(control_loaded.units[*i as usize], &recipe(), ChangeMeta::default(), &vcompute)?,
        );
    }
    timing.push(format!(
        "P4  build from scratch of the same {} vectors: {:.1} ms in total",
        changed.len(),
        started.elapsed().as_secs_f64() * 1e3
    ));
    control.close()?;
    let fresh_db = open_units::<E>(control_disk.as_vfs(), &cfg)?;

    let mut equal_bytes = 0usize;
    for i in &changed {
        let same_id = rebuilt[i] == fresh[i];
        let same = same_id && records(&treated, rebuilt[i])? == records(&fresh_db, fresh[i])?;
        equal_bytes += usize::from(same);
    }
    let p4 = equal_bytes == changed.len() && still_stale == 0;
    put!(
        out,
        "P4  {} rebuilt vectors: identity, descriptor, vector and entry byte for byte equal to a fresh build: {equal_bytes}; stale left after the rebuild: {still_stale}",
        changed.len()
    );
    put!(out, "P4  rebuilt = built anew, byte for byte: {}", verdict(p4));
    Ok((out, timing, p3 && p4))
}

// -------------------------------------------------------------------- tests

/// Runs both engines, prints their reports, and checks that they agree.
#[test]
fn quality_over_the_frozen_set() {
    let set = frozen_set();
    println!("MANIFEST: corpus.txt, vectors.txt and queries.txt match");
    let mut reports: Vec<(&str, Lines)> = Vec::new();
    let mut equalities_hold = true;
    for (name, quality_lines, eq) in [
        ("emulated_log", quality::<EmuLog>(&set), equalities::<EmuLog>(&set)),
        ("kerno_db", quality::<KernoDb>(&set), equalities::<KernoDb>(&set)),
    ] {
        let mut lines = quality_lines.expect("quality measurement");
        let (eq_lines, timing, ok) = eq.expect("equalities");
        lines.extend(eq_lines);
        println!("== engine {name}");
        for l in lines.iter().chain(&timing) {
            println!("{l}");
        }
        equalities_hold &= ok;
        reports.push((name, lines));
    }
    assert_eq!(reports[0].1, reports[1].1, "the two engines report different figures");
    println!("engines: emulated_log and kerno_db report identical figures ({} lines)", reports[0].1.len());
    assert!(equalities_hold, "P3 or P4 is not met: see the report above");
}

#[test]
fn the_manifest_of_the_frozen_set_matches() {
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
