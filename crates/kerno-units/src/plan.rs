//! E5-01: the plan type and its explanation (etapa5-acta.md §3).
//!
//! A [`Plan`] says how a query will be answered **and why**: every candidate
//! path that was considered with its estimated cost and whether it could be
//! used, the one chosen, the reason, and the freshness of the index the
//! decision saw (I-23). The plan *is* the explanation: there is no second
//! structure to keep in agreement with it.
//!
//! The candidates are the paths that already exist, nothing else (acta §2.2):
//!
//! | Query | Candidates, in tie-break order |
//! |---|---|
//! | [`QueryKind::Units`] | [`PlanPath::UnitsReference`], [`PlanPath::IndexPrimary`], [`PlanPath::IndexType`] |
//! | [`QueryKind::Relations`] | [`PlanPath::RelationsReference`], [`PlanPath::Adjacency`] |
//! | [`QueryKind::Associative`] | [`PlanPath::AssocReference`], [`PlanPath::AssocAdjacency`] |
//!
//! A plan holds the query's [`Fingerprint`], never the query: it carries no
//! literal and no shape text, so it can leave the process wherever a
//! fingerprint can. Filling candidates and costs is the planner's job
//! ([`crate::planner`], E5-02); this module only refuses a plan that cannot be
//! explained truthfully.
//!
//! # What [`Plan::new`] enforces
//!
//! * **All candidates, once, in order.** The list is exactly the table above
//!   for the fingerprint's kind: a path left out is a path the explanation
//!   would not account for.
//! * **A unit query records the index status; a relation query records none.**
//!   Adjacency entries are written by the change set that writes the relation
//!   (ADR-0014), so the index states of I-23 do not apply to them.
//! * **No stale index is declared usable (I-23).** While the status is not
//!   `Ready`, both index paths must be `Unusable(IndexNotReady(state))` with
//!   that very state; while it is `Ready`, only `IndexType` may be unusable,
//!   and only for lack of a `unit_type` in the filter.
//! * **The reference paths are always usable.** They are the fallback (I-32),
//!   so a plan always has a way to answer. The adjacency path of an
//!   associative query is the one other candidate that may be unusable, for
//!   [`Unusable::AdjacencyAbsent`] or [`Unusable::AdjacencyInconsistent`]: it
//!   has no stored state, so what the planner saw in the key ranges is the
//!   evidence (E7-02).
//! * **The choice is the cheapest usable candidate**, the first in table order
//!   on a tie, and the reason is derived from it, not given: `Fallback` when
//!   the index was not `Ready`, `LowestCost` otherwise.

use crate::fingerprint::{Fingerprint, QueryKind};
use crate::index::{IndexState, IndexStatus};

/// A path a query can be answered through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanPath {
    /// Scan every unit head (`reference_filter`).
    UnitsReference,
    /// `IDX_PRIMARY`: every active entry, filtered.
    IndexPrimary,
    /// `IDX_TYPE`: the range of the filter's type.
    IndexType,
    /// Read every `REL` record (`reference_relations_of`).
    RelationsReference,
    /// The adjacency range of the unit (`relations_of`).
    Adjacency,
    /// The breadth-first traversal over `reference_relations_of`
    /// (`reference_assoc_query`, E7-02).
    AssocReference,
    /// The same traversal taking each hop from the adjacency index, every
    /// answer checked (E7-02).
    AssocAdjacency,
    /// A cached result (E6-02): learned state, not a path over the data. It is
    /// the first thing `query` considers, before the candidates above, and it
    /// is chosen by an exact match of every dependency, not by cost, so it is
    /// not in [`candidates_of`](PlanPath::candidates_of) and a [`Plan`] never
    /// lists it: the answer's `path` is what says it was used (G7).
    ResultCache,
}

impl PlanPath {
    /// The candidates of a kind of query, in tie-break order. The
    /// [`ResultCache`](PlanPath::ResultCache) is not one of them.
    pub fn candidates_of(kind: QueryKind) -> &'static [PlanPath] {
        match kind {
            QueryKind::Units => &[PlanPath::UnitsReference, PlanPath::IndexPrimary, PlanPath::IndexType],
            QueryKind::Relations => &[PlanPath::RelationsReference, PlanPath::Adjacency],
            QueryKind::Associative => &[PlanPath::AssocReference, PlanPath::AssocAdjacency],
        }
    }

    fn is_unit_index(self) -> bool {
        matches!(self, PlanPath::IndexPrimary | PlanPath::IndexType)
    }
}

/// Estimated work of a path, in entries it would read. Only comparable within
/// one plan; how it is estimated is the planner's (E5-02).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cost(pub u64);

/// Why a candidate could not be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unusable {
    /// The index is in this state, not `Ready` (I-23).
    IndexNotReady(IndexState),
    /// `IDX_TYPE` answers only a filter that sets `unit_type`.
    NoTypeFilter,
    /// Relations exist but the adjacency entries of a direction do not: the
    /// index is absent (associative queries only).
    AdjacencyAbsent,
    /// The record range and the two adjacency ranges cannot all be complete
    /// and equal: an entry is missing or there is one without a record
    /// (associative queries only).
    AdjacencyInconsistent,
}

/// Whether the planner could answer through a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Usability {
    Usable,
    Unusable(Unusable),
}

/// A path that was considered, with its estimated cost. The cost of an
/// unusable candidate is what it would have cost, kept so the explanation
/// shows what was given up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Candidate {
    pub path: PlanPath,
    pub cost: Cost,
    pub usability: Usability,
}

/// Why the chosen candidate was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// The cheapest of the usable candidates.
    LowestCost,
    /// The index was in this state, not `Ready`, so the query fell back to the
    /// reference path (I-23).
    Fallback { index: IndexState },
    /// The adjacency path of an associative query could not be used (E7-02).
    AdjacencyUnusable(Unusable),
}

/// A plan that cannot be explained truthfully; see [`Plan::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// The candidates are not exactly those of the query kind, in order.
    Candidates { expected: Vec<PlanPath>, found: Vec<PlanPath> },
    /// A unit query without an index status, or a relation query with one.
    IndexStatus { kind: QueryKind },
    /// A reference or adjacency path marked unusable.
    FallbackUnusable { path: PlanPath },
    /// An index candidate whose usability contradicts the index status.
    Freshness { path: PlanPath, status: IndexStatus },
}

/// The validated choice among a query's candidates. It carries no
/// fingerprint, so the planner can decide (and `query` can run) without
/// hashing; a [`Plan`] is a decision plus the fingerprint of its query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Decision {
    index: Option<IndexStatus>,
    candidates: Vec<Candidate>,
    chosen: usize,
    reason: Reason,
}

impl Decision {
    /// The checks and the choice of [`Plan::new`], for a query of `kind`.
    pub(crate) fn new(
        kind: QueryKind,
        index: Option<IndexStatus>,
        candidates: Vec<Candidate>,
    ) -> Result<Self, PlanError> {
        let expected = PlanPath::candidates_of(kind);
        if !candidates.iter().map(|c| c.path).eq(expected.iter().copied()) {
            return Err(PlanError::Candidates {
                expected: expected.to_vec(),
                found: candidates.iter().map(|c| c.path).collect(),
            });
        }
        if index.is_some() != (kind == QueryKind::Units) {
            return Err(PlanError::IndexStatus { kind });
        }
        for c in &candidates {
            if !c.path.is_unit_index() {
                let may_be_unusable = c.path == PlanPath::AssocAdjacency
                    && matches!(
                        c.usability,
                        Usability::Unusable(Unusable::AdjacencyAbsent | Unusable::AdjacencyInconsistent)
                    );
                if c.usability != Usability::Usable && !may_be_unusable {
                    return Err(PlanError::FallbackUnusable { path: c.path });
                }
                continue;
            }
            // Index paths belong to unit queries, which carry a status (checked above).
            let status = index.ok_or(PlanError::IndexStatus { kind })?;
            let allowed = match status.state {
                IndexState::Ready => {
                    c.usability == Usability::Usable
                        || (c.path == PlanPath::IndexType && c.usability == Usability::Unusable(Unusable::NoTypeFilter))
                }
                state => c.usability == Usability::Unusable(Unusable::IndexNotReady(state)),
            };
            if !allowed {
                return Err(PlanError::Freshness { path: c.path, status });
            }
        }
        // `min_by_key` keeps the first of equal keys: the tie-break order.
        let chosen = candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| c.usability == Usability::Usable)
            .min_by_key(|(_, c)| c.cost)
            .map(|(i, _)| i)
            .expect("the reference path is usable: checked above");
        let adjacency = candidates.iter().find(|c| c.path == PlanPath::AssocAdjacency).map(|c| c.usability);
        let reason = match (index, adjacency) {
            (Some(IndexStatus { state, .. }), _) if state != IndexState::Ready => Reason::Fallback { index: state },
            (_, Some(Usability::Unusable(why))) => Reason::AdjacencyUnusable(why),
            _ => Reason::LowestCost,
        };
        Ok(Decision { index, candidates, chosen, reason })
    }

    pub(crate) fn chosen(&self) -> &Candidate {
        &self.candidates[self.chosen]
    }

    pub(crate) fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }
}

/// How a query will be answered and why; see the [module documentation](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    fingerprint: Fingerprint,
    decision: Decision,
}

impl Plan {
    /// Builds the plan for a query of `fingerprint`'s kind, seen with this
    /// `index` status (`None` for a relation query), choosing among
    /// `candidates`. See the [module documentation](self) for what is checked.
    pub fn new(
        fingerprint: Fingerprint,
        index: Option<IndexStatus>,
        candidates: Vec<Candidate>,
    ) -> Result<Self, PlanError> {
        Ok(Plan { fingerprint, decision: Decision::new(fingerprint.kind(), index, candidates)? })
    }

    pub(crate) fn of(fingerprint: Fingerprint, decision: Decision) -> Self {
        Plan { fingerprint, decision }
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// The index status the decision saw; `None` for a relation query.
    pub fn index(&self) -> Option<IndexStatus> {
        self.decision.index
    }

    /// Every candidate considered, in tie-break order.
    pub fn candidates(&self) -> &[Candidate] {
        &self.decision.candidates
    }

    /// The candidate chosen.
    pub fn chosen(&self) -> &Candidate {
        self.decision.chosen()
    }

    pub fn reason(&self) -> Reason {
        self.decision.reason
    }
}
