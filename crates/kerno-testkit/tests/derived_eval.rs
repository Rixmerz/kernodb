//! E8-01: the frozen evaluation set of the derived data and the vector search.
//!
//! * `regenerated_set_is_byte_identical` — the committed files are exactly
//!   what the seed produces.
//! * `manifest_matches_committed_files` — the SHA-256 of each committed file
//!   is the one in `MANIFEST`, and the manifest lists exactly those files.
//! * `committed_files_parse_back` — the committed text reads back to the same
//!   story, so the files are a faithful rendering and not a lossy one.
//! * `parser_refuses_a_repeated_unit` and `parser_refuses_a_vector_of_the_wrong_size`
//!   — the parsers check what they read.
//! * `quality_set_has_the_agreed_shape` — sizes, provenance classes, query
//!   classes, grades, and the vector dimension, read from the committed files.
//! * `expected_answers_are_reachable_within_the_bounds` — every relevant answer
//!   is reachable under its query's bounds; every distractor is reachable or,
//!   in a vector query, is a judged neighbour.
//! * `vector_top_ten_is_fully_judged` — the exact top ten of each vector query
//!   is judged, so no unjudged unit can sit in it (acta §6.1, convention of judgment).
//! * `identifiers_are_a_permutation_of_creation_order` — the identifiers are a
//!   permutation, and few of them fall on their creation position.
//! * `generation_is_deterministic` and `performance_scale_is_deterministic` —
//!   the same seed gives the same story, at both scales.
//! * `seed_is_not_the_stage_seven_seed` and `testkit_does_not_depend_on_kerno_units`.

use std::collections::{BTreeMap, BTreeSet};

use kerno_testkit::assoc_eval::{SEED as STAGE7_SEED, parse_manifest};
use kerno_testkit::derived_eval::{
    CUT, Class, Corpus, DIM, Direction, Kind, PER_CLASS, Query, QuerySet, Relation, SEED, Scale, Story, parse_corpus,
    parse_queries, parse_vectors, render_corpus, render_queries, render_vectors,
};
use kerno_testkit::sha256;

const CORPUS: &str = include_str!("../data/derived-eval/corpus.txt");
const VECTORS: &str = include_str!("../data/derived-eval/vectors.txt");
const QUERIES: &str = include_str!("../data/derived-eval/queries.txt");
const MANIFEST: &str = include_str!("../data/derived-eval/MANIFEST");
const CARGO_TOML: &str = include_str!("../Cargo.toml");

/// The committed set, read back from its files.
fn frozen() -> (Corpus, Vec<[i32; DIM]>, QuerySet) {
    let corpus = parse_corpus(CORPUS).expect("corpus.txt parses");
    let vectors = parse_vectors(VECTORS).expect("vectors.txt parses");
    let queries = parse_queries(QUERIES).expect("queries.txt parses");
    (corpus, vectors, queries)
}

fn quality() -> Story {
    Story::generate(Scale::Quality, SEED)
}

#[test]
fn regenerated_set_is_byte_identical() {
    let story = quality();
    assert_eq!(render_corpus(&story.corpus), CORPUS, "corpus.txt differs from the seed's output");
    assert_eq!(render_vectors(&story.vectors), VECTORS, "vectors.txt differs from the seed's output");
    assert_eq!(render_queries(&story.queries), QUERIES, "queries.txt differs from the seed's output");
}

#[test]
fn manifest_matches_committed_files() {
    let entries = parse_manifest(MANIFEST).expect("the manifest parses");
    let names: Vec<&str> = entries.iter().map(|(_, name)| name.as_str()).collect();
    assert_eq!(names, ["corpus.txt", "vectors.txt", "queries.txt"], "the manifest lists exactly the frozen files");
    let files: BTreeMap<&str, &[u8]> = BTreeMap::from([
        ("corpus.txt", CORPUS.as_bytes()),
        ("vectors.txt", VECTORS.as_bytes()),
        ("queries.txt", QUERIES.as_bytes()),
    ]);
    for (digest, name) in &entries {
        let actual = sha256::hex(&sha256::digest(files[name.as_str()]));
        assert_eq!(&actual, digest, "{name} does not match its digest in MANIFEST");
    }
}

#[test]
fn committed_files_parse_back() {
    let story = quality();
    let (corpus, vectors, queries) = frozen();
    assert_eq!(corpus, story.corpus);
    assert_eq!(vectors, story.vectors);
    assert_eq!(queries, story.queries);
}

#[test]
fn parser_refuses_a_repeated_unit() {
    // The second unit line gets the identifier of the first one.
    let first = CORPUS.lines().find(|l| l.starts_with("U ")).expect("a unit line");
    let id = first.split_whitespace().nth(1).expect("an identifier");
    let second = CORPUS.lines().filter(|l| l.starts_with("U ")).nth(1).expect("a second unit line");
    let second_id = second.split_whitespace().nth(1).expect("an identifier");
    let mutated = CORPUS.replacen(&format!("U {second_id} "), &format!("U {id} "), 1);
    // The reason is checked too: without the identifier check the same corpus
    // is refused later, for a link whose endpoint does not exist.
    let err = parse_corpus(&mutated).expect_err("a corpus with a repeated unit identifier was accepted");
    assert!(err.to_string().contains("repeated"), "refused for another reason: {err}");
}

#[test]
fn parser_refuses_a_vector_of_the_wrong_size() {
    let mutated = VECTORS.replacen("V 0 ", "V 0 1 ", 1);
    assert!(parse_vectors(&mutated).is_err(), "a vector with one component too many was accepted");
}

#[test]
fn quality_set_has_the_agreed_shape() {
    let (corpus, vectors, set) = frozen();
    assert_eq!(corpus.scale, Scale::Quality);
    assert_eq!(corpus.units.len(), 2000, "acta §6.1: about 2 000 units for quality");
    assert_eq!(corpus.changes.len(), 300);

    let classes: BTreeSet<&str> = corpus.relations.iter().map(|r| r.provenance.class()).collect();
    assert_eq!(classes, BTreeSet::from(["algorithmic", "declared", "imported", "inferred"]));
    let kinds: BTreeSet<Kind> = corpus.units.iter().map(|u| u.kind).collect();
    assert_eq!(kinds.len(), 5, "all five unit kinds are present");

    assert_eq!(vectors.len(), corpus.units.len(), "one vector per unit");
    assert!(vectors.iter().all(|v| v.len() == DIM));

    assert_eq!(set.queries.len(), 100, "acta §6.1: at least 100 queries");
    for class in [Class::VectorNeighbourhood, Class::Temporal, Class::Provenance, Class::Combined] {
        let count = set.queries.iter().filter(|q| q.class == class).count();
        assert_eq!(count, PER_CLASS, "{class:?} has {count} queries");
        assert!(count >= 20, "acta §6.1: at least 20 queries per class");
    }

    let grades: BTreeSet<u8> = set.queries.iter().flat_map(|q| q.expected.iter().map(|j| j.grade)).collect();
    assert_eq!(grades, BTreeSet::from([0, 1, 2, 3]), "all four grades are used");
    for q in &set.queries {
        assert!(q.expected.iter().any(|j| j.grade >= 1), "{} has no relevant answer", q.id);
        assert!(
            q.expected.iter().any(|j| j.grade == 0 && j.unit != q.seed),
            "{} names no distractor, so its bounds would decide it",
            q.id
        );
        assert!(q.expected.windows(2).all(|w| w[0].unit < w[1].unit), "{} is not sorted", q.id);
        assert!(q.expected.iter().all(|j| (j.unit as usize) < corpus.units.len()), "{} names a missing unit", q.id);
        assert!(!q.kinds.is_empty() && q.depth >= 1 && q.depth <= CUT as u8, "{} has bad bounds", q.id);
        assert_eq!(q.vector.is_some(), q.class == Class::VectorNeighbourhood, "{} vector presence", q.id);
    }
}

#[test]
fn vector_file_has_the_declared_dimension() {
    for line in VECTORS.lines().filter(|l| l.starts_with("V ")) {
        assert_eq!(line.split_whitespace().count(), DIM + 2, "a vector line has the wrong number of fields: {line}");
    }
}

/// Breadth-first walk over the parsed corpus under the bounds of `q`. It
/// follows every admitted link, false ones included, so it is a superset of
/// the answers a true path reaches.
fn reach(corpus: &Corpus, q: &Query) -> BTreeSet<u32> {
    let n = corpus.units.len();
    let mut out: Vec<Vec<&Relation>> = vec![Vec::new(); n];
    let mut inbound: Vec<Vec<&Relation>> = vec![Vec::new(); n];
    for r in &corpus.relations {
        out[r.from as usize].push(r);
        inbound[r.to as usize].push(r);
    }
    let admitted = |r: &Relation| {
        q.kinds.contains(&r.kind)
            && (q.provenance.is_empty() || q.provenance.contains(&r.provenance.class()))
            && match q.window {
                Some((lo, hi)) => (lo..=hi).contains(&r.step),
                None => true,
            }
    };
    let mut seen = BTreeSet::from([q.seed]);
    let mut frontier = vec![q.seed];
    for _ in 0..q.depth {
        let mut next = Vec::new();
        for &u in &frontier {
            let mut hops: Vec<u32> = Vec::new();
            if q.direction != Direction::In {
                hops.extend(out[u as usize].iter().filter(|r| admitted(r)).map(|r| r.to));
            }
            if q.direction != Direction::Out {
                hops.extend(inbound[u as usize].iter().filter(|r| admitted(r)).map(|r| r.from));
            }
            for v in hops {
                if seen.insert(v) {
                    next.push(v);
                }
            }
        }
        frontier = next;
    }
    seen
}

#[test]
fn expected_answers_are_reachable_within_the_bounds() {
    let (corpus, _, set) = frozen();
    for q in &set.queries {
        let reachable = reach(&corpus, q);
        for j in &q.expected {
            if j.unit == q.seed {
                assert_eq!(j.grade, 0, "{}: the seed is named with grade 0", q.id);
            } else if j.grade >= 1 {
                assert!(reachable.contains(&j.unit), "{}: relevant unit {} is not reachable", q.id, j.unit);
            } else if !reachable.contains(&j.unit) {
                assert!(
                    q.vector.is_some(),
                    "{}: distractor {} is unreachable and the query has no vector",
                    q.id,
                    j.unit
                );
            }
        }
    }
}

fn dot_cosine(a: &[i32; DIM], b: &[i32; DIM]) -> f64 {
    let dot: i64 = a.iter().zip(b).map(|(x, y)| i64::from(*x) * i64::from(*y)).sum();
    let norm = |v: &[i32; DIM]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    dot as f64 / (norm(a) * norm(b))
}

#[test]
fn vector_top_ten_is_fully_judged() {
    let (_, vectors, set) = frozen();
    for q in set.queries.iter().filter(|q| q.vector.is_some()) {
        let query = q.vector.expect("a vector query has a vector");
        let judged: BTreeSet<u32> = q.expected.iter().map(|j| j.unit).collect();
        let mut ranked: Vec<(f64, u32)> =
            vectors.iter().enumerate().map(|(id, v)| (dot_cosine(v, &query), id as u32)).collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, id) in ranked.iter().take(CUT) {
            assert!(judged.contains(id), "{}: vector neighbour {id} in the exact top ten is not judged", q.id);
        }
    }
}

#[test]
fn identifiers_are_a_permutation_of_creation_order() {
    let (corpus, _, _) = frozen();
    let mut ids: Vec<u32> = corpus.units.iter().map(|u| u.id).collect();
    let fixed = ids.iter().enumerate().filter(|(position, id)| *position as u32 == **id).count();
    assert!(fixed < 20, "{fixed} identifiers sit on their creation position; a permutation has about one");
    ids.sort_unstable();
    assert_eq!(ids, (0..2000).collect::<Vec<u32>>(), "the identifiers are 0..2000 each once");
}

#[test]
fn generation_is_deterministic() {
    let a = Story::generate(Scale::Quality, SEED);
    let b = Story::generate(Scale::Quality, SEED);
    assert_eq!(a, b);
    assert_eq!(render_corpus(&a.corpus), render_corpus(&b.corpus));
}

#[test]
fn performance_scale_is_deterministic() {
    let a = Story::generate(Scale::Performance, SEED);
    let b = Story::generate(Scale::Performance, SEED);
    assert_eq!(a.corpus.units.len(), 20_000, "acta §6.1: 20 000 units for the performance scale");
    assert_eq!(a.corpus.changes.len(), 3_000);
    assert_eq!(a.vectors.len(), 20_000);
    assert_eq!(a, b);
}

#[test]
fn thresholds_of_the_frozen_set_are_within_reach() {
    // acta §6.2, Q3: a perfect ranking gives each query the recall
    // min(1, CUT / relevant). Their mean is the highest recall@10 this set allows.
    let (_, _, set) = frozen();
    let reach: f64 = set
        .queries
        .iter()
        .map(|q| {
            let relevant = q.expected.iter().filter(|j| j.grade >= 1).count();
            (CUT as f64 / relevant as f64).min(1.0)
        })
        .sum::<f64>()
        / set.queries.len() as f64;
    assert!(reach >= 0.90, "Q3 asks 0.90 but a perfect ranking reaches only {reach:.4} on this set");
}

#[test]
fn seed_is_not_the_stage_seven_seed() {
    assert_ne!(SEED, STAGE7_SEED, "acta §6.1: the seed is distinct from the Stage 7 seed");
}

#[test]
fn testkit_does_not_depend_on_kerno_units() {
    assert!(!CARGO_TOML.contains("kerno-units"), "kerno-testkit must not depend on kerno-units");
}
