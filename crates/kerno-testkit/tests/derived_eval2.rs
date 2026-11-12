//! E9-19: the second frozen evaluation set (acta §5, T-E10-V1, T-E10-V2, T-E10-R).
//!
//! * `regenerated_set_is_byte_identical` — the committed files are what the seed writes.
//! * `manifest_matches_committed_files` — each file's SHA-256 is the one in `MANIFEST`.
//! * `committed_files_parse_back` — the files are a faithful rendering of the story.
//! * `every_class_has_at_least_25_vector_queries` — T-E10-V1.
//! * `arm_without_signal_is_not_saturated` — T-E10-V2, on the committed files.
//! * `vector_top_ten_is_fully_judged` — the exact top ten of every query is judged.
//! * `seeds_are_distinct_from_earlier_sets` — the seed is the set's own.
//! * `testkit_does_not_depend_on_kerno_units`.
//!
//! No test here opens a database, so there is no engine to vary.

use std::collections::{BTreeMap, BTreeSet};

use kerno_testkit::assoc_eval::{SEED as STAGE7_SEED, parse_manifest};
use kerno_testkit::derived_eval::{
    CUT, Class, DIM, PER_CLASS, QuerySet, SEED as STAGE8_SEED, parse_corpus, parse_queries, parse_vectors,
    render_corpus, render_queries, render_vectors,
};
use kerno_testkit::derived_eval2::{SATURATION_LIMIT, SEED, arm_without_signal, generate, nearest};
use kerno_testkit::sha256;

const CORPUS: &str = include_str!("../data/derived-eval-2/corpus.txt");
const VECTORS: &str = include_str!("../data/derived-eval-2/vectors.txt");
const QUERIES: &str = include_str!("../data/derived-eval-2/queries.txt");
const MANIFEST: &str = include_str!("../data/derived-eval-2/MANIFEST");
const CARGO_TOML: &str = include_str!("../Cargo.toml");

fn frozen() -> (Vec<[i32; DIM]>, QuerySet) {
    (parse_vectors(VECTORS).expect("vectors.txt parses"), parse_queries(QUERIES).expect("queries.txt parses"))
}

#[test]
fn regenerated_set_is_byte_identical() {
    let story = generate();
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
    let story = generate();
    assert_eq!(parse_corpus(CORPUS).expect("corpus.txt parses"), story.corpus);
    let (vectors, queries) = frozen();
    assert_eq!(vectors, story.vectors);
    assert_eq!(queries, story.queries);
}

#[test]
fn every_class_has_at_least_25_vector_queries() {
    let (_, set) = frozen();
    for class in [Class::VectorNeighbourhood, Class::Temporal, Class::Provenance, Class::Combined] {
        let with_vector = set.queries.iter().filter(|q| q.class == class && q.vector.is_some()).count();
        assert!(with_vector >= 25, "{class:?} has {with_vector} queries with a query vector");
        assert_eq!(with_vector, PER_CLASS, "{class:?}");
    }
    assert!(set.queries.iter().all(|q| q.vector.is_some()), "a query has no vector");
    for q in &set.queries {
        assert!(q.expected.iter().any(|j| j.grade >= 1), "{} has no relevant answer", q.id);
    }
}

#[test]
fn arm_without_signal_is_not_saturated() {
    let (vectors, set) = frozen();
    let (overall, by_class) = arm_without_signal(&vectors, &set);
    assert!(overall <= SATURATION_LIMIT, "the arm without signal scores {overall:.4}, above {SATURATION_LIMIT}");
    assert_eq!(by_class.len(), 4, "a class is missing from the check");
    assert!(overall > 0.0, "a zero score means the check ranked nothing");
}

#[test]
fn vector_top_ten_is_fully_judged() {
    let (vectors, set) = frozen();
    for q in &set.queries {
        let judged: BTreeSet<u32> = q.expected.iter().map(|j| j.unit).collect();
        assert!(q.expected.windows(2).all(|w| w[0].unit < w[1].unit), "{} is not sorted", q.id);
        for id in nearest(&vectors, &q.vector.expect("a vector")).into_iter().take(CUT) {
            assert!(judged.contains(&id), "{}: neighbour {id} of the exact top ten is not judged", q.id);
        }
    }
}

#[test]
fn seeds_are_distinct_from_earlier_sets() {
    assert_ne!(SEED, STAGE7_SEED);
    assert_ne!(SEED, STAGE8_SEED);
    assert_eq!(parse_corpus(CORPUS).expect("corpus.txt parses").seed, SEED, "the files carry the set's seed");
}

#[test]
fn testkit_does_not_depend_on_kerno_units() {
    assert!(!CARGO_TOML.contains("kerno-units"), "kerno-testkit must not depend on kerno-units");
}
