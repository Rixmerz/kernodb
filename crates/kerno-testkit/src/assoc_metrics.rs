//! Retrieval metrics over graded relevance (acta §6.2, E7-00).
//!
//! A result list is the unit indexes a query returned, best first. The
//! expected answers carry grades 0 to 3 ([`Judgment`]). A unit is relevant
//! when its grade is at least 1. Conventions shared by all three metrics:
//!
//! * a unit repeated in the result list counts once, at its first position;
//! * a result list shorter than `k` leaves the missing positions empty, it is
//!   not shortened to fit `k`;
//! * when `k` is 0, or the expected answers hold no relevant unit, the metric
//!   is 0 (the set of [`crate::assoc_eval`] never has such a query).
//!
//! Formulas, with `R` the relevant units and `r_i` the grade at position `i`
//! (1-based), `k` the cut:
//!
//! * `recall@k = |{relevant units in the first k}| / |R|`
//! * `precision@k = |{relevant units in the first k}| / k`
//! * `gain(g) = 2^g - 1`, so the gains are 0, 1, 3 and 7 for grades 0 to 3
//! * `DCG@k = sum over i = 1..k of gain(r_i) / log2(i + 1)`
//! * `IDCG@k` is the same sum over the expected grades sorted from highest to
//!   lowest, and `nDCG@k = DCG@k / IDCG@k`

use crate::assoc_eval::{Judgment, MAX_GRADE};

/// The gain of a grade: `2^grade - 1`. Panics above [`MAX_GRADE`], since a
/// grade outside the scale is a bug in the expected answers, not a score.
pub fn gain(grade: u8) -> f64 {
    assert!(grade <= MAX_GRADE, "grade {grade} is above the scale 0..={MAX_GRADE}");
    f64::from((1u32 << grade) - 1)
}

/// The grade of `index` in `expected`, 0 when it is not listed.
fn grade_of(expected: &[Judgment], index: u32) -> u8 {
    expected.iter().find(|j| j.index == index).map_or(0, |j| j.grade)
}

/// The distinct units of the first `k` results with their grade, in
/// position order. Repeats are dropped at their later positions.
fn first_k<'a>(results: &'a [u32], expected: &'a [Judgment], k: usize) -> impl Iterator<Item = (usize, u8)> + 'a {
    let mut seen = Vec::new();
    results.iter().take(k).enumerate().filter_map(move |(i, &unit)| {
        if seen.contains(&unit) {
            return None;
        }
        seen.push(unit);
        Some((i + 1, grade_of(expected, unit)))
    })
}

fn relevant_total(expected: &[Judgment]) -> usize {
    expected.iter().filter(|j| j.grade >= 1).count()
}

/// Share of the relevant units that appear in the first `k` results.
pub fn recall_at(results: &[u32], expected: &[Judgment], k: usize) -> f64 {
    let total = relevant_total(expected);
    if k == 0 || total == 0 {
        return 0.0;
    }
    let found = first_k(results, expected, k).filter(|&(_, g)| g >= 1).count();
    found as f64 / total as f64
}

/// Share of the first `k` positions that hold a relevant unit.
pub fn precision_at(results: &[u32], expected: &[Judgment], k: usize) -> f64 {
    if k == 0 || relevant_total(expected) == 0 {
        return 0.0;
    }
    let found = first_k(results, expected, k).filter(|&(_, g)| g >= 1).count();
    found as f64 / k as f64
}

/// Discounted cumulative gain of the first `k` results.
pub fn dcg_at(results: &[u32], expected: &[Judgment], k: usize) -> f64 {
    first_k(results, expected, k).map(|(rank, g)| gain(g) / (rank as f64 + 1.0).log2()).sum()
}

/// Normalized discounted cumulative gain of the first `k` results, in `0..=1`.
pub fn ndcg_at(results: &[u32], expected: &[Judgment], k: usize) -> f64 {
    if k == 0 {
        return 0.0;
    }
    let mut grades: Vec<u8> = expected.iter().map(|j| j.grade).collect();
    grades.sort_unstable_by_key(|&g| std::cmp::Reverse(g));
    let ideal: f64 = grades.iter().take(k).enumerate().map(|(i, &g)| gain(g) / (i as f64 + 2.0).log2()).sum();
    if ideal == 0.0 {
        return 0.0;
    }
    dcg_at(results, expected, k) / ideal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(index: u32, grade: u8) -> Judgment {
        Judgment { index, grade }
    }

    /// Relevant units 1 (grade 3), 2 (grade 2), 3 (grade 1); 4 is a distractor.
    fn sample() -> Vec<Judgment> {
        vec![j(1, 3), j(2, 2), j(3, 1), j(4, 0)]
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-12, "{a} != {b}");
    }

    #[test]
    fn gains_follow_two_to_the_grade_minus_one() {
        assert_eq!([gain(0), gain(1), gain(2), gain(3)], [0.0, 1.0, 3.0, 7.0]);
    }

    #[test]
    #[should_panic(expected = "above the scale")]
    fn a_grade_outside_the_scale_is_refused() {
        gain(4);
    }

    /// Results [1, 9, 2] at k = 3. Relevant in the top 3: units 1 and 2 of 3.
    /// DCG = 7/log2(2) + 0 + 3/log2(4) = 8.5. IDCG = 7 + 3/log2(3) + 1/log2(4).
    #[test]
    fn hand_computed_example() {
        let results = [1, 9, 2];
        close(recall_at(&results, &sample(), 3), 2.0 / 3.0);
        close(precision_at(&results, &sample(), 3), 2.0 / 3.0);
        close(dcg_at(&results, &sample(), 3), 8.5);
        let ideal = 7.0 + 3.0 / 3f64.log2() + 1.0 / 4f64.log2();
        close(ndcg_at(&results, &sample(), 3), 8.5 / ideal);
    }

    #[test]
    fn empty_results_score_zero() {
        let none: [u32; 0] = [];
        close(recall_at(&none, &sample(), 10), 0.0);
        close(precision_at(&none, &sample(), 10), 0.0);
        close(ndcg_at(&none, &sample(), 10), 0.0);
    }

    #[test]
    fn no_distractor_is_relevant_and_zero_cut_scores_zero() {
        close(recall_at(&[4], &sample(), 1), 0.0);
        close(precision_at(&[1], &sample(), 0), 0.0);
        close(ndcg_at(&[1], &sample(), 0), 0.0);
    }

    #[test]
    fn expected_without_relevant_units_scores_zero() {
        let only_distractors = [j(4, 0)];
        close(recall_at(&[4], &only_distractors, 5), 0.0);
        close(ndcg_at(&[4], &only_distractors, 5), 0.0);
    }

    /// Ties: equal grades in any order give the same score, and a perfect
    /// order of equal grades scores 1.
    #[test]
    fn ties_do_not_depend_on_their_order() {
        let equal = vec![j(1, 1), j(2, 1), j(3, 1), j(4, 0)];
        close(ndcg_at(&[1, 2, 3], &equal, 3), 1.0);
        close(ndcg_at(&[3, 2, 1], &equal, 3), 1.0);
        close(ndcg_at(&[3, 1, 2], &equal, 3), 1.0);
        close(recall_at(&[4, 1, 2], &equal, 3), 2.0 / 3.0);
        // The distractor first: the relevant units move to positions 2 and 3.
        let expected = (1.0 / 3f64.log2() + 0.5) / (1.0 + 1.0 / 3f64.log2() + 0.5);
        close(ndcg_at(&[4, 1, 2, 3], &equal, 3), expected);
    }

    /// k larger than the list: the list is not shortened to fit k.
    /// Recall = 1/2, precision = 1/10, DCG = 7, IDCG = 7 + 1/log2(3) with k = 10.
    #[test]
    fn k_larger_than_the_result_list() {
        let expected = vec![j(1, 3), j(2, 1)];
        close(recall_at(&[1], &expected, 10), 0.5);
        close(precision_at(&[1], &expected, 10), 0.1);
        close(dcg_at(&[1], &expected, 10), 7.0);
        close(ndcg_at(&[1], &expected, 10), 7.0 / (7.0 + 1.0 / 3f64.log2()));
    }

    /// A repeated relevant unit counts once: results [1, 1, 2] give recall 1,
    /// precision 2/3 and DCG 7 + 3/log2(4).
    #[test]
    fn repeats_count_once() {
        let expected = vec![j(1, 3), j(2, 2)];
        close(recall_at(&[1, 1, 2], &expected, 3), 1.0);
        close(precision_at(&[1, 1, 2], &expected, 3), 2.0 / 3.0);
        close(dcg_at(&[1, 1, 2], &expected, 3), 8.5);
    }
}
