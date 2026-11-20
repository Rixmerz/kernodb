//! Deduplication of near-identical text units: [`UnitDb::dedup_text`].

use std::cmp::Reverse;
use std::collections::BTreeMap;

use kerno_core::contract::TxnEngine;

use super::{Hygiene, change_meta};
use crate::db::{CommitInfo, UnitDb};
use crate::error::UnitError;
use crate::payload::ContentHash;
use crate::types::{CommitSeq, Direction, RelationProvenance, UnitId};

/// The relation from a duplicate to the unit it duplicates (the canonical
/// one). Its provenance is `Algorithmic` with the origin
/// `kerno.hygiene.dedup.exact` (same content hash) or
/// `kerno.hygiene.dedup.normalized` (same normalized text).
pub const DUPLICATE_OF: &str = "kerno.duplicate_of";

const ORIGIN_EXACT: &[u8] = b"kerno.hygiene.dedup.exact";
const ORIGIN_NORMALIZED: &[u8] = b"kerno.hygiene.dedup.normalized";

/// The normalized form two near-identical texts share: UTF-8 text, lower
/// case, whitespace runs collapsed to one space, trimmed. `None` for a payload
/// that is not UTF-8 or has no visible text: those only match on the exact
/// content hash.
pub fn normalize_text(payload: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(payload).ok()?;
    let norm = text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    (!norm.is_empty()).then_some(norm)
}

/// What [`UnitDb::dedup_text`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DedupReport {
    /// The change set with the new relations; `None` when none was needed.
    pub change_set: Option<CommitInfo>,
    /// Active units of the type that were read.
    pub units_examined: u64,
    /// Sets of two or more units with the same exact hash or normalized text.
    pub groups: u64,
    /// `DUPLICATE_OF` relations created: `(duplicate, canonical)`.
    pub links: Vec<(UnitId, UnitId)>,
    /// How many of `links` are exact content-hash matches (the rest differ
    /// only by case or whitespace).
    pub exact_links: u64,
    /// `false` when the change-set limit stopped the pass: call again.
    pub complete: bool,
}

struct Candidate {
    id: UnitId,
    importance: u16,
    created_seq: CommitSeq,
    hash: ContentHash,
}

impl<E: TxnEngine> UnitDb<E> {
    /// Links the duplicates among the active units of `unit_type` to one
    /// canonical unit each, with a [`DUPLICATE_OF`] relation, in one change
    /// set. Nothing is deleted or archived: both units stay readable, and
    /// the relation says which one to keep.
    ///
    /// Units are grouped by normalized text ([`normalize_text`]; an exact
    /// content-hash match always has the same normalized text), a non-text
    /// payload by its exact hash. The canonical unit of a group is the most
    /// important, then the oldest, then the lowest id. A unit that already
    /// has a `DUPLICATE_OF` relation is left as it is, so the pass is
    /// idempotent and does not move a link when the group changes.
    // ponytail: groups are rebuilt from a full scan of the type; fine to
    // ~100k units, then keep a hash index.
    pub fn dedup_text(&self, unit_type: &str) -> Result<DedupReport, UnitError> {
        let snap = self.snapshot()?;
        let mut groups: BTreeMap<Vec<u8>, Vec<Candidate>> = BTreeMap::new();
        let mut report = DedupReport { complete: true, ..DedupReport::default() };
        let mut failure: Option<UnitError> = None;
        snap.reference_scan(|u| {
            if u.unit_type != unit_type {
                return false;
            }
            let importance = match Hygiene::of(u) {
                Ok(h) => h.importance,
                Err(e) => {
                    failure.get_or_insert(e);
                    return false;
                }
            };
            report.units_examined += 1;
            let key = match u.payload.as_deref().and_then(normalize_text) {
                Some(t) => [&b"t"[..], blake3::hash(t.as_bytes()).as_bytes()].concat(),
                None => [&b"h"[..], &u.content_hash.digest].concat(),
            };
            groups.entry(key).or_default().push(Candidate {
                id: u.id,
                importance,
                created_seq: u.created_seq,
                hash: u.content_hash,
            });
            false
        })?;
        if let Some(e) = failure {
            return Err(e);
        }

        let max_ops = self.config().max_changeset_ops;
        let mut cs = self.begin_changeset(change_meta(format!("kerno.hygiene: dedup of {unit_type}")))?;
        'groups: for mut group in groups.into_values().filter(|g| g.len() > 1) {
            report.groups += 1;
            group.sort_by_key(|c| (Reverse(c.importance), c.created_seq, c.id));
            let (canonical, rest) = group.split_first().expect("a group has two units");
            for dup in rest {
                if !snap.relations_of(dup.id, Direction::Out, Some(DUPLICATE_OF))?.is_empty() {
                    continue;
                }
                if report.links.len() >= max_ops {
                    report.complete = false;
                    break 'groups;
                }
                let exact = dup.hash == canonical.hash;
                let origin = if exact { ORIGIN_EXACT } else { ORIGIN_NORMALIZED };
                cs.create_relation(
                    dup.id,
                    canonical.id,
                    DUPLICATE_OF,
                    RelationProvenance::Algorithmic { origin: origin.to_vec() },
                )?;
                report.exact_links += u64::from(exact);
                report.links.push((dup.id, canonical.id));
            }
        }
        if report.links.is_empty() {
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
    use crate::hygiene::testutil::fixture;
    use crate::types::Lifecycle;

    #[test]
    fn normalizes_case_and_whitespace_only() {
        assert_eq!(normalize_text(b"  Hello \n\tWORLD "), Some("hello world".into()));
        assert_eq!(normalize_text(b"hello world"), Some("hello world".into()));
        assert_ne!(normalize_text(b"hello worlds"), normalize_text(b"hello world"));
        assert_eq!(normalize_text(b" \n "), None);
        assert_eq!(normalize_text(&[0xff, 0xfe]), None);
    }

    fn relations(f: &crate::hygiene::testutil::Fixture) -> Vec<crate::db::Relation> {
        f.db.snapshot().unwrap().reference_relations(|_| true).unwrap()
    }

    #[test]
    fn links_exact_and_normalized_duplicates_and_deletes_nothing() {
        let f = fixture();
        let a = f.make("note", b"the user likes tea", Vec::new());
        let b = f.make("note", b"the user likes tea", Vec::new());
        let c = f.make("note", b"  The USER likes\n tea ", Vec::new());
        let other = f.make("note", b"the user likes coffee", Vec::new());
        let elsewhere = f.make("log", b"the user likes tea", Vec::new());

        let r = f.db.dedup_text("note").unwrap();
        assert_eq!((r.units_examined, r.groups, r.exact_links, r.complete), (4, 1, 1, true));
        assert_eq!(r.links, vec![(b, a), (c, a)], "oldest is canonical; links are duplicate -> canonical");
        let rels = relations(&f);
        assert_eq!(rels.len(), 2);
        for rel in &rels {
            assert_eq!((rel.relation_type.as_str(), rel.to), (DUPLICATE_OF, a));
        }
        let origin = |dup| match &rels.iter().find(|r| r.from == dup).unwrap().provenance {
            RelationProvenance::Algorithmic { origin } => origin.clone(),
            other => panic!("unexpected provenance {other:?}"),
        };
        assert_eq!(origin(b), ORIGIN_EXACT);
        assert_eq!(origin(c), ORIGIN_NORMALIZED);
        for id in [a, b, c, other, elsewhere] {
            assert_eq!(f.db.get_unit(id).unwrap().unwrap().lifecycle, Lifecycle::Active, "nothing is deleted");
        }

        // Idempotent.
        let again = f.db.dedup_text("note").unwrap();
        assert_eq!((again.links.len(), again.change_set.is_none()), (0, true));
        assert_eq!(relations(&f).len(), 2);
    }

    #[test]
    fn the_most_important_unit_is_canonical() {
        let f = fixture();
        let old = f.make("note", b"same", Hygiene { importance: 100, ..Hygiene::default() }.encode().unwrap());
        let key = f.make("note", b"same", Hygiene { importance: 900, ..Hygiene::default() }.encode().unwrap());
        let r = f.db.dedup_text("note").unwrap();
        assert_eq!(r.links, vec![(old, key)]);
    }

    #[test]
    fn non_text_payloads_match_only_exactly() {
        let f = fixture();
        let a = f.make("blob", &[0xff, 0x00, 0xfe], Vec::new());
        let b = f.make("blob", &[0xff, 0x00, 0xfe], Vec::new());
        f.make("blob", &[0xff, 0x00, 0xfd], Vec::new());
        let r = f.db.dedup_text("blob").unwrap();
        assert_eq!((r.links, r.exact_links), (vec![(b, a)], 1));
    }

    #[test]
    fn a_later_duplicate_links_to_the_existing_canonical() {
        let f = fixture();
        let a = f.make("note", b"same", Vec::new());
        let b = f.make("note", b"same", Vec::new());
        f.db.dedup_text("note").unwrap();
        let c = f.make("note", b"SAME", Vec::new());
        let r = f.db.dedup_text("note").unwrap();
        assert_eq!(r.links, vec![(c, a)], "only the new unit is linked, b keeps its link");
        let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
        assert!(cs.archive(b, None).is_err(), "the layer still refuses to archive a linked unit");
    }
}
