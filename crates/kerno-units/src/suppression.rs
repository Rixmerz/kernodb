//! E8-05: how a suppression reaches the derived data (etapa8-acta.md §5.3,
//! ADR-0009 option 3, criteria I-1 and I-2).
//!
//! A datum derived from a source is removed when that source is removed, in
//! the same commit, found through the registry's inverse index:
//!
//! * **Deleting a unit.** [`crate::DerivedChangeSet::commit`] removes, after the
//!   commit, every registry entry (persisted or not) that has the unit among
//!   its sources, whatever its other sources: a datum with several sources goes
//!   when any one of them goes. The persisted records of those data (descriptor,
//!   vector, `0x62` entry) are deleted by the database change set itself, in
//!   the transaction of the deletion (E8-04, ADR-0020 §S3), so a new read never
//!   returns them while a snapshot taken before the commit still does.
//! * **Retention.** [`DerivedDb::run_retention`] runs the retention of the
//!   database, which deletes the persisted records of a purged revision in its
//!   own change set (E8-04), and then drops from the registry every entry whose
//!   source revision is gone.
//! * **Deleting a relation.** Absent on purpose: a derived datum has revisions
//!   of units as sources (`DerivedDescriptor::sources`) and the crate has no
//!   notion of a relation as a source, so a relation has no derived data to
//!   remove. A datum that depended on a relation would need a source kind the
//!   registry does not have; this module does not invent one.
//!
//! No promise of physical erasure: the bytes may stay in a log or a table until
//! a compaction covers them (ADR-0009, C-1). Nothing here names a vector or a
//! payload (I-37).

use kerno_core::contract::TxnEngine;

use crate::derived::DerivedId;
use crate::derived_stale::{DerivedDb, Inner};
use crate::error::UnitError;
use crate::retention::{RetentionBudget, RetentionPolicy, RetentionReport};
use crate::types::{Lifecycle, UnitId};

/// What [`DerivedDb::run_retention`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuppressionReport {
    /// What the retention of the database did.
    pub retention: RetentionReport,
    /// The registry entries dropped because a source revision was purged.
    pub removed: Vec<DerivedId>,
}

/// Drops from the registry and from every map each datum that has `unit`
/// among its sources.
pub(crate) fn remove_dependents_of_unit(inner: &mut Inner, unit: UnitId) -> Vec<DerivedId> {
    let ids = inner.registry.dependents_of_unit(unit);
    for id in &ids {
        inner.forget(id);
    }
    ids
}

impl<E: TxnEngine> DerivedDb<E> {
    /// Runs the retention of the database and drops every derived datum, in
    /// the registry, whose source revision it purged (or whose unit is
    /// deleted). The registry lock is held throughout, so a reader of a
    /// derived state never sees the purge without the removal.
    ///
    /// If the retention fails the registry is still reconciled with what did
    /// commit, and the error is returned.
    pub fn run_retention(
        &self,
        policy: &RetentionPolicy,
        budget: RetentionBudget,
    ) -> Result<SuppressionReport, UnitError> {
        let mut inner = self.lock();
        let ran = self.db.run_retention(policy, budget);
        let removed = self.drop_gone_sources(&mut inner)?;
        Ok(SuppressionReport { retention: ran?, removed })
    }

    /// Removes the entries with a source that no longer exists.
    fn drop_gone_sources(&self, inner: &mut Inner) -> Result<Vec<DerivedId>, UnitError> {
        let mut removed = Vec::new();
        for unit in inner.registry.source_units() {
            let head = self.db.get_unit(unit)?;
            for id in inner.registry.dependents_of_unit(unit) {
                let gone = inner.registry.descriptor(&id).is_some_and(|d| {
                    d.sources().any(|s| {
                        s.unit == unit
                            && head.as_ref().is_none_or(|u| u.lifecycle == Lifecycle::Deleted || s.rev < u.oldest)
                    })
                });
                if gone {
                    inner.forget(&id);
                    removed.push(id);
                }
            }
        }
        Ok(removed)
    }
}
