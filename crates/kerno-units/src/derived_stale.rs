//! E8-03: staleness and rebuild of derived data (etapa8-acta.md §5.2,
//! ADR-0018 option 2, invariants I-23, I-24, I-25 and I-38).
//!
//! [`DerivedDb`] owns a [`UnitDb`] and a [`DerivedRegistry`] (E8-02) and keeps
//! the two in step:
//!
//! * **Stale by dependency.** A change set begun through
//!   [`DerivedDb::begin_changeset`] marks, when it commits, every derived
//!   datum that depends on a unit it wrote as `stale`, found through the
//!   registry's inverse index. The registry lock is taken before the commit
//!   and released after the marking, and every read of a derived state takes
//!   the same lock, so no reader sees the database ahead of the registry. A
//!   change set that fails to commit marks nothing. Units changed through
//!   [`DerivedDb::db`] directly bypass this; [`DerivedDb::open`] is the way
//!   to reconcile after such a change.
//! * **Rebuild.** [`DerivedDb::rebuild`] recomputes a `stale` or `failed`
//!   datum from the sources that are still valid (the units that still exist,
//!   at their current revision). The result is the datum a fresh
//!   [`DerivedDb::build`] over the same units makes: same identity, same input
//!   hash, same value (I-24). The function that computes the value is always
//!   supplied by the caller; the core runs no model and opens no network.
//! * **New method or version.** [`DerivedDb::supersede`] registers a new datum
//!   and marks the earlier one `superseded`; it never rewrites it.
//! * **Reopening.** The registry is in memory only (acta §5.4), so it is
//!   rebuilt by [`DerivedDb::open`] from the descriptors the caller still uses
//!   and the sources in the database: a datum whose sources are all current
//!   and whose input hash matches is recomputed and `ready`; any other is
//!   `stale`; none is ever `ready` without having been computed (I-23, R-E8-9).
//!   What cannot be rebuilt from sources (`superseded`, `building`) is not
//!   restored: the caller does not re-supply it.
//!
//! The compute function runs while the registry lock is held and must not
//! call back into the [`DerivedDb`] it was given to.
//!
//! No source content appears in an error or a `Debug` output (I-37).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, PoisonError};

use kerno_core::contract::TxnEngine;

use crate::db::{ChangeMeta, ChangeSet, CommitInfo, UnitDb};
use crate::derived::{DerivedDescriptor, DerivedError, DerivedId, DerivedRegistry, DerivedState};
use crate::error::UnitError;
use crate::payload::ContentHash;
use crate::types::{RevisionId, UnitId};

/// Domain tag of the input hash; bump it if the encoding below changes.
const INPUT_DOMAIN: &[u8] = b"kerno.derived.input.v1";

/// One source as the compute function sees it: the revision, the hash of its
/// content and its payload.
#[derive(Clone, PartialEq, Eq)]
pub struct SourceInput {
    pub id: RevisionId,
    pub content_hash: ContentHash,
    pub payload: Vec<u8>,
}

impl fmt::Debug for SourceInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SourceInput").field("id", &self.id).field("payload_len", &self.payload.len()).finish()
    }
}

/// The caller's function: the descriptor and the sources, in `(unit,
/// revision)` order, give the value; `None` means it could not be computed.
pub type Compute<'a> = &'a dyn Fn(&DerivedDescriptor, &[SourceInput]) -> Option<Vec<u8>>;

/// Why a call of [`DerivedDb`] was refused.
#[derive(Debug)]
pub enum DerivedStaleError {
    Derived(DerivedError),
    Unit(UnitError),
    /// The unit does not exist or is deleted, so it cannot be a source.
    MissingSource(UnitId),
    /// None of the sources of this datum is valid any more.
    NoValidSources(DerivedId),
    /// The compute function returned nothing; the datum is `failed`.
    ComputeFailed(DerivedId),
}

impl fmt::Display for DerivedStaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DerivedStaleError::Derived(e) => e.fmt(f),
            DerivedStaleError::Unit(e) => e.fmt(f),
            DerivedStaleError::MissingSource(_) => f.write_str("a source unit does not exist or is deleted"),
            DerivedStaleError::NoValidSources(_) => f.write_str("no source of the derived datum is valid any more"),
            DerivedStaleError::ComputeFailed(_) => f.write_str("the derived value could not be computed"),
        }
    }
}

impl std::error::Error for DerivedStaleError {}

impl From<DerivedError> for DerivedStaleError {
    fn from(e: DerivedError) -> Self {
        DerivedStaleError::Derived(e)
    }
}

impl From<UnitError> for DerivedStaleError {
    fn from(e: UnitError) -> Self {
        DerivedStaleError::Unit(e)
    }
}

/// The hash of the input of a derived datum: each source's revision and
/// content hash, in order.
pub fn input_hash(inputs: &[SourceInput]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(INPUT_DOMAIN);
    h.update(&(inputs.len() as u64).to_le_bytes());
    for i in inputs {
        h.update(&i.id.unit.0);
        h.update(&i.id.rev.0.to_le_bytes());
        h.update(&i.content_hash.digest);
    }
    *h.finalize().as_bytes()
}

#[derive(Default)]
struct Inner {
    registry: DerivedRegistry,
    /// The value of each `ready` datum; nothing else is kept.
    values: BTreeMap<DerivedId, Vec<u8>>,
}

/// A unit database and the registry of the data derived from it.
pub struct DerivedDb<E: TxnEngine> {
    db: UnitDb<E>,
    inner: Mutex<Inner>,
}

impl<E: TxnEngine> DerivedDb<E> {
    /// Wraps `db` and rebuilds the registry for `known` (see the module
    /// documentation, "Reopening"). `compute` recomputes the data that are
    /// still current.
    pub fn open(
        db: UnitDb<E>,
        known: impl IntoIterator<Item = DerivedDescriptor>,
        compute: Compute<'_>,
    ) -> Result<Self, DerivedStaleError> {
        let this = DerivedDb { db, inner: Mutex::new(Inner::default()) };
        {
            let mut inner = this.lock();
            for d in known {
                let units: BTreeSet<UnitId> = d.sources().map(|s| s.unit).collect();
                let id = inner.registry.register(d)?;
                let desc = inner.registry.descriptor(&id).cloned().ok_or(DerivedError::Unknown(id))?;
                let inputs = this.read_inputs(units)?;
                let current =
                    inputs.iter().map(|i| i.id).eq(desc.sources()) && input_hash(&inputs) == *desc.input_hash();
                if current {
                    // A failed compute leaves the datum `failed`, which is what
                    // it is; reopening does not abort for it.
                    Self::finish(&mut inner, id, &desc, &inputs, compute).ok();
                } else {
                    inner.registry.transition(&id, DerivedState::Stale)?;
                }
            }
        }
        Ok(this)
    }

    /// The underlying database, for reads. A change written through it is
    /// not tracked; use [`DerivedDb::begin_changeset`].
    pub fn db(&self) -> &UnitDb<E> {
        &self.db
    }

    pub fn close(self) -> Result<(), UnitError> {
        self.db.close()
    }

    /// Starts a change set whose commit marks the dependent derived data
    /// stale.
    pub fn begin_changeset(&self, meta: ChangeMeta) -> Result<DerivedChangeSet<'_, E>, UnitError> {
        Ok(DerivedChangeSet { cs: self.db.begin_changeset(meta)?, owner: self })
    }

    pub fn state(&self, id: &DerivedId) -> Option<DerivedState> {
        self.lock().registry.state(id)
    }

    pub fn descriptor(&self, id: &DerivedId) -> Option<DerivedDescriptor> {
        self.lock().registry.descriptor(id).cloned()
    }

    /// The value of a `ready` datum. Values are held only for `ready` data
    /// (dropped when a datum goes stale, is replaced or superseded), so a datum
    /// in any other state has none to serve.
    pub fn value(&self, id: &DerivedId) -> Option<Vec<u8>> {
        self.lock().values.get(id).cloned()
    }

    /// Whether the registry's inverse index is exactly what its entries imply.
    pub fn index_is_exact(&self) -> bool {
        self.lock().registry.index_is_exact()
    }

    /// Builds a derived datum over the current revision of each of `units`.
    pub fn build(
        &self,
        units: impl IntoIterator<Item = UnitId>,
        method: &str,
        version: u32,
        params: BTreeMap<String, String>,
        compute: Compute<'_>,
    ) -> Result<DerivedId, DerivedStaleError> {
        let mut inner = self.lock();
        let units: BTreeSet<UnitId> = units.into_iter().collect();
        let mut inputs = Vec::new();
        for u in &units {
            inputs.push(self.read_input(*u)?.ok_or(DerivedStaleError::MissingSource(*u))?);
        }
        let desc = Self::describe(&inputs, method, version, params)?;
        let id = inner.registry.register(desc.clone())?;
        Self::finish(&mut inner, id, &desc, &inputs, compute)
    }

    /// Rebuilds a `stale` or `failed` datum from its sources that are still
    /// valid. Returns the identity of the rebuilt datum, which is the one a
    /// new [`DerivedDb::build`] over the same units gets: if the sources moved
    /// to newer revisions, that identity differs from `id`, and the old entry
    /// is replaced.
    pub fn rebuild(&self, id: &DerivedId, compute: Compute<'_>) -> Result<DerivedId, DerivedStaleError> {
        let mut inner = self.lock();
        let state = inner.registry.state(id).ok_or(DerivedError::Unknown(*id))?;
        if !state.can_become(DerivedState::Building) {
            return Err(DerivedError::IllegalTransition { id: *id, from: state, to: DerivedState::Building }.into());
        }
        let old = inner.registry.descriptor(id).cloned().ok_or(DerivedError::Unknown(*id))?;
        let inputs = self.valid_inputs(&old)?;
        if inputs.is_empty() {
            inner.registry.transition(id, DerivedState::Building)?;
            inner.registry.transition(id, DerivedState::Failed)?;
            return Err(DerivedStaleError::NoValidSources(*id));
        }
        let desc = Self::describe(&inputs, old.method(), old.version(), old.params().clone())?;
        let new_id = desc.id();
        if new_id != *id && inner.registry.state(&new_id).is_some() {
            return Err(DerivedError::Duplicate(new_id).into());
        }
        inner.registry.remove(id)?;
        inner.registry.register(desc.clone())?;
        Self::finish(&mut inner, new_id, &desc, &inputs, compute)
    }

    /// Creates the datum for a new `method`, `version` and `params` over the
    /// still-valid sources of `old` and marks `old` `superseded`. `old` is not
    /// rewritten.
    pub fn supersede(
        &self,
        old: &DerivedId,
        method: &str,
        version: u32,
        params: BTreeMap<String, String>,
        compute: Compute<'_>,
    ) -> Result<DerivedId, DerivedStaleError> {
        let mut inner = self.lock();
        let state = inner.registry.state(old).ok_or(DerivedError::Unknown(*old))?;
        if !state.can_become(DerivedState::Superseded) {
            return Err(DerivedError::IllegalTransition { id: *old, from: state, to: DerivedState::Superseded }.into());
        }
        let prev = inner.registry.descriptor(old).cloned().ok_or(DerivedError::Unknown(*old))?;
        let inputs = self.valid_inputs(&prev)?;
        if inputs.is_empty() {
            return Err(DerivedStaleError::NoValidSources(*old));
        }
        let desc = Self::describe(&inputs, method, version, params)?;
        let id = inner.registry.register(desc.clone())?;
        inner.registry.transition(old, DerivedState::Superseded)?;
        inner.values.remove(old);
        Self::finish(&mut inner, id, &desc, &inputs, compute)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic in a compute function leaves the datum `building`, which is
        // a state the registry can leave: the data are still consistent.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn read_input(&self, unit: UnitId) -> Result<Option<SourceInput>, UnitError> {
        Ok(self.db.get_unit(unit)?.and_then(|u| {
            let payload = u.payload?;
            Some(SourceInput { id: RevisionId { unit, rev: u.head }, content_hash: u.content_hash, payload })
        }))
    }

    /// The valid current inputs of `units`, in unit order.
    fn read_inputs(&self, units: BTreeSet<UnitId>) -> Result<Vec<SourceInput>, UnitError> {
        let mut out = Vec::new();
        for u in units {
            out.extend(self.read_input(u)?);
        }
        Ok(out)
    }

    fn valid_inputs(&self, d: &DerivedDescriptor) -> Result<Vec<SourceInput>, UnitError> {
        self.read_inputs(d.sources().map(|s| s.unit).collect())
    }

    fn describe(
        inputs: &[SourceInput],
        method: &str,
        version: u32,
        params: BTreeMap<String, String>,
    ) -> Result<DerivedDescriptor, DerivedError> {
        DerivedDescriptor::new(inputs.iter().map(|i| i.id), method, version, params, input_hash(inputs))
    }

    /// Computes the value of the `building` datum `id` and leaves it `ready`,
    /// or `failed` if `compute` gives nothing.
    fn finish(
        inner: &mut Inner,
        id: DerivedId,
        desc: &DerivedDescriptor,
        inputs: &[SourceInput],
        compute: Compute<'_>,
    ) -> Result<DerivedId, DerivedStaleError> {
        match compute(desc, inputs) {
            Some(v) => {
                inner.values.insert(id, v);
                inner.registry.transition(&id, DerivedState::Ready)?;
                Ok(id)
            }
            None => {
                inner.registry.transition(&id, DerivedState::Failed)?;
                Err(DerivedStaleError::ComputeFailed(id))
            }
        }
    }
}

/// A [`ChangeSet`] whose commit keeps the registry in step with the database.
/// It derefs to the change set for the operations.
pub struct DerivedChangeSet<'a, E: TxnEngine> {
    cs: ChangeSet<'a, E>,
    owner: &'a DerivedDb<E>,
}

impl<'a, E: TxnEngine> Deref for DerivedChangeSet<'a, E> {
    type Target = ChangeSet<'a, E>;
    fn deref(&self) -> &Self::Target {
        &self.cs
    }
}

impl<E: TxnEngine> DerefMut for DerivedChangeSet<'_, E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.cs
    }
}

impl<E: TxnEngine> DerivedChangeSet<'_, E> {
    /// Commits, and marks stale every derived datum that depends on a unit
    /// this change set wrote. If the commit fails nothing is marked.
    pub fn commit(self) -> Result<CommitInfo, UnitError> {
        let mut inner = self.owner.lock();
        let info = self.cs.commit()?;
        let units: BTreeSet<UnitId> = info.revisions.iter().map(|r| r.unit).collect();
        for unit in units {
            for id in inner.registry.dependents_of_unit(unit) {
                if matches!(inner.registry.state(&id), Some(DerivedState::Ready | DerivedState::Building)) {
                    // Both states may become stale (DerivedState::can_become).
                    inner.registry.transition(&id, DerivedState::Stale).ok();
                    inner.values.remove(&id);
                }
            }
        }
        Ok(info)
    }

    /// Discards the change set; nothing is marked.
    pub fn abort(self) {}
}
