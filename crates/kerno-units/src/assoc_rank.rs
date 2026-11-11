//! E7-04: the ranking of associative results, its diagnostic mode and the
//! exact vector signal (etapa7-acta.md §3, §4 I-41; ADR-0011;
//! `docs/architecture/etapa7-ranking.md`).
//!
//! [`UnitSnapshot::rank_assoc`] takes the [`AssocResult`] of an associative
//! query and the snapshot it ran on. It has three modes ([`RankMode`]):
//!
//! * `Off` (the default): the hits come back in the order of the result,
//!   which is the reference path's "distance, then unit id". The snapshot is
//!   not read and no score exists.
//! * `On`: every hit gets an integer score and the hits are ordered by
//!   (score descending, unit id ascending).
//! * `Diagnostic`: the same order and score, plus the contribution of each
//!   signal. The five contributions add up to the score exactly (I-41): the
//!   score *is* their sum ([`Contributions::sum`]).
//!
//! Whether ranking becomes the default is decided by the measurement of E7-05,
//! not here.
//!
//! # Signals
//!
//! All arithmetic is integer except the vector signal, which is made stable
//! as described below. A bigger score ranks first.
//!
//! | signal | what it reads | contribution |
//! |---|---|---|
//! | hops | `hit.distance` | `hops_weight * (MAX_ASSOC_DEPTH + 1 - distance)` |
//! | relation type | the types of the path's steps | `type_weight * preferred_steps / path_len` |
//! | recency | the newest `commit_seq` the hit rests on | `recency_weight * H / (H + age)` |
//! | provenance | the weakest class on the path | `provenance_weight * trust(class)` |
//! | similarity | the exact similarity to the caller's vector | `similarity_weight * clamp(sim, -1, 1)` |
//!
//! * **Relation type.** Only the caller knows which types matter, so the
//!   signal is *relative to a set the caller names* ([`RankOptions::preferred_types`]):
//!   the share of the path's steps whose type is in the set (integer division,
//!   rounded toward zero). An empty set gives 0 to every hit.
//! * **Recency** is the age, in commits, of the newest of the hit's head
//!   revision (`Unit::updated_seq`, read from the snapshot) and the relation
//!   that reached it (the last step's `created_seq`), against the snapshot's
//!   own sequence. `H` is [`RECENCY_HALF_LIFE`]: the age at which half of the
//!   weight is left. The clock is never read (I-26).
//! * **Provenance** is the *weakest* step of the path, because a chain is as
//!   trustworthy as its least trustworthy link: `Declared` 3, `Imported` 2,
//!   `Algorithmic` 1, `Inferred` 0.
//! * **Similarity** exists only when the caller supplies a [`VectorSet`] and a
//!   [`VectorQuery`]; a hit without a vector in the set contributes 0.
//!
//! # Determinism
//!
//! No score depends on hash-map order, on the clock or on the caller's order
//! of anything: the hits are ordered by a total order, (score descending, unit
//! id ascending), over integers.
//!
//! The vector signal is the only floating-point computation. It is stable
//! because (1) every input is an `f32`, which `f64` holds exactly; (2) the
//! components are visited in index order with one accumulator each and only
//! `+`, `*`, `/` and `sqrt` are used, all correctly rounded by IEEE-754, and
//! Rust neither reassociates nor fuses them; no `libm` function (`exp`, `ln`,
//! `pow`, `acos`) is called; and (3) the result is *rounded to a fixed-point
//! integer* ([`SIMILARITY_SCALE`] units per 1.0) before it meets any other
//! number, so a score never holds a float and an order never compares one.
//!
//! # Vectors (ADR-0011)
//!
//! They are supplied by the caller, live in memory and are never written
//! anywhere. A [`VectorSet`] has one dimension and one metric (cosine or dot
//! product); the brute-force [`VectorSet::search`] is exact. Dimension and
//! metric are checked, and an empty, non-finite or (under cosine) zero
//! vector is rejected, with a [`VectorError`], before any ranking.
//!
//! # Literals (I-37)
//!
//! The errors, the `Debug` of every type that holds a caller's literal
//! (vectors, relation types) and the non-diagnostic output carry none of
//! them. The diagnostic output holds integers only, derived from the data.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use kerno_core::contract::Engine;

use crate::assoc::{AssocHit, AssocResult, MAX_ASSOC_DEPTH, TraceStep};
use crate::db::UnitSnapshot;
use crate::error::UnitError;
use crate::types::{ProvenanceKind, UnitId};

// ------------------------------------------------------------------ weights

/// Points per hop level of proximity. One hop closer is worth this much.
///
/// It is the unit the other weights are measured in. Proximity is the
/// structural evidence of association and the only signal every hit has, so it
/// is the largest of the weights that need no caller input.
pub const WEIGHT_HOPS: i32 = 1_000;
/// Points per step of trust of the weakest class on the path (0 to 3 steps).
///
/// 3 × 250 = 750 < [`WEIGHT_HOPS`]: provenance alone never overturns a hop.
pub const WEIGHT_PROVENANCE: i32 = 250;
/// Points when every step of the path has a preferred type.
///
/// Below one hop: the caller's taste for a type refines, it does not outrank
/// proximity.
pub const WEIGHT_TYPE: i32 = 500;
/// Points for a hit that changed at the snapshot's own sequence.
///
/// The weakest signal: a recently touched unit is not a more related one, it
/// only breaks a close contest.
pub const WEIGHT_RECENCY: i32 = 400;
/// Points for a similarity of 1.0 (and minus this for -1.0).
///
/// Above one hop and below two: a strong semantic match alone can overturn one
/// hop of distance, never two.
pub const WEIGHT_SIMILARITY: i32 = 1_500;
/// The age, in commits, at which the recency contribution is half its weight.
pub const RECENCY_HALF_LIFE: u32 = 64;
/// Fixed-point units per 1.0 of similarity: six decimals.
pub const SIMILARITY_SCALE: i64 = 1_000_000;

// The relations between the weights are the rationale of the defaults; a
// change that breaks one has to say why.
const _: () = {
    let (h, p, t, r, s) = (WEIGHT_HOPS, WEIGHT_PROVENANCE, WEIGHT_TYPE, WEIGHT_RECENCY, WEIGHT_SIMILARITY);
    // No non-hop signal alone overturns one hop...
    assert!(3 * p < h && t < h && r < h);
    // ...but the two strongest together can.
    assert!(3 * p + t > h);
    // Similarity alone overturns one hop and not two.
    assert!(s > h && s < 2 * h);
};

/// The weight of each signal. [`RankWeights::default`] is the documented
/// default; a caller may state its own (a weight of 0 turns a signal off, a
/// negative one inverts it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RankWeights {
    pub hops: i32,
    pub relation_type: i32,
    pub recency: i32,
    pub provenance: i32,
    pub similarity: i32,
}

impl Default for RankWeights {
    fn default() -> Self {
        RankWeights {
            hops: WEIGHT_HOPS,
            relation_type: WEIGHT_TYPE,
            recency: WEIGHT_RECENCY,
            provenance: WEIGHT_PROVENANCE,
            similarity: WEIGHT_SIMILARITY,
        }
    }
}

// ------------------------------------------------------------------ vectors

/// How two vectors are compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorMetric {
    /// The cosine of the angle: in `[-1, 1]`; undefined for a zero vector.
    Cosine,
    /// The dot product. Unbounded; the rank signal clamps it to `[-1, 1]`.
    Dot,
}

/// Why a vector, a set or a comparison was rejected. No variant holds a value
/// of a vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorError {
    /// A set with dimension 0.
    ZeroDimension,
    /// A vector with no components.
    Empty,
    /// The vector's dimension is not the set's.
    DimensionMismatch { expected: usize, got: usize },
    /// The query's metric is not the set's.
    MetricMismatch { set: VectorMetric, query: VectorMetric },
    /// A component (by position) is NaN or infinite.
    NonFinite { index: usize },
    /// A vector of norm zero under the cosine metric: its cosine is undefined.
    ZeroVector,
}

impl fmt::Display for VectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VectorError::ZeroDimension => write!(f, "a vector set needs a dimension above zero"),
            VectorError::Empty => write!(f, "the vector has no components"),
            VectorError::DimensionMismatch { expected, got } => {
                write!(f, "the vector has dimension {got}, the set {expected}")
            }
            VectorError::MetricMismatch { set, query } => {
                write!(f, "the query metric {query:?} is not the set's {set:?}")
            }
            VectorError::NonFinite { index } => write!(f, "component {index} of the vector is not finite"),
            VectorError::ZeroVector => write!(f, "a zero vector has no cosine"),
        }
    }
}

impl std::error::Error for VectorError {}

/// The shape checks that hold for one vector alone.
fn check_vector(metric: VectorMetric, v: &[f32]) -> Result<(), VectorError> {
    if v.is_empty() {
        return Err(VectorError::Empty);
    }
    if let Some(index) = v.iter().position(|x| !x.is_finite()) {
        return Err(VectorError::NonFinite { index });
    }
    if metric == VectorMetric::Cosine && v.iter().all(|&x| x == 0.0) {
        return Err(VectorError::ZeroVector);
    }
    Ok(())
}

/// The caller's vector to compare the hits against.
#[derive(Clone, PartialEq)]
pub struct VectorQuery {
    metric: VectorMetric,
    vector: Vec<f32>,
}

impl VectorQuery {
    /// A query vector, checked: not empty, finite, and not zero under cosine.
    pub fn new(metric: VectorMetric, vector: Vec<f32>) -> Result<Self, VectorError> {
        check_vector(metric, &vector)?;
        Ok(VectorQuery { metric, vector })
    }

    pub fn metric(&self) -> VectorMetric {
        self.metric
    }

    pub fn dimension(&self) -> usize {
        self.vector.len()
    }
}

impl fmt::Debug for VectorQuery {
    // The components are the caller's data (I-37): only the shape is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VectorQuery {{ metric: {:?}, dimension: {} }}", self.metric, self.vector.len())
    }
}

/// A unit and its exact similarity to a query, in fixed point
/// ([`SIMILARITY_SCALE`] units per 1.0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scored {
    pub unit: UnitId,
    pub similarity: i64,
}

/// Vectors of one dimension and one metric, by unit: supplied by the caller,
/// in memory only (ADR-0011). Nothing here is ever written to the store.
#[derive(Clone, PartialEq)]
pub struct VectorSet {
    dimension: usize,
    metric: VectorMetric,
    vectors: BTreeMap<UnitId, Vec<f32>>,
}

impl VectorSet {
    /// An empty set. `dimension` must be above zero.
    pub fn new(dimension: usize, metric: VectorMetric) -> Result<Self, VectorError> {
        if dimension == 0 {
            return Err(VectorError::ZeroDimension);
        }
        Ok(VectorSet { dimension, metric, vectors: BTreeMap::new() })
    }

    /// Adds the vector of `unit` (replacing a previous one). A vector that is
    /// empty, of another dimension, non-finite or, under cosine, zero is
    /// rejected and the set is unchanged.
    pub fn insert(&mut self, unit: UnitId, vector: Vec<f32>) -> Result<(), VectorError> {
        if vector.is_empty() {
            return Err(VectorError::Empty);
        }
        if vector.len() != self.dimension {
            return Err(VectorError::DimensionMismatch { expected: self.dimension, got: vector.len() });
        }
        check_vector(self.metric, &vector)?;
        self.vectors.insert(unit, vector);
        Ok(())
    }

    pub fn dimension(&self) -> usize {
        self.dimension
    }

    pub fn metric(&self) -> VectorMetric {
        self.metric
    }

    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// The metric and dimension of `query` against the set's: the check that
    /// comes before any comparison.
    pub fn check(&self, query: &VectorQuery) -> Result<(), VectorError> {
        if query.metric != self.metric {
            return Err(VectorError::MetricMismatch { set: self.metric, query: query.metric });
        }
        if query.vector.len() != self.dimension {
            return Err(VectorError::DimensionMismatch { expected: self.dimension, got: query.vector.len() });
        }
        Ok(())
    }

    /// The exact similarity of the vector of `unit` to `query`, or `None` if
    /// the set holds no vector for it.
    pub fn similarity(&self, unit: UnitId, query: &VectorQuery) -> Result<Option<i64>, VectorError> {
        self.check(query)?;
        Ok(self.vectors.get(&unit).map(|v| similarity(self.metric, v, &query.vector)))
    }

    /// The `k` most similar units by brute force: every vector is compared,
    /// in (similarity descending, unit id ascending) order. Exact by
    /// construction (ADR-0011: the recall reference of any future index).
    pub fn search(&self, query: &VectorQuery, k: usize) -> Result<Vec<Scored>, VectorError> {
        self.check(query)?;
        let mut all: Vec<Scored> = self
            .vectors
            .iter()
            .map(|(&unit, v)| Scored { unit, similarity: similarity(self.metric, v, &query.vector) })
            .collect();
        all.sort_by(|a, b| b.similarity.cmp(&a.similarity).then(a.unit.cmp(&b.unit)));
        all.truncate(k);
        Ok(all)
    }
}

impl fmt::Debug for VectorSet {
    // The components are the caller's data (I-37): only the shape is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "VectorSet {{ metric: {:?}, dimension: {}, vectors: {} }}",
            self.metric,
            self.dimension,
            self.vectors.len()
        )
    }
}

/// The similarity of two vectors of the same length, as a fixed-point
/// integer (see the module documentation, "Determinism").
fn similarity(metric: VectorMetric, a: &[f32], b: &[f32]) -> i64 {
    let (mut dot, mut norm_a, mut norm_b) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (f64::from(x), f64::from(y));
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let value = match metric {
        VectorMetric::Dot => dot,
        // Both norms are above zero (zero vectors are rejected on entry), and
        // the product of two squared f32 norms stays far from f64's limits.
        // Rounding can leave the quotient an ulp past 1.0; the fixed-point
        // rounding below absorbs that, since an ulp is far under 1e-6.
        VectorMetric::Cosine => dot / (norm_a * norm_b).sqrt(),
    };
    // `as` saturates at the i64 limits, and the value is finite.
    (value * SIMILARITY_SCALE as f64).round() as i64
}

// ------------------------------------------------------------------ ranking

/// How the hits of an associative result are ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RankMode {
    /// The order of the result ("distance, then unit id"); no score. The
    /// default until the measurement of E7-05 says otherwise.
    #[default]
    Off,
    /// Ordered by score.
    On,
    /// Ordered by score, with the contribution of each signal.
    Diagnostic,
}

/// The vectors to rank against: the caller's set and query vector.
#[derive(Clone, Copy, Debug)]
pub struct VectorSignal<'a> {
    pub set: &'a VectorSet,
    pub query: &'a VectorQuery,
}

/// What [`UnitSnapshot::rank_assoc`] is asked for.
#[derive(Clone, Default)]
pub struct RankOptions<'a> {
    pub mode: RankMode,
    pub weights: RankWeights,
    /// The relation types the relation-type signal rewards.
    pub preferred_types: BTreeSet<String>,
    /// The vector signal; absent means the signal does not exist.
    pub vectors: Option<VectorSignal<'a>>,
}

impl fmt::Debug for RankOptions<'_> {
    // The preferred types are the caller's literals (I-37): only their count.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RankOptions {{ mode: {:?}, weights: {:?}, preferred_types: {}, vectors: {:?} }}",
            self.mode,
            self.weights,
            self.preferred_types.len(),
            self.vectors
        )
    }
}

/// What each signal added to a score. The score is exactly [`Contributions::sum`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Contributions {
    pub hops: i64,
    pub relation_type: i64,
    pub recency: i64,
    pub provenance: i64,
    pub similarity: i64,
}

impl Contributions {
    /// The score: the sum of the five contributions. Each is bounded by an
    /// `i32` weight times a small factor, so the sum cannot overflow.
    pub fn sum(&self) -> i64 {
        self.hops + self.relation_type + self.recency + self.provenance + self.similarity
    }
}

/// A hit with its place in the ranking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedHit {
    pub unit: UnitId,
    pub distance: u32,
    /// The score; `None` with ranking off.
    pub score: Option<i64>,
    /// The contribution of each signal; only in [`RankMode::Diagnostic`].
    pub contributions: Option<Contributions>,
    /// The exact similarity in fixed point; only in [`RankMode::Diagnostic`],
    /// and only for a hit with a vector in the set.
    pub similarity: Option<i64>,
}

/// Why a ranking was refused.
#[derive(Debug)]
pub enum RankError {
    /// The vectors were rejected before any ranking.
    Vector(VectorError),
    /// A read of the snapshot failed.
    Unit(UnitError),
    /// A hit is not an active unit at this snapshot, or its revision is not
    /// the unit's head here: the result was not produced on this snapshot.
    /// The hit is not named: an error carries no data (I-37).
    NotFromThisSnapshot,
}

impl From<VectorError> for RankError {
    fn from(e: VectorError) -> Self {
        RankError::Vector(e)
    }
}

impl From<UnitError> for RankError {
    fn from(e: UnitError) -> Self {
        RankError::Unit(e)
    }
}

impl fmt::Display for RankError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RankError::Vector(e) => write!(f, "invalid vectors: {e}"),
            RankError::Unit(e) => write!(f, "ranking read failed: {e}"),
            RankError::NotFromThisSnapshot => {
                write!(f, "a hit does not belong to the snapshot the ranking was asked on")
            }
        }
    }
}

impl std::error::Error for RankError {}

impl<E: Engine> UnitSnapshot<E> {
    /// Orders the hits of `result`, which must come from an associative query
    /// on this snapshot (see the [module documentation](self)). The vectors,
    /// if any, are checked first and a rejection returns before anything is
    /// ranked, in every mode. Ranking reorders the hits the query returned: a
    /// hit the query cut off (`LimitReached`, `BudgetExhausted`) is not seen.
    pub fn rank_assoc(&self, result: &AssocResult, opts: &RankOptions<'_>) -> Result<Vec<RankedHit>, RankError> {
        if let Some(v) = &opts.vectors {
            v.set.check(v.query)?;
        }
        if opts.mode == RankMode::Off {
            return Ok(result
                .hits
                .iter()
                .map(|h| RankedHit {
                    unit: h.unit,
                    distance: h.distance,
                    score: None,
                    contributions: None,
                    similarity: None,
                })
                .collect());
        }
        let now = self.seq().0;
        let mut ranked = Vec::with_capacity(result.hits.len());
        for hit in &result.hits {
            let head_seq = self.head_seq(hit)?;
            let sim = match &opts.vectors {
                Some(v) => v.set.similarity(hit.unit, v.query)?,
                None => None,
            };
            let c = contributions(&opts.weights, &opts.preferred_types, hit, head_seq, now, sim);
            let diagnostic = opts.mode == RankMode::Diagnostic;
            ranked.push(RankedHit {
                unit: hit.unit,
                distance: hit.distance,
                score: Some(c.sum()),
                contributions: diagnostic.then_some(c),
                similarity: if diagnostic { sim } else { None },
            });
        }
        ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.unit.cmp(&b.unit)));
        Ok(ranked)
    }

    /// The commit sequence of the head revision of the hit's unit here.
    fn head_seq(&self, hit: &AssocHit) -> Result<u64, RankError> {
        match self.reference_get(hit.unit)? {
            Some(u) if u.head == hit.revision => Ok(u.updated_seq.0),
            _ => Err(RankError::NotFromThisSnapshot),
        }
    }
}

/// The five contributions of one hit. `now` is the snapshot's sequence and
/// `head_seq` the commit sequence of the hit's head revision.
fn contributions(
    w: &RankWeights,
    preferred: &BTreeSet<String>,
    hit: &AssocHit,
    head_seq: u64,
    now: u64,
    sim: Option<i64>,
) -> Contributions {
    let depth = i64::from(MAX_ASSOC_DEPTH) + 1;
    let hops = i64::from(w.hops) * (depth - i64::from(hit.distance)).max(0);

    // A hit with no path has no relation to judge: those two signals are 0.
    let steps = hit.path.len() as i64;
    let relation_type = if steps == 0 {
        0
    } else {
        let hits = hit.path.iter().filter(|s| preferred.contains(&s.relation_type)).count() as i64;
        i64::from(w.relation_type) * hits / steps
    };
    let provenance = hit.path.iter().map(|s| trust(s.provenance)).min().map_or(0, |t| i64::from(w.provenance) * t);

    let newest = hit.path.last().map_or(head_seq, |s: &TraceStep| head_seq.max(s.created_seq.0));
    // An age above u32::MAX commits weighs the same as u32::MAX: the
    // contribution is already 0 there.
    let age = i64::from(u32::try_from(now.saturating_sub(newest)).unwrap_or(u32::MAX));
    let half_life = i64::from(RECENCY_HALF_LIFE);
    let recency = i64::from(w.recency) * half_life / (half_life + age);

    let similarity =
        sim.map_or(0, |s| i64::from(w.similarity) * s.clamp(-SIMILARITY_SCALE, SIMILARITY_SCALE) / SIMILARITY_SCALE);

    Contributions { hops, relation_type, recency, provenance, similarity }
}

/// Steps of trust: the class a reader can rely on most comes first.
fn trust(kind: ProvenanceKind) -> i64 {
    match kind {
        ProvenanceKind::Declared => 3,
        ProvenanceKind::Imported => 2,
        ProvenanceKind::Algorithmic => 1,
        ProvenanceKind::Inferred => 0,
    }
}
