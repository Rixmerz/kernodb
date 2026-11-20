//! TTL expiry: [`UnitDb::expire_due`].

use std::collections::BTreeSet;

use kerno_core::contract::TxnEngine;

use super::{Hygiene, change_meta, retirable_relations, retire};
use crate::db::{CommitInfo, UnitDb};
use crate::error::UnitError;
use crate::types::{RevNo, UnitId};

/// What [`UnitDb::expire_due`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpiryReport {
    /// The change set that archived the expired units; `None` when there was
    /// nothing to archive.
    pub change_set: Option<CommitInfo>,
    /// Units whose TTL had passed when the pass read them.
    pub units_due: u64,
    /// Units archived, in unit id order.
    pub expired: Vec<UnitId>,
    /// Due units left active because a relation that is not
    /// [`super::DUPLICATE_OF`] touches them (the layer refuses to archive
    /// them). They are still due: they come back on every call until the
    /// relation is deleted.
    pub blocked: Vec<UnitId>,
    /// `DUPLICATE_OF` relations deleted so that the units could be archived.
    pub relations_removed: u64,
    /// `false` when `max_units` or the change-set limit stopped the pass
    /// before every due unit was done: call again.
    pub complete: bool,
}

impl<E: TxnEngine> UnitDb<E> {
    /// Archives, in one change set, at most `max_units` active units whose
    /// TTL has passed by the injected clock (see [`Hygiene`]).
    ///
    /// An expired unit does not vanish: archiving is a revision with this
    /// change set's reason and actor ([`super::HYGIENE_ACTOR`]), the unit
    /// stays readable and in the history, and `unarchive` undoes it. Like
    /// [`UnitDb::run_retention`], the pass plans on a snapshot, writes
    /// under the write slot, is bounded, and is idempotent: a unit already
    /// archived is not due again. A unit changed between the snapshot and the
    /// write is [`UnitError::Conflict`] and nothing is written: call again.
    /// Archived memory still holds its payload; [`UnitDb::reclaim`] is about
    /// deleted data and does not apply to it.
    // ponytail: reads every unit head each call (linear in units, like the
    // reference path); a TTL index would make it proportional to the due units.
    pub fn expire_due(&self, max_units: usize) -> Result<ExpiryReport, UnitError> {
        if max_units == 0 {
            return Err(UnitError::InvalidArgument("max_units must be at least 1".into()));
        }
        let now = self.config().clock.now();
        let snap = self.snapshot()?;
        let mut due: Vec<(UnitId, RevNo)> = Vec::new();
        let mut failure: Option<UnitError> = None;
        snap.reference_scan(|u| {
            match Hygiene::of(u) {
                Ok(h) if h.is_expired(now) => due.push((u.id, u.head)),
                Ok(_) => {}
                Err(e) => {
                    failure.get_or_insert(e);
                }
            }
            false
        })?;
        if let Some(e) = failure {
            return Err(e);
        }
        let mut report = ExpiryReport { units_due: due.len() as u64, complete: true, ..ExpiryReport::default() };
        let max_ops = self.config().max_changeset_ops;
        let mut cs = self.begin_changeset(change_meta("kerno.hygiene: ttl expired".into()))?;
        let mut gone = BTreeSet::new();
        let mut ops = 0usize;
        for (id, rev) in due {
            let Some(relations) = retirable_relations(&snap, id)? else {
                report.blocked.push(id);
                continue;
            };
            if report.expired.len() == max_units || ops + 1 + relations.len() > max_ops {
                report.complete = false;
                break;
            }
            ops += 1 + relations.len();
            report.relations_removed += retire(&mut cs, id, rev, &relations, &mut gone)?;
            report.expired.push(id);
        }
        if report.expired.is_empty() {
            cs.abort();
        } else {
            report.change_set = Some(cs.commit()?);
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ChangeMeta;
    use crate::history::{HistoryQuery, Order};
    use crate::hygiene::DUPLICATE_OF;
    use crate::hygiene::testutil::{HOUR, T0, fixture};
    use crate::types::{Lifecycle, RelationProvenance, Timestamp};

    fn ttl(micros_from_t0: i64) -> Vec<u8> {
        Hygiene { expires_at: Some(Timestamp(T0.0 + micros_from_t0)), ..Hygiene::default() }.encode().unwrap()
    }

    #[test]
    fn archives_only_what_is_due_and_keeps_it_in_history() {
        let f = fixture();
        let short = f.make("note", b"short lived", ttl(HOUR));
        let long = f.make("note", b"long lived", ttl(10 * HOUR));
        let plain = f.make("note", b"no ttl", b"plain".to_vec());

        let r = f.db.expire_due(10).unwrap();
        assert_eq!((r.units_due, r.change_set.is_none(), r.complete), (0, true, true), "nothing due yet");

        f.advance(2 * HOUR);
        let r = f.db.expire_due(10).unwrap();
        assert_eq!((r.expired.clone(), r.units_due, r.complete), (vec![short], 1, true));
        let info = r.change_set.expect("one change set");

        let gone = f.db.get_unit(short).unwrap().expect("expired unit is still readable");
        assert_eq!(gone.lifecycle, Lifecycle::Archived);
        assert_eq!(gone.payload.as_deref(), Some(&b"short lived"[..]));
        for kept in [long, plain] {
            assert_eq!(f.db.get_unit(kept).unwrap().unwrap().lifecycle, Lifecycle::Active);
        }
        assert_eq!(f.db.get_unit(plain).unwrap().unwrap().metadata, b"plain");

        // It left the active set but not the history: an Archive revision
        // with the pass's reason and actor.
        let q = HistoryQuery { from: None, to: None, order: Order::Asc, limit: 10, with_payload: false };
        let revs = f.db.history(short, q).unwrap();
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[1].kind, crate::types::RevisionKind::Archive);
        let cs = f.db.snapshot().unwrap().change_set(info.change_set).unwrap().unwrap();
        assert_eq!(cs.reason.as_deref(), Some("kerno.hygiene: ttl expired"));
        assert_eq!(cs.provenance.actor.as_deref(), Some(super::super::HYGIENE_ACTOR));
        assert!(f.db.snapshot().unwrap().reference_by_type("note").unwrap().iter().all(|u| u.id != short));

        // Idempotent.
        let again = f.db.expire_due(10).unwrap();
        assert_eq!((again.units_due, again.change_set.is_none()), (0, true));
    }

    #[test]
    fn is_bounded_and_resumes() {
        let f = fixture();
        let ids: Vec<_> = (0..5).map(|i| f.make("note", &[i], ttl(HOUR))).collect();
        f.advance(2 * HOUR);
        let first = f.db.expire_due(2).unwrap();
        assert_eq!((first.expired.len(), first.complete), (2, false));
        let second = f.db.expire_due(10).unwrap();
        assert_eq!((second.expired.len(), second.complete), (3, true));
        let mut all = first.expired;
        all.extend(second.expired);
        assert_eq!(all, ids);
        assert!(f.db.expire_due(0).is_err());
    }

    #[test]
    fn a_declared_relation_blocks_and_is_reported_not_deleted() {
        let f = fixture();
        let a = f.make("note", b"a", ttl(HOUR));
        let b = f.make("note", b"b", Vec::new());
        let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
        let rel = cs.create_relation(a, b, "cites", RelationProvenance::Declared).unwrap();
        cs.commit().unwrap();
        f.advance(2 * HOUR);
        let r = f.db.expire_due(10).unwrap();
        assert_eq!((r.expired.len(), r.blocked.clone(), r.change_set.is_none()), (0, vec![a], true));
        assert!(f.db.snapshot().unwrap().get_relation(rel).unwrap().is_some(), "the relation is untouched");
        assert_eq!(f.db.get_unit(a).unwrap().unwrap().lifecycle, Lifecycle::Active);
    }

    #[test]
    fn duplicate_links_are_removed_once_even_between_two_expiring_units() {
        let f = fixture();
        let a = f.make("note", b"a", ttl(HOUR));
        let b = f.make("note", b"b", ttl(HOUR));
        let c = f.make("note", b"c", Vec::new());
        let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
        let p = || RelationProvenance::Algorithmic { origin: b"test".to_vec() };
        cs.create_relation(a, b, DUPLICATE_OF, p()).unwrap();
        cs.create_relation(b, c, DUPLICATE_OF, p()).unwrap();
        cs.commit().unwrap();
        f.advance(2 * HOUR);
        let r = f.db.expire_due(10).unwrap();
        assert_eq!((r.expired, r.relations_removed, r.complete), (vec![a, b], 2, true));
        let snap = f.db.snapshot().unwrap();
        assert!(snap.reference_relations(|_| true).unwrap().is_empty());
        assert_eq!(snap.get_unit(c).unwrap().unwrap().lifecycle, Lifecycle::Active);
    }

    #[test]
    fn corrupt_hygiene_metadata_fails_loudly() {
        let f = fixture();
        f.make("note", b"x", b"KHYG\x09junk".to_vec());
        assert!(f.db.expire_due(10).is_err(), "a unit is never silently treated as having no TTL");
    }

    #[test]
    fn set_hygiene_is_a_revision_and_keeps_user_metadata() {
        let f = fixture();
        let id = f.make("note", b"x", b"mine".to_vec());
        let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.set_hygiene(id, 800, Some(Timestamp(T0.0 + HOUR))).unwrap();
        cs.commit().unwrap();
        let u = f.db.get_unit(id).unwrap().unwrap();
        assert_eq!(u.head.0, 2);
        assert_eq!(
            Hygiene::of(&u).unwrap(),
            Hygiene { importance: 800, expires_at: Some(Timestamp(T0.0 + HOUR)), user: b"mine".to_vec() }
        );
        let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
        assert!(cs.set_hygiene(id, 1001, None).is_err());
    }
}
