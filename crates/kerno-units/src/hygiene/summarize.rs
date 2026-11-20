//! The summarize hook: [`UnitDb::consolidate_old`].

use std::collections::BTreeMap;
use std::time::Duration;

use kerno_core::contract::TxnEngine;

use super::{Hygiene, change_meta, retirable_relations, retire};
use crate::consolidation::Summarizer;
use crate::db::{CommitInfo, NewUnit, Unit, UnitDb};
use crate::derived_stale::{SourceInput, input_hash};
use crate::error::UnitError;
use crate::types::{RevNo, RevisionId, Timestamp, UnitId};

/// The type of the unit a consolidation creates.
pub const SUMMARY_TYPE: &str = "kerno.summary";

/// Most sources one summary names: its provenance (24 bytes per source) has to
/// fit the 4096 bytes of unit metadata with the method and parameters.
pub const MAX_CONSOLIDATED_SOURCES: usize = 64;

const PROVENANCE_FORMAT: u8 = 1;

/// Which old units [`UnitDb::consolidate_old`] folds into one summary.
#[derive(Clone, Copy, Debug)]
pub struct Consolidation<'a> {
    /// The type of the units to summarize (not [`SUMMARY_TYPE`]).
    pub unit_type: &'a str,
    /// Only units last updated at least this long ago (by the injected clock).
    pub older_than: Duration,
    /// Only units whose importance is at most this; `None` for any.
    pub max_importance: Option<u16>,
    /// Do nothing when fewer units than this qualify; `>= 1`.
    pub min_sources: usize,
    /// Fold at most this many, oldest first; `min_sources..=`
    /// [`MAX_CONSOLIDATED_SOURCES`].
    pub max_sources: usize,
}

/// Where a summary came from: the summarizer and the exact revisions it read.
/// It is stored in the summary unit's metadata ([`Hygiene::user`]); read it
/// back with [`SummaryProvenance::of`]. The input hash is
/// [`crate::input_hash`] of the sources, the identity of a derived summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryProvenance {
    pub method: String,
    pub version: u32,
    pub params: BTreeMap<String, String>,
    pub input_hash: [u8; 32],
    pub sources: Vec<RevisionId>,
}

fn put_str(out: &mut Vec<u8>, s: &str) -> Result<(), UnitError> {
    let len =
        u16::try_from(s.len()).map_err(|_| UnitError::InvalidArgument("summarizer text field too long".into()))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(s.as_bytes());
    Ok(())
}

/// A cursor over the provenance bytes; every read is bounds-checked.
struct Take<'a>(&'a [u8]);

impl<'a> Take<'a> {
    fn n(&mut self, n: usize) -> Result<&'a [u8], UnitError> {
        if self.0.len() < n {
            return Err(UnitError::InvalidArgument("summary provenance is cut short".into()));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn u16(&mut self) -> Result<usize, UnitError> {
        Ok(usize::from(u16::from_be_bytes(self.n(2)?.try_into().expect("2 bytes"))))
    }
    fn str(&mut self) -> Result<String, UnitError> {
        let len = self.u16()?;
        String::from_utf8(self.n(len)?.to_vec())
            .map_err(|_| UnitError::InvalidArgument("summary provenance text is not UTF-8".into()))
    }
}

impl SummaryProvenance {
    fn encode(&self) -> Result<Vec<u8>, UnitError> {
        let mut out = vec![PROVENANCE_FORMAT];
        put_str(&mut out, &self.method)?;
        out.extend_from_slice(&self.version.to_be_bytes());
        out.extend_from_slice(&self.input_hash);
        out.extend_from_slice(&(self.params.len() as u16).to_be_bytes());
        for (k, v) in &self.params {
            put_str(&mut out, k)?;
            put_str(&mut out, v)?;
        }
        out.extend_from_slice(&(self.sources.len() as u16).to_be_bytes());
        for s in &self.sources {
            out.extend_from_slice(&s.unit.0);
            out.extend_from_slice(&s.rev.0.to_be_bytes());
        }
        Ok(out)
    }

    /// The provenance of `unit` when it is a [`SUMMARY_TYPE`] unit made here;
    /// `None` for any other unit.
    pub fn of(unit: &Unit) -> Result<Option<SummaryProvenance>, UnitError> {
        if unit.unit_type != SUMMARY_TYPE {
            return Ok(None);
        }
        let Some(h) = Hygiene::decode(&unit.metadata)? else {
            return Ok(None);
        };
        let mut t = Take(&h.user);
        if t.n(1)?[0] != PROVENANCE_FORMAT {
            return Err(UnitError::InvalidArgument("unknown summary provenance format".into()));
        }
        let method = t.str()?;
        let version = u32::from_be_bytes(t.n(4)?.try_into().expect("4 bytes"));
        let input_hash = t.n(32)?.try_into().expect("32 bytes");
        let mut params = BTreeMap::new();
        for _ in 0..t.u16()? {
            let k = t.str()?;
            params.insert(k, t.str()?);
        }
        let mut sources = Vec::new();
        for _ in 0..t.u16()? {
            let unit = UnitId(t.n(16)?.try_into().expect("16 bytes"));
            let rev = RevNo(u64::from_be_bytes(t.n(8)?.try_into().expect("8 bytes")));
            sources.push(RevisionId { unit, rev });
        }
        Ok(Some(SummaryProvenance { method, version, params, input_hash, sources }))
    }
}

/// What [`UnitDb::consolidate_old`] made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consolidated {
    /// The new [`SUMMARY_TYPE`] unit.
    pub summary: UnitId,
    /// The revisions it summarizes; those units are now archived.
    pub sources: Vec<RevisionId>,
    /// `DUPLICATE_OF` relations deleted so that the sources could be archived.
    pub relations_removed: u64,
    pub commit: CommitInfo,
}

impl<E: TxnEngine> UnitDb<E> {
    /// Folds old active units into one summary unit. The text comes from
    /// `summarizer`, a callback the caller supplies: this crate runs no model
    /// and opens no network (I-31). It is called once, on a snapshot and
    /// before the write slot is taken, so a slow summarizer holds no lock.
    ///
    /// One change set creates the [`SUMMARY_TYPE`] unit and archives every
    /// source, so the summary exists exactly when its sources are archived.
    /// Its importance is the highest of its sources'. The sources keep their
    /// payload and history (`unarchive` brings one back); the summary's
    /// provenance, [`SummaryProvenance::of`], names the summarizer (method,
    /// version, parameters) and the exact revisions it read.
    ///
    /// Units with a relation other than `DUPLICATE_OF` are not eligible (the
    /// layer refuses to archive them). Returns `None`, writing nothing, when
    /// fewer than `min_sources` units qualify. A summarizer that returns
    /// `None` is [`UnitError::InvalidArgument`] and nothing is written. A
    /// unit changed after the snapshot is [`UnitError::Conflict`]: call again.
    pub fn consolidate_old(
        &self,
        sel: &Consolidation<'_>,
        summarizer: &dyn Summarizer,
    ) -> Result<Option<Consolidated>, UnitError> {
        if sel.unit_type == SUMMARY_TYPE {
            return Err(UnitError::InvalidArgument("a summary is not summarized again".into()));
        }
        if sel.min_sources == 0 || sel.max_sources < sel.min_sources || sel.max_sources > MAX_CONSOLIDATED_SOURCES {
            return Err(UnitError::InvalidArgument(format!(
                "sources must satisfy 1 <= min_sources <= max_sources <= {MAX_CONSOLIDATED_SOURCES}"
            )));
        }
        let micros = i64::try_from(sel.older_than.as_micros()).unwrap_or(i64::MAX);
        let cutoff = Timestamp(self.config().clock.now().0.saturating_sub(micros));
        let snap = self.snapshot()?;
        let mut old: Vec<(Timestamp, UnitId)> = Vec::new();
        let mut failure: Option<UnitError> = None;
        snap.reference_scan(|u| {
            if u.unit_type == sel.unit_type && u.updated_at <= cutoff {
                match Hygiene::of(u) {
                    Ok(h) if sel.max_importance.is_none_or(|m| h.importance <= m) => old.push((u.updated_at, u.id)),
                    Ok(_) => {}
                    Err(e) => {
                        failure.get_or_insert(e);
                    }
                }
            }
            false
        })?;
        if let Some(e) = failure {
            return Err(e);
        }
        old.sort();

        // The sources, oldest first: units the layer lets us archive.
        let mut picked: Vec<(Unit, Vec<crate::types::RelationId>)> = Vec::new();
        for (_, id) in old {
            if picked.len() == sel.max_sources {
                break;
            }
            let Some(relations) = retirable_relations(&snap, id)? else {
                continue;
            };
            let Some(unit) = snap.get_unit(id)? else {
                continue;
            };
            picked.push((unit, relations));
        }
        if picked.len() < sel.min_sources {
            return Ok(None);
        }

        // `SourceInput`s in (unit, revision) order, as `Summarizer` expects.
        picked.sort_by_key(|(u, _)| u.id);
        let mut inputs = Vec::with_capacity(picked.len());
        for (u, _) in &picked {
            let Some(payload) = u.payload.clone() else {
                return Err(UnitError::InvalidArgument(format!("unit {} has no payload to summarize", u.id)));
            };
            inputs.push(SourceInput {
                id: RevisionId { unit: u.id, rev: u.head },
                content_hash: u.content_hash,
                payload,
            });
        }
        let text = summarizer
            .summarize(&inputs)
            .ok_or_else(|| UnitError::InvalidArgument("the summarizer produced no text".into()))?;
        let importance =
            picked.iter().map(|(u, _)| Hygiene::of(u).map(|h| h.importance)).try_fold(0, |m, i| i.map(|i| m.max(i)))?;
        let provenance = SummaryProvenance {
            method: summarizer.method().to_owned(),
            version: summarizer.version(),
            params: summarizer.params(),
            input_hash: input_hash(&inputs),
            sources: inputs.iter().map(|i| i.id).collect(),
        };
        let metadata = Hygiene { importance, expires_at: None, user: provenance.encode()? }.encode()?;

        let mut cs =
            self.begin_changeset(change_meta(format!("kerno.hygiene: consolidated {} units", picked.len())))?;
        let summary =
            cs.create(NewUnit { unit_type: SUMMARY_TYPE.to_owned(), schema_version: 1, metadata, payload: text })?;
        let mut gone = std::collections::BTreeSet::new();
        let mut relations_removed = 0;
        for (u, relations) in &picked {
            relations_removed += retire(&mut cs, u.id, u.head, relations, &mut gone)?;
        }
        let commit = cs.commit()?;
        Ok(Some(Consolidated { summary, sources: provenance.sources, relations_removed, commit }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consolidation::TruncatingSummarizer;
    use crate::db::ChangeMeta;
    use crate::hygiene::DUPLICATE_OF;
    use crate::hygiene::testutil::{HOUR, fixture};
    use crate::types::{Lifecycle, RelationProvenance};

    fn all_old<'a>(unit_type: &'a str) -> Consolidation<'a> {
        Consolidation {
            unit_type,
            older_than: Duration::from_secs(3600),
            max_importance: None,
            min_sources: 2,
            max_sources: 10,
        }
    }

    #[test]
    fn folds_old_units_into_one_with_provenance() {
        let f = fixture();
        let a = f.make("note", b"alpha beta", Hygiene { importance: 300, ..Hygiene::default() }.encode().unwrap());
        let b = f.make("note", b"gamma delta", Hygiene { importance: 600, ..Hygiene::default() }.encode().unwrap());
        f.advance(2 * HOUR);
        let fresh = f.make("note", b"too new", Vec::new());
        let other = f.make("log", b"other type", Vec::new());

        let s = TruncatingSummarizer::new(5, 3);
        let done = f.db.consolidate_old(&all_old("note"), &s).unwrap().expect("two old units qualify");
        assert_eq!(done.sources, vec![RevisionId::first(a), RevisionId::first(b)]);

        // One summary unit: the summarizer's text, the highest importance.
        let unit = f.db.get_unit(done.summary).unwrap().unwrap();
        assert_eq!((unit.unit_type.as_str(), unit.payload.as_deref()), (SUMMARY_TYPE, Some(&b"alpha\ngamma"[..])));
        assert_eq!(Hygiene::of(&unit).unwrap().importance, 600);

        // Provenance: the method, version, parameters and exact revisions.
        let p = SummaryProvenance::of(&unit).unwrap().unwrap();
        assert_eq!((p.method.as_str(), p.version), (TruncatingSummarizer::METHOD, 3));
        assert_eq!(p.params, s.params());
        assert_eq!(p.sources, done.sources);
        assert_eq!(p.input_hash, crate::derived_stale::input_hash_of_ids(&done.sources));
        let cs = f.db.snapshot().unwrap().change_set(done.commit.change_set).unwrap().unwrap();
        assert_eq!(cs.reason.as_deref(), Some("kerno.hygiene: consolidated 2 units"));
        assert_eq!(cs.members.len(), 3, "the summary and the two archived sources commit together");

        // Sources are archived, not lost; the rest is untouched.
        for id in [a, b] {
            let u = f.db.get_unit(id).unwrap().unwrap();
            assert_eq!(u.lifecycle, Lifecycle::Archived);
            assert!(u.payload.is_some(), "the original is still readable");
        }
        for id in [fresh, other] {
            assert_eq!(f.db.get_unit(id).unwrap().unwrap().lifecycle, Lifecycle::Active);
        }
        assert_eq!(SummaryProvenance::of(&f.db.get_unit(fresh).unwrap().unwrap()).unwrap(), None);
    }

    #[test]
    fn selection_rules() {
        let f = fixture();
        let low = f.make("note", b"low", Hygiene { importance: 100, ..Hygiene::default() }.encode().unwrap());
        let high = f.make("note", b"high", Hygiene { importance: 900, ..Hygiene::default() }.encode().unwrap());
        let low2 = f.make("note", b"low2", Hygiene { importance: 200, ..Hygiene::default() }.encode().unwrap());
        f.advance(2 * HOUR);
        let s = TruncatingSummarizer::new(8, 1);

        // The important unit is spared; max_sources takes the oldest.
        let sel = Consolidation { max_importance: Some(500), max_sources: 2, ..all_old("note") };
        let done = f.db.consolidate_old(&sel, &s).unwrap().unwrap();
        assert_eq!(done.sources, vec![RevisionId::first(low), RevisionId::first(low2)]);
        assert_eq!(f.db.get_unit(high).unwrap().unwrap().lifecycle, Lifecycle::Active);

        // Below min_sources nothing is written.
        let before = f.db.raw_seq().unwrap();
        assert_eq!(f.db.consolidate_old(&all_old("note"), &s).unwrap(), None);
        assert_eq!(f.db.raw_seq().unwrap(), before);

        // Bad selections.
        assert!(f.db.consolidate_old(&all_old(SUMMARY_TYPE), &s).is_err());
        assert!(f.db.consolidate_old(&Consolidation { min_sources: 0, ..all_old("note") }, &s).is_err());
        assert!(f.db.consolidate_old(&Consolidation { max_sources: 65, ..all_old("note") }, &s).is_err());
    }

    struct Silent;
    impl Summarizer for Silent {
        fn method(&self) -> &str {
            "silent"
        }
        fn version(&self) -> u32 {
            1
        }
        fn summarize(&self, _: &[SourceInput]) -> Option<Vec<u8>> {
            None
        }
    }

    #[test]
    fn a_failing_summarizer_writes_nothing() {
        let f = fixture();
        let a = f.make("note", b"a", Vec::new());
        f.make("note", b"b", Vec::new());
        f.advance(2 * HOUR);
        let before = f.db.raw_seq().unwrap();
        assert!(f.db.consolidate_old(&all_old("note"), &Silent).is_err());
        assert_eq!(f.db.raw_seq().unwrap(), before);
        assert_eq!(f.db.get_unit(a).unwrap().unwrap().lifecycle, Lifecycle::Active);
    }

    #[test]
    fn duplicate_links_go_and_declared_relations_exclude() {
        let f = fixture();
        let a = f.make("note", b"a", Vec::new());
        let b = f.make("note", b"b", Vec::new());
        let c = f.make("note", b"c", Vec::new());
        let d = f.make("note", b"d", Vec::new());
        let mut cs = f.db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.create_relation(a, b, DUPLICATE_OF, RelationProvenance::Algorithmic { origin: b"t".to_vec() }).unwrap();
        cs.create_relation(c, d, "cites", RelationProvenance::Declared).unwrap();
        cs.commit().unwrap();
        f.advance(2 * HOUR);
        let done = f.db.consolidate_old(&all_old("note"), &TruncatingSummarizer::new(4, 1)).unwrap().unwrap();
        assert_eq!((done.sources, done.relations_removed), (vec![RevisionId::first(a), RevisionId::first(b)], 1));
        let snap = f.db.snapshot().unwrap();
        assert_eq!(snap.reference_relations(|_| true).unwrap().len(), 1, "only the declared relation is left");
        for id in [c, d] {
            assert_eq!(snap.get_unit(id).unwrap().unwrap().lifecycle, Lifecycle::Active);
        }
    }

    #[test]
    fn provenance_decoding_refuses_damage() {
        let f = fixture();
        let a = f.make("note", b"a", Vec::new());
        f.make("note", b"b", Vec::new());
        f.advance(2 * HOUR);
        let done = f.db.consolidate_old(&all_old("note"), &TruncatingSummarizer::new(4, 1)).unwrap().unwrap();
        let mut unit = f.db.get_unit(done.summary).unwrap().unwrap();
        assert!(SummaryProvenance::of(&unit).unwrap().is_some());
        unit.metadata.truncate(unit.metadata.len() - 3);
        assert!(SummaryProvenance::of(&unit).is_err());
        let _ = a;
    }
}
