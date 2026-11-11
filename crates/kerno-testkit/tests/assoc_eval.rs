//! E7-00: the frozen evaluation set of the associative memory.
//!
//! * `regenerated_set_is_byte_identical` — the committed files are exactly
//!   what the seed produces.
//! * `manifest_matches_committed_files` — the SHA-256 of each committed file
//!   is the one in `MANIFEST`.
//! * `committed_files_parse_back` — the committed text reads back to the same
//!   story, so the files are a faithful rendering and not a lossy one.
//! * `parser_refuses_a_unit_out_of_order` — the parser checks the stable
//!   local indexes it reads.
//! * `quality_set_has_the_agreed_shape` — size, provenance kinds, query classes
//!   and relevance sizes, read from the committed files.
//! * `expected_answers_are_reachable_within_the_bounds` — every named answer of
//!   the committed set, relevant or distractor, is reachable under its query's
//!   bounds. The breadth-first walk here is a check of the frozen answers; the
//!   answers were not produced by it.
//! * `every_query_has_a_reachable_distractor` — the bounds alone do not decide
//!   any query: each one names a distractor that the bounds reach.
//! * `confidence_alone_does_not_separate_related_links` — thresholding the
//!   confidence of the inferred `related` links does not tell the true from the
//!   false ones (grade 0 against graded).
//! * `same_source_truth_is_not_readable_from_one_field` — the same for the
//!   `same_source` links and the index offset, index distance and creation order.
//! * `performance_scale_is_deterministic` — the 20 000-unit scale is generated
//!   from the same seed and has its size.

use std::collections::BTreeMap;

use kerno_testkit::assoc_eval::{
    Class, Constraints, Corpus, Direction, Judgment, Kind, Provenance, QuerySet, Relation, SEED, Scale, Story,
    parse_corpus, parse_manifest, parse_queries, render_corpus, render_queries,
};
use kerno_testkit::sha256;

const CORPUS: &str = include_str!("../data/assoc-eval/corpus.txt");
const QUERIES: &str = include_str!("../data/assoc-eval/queries.txt");
const MANIFEST: &str = include_str!("../data/assoc-eval/MANIFEST");

/// The committed set, read back from its files: the tests below check what is frozen.
fn frozen() -> (Corpus, QuerySet) {
    let corpus = parse_corpus(CORPUS).expect("corpus.txt parses");
    let queries = parse_queries(QUERIES).expect("queries.txt parses");
    (corpus, queries)
}

fn quality() -> Story {
    Story::generate(Scale::Quality, SEED)
}

#[test]
fn regenerated_set_is_byte_identical() {
    let story = quality();
    assert_eq!(render_corpus(&story.corpus), CORPUS, "corpus.txt differs from the seed's output");
    assert_eq!(render_queries(&story.queries), QUERIES, "queries.txt differs from the seed's output");
}

#[test]
fn manifest_matches_committed_files() {
    let entries = parse_manifest(MANIFEST).expect("the manifest parses");
    let files: BTreeMap<&str, &[u8]> =
        [("corpus.txt", CORPUS.as_bytes()), ("queries.txt", QUERIES.as_bytes())].into_iter().collect();
    assert_eq!(entries.len(), files.len(), "the manifest lists exactly the frozen files");
    for (digest, name) in &entries {
        let bytes = files.get(name.as_str()).unwrap_or_else(|| panic!("{name} is not a frozen file"));
        assert_eq!(&sha256::hex(&sha256::digest(bytes)), digest, "digest of {name}");
    }
}

#[test]
fn committed_files_parse_back() {
    let story = quality();
    assert_eq!(parse_corpus(CORPUS).expect("corpus parses"), story.corpus);
    assert_eq!(parse_queries(QUERIES).expect("queries parse"), story.queries);
}

#[test]
fn parser_refuses_a_unit_out_of_order() {
    // Swapping two unit records breaks the stable local index.
    let mut lines: Vec<&str> = CORPUS.lines().collect();
    let first = lines.iter().position(|l| l.starts_with("U 0 ")).expect("unit 0");
    let second = lines.iter().position(|l| l.starts_with("U 1 ")).expect("unit 1");
    lines.swap(first, second);
    let broken = lines.join("\n");
    assert!(parse_corpus(&broken).is_err(), "a reordered unit must not parse");
}

#[test]
fn quality_set_has_the_agreed_shape() {
    let (corpus, set) = frozen();
    assert_eq!(corpus.units.len(), 2000, "about 2 000 units");
    for kind in Kind::ALL {
        assert!(corpus.relations.iter().any(|r| r.provenance.kind() == kind), "{kind:?} relations are present");
    }
    let distractors = corpus.relations.iter().filter(|r| r.relation_type == "mentions").count();
    assert!(distractors > 0, "cross-topic mentions are present as distractor relations");

    let queries = &set.queries;
    assert!(queries.len() >= 60, "at least 60 queries, found {}", queries.len());
    for (name, class) in [
        ("multi_hop", Class::MultiHop),
        ("temporal", Class::Temporal),
        ("provenance", Class::Provenance),
        ("combined", Class::Combined),
    ] {
        let count = queries.iter().filter(|q| q.class == class).count();
        assert!(count >= 12, "{name} has {count} queries; at least 12 are needed");
    }
    for q in queries {
        let relevant = q.expected.iter().filter(|j| j.grade >= 1).count();
        assert!((1..=10).contains(&relevant), "{}: {relevant} relevant answers; need 1..=10", q.id);
        assert!(q.expected.iter().any(|j| j.grade == 0), "{}: no named distractor", q.id);
        assert!(q.expected.windows(2).all(|w| w[0].index < w[1].index), "{}: sorted, no duplicates", q.id);
        assert!(q.expected.iter().all(|j| j.grade <= 3), "{}: grades 0..=3", q.id);
        assert!(q.expected.iter().all(|j| j.index != q.seed), "{}: the seed is not its own answer", q.id);
        assert!(!q.constraints.relation_types.is_empty(), "{}: a query names its relation types", q.id);
    }
}

/// Whether a relation of the corpus passes the filters of `cons`, written
/// again here so that the check does not share the generator's helper.
fn passes(corpus: &Corpus, cons: &Constraints, rel: &Relation) -> bool {
    let change = &corpus.changes[rel.step as usize - 1];
    cons.relation_types.contains(&rel.relation_type)
        && (cons.kinds.is_empty() || cons.kinds.contains(&rel.provenance.kind()))
        && cons.window.is_none_or(|(lo, hi)| (lo..=hi).contains(&rel.step))
        && cons.actor.as_ref().is_none_or(|a| *a == change.actor)
        && cons.source.as_ref().is_none_or(|s| *s == change.source)
}

/// Breadth-first walk from `seed` under the bounds of `cons`: the minimal
/// depth of every unit reached within `cons.max_depth` steps.
fn reachable(corpus: &Corpus, cons: &Constraints, seed: u32) -> BTreeMap<u32, u8> {
    let mut adjacent: Vec<Vec<(usize, u32, bool)>> = vec![Vec::new(); corpus.units.len()];
    for r in &corpus.relations {
        adjacent[r.from as usize].push((r.index as usize, r.to, true));
        adjacent[r.to as usize].push((r.index as usize, r.from, false));
    }
    let mut depth = BTreeMap::from([(seed, 0u8)]);
    let mut frontier = vec![seed];
    for level in 0..cons.max_depth {
        let mut next = Vec::new();
        for unit in frontier {
            for &(rel, other, from_here) in &adjacent[unit as usize] {
                let follows = match cons.direction {
                    Direction::Both => true,
                    Direction::Out => from_here,
                    Direction::In => !from_here,
                };
                if !follows || depth.contains_key(&other) || !passes(corpus, cons, &corpus.relations[rel]) {
                    continue;
                }
                depth.insert(other, level + 1);
                next.push(other);
            }
        }
        frontier = next;
    }
    depth
}

#[test]
fn expected_answers_are_reachable_within_the_bounds() {
    let (corpus, set) = frozen();
    for q in &set.queries {
        let reach = reachable(&corpus, &q.constraints, q.seed);
        for j in &q.expected {
            assert!(
                reach.contains_key(&j.index),
                "{}: unit {} (grade {}) is not reachable within the bounds",
                q.id,
                j.index,
                j.grade
            );
        }
    }
}

/// Expected answers are not "everything reachable": each query has a named
/// distractor that the bounds do reach, so the bounds alone do not decide it.
#[test]
fn every_query_has_a_reachable_distractor() {
    let (corpus, set) = frozen();
    for q in &set.queries {
        let reach = reachable(&corpus, &q.constraints, q.seed);
        let reached_distractor = q.expected.iter().any(|j: &Judgment| j.grade == 0 && reach.contains_key(&j.index));
        assert!(reached_distractor, "{}: no distractor is reachable within the bounds", q.id);
    }
}

/// Whether some threshold on `value` splits `(value, is_true)` samples with
/// no error, in either direction; and the best accuracy any threshold reaches.
fn threshold_split(samples: &[(i64, bool)]) -> (bool, f64) {
    let mut sorted = samples.to_vec();
    sorted.sort();
    let total_true = sorted.iter().filter(|s| s.1).count();
    let n = sorted.len();
    let (mut true_below, mut false_below) = (0, 0);
    let (mut perfect, mut best) = (false, 0usize);
    for i in 0..n {
        if sorted[i].1 {
            true_below += 1;
        } else {
            false_below += 1;
        }
        if i + 1 < n && sorted[i].0 == sorted[i + 1].0 {
            continue;
        }
        // Everything at or below the threshold is called false, the rest true.
        let right = false_below + (total_true - true_below);
        let correct = right.max(n - right);
        perfect |= correct == n;
        best = best.max(correct);
    }
    (perfect, best as f64 / n as f64)
}

/// Whether the story says a `related` link holds: its target is two
/// revisions back from its source. Read off the corpus, not off the generator.
fn related_is_true(corpus: &Corpus) -> impl Fn(&Relation) -> bool {
    let revises: BTreeMap<u32, u32> =
        corpus.relations.iter().filter(|r| r.relation_type == "revises").map(|r| (r.from, r.to)).collect();
    move |r| revises.get(&r.from).and_then(|d| revises.get(d)) == Some(&r.to)
}

#[test]
fn confidence_alone_does_not_separate_related_links() {
    let (corpus, set) = frozen();
    let is_true = related_is_true(&corpus);
    let samples: Vec<(i64, bool)> = corpus
        .relations
        .iter()
        .filter(|r| r.relation_type == "related")
        .map(|r| match r.provenance {
            Provenance::Inferred { confidence_permille, .. } => (i64::from(confidence_permille), is_true(r)),
            _ => panic!("relation {} is related but not inferred", r.index),
        })
        .collect();
    assert!(samples.iter().any(|s| s.1) && samples.iter().any(|s| !s.1), "both true and false links exist");

    // The reading of truth above agrees with the grades of the PV-RE queries.
    for q in set.queries.iter().filter(|q| q.id.starts_with("PV-RE")) {
        for j in &q.expected {
            let link = corpus
                .relations
                .iter()
                .find(|r| {
                    r.relation_type == "related"
                        && ((r.from == q.seed && r.to == j.index) || (r.to == q.seed && r.from == j.index))
                })
                .expect("every PV-RE answer is a related neighbour of the seed");
            assert_eq!(is_true(link), j.grade >= 1, "{}: unit {}: grade and truth agree", q.id, j.index);
        }
    }

    let (perfect, best) = threshold_split(&samples);
    assert!(!perfect, "a confidence threshold separates every true related link from every false one");
    assert!(best < 0.85, "the best confidence threshold gets {best:.3} of the related links right; need < 0.85");
}

/// A numeric field of a relation, read off the corpus.
type Field = Box<dyn Fn(&Relation) -> i64>;

#[test]
fn same_source_truth_is_not_readable_from_one_field() {
    let (corpus, _) = frozen();
    let cited: BTreeMap<u32, u32> =
        corpus.relations.iter().filter(|r| r.relation_type == "cites").map(|r| (r.from, r.to)).collect();
    let links: Vec<&Relation> = corpus.relations.iter().filter(|r| r.relation_type == "same_source").collect();
    let mut first_of_step: BTreeMap<u64, i64> = BTreeMap::new();
    for r in &corpus.relations {
        first_of_step.entry(r.step).or_insert(i64::from(r.index));
    }
    let fields: [(&str, Field); 6] = [
        ("offset of the end", Box::new(|r| i64::from(r.to % 20))),
        ("offset of the start", Box::new(|r| i64::from(r.from % 20))),
        ("index distance", Box::new(|r| (i64::from(r.to) - i64::from(r.from)).abs())),
        ("signed index distance", Box::new(|r| i64::from(r.to) - i64::from(r.from))),
        ("parity of the index", Box::new(|r| i64::from(r.to % 2))),
        ("creation order in the change set", Box::new(move |r| i64::from(r.index) - first_of_step[&r.step])),
    ];
    for (name, field) in &fields {
        let samples: Vec<(i64, bool)> = links.iter().map(|r| (field(r), cited[&r.from] == cited[&r.to])).collect();
        let (perfect, best) = threshold_split(&samples);
        assert!(!perfect, "{name} separates the true same_source links from the false ones");
        assert!(best < 0.9, "{name}: a threshold gets {best:.3} of the same_source links right; need < 0.9");
    }
}

#[test]
fn performance_scale_is_deterministic() {
    let first = Story::generate(Scale::Performance, SEED);
    let second = Story::generate(Scale::Performance, SEED);
    assert_eq!(first, second, "the same seed gives the same story");
    assert_eq!(first.corpus.units.len(), 20_000, "about 20 000 units");
    assert_eq!(first.corpus.changes.len(), 3000, "three change sets per session");
    assert_eq!(first.queries.queries.len(), 64, "the performance scale has the same query families");
}
