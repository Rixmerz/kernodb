//! E7-03: temporal context and provenance of an associative query, on the
//! reference path (`reference_assoc_query`), the indexed path
//! (`indexed_assoc_query`) and the planner's (`assoc_query`), over
//! `Emulated<LogEngine>` and `KernoDb`.
//!
//! * I-40 (`every_cited_*`, `frozen_corpus_*`): every relation and every
//!   revision a result cites exists in the snapshot queried, with the fields
//!   the trace states, and the path is a real walk from a seed.
//! * The `commit_seq` window and the kind, actor and source filters
//!   (`window_*`, `provenance_*`, `frozen_corpus_*`): each, and combinations
//!   of them, equal a brute-force oracle, also when the answer is empty.
//! * I-39 (`inferred_*`): a trace never shows an inferred, imported or
//!   algorithmic relation as declared.
//! * I-26 (`the_clock_orders_nothing`): two histories with the same commit
//!   sequences and different timestamps answer the same.
//!
//! The expectations come from a model the test keeps while it writes (every
//! relation with its commit sequence, actor, source and provenance; every
//! revision with its commit sequence) and, for the frozen corpus, from the
//! story the corpus is generated from. They are never read back from the code
//! under test. `assoc_indexed::differential_seeded` already holds the three
//! paths equal over every field of the query; what it cannot show is a result
//! whose unit has a head above revision 1, because its graphs never update a
//! unit, so here units are updated between snapshots.

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use harness::assoc::{CORPUS, QUERIES, corpus_query, load_corpus_with_relations, story_provenance};
use kerno_core::contract::{Engine, Options, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, Kind as EvalKind};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const TYPES: [&str; 3] = ["cites", "cite", "refines"];
const ACTORS: [&str; 3] = ["alice", "bot", "importer"];
const SOURCES: [&str; 3] = ["doc/1", "doc/2", "job/1"];
const KINDS: [ProvenanceKind; 4] =
    [ProvenanceKind::Declared, ProvenanceKind::Imported, ProvenanceKind::Algorithmic, ProvenanceKind::Inferred];

// ------------------------------------------------------------------- model

/// A relation as the test wrote it.
#[derive(Clone, Debug)]
struct MRel {
    id: RelationId,
    from: UnitId,
    to: UnitId,
    ty: String,
    seq: u64,
    prov: RelationProvenance,
    kind: ProvenanceKind,
    actor: Option<Vec<u8>>,
    source: Option<Vec<u8>>,
}

/// Everything written so far: relations, and for each unit the commit sequence
/// of each of its revisions (`revs[u][k]` is the sequence of revision `k + 1`).
#[derive(Default)]
struct Model {
    rels: Vec<MRel>,
    revs: BTreeMap<UnitId, Vec<u64>>,
}

impl Model {
    /// The head revision of `unit` as of commit `seq`, and its commit sequence.
    fn head_at(&self, unit: UnitId, seq: u64) -> (u64, u64) {
        let seen: Vec<u64> = self.revs[&unit].iter().copied().filter(|s| *s <= seq).collect();
        (seen.len() as u64, *seen.last().expect("the unit exists at this sequence"))
    }

    fn rel(&self, id: RelationId) -> Option<&MRel> {
        self.rels.iter().find(|r| r.id == id)
    }
}

/// The class of a provenance, written here and not taken from the library.
fn own_kind(p: &RelationProvenance) -> ProvenanceKind {
    match p {
        RelationProvenance::Declared => ProvenanceKind::Declared,
        RelationProvenance::Imported { .. } => ProvenanceKind::Imported,
        RelationProvenance::Algorithmic { .. } => ProvenanceKind::Algorithmic,
        RelationProvenance::Inferred { .. } => ProvenanceKind::Inferred,
    }
}

fn type_ok(q: &AssocQuery, r: &MRel) -> bool {
    q.relation_types.as_ref().is_none_or(|t| t.contains(&r.ty))
}

/// The window and the three provenance conditions.
fn context_ok(q: &AssocQuery, r: &MRel) -> bool {
    q.window.is_none_or(|w| w.lo.0 <= r.seq && r.seq <= w.hi.0)
        && q.provenance.kinds.as_ref().is_none_or(|k| k.contains(&r.kind))
        && q.provenance.actor.as_ref().is_none_or(|a| r.actor.as_ref() == Some(a))
        && q.provenance.source.as_ref().is_none_or(|s| r.source.as_ref() == Some(s))
}

// ------------------------------------------------------------------ oracle

struct WantHit {
    unit: UnitId,
    distance: u32,
    path: Vec<RelationId>,
}

struct Want {
    hits: Vec<WantHit>,
    visited: u64,
}

/// The answer written out from the documented order (`assoc.rs`, module
/// documentation) over every relation at once. Only for queries whose limit
/// and budget are not reached.
fn oracle(model: &Model, seq: u64, q: &AssocQuery) -> Want {
    let visible: Vec<&MRel> = model.rels.iter().filter(|r| r.seq <= seq).collect();
    let dirs: &[bool] = match q.direction {
        AssocDirection::Out => &[true],
        AssocDirection::In => &[false],
        AssocDirection::Both => &[true, false],
    };
    let seeds: BTreeSet<UnitId> = q.seeds.iter().copied().collect();
    let mut seen = seeds.clone();
    let mut frontier: Vec<(UnitId, Vec<RelationId>)> = seeds.iter().map(|s| (*s, Vec::new())).collect();
    let (mut hits, mut visited) = (Vec::new(), 0);
    for depth in 1..=q.max_depth {
        let mut level: BTreeMap<UnitId, Vec<RelationId>> = BTreeMap::new();
        for (here, path) in &frontier {
            for &out in dirs {
                let mut adjacent: Vec<&MRel> =
                    visible.iter().copied().filter(|r| if out { r.from == *here } else { r.to == *here }).collect();
                adjacent.sort_by(|a, b| (a.ty.as_bytes(), a.id.0).cmp(&(b.ty.as_bytes(), b.id.0)));
                for r in adjacent {
                    if !type_ok(q, r) {
                        continue;
                    }
                    visited += 1;
                    if !context_ok(q, r) {
                        continue;
                    }
                    let there = if out { r.to } else { r.from };
                    if seen.insert(there) {
                        let mut p = path.clone();
                        p.push(r.id);
                        level.insert(there, p);
                    }
                }
            }
        }
        frontier = level.iter().map(|(u, p)| (*u, p.clone())).collect();
        hits.extend(level.into_iter().map(|(unit, path)| WantHit { unit, distance: depth, path }));
        if frontier.is_empty() {
            break;
        }
    }
    Want { hits, visited }
}

/// `res` is the oracle's answer, field by field.
#[track_caller]
fn assert_answer(model: &Model, seq: u64, res: &AssocResult, want: &Want, ctx: &str) {
    assert_eq!(res.ending, AssocEnding::Exhausted, "{ctx}");
    assert_eq!(res.visited, want.visited, "{ctx}: entries examined");
    let got: Vec<_> =
        res.hits.iter().map(|h| (h.unit, h.distance, h.path.iter().map(|s| s.relation).collect::<Vec<_>>())).collect();
    let exp: Vec<_> = want.hits.iter().map(|h| (h.unit, h.distance, h.path.clone())).collect();
    assert_eq!(got, exp, "{ctx}: hits");
    for h in &res.hits {
        assert_eq!(h.revision, RevNo(model.head_at(h.unit, seq).0), "{ctx}: revision of {}", h.unit);
        for s in &h.path {
            let r = model.rel(s.relation).expect("the oracle's relations are the model's");
            assert_eq!(
                (s.relation_type.as_str(), s.from, s.to, s.created_seq.0, s.provenance),
                (r.ty.as_str(), r.from, r.to, r.seq, r.kind),
                "{ctx}: step {}",
                s.relation
            );
        }
    }
}

// ------------------------------------------------------- trace verification

/// Everything a result cites is in the snapshot (I-40). Works under any
/// bound: it needs no oracle of the whole answer.
#[track_caller]
fn verify_trace<E: Engine>(snap: &UnitSnapshot<E>, model: &Model, q: &AssocQuery, res: &AssocResult, ctx: &str) {
    let seq = snap.seq();
    let seeds: BTreeSet<UnitId> = q.seeds.iter().copied().collect();
    let by_unit: BTreeMap<UnitId, &AssocHit> = res.hits.iter().map(|h| (h.unit, h)).collect();
    assert_eq!(by_unit.len(), res.hits.len(), "{ctx}: a unit is returned twice");
    let order: Vec<_> = res.hits.iter().map(|h| (h.distance, h.unit)).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{ctx}: hits are not in (distance, unit) order");
    let out = matches!(q.direction, AssocDirection::Out | AssocDirection::Both);
    let inn = matches!(q.direction, AssocDirection::In | AssocDirection::Both);

    for hit in &res.hits {
        let name = format!("{ctx}: hit {}", hit.unit);
        assert!(!seeds.contains(&hit.unit), "{name} is a seed");
        assert_eq!(hit.path.len(), hit.distance as usize, "{name}: the distance is the path length");

        // The revision cited exists at the snapshot, is the unit's head there,
        // and no later one is visible.
        let (head, head_seq) = model.head_at(hit.unit, seq.0);
        assert_eq!(hit.revision, RevNo(head), "{name}: revision");
        let stored = snap.get_revision(hit.unit, hit.revision).unwrap_or_else(|e| panic!("{name}: revision: {e}"));
        assert_eq!(stored.commit_seq.0, head_seq, "{name}: commit sequence of the revision");
        assert!(stored.commit_seq <= seq, "{name}: a revision from after the snapshot");
        assert!(
            matches!(snap.get_revision(hit.unit, RevNo(head + 1)), Err(UnitError::RevisionNotFound { .. })),
            "{name}: a later revision is visible"
        );

        // Every relation cited exists, in the model and in the snapshot, and
        // the query lets it through.
        let mut cited = BTreeSet::new();
        for s in &hit.path {
            assert!(cited.insert(s.relation), "{name}: a relation is followed twice");
            let m = model.rel(s.relation).unwrap_or_else(|| panic!("{name}: {} was never written", s.relation));
            assert!(m.seq <= seq.0, "{name}: {} is from after the snapshot", s.relation);
            assert_eq!(
                (s.relation_type.as_str(), s.from, s.to, s.created_seq.0, s.provenance),
                (m.ty.as_str(), m.from, m.to, m.seq, m.kind),
                "{name}: step {}",
                s.relation
            );
            let r = snap
                .get_relation(s.relation)
                .unwrap()
                .unwrap_or_else(|| panic!("{name}: {} is not in the snapshot", s.relation));
            assert_eq!((r.from, r.to, &r.relation_type, r.created_seq.0), (m.from, m.to, &m.ty, m.seq), "{name}");
            // Origin and confidence are kept, not only the class (I-39).
            assert_eq!(r.provenance, m.prov, "{name}: stored provenance of {}", s.relation);
            assert!(type_ok(q, m) && context_ok(q, m), "{name}: {} is excluded by the query", s.relation);
        }

        // The relations are a walk from a seed to the unit, each hop in a
        // followed direction, and each unit on the way is a seed or a hit of
        // this result at the distance of its position.
        let first = &hit.path[0];
        let walks = [first.from, first.to].into_iter().filter(|s| seeds.contains(s)).any(|start| {
            let mut here = start;
            for (k, s) in hit.path.iter().enumerate() {
                here = if out && s.from == here {
                    s.to
                } else if inn && s.to == here {
                    s.from
                } else {
                    return false;
                };
                if k + 1 < hit.path.len() && by_unit.get(&here).is_none_or(|h| h.distance as usize != k + 1) {
                    return false;
                }
            }
            here == hit.unit
        });
        assert!(walks, "{name}: the path is not a walk from a seed through hits of the result");
    }
}

// ---------------------------------------------------------------- the paths

/// The three answers of `q`, each unwrapped: the planner's must not have
/// fallen back on a sound store.
fn answers<E: Engine>(snap: &UnitSnapshot<E>, q: &AssocQuery, ctx: &str) -> [(&'static str, AssocResult); 3] {
    let reference = snap.reference_assoc_query(q).unwrap_or_else(|e| panic!("{ctx}: reference: {e}"));
    let indexed = snap.indexed_assoc_query(q).unwrap_or_else(|e| panic!("{ctx}: indexed: {e}"));
    let planned = snap.assoc_query(q).unwrap_or_else(|e| panic!("{ctx}: planner: {e}"));
    assert_eq!(planned.fallback, None, "{ctx}: a sound store never falls back");
    [("reference", reference), ("indexed", indexed), ("planner", planned.result)]
}

/// Every path equal to the oracle and every trace verified.
#[track_caller]
fn check_against_oracle<E: Engine>(snap: &UnitSnapshot<E>, model: &Model, q: &AssocQuery, ctx: &str) -> Want {
    let want = oracle(model, snap.seq().0, q);
    for (path, res) in answers(snap, q, ctx) {
        let ctx = format!("{ctx} [{path}]");
        assert_answer(model, snap.seq().0, &res, &want, &ctx);
        verify_trace(snap, model, q, &res, &ctx);
    }
    want
}

// ------------------------------------------------------------------- world

fn new_unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: Vec::new(), payload: payload.to_vec() }
}

fn meta(actor: Option<&[u8]>, source: Option<&[u8]>) -> ChangeMeta {
    ChangeMeta { actor: actor.map(<[u8]>::to_vec), source: source.map(<[u8]>::to_vec), ..ChangeMeta::default() }
}

fn pick<'a>(rng: &mut Rng, of: &'a [&'a str]) -> Option<&'a [u8]> {
    rng.chance(3, 4).then(|| of[rng.below(of.len() as u64) as usize].as_bytes())
}

fn random_provenance(rng: &mut Rng) -> RelationProvenance {
    match rng.below(4) {
        0 => RelationProvenance::Declared,
        1 => RelationProvenance::Imported { source: b"wiki".to_vec() },
        2 => RelationProvenance::Algorithmic { origin: b"dedup".to_vec() },
        _ => RelationProvenance::Inferred { origin: b"model".to_vec(), confidence_permille: rng.below(1001) as u16 },
    }
}

/// A database that grows in change sets of a few relations, with random actors
/// and sources, and now and then a unit updated, next to a model of what was
/// written. The ids, sequences and contents depend on the seed alone, never on
/// the clock.
struct World<E: TxnEngine> {
    db: UnitDb<E>,
    units: Vec<UnitId>,
    model: Model,
    rng: Rng,
    seq: u64,
}

impl<E: TxnEngine> World<E> {
    fn new(seed: u64, clock: Arc<FixedClock>) -> Self {
        let mut c = UnitConfig::with_sources(clock, Arc::new(CounterIds::new(1)));
        c.engine = Options::small_for_tests();
        let db = UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open");
        let mut rng = Rng::new(seed);
        let n = 9 + rng.below(5) as usize;
        let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
        let units: Vec<UnitId> = (0..n).map(|i| cs.create(new_unit(&[i as u8])).expect("create")).collect();
        assert_eq!(cs.commit().expect("commit").commit_seq.0, 1);
        let mut model = Model::default();
        for u in &units {
            model.revs.insert(*u, vec![1]);
        }
        World { db, units, model, rng, seq: 1 }
    }

    fn with_seed(seed: u64) -> Self {
        Self::new(seed, Arc::new(FixedClock::new(T0)))
    }

    fn grow(&mut self, rounds: u32) {
        for _ in 0..rounds {
            let (actor, source) = (pick(&mut self.rng, &ACTORS), pick(&mut self.rng, &SOURCES));
            let mut cs = self.db.begin_changeset(meta(actor, source)).expect("begin");
            let mut written = Vec::new();
            for _ in 0..self.rng.range(1, 4) {
                let from = self.units[self.rng.below(self.units.len() as u64) as usize];
                let to = self.units[self.rng.below(self.units.len() as u64) as usize];
                let ty = TYPES[self.rng.below(TYPES.len() as u64) as usize];
                let prov = random_provenance(&mut self.rng);
                let id = cs.create_relation(from, to, ty, prov.clone()).expect("create_relation");
                written.push((id, from, to, ty, prov));
            }
            let mut updated = BTreeSet::new();
            for _ in 0..self.rng.below(3) {
                updated.insert(self.units[self.rng.below(self.units.len() as u64) as usize]);
            }
            for u in &updated {
                let upd = UnitUpdate { payload: vec![self.seq as u8, 7], schema_version: None, metadata: None };
                cs.update(*u, upd, None).expect("update");
            }
            let info = cs.commit().expect("commit");
            self.seq += 1;
            assert_eq!(info.commit_seq.0, self.seq);
            for (id, from, to, ty, prov) in written {
                self.model.rels.push(MRel {
                    id,
                    from,
                    to,
                    ty: ty.to_owned(),
                    seq: self.seq,
                    kind: own_kind(&prov),
                    prov,
                    actor: actor.map(<[u8]>::to_vec),
                    source: source.map(<[u8]>::to_vec),
                });
            }
            for u in updated {
                self.model.revs.get_mut(&u).expect("known unit").push(self.seq);
            }
        }
    }

    fn snapshot(&self) -> UnitSnapshot<E> {
        self.db.snapshot().expect("snapshot")
    }
}

/// A query setting, over many draws, every field. With `bounded` the limit and
/// the budget are often small; otherwise they are the hard maxima.
fn random_query(rng: &mut Rng, units: &[UnitId], last: u64, bounded: bool) -> AssocQuery {
    let seeds = (0..rng.range(1, 3)).map(|_| units[rng.below(units.len() as u64) as usize]).collect();
    let (limit, budget) = if bounded {
        ([1usize, 2, 3, 5, 50, 1000][rng.below(6) as usize], [1u64, 2, 3, 5, 8, 13, 40, 100_000][rng.below(8) as usize])
    } else {
        (MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS)
    };
    let mut q = AssocQuery::new(seeds, rng.range(1, 4) as u32, limit, budget);
    q.direction = [AssocDirection::Out, AssocDirection::In, AssocDirection::Both][rng.below(3) as usize];
    if rng.chance(1, 3) {
        let mut types: BTreeSet<String> = TYPES.iter().filter(|_| rng.chance(1, 2)).map(|t| (*t).to_owned()).collect();
        if types.is_empty() {
            types.insert(TYPES[0].to_owned());
        }
        q.relation_types = Some(types);
    }
    if rng.chance(1, 2) {
        let (a, b) = (rng.range(0, last + 1), rng.range(0, last + 1));
        q.window = Some(SeqWindow { lo: CommitSeq(a.min(b)), hi: CommitSeq(a.max(b)) });
    }
    if rng.chance(1, 3) {
        let mut kinds: BTreeSet<ProvenanceKind> = KINDS.into_iter().filter(|_| rng.chance(1, 2)).collect();
        if kinds.is_empty() {
            kinds.insert(KINDS[rng.below(4) as usize]);
        }
        q.provenance.kinds = Some(kinds);
    }
    if rng.chance(1, 3) {
        q.provenance.actor = Some(ACTORS.get(rng.below(4) as usize).unwrap_or(&"nobody").as_bytes().to_vec());
    }
    if rng.chance(1, 3) {
        q.provenance.source = Some(SOURCES.get(rng.below(4) as usize).unwrap_or(&"nobody").as_bytes().to_vec());
    }
    q
}

// -------------------------------------------------------------------- I-40

fn every_cited_relation_and_revision_exists_in_the_snapshot<E: TxnEngine>() {
    let (mut hits, mut empty, mut updated_heads, mut bounded_ends) = (0, 0, 0, BTreeSet::new());
    for seed in 1..=6 {
        let mut w = World::<E>::with_seed(seed);
        w.grow(8);
        let old = w.snapshot();
        w.grow(8);
        let new = w.snapshot();
        let mut rng = Rng::new(1000 + seed);
        for n in 0..50 {
            for (name, snap) in [("old", &old), ("new", &new)] {
                let q = random_query(&mut rng, &w.units, snap.seq().0, true);
                let ctx = format!("seed {seed} query {n} on the {name} snapshot");
                let [reference, indexed, planned] = answers(snap, &q, &ctx);
                assert_eq!(indexed.1, reference.1, "{ctx}: indexed vs reference");
                assert_eq!(planned.1, reference.1, "{ctx}: planner vs reference");
                for (path, res) in [&reference, &indexed, &planned] {
                    verify_trace(snap, &w.model, &q, res, &format!("{ctx} [{path}]"));
                }
                hits += reference.1.hits.len();
                empty += usize::from(reference.1.hits.is_empty());
                updated_heads += reference.1.hits.iter().filter(|h| h.revision > RevNo(1)).count();
                bounded_ends.insert(format!("{:?}", reference.1.ending));
            }
        }
    }
    // The generator reached what the checks are about, or they say nothing.
    assert!(hits > 500 && empty > 10, "hits {hits}, empty {empty}");
    assert!(updated_heads > 50, "results whose unit was updated: {updated_heads}");
    assert_eq!(bounded_ends.len(), 3, "every ending: {bounded_ends:?}");
}

/// The cited relation and revision must also be absent from what a snapshot
/// from before them cannot see: an old snapshot never cites a later one.
fn an_old_snapshot_never_cites_what_came_after_it<E: TxnEngine>() {
    let mut w = World::<E>::with_seed(11);
    w.grow(6);
    let old = w.snapshot();
    let old_seq = old.seq().0;
    w.grow(10);
    let new = w.snapshot();
    let ids: BTreeSet<RelationId> = w.model.rels.iter().filter(|r| r.seq > old_seq).map(|r| r.id).collect();
    assert!(!ids.is_empty());
    let mut rng = Rng::new(5);
    let mut later_seen_on_new = 0;
    for n in 0..60 {
        let q = random_query(&mut rng, &w.units, new.seq().0, false);
        for (_, res) in answers(&old, &q, "old") {
            let cited = res.hits.iter().flat_map(|h| &h.path).filter(|s| ids.contains(&s.relation)).count();
            assert_eq!(cited, 0, "query {n}: the old snapshot cites a relation written after it");
            assert!(res.hits.iter().all(|h| h.revision.0 <= w.model.head_at(h.unit, old_seq).0));
        }
        for (_, res) in answers(&new, &q, "new") {
            later_seen_on_new += res.hits.iter().flat_map(|h| &h.path).filter(|s| ids.contains(&s.relation)).count();
        }
    }
    assert!(later_seen_on_new > 50, "the later relations do matter to the queries: {later_seen_on_new}");
}

/// The frozen corpus, with the model built from the story it is generated
/// from: every result of the 64 queries, and of tighter versions of them, on
/// the three paths.
fn frozen_corpus_model(corpus: &assoc_eval::Corpus, ids: &[UnitId], rel_ids: &[RelationId]) -> Model {
    let by_step: BTreeMap<u64, &assoc_eval::Change> = corpus.changes.iter().map(|c| (c.step, c)).collect();
    let mut model = Model::default();
    for u in &corpus.units {
        model.revs.insert(ids[u.index as usize], vec![u.step]);
    }
    for (i, r) in corpus.relations.iter().enumerate() {
        let change = by_step[&r.step];
        model.rels.push(MRel {
            id: rel_ids[i],
            from: ids[r.from as usize],
            to: ids[r.to as usize],
            ty: r.relation_type.clone(),
            seq: r.step,
            prov: story_provenance(&r.provenance),
            kind: match r.provenance.kind() {
                EvalKind::Declared => ProvenanceKind::Declared,
                EvalKind::Imported => ProvenanceKind::Imported,
                EvalKind::Algorithmic => ProvenanceKind::Algorithmic,
                EvalKind::Inferred => ProvenanceKind::Inferred,
            },
            actor: Some(change.actor.clone().into_bytes()),
            source: Some(change.source.clone().into_bytes()),
        });
    }
    model
}

fn frozen_corpus_cites_only_what_the_snapshot_holds<E: TxnEngine>() {
    let corpus = assoc_eval::parse_corpus(CORPUS).expect("corpus parses");
    let set = assoc_eval::parse_queries(QUERIES).expect("queries parse");
    let (db, ids, rel_ids) = load_corpus_with_relations::<E>(&corpus);
    let model = frozen_corpus_model(&corpus, &ids, &rel_ids);
    let snap = db.snapshot().unwrap();
    assert_eq!(set.queries.len(), 64);

    let mut steps = 0;
    for q in &set.queries {
        let query = corpus_query(&ids, q);
        let want = check_against_oracle(&snap, &model, &query, &q.id);
        steps += want.hits.iter().map(|h| h.path.len()).sum::<usize>();
        // The expected answers of the set are cited too (the set derives them
        // from the story, not from this code).
        for j in &q.expected {
            assert!(
                want.hits.iter().any(|h| h.unit == ids[j.index as usize]),
                "{}: expected unit {} is not a hit",
                q.id,
                j.index
            );
        }
        // Tighter bounds: every ending, every result still cites only what exists.
        let full = snap.reference_assoc_query(&query).unwrap();
        for (limit, budget) in [(3, 100_000), (1000, full.visited.max(2) - 1), (2, 7)] {
            let tight = AssocQuery { limit, visit_budget: budget, ..query.clone() };
            let ctx = format!("{} limit {limit} budget {budget}", q.id);
            let [reference, indexed, planned] = answers(&snap, &tight, &ctx);
            assert_eq!(indexed.1, reference.1, "{ctx}");
            assert_eq!(planned.1, reference.1, "{ctx}");
            verify_trace(&snap, &model, &tight, &reference.1, &ctx);
        }
    }
    assert!(steps > 500, "the set walks real paths: {steps} steps");
}

// ------------------------------------------------------ window and filters

/// What each condition of a query is for, to show that a sweep made it matter.
#[derive(Default, Debug)]
struct Tally {
    empty: u32,
    nonempty: u32,
    narrowed: [u32; 4], // window, kinds, actor, source
}

impl Tally {
    /// Runs `q` on every path against the oracle, and counts which of its
    /// conditions changed the answer (removing it gives another one).
    fn check<E: Engine>(&mut self, snap: &UnitSnapshot<E>, model: &Model, q: &AssocQuery, ctx: &str) {
        let want = check_against_oracle(snap, model, q, ctx);
        if want.hits.is_empty() {
            self.empty += 1;
        } else {
            self.nonempty += 1;
        }
        let same = |without: &AssocQuery| {
            let other = oracle(model, snap.seq().0, without);
            let key = |w: &Want| w.hits.iter().map(|h| (h.unit, h.path.clone())).collect::<Vec<_>>();
            key(&other) == key(&want)
        };
        for i in 0..4 {
            if let Some(without) = without_condition(q, i) {
                self.narrowed[i] += u32::from(!same(&without));
            }
        }
    }
}

/// `q` without its condition `i` (0 window, 1 kinds, 2 actor, 3 source), or
/// `None` when it has none.
fn without_condition(q: &AssocQuery, i: usize) -> Option<AssocQuery> {
    let mut v = q.clone();
    let present = match i {
        0 => v.window.take().is_some(),
        1 => v.provenance.kinds.take().is_some(),
        2 => v.provenance.actor.take().is_some(),
        _ => v.provenance.source.take().is_some(),
    };
    present.then_some(v)
}

fn big_query(seeds: Vec<UnitId>, direction: AssocDirection) -> AssocQuery {
    let mut q = AssocQuery::new(seeds, 3, MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS);
    q.direction = direction;
    q
}

fn window_and_provenance_filters_equal_the_brute_force_oracle<E: TxnEngine>() {
    let mut tally = Tally::default();
    for seed in 1..=4 {
        let mut w = World::<E>::with_seed(20 + seed);
        w.grow(8);
        let old = w.snapshot();
        w.grow(6);
        let new = w.snapshot();
        let mut rng = Rng::new(seed);
        for (name, snap) in [("old", &old), ("new", &new)] {
            let last = snap.seq().0;
            let ctx = |what: &str| format!("seed {seed} {name} {what}");
            let seeds = |rng: &mut Rng| vec![w.units[rng.below(w.units.len() as u64) as usize]];

            // The window: every pair of a grid that has both ends of the
            // history, the commit just before and after it, and past it.
            let grid = [0, 1, 2, 3, last / 2, last - 1, last, last + 1, last + 9];
            for &lo in &grid {
                for &hi in grid.iter().filter(|h| **h >= lo) {
                    let mut q = big_query(seeds(&mut rng), AssocDirection::Both);
                    q.window = Some(SeqWindow { lo: CommitSeq(lo), hi: CommitSeq(hi) });
                    tally.check(snap, &w.model, &q, &ctx(&format!("window {lo}..={hi}")));
                }
            }
            // Each kind set, with no other condition.
            for mask in 1..16u32 {
                let kinds: BTreeSet<_> = (0..4).filter(|i| mask & (1 << i) != 0).map(|i| KINDS[i]).collect();
                let mut q = big_query(seeds(&mut rng), AssocDirection::Out);
                q.provenance.kinds = Some(kinds);
                tally.check(snap, &w.model, &q, &ctx(&format!("kinds {mask:04b}")));
            }
            // Each actor and each source, a value nothing has included.
            for actor in ACTORS.iter().chain(&["nobody"]) {
                let mut q = big_query(seeds(&mut rng), AssocDirection::Both);
                q.provenance.actor = Some(actor.as_bytes().to_vec());
                tally.check(snap, &w.model, &q, &ctx(&format!("actor {actor}")));
            }
            for source in SOURCES.iter().chain(&["nobody"]) {
                let mut q = big_query(seeds(&mut rng), AssocDirection::In);
                q.provenance.source = Some(source.as_bytes().to_vec());
                tally.check(snap, &w.model, &q, &ctx(&format!("source {source}")));
            }
            // Combinations, with types and directions and several seeds.
            for n in 0..40 {
                let mut q = random_query(&mut rng, &w.units, last, false);
                // A combination has at least two of the four conditions.
                q.window.get_or_insert(SeqWindow { lo: CommitSeq(1), hi: CommitSeq(last) });
                q.provenance.actor.get_or_insert_with(|| ACTORS[0].as_bytes().to_vec());
                tally.check(snap, &w.model, &q, &ctx(&format!("combination {n}")));
            }
        }
    }
    assert!(tally.empty > 40 && tally.nonempty > 200, "empty and non-empty answers: {tally:?}");
    assert!(tally.narrowed.iter().all(|n| *n > 20), "every condition changed some answer: {tally:?}");
}

/// The 64 queries carry window, kinds, actor and source of their own; the
/// oracle is built from the story. Here each condition is also taken away, one
/// at a time, so that each of them is shown to decide something on the corpus.
fn frozen_corpus_filters_equal_the_oracle_one_condition_at_a_time<E: TxnEngine>() {
    let corpus = assoc_eval::parse_corpus(CORPUS).expect("corpus parses");
    let set = assoc_eval::parse_queries(QUERIES).expect("queries parse");
    let (db, ids, rel_ids) = load_corpus_with_relations::<E>(&corpus);
    let model = frozen_corpus_model(&corpus, &ids, &rel_ids);
    let snap = db.snapshot().unwrap();
    let mut tally = Tally::default();
    for q in &set.queries {
        let query = corpus_query(&ids, q);
        let mut variants = vec![("as written".to_owned(), query.clone())];
        for i in 0..4 {
            if let Some(v) = without_condition(&query, i) {
                variants.push((format!("without condition {i}"), v));
            }
        }
        for (what, v) in variants {
            tally.check(&snap, &model, &v, &format!("{} {what}", q.id));
        }
    }
    assert!(tally.nonempty > 64, "{tally:?}");
    assert!(tally.narrowed.iter().all(|n| *n > 3), "every condition decided some answer of the set: {tally:?}");
}

// -------------------------------------------------------------------- I-39

/// Hits as `(unit, [(relation, class shown)])`.
type Steps = Vec<(UnitId, Vec<(RelationId, ProvenanceKind)>)>;

fn inferred_is_never_declared_in_a_trace<E: TxnEngine>() {
    let w = World::<E>::with_seed(1);
    let (db, u) = (&w.db, &w.units);
    let ev = |m: ChangeMeta, from, to, p: RelationProvenance| {
        let mut cs = db.begin_changeset(m).unwrap();
        let id = cs.create_relation(from, to, "t", p).unwrap();
        cs.commit().unwrap();
        id
    };
    // Four parallel relations a->b of one type, one per class, then a second
    // hop b->c that is inferred and c->d that is declared.
    let declared = ev(meta(Some(b"alice"), None), u[0], u[1], RelationProvenance::Declared);
    let imported =
        ev(meta(Some(b"alice"), None), u[0], u[1], RelationProvenance::Imported { source: b"wiki".to_vec() });
    let algorithmic =
        ev(meta(Some(b"alice"), None), u[0], u[1], RelationProvenance::Algorithmic { origin: b"dedup".to_vec() });
    let inferred = ev(
        meta(Some(b"alice"), None),
        u[0],
        u[1],
        RelationProvenance::Inferred { origin: b"model-7".to_vec(), confidence_permille: 650 },
    );
    let inferred_hop = ev(
        meta(Some(b"alice"), None),
        u[1],
        u[2],
        RelationProvenance::Inferred { origin: b"model-7".to_vec(), confidence_permille: 400 },
    );
    let declared_hop = ev(meta(Some(b"alice"), None), u[2], u[3], RelationProvenance::Declared);
    let snap = w.snapshot();

    let run = |kinds: Option<&[ProvenanceKind]>| {
        let mut q = big_query(vec![u[0]], AssocDirection::Out);
        q.provenance.kinds = kinds.map(|k| k.iter().copied().collect());
        answers(&snap, &q, "inferred")
    };
    let steps = |res: &AssocResult| -> Steps {
        res.hits.iter().map(|h| (h.unit, h.path.iter().map(|s| (s.relation, s.provenance)).collect())).collect()
    };
    use ProvenanceKind::{Algorithmic, Declared, Imported, Inferred};
    let cases: Vec<(Option<&[ProvenanceKind]>, Steps)> = vec![
        // No filter: the first relation in id order reaches b, and it is the declared one.
        (
            None,
            vec![
                (u[1], vec![(declared, Declared)]),
                (u[2], vec![(declared, Declared), (inferred_hop, Inferred)]),
                (u[3], vec![(declared, Declared), (inferred_hop, Inferred), (declared_hop, Declared)]),
            ],
        ),
        // Only inferred: the path says inferred at every step, and stops at the declared hop.
        (
            Some(&[Inferred]),
            vec![(u[1], vec![(inferred, Inferred)]), (u[2], vec![(inferred, Inferred), (inferred_hop, Inferred)])],
        ),
        // Only declared: nothing inferred is followed, so c is not reached.
        (Some(&[Declared]), vec![(u[1], vec![(declared, Declared)])]),
        (Some(&[Imported]), vec![(u[1], vec![(imported, Imported)])]),
        (Some(&[Algorithmic]), vec![(u[1], vec![(algorithmic, Algorithmic)])]),
        (Some(&[Imported, Algorithmic]), vec![(u[1], vec![(imported, Imported)])]),
        (
            Some(&[Imported, Inferred]),
            vec![(u[1], vec![(imported, Imported)]), (u[2], vec![(imported, Imported), (inferred_hop, Inferred)])],
        ),
    ];
    for (kinds, want) in cases {
        for (path, res) in run(kinds) {
            assert_eq!(steps(&res), want, "kinds {kinds:?} on the {path} path");
        }
    }
    // The stored record keeps origin and confidence, and the class in the
    // trace is the class of that record (a trace says `Declared` only for a
    // record that is declared).
    let stored = |id| snap.get_relation(id).unwrap().unwrap().provenance;
    assert_eq!(stored(declared), RelationProvenance::Declared);
    assert_eq!(
        stored(inferred),
        RelationProvenance::Inferred { origin: b"model-7".to_vec(), confidence_permille: 650 }
    );
    for (path, res) in run(None) {
        for h in &res.hits {
            for s in &h.path {
                assert_eq!(s.provenance == Declared, stored(s.relation) == RelationProvenance::Declared, "{path}");
            }
        }
    }

    // Over the generated worlds: no step of any result shows a class other
    // than the one the model wrote, and every class does appear.
    let mut seen_kinds = BTreeSet::new();
    for seed in 1..=3 {
        let mut w = World::<E>::with_seed(seed);
        w.grow(12);
        let snap = w.snapshot();
        let mut rng = Rng::new(seed);
        for n in 0..30 {
            let mut q = random_query(&mut rng, &w.units, snap.seq().0, false);
            q.provenance = ProvenanceFilter::default();
            for (path, res) in answers(&snap, &q, &format!("seed {seed} {n}")) {
                for s in res.hits.iter().flat_map(|h| &h.path) {
                    let m = w.model.rel(s.relation).expect("written");
                    assert_eq!(s.provenance, m.kind, "{path}: {} is {:?}", s.relation, m.kind);
                    seen_kinds.insert(s.provenance);
                }
            }
        }
    }
    assert_eq!(seen_kinds.len(), 4, "every class appeared in a trace: {seen_kinds:?}");
}

// -------------------------------------------------------------------- I-26

fn the_clock_orders_nothing<E: TxnEngine>() {
    // The same history under three clocks: constant, going backwards in big
    // jumps, and a steady one. The commit sequences are equal; the timestamps
    // of the revisions and of the change sets are not.
    let clocks = || {
        [
            Arc::new(FixedClock::new(T0)),
            Arc::new(FixedClock::stepping(Timestamp(9_000_000_000_000_000), -1_234_567_891)),
            Arc::new(FixedClock::stepping(Timestamp(5), 3)),
        ]
    };
    let mut windows = 0;
    for seed in 1..=4 {
        let mut worlds: Vec<World<E>> = clocks().into_iter().map(|c| World::new(seed, c)).collect();
        for w in &mut worlds {
            w.grow(8);
        }
        let snaps: Vec<UnitSnapshot<E>> = worlds.iter().map(World::snapshot).collect();
        let (w0, last) = (&worlds[0], snaps[0].seq().0);

        // The timestamps differ, and the backwards clock is not monotonic in the sequence.
        let stamp = |i: usize, s: u64| snaps[i].change_set(CommitSeq(s)).unwrap().expect("change set").created_at;
        assert!(stamp(0, 2) != stamp(1, 2) && stamp(1, 2) != stamp(2, 2), "the clocks give other timestamps");
        assert!(stamp(1, last) < stamp(1, 2), "the backwards clock goes back as the sequence goes on");
        assert!(stamp(2, last) > stamp(2, 2));
        let rev_at = |i: usize| snaps[i].get_revision(w0.units[0], RevNo(1)).unwrap().created_at;
        assert!(rev_at(0) != rev_at(1) && rev_at(1) != rev_at(2));

        let mut rng = Rng::new(7 + seed);
        for n in 0..40 {
            let bounded = n % 2 == 0;
            let q = random_query(&mut rng, &w0.units, last, bounded);
            windows += usize::from(q.window.is_some());
            let ctx = format!("seed {seed} query {n}");
            let first = answers(&snaps[0], &q, &ctx);
            for (i, snap) in snaps.iter().enumerate().skip(1) {
                for (a, b) in first.iter().zip(answers(snap, &q, &ctx)) {
                    assert_eq!(format!("{:?}", a.1), format!("{:?}", b.1), "{ctx}: clock {i}, {} path", a.0);
                }
            }
            // And each is the oracle's, so the three clocks are not merely
            // wrong in the same way.
            if !bounded {
                for (i, snap) in snaps.iter().enumerate() {
                    let want = oracle(&worlds[i].model, last, &q);
                    for (path, res) in answers(snap, &q, &ctx) {
                        assert_answer(&worlds[i].model, last, &res, &want, &format!("{ctx} clock {i} [{path}]"));
                    }
                }
            }
        }
    }
    assert!(windows > 40, "queries with a window: {windows}");
}

macro_rules! engine_tests {
    ($m:ident, $E:ty) => {
        mod $m {
            use super::*;
            type E = $E;
            #[test]
            fn every_cited_relation_and_revision_exists_in_the_snapshot() {
                super::every_cited_relation_and_revision_exists_in_the_snapshot::<E>();
            }
            #[test]
            fn an_old_snapshot_never_cites_what_came_after_it() {
                super::an_old_snapshot_never_cites_what_came_after_it::<E>();
            }
            #[test]
            fn frozen_corpus_cites_only_what_the_snapshot_holds() {
                super::frozen_corpus_cites_only_what_the_snapshot_holds::<E>();
            }
            #[test]
            fn window_and_provenance_filters_equal_the_brute_force_oracle() {
                super::window_and_provenance_filters_equal_the_brute_force_oracle::<E>();
            }
            #[test]
            fn frozen_corpus_filters_equal_the_oracle_one_condition_at_a_time() {
                super::frozen_corpus_filters_equal_the_oracle_one_condition_at_a_time::<E>();
            }
            #[test]
            fn inferred_is_never_declared_in_a_trace() {
                super::inferred_is_never_declared_in_a_trace::<E>();
            }
            #[test]
            fn the_clock_orders_nothing() {
                super::the_clock_orders_nothing::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, EmuLog);
engine_tests!(kerno, KernoDb);
