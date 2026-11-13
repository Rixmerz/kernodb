//! E7-02: the indexed execution of an associative query and its place in the
//! planner (`UnitSnapshot::indexed_assoc_query`, `assoc_query`,
//! `explain_assoc`) over `Emulated<LogEngine>` and `KernoDb`.
//!
//! * `differential_*` (I-32): the indexed path, the planner's answer and the
//!   reference path return the same result (hits, order, traces, visits,
//!   ending) or the same error, over generated graphs and queries that set
//!   every field of `AssocQuery`, over more relations than one scan page, and
//!   over the frozen evaluation corpus.
//! * `false_index_answers_*`: entries missing, extra, wrong, from a stale
//!   index, a record from the future and an absent index, written straight to
//!   the engine. Each is caught, the reference path answers, and the fallback
//!   is on the outcome, in the plan and in the observation. The one thing the
//!   checks cannot see (an entry missing at both ends of a relation, in a
//!   store too big for the planner's probes) is pinned as the documented limit.
//! * The rest: the plan, the observation, and the shape of the query.
//!
//! The reference path is the oracle; it never reads the adjacency index, so
//! damage to the index cannot change what it answers.

mod harness;

use std::collections::BTreeSet;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use harness::assoc::{CORPUS, Patch, QUERIES, corpus_query, load_corpus};
use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{Adjacency, Key, RelationRecord, TAG_REL_OUT};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

fn put_all<E: TxnEngine>(db: &UnitDb<E>, puts: Patch) {
    db.raw_commit(puts).expect("raw commit");
}

/// What the store holds now under each of `keys`, as a patch that puts it
/// back (a deletion for a key that is absent).
fn undo_of<E: TxnEngine>(db: &UnitDb<E>, keys: &[Vec<u8>]) -> Patch {
    keys.iter().map(|k| (k.clone(), db.raw_get(k).expect("get"))).collect()
}

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

// ---------------------------------------------------------------- fixtures

fn open<E: TxnEngine>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open")
}

fn new_unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: Vec::new(), payload: payload.to_vec() }
}

fn units<E: TxnEngine>(db: &UnitDb<E>, n: usize) -> Vec<UnitId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    let ids = (0..n).map(|i| cs.create(new_unit(&[i as u8])).expect("create")).collect();
    cs.commit().expect("commit");
    ids
}

fn meta(actor: Option<&str>, source: Option<&str>) -> ChangeMeta {
    ChangeMeta {
        actor: actor.map(|a| a.as_bytes().to_vec()),
        source: source.map(|s| s.as_bytes().to_vec()),
        ..ChangeMeta::default()
    }
}

const TYPES: [&str; 4] = ["cites", "cite", "refines", "t"];
const ACTORS: [&str; 3] = ["alice", "bot", "importer"];
const SOURCES: [&str; 3] = ["doc/1", "doc/2", "job/1"];

fn provenance(rng: &mut Rng) -> RelationProvenance {
    match rng.below(4) {
        0 => RelationProvenance::Declared,
        1 => RelationProvenance::Imported { source: b"wiki".to_vec() },
        2 => RelationProvenance::Algorithmic { origin: b"dedup".to_vec() },
        _ => RelationProvenance::Inferred { origin: b"model".to_vec(), confidence_permille: rng.below(1001) as u16 },
    }
}

fn pick<'a>(rng: &mut Rng, of: &'a [&'a str]) -> Option<&'a str> {
    rng.chance(3, 4).then(|| of[rng.below(of.len() as u64) as usize])
}

/// More relations in change sets with random actors and sources: self
/// relations and parallel ones happen, and the four provenance classes.
fn grow<E: TxnEngine>(db: &UnitDb<E>, rng: &mut Rng, u: &[UnitId], sets: u64) {
    for _ in 0..sets {
        let mut cs = db.begin_changeset(meta(pick(rng, &ACTORS), pick(rng, &SOURCES))).expect("begin");
        for _ in 0..rng.range(3, 8) {
            let (from, to) = (u[rng.below(u.len() as u64) as usize], u[rng.below(u.len() as u64) as usize]);
            let ty = TYPES[rng.below(TYPES.len() as u64) as usize];
            cs.create_relation(from, to, ty, provenance(rng)).expect("create_relation");
        }
        cs.commit().expect("commit");
    }
}

/// A graph of 10 to 17 units with about 30 to 60 relations, and a hub (unit 0)
/// that starts most of one change set, so that a query from the hub is one for
/// which the full scan is the cheaper plan.
fn world<E: TxnEngine>(rng: &mut Rng) -> (UnitDb<E>, Vec<UnitId>) {
    let db = open::<E>();
    let u = units(&db, 10 + rng.below(8) as usize);
    let sets = 6 + rng.below(4);
    grow(&db, rng, &u, sets);
    let mut cs = db.begin_changeset(meta(Some("hub"), None)).expect("begin");
    for _ in 0..30 {
        let to = u[rng.below(u.len() as u64) as usize];
        cs.create_relation(u[0], to, TYPES[rng.below(2) as usize], RelationProvenance::Declared).expect("hub");
    }
    cs.commit().expect("commit");
    (db, u)
}

/// A query that sets, over many draws, every field: now and then it is also
/// malformed or starts at a unit that does not exist.
fn random_query(rng: &mut Rng, u: &[UnitId], last_seq: u64) -> AssocQuery {
    let mut seeds: Vec<UnitId> = (0..rng.range(1, 3)).map(|_| u[rng.below(u.len() as u64) as usize]).collect();
    // The hub, following what it starts: the case where the scan is the cheaper plan.
    let from_the_hub = rng.chance(1, 5);
    if from_the_hub {
        seeds = vec![u[0]];
    }
    if rng.chance(1, 20) {
        seeds.push(UnitId([0xEE; 16]));
    }
    if rng.chance(1, 40) {
        seeds.clear();
    }
    let depth = if rng.chance(1, 25) { rng.range(0, 6) as u32 } else { rng.range(1, 4) as u32 };
    let limit = [1usize, 2, 3, 5, 50, 1000][rng.below(6) as usize];
    let budget = [1u64, 2, 3, 5, 8, 13, 40, 100_000][rng.below(8) as usize];
    let mut q = AssocQuery::new(seeds, depth, limit, budget);
    q.direction = [AssocDirection::Out, AssocDirection::In, AssocDirection::Both][rng.below(3) as usize];
    if from_the_hub {
        q.direction = AssocDirection::Out;
    }
    if rng.chance(1, 2) {
        let mut types: BTreeSet<String> = TYPES.iter().filter(|_| rng.chance(1, 2)).map(|t| (*t).to_owned()).collect();
        if rng.chance(1, 4) {
            types.insert("absent".into());
        }
        q.relation_types = Some(types);
    }
    if rng.chance(1, 3) {
        let (a, b) = (rng.range(1, last_seq + 1), rng.range(1, last_seq + 1));
        q.window = Some(SeqWindow { lo: CommitSeq(a.min(b)), hi: CommitSeq(a.max(b)) });
        if rng.chance(1, 30) {
            q.window = Some(SeqWindow { lo: CommitSeq(a.max(b) + 1), hi: CommitSeq(a.min(b)) });
        }
    }
    if rng.chance(1, 3) {
        let all =
            [ProvenanceKind::Declared, ProvenanceKind::Imported, ProvenanceKind::Algorithmic, ProvenanceKind::Inferred];
        q.provenance.kinds = Some(all.into_iter().filter(|_| rng.chance(1, 2)).collect());
    }
    if rng.chance(1, 4) {
        q.provenance.actor =
            Some(pick(rng, &["alice", "bot", "importer", "nobody"]).unwrap_or("hub").as_bytes().to_vec());
    }
    if rng.chance(1, 4) {
        q.provenance.source = Some(pick(rng, &SOURCES).unwrap_or("doc/1").as_bytes().to_vec());
    }
    q
}

// ------------------------------------------------------------ comparisons

/// What the two sides answered, as text: every field of a result, or the
/// error. Equal text means equal results.
fn said<T: std::fmt::Debug>(r: &Result<T, AssocError>) -> String {
    format!("{r:?}")
}

/// How many queries of a run took each route, to prove the run covered them.
#[derive(Default, Debug)]
struct Coverage {
    adjacency: u32,
    reference: u32,
    rejected: u32,
    seed_not_active: u32,
    exhausted: u32,
    limit_reached: u32,
    budget_exhausted: u32,
    deep: u32,
    types: u32,
    windows: u32,
    kinds: u32,
    actors: u32,
    sources: u32,
    directions: [u32; 3],
}

/// The three answers of `q` on `snap` are one answer.
#[track_caller]
fn check_query<E: Engine>(snap: &UnitSnapshot<E>, q: &AssocQuery, cover: &mut Coverage, ctx: &str) {
    let key = FingerprintKey::from_bytes([0x42; 32]);
    let want = snap.reference_assoc_query(q);
    let indexed = snap.indexed_assoc_query(q);
    assert_eq!(said(&indexed), said(&want), "{ctx}: indexed vs reference for {q:?}");

    let plan = snap.explain_assoc(&key, q);
    let out = snap.assoc_query(q);
    match &want {
        Err(AssocError::Query(_)) => {
            assert!(matches!(plan, Err(AssocError::Query(_))), "{ctx}: a rejected query has no plan");
            assert_eq!(said(&out.map(|o| o.result)), said(&want), "{ctx}");
            cover.rejected += 1;
        }
        Err(AssocError::SeedNotActive { .. }) => {
            plan.expect("a plan does not look at the seeds");
            assert_eq!(said(&out.map(|o| o.result)), said(&want), "{ctx}");
            cover.seed_not_active += 1;
        }
        Err(other) => panic!("{ctx}: the reference path failed on a sound store: {other:?}"),
        Ok(res) => {
            let plan = plan.expect("a valid query has a plan");
            assert_eq!(plan, snap.explain_assoc(&key, q).unwrap(), "{ctx}: the plan is deterministic");
            let out = out.expect("assoc_query");
            assert_eq!(out.result, *res, "{ctx}: planner vs reference for {q:?}");
            assert_eq!(out.fallback, None, "{ctx}: a sound store never falls back");
            assert_eq!(out.path, plan.chosen().path, "{ctx}: the path run is the path explained");
            match out.path {
                PlanPath::AssocAdjacency => cover.adjacency += 1,
                PlanPath::AssocReference => cover.reference += 1,
                other => panic!("{ctx}: unexpected path {other:?}"),
            }
            match res.ending {
                AssocEnding::Exhausted => cover.exhausted += 1,
                AssocEnding::LimitReached => cover.limit_reached += 1,
                AssocEnding::BudgetExhausted => cover.budget_exhausted += 1,
            }
            cover.deep += u32::from(res.hits.iter().any(|h| h.distance >= 2));
            cover.types += u32::from(q.relation_types.is_some());
            cover.windows += u32::from(q.window.is_some());
            cover.kinds += u32::from(q.provenance.kinds.is_some());
            cover.actors += u32::from(q.provenance.actor.is_some());
            cover.sources += u32::from(q.provenance.source.is_some());
            cover.directions[match q.direction {
                AssocDirection::Out => 0,
                AssocDirection::In => 1,
                AssocDirection::Both => 2,
            }] += 1;
        }
    }
}

// ------------------------------------------------------------ differential

fn differential_seeded<E: TxnEngine>() {
    let mut cover = Coverage::default();
    for seed in 1..=6 {
        let mut rng = Rng::new(seed);
        let (db, u) = world::<E>(&mut rng);
        let old = db.snapshot().unwrap();
        grow(&db, &mut rng, &u, 2);
        let new = db.snapshot().unwrap();
        for n in 0..60 {
            for (name, snap) in [("old", &old), ("new", &new)] {
                let q = random_query(&mut rng, &u, snap.seq().0);
                check_query(snap, &q, &mut cover, &format!("seed {seed} query {n} on the {name} snapshot"));
            }
        }
        let report = db.verify_units().unwrap();
        assert!(report.is_clean(), "seed {seed}: {:?}", report.findings);
    }
    // The generator reached every route and every field, or the test says nothing.
    let c = &cover;
    assert!(c.adjacency > 40 && c.reference > 5, "both paths must be planned: {c:?}");
    assert!(c.rejected > 5 && c.seed_not_active > 5, "rejections must be covered: {c:?}");
    assert!(c.exhausted > 40 && c.limit_reached > 10 && c.budget_exhausted > 10, "every ending: {c:?}");
    assert!(c.deep > 40, "several hops: {c:?}");
    assert!(c.types > 30 && c.windows > 30 && c.kinds > 30 && c.actors > 20 && c.sources > 20, "every filter: {c:?}");
    assert!(c.directions.iter().all(|n| *n > 30), "every direction: {c:?}");
}

/// More entries on one unit than one scan page (256), so the paged scan of the
/// index crosses a boundary while the traversal budget does too.
fn differential_across_scan_pages<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 4);
    let mut cs = db.begin_changeset(meta(Some("alice"), None)).unwrap();
    for i in 0..600usize {
        let (from, to, ty) = if i % 3 == 0 { (u[0], u[1], "a") } else { (u[2], u[0], "b") };
        cs.create_relation(from, to, ty, RelationProvenance::Declared).unwrap();
    }
    // One relation from a unit that reaches the crowded ones in a hop: the
    // index is the cheap plan for it, and the traversal crosses the pages.
    cs.create_relation(u[3], u[1], "a", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let snap = db.snapshot().unwrap();
    let mut cover = Coverage::default();
    let sets: [Option<&[&str]>; 4] = [None, Some(&["a"]), Some(&["b"]), Some(&["a", "b"])];
    for seed in [u[0], u[3]] {
        for types in sets {
            for budget in [1, 255, 256, 257, 400, 599, 600, 1200, 100_000] {
                for direction in [AssocDirection::Out, AssocDirection::In, AssocDirection::Both] {
                    for limit in [1, 2, 1000] {
                        let mut q = AssocQuery::new(vec![seed], 3, limit, budget);
                        q.direction = direction;
                        q.relation_types = types.map(|t| t.iter().map(|s| (*s).to_owned()).collect());
                        let ctx = format!("{seed} {types:?} budget {budget} {direction:?} limit {limit}");
                        check_query(&snap, &q, &mut cover, &ctx);
                    }
                }
            }
        }
    }
    assert!(cover.budget_exhausted > 10 && cover.limit_reached > 10 && cover.exhausted > 10, "{cover:?}");
    assert!(cover.adjacency > 20 && cover.reference > 20, "both plans: {cover:?}");
}

fn frozen_corpus_indexed_equals_reference<E: TxnEngine>() {
    let corpus = assoc_eval::parse_corpus(CORPUS).expect("corpus parses");
    let set = assoc_eval::parse_queries(QUERIES).expect("queries parse");
    let (db, ids) = load_corpus::<E>(&corpus);
    let snap = db.snapshot().unwrap();
    assert_eq!(set.queries.len(), 64);

    let mut cover = Coverage::default();
    for q in &set.queries {
        let query = corpus_query(&ids, q);
        check_query(&snap, &query, &mut cover, &q.id);
        // The same query under tight bounds: every ending, the same answer.
        let full = snap.reference_assoc_query(&query).unwrap();
        for (limit, budget) in [(3, 100_000), (1000, full.visited.max(2) - 1), (2, 7)] {
            let tight = AssocQuery { limit, visit_budget: budget, ..query.clone() };
            check_query(&snap, &tight, &mut cover, &format!("{} limit {limit} budget {budget}", q.id));
        }
    }
    // Several seeds, every relation, both directions, the hard maxima.
    let seeds: Vec<UnitId> = [4, 24, 44].iter().map(|i| ids[*i]).collect();
    let big = AssocQuery::new(seeds, MAX_ASSOC_DEPTH, MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS);
    check_query(&snap, &big, &mut cover, "three seeds");
    assert_eq!(snap.indexed_assoc_query(&big).unwrap().ending, AssocEnding::LimitReached);

    assert!(cover.deep >= 100, "the set must exercise several hops: {cover:?}");
    assert!(cover.exhausted > 60 && cover.limit_reached > 30 && cover.budget_exhausted > 30, "{cover:?}");
    assert_eq!(cover.rejected + cover.seed_not_active, 0, "{cover:?}");
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
}

// ------------------------------------------------------ false index answers

/// A ring `a→b a→c b→d c→d(u) d→e e→a`, one relation per change set, and
/// two more units `x`, `y` that no ring unit reaches. With `filler` relations
/// `x→y` the store holds more records than the planner's probe reads, so it
/// can no longer see the size of the index and the checks of each hop are what
/// is left.
struct Fixture<E: TxnEngine> {
    db: UnitDb<E>,
    u: Vec<UnitId>,
    r: Vec<RelationId>,
}

const RING: [(usize, usize, &str); 6] = [(0, 1, "t"), (0, 2, "t"), (1, 3, "t"), (2, 3, "u"), (3, 4, "t"), (4, 0, "t")];
const FORGED: RelationId = RelationId([0xAB; 16]);

fn fixture<E: TxnEngine>(filler: usize) -> Fixture<E> {
    let db = open::<E>();
    let u = units(&db, 7);
    let r = RING
        .iter()
        .map(|&(f, t, ty)| {
            let mut cs = db.begin_changeset(meta(Some("alice"), Some("doc/1"))).unwrap();
            let id = cs.create_relation(u[f], u[t], ty, RelationProvenance::Declared).unwrap();
            cs.commit().unwrap();
            id
        })
        .collect();
    if filler > 0 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        for _ in 0..filler {
            cs.create_relation(u[5], u[6], "z", RelationProvenance::Declared).unwrap();
        }
        cs.commit().unwrap();
    }
    Fixture { db, u, r }
}

fn ring_query(fx: &Fixture<impl TxnEngine>) -> AssocQuery {
    AssocQuery::new(vec![fx.u[0]], 4, 1000, 100_000)
}

fn out_key(unit: UnitId, ty: &str, edge: RelationId) -> Vec<u8> {
    Key::RelationOut { unit, relation_type: ty.into(), edge }.encode().unwrap()
}

fn in_key(unit: UnitId, ty: &str, edge: RelationId) -> Vec<u8> {
    Key::RelationIn { unit, relation_type: ty.into(), edge }.encode().unwrap()
}

fn rel_key(edge: RelationId) -> Vec<u8> {
    Key::Relation { edge }.encode().unwrap()
}

fn entry(other: UnitId) -> Option<Vec<u8>> {
    Some(Adjacency { other }.encode())
}

fn record(from: UnitId, to: UnitId, ty: &str, created_seq: CommitSeq) -> Option<Vec<u8>> {
    Some(
        RelationRecord { from, to, relation_type: ty.into(), created_seq, provenance: RelationProvenance::Declared }
            .encode()
            .unwrap(),
    )
}

/// How a false answer is caught.
#[derive(Clone, Copy, Debug)]
enum Caught {
    /// Before the traversal: the planner marks the adjacency path unusable.
    Planner(Unusable),
    /// During it: a hop failed its check or the index reported damage.
    Hop(FallbackCause),
}

struct Damage<E: TxnEngine> {
    name: &'static str,
    /// Any real commits the damage starts from, and the raw writes.
    patch: fn(&Fixture<E>) -> Patch,
    /// Whether the raw writes change what the reference path answers (a
    /// forged record does; an entry does not).
    changes_data: bool,
    small: Caught,
    large: Caught,
}

fn wrong_value<E: TxnEngine>(fx: &Fixture<E>) -> Patch {
    vec![(out_key(fx.u[0], "t", fx.r[0]), entry(fx.u[3]))]
}

fn damages<E: TxnEngine>() -> Vec<Damage<E>> {
    let unmirrored = Caught::Hop(FallbackCause::Invalid(Violation::Unmirrored));
    let corrupt = Caught::Hop(FallbackCause::Corrupt);
    let inconsistent = Caught::Planner(Unusable::AdjacencyInconsistent);
    vec![
        Damage {
            name: "missing entry: the mirror of a relation the traversal follows out",
            patch: |fx| vec![(in_key(fx.u[1], "t", fx.r[0]), None)],
            changes_data: false,
            small: inconsistent,
            large: unmirrored,
        },
        Damage {
            name: "missing entry: the followed side, seen from the other end",
            patch: |fx| vec![(out_key(fx.u[0], "t", fx.r[1]), None)],
            changes_data: false,
            small: inconsistent,
            // P1: the end that lost the entry now refuses its own read (count
            // record), before the mirror check of the other end is reached.
            large: corrupt,
        },
        Damage {
            name: "missing entry: a mirror that names another unit",
            patch: |fx| vec![(in_key(fx.u[1], "t", fx.r[0]), entry(fx.u[4]))],
            changes_data: false,
            small: unmirrored,
            large: unmirrored,
        },
        Damage {
            name: "extra entry without a record",
            patch: |fx| vec![(out_key(fx.u[0], "t", FORGED), entry(fx.u[2]))],
            changes_data: false,
            small: inconsistent,
            large: corrupt,
        },
        Damage {
            name: "extra entry with a record and no mirror",
            patch: |fx| {
                let (a, e) = (fx.u[0], fx.u[4]);
                vec![(rel_key(FORGED), record(a, e, "t", CommitSeq(2))), (out_key(a, "t", FORGED), entry(e))]
            },
            changes_data: true,
            small: inconsistent,
            // P1: an entry the count record did not record is refused by the
            // count before the mirror check is reached.
            large: corrupt,
        },
        Damage {
            name: "an entry that names the wrong unit",
            patch: wrong_value,
            changes_data: false,
            small: corrupt,
            large: corrupt,
        },
        Damage {
            name: "wrong revision: a record created after the snapshot",
            patch: |fx| {
                let seq = CommitSeq(fx.db.snapshot().unwrap().seq().0 + 50);
                vec![(rel_key(fx.r[2]), record(fx.u[1], fx.u[3], "t", seq))]
            },
            changes_data: true,
            small: Caught::Hop(FallbackCause::Invalid(Violation::BeyondSnapshot)),
            large: Caught::Hop(FallbackCause::Invalid(Violation::BeyondSnapshot)),
        },
        Damage {
            name: "stale index: entries of a relation deleted since",
            patch: |fx| {
                let mut cs = fx.db.begin_changeset(ChangeMeta::default()).unwrap();
                cs.delete_relation(fx.r[3]).unwrap();
                cs.commit().unwrap();
                vec![(out_key(fx.u[2], "u", fx.r[3]), entry(fx.u[3])), (in_key(fx.u[3], "u", fx.r[3]), entry(fx.u[2]))]
            },
            changes_data: false,
            small: inconsistent,
            large: corrupt,
        },
        Damage {
            name: "stale index: a relation created since has no entries",
            patch: |fx| {
                let mut cs = fx.db.begin_changeset(ChangeMeta::default()).unwrap();
                let id = cs.create_relation(fx.u[0], fx.u[5], "t", RelationProvenance::Declared).unwrap();
                cs.commit().unwrap();
                vec![(out_key(fx.u[0], "t", id), None), (in_key(fx.u[5], "t", id), None)]
            },
            changes_data: false,
            small: inconsistent,
            // P1: both entries lost, the counts of both ends still say one.
            large: corrupt,
        },
        Damage {
            name: "absent index: no outgoing entry at all",
            patch: |fx| {
                let snap = fx.db.raw_snapshot().unwrap();
                let (start, end) = Key::tag_range(TAG_REL_OUT);
                let all = snap.scan(Bound::Included(&start), Bound::Excluded(&end), 10_000).unwrap();
                all.into_iter().map(|(k, _)| (k, None)).collect()
            },
            changes_data: false,
            small: Caught::Planner(Unusable::AdjacencyAbsent),
            large: Caught::Planner(Unusable::AdjacencyAbsent),
        },
    ]
}

/// One damage on one store: the planner's answer is the reference's, the
/// fallback says why, and putting the bytes back puts the index path back.
fn false_answer_is_caught<E: TxnEngine>(d: &Damage<E>, filler: usize) {
    let large = filler > 0;
    let expected = if large { d.large } else { d.small };
    let ctx = format!("{} ({})", d.name, if large { "large store" } else { "small store" });
    let fx = fixture::<E>(filler);
    let q = ring_query(&fx);
    let key = FingerprintKey::from_bytes([0x42; 32]);

    let patch = (d.patch)(&fx);
    let undo = undo_of(&fx.db, &patch.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>());
    let baseline = fx.db.snapshot().unwrap().reference_assoc_query(&q).unwrap();
    put_all(&fx.db, patch);

    let snap = fx.db.snapshot().unwrap();
    // The reference path never reads the index: the damage cannot reach it.
    let want = snap.reference_assoc_query(&q).unwrap();
    assert_eq!(want == baseline, !d.changes_data, "{ctx}: does the damage change the data?");

    fx.db.observer().set_key(FingerprintKey::from_bytes([0x42; 32]));
    fx.db.observer().set_level(ObservationLevel::Fingerprint).unwrap();
    let plan = snap.explain_assoc(&key, &q).unwrap();
    let out = snap.assoc_query(&q).unwrap();
    assert_eq!(out.result, want, "{ctx}: the answer is the reference's");

    let abandoned = PlanPath::AssocAdjacency;
    let adjacency = plan.candidates()[1];
    match expected {
        Caught::Planner(why) => {
            assert_eq!(plan.chosen().path, PlanPath::AssocReference, "{ctx}");
            assert_eq!(adjacency.usability, Usability::Unusable(why), "{ctx}: the plan says why");
            assert_eq!(plan.reason(), Reason::AdjacencyUnusable(why), "{ctx}");
            assert_eq!(out.fallback, Some(Fallback { abandoned, cause: FallbackCause::Unusable(why) }), "{ctx}");
            assert_eq!(out.path, PlanPath::AssocReference, "{ctx}");
        }
        Caught::Hop(cause) => {
            // The plan cannot see this damage: it chose the index, the check refused it.
            assert_eq!(plan.chosen().path, abandoned, "{ctx}");
            assert_eq!(adjacency.usability, Usability::Usable, "{ctx}");
            assert_eq!(out.fallback, Some(Fallback { abandoned, cause }), "{ctx}");
            assert_eq!(out.path, PlanPath::AssocReference, "{ctx}");
            // The index path alone refuses it with the same class of error.
            match (cause, snap.indexed_assoc_query(&q)) {
                (FallbackCause::Invalid(v), Err(AssocError::IndexAnswer(got))) => assert_eq!(got, v, "{ctx}"),
                (
                    FallbackCause::Corrupt,
                    Err(AssocError::Unit(UnitError::Storage(kerno_core::Error::Corruption(_)))),
                ) => {}
                (_, other) => panic!("{ctx}: the index path answered {other:?}"),
            }
        }
    }

    // The observation says the same as the outcome.
    let records = fx.db.observer().records();
    let seen = records.last().expect("an observation");
    assert_eq!(seen.fingerprint.kind(), QueryKind::Associative, "{ctx}");
    assert_eq!(
        (seen.path, seen.fallback, seen.rows_returned),
        (Some(out.path), out.fallback, out.result.hits.len() as u64),
        "{ctx}"
    );
    assert_eq!(fx.db.observer().stats().fallbacks, u64::from(out.fallback.is_some()), "{ctx}");

    // Back to the original bytes: the index path answers, with no fallback.
    drop(snap);
    put_all(&fx.db, undo);
    let healed = fx.db.snapshot().unwrap();
    let again = healed.assoc_query(&q).unwrap();
    assert_eq!((again.path, again.fallback), (PlanPath::AssocAdjacency, None), "{ctx}: healed");
    assert_eq!(again.result, healed.reference_assoc_query(&q).unwrap(), "{ctx}: healed");
    let report = fx.db.verify_units().unwrap();
    assert!(report.is_clean(), "{ctx}: {:?}", report.findings);
}

fn false_index_answers_are_detected_and_answered_through_the_fallback<E: TxnEngine>() {
    for d in damages::<E>() {
        false_answer_is_caught(&d, 0);
        false_answer_is_caught(&d, 70);
    }
}

/// An entry missing at both ends of a relation leaves nothing for a check to
/// compare, and a store of more than 64 records is more than a probe sees
/// (acta §5, H-1): the answer is wrong and nothing flags it. A small store
/// shows the same damage to the planner. The verifier sees it in both.
/// A relation created since with both of its entries lost used to be the one
/// damage nothing flagged on a large store (K1, from both ends). With the
/// counts (P1) the large store catches it at the hop.
fn a_relation_lost_with_both_entries_is_caught_by_its_counts<E: TxnEngine>() {
    let missing = damages::<E>().into_iter().find(|d| d.name.contains("has no entries")).unwrap();
    assert!(matches!((missing.small, missing.large), (Caught::Planner(_), Caught::Hop(FallbackCause::Corrupt))));
    false_answer_is_caught(&missing, 0);
    false_answer_is_caught(&missing, 70);
}

// ------------------------------------------------- the fallback and its causes

/// A hop that fails for a reason that is not damage is the caller's error: a
/// second attempt would hide it.
fn a_failure_that_is_not_damage_is_not_hidden<E: TxnEngine>() {
    let fx = fixture::<E>(0);
    let q = ring_query(&fx);
    // A seed that is not an active unit is rejected before any path runs, on both.
    let ghost = AssocQuery::new(vec![UnitId([0xEE; 16])], 2, 10, 100);
    let snap = fx.db.snapshot().unwrap();
    for err in [snap.assoc_query(&ghost).map(|o| o.result), snap.indexed_assoc_query(&ghost)] {
        assert!(matches!(err, Err(AssocError::SeedNotActive { .. })), "{err:?}");
    }
    let zero = AssocQuery::new(vec![fx.u[0]], 0, 10, 100);
    assert!(matches!(snap.assoc_query(&zero), Err(AssocError::Query(AssocQueryError::DepthOutOfRange { .. }))));
    // Damage to a unit head the traversal opens fails the reference path too: the error is returned.
    let head = Key::UnitHead { unit: fx.u[1] }.encode().unwrap();
    let good = fx.db.raw_snapshot().unwrap().get(&head).unwrap().unwrap();
    put_all(&fx.db, vec![(head, Some(good[..good.len() - 1].to_vec()))]);
    let snap = fx.db.snapshot().unwrap();
    let err = snap.assoc_query(&q).unwrap_err();
    assert!(matches!(err, AssocError::Unit(UnitError::Storage(kerno_core::Error::Corruption(_)))), "{err:?}");
    assert_eq!(snap.reference_assoc_query(&q).unwrap_err().to_string(), err.to_string());
}

// ------------------------------------------------------- plan and observation

fn the_plan_compares_the_first_hop_and_says_why<E: TxnEngine>() {
    let fx = fixture::<E>(0);
    let snap = fx.db.snapshot().unwrap();
    let key = FingerprintKey::from_bytes([0x42; 32]);
    // Six relations. From `a`, both ways: two expansions of a 6-record scan
    // (12) against two reads for each of the 3 entries (6).
    let plan = snap.explain_assoc(&key, &ring_query(&fx)).unwrap();
    let costs: Vec<_> = plan.candidates().iter().map(|c| (c.path, c.cost, c.usability)).collect();
    assert_eq!(
        costs,
        [
            (PlanPath::AssocReference, Cost(12), Usability::Usable),
            (PlanPath::AssocAdjacency, Cost(6), Usability::Usable)
        ]
    );
    assert_eq!((plan.chosen().path, plan.reason(), plan.index()), (PlanPath::AssocAdjacency, Reason::LowestCost, None));
    assert_eq!(plan.fingerprint().kind(), QueryKind::Associative);
    assert!(plan.fingerprint().to_string().starts_with("assoc/blake3-keyed/v1/"));

    // Out only from `b`: one expansion, one entry; and a seed without relations.
    let mut q = AssocQuery::new(vec![fx.u[1]], 2, 10, 100);
    q.direction = AssocDirection::Out;
    let plan = snap.explain_assoc(&key, &q).unwrap();
    assert_eq!((plan.candidates()[0].cost, plan.candidates()[1].cost), (Cost(6), Cost(2)));
    let q = AssocQuery::new(vec![fx.u[6]], 2, 10, 100);
    let plan = snap.explain_assoc(&key, &q).unwrap();
    assert_eq!((plan.candidates()[0].cost, plan.candidates()[1].cost), (Cost(12), Cost(0)));

    // A store without relations: both cost nothing and the reference, first in
    // table order, answers.
    let empty = open::<E>();
    let u = units(&empty, 1);
    let snap = empty.snapshot().unwrap();
    let plan = snap.explain_assoc(&key, &AssocQuery::new(u.clone(), 1, 1, 1)).unwrap();
    assert_eq!(
        (plan.chosen().path, plan.candidates()[0].cost, plan.candidates()[1].cost),
        (PlanPath::AssocReference, Cost(0), Cost(0))
    );
    let out = snap.assoc_query(&AssocQuery::new(u, 1, 1, 1)).unwrap();
    assert_eq!((out.path, out.fallback, out.result.hits.len()), (PlanPath::AssocReference, None, 0));
}

fn observations_record_the_path_the_rows_and_the_class_of_an_error<E: TxnEngine>() {
    let fx = fixture::<E>(0);
    let snap = fx.db.snapshot().unwrap();
    let obs = fx.db.observer();
    obs.set_key(FingerprintKey::from_bytes([0x42; 32]));
    obs.set_level(ObservationLevel::Diagnostic).unwrap();
    let q = ring_query(&fx);
    let res = snap.assoc_query(&q).unwrap();
    let mut filtered = q.clone();
    filtered.relation_types = Some(["t".to_owned()].into());
    filtered.window = Some(SeqWindow { lo: CommitSeq(1), hi: CommitSeq(9) });
    filtered.provenance.kinds = Some([ProvenanceKind::Declared].into());
    filtered.provenance.actor = Some(b"alice".to_vec());
    filtered.provenance.source = Some(b"doc/1".to_vec());
    filtered.direction = AssocDirection::Out;
    snap.assoc_query(&filtered).unwrap();
    let ghost = AssocQuery::new(vec![UnitId([0xEE; 16])], 1, 1, 1);
    let _ = snap.assoc_query(&ghost);
    let _ = snap.assoc_query(&AssocQuery::new(Vec::new(), 1, 1, 1));

    let records = obs.records();
    assert_eq!(records.len(), 4);
    assert_eq!(
        (records[0].path, records[0].rows_returned, records[0].outcome, records[0].plan_kind()),
        (Some(PlanPath::AssocAdjacency), res.result.hits.len() as u64, Outcome::Ok, Some(PlanKind::Planned))
    );
    assert_eq!(records[0].commit_seq, snap.seq());
    let texts: Vec<_> = records.iter().map(|r| r.fingerprint_text.as_deref().unwrap()).collect();
    assert_eq!(texts[0], "assoc[seeds=?unit_ids,dir=both,depth=?u32,limit=?u32,budget=?u64]");
    assert_eq!(
        texts[1],
        "assoc[seeds=?unit_ids,dir=out,depth=?u32,limit=?u32,budget=?u64,types=?texts,window=?seqs,kinds=?kinds,actor=?bytes,source=?bytes]"
    );
    // A rejected query leaves a class, no path and no rows; never its message.
    for r in &records[2..] {
        assert_eq!((r.path, r.rows_returned, r.outcome), (None, 0, Outcome::Error(ErrorKind::Invalid)));
    }
    let stats = obs.stats();
    assert_eq!((stats.observed, stats.errors, stats.fallbacks), (4, 2, 0));
    assert_eq!(stats.executed(PlanPath::AssocAdjacency), 2);
    assert_eq!(stats.executed(PlanPath::AssocReference), 0);
    // Every path has a counter of its own.
    for other in [PlanPath::UnitsReference, PlanPath::IndexPrimary, PlanPath::RelationsReference, PlanPath::Adjacency] {
        assert_eq!(stats.executed(other), 0, "{other:?}");
    }

    // Reference chosen: a hub that starts most relations makes the scan cheaper.
    let hub = open::<E>();
    let u = units(&hub, 2);
    let mut cs = hub.begin_changeset(ChangeMeta::default()).unwrap();
    for _ in 0..5 {
        cs.create_relation(u[0], u[1], "t", RelationProvenance::Declared).unwrap();
    }
    cs.commit().unwrap();
    hub.observer().set_key(FingerprintKey::from_bytes([0x42; 32]));
    hub.observer().set_level(ObservationLevel::Aggregate).unwrap();
    let mut q = AssocQuery::new(vec![u[0]], 1, 10, 100);
    q.direction = AssocDirection::Out;
    let out = hub.snapshot().unwrap().assoc_query(&q).unwrap();
    assert_eq!((out.path, out.fallback, out.result.hits.len()), (PlanPath::AssocReference, None, 1));
    assert_eq!(hub.observer().stats().executed(PlanPath::AssocReference), 1);
}

/// The shape hashes structure and no literal.
fn the_shape_ignores_literals_and_sees_structure<E: TxnEngine>() {
    let _ = std::marker::PhantomData::<E>;
    let key = FingerprintKey::from_bytes([3; 32]);
    let of = |q: &AssocQuery| NormalizedQuery::of_assoc(q).fingerprint(&key);
    let a = UnitId([1; 16]);
    let b = UnitId([2; 16]);
    let mut base = AssocQuery::new(vec![a], 2, 10, 100);
    base.relation_types = Some(["x".to_owned()].into());
    base.window = Some(SeqWindow { lo: CommitSeq(1), hi: CommitSeq(2) });
    base.provenance.kinds = Some([ProvenanceKind::Declared].into());
    base.provenance.actor = Some(b"alice".to_vec());
    base.provenance.source = Some(b"doc".to_vec());

    // Other literals: same fingerprint.
    let mut other = AssocQuery::new(vec![b, a, b], 4, 1000, 100_000);
    other.relation_types = Some(["y".to_owned(), "z".to_owned()].into());
    other.window = Some(SeqWindow { lo: CommitSeq(7), hi: CommitSeq(900) });
    other.provenance.kinds = Some([ProvenanceKind::Inferred, ProvenanceKind::Imported].into());
    other.provenance.actor = Some(b"bot".to_vec());
    other.provenance.source = Some(b"job".to_vec());
    assert_eq!(of(&base), of(&other));
    assert_eq!(NormalizedQuery::of_assoc(&base).text(), NormalizedQuery::of_assoc(&other).text());

    // Other structure: each condition, and the direction, changes it.
    let mut seen = vec![of(&base)];
    let variants: [fn(&mut AssocQuery); 6] = [
        |q| q.direction = AssocDirection::Out,
        |q| q.direction = AssocDirection::In,
        |q| q.relation_types = None,
        |q| q.window = None,
        |q| q.provenance.kinds = None,
        |q| q.provenance.actor = None,
    ];
    for v in variants {
        let mut q = base.clone();
        v(&mut q);
        seen.push(of(&q));
    }
    let mut q = base.clone();
    q.provenance.source = None;
    seen.push(of(&q));
    let distinct: BTreeSet<_> = seen.iter().map(|f| f.to_string()).collect();
    assert_eq!(distinct.len(), seen.len(), "each structure has its own fingerprint");
    assert_ne!(
        of(&base).to_string(),
        NormalizedQuery::of_relations(&RelationQuery { unit: a, direction: Direction::Out, relation_type: None })
            .fingerprint(&key)
            .to_string()
    );
}

macro_rules! engine_tests {
    ($m:ident, $E:ty) => {
        mod $m {
            use super::*;
            type E = $E;
            #[test]
            fn differential_seeded() {
                super::differential_seeded::<E>();
            }
            #[test]
            fn differential_across_scan_pages() {
                super::differential_across_scan_pages::<E>();
            }
            #[test]
            fn frozen_corpus_indexed_equals_reference() {
                super::frozen_corpus_indexed_equals_reference::<E>();
            }
            #[test]
            fn false_index_answers_are_detected_and_answered_through_the_fallback() {
                super::false_index_answers_are_detected_and_answered_through_the_fallback::<E>();
            }
            #[test]
            fn a_relation_lost_with_both_entries_is_caught_by_its_counts() {
                super::a_relation_lost_with_both_entries_is_caught_by_its_counts::<E>();
            }
            #[test]
            fn a_failure_that_is_not_damage_is_not_hidden() {
                super::a_failure_that_is_not_damage_is_not_hidden::<E>();
            }
            #[test]
            fn the_plan_compares_the_first_hop_and_says_why() {
                super::the_plan_compares_the_first_hop_and_says_why::<E>();
            }
            #[test]
            fn observations_record_the_path_the_rows_and_the_class_of_an_error() {
                super::observations_record_the_path_the_rows_and_the_class_of_an_error::<E>();
            }
            #[test]
            fn the_shape_ignores_literals_and_sees_structure() {
                super::the_shape_ignores_literals_and_sees_structure::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, EmuLog);
engine_tests!(kerno, KernoDb);
