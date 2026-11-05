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
//! | 6. Record metrics | [`QueryOutcome`] and [`RelationsOutcome`] carry the path and the fallback; storing observations is E5-03 |
//! | 7. Validate and fall back | the answer is checked against the query and, if the index path failed or the check fails, the **reference path answers** and the outcome says so ([`Fallback`]) |
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

use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot};

use crate::codec::{Key, TAG_REL, TAG_REL_IN, TAG_REL_OUT};
use crate::db::{Relation, Unit, UnitSnapshot, corruption};
use crate::error::UnitError;
use crate::fingerprint::{FingerprintKey, NormalizedQuery, QueryKind, RelationQuery};
use crate::index::{IndexPath, IndexState, IndexStatus, QueryOutcome, QuerySource, UnitFilter};
use crate::plan::{Candidate, Cost, Decision, Plan, PlanError, PlanPath, Unusable, Usability};
use crate::types::{Direction, Lifecycle};

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
        let (status, decision) = self.decide_units(filter)?;
        self.execute_units(filter, status, decision.chosen().path, |via, f| self.scan_path(via, f))
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
        let decision = self.decide_relations(query)?;
        self.execute_relations(query, decision.chosen().path, |q| {
            self.relations_of(q.unit, q.direction, q.relation_type.as_deref())
        })
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
                    Ok(()) => return Ok(RelationsOutcome { relations, path: chosen, fallback: None }),
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
    fn relation_costs_compare_the_probes() {
        let c = relation_candidates(Probe::new(500, false), Probe::new(3, true));
        assert_eq!((c[0].cost, c[1].cost), (Cost(500), Cost(6)));
        let c = relation_candidates(Probe::new(3, true), Probe::new(3, true));
        assert_eq!((c[0].cost, c[1].cost), (Cost(3), Cost(6)));
    }
}
