//! Reference model of the layer (`etapa3-design.md` §8.1).
//!
//! The model stores revisions and change sets **as already decided**: it does
//! not decide what an operation does (conflicts, which lifecycle an update
//! leaves, policies, retention). It only checks the structural rules that
//! hold for any operation — `rev_no` and `commit_seq` grow together per unit
//! (I-26, IU-3), `parent_rev = rev - 1`, change sets are applied in strictly
//! increasing `commit_seq` — and answers queries from that history.
//!
//! History policies (E3-05) are modelled: [`UnitModel::set_policy`] records
//! a `Maintenance` change set, [`UnitModel::apply`] drops what the policy in
//! force does not keep of the superseded revisions, and the `read_*` methods
//! answer as the database does, [`UnitError::HistoryUnavailable`] included
//! (see [`crate::history`] for the rules). The plain queries
//! ([`history`](UnitModel::history), [`unit_as_of`](UnitModel::unit_as_of),
//! …) keep answering from the full logical history.
//!
//! Retention (E3-06) is modelled by [`UnitModel::retain`], which decides
//! independently of the database what an unbounded `run_retention` does
//! (see [`crate::retention`] for the rules). Building revisions from
//! operations stays with the tests that drive it.

use std::collections::BTreeMap;

use crate::error::UnitError;
use crate::retention::{RetentionBudget, RetentionPolicy, RetentionRule};
use crate::types::{CommitSeq, HistoryPolicy, Lifecycle, Purpose, RevNo, RevisionKind, Timestamp, UnitId};

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
    /// Whether the layer still stores this revision's payload: `false` once
    /// an `Audit` policy dropped it (the logical `payload` is kept here so
    /// tests can compare hashes).
    pub content_kept: bool,
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
    /// `Delete` with a payload (UNIT-21), or `lifecycle = Deleted` on a kind
    /// other than `Delete` (or the reverse).
    BadLifecycle { unit: UnitId, kind: RevisionKind },
}

/// `BTreeMap<UnitId, Vec<ModelRevision>>` plus the list of change sets,
/// the policies and each unit's oldest retained revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitModel {
    units: BTreeMap<UnitId, Vec<ModelRevision>>,
    change_sets: Vec<ModelChangeSet>,
    default_policy: HistoryPolicy,
    policies: BTreeMap<String, HistoryPolicy>,
    oldest: BTreeMap<UnitId, RevNo>,
}

impl Default for UnitModel {
    fn default() -> Self {
        Self::with_default_policy(HistoryPolicy::Versioned)
    }
}

impl UnitModel {
    /// A model whose default policy is `Versioned`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A model of a database whose layer mark holds `p`.
    pub fn with_default_policy(p: HistoryPolicy) -> Self {
        UnitModel {
            units: BTreeMap::new(),
            change_sets: Vec::new(),
            default_policy: p,
            policies: BTreeMap::new(),
            oldest: BTreeMap::new(),
        }
    }

    /// The policy in force for `unit_type`.
    pub fn policy(&self, unit_type: &str) -> HistoryPolicy {
        self.policies.get(unit_type).copied().unwrap_or(self.default_policy)
    }

    /// A `set_policy` change set: purpose `Maintenance`, no members.
    pub fn set_policy(&mut self, seq: CommitSeq, unit_type: &str, p: HistoryPolicy) -> Result<(), ModelError> {
        if seq <= self.last_seq() {
            return Err(ModelError::SeqNotIncreasing { last: self.last_seq(), got: seq });
        }
        self.policies.insert(unit_type.to_owned(), p);
        self.change_sets.push(ModelChangeSet { seq, purpose: Purpose::Maintenance, members: Vec::new() });
        Ok(())
    }

    /// The oldest revision the layer keeps of `unit` (1 when unknown).
    pub fn oldest(&self, unit: UnitId) -> RevNo {
        self.oldest.get(&unit).copied().unwrap_or(RevNo(1))
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
            let is_delete = c.kind == RevisionKind::Delete;
            if (is_delete && c.payload.is_some()) || (is_delete != (c.lifecycle == Lifecycle::Deleted)) {
                return Err(ModelError::BadLifecycle { unit: c.unit, kind: c.kind });
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
            let policy = self.policy(&c.unit_type);
            let history = self.units.entry(c.unit).or_default();
            let rev = RevNo(history.len() as u64 + 1);
            match (policy, history.last_mut()) {
                (HistoryPolicy::Audit, Some(prev)) => prev.content_kept = false,
                (HistoryPolicy::None, Some(_)) => {
                    self.oldest.insert(c.unit, rev);
                }
                _ => {}
            }
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
                content_kept: true,
            });
        }
        self.change_sets.push(ModelChangeSet { seq, purpose, members });
        Ok(self.change_sets.last())
    }

    /// What one unbounded `run_retention(policy)` does, at clock reading
    /// `now`, with `created_at(seq)` the clock reading of change set `seq`
    /// (see [`retain_within`](Self::retain_within)).
    pub fn retain(
        &mut self,
        seq: CommitSeq,
        policy: &RetentionPolicy,
        now: Timestamp,
        created_at: impl Fn(CommitSeq) -> Timestamp,
    ) -> Result<Option<&ModelChangeSet>, ModelError> {
        let budget = RetentionBudget { max_units: usize::MAX, max_ops: usize::MAX };
        Ok(self.retain_within(seq, policy, budget, now, created_at)?.0)
    }

    /// What one `run_retention(policy, budget)` does: records a `Retention`
    /// change set `seq` (when some unit changes) whose members are `(unit,
    /// first retained revision, its kind)`, moves each unit's oldest
    /// revision and marks the payloads `Audit` drops. Units are visited in id
    /// order; a unit costs 4 writes when it loses revisions (two range
    /// deletions, head, member), else 1 (member), plus 2 per dropped
    /// payload; the call stops (returning `false`) at the first unit that
    /// does not fit, after taking the payload drops that fit.
    pub fn retain_within(
        &mut self,
        seq: CommitSeq,
        policy: &RetentionPolicy,
        budget: RetentionBudget,
        now: Timestamp,
        created_at: impl Fn(CommitSeq) -> Timestamp,
    ) -> Result<(Option<&ModelChangeSet>, bool), ModelError> {
        let mut members = Vec::new();
        let mut plans = Vec::new();
        let (mut ops, mut complete) = (0usize, true);
        for (&unit, history) in &self.units {
            let head = history.last().expect("a unit has a revision");
            let hp = self.policy(&head.unit_type);
            let oldest = self.oldest(unit);
            let retained = &history[oldest.0 as usize - 1..];
            let mut k = match (hp, policy.rule(&head.unit_type)) {
                (HistoryPolicy::None, _) => head.rev,
                (_, RetentionRule::KeepAll) => oldest,
                (_, RetentionRule::KeepLast(n)) => RevNo(head.rev.0.saturating_sub(n.saturating_sub(1))),
                (_, RetentionRule::KeepSince(s)) => {
                    retained.iter().rev().find(|r| r.commit_seq <= s).map_or(oldest, |r| r.rev)
                }
                (_, RetentionRule::KeepFor(d)) => {
                    let cutoff = now.0.saturating_sub(i64::try_from(d.as_micros()).unwrap_or(i64::MAX));
                    retained
                        .iter()
                        .find(|r| created_at(r.commit_seq).0 >= cutoff)
                        .map_or(head.rev, |r| RevNo(r.rev.0 - 1))
                }
            };
            if hp != HistoryPolicy::None && head.kind == RevisionKind::Delete && head.rev.0 > 1 {
                k = k.min(RevNo(head.rev.0 - 1));
            }
            let k = k.max(oldest).min(head.rev);
            let mut drops: Vec<RevNo> = if hp == HistoryPolicy::Audit {
                history[k.0 as usize - 1..history.len() - 1]
                    .iter()
                    .filter(|r| r.content_kept && r.payload.is_some())
                    .map(|r| r.rev)
                    .collect()
            } else {
                Vec::new()
            };
            if k == oldest && drops.is_empty() {
                continue;
            }
            let base = if k > oldest { 4 } else { 1 };
            let room = budget.max_ops - ops;
            if members.len() == budget.max_units || room < base + if drops.is_empty() { 0 } else { 2 } {
                complete = false;
                break;
            }
            let take = drops.len().min((room - base) / 2);
            let partial = take < drops.len();
            drops.truncate(take);
            ops += base + 2 * take;
            members.push((unit, k, history[k.0 as usize - 1].kind));
            plans.push((unit, k, drops));
            if partial {
                complete = false;
                break;
            }
        }
        if members.is_empty() {
            return Ok((None, complete));
        }
        if seq <= self.last_seq() {
            return Err(ModelError::SeqNotIncreasing { last: self.last_seq(), got: seq });
        }
        for (unit, k, drops) in plans {
            self.oldest.insert(unit, k);
            let history = self.units.get_mut(&unit).expect("unit");
            for r in drops {
                history[r.0 as usize - 1].content_kept = false;
            }
        }
        self.change_sets.push(ModelChangeSet { seq, purpose: Purpose::Retention, members });
        Ok((self.change_sets.last(), complete))
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

    /// Every unit that existed once `seq` committed, in id order, with its
    /// state then (units created later are left out).
    pub fn state_as_of(&self, seq: CommitSeq) -> Vec<(UnitId, ModelUnitAt<'_>)> {
        self.units.keys().map(|&u| (u, self.unit_as_of(u, seq))).filter(|(_, at)| *at != ModelUnitAt::Absent).collect()
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

    // -------------------------------------------------------- policy-aware reads

    fn unit_policy(&self, unit: UnitId) -> Option<HistoryPolicy> {
        self.head(unit).map(|h| self.policy(&h.unit_type))
    }

    fn check_content(r: &ModelRevision) -> Result<(), UnitError> {
        if !r.content_kept && r.kind != RevisionKind::Delete {
            return Err(UnitError::HistoryUnavailable { policy: HistoryPolicy::Audit });
        }
        Ok(())
    }

    /// `rev` checked as the database checks it: above the head, then the
    /// policy, then retention.
    fn check_bounds(&self, unit: UnitId, rev: RevNo) -> Result<&ModelRevision, UnitError> {
        let head = self.head(unit).ok_or(UnitError::NotFound { unit })?;
        if rev.0 == 0 || rev > head.rev {
            return Err(UnitError::RevisionNotFound { unit, rev });
        }
        let policy = self.policy(&head.unit_type);
        if rev < head.rev && policy != HistoryPolicy::Versioned {
            return Err(UnitError::HistoryUnavailable { policy });
        }
        let oldest = self.oldest(unit);
        if rev < oldest {
            return Err(UnitError::Pruned { unit, oldest });
        }
        Ok(self.revision(unit, rev).expect("revision within bounds"))
    }

    /// What `get_revision` (and each side of `compare`) answers.
    pub fn read_revision(&self, unit: UnitId, rev: RevNo) -> Result<&ModelRevision, UnitError> {
        let r = self.check_bounds(unit, rev)?;
        Self::check_content(r)?;
        Ok(r)
    }

    /// What `history` answers for revisions `[from, to)` (`None`: the
    /// oldest retained one / past the head) in ascending order, at most
    /// `limit` (taken from the low end with `asc`, the high end otherwise),
    /// returned oldest first.
    pub fn read_history(
        &self,
        unit: UnitId,
        from: Option<RevNo>,
        to: Option<RevNo>,
        asc: bool,
        limit: usize,
        with_payload: bool,
    ) -> Result<Vec<&ModelRevision>, UnitError> {
        if limit == 0 {
            return Err(UnitError::InvalidArgument("limit must be at least 1".into()));
        }
        let head = self.head(unit).ok_or(UnitError::NotFound { unit })?;
        match self.policy(&head.unit_type) {
            HistoryPolicy::None => return Err(UnitError::HistoryUnavailable { policy: HistoryPolicy::None }),
            HistoryPolicy::Audit if with_payload => {
                return Err(UnitError::HistoryUnavailable { policy: HistoryPolicy::Audit });
            }
            _ => {}
        }
        let oldest = self.oldest(unit);
        if from.is_none() && oldest.0 > 1 && to.is_some_and(|t| t <= oldest) {
            return Err(UnitError::Pruned { unit, oldest });
        }
        let from = from.unwrap_or(oldest);
        if from.0 == 0 {
            return Err(UnitError::InvalidArgument("history from revision 0".into()));
        }
        if from < oldest {
            return Err(UnitError::Pruned { unit, oldest });
        }
        let end = to.map_or(head.rev.0 + 1, |t| t.0.min(head.rev.0 + 1));
        if from.0 >= end {
            return Ok(Vec::new());
        }
        let take = (end - from.0).min(limit as u64);
        let (lo, hi) = if asc { (from.0, from.0 + take) } else { (end - take, end) };
        let out: Vec<&ModelRevision> = (lo..hi).map(|r| self.revision(unit, RevNo(r)).expect("revision")).collect();
        if with_payload {
            for r in &out {
                Self::check_content(r)?;
            }
        }
        Ok(out)
    }

    /// What `unit_as_of` answers. `Ok(None)` stands for `UnitAt::Pruned`.
    pub fn read_as_of(&self, unit: UnitId, seq: CommitSeq) -> Result<Option<ModelUnitAt<'_>>, UnitError> {
        let Some(policy) = self.unit_policy(unit) else {
            return Ok(Some(ModelUnitAt::Absent));
        };
        let at = self.unit_as_of(unit, seq);
        let r = match at {
            ModelUnitAt::Absent => return Ok(Some(at)),
            ModelUnitAt::Present(r) | ModelUnitAt::DeletedAt(r) => r,
        };
        let head = self.head(unit).expect("head");
        if r.rev < head.rev {
            if policy != HistoryPolicy::Versioned {
                return Err(UnitError::HistoryUnavailable { policy });
            }
            if r.rev < self.oldest(unit) {
                return Ok(None);
            }
        }
        Self::check_content(r)?;
        Ok(Some(at))
    }

    /// What `state_as_of(seq, after, limit)` answers (`None` in a pair
    /// stands for `UnitAt::Pruned`).
    #[allow(clippy::type_complexity)]
    pub fn read_state_as_of(
        &self,
        seq: CommitSeq,
        after: Option<UnitId>,
        limit: usize,
    ) -> Result<Vec<(UnitId, Option<ModelUnitAt<'_>>)>, UnitError> {
        if limit == 0 {
            return Err(UnitError::InvalidArgument("limit must be at least 1".into()));
        }
        let mut out = Vec::new();
        for u in self.units.keys().copied().filter(|u| after.is_none_or(|a| *u > a)) {
            match self.read_as_of(u, seq)? {
                Some(ModelUnitAt::Absent) => {}
                at => out.push((u, at)),
            }
            if out.len() == limit {
                break;
            }
        }
        Ok(out)
    }

    /// The source of `restore(unit, from)` as the database checks it, after
    /// `NotFound`/`Deleted` and the lifecycle (`Active` only).
    pub fn read_restore_source(&self, unit: UnitId, from: RevNo) -> Result<&ModelRevision, UnitError> {
        let head = self.head(unit).ok_or(UnitError::NotFound { unit })?;
        if head.lifecycle == Lifecycle::Deleted {
            return Err(UnitError::Deleted { unit });
        }
        if head.lifecycle != Lifecycle::Active {
            return Err(UnitError::InvalidArgument(format!("unit {unit} is not Active")));
        }
        let r = self.check_bounds(unit, from)?;
        Self::check_content(r)?;
        if r.payload.is_none() {
            return Err(UnitError::InvalidArgument(format!("revision {from} of unit {unit} is a tombstone")));
        }
        Ok(r)
    }
}
