//! E7-01: the associative query model and its reference path (etapa7-acta.md
//! §3, §4; `docs/architecture/etapa7-consulta-asociativa.md`).
//!
//! An [`AssocQuery`] starts at one or more seed units and follows typed
//! relations for several hops, under three mandatory bounds (depth, result
//! limit, visit budget) that a caller may lower but never raise past the
//! hard maxima [`MAX_ASSOC_DEPTH`], [`MAX_ASSOC_LIMIT`] and
//! [`MAX_ASSOC_VISITS`]. Each hit of the [`AssocResult`] carries its trace:
//! the relations of the path that reached it and its current revision.
//!
//! [`UnitSnapshot::reference_assoc_query`] is the reference path: a
//! breadth-first traversal built only on
//! [`UnitSnapshot::reference_relations_of`], so it touches no adjacency
//! index. The indexed path (E7-02) is the very same traversal taking each
//! hop's neighbours from [`UnitSnapshot::relations_of`] instead, every answer
//! checked ([`UnitSnapshot::indexed_assoc_query`]); the planner chooses
//! between them and falls back to the reference path
//! ([`UnitSnapshot::assoc_query`], `docs/architecture/etapa7-ejecucion-indexada.md`).
//! Both are held to the same answer on the same snapshot (I-32): sharing the
//! traversal makes that true of the order, the budget and the endings by
//! construction, and the differential tests make it true of the data.
//!
//! # Order (I-26)
//!
//! Hits are in the order (hop distance, unit id). Which path a hit carries
//! is the first one in this examination order:
//!
//! 1. the units of one hop level are expanded in ascending unit id (the
//!    seeds, at distance 0, too);
//! 2. for one unit, the relations it starts (`Out`) come before the ones it
//!    ends (`In`), when the query follows both;
//! 3. within one direction, relations are in relation type bytes, then
//!    relation id bytes (the order of `reference_relations_of`).
//!
//! Nothing depends on hash-map iteration, on the caller's order of seeds, or
//! on any clock reading.
//!
//! # Visit budget
//!
//! An entry is *examined* when the traversal looks at a relation adjacent to
//! the unit being expanded, in a followed direction, whose type is in the
//! query's type set (any type when there is none). It counts before the
//! window and provenance filters, which need the relation's record anyway,
//! and whether or not it leads to a new unit. A relation from a unit to
//! itself has two adjacency entries and counts twice when both directions
//! are followed.
//!
//! # Units that are not active
//!
//! The layer refuses to archive or delete a unit that a relation touches
//! (ADR-0014), so at a sound snapshot every end of every relation is
//! `Active`. The traversal still decides explicitly what it does if one is
//! not (damage that `verify_units` reports as UNIT-36): a seed that is not an
//! active unit rejects the query ([`AssocError::SeedNotActive`]); any other
//! unit that is missing, archived or deleted is treated as absent: it is not
//! returned and not expanded, and the entry that led to it still counts as
//! examined. This is the rule of the reference query path: a query sees
//! active units only.

use std::collections::{BTreeMap, BTreeSet};

use kerno_core::contract::Engine;

use crate::db::{Relation, UnitSnapshot, corruption};
use crate::error::UnitError;
use crate::fingerprint::RelationQuery;
use crate::history::Provenance;
use crate::observation::ErrorKind;
use crate::planner::Violation;
use crate::types::{CommitSeq, Direction, ProvenanceKind, RelationId, RevNo, UnitId};

/// Hard maximum of [`AssocQuery::max_depth`] (acta §8, decision 5).
pub const MAX_ASSOC_DEPTH: u32 = 4;
/// Hard maximum of [`AssocQuery::limit`].
pub const MAX_ASSOC_LIMIT: usize = 1_000;
/// Hard maximum of [`AssocQuery::visit_budget`]: relation entries examined.
pub const MAX_ASSOC_VISITS: u64 = 100_000;

/// Which relations are followed from the unit the traversal is at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssocDirection {
    /// Relations that start at the unit.
    Out,
    /// Relations that end at the unit.
    In,
    /// Both: `Out` entries first, then `In` entries.
    Both,
}

impl AssocDirection {
    pub(crate) fn directions(self) -> &'static [Direction] {
        match self {
            AssocDirection::Out => &[Direction::Out],
            AssocDirection::In => &[Direction::In],
            AssocDirection::Both => &[Direction::Out, Direction::In],
        }
    }
}

/// An inclusive window of commit sequences, applied to the `created_seq` of
/// each relation followed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeqWindow {
    pub lo: CommitSeq,
    pub hi: CommitSeq,
}

/// A filter on the provenance of the relations followed. Every field that is
/// set must match; the default matches every relation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvenanceFilter {
    /// The kinds of relation provenance allowed (`None`: any).
    pub kinds: Option<BTreeSet<ProvenanceKind>>,
    /// The actor of the change set that created the relation (`None`: any).
    pub actor: Option<Vec<u8>>,
    /// The source of the change set that created the relation (`None`: any).
    pub source: Option<Vec<u8>>,
}

/// An associative query. The three bounds have no default: a caller states
/// them, and [`AssocQuery::validate`] holds them under the hard maxima.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssocQuery {
    /// Where the traversal starts: at least one. Repeated seeds count once.
    pub seeds: Vec<UnitId>,
    /// The relation types followed (`None`: any type).
    pub relation_types: Option<BTreeSet<String>>,
    pub direction: AssocDirection,
    /// Most hops from a seed, `1..=`[`MAX_ASSOC_DEPTH`].
    pub max_depth: u32,
    /// Most hits returned, `1..=`[`MAX_ASSOC_LIMIT`].
    pub limit: usize,
    /// Most relation entries examined, `1..=`[`MAX_ASSOC_VISITS`].
    pub visit_budget: u64,
    /// Only relations created inside the window are followed.
    pub window: Option<SeqWindow>,
    pub provenance: ProvenanceFilter,
}

impl AssocQuery {
    /// A query from `seeds` following any relation in both directions, with
    /// the three bounds given and no window or provenance filter.
    pub fn new(seeds: Vec<UnitId>, max_depth: u32, limit: usize, visit_budget: u64) -> Self {
        AssocQuery {
            seeds,
            relation_types: None,
            direction: AssocDirection::Both,
            max_depth,
            limit,
            visit_budget,
            window: None,
            provenance: ProvenanceFilter::default(),
        }
    }

    /// Checks the query without reading anything. A query that fails is
    /// never run.
    pub fn validate(&self) -> Result<(), AssocQueryError> {
        if self.seeds.is_empty() {
            return Err(AssocQueryError::NoSeeds);
        }
        if self.max_depth == 0 || self.max_depth > MAX_ASSOC_DEPTH {
            return Err(AssocQueryError::DepthOutOfRange { depth: self.max_depth });
        }
        if self.limit == 0 || self.limit > MAX_ASSOC_LIMIT {
            return Err(AssocQueryError::LimitOutOfRange { limit: self.limit });
        }
        if self.visit_budget == 0 || self.visit_budget > MAX_ASSOC_VISITS {
            return Err(AssocQueryError::BudgetOutOfRange { budget: self.visit_budget });
        }
        if self.relation_types.as_ref().is_some_and(BTreeSet::is_empty) {
            return Err(AssocQueryError::EmptyRelationTypes);
        }
        if self.provenance.kinds.as_ref().is_some_and(BTreeSet::is_empty) {
            return Err(AssocQueryError::EmptyProvenanceKinds);
        }
        if let Some(w) = self.window
            && w.lo > w.hi
        {
            return Err(AssocQueryError::InvertedWindow { lo: w.lo, hi: w.hi });
        }
        Ok(())
    }
}

/// Why a query was rejected before running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssocQueryError {
    /// No seed unit.
    NoSeeds,
    /// `max_depth` is 0 or above [`MAX_ASSOC_DEPTH`].
    DepthOutOfRange { depth: u32 },
    /// `limit` is 0 or above [`MAX_ASSOC_LIMIT`].
    LimitOutOfRange { limit: usize },
    /// `visit_budget` is 0 or above [`MAX_ASSOC_VISITS`].
    BudgetOutOfRange { budget: u64 },
    /// A relation type set that is present and empty would follow nothing.
    EmptyRelationTypes,
    /// A provenance kind set that is present and empty would follow nothing.
    EmptyProvenanceKinds,
    /// The window's lower bound is above its upper bound.
    InvertedWindow { lo: CommitSeq, hi: CommitSeq },
}

impl std::fmt::Display for AssocQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssocQueryError::NoSeeds => write!(f, "an associative query needs at least one seed"),
            AssocQueryError::DepthOutOfRange { depth } => {
                write!(f, "max_depth {depth} is outside 1..={MAX_ASSOC_DEPTH}")
            }
            AssocQueryError::LimitOutOfRange { limit } => write!(f, "limit {limit} is outside 1..={MAX_ASSOC_LIMIT}"),
            AssocQueryError::BudgetOutOfRange { budget } => {
                write!(f, "visit_budget {budget} is outside 1..={MAX_ASSOC_VISITS}")
            }
            AssocQueryError::EmptyRelationTypes => write!(f, "the relation type set is empty"),
            AssocQueryError::EmptyProvenanceKinds => write!(f, "the provenance kind set is empty"),
            AssocQueryError::InvertedWindow { lo, hi } => write!(f, "window {lo}..={hi} has its bounds inverted"),
        }
    }
}

impl std::error::Error for AssocQueryError {}

/// Errors of an associative query.
#[derive(Debug)]
pub enum AssocError {
    /// The query was rejected before running.
    Query(AssocQueryError),
    /// A seed is not an active unit at the snapshot: it never existed, or it
    /// is archived or deleted.
    SeedNotActive { unit: UnitId },
    /// A read failed.
    Unit(UnitError),
    /// The adjacency index answered something that is not an answer to the
    /// hop it was asked (E7-02). Only the indexed path returns it; the planner
    /// turns it into a recorded fallback to the reference path.
    IndexAnswer(Violation),
}

impl AssocError {
    /// The class of the error, for an observation: never its message.
    pub(crate) fn kind(&self) -> ErrorKind {
        match self {
            AssocError::Query(_) | AssocError::SeedNotActive { .. } => ErrorKind::Invalid,
            AssocError::Unit(e) => ErrorKind::of(e),
            AssocError::IndexAnswer(_) => ErrorKind::Corruption,
        }
    }
}

impl From<AssocQueryError> for AssocError {
    fn from(e: AssocQueryError) -> Self {
        AssocError::Query(e)
    }
}

impl From<UnitError> for AssocError {
    fn from(e: UnitError) -> Self {
        AssocError::Unit(e)
    }
}

impl std::fmt::Display for AssocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AssocError::Query(e) => write!(f, "invalid associative query: {e}"),
            AssocError::SeedNotActive { unit } => write!(f, "seed {unit} is not an active unit at this snapshot"),
            AssocError::Unit(e) => write!(f, "associative query read failed: {e}"),
            AssocError::IndexAnswer(v) => write!(f, "the adjacency index gave a wrong answer: {v:?}"),
        }
    }
}

impl std::error::Error for AssocError {}

/// One relation followed on the path to a hit. `from` and `to` are the
/// relation's own ends, not the direction it was followed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceStep {
    pub relation: RelationId,
    pub relation_type: String,
    pub from: UnitId,
    pub to: UnitId,
    pub created_seq: CommitSeq,
    pub provenance: ProvenanceKind,
}

/// A unit reached by the traversal, with its trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssocHit {
    pub unit: UnitId,
    /// Hops from the nearest seed: `path.len()`.
    pub distance: u32,
    /// The unit's head revision at the snapshot.
    pub revision: RevNo,
    /// The relations followed from a seed to `unit`, in order.
    pub path: Vec<TraceStep>,
}

/// Why the traversal stopped. Exactly one holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssocEnding {
    /// Nothing was left within the depth bound: the hits are the whole
    /// answer.
    Exhausted,
    /// More hits exist than `limit`: the hits are the first `limit` of the
    /// answer.
    LimitReached,
    /// The visit budget ran out before the traversal could finish: the hits
    /// are valid but the answer may hold more, and a hit of the level being
    /// expanded may be missing before one that is returned.
    BudgetExhausted,
}

/// The answer of an associative query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssocResult {
    /// In the order (distance, unit id); the seeds are not hits.
    pub hits: Vec<AssocHit>,
    pub ending: AssocEnding,
    /// Relation entries examined (never above the budget).
    pub visited: u64,
}

impl<E: Engine> UnitSnapshot<E> {
    /// The reference answer of an associative query: a breadth-first
    /// traversal over [`UnitSnapshot::reference_relations_of`] only (see the
    /// [module documentation](self) for the order, the budget and the
    /// handling of units that are not active). Each unit is returned once,
    /// with the first path that reaches it.
    pub fn reference_assoc_query(&self, q: &AssocQuery) -> Result<AssocResult, AssocError> {
        let seeds = self.active_seeds(q)?;
        self.reference_traverse(q, seeds)
    }

    /// The answer of an associative query taking every hop from the adjacency
    /// index: [`UnitSnapshot::relations_of`], with each answer checked
    /// ([`UnitSnapshot::check_adjacent`]). The result equals
    /// [`UnitSnapshot::reference_assoc_query`]'s on the same snapshot (I-32).
    /// This is the path alone, chosen by the caller: a wrong answer from the
    /// index is the error [`AssocError::IndexAnswer`], not a fallback.
    /// [`UnitSnapshot::assoc_query`] is the planner's call.
    pub fn indexed_assoc_query(&self, q: &AssocQuery) -> Result<AssocResult, AssocError> {
        let seeds = self.active_seeds(q)?;
        self.indexed_traverse(q, seeds, |unit, dir, relation_type| self.relations_of(unit, dir, relation_type))
    }

    /// The query validated and its seeds found active, before any path runs.
    pub(crate) fn active_seeds(&self, q: &AssocQuery) -> Result<BTreeSet<UnitId>, AssocError> {
        q.validate()?;
        let seeds: BTreeSet<UnitId> = q.seeds.iter().copied().collect();
        for &seed in &seeds {
            if self.reference_get(seed)?.is_none() {
                return Err(AssocError::SeedNotActive { unit: seed });
            }
        }
        Ok(seeds)
    }

    pub(crate) fn reference_traverse(
        &self,
        q: &AssocQuery,
        seeds: BTreeSet<UnitId>,
    ) -> Result<AssocResult, AssocError> {
        self.traverse(q, seeds, |here, dir| Ok(self.reference_relations_of(here, dir, None)?))
    }

    /// The traversal with each hop read through `hop` (the adjacency range of
    /// one unit, direction and type) and checked. One read per relation type
    /// of the query, in byte order of the type: concatenated they are in the
    /// (type, id) order of `reference_relations_of`, and the entries of a type
    /// the query does not follow are never read.
    pub(crate) fn indexed_traverse(
        &self,
        q: &AssocQuery,
        seeds: BTreeSet<UnitId>,
        mut hop: impl FnMut(UnitId, Direction, Option<&str>) -> Result<Vec<Relation>, UnitError>,
    ) -> Result<AssocResult, AssocError> {
        let types: Vec<Option<&str>> = match &q.relation_types {
            None => vec![None],
            Some(set) => set.iter().map(|t| Some(t.as_str())).collect(),
        };
        self.traverse(q, seeds, |here, dir| {
            let mut found = Vec::new();
            for &relation_type in &types {
                let part = hop(here, dir, relation_type)?;
                let asked =
                    RelationQuery { unit: here, direction: dir, relation_type: relation_type.map(str::to_owned) };
                self.check_adjacent(&asked, &part)?;
                found.extend(part);
            }
            Ok(found)
        })
    }

    /// The breadth-first traversal; `neighbours` gives the relations of one
    /// unit in one direction in (type, id) order, of any type.
    fn traverse(
        &self,
        q: &AssocQuery,
        seeds: BTreeSet<UnitId>,
        mut neighbours: impl FnMut(UnitId, Direction) -> Result<Vec<Relation>, AssocError>,
    ) -> Result<AssocResult, AssocError> {
        let mut run = Run { snap: self, q, visited: 0, actors: BTreeMap::new() };
        // Every unit reached so far, active or not, so that a unit is
        // resolved once.
        let mut seen: BTreeSet<UnitId> = seeds.clone();
        let mut hits: Vec<AssocHit> = Vec::new();
        let mut frontier: Vec<(UnitId, Vec<TraceStep>)> = seeds.into_iter().map(|s| (s, Vec::new())).collect();

        for _ in 0..q.max_depth {
            let mut level: BTreeMap<UnitId, AssocHit> = BTreeMap::new();
            for (here, path) in &frontier {
                for &dir in q.direction.directions() {
                    let entries = neighbours(*here, dir)?;
                    for rel in entries.into_iter().filter(|r| type_allowed(q, r)) {
                        if run.visited == q.visit_budget {
                            return Ok(run.finish_partial(hits, level));
                        }
                        run.visited += 1;
                        if !run.followed(&rel)? {
                            continue;
                        }
                        let there = if dir == Direction::Out { rel.to } else { rel.from };
                        if !seen.insert(there) {
                            continue;
                        }
                        let Some(unit) = self.reference_get(there)? else { continue };
                        if hits.len() == q.limit {
                            // One more hit exists beyond the limit.
                            return Ok(AssocResult { hits, ending: AssocEnding::LimitReached, visited: run.visited });
                        }
                        let mut p = path.clone();
                        p.push(step(&rel));
                        level.insert(
                            there,
                            AssocHit { unit: there, distance: p.len() as u32, revision: unit.head, path: p },
                        );
                    }
                }
            }
            frontier = level.values().map(|h| (h.unit, h.path.clone())).collect();
            hits.extend(level.into_values());
            if hits.len() > q.limit {
                hits.truncate(q.limit);
                return Ok(AssocResult { hits, ending: AssocEnding::LimitReached, visited: run.visited });
            }
            if frontier.is_empty() {
                break;
            }
        }
        Ok(AssocResult { hits, ending: AssocEnding::Exhausted, visited: run.visited })
    }
}

fn type_allowed(q: &AssocQuery, r: &Relation) -> bool {
    q.relation_types.as_ref().is_none_or(|t| t.contains(&r.relation_type))
}

fn step(r: &Relation) -> TraceStep {
    TraceStep {
        relation: r.id,
        relation_type: r.relation_type.clone(),
        from: r.from,
        to: r.to,
        created_seq: r.created_seq,
        provenance: r.provenance.kind(),
    }
}

struct Run<'a, E: Engine> {
    snap: &'a UnitSnapshot<E>,
    q: &'a AssocQuery,
    visited: u64,
    /// Actor and source of the change sets read so far, by commit sequence.
    actors: BTreeMap<CommitSeq, Provenance>,
}

impl<E: Engine> Run<'_, E> {
    /// Whether the window and the provenance filter let `r` be followed.
    fn followed(&mut self, r: &Relation) -> Result<bool, AssocError> {
        if self.q.window.is_some_and(|w| r.created_seq < w.lo || r.created_seq > w.hi) {
            return Ok(false);
        }
        let f = &self.q.provenance;
        if f.kinds.as_ref().is_some_and(|k| !k.contains(&r.provenance.kind())) {
            return Ok(false);
        }
        if f.actor.is_none() && f.source.is_none() {
            return Ok(true);
        }
        let by = self.change_set_provenance(r)?;
        Ok(f.actor.as_ref().is_none_or(|a| by.actor.as_ref() == Some(a))
            && f.source.as_ref().is_none_or(|s| by.source.as_ref() == Some(s)))
    }

    /// The actor and source of the change set that created `r`.
    fn change_set_provenance(&mut self, r: &Relation) -> Result<&Provenance, AssocError> {
        if !self.actors.contains_key(&r.created_seq) {
            let Some(cs) = self.snap.change_set(r.created_seq)? else {
                return Err(corruption(format!(
                    "relation {}: its change set {} has no CHANGESET record",
                    r.id, r.created_seq
                ))
                .into());
            };
            self.actors.insert(r.created_seq, cs.provenance);
        }
        Ok(&self.actors[&r.created_seq])
    }

    /// The result when the budget ends inside a level: what was settled
    /// before it plus what the level found so far, cut at the limit.
    fn finish_partial(&self, mut hits: Vec<AssocHit>, level: BTreeMap<UnitId, AssocHit>) -> AssocResult {
        hits.extend(level.into_values());
        hits.truncate(self.q.limit);
        AssocResult { hits, ending: AssocEnding::BudgetExhausted, visited: self.visited }
    }
}
