//! The second frozen evaluation set of the derived data and the vector search
//! (Stage 9, E9-19; acta §5, T-E10-V1 and T-E10-V2).
//!
//! It is built on the story of [`crate::derived_eval`] with a seed of its own,
//! and differs in one respect: every query carries a query vector, in each of
//! the four classes, so a vector-only arm can be scored on all of them. The
//! graded answers still come from the story's true links; the ten units
//! nearest to each added query vector that the walk did not judge are judged
//! as grade 0, so the exact top ten of every query is fully judged.
//!
//! The set is frozen without measuring anything with it, except the
//! non-saturation check [`arm_without_signal`]: the arm that ranks by the
//! query vector alone must score at most [`SATURATION_LIMIT`] in nDCG@10, or
//! the seed is replaced before the files are written.

use std::collections::BTreeMap;

use crate::assoc_eval;
use crate::assoc_metrics::ndcg_at;
use crate::derived_eval::{CUT, Class, DIM, Judgment, QuerySet, Scale, Story, cosine};
use crate::rng::Rng;

/// The seed of this set. It is neither the Stage 7 seed nor the Stage 8 one.
pub const SEED: u64 = 20_261_203;
const QUERY_VECTOR_STREAM: u64 = 0xD9E1_0000_0000_0019;
/// Noise around the seed unit's vector, per component, for the added query vectors.
const QUERY_NOISE: i64 = 150;
/// Highest nDCG@10 of the arm without signal, over all queries (T-E10-V2).
pub const SATURATION_LIMIT: f64 = 0.96;

/// Generates the set: the Stage 8 story at quality scale with [`SEED`], and a
/// query vector in every query.
pub fn generate() -> Story {
    with_vectors(Story::generate(Scale::Quality, SEED))
}

fn with_vectors(mut story: Story) -> Story {
    let mut rng = Rng::new(story.queries.seed ^ QUERY_VECTOR_STREAM);
    for q in &mut story.queries.queries {
        if q.vector.is_some() {
            continue;
        }
        let centre = story.vectors[q.seed as usize];
        let mut v = [0i32; DIM];
        for (x, c) in v.iter_mut().zip(&centre) {
            let offset = rng.range(0, (2 * QUERY_NOISE) as u64) as i64 - QUERY_NOISE;
            *x = (i64::from(*c) + offset) as i32;
        }
        q.vector = Some(v);
        let mut judged: BTreeMap<u32, u8> = q.expected.iter().map(|j| (j.unit, j.grade)).collect();
        let mut added = 0;
        for id in nearest(&story.vectors, &v) {
            if added == CUT {
                break;
            }
            if let std::collections::btree_map::Entry::Vacant(slot) = judged.entry(id) {
                slot.insert(0);
                added += 1;
            }
        }
        q.expected = judged.into_iter().map(|(unit, grade)| Judgment { unit, grade }).collect();
    }
    story
}

/// Unit identifiers by decreasing cosine to `query`; ties by identifier.
pub fn nearest(vectors: &[[i32; DIM]], query: &[i32; DIM]) -> Vec<u32> {
    let mut ranked: Vec<(f64, u32)> = vectors.iter().enumerate().map(|(id, v)| (cosine(v, query), id as u32)).collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|(_, id)| id).collect()
}

/// nDCG@10 of the arm without signal: the exact search by the query vector
/// alone. Returns the mean over all queries and the mean of each class.
pub fn arm_without_signal(vectors: &[[i32; DIM]], set: &QuerySet) -> (f64, Vec<(Class, f64)>) {
    let mut all = Vec::new();
    let mut by_class: BTreeMap<&'static str, (Class, Vec<f64>)> = BTreeMap::new();
    for q in &set.queries {
        let vector = q.vector.expect("every query of this set has a vector");
        let results: Vec<u32> = nearest(vectors, &vector).into_iter().take(CUT).collect();
        let expected: Vec<assoc_eval::Judgment> =
            q.expected.iter().map(|j| assoc_eval::Judgment { index: j.unit, grade: j.grade }).collect();
        let score = ndcg_at(&results, &expected, CUT);
        all.push(score);
        by_class.entry(q.class.name()).or_insert((q.class, Vec::new())).1.push(score);
    }
    let mean = |xs: &[f64]| xs.iter().sum::<f64>() / xs.len() as f64;
    (mean(&all), by_class.into_values().map(|(c, xs)| (c, mean(&xs))).collect())
}
