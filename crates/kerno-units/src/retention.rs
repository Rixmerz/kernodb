//! Retention and GC (`etapa3-design.md` §5, §6.5, E3-06): [`RetentionPolicy`],
//! [`UnitDb::run_retention`], [`UnitDb::compact`] and their reports.
//!
//! # Contracts
//!
//! * **Logical retention only removes a prefix.** For each unit retention
//!   picks a new `oldest_rev = k` (never above the head) and, in one change
//!   set with purpose `Retention`, writes `delete_range(REV_META[oldest, k))`,
//!   `delete_range(REV_PAYLOAD[oldest, k))`, the head with `oldest_rev = k`
//!   and one `CS_MEMBER` naming `(unit, k, kind of k)`, the first retained
//!   revision. Head and ranges change in the same engine commit (I-21): a
//!   crash leaves every unit pruned whole or not at all (IU-7).
//! * **What is always kept.** The head revision; when the head is a `Delete`
//!   tombstone, also the revision before it (design §5.2) — except under
//!   history policy `None`, whose rule already keeps only the head.
//! * **History policies are enforced on what is stored.** Under `None` every
//!   revision below the head is pruned (`k = head`); under `Audit` every
//!   retained revision below the head that still has its `REV_PAYLOAD` (left
//!   over from an earlier `Versioned` period, "Nota de E3-05") loses it: its
//!   `REV_META` is rewritten with `payload_present = 0`, keeping hash and
//!   length, exactly as `Audit` does to a superseded head.
//! * **Rules** ([`RetentionRule`]) only decide `k`; they read `commit_seq`
//!   (`KeepSince`) or the descriptive `created_at` (`KeepFor`) of the
//!   retained revisions and keep, besides the revisions they name, the
//!   revision in force at their cut-off point, so that as-of reads from the
//!   cut-off on keep answering.
//! * **Bounded and resumable.** One call is one change set bounded by
//!   [`RetentionBudget`]; each call starts from the first unit again, units
//!   with nothing to do cost reads but no writes, so repeating a call is
//!   idempotent and a crash leaves nothing to recover.
//! * **Snapshots keep reading (IU-8).** Pruning is a commit like any other:
//!   a [`crate::UnitSnapshot`] taken before it keeps its whole history.
//!   Logical deletion is not physical deletion (§5.3): the bytes stay until
//!   the engine compacts them and no snapshot needs them.
//! * **Damage is never "nothing to do".** A missing or out-of-order
//!   `REV_META` inside `[oldest_rev, head_rev]` is `Storage(Corruption)`.

use std::collections::BTreeMap;
use std::ops::Bound;
use std::time::Duration;

use kerno_core::contract::{TestHooks, Txn, TxnEngine};

use crate::codec::{Key, LAYER_MARK_NAME, RevisionMeta, TAG_LAYER_META, TAG_REV_META, TAG_REV_PAYLOAD, UnitHead};
use crate::db::{ChangeSet, CommitInfo, UnitDb, Write, context, corruption, key};
use crate::error::UnitError;
use crate::types::{CommitSeq, HistoryPolicy, RevNo, RevisionKind, Timestamp, UnitId};

/// How much history of a unit type retention keeps (design §5.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RetentionRule {
    /// Keep every revision (the default).
    #[default]
    KeepAll,
    /// Keep the newest `n >= 1` revisions.
    KeepLast(u64),
    /// Keep the revisions created in the last `d` (by the injected clock;
    /// `created_at` is descriptive, so this only cuts a prefix) and the one
    /// in force at the cut-off.
    KeepFor(Duration),
    /// Keep what answers every as-of read at `s` and later: the revision in
    /// force at `s` and every later one.
    KeepSince(CommitSeq),
}

/// The [`RetentionRule`] of each unit type (design §5.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionPolicy {
    /// The rule of every type not in `by_type`.
    pub default: RetentionRule,
    pub by_type: BTreeMap<String, RetentionRule>,
}

impl RetentionPolicy {
    /// One rule for every type.
    pub fn all(rule: RetentionRule) -> Self {
        RetentionPolicy { default: rule, by_type: BTreeMap::new() }
    }

    /// The same policy with `rule` for `unit_type`.
    pub fn with_type(mut self, unit_type: &str, rule: RetentionRule) -> Self {
        self.by_type.insert(unit_type.to_owned(), rule);
        self
    }

    /// The rule of `unit_type`.
    pub fn rule(&self, unit_type: &str) -> RetentionRule {
        self.by_type.get(unit_type).copied().unwrap_or(self.default)
    }

    fn validate(&self) -> Result<(), UnitError> {
        for r in std::iter::once(&self.default).chain(self.by_type.values()) {
            if *r == RetentionRule::KeepLast(0) {
                return Err(UnitError::InvalidArgument("KeepLast(0) would drop the head".into()));
            }
        }
        Ok(())
    }
}

/// Bounds of one [`UnitDb::run_retention`] call (design §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetentionBudget {
    /// Most units the change set touches (its members), `>= 1`.
    pub max_units: usize,
    /// Most engine writes for the units (range deletions, head and
    /// `REV_META` rewrites, payload deletions, members), `>= 4`: one unit's
    /// prune needs 4. The change set record is not counted.
    pub max_ops: usize,
}

impl Default for RetentionBudget {
    /// 1 000 units, 10 000 writes.
    fn default() -> Self {
        RetentionBudget { max_units: 1_000, max_ops: 10_000 }
    }
}

/// What [`UnitDb::run_retention`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionReport {
    /// The `Retention` change set, `None` when there was nothing to do.
    pub change_set: Option<CommitInfo>,
    /// Units whose head was read.
    pub units_examined: u64,
    /// Units the change set touched (its members).
    pub units_retained: u64,
    /// `REV_META` (with their payloads) removed by range deletions.
    pub revisions_pruned: u64,
    /// `REV_PAYLOAD` removed from retained revisions under `Audit`.
    pub payloads_dropped: u64,
    /// Key and value bytes no longer reachable. They are still on disk until
    /// [`UnitDb::compact`] (and until no snapshot needs them, I-05).
    pub bytes_logically_freed: u64,
    /// `false` when the budget stopped the call before every unit was done:
    /// call again.
    pub complete: bool,
}

/// What [`UnitDb::compact`] did. The sizes are the engine's live table
/// bytes (the sum of its `l<N>_bytes` statistics); `None` when the engine
/// reports none. Files a snapshot still pins are not in the live set but are
/// still on disk until that snapshot goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompactReport {
    pub stored_bytes_before: Option<u64>,
    pub stored_bytes_after: Option<u64>,
}

impl CompactReport {
    /// `before - after` (0 if it grew), when both are known.
    pub fn bytes_reclaimed(&self) -> Option<u64> {
        Some(self.stored_bytes_before?.saturating_sub(self.stored_bytes_after?))
    }
}

/// Live table bytes from an engine's statistics: the sum of `l<N>_bytes`.
fn stored_bytes(stats: &[(String, u64)]) -> Option<u64> {
    let mut total = None;
    for (name, v) in stats {
        let level = name.strip_prefix('l').and_then(|r| r.strip_suffix("_bytes"));
        if level.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())) {
            total = Some(total.unwrap_or(0) + v);
        }
    }
    total
}

/// Length of a `REV_META`/`REV_PAYLOAD` key.
const REV_KEY_LEN: u64 = 25;

/// What retention does to one unit.
struct UnitPlan {
    /// New `oldest_rev`.
    keep_from: RevNo,
    /// Its kind (the member's kind).
    keep_kind: RevisionKind,
    /// Revisions `[oldest, keep_from)` and the bytes they free.
    pruned: u64,
    pruned_bytes: u64,
    /// Retained revisions below the head whose payload `Audit` drops.
    drops: Vec<(RevNo, RevisionMeta)>,
}

/// The first revision retention keeps, given the retained `metas`
/// (`[oldest, head]`, in order).
fn keep_from(
    metas: &[(RevNo, RevisionMeta, u64)],
    head: &UnitHead,
    policy: HistoryPolicy,
    rule: RetentionRule,
    now: Timestamp,
) -> RevNo {
    let (oldest, top) = (head.oldest_rev, head.head_rev);
    if policy == HistoryPolicy::None {
        return top;
    }
    let mut k = match rule {
        RetentionRule::KeepAll => oldest,
        RetentionRule::KeepLast(n) => RevNo(top.0.saturating_sub(n.saturating_sub(1))),
        RetentionRule::KeepSince(s) => metas.iter().rev().find(|(_, m, _)| m.commit_seq <= s).map_or(oldest, |r| r.0),
        RetentionRule::KeepFor(d) => {
            let micros = i64::try_from(d.as_micros()).unwrap_or(i64::MAX);
            let cutoff = now.0.saturating_sub(micros);
            match metas.iter().find(|(_, m, _)| m.created_at.0 >= cutoff) {
                Some((r, _, _)) => RevNo(r.0.saturating_sub(1)),
                None => top,
            }
        }
    };
    if metas.last().is_some_and(|(_, m, _)| m.kind == RevisionKind::Delete) && top.0 > 1 {
        k = k.min(RevNo(top.0 - 1));
    }
    k.max(oldest).min(top)
}

/// Every retained `REV_META` of `unit` (`[oldest, head]`), checked to be
/// contiguous with strictly growing `commit_seq` (IU-2, IU-3).
fn retained_metas<T: Txn>(
    txn: &T,
    unit: UnitId,
    head: &UnitHead,
) -> Result<Vec<(RevNo, RevisionMeta, u64)>, UnitError> {
    let (_, end) = Key::unit_rev_range(TAG_REV_META, unit);
    let mut out: Vec<(RevNo, RevisionMeta, u64)> = Vec::new();
    let mut next = head.oldest_rev;
    while next <= head.head_rev {
        let start = key(Key::RevMeta { unit, rev: next })?;
        let want = (head.head_rev.0 - next.0 + 1).min(1024) as usize;
        let page = txn.scan(Bound::Included(&start), Bound::Excluded(&end), want)?;
        if page.is_empty() {
            return Err(corruption(format!("unit {unit}: REV_META of retained revision {next} is missing")));
        }
        for (k, v) in page {
            let rev = next;
            if rev > head.head_rev {
                break;
            }
            if Key::decode(&k)? != (Key::RevMeta { unit, rev }) {
                return Err(corruption(format!("unit {unit}: REV_META of retained revision {rev} is missing")));
            }
            let meta =
                RevisionMeta::decode(rev, &v).map_err(|e| context(e, &format!("REV_META of unit {unit} rev {rev}")))?;
            if out.last().is_some_and(|(_, p, _)| meta.commit_seq <= p.commit_seq) {
                return Err(corruption(format!(
                    "unit {unit}: revision {rev} has commit_seq {} out of order (IU-3)",
                    meta.commit_seq
                )));
            }
            out.push((rev, meta, v.len() as u64));
            next = RevNo(rev.0 + 1);
        }
    }
    match out.last() {
        Some((_, m, _)) if m.commit_seq == head.updated_seq => Ok(out),
        _ => {
            Err(corruption(format!("unit {unit}: UNIT_HEAD disagrees with the REV_META of revision {}", head.head_rev)))
        }
    }
}

/// What retention does to `unit`, or `None` when nothing.
fn plan_unit<T: Txn>(
    txn: &T,
    unit: UnitId,
    head: &UnitHead,
    policy: HistoryPolicy,
    rule: RetentionRule,
    now: Timestamp,
) -> Result<Option<UnitPlan>, UnitError> {
    let idle = match policy {
        HistoryPolicy::Versioned => rule == RetentionRule::KeepAll,
        HistoryPolicy::None => head.oldest_rev == head.head_rev,
        HistoryPolicy::Audit => false,
    };
    if idle || head.oldest_rev == head.head_rev {
        return Ok(None);
    }
    let metas = retained_metas(txn, unit, head)?;
    let k = keep_from(&metas, head, policy, rule, now);
    let i = (k.0 - head.oldest_rev.0) as usize;
    let mut pruned_bytes = 0;
    for (_, m, len) in &metas[..i] {
        pruned_bytes += REV_KEY_LEN + len;
        if m.payload_present {
            pruned_bytes += REV_KEY_LEN + u64::from(m.payload_len);
        }
    }
    let drops: Vec<(RevNo, RevisionMeta)> = if policy == HistoryPolicy::Audit {
        metas[i..metas.len() - 1]
            .iter()
            .filter(|(_, m, _)| m.payload_present)
            .map(|(r, m, _)| (*r, m.clone()))
            .collect()
    } else {
        Vec::new()
    };
    if i == 0 && drops.is_empty() {
        return Ok(None);
    }
    Ok(Some(UnitPlan { keep_from: k, keep_kind: metas[i].1.kind, pruned: i as u64, pruned_bytes, drops }))
}

impl<E: TxnEngine> UnitDb<E> {
    /// Applies `policy` (and the history policy of each type, see the
    /// [module documentation](self)) to at most `budget` units in one change
    /// set with purpose `Retention`. An empty database, or one with nothing
    /// to prune, writes nothing. Call again while `complete` is `false`.
    pub fn run_retention(
        &self,
        policy: &RetentionPolicy,
        budget: RetentionBudget,
    ) -> Result<RetentionReport, UnitError> {
        self.check_poisoned()?;
        policy.validate()?;
        if budget.max_units == 0 || budget.max_ops < 4 {
            return Err(UnitError::InvalidArgument(format!(
                "retention budget {budget:?} needs max_units >= 1 and max_ops >= 4"
            )));
        }
        let mut report = RetentionReport { complete: true, ..RetentionReport::default() };
        {
            let snap = self.engine().snapshot()?;
            let mut mark = vec![TAG_LAYER_META];
            mark.extend_from_slice(LAYER_MARK_NAME.as_bytes());
            if kerno_core::contract::Snapshot::get(&snap, &mark)?.is_none() {
                // No layer mark: nothing the layer wrote, nothing to prune
                // (a store with foreign keys fails at open).
                return Ok(report);
            }
        }
        let mut cs = ChangeSet::begin_retention(self)?;
        let now = cs.now();
        let (_, end) = Key::tag_range(crate::codec::TAG_UNIT_HEAD);
        let mut cursor = vec![crate::codec::TAG_UNIT_HEAD];
        let mut inclusive = true;
        let mut policies: BTreeMap<String, HistoryPolicy> = BTreeMap::new();
        let mut ops = 0usize;
        'scan: loop {
            let start = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = cs.txn()?.scan(start, Bound::Excluded(&end), 256)?;
            let n = page.len();
            for (k, v) in &page {
                let Key::UnitHead { unit } = Key::decode(k)? else {
                    return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
                };
                let head = UnitHead::decode(v).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
                if head.oldest_rev.0 == 0 || head.oldest_rev > head.head_rev {
                    return Err(corruption(format!("unit {unit}: oldest_rev outside 1..=head_rev")));
                }
                report.units_examined += 1;
                let hp = match policies.get(&head.unit_type) {
                    Some(p) => *p,
                    None => {
                        let p = cs.policy(&head.unit_type)?;
                        policies.insert(head.unit_type.clone(), p);
                        p
                    }
                };
                let Some(plan) = plan_unit(cs.txn()?, unit, &head, hp, policy.rule(&head.unit_type), now)? else {
                    continue;
                };
                let base = if plan.pruned > 0 { 4 } else { 1 };
                let room = budget.max_ops - ops;
                if report.units_retained as usize == budget.max_units
                    || room < base + if plan.drops.is_empty() { 0 } else { 2 }
                {
                    report.complete = false;
                    break 'scan;
                }
                let take = plan.drops.len().min((room - base) / 2);
                let mut writes = Vec::with_capacity(base - 1 + 2 * take);
                if plan.pruned > 0 {
                    for tag in [TAG_REV_META, TAG_REV_PAYLOAD] {
                        let (lo, hi) = match tag {
                            TAG_REV_META => (
                                Key::RevMeta { unit, rev: head.oldest_rev },
                                Key::RevMeta { unit, rev: plan.keep_from },
                            ),
                            _ => (
                                Key::RevPayload { unit, rev: head.oldest_rev },
                                Key::RevPayload { unit, rev: plan.keep_from },
                            ),
                        };
                        writes.push(Write::DeleteRange(key(lo)?, key(hi)?));
                    }
                    let next = UnitHead { oldest_rev: plan.keep_from, ..head.clone() };
                    writes.push(Write::Put(key(Key::UnitHead { unit })?, next.encode()?));
                }
                let mut freed = plan.pruned_bytes;
                for (rev, meta) in plan.drops.iter().take(take) {
                    let rewritten = RevisionMeta { payload_present: false, ..meta.clone() };
                    writes.push(Write::Put(key(Key::RevMeta { unit, rev: *rev })?, rewritten.encode(*rev)?));
                    writes.push(Write::Delete(key(Key::RevPayload { unit, rev: *rev })?));
                    freed += REV_KEY_LEN + u64::from(meta.payload_len);
                }
                ops += writes.len() + 1;
                cs.retain(unit, plan.keep_from, plan.keep_kind, writes)?;
                report.units_retained += 1;
                report.revisions_pruned += plan.pruned;
                report.payloads_dropped += take as u64;
                report.bytes_logically_freed += freed;
                if take < plan.drops.len() {
                    report.complete = false;
                    break 'scan;
                }
            }
            match page.into_iter().last() {
                Some((k, _)) if n == 256 => {
                    cursor = k;
                    inclusive = false;
                }
                _ => break,
            }
        }
        let info = cs.commit()?;
        if report.units_retained > 0 {
            report.change_set = Some(info);
        }
        Ok(report)
    }
}

impl<E: TxnEngine + TestHooks> UnitDb<E> {
    /// Asks the engine to compact the revision ranges (`REV_META` and
    /// `REV_PAYLOAD`, design §5.1) so that what retention deleted leaves the
    /// live files. It delegates to the engine's directed compaction
    /// (`compact_range`); an engine without it answers its own error
    /// (`InvalidArgument("unsupported")`). What a snapshot still sees stays
    /// (I-05).
    pub fn compact(&self) -> Result<CompactReport, UnitError> {
        self.check_poisoned()?;
        let before = stored_bytes(&self.engine().stats());
        self.engine().compact_range(Bound::Included(&[TAG_REV_META]), Bound::Excluded(&[TAG_REV_PAYLOAD + 1]))?;
        let after = stored_bytes(&self.engine().stats());
        Ok(CompactReport { stored_bytes_before: before, stored_bytes_after: after })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_bytes_sums_levels_only() {
        let s = |v: &[(&str, u64)]| v.iter().map(|(n, x)| (n.to_string(), *x)).collect::<Vec<_>>();
        assert_eq!(stored_bytes(&s(&[])), None);
        assert_eq!(stored_bytes(&s(&[("l0_bytes", 5), ("l6_bytes", 7), ("l0_files", 9), ("wal_bytes", 3)])), Some(12));
        assert_eq!(stored_bytes(&s(&[("l_bytes", 5), ("lx_bytes", 1)])), None);
    }

    #[test]
    fn report_reclaimed() {
        let r = CompactReport { stored_bytes_before: Some(10), stored_bytes_after: Some(4) };
        assert_eq!(r.bytes_reclaimed(), Some(6));
        assert_eq!(CompactReport { stored_bytes_after: None, ..r }.bytes_reclaimed(), None);
        assert_eq!(CompactReport { stored_bytes_after: Some(11), ..r }.bytes_reclaimed(), Some(0));
    }
}
