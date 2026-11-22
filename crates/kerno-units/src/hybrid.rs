//! Hybrid retrieval: one query over keywords, vectors and relations (Phase B).
//!
//! [`UnitSnapshot::hybrid_search`] runs up to three legs and fuses their
//! rankings with weighted reciprocal rank fusion (RRF):
//!
//! | leg | source | rank order |
//! |---|---|---|
//! | keyword | [`UnitSnapshot::search_text`] (BM25) | score, then unit id |
//! | vector | a [`NearestUnits`] index the caller supplies: an [`AnnIndex`] (HNSW) or a [`VectorSet`] (exact) | nearest first |
//! | graph | [`UnitSnapshot::assoc_query`] | (distance, unit id) |
//!
//! # Fusion
//!
//! A unit holding rank `r` (1-based) in a leg of weight `w` earns
//! `w * RRF_SCALE / (rrf_k + r)` (integer division, the multiplication
//! first); its score is the sum over the legs that hold it. At `rrf_k = 60`
//! and weight 1, rank 1 earns 16_393_442, rank 2 earns 16_129_032 and rank 3
//! earns 15_873_015; weight 3 at rank 1 earns 49_180_327. The fusion is a
//! union: a unit one leg finds is a candidate even if the others miss it.
//! Hits are ordered by (score descending, unit id ascending), so the answer
//! is deterministic and no float takes part in the fusion (BM25 floats only
//! decide the keyword leg's own order).
//!
//! # Fusion modes
//!
//! [`Fusion::Rrf`] (the default) is the union above. [`Fusion::GraphGated`]
//! computes the same scores and ranks but changes the order: when the graph
//! leg ran, every unit the walk found ranks above every unit it did not, and
//! the walk's hits keep their hop order (a unit one hop from a seed ranks above
//! a unit two hops away whatever the other legs say). Inside one hop the order
//! is (score descending, unit id ascending), so the vector and keyword legs
//! refine each hop; the units no link reaches follow all of them, in union
//! order. The graph is then the authority on which units lead and how near they
//! are, which is what a query asks for when its relations carry the relevance
//! (a time window or a provenance filter that the other legs cannot see).
//! Without a graph leg, or when the walk finds nothing, no unit has a hop and
//! the two modes return the same list.
//!
//! # Rules fixed by the design
//!
//! * A leg of weight 0, or without its input, is not run.
//! * When the graph leg runs, its seeds are removed from the keyword and
//!   vector legs before ranks are assigned: "the seeds are not hits"
//!   ([`crate::AssocResult`]).
//! * The vector index belongs to the caller and may lag the units. A vector
//!   hit is kept only if its unit is `Active` at the snapshot, so an archived
//!   or deleted unit never resurfaces. Ranks count after that filter.
//! * Each leg contributes at most `candidates` units.
//! * I-37: neither the `Debug` of a query nor any error carries the query
//!   text or a vector element.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use kerno_core::contract::{Engine, TxnEngine};

use crate::ann::AnnIndex;
use crate::assoc::{AssocError, AssocQuery};
use crate::assoc_rank::{VectorQuery, VectorSet};
use crate::db::{UnitDb, UnitSnapshot};
use crate::error::UnitError;
use crate::types::{Lifecycle, UnitId};

/// The `k` of RRF when a query does not choose one.
pub const DEFAULT_RRF_K: u32 = 60;
/// Units each leg contributes when a query does not choose.
pub const DEFAULT_CANDIDATES: usize = 100;
/// Most hits one query returns.
pub const MAX_HYBRID_LIMIT: usize = 1_000;
/// Most units one leg contributes.
pub const MAX_HYBRID_CANDIDATES: usize = 1_000;
/// Largest weight of a leg.
pub const MAX_HYBRID_WEIGHT: u32 = 1_000;
/// The fixed-point scale of a contribution: one full unit of rank score.
pub const RRF_SCALE: u64 = 1_000_000_000;

/// Something that answers "which units are nearest to this vector".
pub trait NearestUnits {
    /// Up to `k` units nearest to `query`, nearest first.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for a query the index refuses (another dimension, a
    /// non-finite element, a zero vector under cosine).
    fn nearest(&self, query: &[f32], k: usize) -> Result<Vec<UnitId>, UnitError>;
}

impl<A: Engine> NearestUnits for AnnIndex<A> {
    fn nearest(&self, query: &[f32], k: usize) -> Result<Vec<UnitId>, UnitError> {
        Ok(self.search(query, k)?.into_iter().map(|h| h.unit).collect())
    }
}

impl NearestUnits for VectorSet {
    fn nearest(&self, query: &[f32], k: usize) -> Result<Vec<UnitId>, UnitError> {
        let refuse = |e: crate::assoc_rank::VectorError| UnitError::InvalidArgument(format!("vector leg: {e}"));
        let q = VectorQuery::new(self.metric(), query.to_vec()).map_err(refuse)?;
        Ok(self.search(&q, k).map_err(refuse)?.into_iter().map(|s| s.unit).collect())
    }
}

/// How the legs' rankings become one list. The scores and the per-leg ranks of
/// a hit do not depend on it: only the order does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fusion {
    /// Weighted reciprocal rank fusion over the union of the legs.
    #[default]
    Rrf,
    /// Units the graph leg found first, by hops from the seeds; the others
    /// after. Within a hop, and among the others, [`Fusion::Rrf`] order. See
    /// the [module documentation](self).
    GraphGated,
}

/// How much each leg counts in the fusion; 0 switches a leg off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HybridWeights {
    pub keyword: u32,
    pub vector: u32,
    pub graph: u32,
}

impl Default for HybridWeights {
    fn default() -> Self {
        HybridWeights { keyword: 1, vector: 1, graph: 1 }
    }
}

/// The vector leg: an index and the vector to look up in it.
#[derive(Clone, Copy)]
pub struct VectorLeg<'q> {
    pub index: &'q dyn NearestUnits,
    pub query: &'q [f32],
}

impl fmt::Debug for VectorLeg<'_> {
    // The components are the caller's data (I-37): only the shape is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VectorLeg {{ dimension: {} }}", self.query.len())
    }
}

/// One hybrid query. A leg runs when its input is present and its weight is
/// above 0; at least one must run.
#[derive(Clone)]
pub struct HybridQuery<'q> {
    /// The keyword leg's text.
    pub text: Option<&'q str>,
    pub vector: Option<VectorLeg<'q>>,
    /// The graph leg's query. Its own `limit` bounds the traversal; the leg
    /// contributes at most `candidates` units.
    pub graph: Option<AssocQuery>,
    pub weights: HybridWeights,
    /// How the legs combine; [`Fusion::Rrf`] unless the caller chooses.
    pub fusion: Fusion,
    /// The `k` of RRF.
    pub rrf_k: u32,
    /// Units each leg contributes, `1..=`[`MAX_HYBRID_CANDIDATES`].
    pub candidates: usize,
    /// Hits returned, `1..=`[`MAX_HYBRID_LIMIT`].
    pub limit: usize,
}

impl HybridQuery<'_> {
    /// A query with no leg yet, default weights, [`DEFAULT_RRF_K`] and
    /// [`DEFAULT_CANDIDATES`], returning at most `limit` hits.
    pub fn new(limit: usize) -> Self {
        HybridQuery {
            text: None,
            vector: None,
            graph: None,
            weights: HybridWeights::default(),
            fusion: Fusion::default(),
            rrf_k: DEFAULT_RRF_K,
            candidates: DEFAULT_CANDIDATES,
            limit,
        }
    }

    fn runs_keyword(&self) -> bool {
        self.text.is_some() && self.weights.keyword > 0
    }

    fn runs_vector(&self) -> bool {
        self.vector.is_some() && self.weights.vector > 0
    }

    fn runs_graph(&self) -> bool {
        self.graph.is_some() && self.weights.graph > 0
    }

    fn validate(&self) -> Result<(), UnitError> {
        let bad = |what: String| Err(UnitError::InvalidArgument(format!("hybrid query: {what}")));
        if !(1..=MAX_HYBRID_LIMIT).contains(&self.limit) {
            return bad(format!("limit {} is outside 1..={MAX_HYBRID_LIMIT}", self.limit));
        }
        if !(1..=MAX_HYBRID_CANDIDATES).contains(&self.candidates) {
            return bad(format!("candidates {} is outside 1..={MAX_HYBRID_CANDIDATES}", self.candidates));
        }
        let w = self.weights;
        for (name, weight) in [("keyword", w.keyword), ("vector", w.vector), ("graph", w.graph)] {
            if weight > MAX_HYBRID_WEIGHT {
                return bad(format!("weight {weight} of the {name} leg is above {MAX_HYBRID_WEIGHT}"));
            }
        }
        if !(self.runs_keyword() || self.runs_vector() || self.runs_graph()) {
            return bad("no leg to run (give text, vector or graph with a weight above 0)".into());
        }
        Ok(())
    }
}

impl fmt::Debug for HybridQuery<'_> {
    // The text and the vector are the caller's data (I-37): only sizes are shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HybridQuery")
            .field("text_bytes", &self.text.map(str::len))
            .field("vector_dimension", &self.vector.map(|v| v.query.len()))
            .field("graph_seeds", &self.graph.as_ref().map(|g| g.seeds.len()))
            .field("weights", &self.weights)
            .field("fusion", &self.fusion)
            .field("rrf_k", &self.rrf_k)
            .field("candidates", &self.candidates)
            .field("limit", &self.limit)
            .finish()
    }
}

/// The 1-based rank a unit held in each leg that found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LegRanks {
    pub keyword: Option<u32>,
    pub vector: Option<u32>,
    pub graph: Option<u32>,
}

/// One fused hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HybridHit {
    pub unit: UnitId,
    /// The sum of the leg contributions, in units of `1 / RRF_SCALE`.
    pub score: u64,
    pub ranks: LegRanks,
}

/// What a leg of rank `rank` and weight `weight` adds to a unit's score.
fn contribution(weight: u32, rrf_k: u32, rank: u32) -> u64 {
    u64::from(weight) * RRF_SCALE / (u64::from(rrf_k) + u64::from(rank))
}

fn graph_error(e: AssocError) -> UnitError {
    match e {
        AssocError::Query(e) => UnitError::InvalidArgument(format!("hybrid query: graph leg: {e}")),
        AssocError::SeedNotActive { unit } => UnitError::InvalidArgument(format!(
            "hybrid query: graph leg: seed {unit} is not an active unit at this snapshot"
        )),
        AssocError::Unit(e) => e,
        AssocError::IndexAnswer(v) => UnitError::Storage(kerno_core::Error::Corruption(format!(
            "hybrid query: graph leg: index answer violation {v:?}"
        ))),
    }
}

impl<E: Engine> UnitSnapshot<E> {
    /// The units that best match `q` across its legs, fused by weighted
    /// reciprocal rank fusion in the order `q.fusion` gives (see the
    /// [module documentation](self)), at most `q.limit`, best first.
    ///
    /// # Errors
    ///
    /// `NotIndexed` for a keyword leg when no type is keyword-indexed; `InvalidArgument` for a
    /// query outside its bounds, with no leg to run,
    /// whose graph leg the model rejects or whose seed is not active, or whose
    /// vector the index refuses; `Storage` when a read fails or a record is
    /// damaged.
    pub fn hybrid_search(&self, q: &HybridQuery<'_>) -> Result<Vec<HybridHit>, UnitError> {
        q.validate()?;
        let mut fused: BTreeMap<UnitId, (u64, LegRanks)> = BTreeMap::new();
        let mut add = |leg: &[UnitId], weight: u32, set: fn(&mut LegRanks, u32)| {
            for (i, &unit) in leg.iter().enumerate() {
                // `leg` holds at most MAX_HYBRID_CANDIDATES units.
                let rank = u32::try_from(i + 1).unwrap_or(u32::MAX);
                let entry = fused.entry(unit).or_default();
                entry.0 += contribution(weight, q.rrf_k, rank);
                set(&mut entry.1, rank);
            }
        };

        let mut seeds: BTreeSet<UnitId> = BTreeSet::new();
        let mut graph = Vec::new();
        // Hops from the seeds of each unit the graph leg contributes.
        let mut hops: BTreeMap<UnitId, u32> = BTreeMap::new();
        if let (true, Some(g)) = (q.runs_graph(), &q.graph) {
            seeds.extend(g.seeds.iter().copied());
            let outcome = self.assoc_query(g).map_err(graph_error)?;
            for hit in outcome.result.hits.iter().take(q.candidates) {
                graph.push(hit.unit);
                hops.insert(hit.unit, hit.distance);
            }
        }

        let mut keyword = Vec::new();
        if let (true, Some(text)) = (q.runs_keyword(), q.text) {
            // With no type keyword-indexed the leg can never match: say so, not "no hits".
            if !self.has_keyword_index()? {
                return Err(UnitError::NotIndexed {
                    detail: "no unit type is keyword-indexed: turn it on for a type with set_fulltext".into(),
                });
            }
            let hits = self.search_text(text, q.candidates + seeds.len())?;
            keyword = hits.into_iter().map(|h| h.unit).filter(|u| !seeds.contains(u)).take(q.candidates).collect();
        }

        let mut vector = Vec::new();
        if let (true, Some(leg)) = (q.runs_vector(), q.vector) {
            for unit in leg.index.nearest(leg.query, q.candidates + seeds.len())? {
                if vector.len() == q.candidates {
                    break;
                }
                if seeds.contains(&unit) {
                    continue;
                }
                if self.get_unit(unit)?.is_some_and(|u| u.lifecycle == Lifecycle::Active) {
                    vector.push(unit);
                }
            }
        }

        add(&keyword, q.weights.keyword, |r, n| r.keyword = Some(n));
        add(&vector, q.weights.vector, |r, n| r.vector = Some(n));
        add(&graph, q.weights.graph, |r, n| r.graph = Some(n));

        let mut hits: Vec<HybridHit> =
            fused.into_iter().map(|(unit, (score, ranks))| HybridHit { unit, score, ranks }).collect();
        // Under `GraphGated` the walk's hits come first, nearest hop first, and
        // the units it did not find after them; under `Rrf` no hit has a tier.
        let tier = |h: &HybridHit| match (q.fusion, hops.get(&h.unit)) {
            (Fusion::Rrf, _) => (0, 0),
            (Fusion::GraphGated, Some(&hop)) => (0, hop),
            (Fusion::GraphGated, None) => (1, 0),
        };
        hits.sort_by(|a, b| tier(a).cmp(&tier(b)).then(b.score.cmp(&a.score)).then(a.unit.cmp(&b.unit)));
        hits.truncate(q.limit);
        Ok(hits)
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// [`UnitSnapshot::hybrid_search`] on a fresh snapshot.
    pub fn hybrid_search(&self, q: &HybridQuery<'_>) -> Result<Vec<HybridHit>, UnitError> {
        self.snapshot()?.hybrid_search(q)
    }
}
