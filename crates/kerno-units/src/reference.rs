//! E4-01: the reference query path (etapa4-acta.md §3).
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
//!   corrupt unit fails the whole query instead of being skipped.
//!
//! Relations (E4-02, ADR-0014): [`UnitSnapshot::reference_relations`] reads
//! every `REL` record and never an adjacency entry, so it is the definition
//! of what [`UnitSnapshot::relations_of`] (the index path) must return on the
//! same snapshot. It does not filter by the lifecycle of the ends: the layer
//! keeps both ends of every relation `Active` (archiving or deleting a unit
//! with relations is refused), and the verifier checks it (UNIT-36).

use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot};

use crate::codec::{Key, RelationRecord, TAG_REL, TAG_UNIT_HEAD};
use crate::db::{Relation, Unit, UnitSnapshot, corruption, scan_all};
use crate::error::UnitError;
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
        let (_, end) = Key::tag_range(TAG_UNIT_HEAD);
        let mut cursor = vec![TAG_UNIT_HEAD];
        let mut inclusive = true;
        let mut out = Vec::new();
        loop {
            let start = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
            let page = self.snap.scan(start, Bound::Excluded(&end), PAGE)?;
            let n = page.len();
            for (k, _) in &page {
                let Key::UnitHead { unit } = Key::decode(k)? else {
                    return Err(corruption("a key in the UNIT_HEAD range is not a UNIT_HEAD".into()));
                };
                // The head may not vanish inside one snapshot.
                let Some(u) = self.get_unit(unit)? else {
                    return Err(corruption(format!("unit {unit}: UNIT_HEAD vanished inside a snapshot")));
                };
                if u.lifecycle == Lifecycle::Active && pred(&u) {
                    out.push(u);
                }
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
