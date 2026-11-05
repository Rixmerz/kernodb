//! E4-03: the primary index and basic filters (etapa4-acta.md §3, ADR-0007).
//!
//! The primary index lists the **active** units of a snapshot in unit id
//! order, each with the `UNIT_HEAD` fields a basic filter reads
//! ([`UnitFilter`]: `unit_type`, `schema_version`). A filtered query scans
//! those entries, applies the filter to them, and opens only the units that
//! match (a verified read, like `get_unit`): it never reads the revisions or
//! payloads of the units it excludes, nor the heads of archived and deleted
//! ones. The reference path ([`UnitSnapshot::reference_filter`]) is the
//! definition of its answer: on the same snapshot both return the same units
//! in the same order (I-32 applied to indexes).
//!
//! # Freshness (I-23)
//!
//! The index is **derived**: [`UnitDb::rebuild_index`] writes it, and a change
//! set does not maintain it. Its state is always relative to the snapshot that
//! asks:
//!
//! | State | Meaning |
//! |---|---|
//! | `Ready` | The entries were written by the commit the snapshot is at (watermark = snapshot sequence): no commit came after them, so they are exactly the active units of the snapshot. |
//! | `Stale` | A commit came after the watermark, or no build ever happened (no watermark). The entries, if any, describe an older state. |
//! | `Failed` | The last build attempt failed (a damaged head, a batch over the engine limit). The entries, if any, are those of the watermark. |
//! | `Building` | A build is waiting for the write slot or running **on this handle**. Only [`UnitDb::index_status`] reports it: a snapshot is a view of stored data, and a build in flight has stored nothing yet. |
//!
//! `Ready` and `Failed` are stored (`IDX_META`, `units-v1.md` §3.9), so the
//! status survives a crash and a reopen; `Stale` and `Building` are never
//! stored. The **watermark** is the commit sequence of the change set that
//! wrote the entries. That change set changes no unit, so the entries are a
//! function of the state at that very commit, and the build can write its own
//! sequence (it is predicted, as in every change set, design §2.5). Any later
//! commit makes the index `Stale`, including one that touches no unit
//! (conservative on purpose: a query never claims freshness it cannot prove).
//!
//! # Contracts
//!
//! * **A query does not declare freshness it lacks.** [`UnitSnapshot::indexed_scan`]
//!   answers only when the index is `Ready` at that snapshot and refuses with
//!   [`UnitError::IndexNotReady`] otherwise; [`UnitSnapshot::query`] falls back
//!   to the reference path and says so in [`QuerySource`], with the status that
//!   forced it. Older snapshots keep their own status: one taken right after a
//!   build stays `Ready` however many commits follow.
//! * **A build is one change set (I-21).** It deletes the previous entries,
//!   writes the new ones and the `IDX_META` record in a single engine
//!   transaction, under the write slot, so a crash leaves the previous
//!   generation or the whole new one, never a mix. A rebuilt index equals a
//!   fresh one (I-24): the entries are a function of the heads alone, and
//!   the previous ones are removed first.
//! * **A failed build changes no entry.** It records `Failed` (with the
//!   previous watermark) unless the engine itself is unusable (`Io`,
//!   `Poisoned`), where the record could not be written either.
//! * **Only heads are read by a build**, not revisions or payloads: deeper
//!   damage is for `verify_units` (K-E3-7), and an indexed query verifies each
//!   unit it returns. So on a damaged store the index path can answer where
//!   the reference path, which opens every unit, fails.
//!
//! ponytail: a build holds every entry in memory and writes them in one
//! batch, so it is limited by the engine's batch limits (1 Mi operations and
//! 64 MiB by default; beyond them it fails with `ChangeSetTooLarge` and the
//! status is `Failed`). Upgrade path: build in key-ordered chunks under a
//! `Building` record, which is the E4-04 question.

use std::ops::Bound;
use std::sync::atomic::{AtomicUsize, Ordering};

use kerno_core::contract::{Engine, Snapshot, Txn, TxnEngine};

use crate::codec::{IndexMeta, Key, PrimaryEntry, TAG_IDX_PRIMARY, TAG_UNIT_HEAD, UnitHead};
use crate::db::{ChangeSet, Unit, UnitDb, UnitSnapshot, Write, context, corruption, key, scan_all};
use crate::error::UnitError;
use crate::types::{BuildOutcome, CommitSeq, Lifecycle, UnitId};

/// Heads read per scan call by a build.
const HEAD_PAGE: usize = 256;

/// Freshness of the primary index (I-23); see the [module documentation](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndexState {
    Ready,
    Building,
    Stale,
    Failed,
}

/// The state of the primary index and the commit its entries reflect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexStatus {
    pub state: IndexState,
    /// The commit sequence the entries reflect; `None` when no build ever
    /// succeeded.
    pub watermark: Option<CommitSeq>,
}

/// A basic filter: every condition that is set must hold. The default filter
/// matches every active unit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitFilter {
    pub unit_type: Option<String>,
    pub schema_version: Option<u32>,
}

impl UnitFilter {
    pub fn matches(&self, unit_type: &str, schema_version: u32) -> bool {
        self.unit_type.as_deref().is_none_or(|t| t == unit_type)
            && self.schema_version.is_none_or(|v| v == schema_version)
    }
}

/// Where a [`UnitSnapshot::query`] answer came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuerySource {
    /// The primary index, `Ready` at the snapshot, with this watermark.
    Index { watermark: CommitSeq },
    /// The reference path, because the index was in this status.
    Reference { index: IndexStatus },
}

/// The units a query matched and the path that found them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryOutcome {
    pub units: Vec<Unit>,
    pub source: QuerySource,
}

/// Counts a build from the moment it is requested until it ends, whatever
/// the outcome.
struct Queued<'a>(&'a AtomicUsize);

impl<'a> Queued<'a> {
    fn enter(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Queued(count)
    }
}

impl Drop for Queued<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// Rebuilds the primary index from the units at the latest commit (and
    /// builds it if there is none): one change set with purpose `Index` that
    /// replaces every entry and the `IDX_META` record, and returns the new
    /// status (`Ready`, watermark = that commit). Blocks while another change
    /// set holds the write slot; meanwhile [`UnitDb::index_status`] says
    /// `Building`.
    ///
    /// On failure nothing of the new generation is stored and the error is
    /// returned; unless the engine is unusable, the status becomes `Failed`
    /// (see the [module documentation](self)). Calling it again after the cause
    /// is fixed rebuilds from scratch.
    pub fn rebuild_index(&self) -> Result<IndexStatus, UnitError> {
        let _queued = Queued::enter(&self.index_builds);
        let mut cs = ChangeSet::begin_index(self)?;
        match write_generation(&mut cs) {
            Ok(()) => {
                let watermark = cs.commit()?.commit_seq;
                Ok(IndexStatus { state: IndexState::Ready, watermark: Some(watermark) })
            }
            Err(e) => {
                drop(cs); // aborts: releases the write slot, stores nothing
                if !matches!(&e, UnitError::Storage(kerno_core::Error::Io(_) | kerno_core::Error::Poisoned(_))) {
                    // The build error is what the caller needs. If the marker
                    // cannot be written either (a damaged IDX_META, a full
                    // disk), the stored status stays what it was, which is
                    // still true: nothing claims a freshness the index lacks.
                    let _ = record_failure(self);
                }
                Err(e)
            }
        }
    }

    /// The status of the primary index as of the latest commit, with
    /// `Building` while a [`rebuild_index`](Self::rebuild_index) of this handle
    /// is waiting or running.
    pub fn index_status(&self) -> Result<IndexStatus, UnitError> {
        let status = self.snapshot()?.index_status()?;
        if self.index_builds.load(Ordering::SeqCst) > 0 {
            return Ok(IndexStatus { state: IndexState::Building, ..status });
        }
        Ok(status)
    }
}

/// Deletes the previous generation, writes the entries of every active unit
/// and the `IDX_META` that says what they reflect.
fn write_generation<E: TxnEngine>(cs: &mut ChangeSet<'_, E>) -> Result<(), UnitError> {
    let entries = active_entries(cs)?;
    // The range delete goes first: inside a batch it hides what came before
    // it, not what comes after (SEM-3).
    let (start, end) = Key::tag_range(TAG_IDX_PRIMARY);
    cs.write(&Write::DeleteRange(start, end))?;
    for (unit, entry) in &entries {
        cs.write(&Write::Put(key(Key::IndexPrimary { unit: *unit })?, entry.encode()?))?;
    }
    let meta =
        IndexMeta { outcome: BuildOutcome::Succeeded, watermark: cs.commit_seq(), entries: entries.len() as u64 };
    cs.write(&Write::Put(key(Key::IndexMeta)?, meta.encode()?))
}

/// The entry of every `Active` unit, from `UNIT_HEAD` alone, in unit id order.
fn active_entries<E: TxnEngine>(cs: &ChangeSet<'_, E>) -> Result<Vec<(UnitId, PrimaryEntry)>, UnitError> {
    let (_, end) = Key::tag_range(TAG_UNIT_HEAD);
    let mut cursor = vec![TAG_UNIT_HEAD];
    let mut inclusive = true;
    let mut out = Vec::new();
    loop {
        let lo = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
        let page = cs.txn()?.scan(lo, Bound::Excluded(end.as_slice()), HEAD_PAGE)?;
        let n = page.len();
        for (k, v) in &page {
            let Key::UnitHead { unit } = Key::decode(k)? else {
                return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
            };
            let head = UnitHead::decode(v).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
            if head.lifecycle == Lifecycle::Active {
                out.push((unit, PrimaryEntry { unit_type: head.unit_type, schema_version: head.schema_version }));
            }
        }
        match page.into_iter().last() {
            Some((k, _)) if n == HEAD_PAGE => {
                cursor = k;
                inclusive = false;
            }
            _ => return Ok(out),
        }
    }
}

/// Stores that the last build failed, keeping the watermark and entry count
/// of the generation still stored.
fn record_failure<E: TxnEngine>(db: &UnitDb<E>) -> Result<(), UnitError> {
    let mut cs = ChangeSet::begin_index(db)?;
    let previous = match cs.txn()?.get(&key(Key::IndexMeta)?)? {
        Some(raw) => IndexMeta::decode(&raw).map_err(|e| context(e, "IDX_META"))?,
        None => IndexMeta { outcome: BuildOutcome::Failed, watermark: CommitSeq(0), entries: 0 },
    };
    let meta = IndexMeta { outcome: BuildOutcome::Failed, ..previous };
    cs.write(&Write::Put(key(Key::IndexMeta)?, meta.encode()?))?;
    cs.commit()?;
    Ok(())
}

impl<E: Engine> UnitSnapshot<E> {
    /// The status of the primary index as this snapshot sees it: `Ready`,
    /// `Stale` or `Failed` (never `Building`, see the [module
    /// documentation](self)). A damaged `IDX_META` is an error, not a state.
    pub fn index_status(&self) -> Result<IndexStatus, UnitError> {
        let Some(raw) = self.snap.get(&key(Key::IndexMeta)?)? else {
            return Ok(IndexStatus { state: IndexState::Stale, watermark: None });
        };
        let meta = IndexMeta::decode(&raw).map_err(|e| context(e, "IDX_META"))?;
        let seen = self.seq();
        if meta.watermark > seen {
            return Err(corruption(format!("IDX_META watermark {} is after the snapshot {seen}", meta.watermark)));
        }
        let state = match meta.outcome {
            BuildOutcome::Failed => IndexState::Failed,
            BuildOutcome::Succeeded if meta.watermark == seen => IndexState::Ready,
            BuildOutcome::Succeeded => IndexState::Stale,
        };
        Ok(IndexStatus { state, watermark: (meta.watermark.0 != 0).then_some(meta.watermark) })
    }

    /// Every active unit matching `filter`, in unit id order, found through
    /// the primary index. Refuses with [`UnitError::IndexNotReady`] unless the
    /// index is `Ready` at this snapshot. Each unit returned is read and
    /// verified like `get_unit`, and its entry must agree with its head
    /// (`Storage(Corruption)` otherwise). On the same snapshot the answer is
    /// [`UnitSnapshot::reference_filter`]'s (I-32).
    pub fn indexed_scan(&self, filter: &UnitFilter) -> Result<Vec<Unit>, UnitError> {
        let status = self.index_status()?;
        match (status.state, status.watermark) {
            (IndexState::Ready, Some(_)) => self.scan_entries(filter),
            (state, watermark) => Err(UnitError::IndexNotReady { state, watermark }),
        }
    }

    /// [`indexed_scan`](Self::indexed_scan) when the index is `Ready` at this
    /// snapshot, else [`reference_filter`](Self::reference_filter); the
    /// outcome says which and why, so the caller always knows what freshness
    /// stands behind the answer (I-23).
    pub fn query(&self, filter: &UnitFilter) -> Result<QueryOutcome, UnitError> {
        let status = self.index_status()?;
        match (status.state, status.watermark) {
            (IndexState::Ready, Some(watermark)) => {
                Ok(QueryOutcome { units: self.scan_entries(filter)?, source: QuerySource::Index { watermark } })
            }
            _ => Ok(QueryOutcome {
                units: self.reference_filter(filter)?,
                source: QuerySource::Reference { index: status },
            }),
        }
    }

    /// The entries that match `filter`, opened as units.
    fn scan_entries(&self, filter: &UnitFilter) -> Result<Vec<Unit>, UnitError> {
        let (start, end) = Key::tag_range(TAG_IDX_PRIMARY);
        let mut out = Vec::new();
        scan_all(&self.snap, start, end, |k, v| {
            let Key::IndexPrimary { unit } = Key::decode(k)? else {
                return Err(corruption("a key in the IDX_PRIMARY range is not an IDX_PRIMARY".into()));
            };
            let entry = PrimaryEntry::decode(v).map_err(|e| context(e, &format!("IDX_PRIMARY of unit {unit}")))?;
            if !filter.matches(&entry.unit_type, entry.schema_version) {
                return Ok(());
            }
            let Some(u) = self.get_unit(unit)? else {
                return Err(corruption(format!("IDX_PRIMARY entry of unit {unit} has no UNIT_HEAD")));
            };
            if u.lifecycle != Lifecycle::Active
                || u.unit_type != entry.unit_type
                || u.schema_version != entry.schema_version
            {
                return Err(corruption(format!("IDX_PRIMARY entry of unit {unit} disagrees with its UNIT_HEAD")));
            }
            out.push(u);
            Ok(())
        })?;
        Ok(out)
    }
}
