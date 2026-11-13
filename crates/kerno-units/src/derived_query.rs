//! E9-09: reads through [`DerivedDb`] (FRICTION 1, 2 and 9).
//!
//! The derived layer guards its [`UnitDb`](crate::UnitDb), so a caller could
//! not list, query or read history through it. These methods answer from a
//! [`UnitSnapshot`], which only reads: the database itself is still not handed
//! out. They add no write and no state.

use crate::UnitSnapshot;
use crate::assoc_rank::{Scored, VectorError, VectorMetric, VectorQuery, VectorSet};
use crate::derived::DerivedId;
use crate::derived_stale::DerivedDb;
use crate::error::UnitError;
use crate::history::{HistoryQuery, Revision};
use crate::ids::PublicId;
use crate::index::{QueryOutcome, UnitFilter};
use crate::types::UnitId;
use kerno_core::contract::TxnEngine;

impl<E: TxnEngine> DerivedDb<E> {
    /// A read view of the units at the current commit sequence. Everything a
    /// [`UnitSnapshot`] answers (`get_unit`, `query`, `history`, `as_of`,
    /// relations) is read from one point.
    pub fn snapshot(&self) -> Result<UnitSnapshot<E>, UnitError> {
        self.db.snapshot()
    }

    /// [`UnitDb::public_id`](crate::UnitDb::public_id): the token issued for
    /// `unit` at the current commit, `None` for a unit without one.
    pub fn public_id(&self, unit: UnitId) -> Result<Option<PublicId>, UnitError> {
        self.db.public_id(unit)
    }

    /// [`UnitDb::resolve_public_id`](crate::UnitDb::resolve_public_id): the
    /// unit `token` names, `None` if it was never issued or its unit was
    /// deleted. A token grants nothing (I-27).
    pub fn resolve_public_id(&self, token: &PublicId) -> Result<Option<UnitId>, UnitError> {
        self.db.resolve_public_id(token)
    }

    /// Every active unit matching `filter`, through the planner's path (see
    /// [`UnitSnapshot::query`]).
    pub fn query(&self, filter: &UnitFilter) -> Result<QueryOutcome, UnitError> {
        self.db.snapshot()?.query(filter)
    }

    /// The retained revisions of `unit`, newest first, metadata only (see
    /// [`UnitSnapshot::history`]; use [`DerivedDb::snapshot`] for a page, a
    /// range or the payloads).
    pub fn history(&self, unit: UnitId) -> Result<Vec<Revision>, UnitError> {
        self.db.snapshot()?.history(unit, HistoryQuery::latest(usize::MAX))
    }

    /// The derived data computed from any revision of `unit`, by identity, in
    /// identity order.
    pub fn derived_of(&self, unit: UnitId) -> Vec<DerivedId> {
        self.lock().registry.dependents_of_unit(unit)
    }

    /// The exact set of the `Ready` persisted vectors of dimension
    /// `dimension`, by their source unit. Built once: pass it to as many
    /// searches as needed. A vector the set refuses (non-finite, or zero under
    /// cosine) is left out. When a unit has several such vectors, the one with
    /// the greatest [`DerivedId`] is used.
    pub fn vector_set(&self, dimension: usize, metric: VectorMetric) -> Result<VectorSet, VectorError> {
        let mut set = VectorSet::new(dimension, metric)?;
        let inner = self.lock();
        for (id, v) in &inner.vectors {
            if v.len() != dimension {
                continue;
            }
            let mut sources = inner.registry.descriptor(id).into_iter().flat_map(|d| d.sources());
            if let (Some(source), None) = (sources.next(), sources.next()) {
                // A refused vector is not a candidate (see above).
                let _ = set.insert(source.unit, v.clone());
            }
        }
        Ok(set)
    }

    /// For each of `queries`, the `k` units whose persisted vector is most
    /// similar, in (similarity descending, unit id ascending) order. The
    /// [`VectorSet`] is built once for the whole batch, from the dimension of
    /// the first query; a query of another dimension or metric is
    /// [`VectorError`]. An empty batch gives an empty answer.
    pub fn search_vectors(
        &self,
        metric: VectorMetric,
        queries: &[VectorQuery],
        k: usize,
    ) -> Result<Vec<Vec<Scored>>, VectorError> {
        let Some(first) = queries.first() else {
            return Ok(Vec::new());
        };
        let set = self.vector_set(first.dimension(), metric)?;
        queries.iter().map(|q| set.search(q, k)).collect()
    }
}
