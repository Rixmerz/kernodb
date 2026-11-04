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
//! Relations are not defined yet (E4-02, ADR-0014): there is no relation
//! query here. E4-02 adds its reference traversal in this module before any
//! relation query is published.

use std::ops::Bound;

use kerno_core::contract::{Engine, Snapshot};

use crate::codec::{Key, TAG_UNIT_HEAD};
use crate::db::{Unit, UnitSnapshot, corruption};
use crate::error::UnitError;
use crate::types::{Lifecycle, UnitId};

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
