//! Reference model of the layer (`etapa3-design.md` §8.1), reduced to what
//! E3-02 can define without the operations of E3-03.
//!
//! The model stores revisions and change sets **as already decided**: it does
//! not decide what an operation does (conflicts, which lifecycle an update
//! leaves, policies, retention). It only checks the structural rules that
//! hold for any operation — `rev_no` and `commit_seq` grow together per unit
//! (I-26, IU-3), `parent_rev = rev - 1`, change sets are applied in strictly
//! increasing `commit_seq` — and answers queries from that history.
//!
//! Deferred: building revisions from operations and `expected` revisions
//! (E3-03), `compare`, `state_as_of` across units and paging (E3-04),
//! history policies (E3-05), retention and `Pruned` (E3-06).

use std::collections::BTreeMap;

use crate::types::{CommitSeq, Lifecycle, Purpose, RevNo, RevisionKind, UnitId};

/// One revision of a unit, with the unit state it leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRevision {
    pub rev: RevNo,
    pub kind: RevisionKind,
    pub commit_seq: CommitSeq,
    pub unit_type: String,
    pub schema_version: u32,
    /// Lifecycle after this revision (decided by the operation, E3-03).
    pub lifecycle: Lifecycle,
    pub restored_from: Option<RevNo>,
    pub metadata: Vec<u8>,
    pub payload: Option<Vec<u8>>,
}

/// A revision a change set adds to a unit; `rev` is assigned by the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChange {
    pub unit: UnitId,
    pub kind: RevisionKind,
    pub unit_type: String,
    pub schema_version: u32,
    pub lifecycle: Lifecycle,
    pub restored_from: Option<RevNo>,
    pub metadata: Vec<u8>,
    pub payload: Option<Vec<u8>>,
}

/// A committed change set: its sequence, purpose and members in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChangeSet {
    pub seq: CommitSeq,
    pub purpose: Purpose,
    pub members: Vec<(UnitId, RevNo, RevisionKind)>,
}

/// What a unit was at a commit sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelUnitAt<'a> {
    /// Not created yet at that sequence.
    Absent,
    Present(&'a ModelRevision),
    DeletedAt(&'a ModelRevision),
}

/// Why the model refused a change set. Refusal leaves the model unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// `seq` is not greater than the last applied sequence.
    SeqNotIncreasing { last: CommitSeq, got: CommitSeq },
    /// `Create` for a unit that exists, or another kind for one that does not.
    BadKind { unit: UnitId, kind: RevisionKind },
    /// `restored_from` missing on `Restore`, present elsewhere, or not an
    /// earlier revision.
    BadRestore { unit: UnitId },
    /// The same unit twice in one change set (one revision per unit per commit, I-26).
    DuplicateUnit { unit: UnitId },
}

/// `BTreeMap<UnitId, Vec<ModelRevision>>` plus the list of change sets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitModel {
    units: BTreeMap<UnitId, Vec<ModelRevision>>,
    change_sets: Vec<ModelChangeSet>,
}

impl UnitModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sequence of the last applied change set, 0 when empty.
    pub fn last_seq(&self) -> CommitSeq {
        self.change_sets.last().map_or(CommitSeq(0), |c| c.seq)
    }

    /// Applies a change set atomically: either every change is appended or
    /// none (and the error says why). An empty change set writes nothing.
    pub fn apply(
        &mut self,
        seq: CommitSeq,
        purpose: Purpose,
        changes: Vec<ModelChange>,
    ) -> Result<Option<&ModelChangeSet>, ModelError> {
        if changes.is_empty() {
            return Ok(None);
        }
        if seq <= self.last_seq() {
            return Err(ModelError::SeqNotIncreasing { last: self.last_seq(), got: seq });
        }
        let mut seen = std::collections::BTreeSet::new();
        for c in &changes {
            if !seen.insert(c.unit) {
                return Err(ModelError::DuplicateUnit { unit: c.unit });
            }
            let next = self.units.get(&c.unit).map_or(1, |h| h.len() as u64 + 1);
            if (c.kind == RevisionKind::Create) != (next == 1) {
                return Err(ModelError::BadKind { unit: c.unit, kind: c.kind });
            }
            let restore_ok = match (c.kind, c.restored_from) {
                (RevisionKind::Restore, Some(r)) => r.0 >= 1 && r.0 < next,
                (RevisionKind::Restore, None) => false,
                (_, r) => r.is_none(),
            };
            if !restore_ok {
                return Err(ModelError::BadRestore { unit: c.unit });
            }
        }
        let mut members = Vec::with_capacity(changes.len());
        for c in changes {
            let history = self.units.entry(c.unit).or_default();
            let rev = RevNo(history.len() as u64 + 1);
            members.push((c.unit, rev, c.kind));
            history.push(ModelRevision {
                rev,
                kind: c.kind,
                commit_seq: seq,
                unit_type: c.unit_type,
                schema_version: c.schema_version,
                lifecycle: c.lifecycle,
                restored_from: c.restored_from,
                metadata: c.metadata,
                payload: c.payload,
            });
        }
        self.change_sets.push(ModelChangeSet { seq, purpose, members });
        Ok(self.change_sets.last())
    }

    /// The current revision of a unit (including a tombstone).
    pub fn head(&self, unit: UnitId) -> Option<&ModelRevision> {
        self.units.get(&unit).and_then(|h| h.last())
    }

    /// Every revision of a unit, oldest first; parent of `rev` is `rev - 1`.
    pub fn history(&self, unit: UnitId) -> &[ModelRevision] {
        self.units.get(&unit).map_or(&[], Vec::as_slice)
    }

    pub fn revision(&self, unit: UnitId, rev: RevNo) -> Option<&ModelRevision> {
        let i = usize::try_from(rev.0.checked_sub(1)?).ok()?;
        self.history(unit).get(i)
    }

    /// The unit as it was once `seq` committed (IU-6).
    pub fn unit_as_of(&self, unit: UnitId, seq: CommitSeq) -> ModelUnitAt<'_> {
        match self.history(unit).iter().rev().find(|r| r.commit_seq <= seq) {
            None => ModelUnitAt::Absent,
            Some(r) if r.lifecycle == Lifecycle::Deleted => ModelUnitAt::DeletedAt(r),
            Some(r) => ModelUnitAt::Present(r),
        }
    }

    /// Up to `limit` change sets with `seq > after`, in commit order.
    pub fn change_sets_since(&self, after: CommitSeq, limit: usize) -> &[ModelChangeSet] {
        let start = self.change_sets.partition_point(|c| c.seq <= after);
        let end = start.saturating_add(limit).min(self.change_sets.len());
        &self.change_sets[start..end]
    }

    pub fn change_set(&self, seq: CommitSeq) -> Option<&ModelChangeSet> {
        self.change_sets.binary_search_by_key(&seq, |c| c.seq).ok().map(|i| &self.change_sets[i])
    }

    /// Unit ids in key order.
    pub fn unit_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.units.keys().copied()
    }
}
