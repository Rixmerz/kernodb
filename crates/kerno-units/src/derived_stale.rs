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
//!   change set that fails to commit marks nothing. The database is never
//!   handed out, so no write can bypass the marking.
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
use std::sync::{Mutex, MutexGuard, PoisonError};

use kerno_core::contract::TxnEngine;

use crate::db::{ChangeMeta, ChangeSet, CommitInfo, NewUnit, Unit, UnitDb, UnitUpdate};
use crate::derived::{DerivedDescriptor, DerivedError, DerivedId, DerivedRegistry, DerivedState};
use crate::error::UnitError;
use crate::payload::ContentHash;
use crate::suppression::source_gone;
use crate::types::{CommitSeq, RelationId, RelationProvenance, RevNo, RevisionId, UnitId};

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
    /// The datum is persisted: it changes through the vector methods, which
    /// write the database (ADR-0020).
    Persisted(DerivedId),
    /// The compute function returned nothing; the datum is `failed`.
    ComputeFailed(DerivedId),
}

impl fmt::Display for DerivedStaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DerivedStaleError::Derived(e) => e.fmt(f),
            DerivedStaleError::Unit(e) => e.fmt(f),
            DerivedStaleError::MissingSource(unit) => write!(f, "source unit {unit} does not exist or is deleted"),
            DerivedStaleError::Persisted(_) => f.write_str("the derived datum is persisted; use the vector methods"),
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

/// The hash of the input of a derived datum: each source's unit and revision,
/// in order. A revision is immutable and identifies its content, so the
/// content hash adds nothing to the identity and is not hashed.
pub fn input_hash(inputs: &[SourceInput]) -> [u8; 32] {
    let ids: Vec<RevisionId> = inputs.iter().map(|i| i.id).collect();
    input_hash_of_ids(&ids)
}

/// [`input_hash`] over the revisions alone (ADR-0020 §C.2).
pub(crate) fn input_hash_of_ids(ids: &[RevisionId]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(INPUT_DOMAIN);
    h.update(&(ids.len() as u64).to_le_bytes());
    for i in ids {
        h.update(&i.unit.0);
        h.update(&i.rev.0.to_le_bytes());
    }
    *h.finalize().as_bytes()
}

#[derive(Default)]
pub(crate) struct Inner {
    pub(crate) registry: DerivedRegistry,
    /// The value of each `ready` datum; nothing else is kept.
    pub(crate) values: BTreeMap<DerivedId, Vec<u8>>,
    /// The vector of each `ready` persisted datum (E8-04).
    pub(crate) vectors: BTreeMap<DerivedId, Vec<f32>>,
    /// The data whose descriptor is in the database (E8-04): the registry
    /// mirrors it, and a change set keeps the two in step.
    pub(crate) persisted: BTreeSet<DerivedId>,
}

impl Inner {
    /// Drops `id` from the registry and from every map.
    pub(crate) fn forget(&mut self, id: &DerivedId) {
        let _ = self.registry.remove(id);
        self.values.remove(id);
        self.vectors.remove(id);
        self.persisted.remove(id);
    }
}

/// What a committed change set did to the persisted derived data it wrote;
/// applied to the registry after the commit.
pub(crate) enum Pending {
    Put { desc: DerivedDescriptor, vector: Vec<f32> },
    Supersede { old: DerivedId },
    Remove { old: DerivedId },
}

/// A unit database and the registry of the data derived from it.
///
/// The database is not reachable except through reads and
/// [`DerivedDb::begin_changeset`], so a source cannot change without its
/// dependents being marked in the same commit:
///
/// ```compile_fail
/// fn bypass<E: kerno_core::contract::TxnEngine>(d: &kerno_units::DerivedDb<E>) {
///     let _ = d.db();
/// }
/// ```
pub struct DerivedDb<E: TxnEngine> {
    pub(crate) db: UnitDb<E>,
    inner: Mutex<Inner>,
}

impl<E: TxnEngine> DerivedDb<E> {
    /// An empty registry over `db`.
    pub(crate) fn wrap(db: UnitDb<E>) -> Self {
        DerivedDb { db, inner: Mutex::new(Inner::default()) }
    }

    /// Wraps `db` and rebuilds the registry for `known` (see the module
    /// documentation, "Reopening"). `compute` recomputes the data that are
    /// still current. Also returns the ids of the known descriptors dropped
    /// because a source is deleted or purged.
    pub fn open(
        db: UnitDb<E>,
        known: impl IntoIterator<Item = DerivedDescriptor>,
        compute: Compute<'_>,
    ) -> Result<(Self, Vec<DerivedId>), DerivedStaleError> {
        let this = DerivedDb { db, inner: Mutex::new(Inner::default()) };
        let mut dropped = Vec::new();
        {
            let mut inner = this.lock();
            for d in known {
                let units: BTreeSet<UnitId> = d.sources().map(|s| s.unit).collect();
                // A datum goes when any one of its sources goes (E8-05): it is
                // dropped and reported, never rebuilt from the sources left.
                let mut gone = false;
                for u in &units {
                    gone |= source_gone(&d, *u, this.db.get_unit(*u)?.as_ref());
                }
                if gone {
                    dropped.push(d.id());
                    continue;
                }
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
        Ok((this, dropped))
    }

    /// Reads a unit. The database itself is not handed out: every write goes
    /// through [`DerivedDb::begin_changeset`], so the registry cannot fall
    /// behind it.
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        self.db.get_unit(id)
    }

    pub fn close(self) -> Result<(), UnitError> {
        self.db.close()
    }

    /// Starts a change set whose commit marks the dependent derived data
    /// stale.
    pub fn begin_changeset(&self, meta: ChangeMeta) -> Result<DerivedChangeSet<'_, E>, UnitError> {
        Ok(DerivedChangeSet {
            cs: self.db.begin_changeset(meta)?,
            owner: self,
            pending: Vec::new(),
            deleted: BTreeSet::new(),
        })
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
        if inner.persisted.contains(id) {
            return Err(DerivedStaleError::Persisted(*id));
        }
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
        if inner.persisted.contains(old) {
            return Err(DerivedStaleError::Persisted(*old));
        }
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

    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic in a compute function leaves the datum `building`, which is
        // a state the registry can leave: the data are still consistent.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn read_input(&self, unit: UnitId) -> Result<Option<SourceInput>, UnitError> {
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

    /// The current inputs of the sources of `d`, in unit order. A source that
    /// is missing or deleted is an error naming it, never an input left out,
    /// so a rebuild cannot come out over fewer sources than a build would
    /// (E8 F-03). When no source is valid the result is empty and the caller
    /// answers `NoValidSources`.
    pub(crate) fn valid_inputs(&self, d: &DerivedDescriptor) -> Result<Vec<SourceInput>, DerivedStaleError> {
        let units: BTreeSet<UnitId> = d.sources().map(|s| s.unit).collect();
        let mut out = Vec::new();
        let mut missing = None;
        for u in units {
            match self.read_input(u)? {
                Some(input) => out.push(input),
                None => missing = missing.or(Some(u)),
            }
        }
        match missing {
            Some(unit) if !out.is_empty() => Err(DerivedStaleError::MissingSource(unit)),
            _ => Ok(out),
        }
    }

    pub(crate) fn describe(
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
///
/// The inner change set is a private field and is never handed out: there is
/// no `Deref`, no accessor and no `into_inner`. The only operations are the
/// forwarding methods below and [`DerivedChangeSet::commit`], which is the
/// only way to commit and marks the dependents. So a caller cannot commit
/// through the inner type (for instance by swapping it for a change set of
/// the plain database):
///
/// ```compile_fail
/// use kerno_core::contract::TxnEngine;
/// use kerno_units::{ChangeSet, DerivedChangeSet};
/// fn bypass<E: TxnEngine>(cs: &mut DerivedChangeSet<'_, E>) {
///     let _inner: &mut ChangeSet<'_, E> = &mut *cs;
/// }
/// ```
///
/// ```compile_fail
/// use kerno_core::contract::TxnEngine;
/// use kerno_units::DerivedChangeSet;
/// fn bypass<E: TxnEngine>(cs: DerivedChangeSet<'_, E>) {
///     let _ = cs.cs.commit();
/// }
/// ```
pub struct DerivedChangeSet<'a, E: TxnEngine> {
    pub(crate) cs: ChangeSet<'a, E>,
    owner: &'a DerivedDb<E>,
    /// Persisted derived data written by this change set (E8-04), applied to
    /// the registry once it commits.
    pub(crate) pending: Vec<Pending>,
    /// Units this change set deletes: their persisted derived data go with them.
    deleted: BTreeSet<UnitId>,
}

impl<E: TxnEngine> DerivedChangeSet<'_, E> {
    /// The sequence this change set will commit with.
    pub fn commit_seq(&self) -> CommitSeq {
        self.cs.commit_seq()
    }

    pub fn create(&mut self, new: NewUnit) -> Result<UnitId, UnitError> {
        self.cs.create(new)
    }

    pub fn update(&mut self, id: UnitId, upd: UnitUpdate, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.cs.update(id, upd, expected)
    }

    pub fn archive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.cs.archive(id, expected)
    }

    pub fn unarchive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.cs.unarchive(id, expected)
    }

    pub fn delete(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        let rev = self.cs.delete(id, expected)?;
        self.deleted.insert(id);
        Ok(rev)
    }

    pub fn restore(&mut self, id: UnitId, from: RevNo, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.cs.restore(id, from, expected)
    }

    pub fn create_relation(
        &mut self,
        from: UnitId,
        to: UnitId,
        relation_type: &str,
        provenance: RelationProvenance,
    ) -> Result<RelationId, UnitError> {
        self.cs.create_relation(from, to, relation_type, provenance)
    }

    pub fn delete_relation(&mut self, edge: RelationId) -> Result<(), UnitError> {
        self.cs.delete_relation(edge)
    }

    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        self.cs.get_unit(id)
    }

    /// Commits, and marks stale every derived datum that depends on a unit
    /// this change set wrote. If the commit fails nothing is marked: the
    /// marking is after `commit()?`, so an `Err` returns before it. A test
    /// makes the data sync of the commit fail on the simulated disk and checks
    /// that the datum stays ready.
    pub fn commit(self) -> Result<CommitInfo, UnitError> {
        // The registry lock is taken before the commit and held until the
        // marking is done. Every read of a derived state takes the same lock,
        // so a reader sees the source change and its stale marks together, never
        // the new source with a registry still behind it (I-23). Taking it after
        // the commit would open that window. This order is not covered by a
        // test: showing it needs a reader racing the commit, which cannot be
        // made deterministic.
        let mut inner = self.owner.lock();
        let info = self.cs.commit()?;
        // The persisted data changed in the commit itself (ADR-0020 §S3); the
        // registry follows what that transaction wrote.
        for p in self.pending {
            match p {
                Pending::Remove { old } => inner.forget(&old),
                Pending::Supersede { old } => {
                    // The commit is the truth; a registry that was behind it is
                    // rebuilt from the database on the next open.
                    let _ = inner.registry.transition(&old, DerivedState::Superseded);
                    inner.vectors.remove(&old);
                }
                Pending::Put { desc, vector } => {
                    let id = desc.id();
                    inner.forget(&id);
                    if inner.registry.register(desc).is_ok()
                        && inner.registry.transition(&id, DerivedState::Ready).is_ok()
                    {
                        inner.persisted.insert(id);
                        inner.vectors.insert(id, vector);
                    }
                }
            }
        }
        let heads: BTreeMap<UnitId, RevNo> = info.revisions.iter().map(|r| (r.unit, r.rev)).collect();
        for (unit, head) in heads {
            if self.deleted.contains(&unit) {
                // A deletion removes every datum that has the unit among its
                // sources, whatever its other sources (E8-05, acta §5.3).
                crate::suppression::remove_dependents_of_unit(&mut inner, unit);
                continue;
            }
            for id in inner.registry.dependents_of_unit(unit) {
                if inner.persisted.contains(&id) {
                    // Persisted data: a change left `Ready` only what is
                    // computed from the new head, which is what the
                    // transaction wrote.
                    let from = inner.registry.descriptor(&id).and_then(|d| d.sources().find(|s| s.unit == unit));
                    if from.is_some_and(|s| s.rev != head)
                        && inner.registry.transition(&id, DerivedState::Stale).is_ok()
                    {
                        inner.vectors.remove(&id);
                    }
                    continue;
                }
                // `ready` and `building` may become stale (DerivedState::can_become);
                // `stale`, `failed` and `superseded` data have nothing to mark,
                // so the refusal of those transitions is the answer, not an error.
                if inner.registry.transition(&id, DerivedState::Stale).is_ok() {
                    inner.values.remove(&id);
                }
            }
        }
        Ok(info)
    }

    /// Discards the change set; nothing is marked.
    pub fn abort(self) {}
}

#[cfg(test)]
mod tests {
    //! A source deleted while its dependents stay registered cannot be made
    //! through the public API (a delete removes them in the same commit), so
    //! the explicit error of `valid_inputs` is shown here, deleting behind the
    //! registry.

    use std::path::Path;
    use std::sync::Arc;

    use kerno_core::contract::Options;
    use kerno_core::vfs::Vfs;
    use kerno_storage::KernoDb;
    use kerno_testkit::emulated::Emulated;
    use kerno_testkit::reference::LogEngine;
    use kerno_testkit::vfs::SimVfs;

    use super::*;
    use crate::clock::{CounterIds, FixedClock};
    use crate::db::UnitConfig;
    use crate::types::Timestamp;

    fn cfg() -> UnitConfig {
        let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(Timestamp(1))), Arc::new(CounterIds::new(1)));
        c.engine = Options::small_for_tests();
        c
    }

    fn compute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<u8>> {
        Some(inputs.iter().flat_map(|i| i.payload.clone()).collect())
    }

    fn create<E: TxnEngine>(db: &DerivedDb<E>, payload: &[u8]) -> UnitId {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let unit = NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() };
        let id = cs.create(unit).unwrap();
        cs.commit().unwrap();
        id
    }

    /// Deletes `unit` on the database itself, so the registry keeps the
    /// datum a delete through the derived change set would have removed.
    fn delete_behind_the_registry<E: TxnEngine>(db: &DerivedDb<E>, unit: UnitId) {
        let mut cs = db.db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.delete(unit, None).unwrap();
        cs.commit().unwrap();
    }

    fn a_missing_source_is_an_error_naming_it<E: TxnEngine>() {
        let vfs: Arc<dyn Vfs> = Arc::new(SimVfs::new(1));
        let unit_db = UnitDb::<E>::open(vfs, Path::new("/db"), &cfg()).unwrap();
        let db = DerivedDb::open(unit_db, Vec::new(), &compute).unwrap().0;
        let (a, b) = (create(&db, b"a"), create(&db, b"b"));
        let id = db.build([a, b], "concat", 1, BTreeMap::new(), &compute).unwrap();
        let desc = db.descriptor(&id).unwrap();
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(a, UnitUpdate { payload: b"a2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
        cs.commit().unwrap();
        assert_eq!(db.state(&id), Some(DerivedState::Stale));

        // One source gone, one valid: an error naming the gone one, never a
        // datum over the valid one alone.
        delete_behind_the_registry(&db, b);
        let names_b = |e: DerivedStaleError| matches!(e, DerivedStaleError::MissingSource(u) if u == b);
        assert!(names_b(db.valid_inputs(&desc).unwrap_err()));
        assert!(names_b(db.rebuild(&id, &compute).unwrap_err()));
        assert!(names_b(db.supersede(&id, "concat", 2, BTreeMap::new(), &compute).unwrap_err()));
        assert_eq!(db.state(&id), Some(DerivedState::Stale), "a refused rebuild leaves the datum as it was");

        // Every source gone: no input, and the callers answer NoValidSources.
        delete_behind_the_registry(&db, a);
        assert!(db.valid_inputs(&desc).unwrap().is_empty());
        assert!(matches!(db.rebuild(&id, &compute), Err(DerivedStaleError::NoValidSources(i)) if i == id));
    }

    #[test]
    fn emulated_log_a_missing_source_is_an_error_naming_it() {
        a_missing_source_is_an_error_naming_it::<Emulated<LogEngine>>();
    }

    #[test]
    fn kerno_db_a_missing_source_is_an_error_naming_it() {
        a_missing_source_is_an_error_naming_it::<KernoDb>();
    }
}
