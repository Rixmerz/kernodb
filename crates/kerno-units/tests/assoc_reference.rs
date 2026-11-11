//! E7-01: the associative query model and its reference path
//! (`UnitSnapshot::reference_assoc_query`) over `Emulated<LogEngine>` and
//! `KernoDb`.
//!
//! * Fixed vectors: small graphs (a diamond with a cycle, a parallel relation
//!   and a self-relation; a graph with every provenance class) and the exact
//!   hits, distances and traces the order of `assoc.rs` gives.
//! * Validation: every rejection, and the hard maxima themselves accepted.
//! * The three endings, including a budget that runs out mid-level, and that
//!   a limited or budgeted answer agrees with the unbounded one.
//! * Determinism, snapshot isolation, the explicit handling of units that are
//!   not active, and that the path never reads the adjacency index.
//! * The frozen evaluation corpus: every trace is verified against the
//!   snapshot, and the hits equal those of a breadth-first oracle written in
//!   the test over the raw relations.
//!
//! The expectations are written by hand or computed by the oracle of this
//! file, never read back from the code under test.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::assoc_eval::{self, Direction as EvalDirection, Kind as EvalKind, Provenance as StoryProvenance};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{Adjacency, Key, RelationRecord};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const CORPUS: &str = include_str!("../../kerno-testkit/data/assoc-eval/corpus.txt");
const QUERIES: &str = include_str!("../../kerno-testkit/data/assoc-eval/queries.txt");

// ---------------------------------------------------------------- fixtures

fn open_with<E: TxnEngine>(ids: Arc<dyn UnitIdSource>) -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), ids);
    c.engine = Options::small_for_tests();
    UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open")
}

fn open<E: TxnEngine>() -> UnitDb<E> {
    open_with(Arc::new(CounterIds::new(1)))
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

/// One relation in its own change set, so its `created_seq` is its own.
fn relate_by<E: TxnEngine>(
    db: &UnitDb<E>,
    (from, to): (UnitId, UnitId),
    ty: &str,
    p: RelationProvenance,
    m: ChangeMeta,
) -> RelationId {
    let mut cs = db.begin_changeset(m).expect("begin");
    let id = cs.create_relation(from, to, ty, p).expect("create_relation");
    cs.commit().expect("commit");
    id
}

fn relate<E: TxnEngine>(db: &UnitDb<E>, ends: (UnitId, UnitId), ty: &str) -> RelationId {
    relate_by(db, ends, ty, RelationProvenance::Declared, ChangeMeta::default())
}

/// The shared diamond-with-a-cycle graph. Units `a..g` are `u[0]..u[6]`
/// (ids 1..=7); the change set that creates them is commit 1. Relations, each
/// in its own change set, `r[k]` created at commit `k + 2`:
///
/// ```text
/// r1 a->b t   r2 a->c t   r3 b->d t   r4 c->d t   r5 d->a t
/// r6 d->e u   r7 e->f t   r8 a->b t (parallel to r1)   r9 c->c t (self)
/// ```
///
/// `g` has no relation.
struct Diamond<E: TxnEngine> {
    db: UnitDb<E>,
    u: Vec<UnitId>,
    r: Vec<RelationId>,
}

const DIAMOND: [(usize, usize, &str); 9] = [
    (0, 1, "t"),
    (0, 2, "t"),
    (1, 3, "t"),
    (2, 3, "t"),
    (3, 0, "t"),
    (3, 4, "u"),
    (4, 5, "t"),
    (0, 1, "t"),
    (2, 2, "t"),
];

fn diamond<E: TxnEngine>() -> Diamond<E> {
    let db = open::<E>();
    let u = units(&db, 7);
    let r = DIAMOND.iter().map(|&(f, t, ty)| relate(&db, (u[f], u[t]), ty)).collect();
    Diamond { db, u, r }
}

fn big(seeds: &[UnitId], depth: u32) -> AssocQuery {
    AssocQuery::new(seeds.to_vec(), depth, MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS)
}

/// Hits as `(unit, relation ids of the path)`.
type Shape = Vec<(UnitId, Vec<RelationId>)>;

fn shape(res: &AssocResult) -> Shape {
    res.hits.iter().map(|h| (h.unit, h.path.iter().map(|s| s.relation).collect())).collect()
}

fn run<E: Engine>(snap: &UnitSnapshot<E>, q: &AssocQuery) -> AssocResult {
    snap.reference_assoc_query(q).expect("reference_assoc_query")
}

fn rejected<E: Engine>(snap: &UnitSnapshot<E>, q: &AssocQuery) -> AssocQueryError {
    match snap.reference_assoc_query(q) {
        Err(AssocError::Query(e)) => e,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

type Patch = Vec<(Vec<u8>, Option<Vec<u8>>)>;

fn put_all<E: Engine>(db: &E, puts: Patch) {
    let mut b = WriteBatch::new();
    for (k, v) in puts {
        match v {
            Some(v) => b.put(k, v),
            None => b.delete(k),
        };
    }
    db.commit(&b).unwrap();
}

// -------------------------------------------------------------- validation

fn validation_rejects_every_unbounded_or_malformed_query<E: TxnEngine>() {
    let g = diamond::<E>();
    let snap = g.db.snapshot().unwrap();
    let seeds = &g.u[..1];
    let ok = || AssocQuery::new(seeds.to_vec(), 2, 10, 100);

    assert_eq!(rejected(&snap, &AssocQuery::new(Vec::new(), 2, 10, 100)), AssocQueryError::NoSeeds);
    for depth in [0, 5, u32::MAX] {
        let q = AssocQuery { max_depth: depth, ..ok() };
        assert_eq!(rejected(&snap, &q), AssocQueryError::DepthOutOfRange { depth });
    }
    for limit in [0, 1_001, usize::MAX] {
        let q = AssocQuery { limit, ..ok() };
        assert_eq!(rejected(&snap, &q), AssocQueryError::LimitOutOfRange { limit });
    }
    for budget in [0, 100_001, u64::MAX] {
        let q = AssocQuery { visit_budget: budget, ..ok() };
        assert_eq!(rejected(&snap, &q), AssocQueryError::BudgetOutOfRange { budget });
    }
    let q = AssocQuery { relation_types: Some(BTreeSet::new()), ..ok() };
    assert_eq!(rejected(&snap, &q), AssocQueryError::EmptyRelationTypes);
    let q = AssocQuery { provenance: ProvenanceFilter { kinds: Some(BTreeSet::new()), ..Default::default() }, ..ok() };
    assert_eq!(rejected(&snap, &q), AssocQueryError::EmptyProvenanceKinds);
    let window = SeqWindow { lo: CommitSeq(5), hi: CommitSeq(4) };
    assert_eq!(
        rejected(&snap, &AssocQuery { window: Some(window), ..ok() }),
        AssocQueryError::InvertedWindow { lo: CommitSeq(5), hi: CommitSeq(4) }
    );

    // `validate` alone reports the same, without a snapshot.
    assert_eq!(AssocQuery::new(Vec::new(), 2, 10, 100).validate(), Err(AssocQueryError::NoSeeds));
    assert_eq!(ok().validate(), Ok(()));
    // The first failure wins in a fixed order: seeds, depth, limit, budget.
    assert_eq!(AssocQuery::new(Vec::new(), 0, 0, 0).validate(), Err(AssocQueryError::NoSeeds));
    assert_eq!(AssocQuery::new(g.u.clone(), 0, 0, 0).validate(), Err(AssocQueryError::DepthOutOfRange { depth: 0 }));
    assert_eq!(AssocQuery::new(g.u.clone(), 1, 0, 0).validate(), Err(AssocQueryError::LimitOutOfRange { limit: 0 }));
    // Every error says which bound it is.
    assert!(AssocQueryError::NoSeeds.to_string().contains("seed"));
    assert!(AssocQueryError::DepthOutOfRange { depth: 9 }.to_string().contains("max_depth 9"));
    assert!(AssocQueryError::LimitOutOfRange { limit: 0 }.to_string().contains("limit 0"));
    assert!(AssocQueryError::BudgetOutOfRange { budget: 7 }.to_string().contains("visit_budget 7"));
}

fn hard_maxima_are_public_and_accepted<E: TxnEngine>() {
    assert_eq!((MAX_ASSOC_DEPTH, MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS), (4, 1_000, 100_000));
    let g = diamond::<E>();
    let snap = g.db.snapshot().unwrap();
    // A caller may lower the bounds or sit exactly on the maxima.
    let at_max = AssocQuery::new(vec![g.u[0]], MAX_ASSOC_DEPTH, MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS);
    assert_eq!(at_max.validate(), Ok(()));
    assert_eq!(run(&snap, &at_max).hits.len(), 5);
    let lowest = AssocQuery::new(vec![g.u[0]], 1, 1, 1);
    assert_eq!(lowest.validate(), Ok(()));
    assert_eq!(run(&snap, &lowest).hits.len(), 1);
}

fn seeds_must_be_active_units<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 4);
    relate(&db, (u[0], u[1]), "t");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[2], None).unwrap();
    cs.delete(u[3], None).unwrap();
    cs.commit().unwrap();
    let snap = db.snapshot().unwrap();

    for (name, seed) in [("archived", u[2]), ("deleted", u[3]), ("unknown", UnitId([0xEE; 16]))] {
        // Alone, or next to a good seed: the whole query is refused.
        for seeds in [vec![seed], vec![u[0], seed]] {
            let e = snap.reference_assoc_query(&big(&seeds, 2)).unwrap_err();
            assert!(matches!(e, AssocError::SeedNotActive { unit } if unit == seed), "{name}: {e}");
            assert!(e.to_string().contains(&seed.to_string()), "{name}: {e}");
        }
    }
    // Validation comes before the seed check.
    let e = snap.reference_assoc_query(&AssocQuery::new(vec![u[2]], 0, 1, 1)).unwrap_err();
    assert!(matches!(e, AssocError::Query(AssocQueryError::DepthOutOfRange { .. })), "{e}");
}

// ----------------------------------------------------------------- vectors

fn seeds_are_not_results_and_order_does_not_depend_on_them<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let snap = g.db.snapshot().unwrap();

    // b and c are seeds; d is reachable from both at distance 1 and keeps the
    // path of the seed with the lower unit id (b). The self-relation r9 of c
    // reaches no new unit. Neither seed is a hit.
    let want: Shape = vec![(u[3], vec![r[2]])];
    let mut q = big(&[u[1], u[2]], 1);
    q.direction = AssocDirection::Out;
    for seeds in [vec![u[1], u[2]], vec![u[2], u[1]], vec![u[2], u[1], u[2], u[1]]] {
        q.seeds = seeds;
        assert_eq!(shape(&run(&snap, &q)), want);
    }
    // A seed that another seed reaches is still not a hit: a and b are seeds.
    let mut q = big(&[u[0], u[1]], 1);
    q.direction = AssocDirection::Out;
    assert_eq!(shape(&run(&snap, &q)), vec![(u[2], vec![r[1]]), (u[3], vec![r[2]])]);
    // Sorted by unit id inside a level, not by discovery.
    let mut q = big(&[u[3]], 1);
    q.direction = AssocDirection::Both;
    let res = run(&snap, &q);
    let ids: Vec<_> = res.hits.iter().map(|h| h.unit).collect();
    assert_eq!(ids, vec![u[0], u[1], u[2], u[4]]);
}

fn out_traversal_vector<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let snap = g.db.snapshot().unwrap();
    let mut q = big(&[u[0]], 4);
    q.direction = AssocDirection::Out;
    let res = run(&snap, &q);
    // Parallel r8 (a->b) and self r9 (c->c) add nothing; d is reached by b
    // (r3) and by c (r4) and keeps the path through b; d->a (r5) closes the
    // cycle without returning the seed.
    let want: Shape = vec![
        (u[1], vec![r[0]]),
        (u[2], vec![r[1]]),
        (u[3], vec![r[0], r[2]]),
        (u[4], vec![r[0], r[2], r[5]]),
        (u[5], vec![r[0], r[2], r[5], r[6]]),
    ];
    assert_eq!(shape(&res), want);
    assert_eq!(res.hits.iter().map(|h| h.distance).collect::<Vec<_>>(), vec![1, 1, 2, 3, 4]);
    assert_eq!(res.ending, AssocEnding::Exhausted);
    // a: r1 r2 r8; b: r3; c: r4 r9; d: r5 r6; e: r7 (f, at depth 4, is not expanded).
    assert_eq!(res.visited, 9);

    // The depth bound cuts the same answer.
    for (depth, hits) in [(1, 2), (2, 3), (3, 4)] {
        let mut q = big(&[u[0]], depth);
        q.direction = AssocDirection::Out;
        let res = run(&snap, &q);
        assert_eq!(shape(&res), want[..hits].to_vec(), "depth {depth}");
        assert_eq!(res.ending, AssocEnding::Exhausted, "the depth bound is not a limit");
    }
}

fn in_traversal_vector<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let snap = g.db.snapshot().unwrap();
    let mut q = big(&[u[5]], 4);
    q.direction = AssocDirection::In;
    let res = run(&snap, &q);
    // f <- e <- d <- {b (r3), c (r4)} <- a. b is expanded before c, so a
    // keeps the path through b and its first relation r1 (r8 is parallel).
    let want: Shape = vec![
        (u[4], vec![r[6]]),
        (u[3], vec![r[6], r[5]]),
        (u[1], vec![r[6], r[5], r[2]]),
        (u[2], vec![r[6], r[5], r[3]]),
        (u[0], vec![r[6], r[5], r[2], r[0]]),
    ];
    assert_eq!(shape(&res), want);
    assert_eq!(res.ending, AssocEnding::Exhausted);
    // f: r7; e: r6; d: r3 r4; b: r1 r8; c: r2 r9 (a, at depth 4, is not expanded).
    assert_eq!(res.visited, 8);
}

fn both_traversal_vector_with_cycle_and_self_relation<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let snap = g.db.snapshot().unwrap();
    let mut q = big(&[u[3]], 1);
    q.direction = AssocDirection::Both;
    let res = run(&snap, &q);
    // d starts r5 (->a) and r6 (->e) and ends r3 (b->) and r4 (c->).
    let want: Shape = vec![(u[0], vec![r[4]]), (u[1], vec![r[2]]), (u[2], vec![r[3]]), (u[4], vec![r[5]])];
    assert_eq!(shape(&res), want);
    assert_eq!(res.visited, 4);

    // A relation from c to itself has two adjacency entries: followed in both
    // directions it is examined twice, and it never returns c.
    let q = big(&[u[2]], 1);
    let res = run(&snap, &q);
    assert_eq!(shape(&res), vec![(u[0], vec![r[1]]), (u[3], vec![r[3]])]);
    assert_eq!(res.visited, 4, "c starts r4 and r9 and ends r2 and r9");
    let mut q = big(&[u[2]], 1);
    q.direction = AssocDirection::Out;
    assert_eq!(run(&snap, &q).visited, 2);

    // Both, four hops from a: the whole component, each unit once.
    let res = run(&snap, &big(&[u[0]], 4));
    let mut units: Vec<_> = res.hits.iter().map(|h| h.unit).collect();
    units.sort();
    assert_eq!(units, vec![u[1], u[2], u[3], u[4], u[5]]);
    assert!(res.hits.iter().all(|h| h.unit != u[6]), "g has no relation");
    assert_eq!(res.hits.iter().map(|h| h.distance).collect::<Vec<_>>(), vec![1, 1, 1, 2, 3]);
}

fn trace_carries_the_fields_of_each_relation_and_the_head_revision<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 3);
    let r1 = relate_by(
        &db,
        (u[0], u[1]),
        "cites",
        RelationProvenance::Imported { source: b"wiki".to_vec() },
        meta(Some("imp"), Some("wiki/1")),
    );
    let r2 = relate_by(
        &db,
        (u[2], u[1]),
        "supports",
        RelationProvenance::Inferred { origin: b"m".to_vec(), confidence_permille: 640 },
        ChangeMeta::default(),
    );
    // The head revision of u[2] moves to 3; u[1] stays at 1.
    for p in [b"x", b"y"] {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(u[2], UnitUpdate { payload: p.to_vec(), schema_version: None, metadata: None }, None).unwrap();
        cs.commit().unwrap();
    }
    let snap = db.snapshot().unwrap();
    let res = run(&snap, &big(&[u[0]], 2));
    assert_eq!(res.ending, AssocEnding::Exhausted);
    let want = vec![
        AssocHit {
            unit: u[1],
            distance: 1,
            revision: RevNo(1),
            path: vec![TraceStep {
                relation: r1,
                relation_type: "cites".into(),
                from: u[0],
                to: u[1],
                created_seq: CommitSeq(2),
                provenance: ProvenanceKind::Imported,
            }],
        },
        AssocHit {
            unit: u[2],
            distance: 2,
            revision: RevNo(3),
            // The relation is reported with its own ends (u2 -> u1), though
            // the traversal followed it from u1 to u2.
            path: vec![
                TraceStep {
                    relation: r1,
                    relation_type: "cites".into(),
                    from: u[0],
                    to: u[1],
                    created_seq: CommitSeq(2),
                    provenance: ProvenanceKind::Imported,
                },
                TraceStep {
                    relation: r2,
                    relation_type: "supports".into(),
                    from: u[2],
                    to: u[1],
                    created_seq: CommitSeq(3),
                    provenance: ProvenanceKind::Inferred,
                },
            ],
        },
    ];
    assert_eq!(res.hits, want);
    // An inferred relation stays inferred in the trace (I-39).
    assert_ne!(res.hits[1].path[1].provenance, ProvenanceKind::Declared);
}

// ---------------------------------------------------------------- filters

fn relation_type_filter<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let snap = g.db.snapshot().unwrap();
    let types = |ts: &[&str]| Some(ts.iter().map(|t| (*t).to_owned()).collect::<BTreeSet<_>>());

    // Only `u`: from d the single u relation leads to e; e's t relation (r7)
    // is not followed, its u relation (r6) leads back to d.
    let mut q = big(&[u[3]], 4);
    q.relation_types = types(&["u"]);
    let res = run(&snap, &q);
    assert_eq!(shape(&res), vec![(u[4], vec![r[5]])]);
    // Examined: d starts r6 and ends none of type u; e ends r6.
    assert_eq!(res.visited, 2, "entries of other types are not examined");

    // Only `t` from a, out: r6 is never followed, so e and f are out of reach.
    let mut q = big(&[u[0]], 4);
    q.direction = AssocDirection::Out;
    q.relation_types = types(&["t"]);
    let res = run(&snap, &q);
    assert_eq!(shape(&res), vec![(u[1], vec![r[0]]), (u[2], vec![r[1]]), (u[3], vec![r[0], r[2]])]);

    // A set with both types, and a type no relation has.
    q.relation_types = types(&["u", "t"]);
    assert_eq!(run(&snap, &q).hits.len(), 5);
    q.relation_types = types(&["nothing"]);
    let res = run(&snap, &q);
    assert_eq!((res.hits.len(), res.visited, res.ending), (0, 0, AssocEnding::Exhausted));
    // A prefix of a type is not the type.
    q.relation_types = types(&["tt"]);
    assert!(run(&snap, &q).hits.is_empty());
}

fn window_is_inclusive_and_applies_to_each_relation_followed<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let snap = g.db.snapshot().unwrap();
    // r1..r9 were created at commits 2..=10.
    for (k, rel) in g.r.iter().enumerate() {
        assert_eq!(snap.get_relation(*rel).unwrap().unwrap().created_seq, CommitSeq(k as u64 + 2));
    }
    let win = |lo, hi| Some(SeqWindow { lo: CommitSeq(lo), hi: CommitSeq(hi) });
    let out = |w| {
        let mut q = big(&[u[0]], 4);
        q.direction = AssocDirection::Out;
        q.window = w;
        run(&snap, &q)
    };

    // [3,5]: r2 (3) is on the lower bound and r4 (5) on the upper one. Out of
    // the window: r1, r8 at the first hop, r9, r5, r6 later.
    let res = out(win(3, 5));
    assert_eq!(shape(&res), vec![(u[2], vec![r[1]]), (u[3], vec![r[1], r[3]])]);
    assert_eq!(res.visited, 7, "a: 3, c: 2, d: 2; the window filters after the entry is examined");
    // One-commit windows keep exactly one relation.
    assert_eq!(shape(&out(win(2, 2))), vec![(u[1], vec![r[0]])]);
    assert_eq!(shape(&out(win(3, 3))), vec![(u[2], vec![r[1]])]);
    // A window below, or above, every relation, is empty and exhausted.
    for w in [win(0, 1), win(11, 50)] {
        let res = out(w);
        assert_eq!((res.hits.len(), res.ending), (0, AssocEnding::Exhausted));
    }
    // The window covers the whole history: same answer as no window.
    assert_eq!(shape(&out(win(0, u64::MAX))), shape(&out(None)));
    // A relation outside the window blocks the path even if the next one is
    // inside it: r1 (2) is outside [3,4], so r3 (4) is never reached from b.
    assert_eq!(shape(&out(win(3, 4))), vec![(u[2], vec![r[1]])]);
}

struct Provenanced<E: TxnEngine> {
    db: UnitDb<E>,
    p: Vec<UnitId>,
    e: Vec<RelationId>,
}

/// Units `p0..p5` and seven relations of type `t`, each in its own change set:
///
/// ```text
/// e1 p0->p1 declared             alice  doc/1
/// e2 p0->p2 imported wiki        importer wiki/1
/// e3 p0->p3 algorithmic dedup    bot    job/1
/// e4 p0->p4 inferred m 700       bot    job/2
/// e5 p1->p2 declared             alice  doc/1
/// e6 p2->p5 declared             alice  doc/2
/// e7 p3->p5 declared             (no actor, no source)
/// ```
fn provenanced<E: TxnEngine>() -> Provenanced<E> {
    let db = open::<E>();
    let p = units(&db, 6);
    let imported = RelationProvenance::Imported { source: b"wiki".to_vec() };
    let algorithmic = RelationProvenance::Algorithmic { origin: b"dedup".to_vec() };
    let inferred = RelationProvenance::Inferred { origin: b"m".to_vec(), confidence_permille: 700 };
    let d = RelationProvenance::Declared;
    let e = vec![
        relate_by(&db, (p[0], p[1]), "t", d.clone(), meta(Some("alice"), Some("doc/1"))),
        relate_by(&db, (p[0], p[2]), "t", imported, meta(Some("importer"), Some("wiki/1"))),
        relate_by(&db, (p[0], p[3]), "t", algorithmic, meta(Some("bot"), Some("job/1"))),
        relate_by(&db, (p[0], p[4]), "t", inferred, meta(Some("bot"), Some("job/2"))),
        relate_by(&db, (p[1], p[2]), "t", d.clone(), meta(Some("alice"), Some("doc/1"))),
        relate_by(&db, (p[2], p[5]), "t", d.clone(), meta(Some("alice"), Some("doc/2"))),
        relate_by(&db, (p[3], p[5]), "t", d, ChangeMeta::default()),
    ];
    Provenanced { db, p, e }
}

fn provenance_filters_by_kind_actor_and_source<E: TxnEngine>() {
    let g = provenanced::<E>();
    let (p, e) = (&g.p, &g.e);
    let snap = g.db.snapshot().unwrap();
    let kinds = |ks: &[ProvenanceKind]| Some(ks.iter().copied().collect::<BTreeSet<_>>());
    let bytes = |s: &str| Some(s.as_bytes().to_vec());
    let go = |f: ProvenanceFilter| {
        let mut q = big(&[p[0]], 3);
        q.direction = AssocDirection::Out;
        q.provenance = f;
        run(&snap, &q)
    };

    // No filter: p5 is reached by p2 (e2, e6) before p3 (e3, e7), by id order.
    let res = go(ProvenanceFilter::default());
    let want: Shape =
        vec![(p[1], vec![e[0]]), (p[2], vec![e[1]]), (p[3], vec![e[2]]), (p[4], vec![e[3]]), (p[5], vec![e[1], e[5]])];
    assert_eq!(shape(&res), want);

    // Kind: declared only. The imported path to p2 is closed, so p2 is
    // reached through p1 and p5 one hop later; the other three kinds are not.
    let res = go(ProvenanceFilter { kinds: kinds(&[ProvenanceKind::Declared]), ..Default::default() });
    let want: Shape = vec![(p[1], vec![e[0]]), (p[2], vec![e[0], e[4]]), (p[5], vec![e[0], e[4], e[5]])];
    assert_eq!(shape(&res), want);
    assert_eq!(res.visited, 6, "p0: 4 entries, p1: 1, p2: 1");
    // Kinds imported and inferred: p2 and p4, and no declared relation after.
    let res = go(ProvenanceFilter {
        kinds: kinds(&[ProvenanceKind::Inferred, ProvenanceKind::Imported]),
        ..Default::default()
    });
    assert_eq!(shape(&res), vec![(p[2], vec![e[1]]), (p[4], vec![e[3]])]);
    let kinds_seen: Vec<_> = res.hits.iter().map(|h| h.path[0].provenance).collect();
    assert_eq!(kinds_seen, vec![ProvenanceKind::Imported, ProvenanceKind::Inferred]);
    // Algorithmic alone.
    let res = go(ProvenanceFilter { kinds: kinds(&[ProvenanceKind::Algorithmic]), ..Default::default() });
    assert_eq!(shape(&res), vec![(p[3], vec![e[2]])]);

    // Actor "bot": e3 and e4; e7 (a change set without actor) is not followed.
    let res = go(ProvenanceFilter { actor: bytes("bot"), ..Default::default() });
    assert_eq!(shape(&res), vec![(p[3], vec![e[2]]), (p[4], vec![e[3]])]);
    // Actor "alice": e1, e5, e6.
    let res = go(ProvenanceFilter { actor: bytes("alice"), ..Default::default() });
    assert_eq!(shape(&res), vec![(p[1], vec![e[0]]), (p[2], vec![e[0], e[4]]), (p[5], vec![e[0], e[4], e[5]])]);
    // Source: the source of the change set, not the origin of the provenance.
    let res = go(ProvenanceFilter { source: bytes("job/2"), ..Default::default() });
    assert_eq!(shape(&res), vec![(p[4], vec![e[3]])]);
    let res = go(ProvenanceFilter { source: bytes("doc/1"), ..Default::default() });
    assert_eq!(shape(&res), vec![(p[1], vec![e[0]]), (p[2], vec![e[0], e[4]])]);
    let res = go(ProvenanceFilter { source: bytes("wiki"), ..Default::default() });
    assert!(res.hits.is_empty(), "wiki is the origin of e2's provenance, not a source");
    // Actor and source together, and with a kind: all of them must hold.
    let res = go(ProvenanceFilter { actor: bytes("bot"), source: bytes("job/1"), ..Default::default() });
    assert_eq!(shape(&res), vec![(p[3], vec![e[2]])]);
    let res = go(ProvenanceFilter { actor: bytes("alice"), source: bytes("job/1"), ..Default::default() });
    assert!(res.hits.is_empty());
    let res =
        go(ProvenanceFilter { kinds: kinds(&[ProvenanceKind::Inferred]), actor: bytes("alice"), ..Default::default() });
    assert!(res.hits.is_empty());
    // The filter is applied at every hop: from p2, actor alice and source
    // doc/2 follow e6 only.
    let mut q = big(&[p[2]], 3);
    q.direction = AssocDirection::Out;
    q.provenance = ProvenanceFilter { actor: bytes("alice"), source: bytes("doc/2"), ..Default::default() };
    assert_eq!(shape(&run(&snap, &q)), vec![(p[5], vec![e[5]])]);
    // A source that no change set has.
    let res = go(ProvenanceFilter { source: bytes("nobody"), ..Default::default() });
    assert!(res.hits.is_empty() && res.ending == AssocEnding::Exhausted);

    // Window and provenance together (commits: e1..e7 at 2..=8).
    let mut q = big(&[p[0]], 3);
    q.direction = AssocDirection::Out;
    q.window = Some(SeqWindow { lo: CommitSeq(2), hi: CommitSeq(5) });
    q.provenance = ProvenanceFilter { actor: bytes("bot"), ..Default::default() };
    assert_eq!(shape(&run(&snap, &q)), vec![(p[3], vec![e[2]]), (p[4], vec![e[3]])]);
    q.window = Some(SeqWindow { lo: CommitSeq(5), hi: CommitSeq(8) });
    assert_eq!(shape(&run(&snap, &q)), vec![(p[4], vec![e[3]])]);
}

// ---------------------------------------------------------------- endings

fn the_three_endings_are_distinct<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let snap = g.db.snapshot().unwrap();
    let out = |depth, limit, budget| {
        let mut q = AssocQuery::new(vec![u[0]], depth, limit, budget);
        q.direction = AssocDirection::Out;
        run(&snap, &q)
    };

    // Nothing left: five hits exist and a limit of exactly five does not
    // truncate them.
    let full = out(4, 5, 100);
    assert_eq!((full.hits.len(), full.ending, full.visited), (5, AssocEnding::Exhausted, 9));
    // The same hits b, c from a depth-1 query: nothing left, since the depth
    // bound is part of the question.
    let res = out(1, 2, 100);
    assert_eq!((shape(&res), res.ending), (vec![(u[1], vec![r[0]]), (u[2], vec![r[1]])], AssocEnding::Exhausted));

    // Limit: one hit more than the limit exists. Two hits are returned; d,
    // the third, is found after them and proves there are more.
    let res = out(4, 2, 100);
    assert_eq!((shape(&res), res.ending), (vec![(u[1], vec![r[0]]), (u[2], vec![r[1]])], AssocEnding::LimitReached));
    assert_eq!(res.visited, 4, "stops at d, found by b's r3 after a's three entries");
    // A limit inside a level cuts it by unit id.
    let res = out(4, 1, 100);
    assert_eq!((shape(&res), res.ending), (vec![(u[1], vec![r[0]])], AssocEnding::LimitReached));
    // One below the full answer.
    let res = out(4, 4, 100);
    assert_eq!((res.hits.len(), res.ending), (4, AssocEnding::LimitReached));

    // Budget: the same two hits as the limit case, another ending. The third
    // entry of a (r8) is not examined.
    let res = out(4, 100, 2);
    assert_eq!(
        (shape(&res), res.ending, res.visited),
        (vec![(u[1], vec![r[0]]), (u[2], vec![r[1]])], AssocEnding::BudgetExhausted, 2)
    );
    // Runs out in the middle of a level: only r1 was examined, c is missing
    // from a level that holds b and c.
    let res = out(4, 100, 1);
    assert_eq!((shape(&res), res.ending, res.visited), (vec![(u[1], vec![r[0]])], AssocEnding::BudgetExhausted, 1));
    // A budget of exactly what the traversal needs is not exhausted; one less is.
    let res = out(4, 100, 9);
    assert_eq!((res.hits.len(), res.ending, res.visited), (5, AssocEnding::Exhausted, 9));
    let res = out(4, 100, 8);
    assert_eq!((res.hits.len(), res.ending, res.visited), (4, AssocEnding::BudgetExhausted, 8));
    // A budget that ends before any hit: empty, but not "nothing left".
    let mut q = AssocQuery::new(vec![u[6]], 4, 10, 1);
    assert_eq!(run(&snap, &q).ending, AssocEnding::Exhausted, "g has no relation: nothing left");
    q.seeds = vec![u[0]];
    q.relation_types = Some(["u".to_owned()].into());
    assert_eq!(run(&snap, &q).ending, AssocEnding::Exhausted, "a has no u relation: nothing left");
    q.relation_types = None;
    q.direction = AssocDirection::In;
    let res = run(&snap, &q);
    assert_eq!((res.hits.len(), res.ending), (1, AssocEnding::BudgetExhausted), "a ends r5 only: one entry, then d's");
    // Limit and budget meet: the limit is found before the budget runs out.
    let res = out(4, 2, 4);
    assert_eq!(res.ending, AssocEnding::LimitReached, "the fourth entry finds d");
    let res = out(4, 2, 3);
    assert_eq!(res.ending, AssocEnding::BudgetExhausted);
}

fn bounded_answers_agree_with_the_unbounded_one<E: TxnEngine>() {
    let g = diamond::<E>();
    let snap = g.db.snapshot().unwrap();
    for seed in [g.u[0], g.u[3], g.u[5]] {
        let full = run(&snap, &big(&[seed], 4));
        assert_eq!(full.ending, AssocEnding::Exhausted);
        // Limits: always a prefix of the full answer.
        for limit in 1..=full.hits.len() + 1 {
            let res = run(&snap, &AssocQuery::new(vec![seed], 4, limit, 1_000));
            let n = limit.min(full.hits.len());
            assert_eq!(res.hits, full.hits[..n], "seed {seed} limit {limit}");
            let truncated = limit < full.hits.len();
            assert_eq!(res.ending == AssocEnding::LimitReached, truncated, "seed {seed} limit {limit}");
            assert_eq!(res.ending == AssocEnding::Exhausted, !truncated, "seed {seed} limit {limit}");
        }
        // Budgets: every hit is a hit of the full answer with the same path,
        // in the same order, and the ending says whether it is the whole one.
        for budget in 1..=full.visited + 2 {
            let res = run(&snap, &AssocQuery::new(vec![seed], 4, 1_000, budget));
            let whole = budget >= full.visited;
            assert_eq!(res.ending, if whole { AssocEnding::Exhausted } else { AssocEnding::BudgetExhausted });
            assert_eq!(res.visited, budget.min(full.visited), "seed {seed} budget {budget}");
            for h in &res.hits {
                assert!(full.hits.contains(h), "seed {seed} budget {budget}: {h:?} is not in the full answer");
            }
            let order: Vec<_> = res.hits.iter().map(|h| (h.distance, h.unit)).collect();
            assert!(order.windows(2).all(|w| w[0] < w[1]), "seed {seed} budget {budget}: {order:?}");
            if whole {
                assert_eq!(res.hits, full.hits);
            }
        }
    }
}

// ------------------------------------------------------------ determinism

/// Ids set by hand, so the same logical graph can be written in any order.
struct SteeredIds(AtomicU64);

impl UnitIdSource for SteeredIds {
    fn next_id(&self) -> UnitId {
        let mut id = [0u8; 16];
        id[8..].copy_from_slice(&self.0.load(Ordering::SeqCst).to_be_bytes());
        UnitId(id)
    }
}

/// The diamond graph with unit ids `1..=7` and relation ids `101..=109`,
/// written with the units created in `unit_order` and the relations in
/// `relation_order`, all relations in one change set (one `created_seq`).
fn steered_diamond<E: TxnEngine>(
    unit_order: &[usize],
    relation_order: &[usize],
) -> (UnitDb<E>, Vec<UnitId>, Vec<RelationId>) {
    let ids = Arc::new(SteeredIds(AtomicU64::new(0)));
    let db = open_with::<E>(ids.clone());
    let id = |n: u64| {
        let mut b = [0u8; 16];
        b[8..].copy_from_slice(&n.to_be_bytes());
        b
    };
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for &k in unit_order {
        ids.0.store(k as u64 + 1, Ordering::SeqCst);
        cs.create(new_unit(&[k as u8])).unwrap();
    }
    cs.commit().unwrap();
    let u: Vec<UnitId> = (0..7).map(|k| UnitId(id(k + 1))).collect();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for &k in relation_order {
        ids.0.store(101 + k as u64, Ordering::SeqCst);
        let (f, t, ty) = DIAMOND[k];
        cs.create_relation(u[f], u[t], ty, RelationProvenance::Declared).unwrap();
    }
    cs.commit().unwrap();
    let r = (0..9).map(|k| RelationId(id(101 + k))).collect();
    (db, u, r)
}

fn same_logical_graph_gives_byte_identical_results<E: TxnEngine>() {
    let orders: [([usize; 7], [usize; 9]); 4] = [
        ([0, 1, 2, 3, 4, 5, 6], [0, 1, 2, 3, 4, 5, 6, 7, 8]),
        ([6, 5, 4, 3, 2, 1, 0], [8, 7, 6, 5, 4, 3, 2, 1, 0]),
        ([3, 0, 6, 1, 5, 2, 4], [4, 8, 1, 6, 0, 3, 7, 2, 5]),
        ([0, 1, 2, 3, 4, 5, 6], [7, 6, 5, 4, 3, 2, 1, 0, 8]),
    ];
    let mut seen: Vec<Vec<String>> = Vec::new();
    for (unit_order, relation_order) in orders {
        let (db, u, r) = steered_diamond::<E>(&unit_order, &relation_order);
        let snap = db.snapshot().unwrap();
        let mut outputs = Vec::new();
        for (seeds, dir) in [
            (vec![u[0]], AssocDirection::Out),
            (vec![u[5]], AssocDirection::In),
            (vec![u[0]], AssocDirection::Both),
            (vec![u[3], u[1]], AssocDirection::Both),
        ] {
            let mut q = big(&seeds, 4);
            q.direction = dir;
            let res = run(&snap, &q);
            // The first run is also the hand-written vector of the Out query.
            if dir == AssocDirection::Out {
                let want: Shape = vec![
                    (u[1], vec![r[0]]),
                    (u[2], vec![r[1]]),
                    (u[3], vec![r[0], r[2]]),
                    (u[4], vec![r[0], r[2], r[5]]),
                    (u[5], vec![r[0], r[2], r[5], r[6]]),
                ];
                assert_eq!(shape(&res), want, "units {unit_order:?} relations {relation_order:?}");
            }
            outputs.push(format!("{res:?}"));
        }
        seen.push(outputs);
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "insertion order changed an answer");
}

fn repeated_runs_are_byte_identical<E: TxnEngine>() {
    let g = diamond::<E>();
    let snap = g.db.snapshot().unwrap();
    let q = big(&[g.u[1], g.u[4]], 4);
    let first = format!("{:?}", run(&snap, &q));
    for _ in 0..3 {
        assert_eq!(format!("{:?}", run(&snap, &q)), first);
        // A new snapshot of the same state, and a new clone of the query.
        let again = g.db.snapshot().unwrap();
        assert_eq!(format!("{:?}", run(&again, &q.clone())), first);
    }
}

fn a_snapshot_does_not_see_later_commits<E: TxnEngine>() {
    let g = diamond::<E>();
    let (u, r) = (&g.u, &g.r);
    let before = g.db.snapshot().unwrap();
    let mut q = big(&[u[0]], 4);
    q.direction = AssocDirection::Out;
    let old = run(&before, &q);

    // Later: g becomes reachable from f, a new unit hangs off a, and r1 and
    // r8 (both a->b) go away.
    let extra = units(&g.db, 1)[0];
    let r10 = relate(&g.db, (u[5], u[6]), "t");
    let r11 = relate(&g.db, (u[0], extra), "t");
    let mut cs = g.db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(r[0]).unwrap();
    cs.delete_relation(r[7]).unwrap();
    cs.commit().unwrap();
    let after = g.db.snapshot().unwrap();

    assert_eq!(run(&before, &q), old, "the old snapshot answers as it did");
    assert_eq!(shape(&old).len(), 5);
    let new = run(&after, &q);
    // b is now unreachable from a; c keeps d; the new unit is a hit.
    let want: Shape = vec![
        (u[2], vec![r[1]]),
        (u[3], vec![r[1], r[3]]),
        (extra, vec![r11]),
        (u[4], vec![r[1], r[3], r[5]]),
        (u[5], vec![r[1], r[3], r[5], r[6]]),
    ];
    let mut by_distance = want.clone();
    by_distance.sort_by_key(|(unit, path)| (path.len(), *unit));
    assert_eq!(shape(&new), by_distance);
    assert!(new.hits.iter().all(|h| h.unit != u[1] && h.unit != u[6]), "b is cut off; g is past depth 4");
    // r10 is there for the deeper query, only on the new snapshot.
    let mut deep = big(&[u[5]], 1);
    deep.direction = AssocDirection::Out;
    assert!(run(&before, &deep).hits.is_empty());
    assert_eq!(shape(&run(&after, &deep)), vec![(u[6], vec![r10])]);
}

// -------------------------------------------------- units that are not active

fn ends_that_are_not_active_are_absent<E: TxnEngine>() {
    let db = open::<E>();
    // x is the seed, y a good neighbour, z archived, d deleted, w behind both.
    let u = units(&db, 5);
    let (x, y, z, d, w) = (u[0], u[1], u[2], u[3], u[4]);
    let ry = relate(&db, (x, y), "t");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(z, None).unwrap();
    cs.delete(d, None).unwrap();
    cs.commit().unwrap();
    let missing = UnitId([0xEE; 16]);

    // The layer refuses to write a relation to these; forge the records, as
    // damage would leave them (the verifier reports it as UNIT-36).
    let forge = |edge: u8, from: UnitId, to: UnitId| -> Patch {
        let edge = RelationId([edge; 16]);
        let record = RelationRecord {
            from,
            to,
            relation_type: "t".into(),
            created_seq: CommitSeq(1),
            provenance: RelationProvenance::Declared,
        };
        vec![
            (Key::Relation { edge }.encode().unwrap(), Some(record.encode().unwrap())),
            (
                Key::RelationOut { unit: from, relation_type: "t".into(), edge }.encode().unwrap(),
                Some(Adjacency { other: to }.encode()),
            ),
            (
                Key::RelationIn { unit: to, relation_type: "t".into(), edge }.encode().unwrap(),
                Some(Adjacency { other: from }.encode()),
            ),
        ]
    };
    let mut patch = Vec::new();
    for (edge, from, to) in [(0xA1, x, z), (0xA2, x, d), (0xA3, x, missing), (0xA4, z, w), (0xA5, d, w)] {
        patch.extend(forge(edge, from, to));
    }
    put_all(db.engine(), patch);
    let snap = db.snapshot().unwrap();

    // The forged relations exist for the reference relation read.
    assert_eq!(snap.reference_relations_of(x, Direction::Out, None).unwrap().len(), 4);

    let mut q = big(&[x], 4);
    q.direction = AssocDirection::Both;
    let res = run(&snap, &q);
    // Only y is a hit: z, d and the missing unit are absent, and the traversal
    // does not go through them, so w stays out of reach.
    assert_eq!(shape(&res), vec![(y, vec![ry])]);
    assert_eq!(res.ending, AssocEnding::Exhausted);
    // x starts four relations (ry and three forged); y ends ry.
    assert_eq!(res.visited, 5, "an entry that leads to an absent unit is still examined");
    // From the other side: w is not reachable from x, and its neighbours are
    // not active, so w yields nothing either.
    assert!(matches!(
        snap.reference_assoc_query(&big(&[z], 2)).unwrap_err(),
        AssocError::SeedNotActive { unit } if unit == z
    ));
    assert!(matches!(
        snap.reference_assoc_query(&big(&[d, w], 2)).unwrap_err(),
        AssocError::SeedNotActive { unit } if unit == d
    ));
    let res = run(&snap, &big(&[w], 2));
    assert!(res.hits.is_empty() && res.ending == AssocEnding::Exhausted);
    assert_eq!(res.visited, 2, "w ends the two forged relations; z and d are absent");
}

fn the_reference_path_never_reads_the_adjacency_index<E: TxnEngine>() {
    let g = diamond::<E>();
    let snap = g.db.snapshot().unwrap();
    let mut q = big(&[g.u[0]], 4);
    q.direction = AssocDirection::Both;
    let before = run(&snap, &q);
    assert_eq!(before.hits.len(), 5);

    // Remove every adjacency entry: the index path now sees nothing.
    let mut patch = Vec::new();
    for rel in snap.reference_relations(|_| true).unwrap() {
        let t = rel.relation_type.clone();
        patch.push((
            Key::RelationOut { unit: rel.from, relation_type: t.clone(), edge: rel.id }.encode().unwrap(),
            None,
        ));
        patch.push((Key::RelationIn { unit: rel.to, relation_type: t, edge: rel.id }.encode().unwrap(), None));
    }
    put_all(g.db.engine(), patch);
    let damaged = g.db.snapshot().unwrap();
    assert!(damaged.relations_of(g.u[0], Direction::Out, None).unwrap().is_empty(), "the index is gone");
    assert_eq!(damaged.reference_relations_of(g.u[0], Direction::Out, None).unwrap().len(), 3);
    assert_eq!(run(&damaged, &q), before);
}

// ------------------------------------------------------- the frozen corpus

fn story_provenance(p: &StoryProvenance) -> RelationProvenance {
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
fn load_corpus<E: TxnEngine>(corpus: &assoc_eval::Corpus) -> (UnitDb<E>, Vec<UnitId>) {
    let db = open::<E>();
    let mut ids = Vec::with_capacity(corpus.units.len());
    let (mut next_unit, mut next_rel) = (0, 0);
    for change in &corpus.changes {
        let m = meta(Some(&change.actor), Some(&change.source));
        let mut cs = db.begin_changeset(m).expect("begin change set");
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
            cs.create_relation(from, to, &r.relation_type, story_provenance(&r.provenance)).expect("create relation");
            next_rel += 1;
        }
        let info = cs.commit().expect("commit change set");
        assert_eq!(info.commit_seq.0, change.step, "a step is a commit sequence");
    }
    (db, ids)
}

/// Whether a relation passes the type set, window and provenance of `q`, read
/// from the change set record. Written against `Relation` fields only.
fn admitted<E: Engine>(snap: &UnitSnapshot<E>, q: &AssocQuery, r: &Relation) -> bool {
    let by = snap.change_set(r.created_seq).unwrap().expect("the change set of a relation exists").provenance;
    q.relation_types.as_ref().is_none_or(|t| t.contains(&r.relation_type))
        && q.window.is_none_or(|w| w.lo <= r.created_seq && r.created_seq <= w.hi)
        && q.provenance.kinds.as_ref().is_none_or(|k| k.contains(&r.provenance.kind()))
        && q.provenance.actor.as_ref().is_none_or(|a| by.actor.as_ref() == Some(a))
        && q.provenance.source.as_ref().is_none_or(|s| by.source.as_ref() == Some(s))
}

/// The hop distance of every unit within `q.max_depth`, by a plain breadth-first
/// search over all relations read at once.
fn oracle<E: Engine>(snap: &UnitSnapshot<E>, q: &AssocQuery) -> BTreeMap<UnitId, u32> {
    let allowed: Vec<Relation> =
        snap.reference_relations(|_| true).unwrap().into_iter().filter(|r| admitted(snap, q, r)).collect();
    let mut dist: BTreeMap<UnitId, u32> = q.seeds.iter().map(|s| (*s, 0)).collect();
    let mut frontier: BTreeSet<UnitId> = q.seeds.iter().copied().collect();
    for d in 1..=q.max_depth {
        let mut next = BTreeSet::new();
        for r in &allowed {
            let out = matches!(q.direction, AssocDirection::Out | AssocDirection::Both);
            let inn = matches!(q.direction, AssocDirection::In | AssocDirection::Both);
            for (here, there, wanted) in [(r.from, r.to, out), (r.to, r.from, inn)] {
                if wanted && frontier.contains(&here) && !dist.contains_key(&there) {
                    next.insert(there);
                }
            }
        }
        for n in &next {
            dist.insert(*n, d);
        }
        frontier = next;
    }
    dist
}

/// Everything a result claims is true at the snapshot: each relation of each
/// path exists with the same fields, each hop connects to the next, the path
/// respects the query, and the revision is the head.
#[track_caller]
fn verify_result<E: Engine>(snap: &UnitSnapshot<E>, q: &AssocQuery, res: &AssocResult, name: &str) {
    let seeds: BTreeSet<UnitId> = q.seeds.iter().copied().collect();
    let order: Vec<_> = res.hits.iter().map(|h| (h.distance, h.unit)).collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{name}: hits not in (distance, unit) order or repeated");
    assert!(res.hits.len() <= q.limit && res.visited <= q.visit_budget, "{name}: a bound is broken");
    let out = matches!(q.direction, AssocDirection::Out | AssocDirection::Both);
    let inn = matches!(q.direction, AssocDirection::In | AssocDirection::Both);

    for hit in &res.hits {
        assert!(!seeds.contains(&hit.unit), "{name}: a seed is a hit");
        assert!(hit.distance >= 1 && hit.distance <= q.max_depth, "{name}: distance {}", hit.distance);
        assert_eq!(hit.path.len(), hit.distance as usize, "{name}: path length is the distance");
        let unit = snap.get_unit(hit.unit).unwrap().expect("hit exists");
        assert_eq!(unit.lifecycle, Lifecycle::Active, "{name}: hit is active");
        assert_eq!(hit.revision, unit.head, "{name}: revision is the head");

        // The relations exist, unchanged.
        for s in &hit.path {
            let stored = snap.get_relation(s.relation).unwrap().unwrap_or_else(|| panic!("{name}: relation missing"));
            assert_eq!((stored.from, stored.to, &stored.relation_type), (s.from, s.to, &s.relation_type), "{name}");
            assert_eq!((stored.created_seq, stored.provenance.kind()), (s.created_seq, s.provenance), "{name}");
            assert!(admitted(snap, q, &stored), "{name}: a relation of the path is excluded by the query");
        }
        // The hops connect: some seed, then each relation leaves the unit the
        // last one arrived at (in a followed direction) and arrives at the next.
        let connects = |start: UnitId| -> bool {
            let mut here = start;
            for s in &hit.path {
                let next = if out && s.from == here {
                    s.to
                } else if inn && s.to == here {
                    s.from
                } else {
                    return false;
                };
                here = next;
            }
            here == hit.unit
        };
        let first = &hit.path[0];
        let connected = [first.from, first.to].into_iter().filter(|s| seeds.contains(s)).any(connects);
        assert!(connected, "{name}: the path of {} does not connect a seed to it", hit.unit);
        // A shortest path never follows a relation twice.
        let relations: BTreeSet<_> = hit.path.iter().map(|s| s.relation).collect();
        assert_eq!(relations.len(), hit.path.len(), "{name}: a relation is followed twice");
    }
}

fn corpus_query(ids: &[UnitId], q: &assoc_eval::Query) -> AssocQuery {
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

fn frozen_corpus_traces_are_verifiable<E: TxnEngine>() {
    let corpus = assoc_eval::parse_corpus(CORPUS).expect("corpus parses");
    let set = assoc_eval::parse_queries(QUERIES).expect("queries parse");
    let (db, ids) = load_corpus::<E>(&corpus);
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.seq().0, 300);
    assert_eq!(set.queries.len(), 64);

    let mut with_hits = 0;
    let mut multi_hop = 0;
    for q in &set.queries {
        let query = corpus_query(&ids, q);
        let res = run(&snap, &query);
        verify_result(&snap, &query, &res, &q.id);
        assert_eq!(res.ending, AssocEnding::Exhausted, "{}: the bounds of the set are far below the hard maxima", q.id);

        // The hits and their distances are the plain breadth-first answer.
        let want: BTreeMap<UnitId, u32> = oracle(&snap, &query).into_iter().filter(|(_, d)| *d > 0).collect();
        let got: BTreeMap<UnitId, u32> = res.hits.iter().map(|h| (h.unit, h.distance)).collect();
        assert_eq!(got, want, "{}: hits differ from the oracle", q.id);

        // Every expected answer of the set is a hit (the set checks that they
        // are reachable; here the reference path agrees).
        for j in &q.expected {
            assert!(got.contains_key(&ids[j.index as usize]), "{}: expected unit {} is not a hit", q.id, j.index);
        }
        with_hits += usize::from(!res.hits.is_empty());
        multi_hop += usize::from(res.hits.iter().any(|h| h.distance >= 2));
    }
    assert_eq!(with_hits, 64, "every query reaches something");
    // Counted on the frozen set: 56 of its 64 queries reach distance 2 or more.
    assert_eq!(multi_hop, 56, "the set exercises several hops");

    // Several seeds, every relation, both directions, the hard maxima: the
    // component is larger than the result limit, so the answer is cut, and it
    // is the first `limit` of the breadth-first answer in (distance, unit) order.
    let seeds: Vec<UnitId> = [4, 24, 44].iter().map(|i| ids[*i]).collect();
    let query = big(&seeds, MAX_ASSOC_DEPTH);
    let res = run(&snap, &query);
    verify_result(&snap, &query, &res, "multi-seed");
    let mut want: Vec<(u32, UnitId)> =
        oracle(&snap, &query).into_iter().filter(|(_, d)| *d > 0).map(|(unit, d)| (d, unit)).collect();
    want.sort();
    assert!(want.len() > MAX_ASSOC_LIMIT, "the test needs a component larger than the limit");
    assert_eq!(res.ending, AssocEnding::LimitReached);
    let got: Vec<(u32, UnitId)> = res.hits.iter().map(|h| (h.distance, h.unit)).collect();
    assert_eq!(got, want[..MAX_ASSOC_LIMIT]);
}

macro_rules! engine_tests {
    ($m:ident, $E:ty) => {
        mod $m {
            use super::*;
            type E = $E;
            #[test]
            fn validation_rejects_every_unbounded_or_malformed_query() {
                super::validation_rejects_every_unbounded_or_malformed_query::<E>();
            }
            #[test]
            fn hard_maxima_are_public_and_accepted() {
                super::hard_maxima_are_public_and_accepted::<E>();
            }
            #[test]
            fn seeds_must_be_active_units() {
                super::seeds_must_be_active_units::<E>();
            }
            #[test]
            fn seeds_are_not_results_and_order_does_not_depend_on_them() {
                super::seeds_are_not_results_and_order_does_not_depend_on_them::<E>();
            }
            #[test]
            fn out_traversal_vector() {
                super::out_traversal_vector::<E>();
            }
            #[test]
            fn in_traversal_vector() {
                super::in_traversal_vector::<E>();
            }
            #[test]
            fn both_traversal_vector_with_cycle_and_self_relation() {
                super::both_traversal_vector_with_cycle_and_self_relation::<E>();
            }
            #[test]
            fn trace_carries_the_fields_of_each_relation_and_the_head_revision() {
                super::trace_carries_the_fields_of_each_relation_and_the_head_revision::<E>();
            }
            #[test]
            fn relation_type_filter() {
                super::relation_type_filter::<E>();
            }
            #[test]
            fn window_is_inclusive_and_applies_to_each_relation_followed() {
                super::window_is_inclusive_and_applies_to_each_relation_followed::<E>();
            }
            #[test]
            fn provenance_filters_by_kind_actor_and_source() {
                super::provenance_filters_by_kind_actor_and_source::<E>();
            }
            #[test]
            fn the_three_endings_are_distinct() {
                super::the_three_endings_are_distinct::<E>();
            }
            #[test]
            fn bounded_answers_agree_with_the_unbounded_one() {
                super::bounded_answers_agree_with_the_unbounded_one::<E>();
            }
            #[test]
            fn same_logical_graph_gives_byte_identical_results() {
                super::same_logical_graph_gives_byte_identical_results::<E>();
            }
            #[test]
            fn repeated_runs_are_byte_identical() {
                super::repeated_runs_are_byte_identical::<E>();
            }
            #[test]
            fn a_snapshot_does_not_see_later_commits() {
                super::a_snapshot_does_not_see_later_commits::<E>();
            }
            #[test]
            fn ends_that_are_not_active_are_absent() {
                super::ends_that_are_not_active_are_absent::<E>();
            }
            #[test]
            fn the_reference_path_never_reads_the_adjacency_index() {
                super::the_reference_path_never_reads_the_adjacency_index::<E>();
            }
            #[test]
            fn frozen_corpus_traces_are_verifiable() {
                super::frozen_corpus_traces_are_verifiable::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, EmuLog);
engine_tests!(kerno, KernoDb);
