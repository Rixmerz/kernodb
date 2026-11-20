//! The frozen associative evaluation corpus (E7-00) loaded into a units
//! database, and its queries as `AssocQuery`s. Used by the tests of the
//! indexed path (E7-02).

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Options, TxnEngine};
use kerno_testkit::assoc_eval::{self, Direction as EvalDirection, Kind as EvalKind, Provenance as StoryProvenance};
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

pub const CORPUS: &str = include_str!("../../../kerno-testkit/data/assoc-eval/corpus.txt");
pub const QUERIES: &str = include_str!("../../../kerno-testkit/data/assoc-eval/queries.txt");

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

pub fn story_provenance(p: &StoryProvenance) -> RelationProvenance {
    match p {
        StoryProvenance::Declared => RelationProvenance::Declared,
        StoryProvenance::Imported(source) => RelationProvenance::Imported { source: source.clone().into_bytes() },
        StoryProvenance::Algorithmic(origin) => RelationProvenance::Algorithmic { origin: origin.clone().into_bytes() },
        StoryProvenance::Inferred { origin, confidence_permille } => RelationProvenance::Inferred {
            origin: origin.clone().into_bytes(),
            confidence_permille: *confidence_permille,
        },
    }
}

fn kind_of(k: EvalKind) -> ProvenanceKind {
    match k {
        EvalKind::Declared => ProvenanceKind::Declared,
        EvalKind::Imported => ProvenanceKind::Imported,
        EvalKind::Algorithmic => ProvenanceKind::Algorithmic,
        EvalKind::Inferred => ProvenanceKind::Inferred,
    }
}

/// One change set per step with the step's actor and source; returns the unit
/// id of each corpus index. A step is the commit sequence of its change set.
pub fn load_corpus<E: TxnEngine>(corpus: &assoc_eval::Corpus) -> (UnitDb<E>, Vec<UnitId>) {
    let (db, ids, _) = load_corpus_with_relations(corpus);
    (db, ids)
}

/// [`load_corpus`] that also returns the id of each corpus relation, by its
/// corpus index.
pub fn load_corpus_with_relations<E: TxnEngine>(
    corpus: &assoc_eval::Corpus,
) -> (UnitDb<E>, Vec<UnitId>, Vec<RelationId>) {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    let db = UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open");
    let mut ids = Vec::with_capacity(corpus.units.len());
    let mut rel_ids = Vec::with_capacity(corpus.relations.len());
    let (mut next_unit, mut next_rel) = (0, 0);
    for change in &corpus.changes {
        let meta = ChangeMeta {
            actor: Some(change.actor.clone().into_bytes()),
            source: Some(change.source.clone().into_bytes()),
            ..ChangeMeta::default()
        };
        let mut cs = db.begin_changeset(meta).expect("begin change set");
        while let Some(u) = corpus.units.get(next_unit).filter(|u| u.step == change.step) {
            let new = NewUnit {
                unit_type: u.unit_type.clone(),
                schema_version: 1,
                metadata: Vec::new(),
                payload: u.label.clone().into_bytes(),
            };
            ids.push(cs.create(new).expect("create unit"));
            next_unit += 1;
        }
        while let Some(r) = corpus.relations.get(next_rel).filter(|r| r.step == change.step) {
            let (from, to) = (ids[r.from as usize], ids[r.to as usize]);
            rel_ids.push(
                cs.create_relation(from, to, &r.relation_type, story_provenance(&r.provenance))
                    .expect("create relation"),
            );
            next_rel += 1;
        }
        let info = cs.commit().expect("commit change set");
        assert_eq!(info.commit_seq.0, change.step, "a step is a commit sequence");
    }
    (db, ids, rel_ids)
}

/// A query of the frozen set with the hard maxima as limit and budget.
pub fn corpus_query(ids: &[UnitId], q: &assoc_eval::Query) -> AssocQuery {
    let c = &q.constraints;
    let kinds: BTreeSet<ProvenanceKind> = c.kinds.iter().copied().map(kind_of).collect();
    let mut query =
        AssocQuery::new(vec![ids[q.seed as usize]], u32::from(c.max_depth), MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS);
    query.direction = match c.direction {
        EvalDirection::Out => AssocDirection::Out,
        EvalDirection::In => AssocDirection::In,
        EvalDirection::Both => AssocDirection::Both,
    };
    query.relation_types = Some(c.relation_types.iter().cloned().collect());
    // Steps are commit sequences (checked when the corpus is loaded).
    query.window = c.window.map(|(lo, hi)| SeqWindow { lo: CommitSeq(lo), hi: CommitSeq(hi) });
    query.provenance = ProvenanceFilter {
        kinds: (!kinds.is_empty()).then_some(kinds),
        actor: c.actor.as_ref().map(|a| a.clone().into_bytes()),
        source: c.source.as_ref().map(|s| s.clone().into_bytes()),
    };
    query
}

/// Puts (`Some`) and deletions (`None`) applied straight to the engine.
pub type Patch = Vec<(Vec<u8>, Option<Vec<u8>>)>;

pub fn put_all<E: kerno_core::contract::Engine>(db: &E, puts: Patch) {
    let mut b = kerno_core::WriteBatch::new();
    for (k, v) in puts {
        match v {
            Some(v) => b.put(k, v),
            None => b.delete(k),
        };
    }
    db.commit(&b).expect("raw commit");
}

/// What the engine holds now under each of `keys`, as a patch that puts it
/// back (a deletion for a key that is absent).
pub fn undo_of<E: kerno_core::contract::Engine>(db: &E, keys: &[Vec<u8>]) -> Patch {
    use kerno_core::contract::Snapshot;
    let snap = db.snapshot().expect("snapshot");
    keys.iter().map(|k| (k.clone(), snap.get(k).expect("get"))).collect()
}
