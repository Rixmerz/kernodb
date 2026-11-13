//! E5-02: the deterministic planner (etapa5-acta.md §3, spec §9).
//!
//! The first planner is the seven steps of spec §9 over the paths that
//! already exist (acta §2.2), and `query` and `query_relations` are those
//! steps:
//!
//! | Step | Where |
//! |---|---|
//! | 1. Analyze and validate the query | Every [`UnitFilter`] and [`RelationQuery`] is valid: a condition no stored value can satisfy (an empty or over-long type) is a legitimate query with an empty answer, which both paths already give. The analysis is the typed shape of [`NormalizedQuery`] (E5-01). |
//! | 2. Generate the valid candidate plans | [`PlanPath::candidates_of`], each marked usable or not (I-23) |
//! | 3. Estimate cost with the statistics at hand | [`costs`]: the index state and the entry count of `IDX_META`, and a bounded probe of the range the path would read |
//! | 4. Choose | [`Decision::new`](crate::plan): the cheapest usable, the first in table order on a tie |
//! | 5. Execute | the chosen path, through the same code the explicit calls use |
//! | 6. Record metrics | [`QueryOutcome`] and [`RelationsOutcome`] carry the path and the fallback; `query` hands them to the [`Observer`](crate::observation::Observer) (E5-03) |
//! | 7. Validate and fall back | the answer is checked against the query and, if the index path failed or the check fails, the **reference path answers** and the outcome says so ([`Fallback`]) |
//!
//! Associative queries (E7-02) go through the same steps with two candidates,
//! [`PlanPath::AssocReference`] and [`PlanPath::AssocAdjacency`]; see
//! [`UnitSnapshot::assoc_query`].
//!
//! # Determinism
//!
//! The decision is a pure function of the query and of what the snapshot
//! holds: the same filter on the same snapshot gives the same candidates, the
//! same costs and the same choice, on any engine and after a reopen. Nothing
//! in it reads a clock, a random source or the handle. [`UnitSnapshot::explain_filter`]
//! and [`UnitSnapshot::explain_relations`] return the [`Plan`] that `query`
//! and `query_relations` follow on that snapshot.
//!
//! # Cost
//!
//! A cost is the number of key-value reads a path is expected to make, so
//! costs are comparable within one plan only. With `N` the entries of the
//! stored index generation (`IDX_META`), and a verified open of a unit being
//! [`OPEN_READS`] reads (`load_unit`: head, revision, payload):
//!
//! | Path | Cost |
//! |---|---|
//! | `UnitsReference` | `N × (1 + OPEN_READS)`: reads every head and opens every unit |
//! | `IndexPrimary` | `N + OPEN_READS × m` |
//! | `IndexType` | `(1 + OPEN_READS) × m` |
//! | `RelationsReference` | `r`: every `REL` record |
//! | `Adjacency` | `2 × a`: the entry and the `REL` record of each match |
//!
//! `m` is the number of units the filter keeps. For `IndexType` it is
//! **probed**: at most [`PROBE`] entries of the type range are read, which is
//! exact when the range is shorter. Otherwise, and for a filter that sets
//! only `schema_version`, it is not known; the planner assumes half of `N`
//! for a filter that sets a condition and `N` for one that sets none. `r` and
//! `a` are probed the same way and are lower bounds when the probe is full.
//! No statistic is stored for any of this: the index state, its entry count
//! and the key ranges are what the layer already has.
//!
//! Two consequences worth knowing. A filter without conditions costs the same
//! on the reference path and through `IDX_PRIMARY` (both read and open every
//! unit), so the tie goes to the reference. And an empty or unbuilt index has
//! `N = 0`, so every cost is zero and the reference answers, which is the
//! only usable path anyway.
//!
//! ponytail: the `N / 2` prior and the probe limit are fixed numbers, not
//! learned; W1 (1 % selective type filter) is decided by the probe alone.
//! Upgrade path: a per-type entry count in `IDX_META`, written by the build,
//! once a workload shows the prior choosing the slower path.

use std::collections::BTreeSet;
use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot};

use std::time::Duration;

use crate::assoc::{AssocError, AssocQuery, AssocResult};
use crate::cache::{CacheEntry, CacheFallbackCause, CachedQuery, CachedResult, Dependencies, Evidence, Lookup};
use crate::codec::{Adjacency, Key, TAG_REL, TAG_REL_IN, TAG_REL_OUT};
use crate::db::{Relation, Unit, UnitSnapshot, context, corruption, key};
use crate::error::UnitError;
use crate::fingerprint::{FingerprintKey, NormalizedQuery, QueryKind, RelationQuery};
use crate::index::{IndexPath, IndexState, IndexStatus, QueryOutcome, QuerySource, UnitFilter};
use crate::observation::{Ended, ErrorKind, Observed};
use crate::plan::{Candidate, Cost, Decision, Plan, PlanError, PlanPath, Unusable, Usability};
use crate::types::{Direction, Lifecycle, UnitId};

/// Version of the planner, recorded in every observation (E5-03). Bump it
/// when the candidates, the cost model or the choice change.
pub const PLANNER_VERSION: u32 = 1;

/// Entries a probe reads at most.
pub const PROBE: usize = 64;

/// Reads of a verified open of a unit (`load_unit`): head, revision, payload.
pub const OPEN_READS: u64 = 3;

/// How many entries a key range holds, as far as a probe saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    found: u64,
    /// The range ended before the probe limit: `found` is its size.
    complete: bool,
}

impl Probe {
    #[cfg(test)]
    pub(crate) fn new(found: u64, complete: bool) -> Self {
        Probe { found, complete }
    }
}

/// Why a chosen path did not answer and the reference path did (spec §9,
/// step 7). It holds no text from the query or from the error: only the
/// class of what happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fallback {
    /// The path that was chosen and abandoned.
    pub abandoned: PlanPath,
    pub cause: FallbackCause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackCause {
    /// The path reported damage (`Storage(Corruption)`), such as an index
    /// entry that disagrees with its record. The reference path decides: if
    /// the damage is in the data itself, it fails too and the error is
    /// returned.
    Corrupt,
    /// The path answered something that is not an answer to the query.
    Invalid(Violation),
    /// The path was not usable when the query was planned (the adjacency index
    /// of an associative query is absent or inconsistent): the reference path
    /// answered without trying it.
    Unusable(Unusable),
}

/// What the validation of an answer found wrong (step 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Violation {
    /// An item the query does not select.
    NotMatching,
    /// A unit that is not `Active`.
    NotActive,
    /// Items not in the strictly ascending order the reference path gives.
    OutOfOrder,
    /// A relation created after the snapshot: no index entry of this snapshot
    /// can point at it (associative queries).
    BeyondSnapshot,
    /// An adjacency entry whose mirror entry, on the other end of the
    /// relation, is missing or disagrees (associative queries).
    Unmirrored,
}

/// The relations a query matched and the path that found them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationsOutcome {
    pub relations: Vec<Relation>,
    /// The path that produced `relations`.
    pub path: PlanPath,
    /// Set when the chosen path failed and the reference path answered.
    pub fallback: Option<Fallback>,
}

/// The answer of an associative query, the path that produced it and, if the
/// adjacency path was abandoned, why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssocOutcome {
    pub result: AssocResult,
    /// The path that produced `result`.
    pub path: PlanPath,
    /// Set when the adjacency path was not usable or failed its check and the
    /// reference path answered.
    pub fallback: Option<Fallback>,
}

/// What the planner read to decide an associative query: the size of the
/// record range and of both adjacency ranges, and the first hop.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AssocProbes {
    all: Probe,
    out: Probe,
    inn: Probe,
    /// Expansions of the first hop: seeds times followed directions.
    expansions: u64,
    /// Adjacency entries the first hop would read.
    adjacent: u64,
}

impl AssocProbes {
    #[cfg(test)]
    pub(crate) fn new(all: Probe, out: Probe, inn: Probe, expansions: u64, adjacent: u64) -> Self {
        AssocProbes { all, out, inn, expansions, adjacent }
    }
}

/// Why the adjacency index cannot answer, from the three ranges the planner
/// probed. A healthy store writes a relation's record and both entries in one
/// change set, so the three have the same number of entries. A probe sees at
/// most [`PROBE`] entries, so this finds what a probe can show: a complete
/// range is the exact count and bounds the others from above.
pub(crate) fn adjacency_unusable(all: Probe, out: Probe, inn: Probe) -> Option<Unusable> {
    if all.found > 0 && (out.found == 0 || inn.found == 0) {
        return Some(Unusable::AdjacencyAbsent);
    }
    let ranges = [all, out, inn];
    let disagree = ranges.iter().filter(|x| x.complete).any(|x| ranges.iter().any(|y| y.found > x.found));
    disagree.then_some(Unusable::AdjacencyInconsistent)
}

/// The associative candidates. The cost is that of the first hop, in
/// key-value reads: the reference path reads every `REL` record for each
/// expansion, the adjacency path the entry and the record of each neighbour.
/// Deeper hops repeat the same trade, so the first hop decides for all.
pub(crate) fn assoc_candidates(p: &AssocProbes) -> Vec<Candidate> {
    let adjacency = match adjacency_unusable(p.all, p.out, p.inn) {
        Some(why) => Usability::Unusable(why),
        None => Usability::Usable,
    };
    vec![
        Candidate {
            path: PlanPath::AssocReference,
            cost: Cost(p.all.found.saturating_mul(p.expansions)),
            usability: Usability::Usable,
        },
        Candidate { path: PlanPath::AssocAdjacency, cost: Cost(p.adjacent.saturating_mul(2)), usability: adjacency },
    ]
}

/// Costs of the unit candidates, in table order (reference, primary, type).
/// `type_probe` is the probe of the type range, taken only when it can be
/// used.
pub(crate) fn costs(filter: &UnitFilter, entries: u64, type_probe: Option<Probe>) -> [Cost; 3] {
    let conditions = filter.unit_type.is_some() || filter.schema_version.is_some();
    let prior = if conditions { entries / 2 } else { entries };
    let kept = match type_probe {
        Some(p) if p.complete => p.found,
        Some(p) => prior.max(p.found),
        None => prior,
    }
    .min(entries);
    [
        Cost(entries.saturating_mul(1 + OPEN_READS)),
        Cost(entries.saturating_add(OPEN_READS.saturating_mul(kept))),
        Cost((1 + OPEN_READS).saturating_mul(kept)),
    ]
}

/// The unit candidates of a filter, with the usability the index state and
/// the filter give them (I-23).
pub(crate) fn unit_candidates(
    filter: &UnitFilter,
    state: IndexState,
    entries: u64,
    type_probe: Option<Probe>,
) -> Vec<Candidate> {
    let [reference, primary, by_type] = costs(filter, entries, type_probe);
    let index = match state {
        IndexState::Ready => Usability::Usable,
        other => Usability::Unusable(Unusable::IndexNotReady(other)),
    };
    let type_usability = match (state, filter.unit_type.is_some()) {
        (IndexState::Ready, false) => Usability::Unusable(Unusable::NoTypeFilter),
        _ => index,
    };
    vec![
        Candidate { path: PlanPath::UnitsReference, cost: reference, usability: Usability::Usable },
        Candidate { path: PlanPath::IndexPrimary, cost: primary, usability: index },
        Candidate { path: PlanPath::IndexType, cost: by_type, usability: type_usability },
    ]
}

/// The relation candidates, from the probes of the whole `REL` range and of
/// the adjacency range of the query.
pub(crate) fn relation_candidates(all: Probe, adjacent: Probe) -> Vec<Candidate> {
    vec![
        Candidate { path: PlanPath::RelationsReference, cost: Cost(all.found), usability: Usability::Usable },
        Candidate {
            path: PlanPath::Adjacency,
            cost: Cost(adjacent.found.saturating_mul(2)),
            usability: Usability::Usable,
        },
    ]
}

/// Step 7 for units: the answer selects only what the filter selects, only
/// active units, in the strictly ascending id order of the reference path.
pub(crate) fn check_units(filter: &UnitFilter, units: &[Unit]) -> Result<(), Violation> {
    for unit in units {
        if unit.lifecycle != Lifecycle::Active {
            return Err(Violation::NotActive);
        }
        if !filter.matches(&unit.unit_type, unit.schema_version) {
            return Err(Violation::NotMatching);
        }
    }
    if units.windows(2).any(|w| w[0].id >= w[1].id) {
        return Err(Violation::OutOfOrder);
    }
    Ok(())
}

/// Step 7 for relations: only relations the query selects, in the strictly
/// ascending (relation type, id) order of the reference path.
pub(crate) fn check_relations(query: &RelationQuery, relations: &[Relation]) -> Result<(), Violation> {
    for r in relations {
        let here = match query.direction {
            Direction::Out => r.from,
            Direction::In => r.to,
        };
        if here != query.unit || query.relation_type.as_deref().is_some_and(|t| t != r.relation_type) {
            return Err(Violation::NotMatching);
        }
    }
    let order = |r: &Relation| (r.relation_type.clone().into_bytes(), r.id);
    if relations.windows(2).any(|w| order(&w[0]) >= order(&w[1])) {
        return Err(Violation::OutOfOrder);
    }
    Ok(())
}

fn internal(e: PlanError) -> UnitError {
    // The candidates above are built to satisfy `Decision::new`; reaching
    // this is a planner bug, reported as an error rather than a panic.
    UnitError::InvalidArgument(format!("the planner built a plan it cannot explain: {e:?}"))
}

/// Whether `e` is damage a different path may not share.
fn is_corruption(e: &UnitError) -> bool {
    matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_)))
}

impl<E: Engine> UnitSnapshot<E> {
    /// Entries in `[start, end)`, counting at most [`PROBE`].
    fn probe(&self, start: &[u8], end: &[u8]) -> Result<Probe, UnitError> {
        let n = self.snap.scan(Bound::Included(start), Bound::Excluded(end), PROBE)?.len();
        Ok(Probe { found: n as u64, complete: n < PROBE })
    }

    /// Steps 1 to 4 for a unit filter: the status the decision saw and the
    /// decision.
    fn decide_units(&self, filter: &UnitFilter) -> Result<(IndexStatus, Decision), UnitError> {
        let (status, entries) = self.index_stats()?;
        self.decide_units_with(filter, status, entries)
    }

    /// [`decide_units`](Self::decide_units) over the status and entry count of
    /// an `IDX_META` read the caller already made.
    fn decide_units_with(
        &self,
        filter: &UnitFilter,
        status: IndexStatus,
        entries: u64,
    ) -> Result<(IndexStatus, Decision), UnitError> {
        // The type range is only worth reading when the type index can answer.
        let type_probe = match (status.state, filter.unit_type.as_deref()) {
            (IndexState::Ready, Some(t)) => Some(match Key::type_range(t, filter.schema_version) {
                Some((start, end)) => self.probe(&start, &end)?,
                None => Probe { found: 0, complete: true },
            }),
            _ => None,
        };
        let candidates = unit_candidates(filter, status.state, entries, type_probe);
        let decision = Decision::new(QueryKind::Units, Some(status), candidates).map_err(internal)?;
        Ok((status, decision))
    }

    /// Steps 1 to 4 for a relation query.
    fn decide_relations(&self, query: &RelationQuery) -> Result<Decision, UnitError> {
        let (start, end) = Key::tag_range(TAG_REL);
        let all = self.probe(&start, &end)?;
        let tag = match query.direction {
            Direction::Out => TAG_REL_OUT,
            Direction::In => TAG_REL_IN,
        };
        let adjacent = match Key::adjacency_range(tag, query.unit, query.relation_type.as_deref()) {
            Some((start, end)) => self.probe(&start, &end)?,
            None => Probe { found: 0, complete: true },
        };
        Decision::new(QueryKind::Relations, None, relation_candidates(all, adjacent)).map_err(internal)
    }

    /// Steps 1 to 4 for an associative query. The probes are the ones of
    /// `decide_relations` plus the two whole adjacency ranges, and the first
    /// hop of every seed.
    fn decide_assoc(&self, q: &AssocQuery, seeds: &BTreeSet<UnitId>) -> Result<Decision, UnitError> {
        let whole = |tag| {
            let (start, end) = Key::tag_range(tag);
            self.probe(&start, &end)
        };
        let (all, out, inn) = (whole(TAG_REL)?, whole(TAG_REL_OUT)?, whole(TAG_REL_IN)?);
        let (mut expansions, mut adjacent) = (0u64, 0u64);
        for &seed in seeds {
            for dir in q.direction.directions() {
                let tag = if *dir == Direction::Out { TAG_REL_OUT } else { TAG_REL_IN };
                expansions += 1;
                if let Some((start, end)) = Key::adjacency_range(tag, seed, None) {
                    adjacent = adjacent.saturating_add(self.probe(&start, &end)?.found);
                }
            }
        }
        let probes = AssocProbes { all, out, inn, expansions, adjacent };
        Decision::new(QueryKind::Associative, None, assoc_candidates(&probes)).map_err(internal)
    }

    /// The plan `assoc_query` follows for `q` on this snapshot: both
    /// candidates with their cost, the one chosen and why. A query that would
    /// be rejected has no plan. The plan holds the fingerprint of the shape of
    /// the query under `key`, never a seed, a type, an actor, a source or a
    /// number of the query.
    pub fn explain_assoc(&self, key: &FingerprintKey, q: &AssocQuery) -> Result<Plan, AssocError> {
        q.validate()?;
        let seeds: BTreeSet<UnitId> = q.seeds.iter().copied().collect();
        let decision = self.decide_assoc(q, &seeds)?;
        Ok(Plan::of(NormalizedQuery::of_assoc(q).fingerprint(key), decision))
    }

    /// The plan `query` follows for `filter` on this snapshot, to inspect:
    /// every candidate with its cost and usability, the one chosen, why, and
    /// the index freshness the decision saw (I-23). The plan holds the
    /// fingerprint of the query under `key`, never the filter.
    pub fn explain_filter(&self, key: &FingerprintKey, filter: &UnitFilter) -> Result<Plan, UnitError> {
        let (_, decision) = self.decide_units(filter)?;
        Ok(Plan::of(NormalizedQuery::of_filter(filter).fingerprint(key), decision))
    }

    /// The plan `query_relations` follows for `query` on this snapshot.
    pub fn explain_relations(&self, key: &FingerprintKey, query: &RelationQuery) -> Result<Plan, UnitError> {
        let decision = self.decide_relations(query)?;
        Ok(Plan::of(NormalizedQuery::of_relations(query).fingerprint(key), decision))
    }

    /// Every active unit matching `filter`, in unit id order, through the path
    /// the planner chooses (see the [module documentation](self)): the
    /// cheapest usable, which is never an index that is not `Ready` at this
    /// snapshot (I-23). The answer is the reference path's on the same
    /// snapshot (I-32). The outcome says which path answered, and whether it
    /// was the reference after the chosen one failed its check.
    pub fn query(&self, filter: &UnitFilter) -> Result<QueryOutcome, UnitError> {
        let started = self.observer.begin();
        let outcome = self.answer_units(filter);
        if let Some(started) = started {
            let ended = outcome
                .as_ref()
                .map(|o| Ended { path: o.path, rows: o.units.len(), fallback: o.fallback })
                .map_err(ErrorKind::of);
            self.observer.finish(started, Observed::Units(filter), self.seq(), ended);
        }
        outcome
    }

    /// Steps 1 to 7 for units without learned state: what `query` was before
    /// the result cache existed.
    fn plan_units(&self, filter: &UnitFilter) -> Result<QueryOutcome, UnitError> {
        let (status, entries) = self.index_stats()?;
        let (status, decision) = self.decide_units_with(filter, status, entries)?;
        self.execute_units(filter, status, decision.chosen().path, |via, f| self.scan_path(via, f, entries))
    }

    /// `query` with the result cache as the first thing considered (E6-02).
    ///
    /// * Cache off (the default): [`plan_units`](Self::plan_units), after one
    ///   atomic load.
    /// * On: the entry for this very query is served only if every dependency
    ///   it recorded matches the ones in force **now** (`Dependencies`, checked
    ///   by the lookup on every use, G4) **and** its rows pass `check_units`
    ///   like any planned path's (G5). A failed check drops the entry and the
    ///   reference path answers (G2). Any other miss, or an invalidation, runs
    ///   the planner and stores its answer under the dependencies of this
    ///   snapshot.
    ///
    /// Nothing here reads an [`Observer`](crate::observation::Observer): what
    /// was observed never decides whether an entry may be used (G3).
    fn answer_units(&self, filter: &UnitFilter) -> Result<QueryOutcome, UnitError> {
        if !self.cache.enabled() {
            return self.plan_units(filter);
        }
        let query = CachedQuery::Units(filter.clone());
        let Some(fingerprint) = self.cache.fingerprint_of(&query) else {
            return self.plan_units(filter);
        };
        let (status, entries) = match self.index_stats() {
            Ok(read) => read,
            Err(_) => {
                // A precondition of the lookup (a readable, consistent
                // `IDX_META`) does not hold: the planned path runs and, being
                // unable to read it too, returns the same error as without the
                // cache.
                self.cache.fallback(&fingerprint, CacheFallbackCause::PreconditionFailed);
                return self.plan_units(filter);
            }
        };
        let now = Dependencies::in_force(self.seq(), Some(status));
        let mut discrepancy = None;
        match self.cache.lookup(&fingerprint, &query, &now) {
            Lookup::Hit(result) => {
                let checked = match &*result {
                    CachedResult::Units(units) => check_units(filter, units).map(|()| units),
                    // Unreachable by construction (`CacheEntry::new`); not an answer all the same.
                    CachedResult::Relations(_) => Err(Violation::NotMatching),
                };
                match checked {
                    Ok(units) => {
                        return Ok(QueryOutcome {
                            units: units.clone(),
                            source: QuerySource::Cache { index: status },
                            path: PlanPath::ResultCache,
                            fallback: None,
                        });
                    }
                    Err(violation) => {
                        self.cache.fallback(&fingerprint, CacheFallbackCause::Discrepancy);
                        discrepancy = Some(Fallback {
                            abandoned: PlanPath::ResultCache,
                            cause: FallbackCause::Invalid(violation),
                        });
                    }
                }
            }
            Lookup::Invalidated(_) => self.cache.fallback(&fingerprint, CacheFallbackCause::Invalidated),
            Lookup::Disabled => self.cache.fallback(&fingerprint, CacheFallbackCause::Disabled),
            // Already on the record, with the cause, as `Transition::Missed`.
            Lookup::Miss | Lookup::Inactive => {}
        }
        let outcome = match discrepancy {
            Some(fallback) => QueryOutcome {
                units: self.reference_filter(filter)?,
                source: QuerySource::Reference { index: status },
                path: PlanPath::UnitsReference,
                fallback: Some(fallback),
            },
            None => {
                let (status, decision) = self.decide_units_with(filter, status, entries)?;
                self.execute_units(filter, status, decision.chosen().path, |via, f| self.scan_path(via, f, entries))?
            }
        };
        if self.cache.worth_storing(&fingerprint, &query, &now) {
            self.store(fingerprint, query, now, CachedResult::Units(outcome.units.clone()));
        }
        Ok(outcome)
    }

    /// Stores an answer the planned or the reference path just gave, under
    /// the dependencies of the snapshot it read. Failing to store (the cache
    /// was switched off meanwhile, or the result is over the byte bound) never
    /// fails the query: the cache is advisory.
    fn store(
        &self,
        fingerprint: crate::fingerprint::Fingerprint,
        query: CachedQuery,
        now: Dependencies,
        result: CachedResult,
    ) {
        // The execution that produced the answer is the evidence: one
        // observation, no span. The observer is not consulted (G3).
        let evidence = Evidence { observations: 1, window: Duration::ZERO };
        if let Ok(entry) = CacheEntry::build(fingerprint, query, now, result, evidence)
            && self.cache.store(entry).is_ok()
        {
            // Fails only if the cache was switched off since `create`; then
            // there is nothing to activate.
            let _ = self.cache.arm(&fingerprint);
        }
    }

    /// Steps 5 to 7 for units. `scan` runs an index path; it is a parameter so
    /// that a test can make it fail or answer wrongly.
    fn execute_units(
        &self,
        filter: &UnitFilter,
        status: IndexStatus,
        chosen: PlanPath,
        scan: impl FnOnce(IndexPath, &UnitFilter) -> Result<Vec<Unit>, UnitError>,
    ) -> Result<QueryOutcome, UnitError> {
        let via = match chosen {
            PlanPath::IndexPrimary => Some(IndexPath::Primary),
            PlanPath::IndexType => Some(IndexPath::ByType),
            _ => None,
        };
        let mut fallback = None;
        if let Some(via) = via {
            let watermark =
                status.watermark.ok_or_else(|| corruption("the index is Ready but has no watermark".into()))?;
            match scan(via, filter) {
                Ok(units) => match check_units(filter, &units) {
                    Ok(()) => {
                        return Ok(QueryOutcome {
                            units,
                            source: QuerySource::Index { watermark },
                            path: chosen,
                            fallback: None,
                        });
                    }
                    Err(v) => fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Invalid(v) }),
                },
                Err(e) if is_corruption(&e) => {
                    fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Corrupt });
                }
                Err(e) => return Err(e),
            }
        }
        Ok(QueryOutcome {
            units: self.reference_filter(filter)?,
            source: QuerySource::Reference { index: status },
            path: PlanPath::UnitsReference,
            fallback,
        })
    }

    /// The relations of one unit through the path the planner chooses: the
    /// adjacency index or the scan of every relation. The answer is
    /// [`UnitSnapshot::reference_relations_of`]'s on the same snapshot
    /// (I-32).
    pub fn query_relations(&self, query: &RelationQuery) -> Result<RelationsOutcome, UnitError> {
        let started = self.observer.begin();
        let outcome = self.answer_relations(query);
        if let Some(started) = started {
            let ended = outcome
                .as_ref()
                .map(|o| Ended { path: o.path, rows: o.relations.len(), fallback: o.fallback })
                .map_err(ErrorKind::of);
            self.observer.finish(started, Observed::Relations(query), self.seq(), ended);
        }
        outcome
    }

    /// Steps 1 to 7 for relations without learned state.
    fn plan_relations(&self, query: &RelationQuery) -> Result<RelationsOutcome, UnitError> {
        self.decide_relations(query).and_then(|decision| {
            self.execute_relations(query, decision.chosen().path, |q| {
                self.relations_of(q.unit, q.direction, q.relation_type.as_deref())
            })
        })
    }

    /// `query_relations` with the result cache first; see
    /// [`answer_units`](Self::answer_units). A relation query has no index, so
    /// the dependencies carry none (I-23) and the checks are
    /// `check_relations`.
    fn answer_relations(&self, query: &RelationQuery) -> Result<RelationsOutcome, UnitError> {
        if !self.cache.enabled() {
            return self.plan_relations(query);
        }
        let cached_query = CachedQuery::Relations(query.clone());
        let Some(fingerprint) = self.cache.fingerprint_of(&cached_query) else {
            return self.plan_relations(query);
        };
        let now = Dependencies::in_force(self.seq(), None);
        let mut discrepancy = None;
        match self.cache.lookup(&fingerprint, &cached_query, &now) {
            Lookup::Hit(result) => {
                let checked = match &*result {
                    CachedResult::Relations(relations) => check_relations(query, relations).map(|()| relations),
                    CachedResult::Units(_) => Err(Violation::NotMatching),
                };
                match checked {
                    Ok(relations) => {
                        return Ok(RelationsOutcome {
                            relations: relations.clone(),
                            path: PlanPath::ResultCache,
                            fallback: None,
                        });
                    }
                    Err(violation) => {
                        self.cache.fallback(&fingerprint, CacheFallbackCause::Discrepancy);
                        discrepancy = Some(Fallback {
                            abandoned: PlanPath::ResultCache,
                            cause: FallbackCause::Invalid(violation),
                        });
                    }
                }
            }
            Lookup::Invalidated(_) => self.cache.fallback(&fingerprint, CacheFallbackCause::Invalidated),
            Lookup::Disabled => self.cache.fallback(&fingerprint, CacheFallbackCause::Disabled),
            Lookup::Miss | Lookup::Inactive => {}
        }
        let outcome = match discrepancy {
            Some(fallback) => RelationsOutcome {
                relations: self.reference_relations_of(query.unit, query.direction, query.relation_type.as_deref())?,
                path: PlanPath::RelationsReference,
                fallback: Some(fallback),
            },
            None => self.plan_relations(query)?,
        };
        if self.cache.worth_storing(&fingerprint, &cached_query, &now) {
            self.store(fingerprint, cached_query, now, CachedResult::Relations(outcome.relations.clone()));
        }
        Ok(outcome)
    }

    /// An associative query through the path the planner chooses: the
    /// traversal over the adjacency index when it is usable and cheaper, else
    /// over the full scan of the relations. The answer is
    /// [`UnitSnapshot::reference_assoc_query`]'s on the same snapshot (I-32),
    /// in results, order, traces, visits and ending.
    ///
    /// Every hop read from the index is checked: only relations the hop asks
    /// for ([`check_relations`]), in the reference order, created no later
    /// than the snapshot, and with their mirror entry on the other end. If a
    /// check fails, or the index reports damage, the whole query is answered
    /// again by the reference path and the outcome says why ([`Fallback`]); so
    /// it does when the index was not usable at planning time. The seeds are
    /// checked before any path runs, and a query the model rejects never
    /// reaches the planner.
    ///
    /// The cache is not consulted (acta §2.3).
    pub fn assoc_query(&self, q: &AssocQuery) -> Result<AssocOutcome, AssocError> {
        let started = self.observer.begin();
        let outcome = self.plan_assoc(q);
        if let Some(started) = started {
            let ended = outcome
                .as_ref()
                .map(|o| Ended { path: o.path, rows: o.result.hits.len(), fallback: o.fallback })
                .map_err(AssocError::kind);
            self.observer.finish(started, Observed::Assoc(q), self.seq(), ended);
        }
        outcome
    }

    fn plan_assoc(&self, q: &AssocQuery) -> Result<AssocOutcome, AssocError> {
        let seeds = self.active_seeds(q)?;
        let decision = self.decide_assoc(q, &seeds)?;
        self.execute_assoc(q, seeds, &decision, |unit, dir, relation_type| self.relations_of(unit, dir, relation_type))
    }

    /// Steps 5 to 7 for an associative query; `hop` reads one range of the
    /// adjacency index.
    fn execute_assoc(
        &self,
        q: &AssocQuery,
        seeds: BTreeSet<UnitId>,
        decision: &Decision,
        hop: impl FnMut(UnitId, Direction, Option<&str>) -> Result<Vec<Relation>, UnitError>,
    ) -> Result<AssocOutcome, AssocError> {
        let abandoned = PlanPath::AssocAdjacency;
        let mut fallback = None;
        if decision.chosen().path == abandoned {
            match self.indexed_traverse(q, seeds.clone(), hop) {
                Ok(result) => return Ok(AssocOutcome { result, path: abandoned, fallback: None }),
                Err(AssocError::IndexAnswer(v)) => {
                    fallback = Some(Fallback { abandoned, cause: FallbackCause::Invalid(v) });
                }
                Err(AssocError::Unit(e)) if is_corruption(&e) => {
                    fallback = Some(Fallback { abandoned, cause: FallbackCause::Corrupt });
                }
                Err(e) => return Err(e),
            }
        } else if let Some(Candidate { usability: Usability::Unusable(why), .. }) =
            decision.candidates().iter().find(|c| c.path == abandoned)
        {
            fallback = Some(Fallback { abandoned, cause: FallbackCause::Unusable(*why) });
        }
        Ok(AssocOutcome { result: self.reference_traverse(q, seeds)?, path: PlanPath::AssocReference, fallback })
    }

    /// Step 7 for one hop of the adjacency path: [`check_relations`] (only the
    /// relations asked for, in order), then, relation by relation, that none
    /// is newer than the snapshot and that the mirror entry on the other end
    /// exists and names this end. `relations_of` has already compared each
    /// entry with its `REL` record, so an entry without a record or in
    /// disagreement with it is damage, not a wrong answer.
    ///
    /// Not a completeness check (H-1): an entry that is missing on both ends
    /// of a relation leaves nothing to compare. The planner's probes show it
    /// for small stores ([`adjacency_unusable`]), `verify_units` for all.
    pub(crate) fn check_adjacent(&self, query: &RelationQuery, relations: &[Relation]) -> Result<(), AssocError> {
        check_relations(query, relations).map_err(AssocError::IndexAnswer)?;
        for r in relations {
            if r.created_seq > self.seq() {
                return Err(AssocError::IndexAnswer(Violation::BeyondSnapshot));
            }
            if !self.is_mirrored(query.direction, r)? {
                return Err(AssocError::IndexAnswer(Violation::Unmirrored));
            }
        }
        Ok(())
    }

    /// Whether every relation of an adjacency answer has its mirror entry,
    /// one lookup per relation (K1). A relation that lost the entry on the
    /// other end is damage the answer still shows from this end. The end
    /// that lost its own entry has nothing to traverse and stays invisible
    /// here; `verify_units` sees it.
    fn all_mirrored(&self, query: &RelationQuery, relations: &[Relation]) -> Result<bool, UnitError> {
        for r in relations {
            if !self.is_mirrored(query.direction, r)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether the entry of `r` on the other end from `dir` exists and names
    /// this end.
    fn is_mirrored(&self, dir: Direction, r: &Relation) -> Result<bool, UnitError> {
        let (mirror, names) = match dir {
            Direction::Out => {
                (Key::RelationIn { unit: r.to, relation_type: r.relation_type.clone(), edge: r.id }, r.from)
            }
            Direction::In => {
                (Key::RelationOut { unit: r.from, relation_type: r.relation_type.clone(), edge: r.id }, r.to)
            }
        };
        let Some(raw) = self.snap.get(&key(mirror)?)? else { return Ok(false) };
        let entry = Adjacency::decode(&raw).map_err(|e| context(e, &format!("mirror adjacency entry of {}", r.id)))?;
        Ok(entry.other == names)
    }

    /// Steps 5 to 7 for relations; `adjacency` runs the index path.
    fn execute_relations(
        &self,
        query: &RelationQuery,
        chosen: PlanPath,
        adjacency: impl FnOnce(&RelationQuery) -> Result<Vec<Relation>, UnitError>,
    ) -> Result<RelationsOutcome, UnitError> {
        let mut fallback = None;
        if chosen == PlanPath::Adjacency {
            match adjacency(query) {
                Ok(relations) => match check_relations(query, &relations) {
                    Ok(()) => match self.all_mirrored(query, &relations) {
                        Ok(true) => return Ok(RelationsOutcome { relations, path: chosen, fallback: None }),
                        Ok(false) => {
                            fallback = Some(Fallback {
                                abandoned: chosen,
                                cause: FallbackCause::Invalid(Violation::Unmirrored),
                            });
                        }
                        Err(e) if is_corruption(&e) => {
                            fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Corrupt });
                        }
                        Err(e) => return Err(e),
                    },
                    Err(v) => fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Invalid(v) }),
                },
                Err(e) if is_corruption(&e) => {
                    fallback = Some(Fallback { abandoned: chosen, cause: FallbackCause::Corrupt });
                }
                Err(e) => return Err(e),
            }
        }
        Ok(RelationsOutcome {
            relations: self.reference_relations_of(query.unit, query.direction, query.relation_type.as_deref())?,
            path: PlanPath::RelationsReference,
            fallback,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AssocQuery;
    use crate::types::{CommitSeq, RelationId, RelationProvenance, Timestamp, UnitId};
    use crate::{ContentHash, UnitDb};
    use kerno_testkit::emulated::Emulated;
    use kerno_testkit::reference::LogEngine;

    fn filter(t: Option<&str>, v: Option<u32>) -> UnitFilter {
        UnitFilter { unit_type: t.map(Into::into), schema_version: v }
    }

    fn unit(id: u8, t: &str, v: u32, lifecycle: Lifecycle) -> Unit {
        Unit {
            id: UnitId([id; 16]),
            unit_type: t.into(),
            schema_version: v,
            lifecycle,
            head: crate::RevNo(1),
            oldest: crate::RevNo(1),
            created_seq: CommitSeq(1),
            updated_seq: CommitSeq(1),
            created_at: Timestamp(0),
            updated_at: Timestamp(0),
            metadata: Vec::new(),
            payload: None,
            content_hash: ContentHash::compute(crate::HashAlg::Blake3, 0, b""),
        }
    }

    fn rel(id: u8, from: u8, to: u8, t: &str) -> Relation {
        Relation {
            id: RelationId([id; 16]),
            from: UnitId([from; 16]),
            to: UnitId([to; 16]),
            relation_type: t.into(),
            created_seq: CommitSeq(1),
            provenance: RelationProvenance::Declared,
        }
    }

    #[test]
    fn units_are_checked_against_the_filter_state_and_order() {
        let f = filter(Some("a"), None);
        let ok = [unit(1, "a", 1, Lifecycle::Active), unit(2, "a", 2, Lifecycle::Active)];
        assert_eq!(check_units(&f, &ok), Ok(()));
        assert_eq!(check_units(&f, &[]), Ok(()));
        let wrong_type = [unit(1, "a", 1, Lifecycle::Active), unit(2, "b", 1, Lifecycle::Active)];
        assert_eq!(check_units(&f, &wrong_type), Err(Violation::NotMatching));
        let wrong_version = [unit(1, "a", 2, Lifecycle::Active)];
        assert_eq!(check_units(&filter(Some("a"), Some(1)), &wrong_version), Err(Violation::NotMatching));
        let archived = [unit(1, "a", 1, Lifecycle::Archived)];
        assert_eq!(check_units(&f, &archived), Err(Violation::NotActive));
        let descending = [unit(2, "a", 1, Lifecycle::Active), unit(1, "a", 1, Lifecycle::Active)];
        assert_eq!(check_units(&f, &descending), Err(Violation::OutOfOrder));
        let repeated = [unit(1, "a", 1, Lifecycle::Active), unit(1, "a", 1, Lifecycle::Active)];
        assert_eq!(check_units(&f, &repeated), Err(Violation::OutOfOrder));
    }

    #[test]
    fn relations_are_checked_against_the_query_and_order() {
        let q = |dir, t: Option<&str>| RelationQuery {
            unit: UnitId([1; 16]),
            direction: dir,
            relation_type: t.map(Into::into),
        };
        let out = q(Direction::Out, None);
        assert_eq!(check_relations(&out, &[rel(1, 1, 2, "a"), rel(2, 1, 3, "b")]), Ok(()));
        // A relation that does not start at the unit, or only ends there.
        assert_eq!(check_relations(&out, &[rel(1, 2, 1, "a")]), Err(Violation::NotMatching));
        assert_eq!(check_relations(&q(Direction::In, None), &[rel(1, 2, 1, "a")]), Ok(()));
        assert_eq!(check_relations(&q(Direction::In, None), &[rel(1, 1, 2, "a")]), Err(Violation::NotMatching));
        assert_eq!(check_relations(&q(Direction::Out, Some("a")), &[rel(1, 1, 2, "b")]), Err(Violation::NotMatching));
        // Type first, then id.
        assert_eq!(check_relations(&out, &[rel(2, 1, 2, "a"), rel(1, 1, 2, "b")]), Ok(()));
        assert_eq!(check_relations(&out, &[rel(1, 1, 2, "b"), rel(2, 1, 2, "a")]), Err(Violation::OutOfOrder));
        assert_eq!(check_relations(&out, &[rel(2, 1, 2, "a"), rel(1, 1, 2, "a")]), Err(Violation::OutOfOrder));
    }

    #[test]
    fn costs_follow_the_documented_model() {
        let p = |found, complete| Some(Probe::new(found, complete));
        // Exact probe: the type range is what it is.
        assert_eq!(costs(&filter(Some("t"), None), 1000, p(10, true)), [Cost(4000), Cost(1030), Cost(40)]);
        // Full probe: at least half of N is assumed, never above N.
        assert_eq!(costs(&filter(Some("t"), None), 1000, p(64, false)), [Cost(4000), Cost(2500), Cost(2000)]);
        // The probe saw 64, more than half of N: it is the floor.
        assert_eq!(costs(&filter(Some("t"), None), 100, p(64, false)), [Cost(400), Cost(292), Cost(256)]);
        // ... and the count is capped by N, so everything ties.
        assert_eq!(costs(&filter(Some("t"), None), 10, p(64, false)), [Cost(40), Cost(40), Cost(40)]);
        // Only a schema version: the prior, no probe.
        assert_eq!(costs(&filter(None, Some(1)), 1000, None), [Cost(4000), Cost(2500), Cost(2000)]);
        // No condition: the primary index costs what the reference does.
        assert_eq!(costs(&filter(None, None), 1000, None), [Cost(4000), Cost(4000), Cost(4000)]);
        // An index that was never built.
        assert_eq!(costs(&filter(Some("t"), None), 0, None), [Cost(0), Cost(0), Cost(0)]);
    }

    #[test]
    fn candidates_carry_the_usability_the_state_gives() {
        let f = filter(Some("t"), None);
        let usable = |c: &[Candidate]| c.iter().map(|c| c.usability == Usability::Usable).collect::<Vec<_>>();
        let ready = unit_candidates(&f, IndexState::Ready, 100, Some(Probe::new(3, true)));
        assert_eq!(usable(&ready), [true, true, true]);
        let no_type = unit_candidates(&filter(None, None), IndexState::Ready, 100, None);
        assert_eq!(usable(&no_type), [true, true, false]);
        assert_eq!(no_type[2].usability, Usability::Unusable(Unusable::NoTypeFilter));
        for state in [IndexState::Stale, IndexState::Building, IndexState::Failed] {
            let c = unit_candidates(&f, state, 100, None);
            assert_eq!(usable(&c), [true, false, false], "{state:?}");
            assert_eq!(c[1].usability, Usability::Unusable(Unusable::IndexNotReady(state)));
        }
    }

    /// A database with three units and a built index, and its snapshot.
    fn built_snapshot() -> (UnitDb<Emulated<LogEngine>>, UnitSnapshot<Emulated<LogEngine>>, Vec<UnitId>) {
        use crate::{CounterIds, FixedClock, NewUnit, UnitConfig};
        use kerno_core::contract::Options;
        use kerno_testkit::vfs::SimVfs;
        use std::sync::Arc;

        let mut config =
            UnitConfig::with_sources(Arc::new(FixedClock::new(Timestamp(1))), Arc::new(CounterIds::new(1)));
        config.engine = Options::small_for_tests();
        let vfs = SimVfs::new(1);
        let db = UnitDb::<Emulated<LogEngine>>::open(vfs.as_vfs(), std::path::Path::new("/db"), &config).unwrap();
        let mut cs = db.begin_changeset(crate::ChangeMeta::default()).unwrap();
        let ids = ["a", "b", "a"]
            .iter()
            .map(|t| {
                let new = NewUnit { unit_type: (*t).into(), schema_version: 1, metadata: Vec::new(), payload: vec![1] };
                cs.create(new).unwrap()
            })
            .collect();
        cs.commit().unwrap();
        db.rebuild_index().unwrap();
        let snap = db.snapshot().unwrap();
        (db, snap, ids)
    }

    fn corruption_error() -> UnitError {
        UnitError::Storage(kerno_core::Error::Corruption("test".into()))
    }

    #[test]
    fn a_failing_or_wrong_index_path_falls_back_to_the_reference() {
        let (_db, snap, _) = built_snapshot();
        let (status, _) = snap.index_stats().unwrap();
        let f = filter(Some("a"), None);
        let reference = snap.reference_filter(&f).unwrap();
        assert_eq!(reference.len(), 2);
        let run = |scan: &dyn Fn() -> Result<Vec<Unit>, UnitError>| {
            snap.execute_units(&f, status, PlanPath::IndexType, |_, _| scan())
        };

        // The index path answers well: no fallback, the index is the source.
        let ok = run(&|| Ok(reference.clone())).unwrap();
        assert_eq!((ok.path, ok.fallback), (PlanPath::IndexType, None));
        assert!(matches!(ok.source, QuerySource::Index { .. }));
        assert_eq!(ok.units, reference);

        // A wrong answer: one case per violation. The reference answers and
        // the outcome says which path was abandoned and why.
        let mut wrong_type = reference.clone();
        wrong_type[1] = snap.reference_filter(&filter(Some("b"), None)).unwrap().remove(0);
        let mut archived = reference.clone();
        archived[0].lifecycle = Lifecycle::Archived;
        let mut unordered = reference.clone();
        unordered.reverse();
        for (answer, violation) in
            [(wrong_type, Violation::NotMatching), (archived, Violation::NotActive), (unordered, Violation::OutOfOrder)]
        {
            let out = run(&|| Ok(answer.clone())).unwrap();
            assert_eq!(out.units, reference, "{violation:?}");
            assert_eq!(out.path, PlanPath::UnitsReference, "{violation:?}");
            assert_eq!(out.source, QuerySource::Reference { index: status }, "{violation:?}");
            assert_eq!(
                out.fallback,
                Some(Fallback { abandoned: PlanPath::IndexType, cause: FallbackCause::Invalid(violation) })
            );
        }

        // Damage found by the index path: the reference decides.
        let out = run(&|| Err(corruption_error())).unwrap();
        assert_eq!(out.units, reference);
        assert_eq!(out.fallback, Some(Fallback { abandoned: PlanPath::IndexType, cause: FallbackCause::Corrupt }));

        // An error that is not damage is not hidden by a second attempt.
        let busy = run(&|| Err(UnitError::InvalidArgument("not damage".into())));
        assert!(matches!(busy, Err(UnitError::InvalidArgument(_))), "{busy:?}");

        // The reference path chosen runs nothing else, so there is nothing to
        // fall back from.
        let direct = snap.execute_units(&f, status, PlanPath::UnitsReference, |_, _| panic!("no index path")).unwrap();
        assert_eq!((direct.path, direct.fallback), (PlanPath::UnitsReference, None));
    }

    #[test]
    fn a_failing_or_wrong_adjacency_falls_back_to_the_reference() {
        let (db, snap, ids) = built_snapshot();
        drop(snap);
        let mut cs = db.begin_changeset(crate::ChangeMeta::default()).unwrap();
        for (t, to) in [("x", ids[1]), ("y", ids[2])] {
            cs.create_relation(ids[0], to, t, RelationProvenance::Declared).unwrap();
        }
        cs.commit().unwrap();
        let snap = db.snapshot().unwrap();
        let q = RelationQuery { unit: ids[0], direction: Direction::Out, relation_type: None };
        let reference = snap.reference_relations_of(q.unit, q.direction, None).unwrap();
        assert_eq!(reference.len(), 2);
        let run = |adj: &dyn Fn() -> Result<Vec<Relation>, UnitError>| {
            snap.execute_relations(&q, PlanPath::Adjacency, |_| adj())
        };

        let ok = run(&|| Ok(reference.clone())).unwrap();
        assert_eq!((ok.path, ok.fallback, ok.relations.clone()), (PlanPath::Adjacency, None, reference.clone()));

        let mut foreign = reference.clone();
        foreign[0].from = ids[2];
        let mut unordered = reference.clone();
        unordered.reverse();
        for (answer, violation) in [(foreign, Violation::NotMatching), (unordered, Violation::OutOfOrder)] {
            let out = run(&|| Ok(answer.clone())).unwrap();
            assert_eq!(out.relations, reference, "{violation:?}");
            assert_eq!(out.path, PlanPath::RelationsReference, "{violation:?}");
            assert_eq!(
                out.fallback,
                Some(Fallback { abandoned: PlanPath::Adjacency, cause: FallbackCause::Invalid(violation) })
            );
        }
        let out = run(&|| Err(corruption_error())).unwrap();
        assert_eq!(out.relations, reference);
        assert_eq!(out.fallback, Some(Fallback { abandoned: PlanPath::Adjacency, cause: FallbackCause::Corrupt }));
        assert!(matches!(run(&|| Err(UnitError::InvalidArgument("x".into()))), Err(UnitError::InvalidArgument(_))));
    }

    #[test]
    fn the_adjacency_index_is_usable_only_when_the_probes_can_agree() {
        let p = |found, complete| Probe::new(found, complete);
        let ok = |all, out, inn| adjacency_unusable(all, out, inn);
        // Equal and complete, or all full: nothing to object to.
        assert_eq!(ok(p(5, true), p(5, true), p(5, true)), None);
        assert_eq!(ok(p(64, false), p(64, false), p(64, false)), None);
        assert_eq!(ok(p(0, true), p(0, true), p(0, true)), None);
        // A complete range bounds the others from above, whichever it is.
        assert_eq!(ok(p(5, true), p(4, true), p(5, true)), Some(Unusable::AdjacencyInconsistent));
        assert_eq!(ok(p(5, true), p(5, true), p(4, true)), Some(Unusable::AdjacencyInconsistent));
        assert_eq!(ok(p(4, true), p(5, true), p(5, true)), Some(Unusable::AdjacencyInconsistent));
        assert_eq!(ok(p(0, true), p(1, true), p(0, true)), Some(Unusable::AdjacencyInconsistent));
        assert_eq!(ok(p(5, true), p(64, false), p(5, true)), Some(Unusable::AdjacencyInconsistent));
        // A full probe is only a lower bound: fewer entries than it saw is not a disagreement.
        assert_eq!(ok(p(64, false), p(64, false), p(63, true)), Some(Unusable::AdjacencyInconsistent));
        assert_eq!(ok(p(64, false), p(64, false), p(64, false)), None);
        // Relations and no entry in a direction: the index is absent, not just different.
        assert_eq!(ok(p(3, true), p(0, true), p(3, true)), Some(Unusable::AdjacencyAbsent));
        assert_eq!(ok(p(64, false), p(64, false), p(0, true)), Some(Unusable::AdjacencyAbsent));
        // No relation, no entry: nothing is absent.
        assert_eq!(ok(p(0, true), p(0, true), p(0, true)), None);
    }

    #[test]
    fn assoc_costs_compare_the_first_hop() {
        let p = |found, complete| Probe::new(found, complete);
        let cost = |probes: AssocProbes| {
            let c = assoc_candidates(&probes);
            (c[0].path, c[1].path, c[0].cost, c[1].cost, c[1].usability)
        };
        // Two expansions of a 9-record scan against 4 entries read twice each.
        let probes = AssocProbes::new(p(9, true), p(9, true), p(9, true), 2, 4);
        assert_eq!(
            cost(probes),
            (PlanPath::AssocReference, PlanPath::AssocAdjacency, Cost(18), Cost(8), Usability::Usable)
        );
        // A tie goes to the reference path, first in table order.
        let tie = AssocProbes::new(p(4, true), p(4, true), p(4, true), 2, 4);
        let candidates = assoc_candidates(&tie);
        let decision = Decision::new(QueryKind::Associative, None, candidates).unwrap();
        assert_eq!(decision.chosen().path, PlanPath::AssocReference);
        // An index that is not usable is never chosen, however cheap.
        let broken = AssocProbes::new(p(9, true), p(8, true), p(9, true), 2, 0);
        let candidates = assoc_candidates(&broken);
        assert_eq!(candidates[1].cost, Cost(0));
        assert_eq!(candidates[1].usability, Usability::Unusable(Unusable::AdjacencyInconsistent));
        let decision = Decision::new(QueryKind::Associative, None, candidates).unwrap();
        assert_eq!(decision.chosen().path, PlanPath::AssocReference);
    }

    /// One hop of the adjacency path, as a test injects it.
    type HopAnswer<'a> = dyn Fn(UnitId, Direction, Option<&str>) -> Result<Vec<Relation>, UnitError> + 'a;

    /// Three units; `a` starts two relations, and four more between `b` and
    /// `c` make the scan costlier than the index for a query from `a`.
    fn assoc_snapshot() -> (UnitSnapshot<Emulated<LogEngine>>, Vec<UnitId>) {
        let (db, snap, ids) = built_snapshot();
        drop(snap);
        let mut cs = db.begin_changeset(crate::ChangeMeta::default()).unwrap();
        for (from, to, t) in [(0, 1, "x"), (0, 2, "y"), (1, 2, "x"), (1, 2, "x"), (2, 1, "y"), (2, 1, "y")] {
            cs.create_relation(ids[from], ids[to], t, RelationProvenance::Declared).unwrap();
        }
        cs.commit().unwrap();
        (db.snapshot().unwrap(), ids)
    }

    fn assoc_from(ids: &[UnitId]) -> (AssocQuery, BTreeSet<UnitId>) {
        let mut q = AssocQuery::new(vec![ids[0]], 3, 100, 1000);
        q.direction = crate::AssocDirection::Out;
        (q, BTreeSet::from([ids[0]]))
    }

    #[test]
    fn a_wrong_hop_of_the_adjacency_path_falls_back_to_the_reference() {
        let (snap, ids) = assoc_snapshot();
        let (q, seeds) = assoc_from(&ids);
        let decision = snap.decide_assoc(&q, &seeds).unwrap();
        assert_eq!(decision.chosen().path, PlanPath::AssocAdjacency, "{:?}", decision.candidates());
        let reference = snap.reference_assoc_query(&q).unwrap();
        assert_eq!(reference.hits.len(), 2);
        let run = |hop: &HopAnswer| snap.execute_assoc(&q, seeds.clone(), &decision, hop);
        let right = |u, d, t: Option<&str>| snap.relations_of(u, d, t);

        // A right answer: the index answers, no fallback.
        let ok = run(&right).unwrap();
        assert_eq!((ok.path, ok.fallback, &ok.result), (PlanPath::AssocAdjacency, None, &reference));

        let mine = snap.relations_of(ids[0], Direction::Out, None).unwrap();
        let foreign = snap.relations_of(ids[1], Direction::Out, None).unwrap();
        let mut unordered = mine.clone();
        unordered.reverse();
        let mut repeated = mine.clone();
        repeated.push(mine[1].clone());
        let mut future = mine.clone();
        future[0].created_seq = CommitSeq(snap.seq().0 + 1);
        let mut forged = mine.clone();
        forged[0].id = RelationId([0xEE; 16]);
        let mut extra = mine.clone();
        extra.extend(foreign);
        let cases: [(&str, Vec<Relation>, Violation); 5] = [
            ("a relation of another unit", extra, Violation::NotMatching),
            ("out of order", unordered, Violation::OutOfOrder),
            ("repeated", repeated, Violation::OutOfOrder),
            ("created after the snapshot", future, Violation::BeyondSnapshot),
            ("without its entries", forged, Violation::Unmirrored),
        ];
        for (name, answer, violation) in cases {
            let hop = |u, d, t: Option<&str>| match u == ids[0] {
                true => Ok(answer.clone()),
                false => snap.relations_of(u, d, t),
            };
            let out = run(&hop).unwrap();
            assert_eq!(out.result, reference, "{name}");
            assert_eq!(out.path, PlanPath::AssocReference, "{name}");
            assert_eq!(
                out.fallback,
                Some(Fallback { abandoned: PlanPath::AssocAdjacency, cause: FallbackCause::Invalid(violation) }),
                "{name}"
            );
        }

        // A query that follows one type reads one range per type: a hop that
        // answers with the other types too is not the answer it was asked for.
        let mut typed = q.clone();
        typed.relation_types = Some(BTreeSet::from(["x".to_owned()]));
        let typed_reference = snap.reference_assoc_query(&typed).unwrap();
        assert_eq!(typed_reference.hits.len(), 2);
        let widened = |u, d, _: Option<&str>| snap.relations_of(u, d, None);
        let out = snap.execute_assoc(&typed, seeds.clone(), &decision, widened).unwrap();
        assert_eq!(out.result, typed_reference);
        assert_eq!(
            out.fallback,
            Some(Fallback {
                abandoned: PlanPath::AssocAdjacency,
                cause: FallbackCause::Invalid(Violation::NotMatching)
            })
        );

        // Damage found by the index: the reference decides. An error that is not damage is not hidden.
        let out = run(&|_, _, _| Err(corruption_error())).unwrap();
        assert_eq!(out.result, reference);
        assert_eq!(out.fallback, Some(Fallback { abandoned: PlanPath::AssocAdjacency, cause: FallbackCause::Corrupt }));
        let busy = run(&|_, _, _| Err(UnitError::InvalidArgument("not damage".into())));
        assert!(matches!(busy, Err(AssocError::Unit(UnitError::InvalidArgument(_)))), "{busy:?}");
    }

    #[test]
    fn an_unusable_adjacency_path_is_not_run_and_the_fallback_says_why() {
        let (snap, ids) = assoc_snapshot();
        let (q, seeds) = assoc_from(&ids);
        let p = |found, complete| Probe::new(found, complete);
        let probes = AssocProbes::new(p(9, true), p(0, true), p(9, true), 1, 0);
        let decision = Decision::new(QueryKind::Associative, None, assoc_candidates(&probes)).unwrap();
        let out = snap.execute_assoc(&q, seeds, &decision, |_, _, _| panic!("an unusable index is not read")).unwrap();
        assert_eq!(out.result, snap.reference_assoc_query(&q).unwrap());
        assert_eq!(out.path, PlanPath::AssocReference);
        assert_eq!(
            out.fallback,
            Some(Fallback {
                abandoned: PlanPath::AssocAdjacency,
                cause: FallbackCause::Unusable(Unusable::AdjacencyAbsent)
            })
        );
    }

    #[test]
    fn relation_costs_compare_the_probes() {
        let c = relation_candidates(Probe::new(500, false), Probe::new(3, true));
        assert_eq!((c[0].cost, c[1].cost), (Cost(500), Cost(6)));
        let c = relation_candidates(Probe::new(3, true), Probe::new(3, true));
        assert_eq!((c[0].cost, c[1].cost), (Cost(3), Cost(6)));
    }
}
