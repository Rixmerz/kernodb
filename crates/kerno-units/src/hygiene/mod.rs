//! Memory hygiene for agents (ROADMAP Phase B): importance, expiry (TTL),
//! deduplication and summarizing old units.
//!
//! # Where the data lives
//!
//! **No on-disk format changes.** `UNIT_HEAD`, `REV_META` and the verifier
//! are frozen by `units-v1.md`, and a new column would reach `verify`,
//! `export`, `import` and every caller that builds a [`crate::NewUnit`]. So
//! importance and TTL live in the unit's `metadata` (see [`Hygiene`]), which
//! is versioned with each revision: a change of importance or TTL is a
//! revision with its own change set, readable in the history. A unit whose
//! metadata does not start with the hygiene marker has importance 0 and no
//! TTL. Metadata that starts with the marker is reserved to this module.
//!
//! # What each part does
//!
//! * [`UnitDb::expire_due`]: archives, in one `Application` change set, the
//!   active units whose TTL has passed. Archiving is a revision, so an
//!   expired unit leaves the active set but stays readable, in the history,
//!   and `unarchive` brings it back. The pass is bounded, idempotent and
//!   reports what it did, like [`UnitDb::run_retention`].
//! * [`UnitDb::dedup_text`]: links near-identical units of one type with a
//!   [`DUPLICATE_OF`] relation (exact content hash first, normalized text
//!   second). It deletes nothing.
//! * [`UnitDb::consolidate_old`]: asks a caller-supplied
//!   [`crate::Summarizer`] (the trait of [`crate::consolidation`]; no model
//!   and no network in this crate, I-31) for one text over old units, stores
//!   it as one [`SUMMARY_TYPE`] unit and archives the sources, all in one
//!   change set. The provenance is the summary's [`SummaryProvenance`].
//!
//! # The relation rule
//!
//! The layer refuses to archive or delete a unit while a relation touches it
//! (ADR-0014). The hygiene passes therefore remove the [`DUPLICATE_OF`]
//! relations of a unit they retire, in the same change set, and count them in
//! the report. A unit with any other relation is left alone and reported as
//! blocked: relations a caller declared are never deleted here. For the same
//! reason a summary cannot be linked to its sources by relations (they would
//! forbid archiving them), so its provenance is in its metadata.

mod dedup;
mod expiry;
mod meta;
mod summarize;

#[cfg(test)]
mod testutil;

pub use dedup::{DUPLICATE_OF, DedupReport, normalize_text};
pub use expiry::ExpiryReport;
pub use meta::{Hygiene, MAX_IMPORTANCE};
pub use summarize::{Consolidated, Consolidation, MAX_CONSOLIDATED_SOURCES, SUMMARY_TYPE, SummaryProvenance};

use std::collections::BTreeSet;

use kerno_core::contract::{Engine, TxnEngine};

use crate::db::{ChangeMeta, ChangeSet, UnitSnapshot};
use crate::error::UnitError;
use crate::types::{Direction, RelationId, RevNo, UnitId};

/// The actor recorded on every change set a hygiene pass writes.
pub const HYGIENE_ACTOR: &[u8] = b"kerno.hygiene";

fn change_meta(reason: String) -> ChangeMeta {
    ChangeMeta { reason: Some(reason), actor: Some(HYGIENE_ACTOR.to_vec()), ..ChangeMeta::default() }
}

/// The relations to remove so that `unit` can be archived: every relation of
/// the unit, in either direction, when all are [`DUPLICATE_OF`]; `None` when
/// some other relation touches it.
fn retirable_relations<E: Engine>(snap: &UnitSnapshot<E>, unit: UnitId) -> Result<Option<Vec<RelationId>>, UnitError> {
    let mut ids = BTreeSet::new();
    for dir in [Direction::Out, Direction::In] {
        for r in snap.relations_of(unit, dir, None)? {
            if r.relation_type != DUPLICATE_OF {
                return Ok(None);
            }
            ids.insert(r.id);
        }
    }
    Ok(Some(ids.into_iter().collect()))
}

/// Archives `unit` at `rev` after deleting `relations` (from
/// [`retirable_relations`]). `gone` holds the relations this change set
/// already deleted: one relation joins two retired units and is deleted once.
/// Returns how many relations this call deleted.
fn retire<E: TxnEngine>(
    cs: &mut ChangeSet<'_, E>,
    unit: UnitId,
    rev: RevNo,
    relations: &[RelationId],
    gone: &mut BTreeSet<RelationId>,
) -> Result<u64, UnitError> {
    let mut removed = 0;
    for r in relations {
        if gone.insert(*r) {
            cs.delete_relation(*r)?;
            removed += 1;
        }
    }
    cs.archive(unit, Some(rev))?;
    Ok(removed)
}
