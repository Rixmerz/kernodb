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
//! * `performance_scale_is_deterministic` — the 20 000-unit scale is generated
//!   from the same seed and has its size.

use std::collections::BTreeMap;

use kerno_testkit::assoc_eval::{
    Class, Constraints, Corpus, Direction, Judgment, Kind, QuerySet, Relation, SEED, Scale, Story, parse_corpus,
    parse_manifest, parse_queries, render_corpus, render_queries,
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

#[test]
fn performance_scale_is_deterministic() {
    let first = Story::generate(Scale::Performance, SEED);
    let second = Story::generate(Scale::Performance, SEED);
    assert_eq!(first, second, "the same seed gives the same story");
    assert_eq!(first.corpus.units.len(), 20_000, "about 20 000 units");
    assert_eq!(first.corpus.changes.len(), 3000, "three change sets per session");
    assert_eq!(first.queries.queries.len(), 64, "the performance scale has the same query families");
}
