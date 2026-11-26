//! E4-01: the reference query path (stage4-record.md §3).
//!
//! A full scan over one [`UnitSnapshot`], with no index and no shortcut. It
//! is correct by definition and slow by design: later indexes (E4-03,
//! E4-04) are checked against it on the same snapshot (I-32 applied to
//! indexes), so it must stay the simplest thing that can be right.
//!
//! Definitions:
//!
//! * A query sees the **active** units of the snapshot: units whose head
//!   lifecycle is [`Lifecycle::Active`]. Archived and deleted units are
//!   excluded, as are units created after the snapshot (they are not in
//!   it).
//! * Results are in unit id order.
//! * Every payload returned was verified (the same read as `get_unit`); a
//!   corrupt unit the query would return fails the whole query instead of
//!   being skipped. A filter by type or schema version is decided on the head
//!   alone, so the rest of a unit it excludes is not read.
//!
//! Relations (E4-02, ADR-0014): [`UnitSnapshot::reference_relations`] reads
//! every `REL` record and never an adjacency entry, so it is the definition
//! of what [`UnitSnapshot::relations_of`] (the index path) must return on the
//! same snapshot. It does not filter by the lifecycle of the ends: the layer
//! keeps both ends of every relation `Active` (archiving or deleting a unit
//! with relations is refused), and the verifier checks it (UNIT-36).

use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot};

use crate::codec::{Key, RelationRecord, TAG_REL, TAG_UNIT_HEAD, UnitHead};
use crate::db::{Relation, Unit, UnitPages, UnitSnapshot, context, corruption, scan_all, to_unit};
use crate::error::UnitError;
use crate::index::UnitFilter;
use crate::types::{Direction, Lifecycle, UnitId};

/// Heads read per engine scan call.
const PAGE: usize = 256;

impl<E: Engine> UnitSnapshot<E> {
    /// The active unit `id` at this snapshot, or `None` if it never existed
    /// or is archived or deleted.
    pub fn reference_get(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        Ok(self.get_unit(id)?.filter(|u| u.lifecycle == Lifecycle::Active))
    }

    /// Every active unit at this snapshot for which `pred` holds, in unit id
    /// order. Reads every unit head: linear in the number of units.
    pub fn reference_scan(&self, mut pred: impl FnMut(&Unit) -> bool) -> Result<Vec<Unit>, UnitError> {
        Ok(self.scan_units(|_| true)?.into_iter().filter(|u| pred(u)).collect())
    }

    /// Every active unit whose head `want`s, in unit id order, each read and verified like
    /// [`get_unit`](Self::get_unit). The head comes from the scan, so a unit costs two more
    /// reads (its `REV_META` and its payload), and a unit `want` refuses costs none.
    fn scan_units(&self, mut want: impl FnMut(&UnitHead) -> bool) -> Result<Vec<Unit>, UnitError> {
        let (_, end) = Key::tag_range(TAG_UNIT_HEAD);
        let mut cursor = vec![TAG_UNIT_HEAD];
        let mut inclusive = true;
        let mut out = Vec::new();
        let pages = UnitPages::new(&self.snap, self.checked_reads, self.verify_payload_on_read);
        loop {
            let start = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = self.snap.scan(start, Bound::Excluded(&end), PAGE)?;
            let n = page.len();
            for (k, v) in &page {
                let Key::UnitHead { unit } = Key::decode(k)? else {
                    return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
                };
                let head = UnitHead::decode(v).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
                if head.lifecycle != Lifecycle::Active || !want(&head) {
                    continue;
                }
                // The scanned head stands for the read of its key.
                let Some((head, payload)) = pages.load(unit, Some((k, v)))? else {
                    return Err(corruption(format!("unit {unit}: UNIT_HEAD vanished inside a snapshot")));
                };
                out.push(to_unit(unit, head, payload));
            }
            match page.into_iter().last() {
                Some((k, _)) if n == PAGE => {
                    cursor = k;
                    inclusive = false;
                }
                _ => return Ok(out),
            }
        }
    }

    /// Every active unit of type `unit_type`, in unit id order.
    pub fn reference_by_type(&self, unit_type: &str) -> Result<Vec<Unit>, UnitError> {
        self.reference_scan(|u| u.unit_type == unit_type)
    }

    /// Every active unit matching `filter`, in unit id order: the definition
    /// of what [`UnitSnapshot::indexed_scan`] (the index path) must return on
    /// the same snapshot (E4-03, I-32).
    pub fn reference_filter(&self, filter: &UnitFilter) -> Result<Vec<Unit>, UnitError> {
        self.scan_units(|h| filter.matches(&h.unit_type, h.schema_version))
    }
}

impl<E: Engine> UnitSnapshot<E> {
    /// Every relation at this snapshot for which `pred` holds, in relation id
    /// order. Reads every `REL` record: linear in the number of relations.
    pub fn reference_relations(&self, mut pred: impl FnMut(&Relation) -> bool) -> Result<Vec<Relation>, UnitError> {
        let (start, end) = Key::tag_range(TAG_REL);
        let mut out = Vec::new();
        scan_all(&self.snap, start, end, |k, v| {
            let Key::Relation { edge } = Key::decode(k)? else {
                return Err(corruption("a key in the REL range is not a REL".into()));
            };
            let record = RelationRecord::decode(v).map_err(|e| crate::db::context(e, &format!("REL {edge}")))?;
            let rel = Relation::new(edge, record);
            if pred(&rel) {
                out.push(rel);
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// The reference answer of [`UnitSnapshot::relations_of`]: a filter over
    /// every relation, sorted like the index (relation type, then id).
    pub fn reference_relations_of(
        &self,
        unit: UnitId,
        dir: Direction,
        relation_type: Option<&str>,
    ) -> Result<Vec<Relation>, UnitError> {
        let mut out = self.reference_relations(|r| {
            let here = if dir == Direction::Out { r.from } else { r.to };
            here == unit && relation_type.is_none_or(|t| t == r.relation_type)
        })?;
        out.sort_by(|a, b| (a.relation_type.as_bytes(), a.id).cmp(&(b.relation_type.as_bytes(), b.id)));
        Ok(out)
    }
}
