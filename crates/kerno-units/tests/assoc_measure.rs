//! E7-05: quality of the associative memory over the frozen evaluation set
//! (acta §6.1, §6.2). The report of the committed run is
//! `docs/architecture/etapa7-medicion-salida.txt`; the write-up is
//! `docs/architecture/etapa7-medicion.md`.
//!
//! The arms, over the 64 queries of `crates/kerno-testkit/data/assoc-eval`,
//! cut at 10:
//!
//! * `B1`: one-hop neighbours through `relations_of` (the baseline of E7-00,
//!   the same code as `tests/assoc_baseline.rs`).
//! * `simple`: the associative query in the reference order, "distance, then
//!   unit id" ([`UnitSnapshot::reference_assoc_query`]).
//! * `ranked`: the same result ordered by [`RankMode::On`] with the default
//!   weights, no preferred types and no vectors.
//!
//! The translation of a query into an [`AssocQuery`] is the rule of
//! `tests/harness/assoc.rs` (`corpus_query`), written before this measurement:
//! seed, direction, types, depth, window, provenance kinds, actor and source
//! come from the query file; the limit and the visit budget are the hard
//! maxima of the crate, the same for every query. The limit is the maximum so
//! that ranking orders the whole reachable answer and not a prefix of it.
//!
//! The thresholds of acta §6.2 are printed with their number and whether they
//! are met. They do not fail the test: a threshold that is not met is recorded,
//! never adjusted. What does fail the test is what the measurement itself
//! depends on: the MANIFEST, the equality of the indexed and the reference
//! path (I-32), the sum of the contributions (I-41), the trace of every hit
//! (I-40) and the reproduction of B1 from the committed run.
//!
//! The vector signal is not measured here: the set carries no vectors.
//!
//! ```text
//! cargo test -p kerno-units --release --test assoc_measure -- --nocapture --exact quality_over_the_frozen_set
//! ```

mod harness;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use harness::assoc::{CORPUS, QUERIES, corpus_query, load_corpus_with_relations};
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, CUT, Class, Query};
use kerno_testkit::assoc_metrics::{ndcg_at, recall_at};
use kerno_testkit::sha256;
use kerno_units::assoc_rank::{RankMode, RankOptions};
use kerno_units::*;

const MANIFEST: &str = include_str!("../../kerno-testkit/data/assoc-eval/MANIFEST");
/// The committed run of E7-00: its B1 columns must be reproduced.
const E700: &str = include_str!("../../../docs/architecture/etapa7-e700-salida.txt");

const CLASSES: [(Class, &str); 4] = [
    (Class::MultiHop, "multi_hop"),
    (Class::Temporal, "temporal"),
    (Class::Provenance, "provenance"),
    (Class::Combined, "combined"),
];

/// Thresholds of acta §6.2 (fixed in E7-00 and not changed here).
const RECALL_MIN: f64 = 0.90;
const NDCG_MIN: f64 = 0.80;
/// "al menos un 10 % mejor": the ranked nDCG must reach 1.10 times the other.
const GAIN: f64 = 1.10;

/// `[recall@10, nDCG@10]` of one arm for one query.
type Scores = [f64; 2];

/// A scope of the summary: its label and the rows it covers.
type Scope<'a> = (&'a str, Vec<&'a Row>);

struct Row {
    id: String,
    class: Class,
    hits: usize,
    ending: AssocEnding,
    b1: Scores,
    simple: Scores,
    ranked: Scores,
}

/// B1 as in `tests/assoc_baseline.rs`: one-hop neighbours, outgoing then
/// incoming, deduplicated, cut at 10.
fn baseline_b1(snap: &UnitSnapshot<KernoDb>, index_of: &HashMap<UnitId, u32>, seed: UnitId) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    for (dir, pick_other) in [(Direction::Out, true), (Direction::In, false)] {
        for rel in snap.relations_of(seed, dir, None).expect("relations_of") {
            let other = if pick_other { rel.to } else { rel.from };
            let index = index_of[&other];
            if !out.contains(&index) {
                out.push(index);
            }
        }
    }
    out.truncate(CUT);
    out
}

fn scores(results: &[u32], q: &Query) -> Scores {
    [recall_at(results, &q.expected, CUT), ndcg_at(results, &q.expected, CUT)]
}

/// I-40 on one result: every hit is a walk from a seed through hits of the
/// result, and every relation it cites exists in the snapshot with the fields
/// its trace states, follows the query, and was created no later than the
/// snapshot; its revision is the unit's head there. `None` when every hit
/// verifies, otherwise the first failure.
fn trace_failure(snap: &UnitSnapshot<KernoDb>, q: &AssocQuery, res: &AssocResult) -> Option<String> {
    let seq = snap.seq();
    let seeds: BTreeSet<UnitId> = q.seeds.iter().copied().collect();
    let distance: BTreeMap<UnitId, u32> = res.hits.iter().map(|h| (h.unit, h.distance)).collect();
    let out = matches!(q.direction, AssocDirection::Out | AssocDirection::Both);
    let inn = matches!(q.direction, AssocDirection::In | AssocDirection::Both);
    for hit in &res.hits {
        if seeds.contains(&hit.unit) || hit.path.len() != hit.distance as usize {
            return Some(format!("unit {}: a seed, or the distance is not the path length", hit.unit));
        }
        let unit = snap.reference_get(hit.unit).expect("unit read");
        let Some(unit) = unit else {
            return Some(format!("unit {}: a hit is not active", hit.unit));
        };
        let rev = snap.get_revision(hit.unit, hit.revision).expect("cited revision");
        if hit.revision != unit.head || rev.commit_seq > seq {
            return Some(format!("unit {}: the revision is not the head at the snapshot", hit.unit));
        }
        for s in &hit.path {
            let Some(stored) = snap.get_relation(s.relation).expect("relation read") else {
                return Some(format!("relation {} is not in the snapshot", s.relation));
            };
            let trace = (s.from, s.to, s.relation_type.as_str(), s.created_seq, s.provenance);
            let kept =
                (stored.from, stored.to, stored.relation_type.as_str(), stored.created_seq, stored.provenance.kind());
            if trace != kept || stored.created_seq > seq {
                return Some(format!("relation {}: the trace differs from the stored relation", s.relation));
            }
            if q.relation_types.as_ref().is_some_and(|t| !t.contains(&s.relation_type)) {
                return Some(format!("relation {}: its type is not followed", s.relation));
            }
            if q.window.is_some_and(|w| s.created_seq < w.lo || s.created_seq > w.hi) {
                return Some(format!("relation {}: outside the window", s.relation));
            }
            if q.provenance.kinds.as_ref().is_some_and(|k| !k.contains(&s.provenance)) {
                return Some(format!("relation {}: its provenance is not admitted", s.relation));
            }
        }
        let first = &hit.path[0];
        let walks = [first.from, first.to].into_iter().filter(|u| seeds.contains(u)).any(|start| {
            let mut here = start;
            for (k, s) in hit.path.iter().enumerate() {
                here = if out && s.from == here {
                    s.to
                } else if inn && s.to == here {
                    s.from
                } else {
                    return false;
                };
                if k + 1 < hit.path.len() && distance.get(&here).is_none_or(|d| *d as usize != k + 1) {
                    return false;
                }
            }
            here == hit.unit
        });
        if !walks {
            return Some(format!("unit {}: the path is not a walk from a seed through the result", hit.unit));
        }
    }
    None
}

fn mean(rows: &[&Row], pick: fn(&Row) -> Scores) -> Scores {
    let n = rows.len().max(1) as f64;
    let sum = rows.iter().fold([0.0, 0.0], |acc, r| {
        let s = pick(r);
        [acc[0] + s[0], acc[1] + s[1]]
    });
    [sum[0] / n, sum[1] / n]
}

fn verdict(met: bool) -> &'static str {
    if met { "met" } else { "not met" }
}

/// Checks the frozen files against `MANIFEST` before anything is measured.
fn verify_manifest() -> usize {
    let entries = kerno_testkit::assoc_eval::parse_manifest(MANIFEST).expect("the MANIFEST parses");
    assert_eq!(entries.len(), 2, "the MANIFEST lists exactly the two frozen files");
    for (digest, name) in &entries {
        let bytes = match name.as_str() {
            "corpus.txt" => CORPUS.as_bytes(),
            "queries.txt" => QUERIES.as_bytes(),
            other => panic!("the MANIFEST names {other}, which is not a frozen file"),
        };
        assert_eq!(&sha256::hex(&sha256::digest(bytes)), digest, "MANIFEST: {name} does not match the frozen file");
    }
    entries.len()
}

/// The B1 columns of the committed run, by query id, as printed there.
type Committed = BTreeMap<String, [String; 2]>;

fn committed_b1() -> Committed {
    // A query line has six fields and a query id (which contains `-`); the
    // header and the summary have no such id.
    E700.lines()
        .map(|line| line.split_whitespace().collect::<Vec<&str>>())
        .filter(|t| t.len() == 6 && t[0].contains('-'))
        .map(|t| (t[0].to_owned(), [t[4].to_owned(), t[5].to_owned()]))
        .collect()
}

#[test]
fn quality_over_the_frozen_set() {
    let manifest_entries = verify_manifest();
    println!("MANIFEST: {manifest_entries} frozen files match (corpus.txt, queries.txt)");

    let corpus = assoc_eval::parse_corpus(CORPUS).expect("corpus parses");
    let set = assoc_eval::parse_queries(QUERIES).expect("queries parse");
    assert_eq!(set.queries.len(), 64, "the frozen set has 64 queries");

    let (db, ids, _) = load_corpus_with_relations::<KernoDb>(&corpus);
    let index_of: HashMap<UnitId, u32> = ids.iter().enumerate().map(|(i, id)| (*id, i as u32)).collect();
    let snap = db.snapshot().expect("snapshot");

    let mut rows = Vec::new();
    let mut traces = (0usize, 0usize); // (verified, total) over every hit of every result
    let mut diag_checks = 0usize;
    let mut fallbacks = 0usize;
    for q in &set.queries {
        let query = corpus_query(&ids, q);
        let simple = snap.reference_assoc_query(&query).expect("reference path");
        let indexed = snap.indexed_assoc_query(&query).expect("indexed path");
        assert_eq!(indexed, simple, "{}: the indexed path differs from the reference path (I-32)", q.id);
        let planned = snap.assoc_query(&query).expect("planner");
        assert_eq!(planned.result, simple, "{}: the planner's answer differs from the reference", q.id);
        fallbacks += usize::from(planned.fallback.is_some());

        let ranked =
            snap.rank_assoc(&simple, &RankOptions { mode: RankMode::On, ..RankOptions::default() }).expect("ranking");
        let diagnostic = snap
            .rank_assoc(&simple, &RankOptions { mode: RankMode::Diagnostic, ..RankOptions::default() })
            .expect("diagnostic ranking");
        assert!(
            diagnostic.iter().map(|h| h.unit).eq(ranked.iter().map(|h| h.unit)),
            "{}: the diagnostic order differs from the ranked order",
            q.id
        );
        for h in &diagnostic {
            let c = h.contributions.expect("diagnostic mode gives the contributions");
            assert_eq!(Some(c.sum()), h.score, "{}: the contributions do not add up to the score (I-41)", q.id);
            diag_checks += 1;
        }

        // Ranking reorders these same hits, so their traces are the ranked
        // answer's traces: one check covers both orders.
        if let Some(why) = trace_failure(&snap, &query, &simple) {
            panic!("{}: a hit does not verify (I-40): {why}", q.id);
        }
        traces.0 += simple.hits.len();
        traces.1 += simple.hits.len();

        let simple_units: Vec<u32> = simple.hits.iter().take(CUT).map(|h| index_of[&h.unit]).collect();
        let ranked_units: Vec<u32> = ranked.iter().take(CUT).map(|h| index_of[&h.unit]).collect();
        let b1 = baseline_b1(&snap, &index_of, ids[q.seed as usize]);
        rows.push(Row {
            id: q.id.clone(),
            class: q.class,
            hits: simple.hits.len(),
            ending: simple.ending,
            b1: scores(&b1, q),
            simple: scores(&simple_units, q),
            ranked: scores(&ranked_units, q),
        });
    }

    // The B1 of this run is the B1 of the committed run.
    let committed = committed_b1();
    let mut reproduced = 0;
    for r in &rows {
        let want = committed.get(&r.id).unwrap_or_else(|| panic!("{} is not in the committed run", r.id));
        let got = [format!("{:.3}", r.b1[0]), format!("{:.3}", r.b1[1])];
        assert_eq!(&got, want, "{}: B1 differs from the committed run", r.id);
        reproduced += 1;
    }
    assert_eq!(reproduced, 64, "every query's B1 is in the committed run");

    println!("queries: {} translated and run on the reference, indexed and planner paths", rows.len());
    println!("I-32: indexed == reference == planner for all {} queries; planner fallbacks: {fallbacks}", rows.len());
    println!("I-41: contributions add up to the score for {diag_checks} hits");
    println!("I-40: {} of {} hits verify (trace, revision, walk, filters)", traces.0, traces.1);
    println!("B1: reproduced for all {reproduced} queries from the committed run (etapa7-e700-salida.txt)");
    let mut endings: BTreeMap<String, usize> = BTreeMap::new();
    for r in &rows {
        *endings.entry(format!("{:?}", r.ending)).or_default() += 1;
    }
    println!("endings of the simple order: {endings:?}");

    println!("per query (cut {CUT}; B1 = one-hop neighbours; simple = reference order; ranked = RankMode::On)");
    println!("query      class        hits  B1 R@10  B1 nDCG  simple R  simple nDCG  ranked R  ranked nDCG  ending");
    for r in &rows {
        println!(
            "{:<10} {:<11} {:>5} {:>8.3} {:>8.3} {:>9.3} {:>12.3} {:>9.3} {:>12.3}  {:?}",
            r.id,
            r.class.name(),
            r.hits,
            r.b1[0],
            r.b1[1],
            r.simple[0],
            r.simple[1],
            r.ranked[0],
            r.ranked[1],
            r.ending
        );
    }

    println!("summary (mean over the queries; the vector signal is not measured: the set has no vectors)");
    println!("scope        queries  B1 R@10  B1 nDCG  simple R  simple nDCG  ranked R  ranked nDCG");
    let mut summary: Vec<Scope> = vec![("overall", rows.iter().collect())];
    for (class, name) in CLASSES {
        summary.push((name, rows.iter().filter(|r| r.class == class).collect()));
    }
    let mut overall = [[0.0; 2]; 3];
    for (label, scope) in &summary {
        let b1 = mean(scope, |r| r.b1);
        let simple = mean(scope, |r| r.simple);
        let ranked = mean(scope, |r| r.ranked);
        println!(
            "{:<12} {:>7} {:>8.3} {:>8.3} {:>9.3} {:>12.3} {:>9.3} {:>12.3}",
            label,
            scope.len(),
            b1[0],
            b1[1],
            simple[0],
            simple[1],
            ranked[0],
            ranked[1]
        );
        if *label == "overall" {
            overall = [b1, simple, ranked];
        }
    }

    println!("thresholds of acta §6.2 (each with its number; a threshold not met is recorded, not tuned)");
    let [b1, simple, ranked] = overall;
    let t1 = ranked[0] >= RECALL_MIN;
    println!("T1  recall@10 with ranking, overall >= {RECALL_MIN:.2}: {:.3} -> {}", ranked[0], verdict(t1));
    let t2 = ranked[1] >= NDCG_MIN;
    println!("T2  nDCG@10 with ranking, overall >= {NDCG_MIN:.2}: {:.3} -> {}", ranked[1], verdict(t2));
    let bar_b1 = GAIN * b1[1];
    let t3 = ranked[1] >= bar_b1;
    println!(
        "T3  nDCG@10 with ranking >= {GAIN:.2} x B1 ({:.3} x {GAIN:.2} = {bar_b1:.3}): {:.3} -> {}",
        b1[1],
        ranked[1],
        verdict(t3)
    );
    let bar_simple = GAIN * simple[1];
    let t4 = ranked[1] >= bar_simple;
    println!(
        "T4  nDCG@10 with ranking >= {GAIN:.2} x simple order ({:.3} x {GAIN:.2} = {bar_simple:.3}): {:.3} -> {}",
        simple[1],
        ranked[1],
        verdict(t4)
    );
    for (label, scope) in summary.iter().skip(1) {
        let b = mean(scope, |r| r.b1)[1];
        let rk = mean(scope, |r| r.ranked)[1];
        println!("T5  nDCG@10 with ranking, {label} >= B1 ({rk:.3} vs {b:.3}): {}", verdict(rk >= b));
    }
    let traces_met = traces.0 == traces.1;
    println!(
        "C1  100 % of results with a verifiable trace (I-40; a correctness criterion, not a quality threshold): {}/{} -> {}",
        traces.0,
        traces.1,
        verdict(traces_met)
    );
}
