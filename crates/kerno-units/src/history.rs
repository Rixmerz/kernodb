//! History and as-of reads (`etapa3-design.md` §3.5, §6.4, E3-04):
//! `get_revision`, `history`, `unit_as_of`, `state_as_of`, `change_set`,
//! `change_sets_since` and `compare`. `restore` is a change-set operation
//! ([`crate::ChangeSet::restore`]).
//!
//! # Contracts
//!
//! * **One snapshot per call.** Every read goes through one engine snapshot
//!   (the [`UnitSnapshot`]'s, or a fresh one for the [`UnitDb`] shortcuts),
//!   so a head and its revisions are read coherently (I-05).
//! * **Order is `commit_seq`, never the clock (I-26).** As-of reads locate
//!   revisions by `commit_seq` only; `created_at` is returned as data and
//!   never compared.
//! * **Reads verify.** Every payload read goes through [`verify_payload`]
//!   against its `REV_META`. A missing `REV_META` inside
//!   `[oldest_rev, head_rev]`, a revision chain whose `commit_seq` does not
//!   grow strictly (IU-3), or change-set members that do not match
//!   `member_count` are `Storage(Corruption)`: damage is never reported as
//!   `NotFound`, `Absent` or an empty result.
//! * **Policies are errors, not empty results (E3-05, design §3.4).** The
//!   policy in force now for the unit's type decides; a read that needs what
//!   it does not keep is [`UnitError::HistoryUnavailable`]:
//!   - `get_revision`/`compare`/`restore` of a revision below the head under
//!     `None` or `Audit` (checked after `RevisionNotFound`, before `Pruned`);
//!   - `history` under `None` (always), and with `with_payload` under `Audit`
//!     (always);
//!   - `unit_as_of`/`state_as_of` at a sequence between a unit's creation and
//!     its last revision under `None` or `Audit` (before creation is still
//!     `Absent`, from the last revision on the head answers);
//!   - under any policy, the payload of a non-tombstone revision that an
//!     `Audit` period dropped (`payload_present = 0`): `HistoryUnavailable {
//!     policy: Audit }`.
//!
//!   `state_as_of` fails as a whole when one unit of its page fails.
//! * **Bounded.** `history`, `state_as_of` and `change_sets_since` take an
//!   explicit `limit >= 1`; no call reads a whole history without one.

use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot, TxnEngine};

use crate::codec::{ChangeSetRecord, CsMember, Key, RevisionMeta, TAG_CHANGESET, TAG_CS_MEMBER, UnitHead};
use crate::db::{GetFn, UnitDb, UnitSnapshot, context, corruption, key, policy_of};
use crate::error::UnitError;
use crate::payload::{ContentHash, verify_payload};
use crate::types::{
    ChangeSetId, CommitSeq, HistoryPolicy, Lifecycle, Purpose, RevNo, RevisionId, RevisionKind, Timestamp, UnitId,
};

/// Order of [`HistoryQuery`] results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    /// Oldest first.
    Asc,
    /// Newest first.
    Desc,
}

/// A page of a unit's history (design §6.4): revisions in `[from, to)`,
/// at most `limit` of them. With `Asc` the page is the oldest `limit` of the
/// range, with `Desc` the newest `limit`, newest first. To continue, pass the
/// last revision returned as the next `to` (`Desc`) or its successor as the
/// next `from` (`Asc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryQuery {
    /// First revision, inclusive; `None` is the oldest retained one. A
    /// revision below the oldest retained one is [`UnitError::Pruned`], and
    /// so is `None` with a `to` at or below the oldest retained revision
    /// when retention has pruned something (`oldest > 1`).
    pub from: Option<RevNo>,
    /// End revision, exclusive; `None` (or anything past the head) is "up to
    /// the head".
    pub to: Option<RevNo>,
    pub order: Order,
    /// At least 1.
    pub limit: usize,
    /// Whether to read and verify each revision's payload.
    pub with_payload: bool,
}

impl HistoryQuery {
    /// The newest `limit` revisions, newest first, metadata only.
    pub fn latest(limit: usize) -> Self {
        HistoryQuery { from: None, to: None, order: Order::Desc, limit, with_payload: false }
    }

    /// The oldest `limit` retained revisions, oldest first, metadata only.
    pub fn oldest(limit: usize) -> Self {
        HistoryQuery { from: None, to: None, order: Order::Asc, limit, with_payload: false }
    }
}

/// The point of an as-of read. Only commit sequences: `AsOf::Time` is out of
/// Stage 3 (decision 8, I-26).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsOf {
    /// The state once every change set with `commit_seq <= s` committed.
    Seq(CommitSeq),
}

/// Who and what produced a revision (copied from its change set).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Provenance {
    pub actor: Option<Vec<u8>>,
    pub source: Option<Vec<u8>>,
}

fn opt(v: Vec<u8>) -> Option<Vec<u8>> {
    (!v.is_empty()).then_some(v)
}

/// One revision of a unit (design §6.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    pub id: RevisionId,
    pub kind: RevisionKind,
    pub parent: Option<RevNo>,
    pub commit_seq: CommitSeq,
    /// Equal to `commit_seq` (design §2.5).
    pub change_set: ChangeSetId,
    /// From the injected clock: descriptive, never used for ordering (I-26).
    pub created_at: Timestamp,
    /// The lifecycle this revision leaves the unit in, implied by `kind`:
    /// `Archive` → `Archived`, `Delete` → `Deleted`, every other kind →
    /// `Active`.
    pub lifecycle: Lifecycle,
    pub schema_version: u32,
    pub content_hash: ContentHash,
    pub payload_len: u32,
    pub restored_from: Option<RevNo>,
    /// The unit's metadata as of this revision (versioned, decision 6).
    pub metadata: Vec<u8>,
    pub provenance: Provenance,
    /// The verified payload; `None` for a tombstone or when the read asked
    /// for metadata only.
    pub payload: Option<Vec<u8>>,
}

fn lifecycle_after(kind: RevisionKind) -> Lifecycle {
    match kind {
        RevisionKind::Archive => Lifecycle::Archived,
        RevisionKind::Delete => Lifecycle::Deleted,
        _ => Lifecycle::Active,
    }
}

fn to_revision(unit: UnitId, rev: RevNo, meta: RevisionMeta, payload: Option<Vec<u8>>) -> Revision {
    Revision {
        id: RevisionId { unit, rev },
        kind: meta.kind,
        parent: meta.parent_rev,
        commit_seq: meta.commit_seq,
        change_set: meta.commit_seq,
        created_at: meta.created_at,
        lifecycle: lifecycle_after(meta.kind),
        schema_version: meta.schema_version,
        content_hash: meta.content_hash,
        payload_len: meta.payload_len,
        restored_from: meta.restored_from,
        metadata: meta.metadata,
        provenance: Provenance { actor: opt(meta.actor_ref), source: opt(meta.source_ref) },
        payload,
    }
}

/// What a unit was at an as-of point (design §6.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitAt {
    /// Not created yet at that point (or never created).
    Absent,
    /// The revision in force, with its verified payload.
    Present(Revision),
    /// The unit was deleted; the tombstone revision in force.
    DeletedAt(Revision),
    /// The revision in force at that point was removed by retention.
    Pruned { oldest: RevNo },
}

/// Result of `compare` (design §3.5): both revisions with their payloads.
/// A structural diff belongs to ADR-0008 and is not defined here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comparison {
    pub a: Revision,
    pub b: Revision,
    /// Same content hash and same payload bytes (two tombstones are the same).
    pub same_content: bool,
}

/// One member of a change set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeSetMember {
    pub revision: RevisionId,
    pub kind: RevisionKind,
}

/// A committed change set (design §2.5, §6.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSetInfo {
    /// Equal to its `commit_seq`.
    pub id: ChangeSetId,
    pub created_at: Timestamp,
    pub purpose: Purpose,
    pub reason: Option<String>,
    pub provenance: Provenance,
    pub correlation: Option<Vec<u8>>,
    /// In ordinal order. A member whose revision was pruned is still listed.
    pub members: Vec<ChangeSetMember>,
}

/// `(unit, state)` pairs returned by `state_as_of`.
pub type UnitStates = Vec<(UnitId, UnitAt)>;

// ------------------------------------------------------------ point reads

/// A unit's head, decoded, or `None` if the unit never existed.
pub(crate) fn read_head(get: &GetFn<'_>, id: UnitId) -> Result<Option<UnitHead>, UnitError> {
    let Some(raw) = get(&key(Key::UnitHead { unit: id })?)? else {
        return Ok(None);
    };
    let head = UnitHead::decode(&raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {id}")))?;
    if head.oldest_rev.0 == 0 || head.oldest_rev > head.head_rev {
        return Err(corruption(format!(
            "unit {id}: oldest_rev {} outside 1..=head_rev {}",
            head.oldest_rev, head.head_rev
        )));
    }
    Ok(Some(head))
}

/// `rev` must be a retained revision, `[oldest_rev, head_rev]`, whose
/// content `policy` keeps (only the head's under `None`/`Audit`).
pub(crate) fn check_bounds(head: &UnitHead, id: UnitId, rev: RevNo, policy: HistoryPolicy) -> Result<(), UnitError> {
    if rev.0 == 0 || rev > head.head_rev {
        return Err(UnitError::RevisionNotFound { unit: id, rev });
    }
    if rev < head.head_rev && policy != HistoryPolicy::Versioned {
        return Err(UnitError::HistoryUnavailable { policy });
    }
    if rev < head.oldest_rev {
        return Err(UnitError::Pruned { unit: id, oldest: head.oldest_rev });
    }
    Ok(())
}

/// The `REV_META` of a revision that must exist: missing is corruption.
pub(crate) fn read_meta(get: &GetFn<'_>, id: UnitId, rev: RevNo) -> Result<RevisionMeta, UnitError> {
    let Some(raw) = get(&key(Key::RevMeta { unit: id, rev })?)? else {
        return Err(corruption(format!("unit {id}: REV_META of retained revision {rev} is missing")));
    };
    Ok(RevisionMeta::decode(rev, &raw).map_err(|e| context(e, &format!("REV_META of unit {id} rev {rev}")))?)
}

/// A revision whose payload an `Audit` period dropped has no content to read.
pub(crate) fn check_content(meta: &RevisionMeta) -> Result<(), UnitError> {
    if !meta.payload_present && meta.kind != RevisionKind::Delete {
        return Err(UnitError::HistoryUnavailable { policy: HistoryPolicy::Audit });
    }
    Ok(())
}

/// The verified payload of a revision (`None` for a tombstone).
pub(crate) fn read_payload(
    get: &GetFn<'_>,
    id: UnitId,
    rev: RevNo,
    meta: &RevisionMeta,
) -> Result<Option<Vec<u8>>, UnitError> {
    let stored = get(&key(Key::RevPayload { unit: id, rev })?)?;
    let verified = verify_payload(&meta.payload_decl(), stored.as_deref())
        .map_err(|e| crate::PayloadError { detail: format!("unit {id} rev {rev}: {}", e.detail), ..e })?;
    Ok(verified.bytes().map(<[u8]>::to_vec))
}

fn load_revision(get: &GetFn<'_>, id: UnitId, rev: RevNo, with_payload: bool) -> Result<Revision, UnitError> {
    let meta = read_meta(get, id, rev)?;
    let payload = if with_payload {
        check_content(&meta)?;
        read_payload(get, id, rev, &meta)?
    } else {
        None
    };
    Ok(to_revision(id, rev, meta, payload))
}

/// The commit sequence of a revision, checked to lie strictly between the
/// sequences of its known neighbours (IU-3).
fn probe(get: &GetFn<'_>, id: UnitId, rev: RevNo, above: CommitSeq, below: CommitSeq) -> Result<CommitSeq, UnitError> {
    let cs = read_meta(get, id, rev)?.commit_seq;
    if cs <= above || cs >= below {
        return Err(corruption(format!(
            "unit {id}: revision {rev} has commit_seq {cs}, outside ({above}, {below}) of its neighbours"
        )));
    }
    Ok(cs)
}

/// The unit at sequence `s` (design §3.5): the newest revision with
/// `commit_seq <= s`, found by binary search over `REV_META` (O(log n)
/// point reads). `s` must not exceed the snapshot's sequence.
fn unit_at(get: &GetFn<'_>, id: UnitId, s: CommitSeq, fallback: HistoryPolicy) -> Result<UnitAt, UnitError> {
    let Some(head) = read_head(get, id)? else {
        return Ok(UnitAt::Absent);
    };
    let policy = policy_of(get, &head.unit_type, fallback)?;
    unit_at_head(get, id, &head, s, policy)
}

fn unit_at_head(
    get: &GetFn<'_>,
    id: UnitId,
    head: &UnitHead,
    s: CommitSeq,
    policy: HistoryPolicy,
) -> Result<UnitAt, UnitError> {
    if s < head.created_seq {
        return Ok(UnitAt::Absent);
    }
    let found = if head.updated_seq <= s {
        let meta = read_meta(get, id, head.head_rev)?;
        if meta.commit_seq != head.updated_seq {
            return Err(corruption(format!(
                "unit {id}: UNIT_HEAD updated_seq {} disagrees with revision {} (commit_seq {})",
                head.updated_seq, head.head_rev, meta.commit_seq
            )));
        }
        head.head_rev
    } else if policy != HistoryPolicy::Versioned {
        return Err(UnitError::HistoryUnavailable { policy });
    } else {
        // created_seq <= s < updated_seq, so head_rev > 1 and the answer is
        // below head_rev. Invariant: cs(lo) <= s < cs(hi).
        let top = CommitSeq(u64::MAX);
        let hi_cs = probe(get, id, head.head_rev, CommitSeq(0), top)?;
        if hi_cs != head.updated_seq {
            return Err(corruption(format!(
                "unit {id}: UNIT_HEAD updated_seq {} disagrees with revision {} (commit_seq {hi_cs})",
                head.updated_seq, head.head_rev
            )));
        }
        let (mut lo, mut hi) = (head.oldest_rev, head.head_rev);
        let mut lo_cs = if lo == hi { hi_cs } else { probe(get, id, lo, CommitSeq(0), hi_cs)? };
        if head.oldest_rev.0 == 1 && lo_cs != head.created_seq {
            return Err(corruption(format!(
                "unit {id}: UNIT_HEAD created_seq {} disagrees with revision 1 (commit_seq {lo_cs})",
                head.created_seq
            )));
        }
        if lo_cs > s {
            return Ok(UnitAt::Pruned { oldest: head.oldest_rev });
        }
        let mut hi_cs = hi_cs;
        while hi.0 - lo.0 > 1 {
            let mid = RevNo(lo.0 + (hi.0 - lo.0) / 2);
            let cs = probe(get, id, mid, lo_cs, hi_cs)?;
            if cs <= s {
                (lo, lo_cs) = (mid, cs);
            } else {
                (hi, hi_cs) = (mid, cs);
            }
        }
        lo
    };
    let rev = load_revision(get, id, found, true)?;
    Ok(if rev.kind == RevisionKind::Delete { UnitAt::DeletedAt(rev) } else { UnitAt::Present(rev) })
}

// ------------------------------------------------------------ snapshot API

fn need_limit(limit: usize) -> Result<(), UnitError> {
    if limit == 0 {
        return Err(UnitError::InvalidArgument("limit must be at least 1".into()));
    }
    Ok(())
}

/// Key/value pairs of one engine scan.
type KvPage = Vec<(Vec<u8>, Vec<u8>)>;

fn scan_all<S: Snapshot>(snap: &S, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<KvPage, UnitError> {
    Ok(snap.scan(start, end, limit)?)
}

impl<E: Engine> UnitSnapshot<E> {
    fn getter(&self) -> impl Fn(&[u8]) -> kerno_core::Result<Option<Vec<u8>>> + '_ {
        |k| self.snap.get(k)
    }

    fn check_as_of(&self, at: AsOf) -> Result<CommitSeq, UnitError> {
        let AsOf::Seq(s) = at;
        if s > self.seq() {
            return Err(UnitError::InvalidArgument(format!(
                "as-of sequence {s} is after the snapshot's {}",
                self.seq()
            )));
        }
        Ok(s)
    }

    /// One revision with its verified payload. `rev` above the head (or 0)
    /// is [`UnitError::RevisionNotFound`], one whose content the policy does
    /// not keep [`UnitError::HistoryUnavailable`], below the oldest retained
    /// one [`UnitError::Pruned`]; an unknown unit is [`UnitError::NotFound`].
    pub fn get_revision(&self, id: UnitId, rev: RevNo) -> Result<Revision, UnitError> {
        let get = self.getter();
        let head = read_head(&get, id)?.ok_or(UnitError::NotFound { unit: id })?;
        check_bounds(&head, id, rev, policy_of(&get, &head.unit_type, self.fallback)?)?;
        load_revision(&get, id, rev, true)
    }

    /// A page of the unit's history (see [`HistoryQuery`]): one bounded
    /// `scan` of its `REV_META` range, plus a verified payload read per
    /// revision when `with_payload`.
    pub fn history(&self, id: UnitId, q: HistoryQuery) -> Result<Vec<Revision>, UnitError> {
        need_limit(q.limit)?;
        let get = self.getter();
        let head = read_head(&get, id)?.ok_or(UnitError::NotFound { unit: id })?;
        match policy_of(&get, &head.unit_type, self.fallback)? {
            HistoryPolicy::None => return Err(UnitError::HistoryUnavailable { policy: HistoryPolicy::None }),
            HistoryPolicy::Audit if q.with_payload => {
                return Err(UnitError::HistoryUnavailable { policy: HistoryPolicy::Audit });
            }
            _ => {}
        }
        // A range that ends at or below the oldest retained revision asks
        // only for pruned revisions: `Pruned`, not an empty page (E3-06).
        if q.from.is_none() && head.oldest_rev.0 > 1 && q.to.is_some_and(|t| t <= head.oldest_rev) {
            return Err(UnitError::Pruned { unit: id, oldest: head.oldest_rev });
        }
        let from = q.from.unwrap_or(head.oldest_rev);
        if from.0 == 0 {
            return Err(UnitError::InvalidArgument("history from revision 0".into()));
        }
        if from < head.oldest_rev {
            return Err(UnitError::Pruned { unit: id, oldest: head.oldest_rev });
        }
        let end = q.to.map_or(head.head_rev.0 + 1, |t| t.0.min(head.head_rev.0 + 1));
        if from.0 >= end {
            return Ok(Vec::new());
        }
        let take = (end - from.0).min(q.limit as u64);
        let (lo, hi) = match q.order {
            Order::Asc => (from.0, from.0 + take),
            Order::Desc => (end - take, end),
        };
        let start = key(Key::RevMeta { unit: id, rev: RevNo(lo) })?;
        let (_, unit_end) = Key::unit_rev_range(crate::codec::TAG_REV_META, id);
        let stop = if hi > head.head_rev.0 { unit_end } else { key(Key::RevMeta { unit: id, rev: RevNo(hi) })? };
        let page = scan_all(&self.snap, Bound::Included(&start), Bound::Excluded(&stop), take as usize)?;
        let mut out = Vec::with_capacity(page.len());
        let mut prev: Option<CommitSeq> = None;
        // The key each entry must have: the start key with its revision
        // bumped in place (bytes compared, nothing decoded).
        let mut want = start;
        for (i, (k, v)) in page.into_iter().enumerate() {
            let rev = RevNo(lo + i as u64);
            let at = want.len() - 8;
            want[at..].copy_from_slice(&rev.0.to_be_bytes());
            if k != want {
                return Err(corruption(format!("unit {id}: REV_META of retained revision {rev} is missing")));
            }
            let meta =
                RevisionMeta::decode(rev, &v).map_err(|e| context(e, &format!("REV_META of unit {id} rev {rev}")))?;
            if prev.is_some_and(|p| meta.commit_seq <= p) || meta.commit_seq > head.updated_seq {
                return Err(corruption(format!(
                    "unit {id}: revision {rev} has commit_seq {} out of order (IU-3)",
                    meta.commit_seq
                )));
            }
            prev = Some(meta.commit_seq);
            let payload = if q.with_payload {
                check_content(&meta)?;
                read_payload(&get, id, rev, &meta)?
            } else {
                None
            };
            out.push(to_revision(id, rev, meta, payload));
        }
        if out.len() as u64 != take {
            let missing = RevNo(lo + out.len() as u64);
            return Err(corruption(format!("unit {id}: REV_META of retained revision {missing} is missing")));
        }
        if q.order == Order::Desc {
            out.reverse();
        }
        Ok(out)
    }

    /// The unit as of `at` (design §3.5, IU-6). A sequence after the
    /// snapshot's is [`UnitError::InvalidArgument`].
    pub fn unit_as_of(&self, id: UnitId, at: AsOf) -> Result<UnitAt, UnitError> {
        let s = self.check_as_of(at)?;
        unit_at(&self.getter(), id, s, self.fallback)
    }

    /// The state of the units at `at`, in unit id order, starting after
    /// `after`: up to `limit` units that existed at `at` (units created
    /// later are skipped, not reported as `Absent`). Linear in the number of
    /// units (design §3.5): a unit whose head is not newer than `at` is
    /// answered from its head, the others by binary search.
    pub fn state_as_of(&self, at: AsOf, after: Option<UnitId>, limit: usize) -> Result<UnitStates, UnitError> {
        need_limit(limit)?;
        let s = self.check_as_of(at)?;
        let get = self.getter();
        let (_, end) = Key::tag_range(crate::codec::TAG_UNIT_HEAD);
        let mut cursor = match after {
            Some(u) => key(Key::UnitHead { unit: u })?,
            None => vec![crate::codec::TAG_UNIT_HEAD],
        };
        let mut inclusive = after.is_none();
        let mut out = Vec::new();
        let mut policies: std::collections::BTreeMap<String, HistoryPolicy> = std::collections::BTreeMap::new();
        loop {
            let start = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = scan_all(&self.snap, start, Bound::Excluded(&end), 256)?;
            let n = page.len();
            for (k, v) in &page {
                let Key::UnitHead { unit } = Key::decode(k)? else {
                    return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
                };
                let head = UnitHead::decode(v).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
                if head.oldest_rev.0 == 0 || head.oldest_rev > head.head_rev {
                    return Err(corruption(format!("unit {unit}: oldest_rev outside 1..=head_rev")));
                }
                let policy = match policies.get(&head.unit_type) {
                    Some(p) => *p,
                    None => {
                        let p = policy_of(&get, &head.unit_type, self.fallback)?;
                        policies.insert(head.unit_type.clone(), p);
                        p
                    }
                };
                match unit_at_head(&get, unit, &head, s, policy)? {
                    UnitAt::Absent => {}
                    state => out.push((unit, state)),
                }
                if out.len() == limit {
                    return Ok(out);
                }
            }
            match page.into_iter().last() {
                Some((k, _)) if n == 256 => {
                    cursor = k;
                    inclusive = false;
                }
                _ => return Ok(out),
            }
        }
    }

    /// The change set committed with sequence `id`, or `None` if no change
    /// set of the layer has that sequence.
    pub fn change_set(&self, id: ChangeSetId) -> Result<Option<ChangeSetInfo>, UnitError> {
        if id.0 == 0 {
            return Ok(None);
        }
        let Some(raw) = self.snap.get(&key(Key::ChangeSet { seq: id })?)? else {
            return Ok(None);
        };
        Ok(Some(self.change_set_info(id, &raw)?))
    }

    /// Up to `limit` change sets with `commit_seq > after`, in commit order
    /// (design §3.5: "what changed since `s`"). Sequences are not contiguous.
    pub fn change_sets_since(&self, after: CommitSeq, limit: usize) -> Result<Vec<ChangeSetInfo>, UnitError> {
        need_limit(limit)?;
        let mut start = vec![TAG_CHANGESET];
        start.extend_from_slice(&after.0.to_be_bytes());
        let end = vec![TAG_CS_MEMBER];
        let page = scan_all(&self.snap, Bound::Excluded(&start), Bound::Excluded(&end), limit)?;
        let mut out = Vec::with_capacity(page.len());
        for (k, v) in page {
            let Key::ChangeSet { seq } = Key::decode(&k)? else {
                return Err(corruption("a key in the CHANGESET range is not a CHANGESET".into()));
            };
            out.push(self.change_set_info(seq, &v)?);
        }
        Ok(out)
    }

    fn change_set_info(&self, seq: CommitSeq, raw: &[u8]) -> Result<ChangeSetInfo, UnitError> {
        let rec = ChangeSetRecord::decode(raw).map_err(|e| context(e, &format!("CHANGESET {seq}")))?;
        let mut start = vec![TAG_CS_MEMBER];
        start.extend_from_slice(&seq.0.to_be_bytes());
        let mut end = start.clone();
        end.extend_from_slice(&u32::MAX.to_be_bytes());
        let want = rec.member_count as usize;
        let page = scan_all(&self.snap, Bound::Included(&start), Bound::Included(&end), want + 1)?;
        if page.len() != want {
            return Err(corruption(format!(
                "CHANGESET {seq}: member_count {want} but {} CS_MEMBER records",
                page.len()
            )));
        }
        let mut members = Vec::with_capacity(want);
        for (i, (k, v)) in page.into_iter().enumerate() {
            if Key::decode(&k)? != (Key::CsMember { seq, ordinal: i as u32 }) {
                return Err(corruption(format!("CHANGESET {seq}: CS_MEMBER ordinal {i} is missing")));
            }
            let m = CsMember::decode(&v).map_err(|e| context(e, &format!("CS_MEMBER {seq}/{i}")))?;
            members.push(ChangeSetMember { revision: RevisionId { unit: m.unit, rev: m.rev_no }, kind: m.kind });
        }
        let reason = match String::from_utf8(rec.reason) {
            Ok(r) => (!r.is_empty()).then_some(r),
            Err(_) => return Err(corruption(format!("CHANGESET {seq}: reason is not UTF-8"))),
        };
        Ok(ChangeSetInfo {
            id: seq,
            created_at: rec.created_at,
            purpose: rec.purpose,
            reason,
            provenance: Provenance { actor: opt(rec.actor_ref), source: opt(rec.source_ref) },
            correlation: opt(rec.correlation),
            members,
        })
    }

    /// Both revisions with verified payloads and whether their content is
    /// the same (design §3.5). Errors as [`get_revision`](Self::get_revision).
    pub fn compare(&self, id: UnitId, a: RevNo, b: RevNo) -> Result<Comparison, UnitError> {
        let a = self.get_revision(id, a)?;
        let b = self.get_revision(id, b)?;
        let same_content = a.content_hash == b.content_hash && a.payload == b.payload;
        Ok(Comparison { a, b, same_content })
    }
}

// ------------------------------------------------------------ db shortcuts

impl<E: TxnEngine> UnitDb<E> {
    /// [`UnitSnapshot::get_revision`] on a fresh snapshot.
    pub fn get_revision(&self, id: UnitId, rev: RevNo) -> Result<Revision, UnitError> {
        self.snapshot()?.get_revision(id, rev)
    }

    /// [`UnitSnapshot::history`] on a fresh snapshot.
    pub fn history(&self, id: UnitId, q: HistoryQuery) -> Result<Vec<Revision>, UnitError> {
        self.snapshot()?.history(id, q)
    }

    /// [`UnitSnapshot::unit_as_of`] on a fresh snapshot.
    pub fn unit_as_of(&self, id: UnitId, at: AsOf) -> Result<UnitAt, UnitError> {
        self.snapshot()?.unit_as_of(id, at)
    }

    /// [`UnitSnapshot::state_as_of`] on a fresh snapshot.
    pub fn state_as_of(&self, at: AsOf, after: Option<UnitId>, limit: usize) -> Result<UnitStates, UnitError> {
        self.snapshot()?.state_as_of(at, after, limit)
    }

    /// [`UnitSnapshot::change_set`] on a fresh snapshot.
    pub fn change_set(&self, id: ChangeSetId) -> Result<Option<ChangeSetInfo>, UnitError> {
        self.snapshot()?.change_set(id)
    }

    /// [`UnitSnapshot::change_sets_since`] on a fresh snapshot.
    pub fn change_sets_since(&self, after: CommitSeq, limit: usize) -> Result<Vec<ChangeSetInfo>, UnitError> {
        self.snapshot()?.change_sets_since(after, limit)
    }

    /// [`UnitSnapshot::compare`] on a fresh snapshot.
    pub fn compare(&self, id: UnitId, a: RevNo, b: RevNo) -> Result<Comparison, UnitError> {
        self.snapshot()?.compare(id, a, b)
    }

    /// A change set with a single [`restore`](crate::ChangeSet::restore).
    pub fn restore(
        &self,
        id: UnitId,
        from: RevNo,
        expected: Option<RevNo>,
        meta: crate::ChangeMeta,
    ) -> Result<(RevNo, crate::CommitInfo), UnitError> {
        let mut cs = self.begin_changeset(meta)?;
        let rev = cs.restore(id, from, expected)?;
        Ok((rev, cs.commit()?))
    }
}
