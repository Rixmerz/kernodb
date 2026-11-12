//! E5-04: sentinels (etapa5-acta.md §3, ADR-0015, I-37, I-29, IU-10).
//!
//! A sentinel is a string no layer of Kerno would ever write by itself. One is
//! planted in every place a user's content can enter the layer — unit type,
//! payload, metadata, relation type, provenance source and origin, and the
//! values of both filters — and then searched for in everything the layer
//! reports about a query: the observer's records and counters, the plan
//! explanations, the fingerprints, and the errors of queries and writes. This
//! is done at every observation level.
//!
//! The acta asks for no sentinel outside `diagnostic`. The shape text that
//! `diagnostic` adds replaces each literal with a typed placeholder, so the
//! test asks for more: no sentinel at any level. A change that makes
//! `diagnostic` carry a literal on purpose must relax that single assertion,
//! and it must not touch the levels below.
//!
//! * `no_sentinel_in_any_observability_explanation_or_error_output` — the
//!   search itself. It first proves that it can find what it looks for: the
//!   answer handed to the caller does hold the sentinels. A literal hashed
//!   into the fingerprint never shows as text, so it also checks that every
//!   fingerprint reported equals that of the same shape with other literals.
//! * `levels_below_diagnostic_carry_no_shape_text` — the structural half of
//!   I-37: `fingerprint_text` exists at `diagnostic` only, and it is the
//!   placeholder shape.
//! * `no_sentinel_in_any_associative_plan_observation_or_error` (E7-02) — the
//!   same search for associative queries: the sentinels are the relation
//!   types, the actor and the source of the change set, the bounds of the
//!   window and of the depth, and the seeds' identifiers. The plan and the
//!   observations carry none of them at any level; the errors a caller gets
//!   carry none of the content (a rejected query quotes its own bounds back to
//!   its caller, so those numbers are searched in the observer only).

mod harness;

use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::Key;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

const S_TYPE: &str = "SNT-TYPE-7f3a91c2";
const S_PAYLOAD: &[u8] = b"SNT-PAYLOAD-5be0d447";
const S_METADATA: &[u8] = b"SNT-METADATA-c9a1e6f0";
const S_REL_TYPE: &str = "SNT-RELTYPE-28d4b7a5";
const S_SOURCE: &[u8] = b"SNT-SOURCE-61f3a0cc";
const S_ORIGIN: &[u8] = b"SNT-ORIGIN-e4079b3d";
const S_OVERSIZE: &str = "SNT-OVERSIZE-0d6c52e8-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
/// The value of the schema version filter, as a number a text search can find.
const S_VERSION: u32 = 4_071_337_791;
/// The actor and the source of the change set that creates the data (E7-02: the
/// filters of an associative query compare against them).
const S_ACTOR: &[u8] = b"SNT-ACTOR-3b9d70e1";
const S_CS_SOURCE: &[u8] = b"SNT-CSSOURCE-a85c14f6";
/// An upper bound of a window, as a number a text search can find.
const S_WINDOW: u64 = 4_071_337_792;
/// A depth far over the hard maximum: a rejected query that quotes it.
const S_DEPTH: u32 = 4_071_337_795;

/// Each sentinel in every form a `Debug` or `Display` of it could take: the
/// text, the decimal list of a byte slice (`{:?}` of a `Vec<u8>`) and hex.
fn forms(bytes: &[u8]) -> Vec<String> {
    let list = bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    vec![String::from_utf8(bytes.to_vec()).unwrap(), list, hex]
}

fn sentinels() -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("unit type", forms(S_TYPE.as_bytes())),
        ("payload", forms(S_PAYLOAD)),
        ("metadata", forms(S_METADATA)),
        ("relation type", forms(S_REL_TYPE.as_bytes())),
        ("provenance source", forms(S_SOURCE)),
        ("provenance origin", forms(S_ORIGIN)),
        ("oversize type", forms(S_OVERSIZE.as_bytes())),
        ("schema version", vec![S_VERSION.to_string()]),
        ("change set actor", forms(S_ACTOR)),
        ("change set source", forms(S_CS_SOURCE)),
    ]
}

fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: Target>() -> UnitDb<E> {
    open_units::<E>(SimVfs::new(1).as_vfs(), &config()).expect("open")
}

const LEVELS: [ObservationLevel; 4] =
    [ObservationLevel::Off, ObservationLevel::Aggregate, ObservationLevel::Fingerprint, ObservationLevel::Diagnostic];

/// Two units that carry every sentinel, related by three relations whose type
/// and provenance carry the rest. Returns their ids.
fn plant<E: Target>(db: &UnitDb<E>) -> [UnitId; 2] {
    let meta =
        ChangeMeta { actor: Some(S_ACTOR.to_vec()), source: Some(S_CS_SOURCE.to_vec()), ..ChangeMeta::default() };
    let mut cs = db.begin_changeset(meta).unwrap();
    let new = |schema| NewUnit {
        unit_type: S_TYPE.into(),
        schema_version: schema,
        metadata: S_METADATA.to_vec(),
        payload: S_PAYLOAD.to_vec(),
    };
    let a = cs.create(new(S_VERSION)).unwrap();
    let b = cs.create(new(1)).unwrap();
    cs.create_relation(a, b, S_REL_TYPE, RelationProvenance::Imported { source: S_SOURCE.to_vec() }).unwrap();
    cs.create_relation(b, a, S_REL_TYPE, RelationProvenance::Algorithmic { origin: S_ORIGIN.to_vec() }).unwrap();
    cs.create_relation(
        a,
        b,
        S_REL_TYPE,
        RelationProvenance::Inferred { origin: S_ORIGIN.to_vec(), confidence_permille: 500 },
    )
    .unwrap();
    cs.commit().unwrap();
    [a, b]
}

/// Every query shape, with the sentinels as literals.
fn filters() -> Vec<UnitFilter> {
    vec![
        UnitFilter { unit_type: Some(S_TYPE.into()), schema_version: None },
        UnitFilter { unit_type: None, schema_version: Some(S_VERSION) },
        UnitFilter { unit_type: Some(S_TYPE.into()), schema_version: Some(S_VERSION) },
    ]
}

fn relation_queries(unit: UnitId) -> Vec<RelationQuery> {
    [Direction::Out, Direction::In]
        .into_iter()
        .map(|direction| RelationQuery { unit, direction, relation_type: Some(S_REL_TYPE.into()) })
        .collect()
}

/// The fingerprints of the same shapes with other literals (another type,
/// version and unit). A literal that reaches the hash input, and so never
/// shows as text, makes a fingerprint differ from these.
fn literal_free_fingerprints(key: &FingerprintKey, ids: [UnitId; 2]) -> Vec<Fingerprint> {
    let units = filters().into_iter().map(|f| {
        NormalizedQuery::of_filter(&UnitFilter {
            unit_type: f.unit_type.map(|_| "other".into()),
            schema_version: f.schema_version.map(|_| 1),
        })
    });
    let relations = relation_queries(ids[1]).into_iter().map(|q| {
        NormalizedQuery::of_relations(&RelationQuery { relation_type: q.relation_type.map(|_| "other".into()), ..q })
    });
    let assoc = assoc_queries(ids).into_iter().chain(rejected_assoc_queries(ids));
    let assoc = assoc.map(|q| NormalizedQuery::of_assoc(&literal_free(&q)));
    units.chain(relations).chain(assoc).map(|q| q.fingerprint(key)).collect()
}

/// Every fingerprint the layer reported, in plans and records, is one of a
/// literal-free shape.
fn assert_shape_only(reported: impl IntoIterator<Item = Fingerprint>, shapes: &[Fingerprint], level: ObservationLevel) {
    for fp in reported {
        assert!(shapes.contains(&fp), "fingerprint {fp} at level {level:?} depends on a literal");
    }
}

/// Truncates the head of `unit` in the engine, so reading it is corruption.
fn damage_head<E: Engine>(engine: &E, unit: UnitId) {
    let head = Key::UnitHead { unit }.encode().unwrap();
    let good = engine.snapshot().unwrap().get(&head).unwrap().unwrap();
    let mut batch = WriteBatch::new();
    batch.put(head, good[..good.len() - 1].to_vec());
    engine.commit(&batch).expect("raw commit");
}

/// Everything the layer says about the queries of one level, as text.
fn observed_output<E: Target>(db: &UnitDb<E>, level: ObservationLevel, ids: [UnitId; 2]) -> String {
    db.observer().set_level(level).unwrap();
    db.observer().set_key(FingerprintKey::from_bytes([7; 32]));
    let key = FingerprintKey::from_bytes([7; 32]);
    let snap = db.snapshot().unwrap();
    let mut out = String::new();
    let mut say = |what: &str, text: String| {
        out.push_str(&format!("[{what}] {text}\n"));
    };
    let mut reported = Vec::new();

    for f in filters() {
        let plan = snap.explain_filter(&key, &f).unwrap();
        reported.push(*plan.fingerprint());
        say("explain_filter", format!("{plan:?} {:?} {}", plan.fingerprint(), plan.fingerprint()));
        let outcome = snap.query(&f).unwrap();
        say("query path", format!("{:?} {:?} {:?}", outcome.source, outcome.path, outcome.fallback));
    }
    for q in relation_queries(ids[0]) {
        let plan = snap.explain_relations(&key, &q).unwrap();
        reported.push(*plan.fingerprint());
        say("explain_relations", format!("{plan:?} {:?} {}", plan.fingerprint(), plan.fingerprint()));
        let outcome = snap.query_relations(&q).unwrap();
        say("relations path", format!("{:?} {:?}", outcome.path, outcome.fallback));
    }

    // Errors: a write that is refused (the query errors are in `observed_output_after_damage`).
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let refused = cs.create(NewUnit {
        unit_type: S_OVERSIZE.into(),
        schema_version: S_VERSION,
        metadata: S_METADATA.to_vec(),
        payload: S_PAYLOAD.to_vec(),
    });
    match refused {
        Err(e) => say("write error", format!("{e} | {e:?}")),
        Ok(_) => {
            if let Err(e) = cs.commit() {
                say("write error", format!("{e} | {e:?}"));
            }
        }
    }

    say("stats", format!("{:?}", db.observer().stats()));
    for r in db.observer().records() {
        reported.push(r.fingerprint);
        say("record", format!("{r:#?}"));
    }
    assert_shape_only(reported, &literal_free_fingerprints(&key, ids), level);
    out
}

fn no_sentinel_in_any_observability_explanation_or_error_output<E: Target>() {
    let db = open::<E>();
    let ids = plant(&db);
    let wanted = sentinels();

    // The search can find what it looks for: the caller's own answer holds them.
    let snap = db.snapshot().unwrap();
    let answer = format!(
        "{:?} {:?}",
        snap.query(&filters()[2]).unwrap(),
        snap.query_relations(&relation_queries(ids[0])[0]).unwrap()
    );
    let not_in_these_answers = ["oversize type", "provenance origin", "change set actor", "change set source"];
    for (what, needles) in wanted.iter().filter(|(w, _)| !not_in_these_answers.contains(w)) {
        assert!(
            needles.iter().any(|n| answer.contains(n.as_str())),
            "control: the answer to the caller must hold the {what} sentinel"
        );
    }
    // The change set holds the actor and the source: the filters of the
    // associative queries compare against them, so they must exist to be found.
    let recorded = format!("{:?}", snap.change_set(CommitSeq(1)).unwrap());
    for (what, needles) in wanted.iter().filter(|(w, _)| w.starts_with("change set")) {
        assert!(needles.iter().any(|n| recorded.contains(n.as_str())), "control: the change set must hold the {what}");
    }
    drop(snap);

    // A ready index, so the explanations hold the index candidates as usable.
    db.rebuild_index().unwrap();
    let mut outputs = Vec::new();
    for level in LEVELS {
        outputs.push((level, observed_output(&db, level, ids)));
    }
    // Damage the head of the second unit: queries and the explanation of the
    // damaged state now fail or fall back.
    damage_head(db.engine(), ids[1]);
    for level in LEVELS {
        outputs.push((level, observed_output_after_damage(&db, level, ids)));
    }

    for (level, text) in &outputs {
        for (what, needles) in &wanted {
            for n in needles {
                assert!(!text.contains(n.as_str()), "{what} sentinel {n:?} leaked at level {level:?}:\n{text}");
            }
        }
    }

    // The search is not vacuous: observation levels recorded, and errors were reported.
    let at = |l: ObservationLevel| outputs.iter().find(|(lv, _)| *lv == l).unwrap().1.clone();
    assert!(at(ObservationLevel::Off).contains("explain_filter"));
    assert!(at(ObservationLevel::Fingerprint).contains("[record]"), "fingerprint level records");
    assert!(at(ObservationLevel::Diagnostic).contains("units[unit_type=?text"), "diagnostic shows the shape");
    assert!(at(ObservationLevel::Off).contains("[write error]"), "the oversize type must be refused and searched");
    assert!(
        outputs.iter().any(|(_, o)| o.contains("[query error]")),
        "the damaged head must surface as a query error that this test searched"
    );
}

/// `observed_output` with every query allowed to fail: the damaged head makes
/// the reference path fail, and the error is the output under search.
fn observed_output_after_damage<E: Target>(db: &UnitDb<E>, level: ObservationLevel, ids: [UnitId; 2]) -> String {
    db.observer().set_level(level).unwrap();
    db.observer().set_key(FingerprintKey::from_bytes([7; 32]));
    let key = FingerprintKey::from_bytes([7; 32]);
    let snap = db.snapshot().unwrap();
    let mut out = String::new();
    for f in filters() {
        match snap.explain_filter(&key, &f) {
            Ok(plan) => out.push_str(&format!("[explain_filter] {plan:?}\n")),
            Err(e) => out.push_str(&format!("[explain error] {e} | {e:?}\n")),
        }
        if let Err(e) = snap.query(&f) {
            out.push_str(&format!("[query error] {e} | {e:?}\n"));
        }
    }
    for q in relation_queries(ids[0]) {
        match snap.query_relations(&q) {
            Ok(o) => out.push_str(&format!("[relations] {:?} {:?}\n", o.path, o.fallback)),
            Err(e) => out.push_str(&format!("[relations error] {e} | {e:?}\n")),
        }
    }
    for q in assoc_queries(ids) {
        match snap.assoc_query(&q) {
            Ok(o) => out.push_str(&format!("[assoc] {:?} {:?}\n", o.path, o.fallback)),
            Err(e) => out.push_str(&format!("[assoc error] {e} | {e:?}\n")),
        }
    }
    if let Err(e) = db.rebuild_index() {
        out.push_str(&format!("[rebuild error] {e} | {e:?}\n"));
    }
    out.push_str(&format!("[stats] {:?}\n", db.observer().stats()));
    let records = db.observer().records();
    for r in &records {
        out.push_str(&format!("[record] {r:#?}\n"));
    }
    assert_shape_only(records.iter().map(|r| r.fingerprint), &literal_free_fingerprints(&key, ids), level);
    out
}

/// Associative queries with the sentinels as literals: every field that holds
/// one is set.
fn assoc_queries(ids: [UnitId; 2]) -> Vec<AssocQuery> {
    let mut all = AssocQuery::new(vec![ids[0], ids[1]], 2, 10, 100);
    all.direction = AssocDirection::Out;
    all.relation_types = Some([S_REL_TYPE.to_owned(), S_OVERSIZE.to_owned()].into());
    all.window = Some(SeqWindow { lo: CommitSeq(1), hi: CommitSeq(S_WINDOW) });
    all.provenance.kinds = Some([ProvenanceKind::Imported, ProvenanceKind::Inferred].into());
    all.provenance.actor = Some(S_ACTOR.to_vec());
    all.provenance.source = Some(S_CS_SOURCE.to_vec());
    let mut inwards = all.clone();
    inwards.direction = AssocDirection::In;
    inwards.provenance.source = Some(S_SOURCE.to_vec());
    let mut plain = AssocQuery::new(vec![ids[0]], 3, 10, 100);
    plain.relation_types = Some([S_REL_TYPE.to_owned()].into());
    vec![all, inwards, plain]
}

/// Queries the model rejects, carrying sentinel numbers, and one that starts at
/// a unit that is not there. Their errors quote the caller's own bounds back to
/// the caller; the observer must not.
fn rejected_assoc_queries(ids: [UnitId; 2]) -> Vec<AssocQuery> {
    let mut inverted = AssocQuery::new(vec![ids[0]], 1, 1, 1);
    inverted.window = Some(SeqWindow { lo: CommitSeq(S_WINDOW + 1), hi: CommitSeq(S_WINDOW) });
    inverted.relation_types = Some([S_REL_TYPE.to_owned()].into());
    inverted.provenance.actor = Some(S_ACTOR.to_vec());
    let deep = AssocQuery::new(vec![ids[0]], S_DEPTH, 1, 1);
    let mut ghost = AssocQuery::new(vec![UnitId([0xEE; 16])], 1, 1, 1);
    ghost.provenance.source = Some(S_CS_SOURCE.to_vec());
    vec![inverted, deep, ghost]
}

/// Everything the layer says about the associative queries of one level, as
/// text: `(the plans, outcomes and observations, the errors)`.
fn assoc_output<E: Target>(db: &UnitDb<E>, level: ObservationLevel, ids: [UnitId; 2]) -> (String, String) {
    db.observer().clear();
    db.observer().set_level(level).unwrap();
    db.observer().set_key(FingerprintKey::from_bytes([7; 32]));
    let key = FingerprintKey::from_bytes([7; 32]);
    let snap = db.snapshot().unwrap();
    let (mut out, mut errors) = (String::new(), String::new());
    let mut shapes = Vec::new();
    for q in assoc_queries(ids) {
        let plan = snap.explain_assoc(&key, &q).unwrap();
        shapes.push(*plan.fingerprint());
        out.push_str(&format!("[explain_assoc] {plan:?} {:?} {}\n", plan.fingerprint(), plan.fingerprint()));
        let o = snap.assoc_query(&q).unwrap();
        out.push_str(&format!("[assoc path] {:?} {:?}\n", o.path, o.fallback));
    }
    for q in rejected_assoc_queries(ids) {
        match snap.assoc_query(&q) {
            Ok(_) => panic!("{q:?} must be rejected"),
            Err(e) => errors.push_str(&format!("[assoc error] {e} | {e:?}\n")),
        }
    }
    out.push_str(&format!("[stats] {:?}\n", db.observer().stats()));
    let records = db.observer().records();
    for r in &records {
        shapes.push(r.fingerprint);
        out.push_str(&format!("[record] {r:#?}\n"));
    }
    // Every fingerprint is one of the literal-free shapes of these queries.
    assert_shape_only(shapes, &literal_free_fingerprints(&key, ids), level);
    (out, errors)
}

/// The same query with other literals: another seed, type set, bounds, kinds,
/// actor and source (the kind set keeps its structure: set or not).
fn literal_free(q: &AssocQuery) -> AssocQuery {
    let mut other = AssocQuery::new(vec![UnitId([9; 16])], 1, 1, 1);
    other.direction = q.direction;
    other.relation_types = q.relation_types.as_ref().map(|_| ["other".to_owned()].into());
    other.window = q.window.map(|_| SeqWindow { lo: CommitSeq(1), hi: CommitSeq(2) });
    other.provenance.kinds = q.provenance.kinds.as_ref().map(|_| [ProvenanceKind::Declared].into());
    other.provenance.actor = q.provenance.actor.as_ref().map(|_| b"other".to_vec());
    other.provenance.source = q.provenance.source.as_ref().map(|_| b"other".to_vec());
    other
}

/// Identifiers in the forms a `Debug` or `Display` of them takes.
fn id_forms(id: UnitId) -> Vec<String> {
    let list = id.0.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    vec![id.to_string(), list]
}

fn no_sentinel_in_any_associative_plan_observation_or_error<E: Target>() {
    let db = open::<E>();
    let ids = plant(&db);
    let content = sentinels();
    let window = ("window bound", vec![S_WINDOW.to_string(), (S_WINDOW + 1).to_string()]);
    let depth = ("depth", vec![S_DEPTH.to_string()]);
    let identifiers =
        [("seed", id_forms(ids[0])), ("seed", id_forms(ids[1])), ("absent seed", id_forms(UnitId([0xEE; 16])))];

    // The search can find what it looks for: the answer to the caller holds
    // the relation type; the errors hold the caller's bounds and the absent seed.
    let snap = db.snapshot().unwrap();
    let answer = format!("{:?}", snap.assoc_query(&assoc_queries(ids)[2]).unwrap().result);
    assert!(answer.contains(S_REL_TYPE), "control: the answer must hold the relation type: {answer}");
    drop(snap);

    db.rebuild_index().unwrap();
    let mut outputs = Vec::new();
    for level in LEVELS {
        outputs.push((level, assoc_output(&db, level, ids)));
    }
    for (level, (out, errors)) in &outputs {
        for (what, needles) in content.iter().chain([&window, &depth]).chain(identifiers.iter()) {
            for n in needles {
                assert!(!out.contains(n.as_str()), "{what} sentinel {n:?} leaked at level {level:?}:\n{out}");
            }
        }
        for (what, needles) in &content {
            for n in needles {
                assert!(
                    !errors.contains(n.as_str()),
                    "{what} sentinel {n:?} leaked in an error at {level:?}:\n{errors}"
                );
            }
        }
    }
    // The search is not vacuous: the errors do quote the caller's own bounds.
    let (_, errors) = &outputs[0].1;
    assert!(
        errors.contains(&S_WINDOW.to_string()),
        "control: the rejection quotes the bounds to its caller:\n{errors}"
    );
    assert!(errors.contains(&ids[0].to_string()) || errors.contains(&UnitId([0xEE; 16]).to_string()), "{errors}");
    let at = |l: ObservationLevel| &outputs.iter().find(|(lv, _)| *lv == l).unwrap().1.0;
    assert!(at(ObservationLevel::Off).contains("[explain_assoc]"));
    assert!(!at(ObservationLevel::Off).contains("[record]"), "off keeps nothing");
    assert!(at(ObservationLevel::Fingerprint).contains("[record]"));
    assert!(at(ObservationLevel::Diagnostic).contains("assoc[seeds=?unit_ids,dir=out"), "diagnostic shows the shape");
    // Each rejected query was observed as a class.
    assert!(at(ObservationLevel::Fingerprint).contains("Error(\n"), "rejected queries are recorded");
}

fn levels_below_diagnostic_carry_no_shape_text<E: Target>() {
    let db = open::<E>();
    let ids = plant(&db);
    for level in LEVELS {
        db.observer().clear();
        db.observer().set_level(level).unwrap();
        db.observer().set_key(FingerprintKey::from_bytes([7; 32]));
        let snap = db.snapshot().unwrap();
        snap.query(&filters()[2]).unwrap();
        snap.query_relations(&relation_queries(ids[0])[0]).unwrap();
        snap.assoc_query(&assoc_queries(ids)[0]).unwrap();
        let records = db.observer().records();
        match level {
            ObservationLevel::Off | ObservationLevel::Aggregate => assert!(records.is_empty(), "{level:?}"),
            ObservationLevel::Fingerprint => {
                assert_eq!(records.len(), 3);
                assert!(records.iter().all(|r| r.fingerprint_text.is_none()), "no shape text at {level:?}");
            }
            ObservationLevel::Diagnostic => {
                let texts: Vec<_> = records.iter().map(|r| r.fingerprint_text.as_deref()).collect();
                assert_eq!(
                    texts,
                    [
                        Some("units[unit_type=?text,schema_version=?u32]"),
                        Some("relations[out:unit=?unit_id,relation_type=?text]"),
                        Some(
                            "assoc[seeds=?unit_ids,dir=out,depth=?u32,limit=?u32,budget=?u64,types=?texts,window=?seqs,kinds=?kinds,actor=?bytes,source=?bytes]"
                        )
                    ]
                );
            }
        }
    }
}

// ---------------------------------------------------------------- E8-04

const S_VEC_PROFILE: &str = "SNT-PROFILE-9d41b6e3";
const S_VEC_METHOD: &str = "SNT-METHOD-70ac2f58";
const S_VEC_PARAM: &str = "SNT-PARAM-b3e81c07";
/// A vector element, as a number a text search can find in any of its forms.
const S_VEC_ELEM: f32 = 4_071_337.5;

fn vector_forms() -> Vec<String> {
    let bits = S_VEC_ELEM.to_bits();
    let mut out = vec![
        format!("{S_VEC_ELEM}"),
        format!("{S_VEC_ELEM:?}"),
        format!("{S_VEC_ELEM:e}"),
        bits.to_string(),
        format!("{bits:08x}"),
        "4071337".to_string(),
    ];
    let be = bits.to_be_bytes();
    out.push(be.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", "));
    out
}

fn vector_sentinels() -> Vec<(&'static str, Vec<String>)> {
    let mut all: Vec<_> =
        sentinels().into_iter().filter(|(n, _)| matches!(*n, "payload" | "metadata" | "unit type")).collect();
    all.push(("vector element", vector_forms()));
    all.push(("vector profile", forms(S_VEC_PROFILE.as_bytes())));
    all.push(("vector method", forms(S_VEC_METHOD.as_bytes())));
    all.push(("vector param", forms(S_VEC_PARAM.as_bytes())));
    all
}

/// E8-04 (ADR-0020, T24, I-37): the values of a vector and the contents of the
/// units it was made from appear in no error, no `Debug` or `Display` of the
/// new types, no report of opening and no finding of `verify_units`, for
/// healthy data and for every damage the records can suffer.
fn no_sentinel_in_vector_types_errors_or_reports<E: Target>() {
    let disk = SimVfs::new(5);
    let db = open_units::<E>(disk.as_vfs(), &config()).unwrap();
    let (dd, _) = DerivedDb::open_persisted(db).unwrap();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let u = cs
        .create(NewUnit {
            unit_type: S_TYPE.into(),
            schema_version: 1,
            metadata: S_METADATA.to_vec(),
            payload: S_PAYLOAD.to_vec(),
        })
        .unwrap();
    let spec = VectorSpec {
        source: RevisionId { unit: u, rev: RevNo(1) },
        method: S_VEC_METHOD.into(),
        method_version: 1,
        profile: S_VEC_PROFILE.into(),
        params: std::collections::BTreeMap::from([("k".to_owned(), S_VEC_PARAM.to_owned())]),
        vector: vec![S_VEC_ELEM, 1.0],
    };
    let id = cs.put_vector(&spec).unwrap();
    cs.commit().unwrap();

    let mut out = String::new();
    let mut say = |what: &str, text: String| out.push_str(&format!("[{what}] {text}\n"));
    // The control: the caller does get its vector, so the search could find it.
    let served = format!("{:?}", dd.vector(&id));
    assert!(
        vector_forms().iter().any(|f| served.contains(f.as_str())),
        "the search must be able to find the sentinel: {served}"
    );
    say("spec", format!("{spec:?}"));
    say("descriptor", format!("{:?}", dd.descriptor(&id)));
    say("state", format!("{:?}", dd.state(&id)));

    // Refusals.
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    for bad in [
        VectorSpec { vector: vec![S_VEC_ELEM, f32::NAN], ..spec.clone() },
        VectorSpec { vector: vec![S_VEC_ELEM; 5000], ..spec.clone() },
        VectorSpec { profile: S_VEC_PROFILE.repeat(5), ..spec.clone() },
        VectorSpec { source: RevisionId { unit: u, rev: RevNo(7) }, ..spec.clone() },
        spec.clone(),
    ] {
        match cs.put_vector(&bad) {
            Ok(_) => panic!("refused spec accepted"),
            Err(e) => say("put_vector", format!("{e} / {e:?}")),
        }
    }
    cs.abort();
    dd.close().unwrap();

    // Healthy and damaged records: every byte of the descriptor and of the
    // vector flipped in turn, then opened and verified.
    let healthy = disk.dump();
    let key = |t: u8| {
        let db = open_units::<E>(SimVfs::restore(&healthy).as_vfs(), &config()).unwrap();
        let snap = db.engine().snapshot().unwrap();
        let all = snap.scan(std::ops::Bound::Included(&[t][..]), std::ops::Bound::Excluded(&[t + 1][..]), 10).unwrap();
        db.close().ok();
        all.into_iter().next().unwrap()
    };
    let (desc_k, desc_v) = key(0x60);
    let (vec_k, vec_v) = key(0x61);
    let mut damages = vec![(desc_k.clone(), None), (vec_k.clone(), None)];
    for i in 0..desc_v.len() {
        let mut v = desc_v.clone();
        v[i] ^= 0x04;
        damages.push((desc_k.clone(), Some(v)));
    }
    for i in 0..vec_v.len() {
        let mut v = vec_v.clone();
        v[i] ^= 0x04;
        damages.push((vec_k.clone(), Some(v)));
    }
    damages.push((desc_k.clone(), Some(desc_v[..desc_v.len() - 1].to_vec())));
    for e in vector_decode_errors(&desc_v, &vec_v) {
        say("decode", e);
    }
    for (n, (k, v)) in damages.into_iter().enumerate() {
        let disk = SimVfs::restore(&healthy);
        let db = open_units::<E>(disk.as_vfs(), &config()).unwrap();
        let mut b = WriteBatch::new();
        match v {
            Some(v) => b.put(k, v),
            None => b.delete(k),
        };
        db.engine().commit(&b).unwrap();
        let report = db.verify_units().unwrap();
        say(
            "verify",
            format!("{n}: {:?} {}", report.findings, report.findings.iter().map(|f| f.to_string()).collect::<String>()),
        );
        for f in &report.findings {
            say("finding error", format!("{:?} {}", f.to_error(), f.to_error()));
        }
        match DerivedDb::open_persisted(db) {
            Ok((dd, r)) => {
                say("reopen", format!("{n}: {r:?} {:?}", r.findings));
                match dd.repair_derived(ChangeMeta::default()) {
                    Ok(r) => say("repair", format!("{r:?}")),
                    Err(e) => say("repair error", format!("{e} / {e:?}")),
                }
                let c = dd.begin_changeset(ChangeMeta::default()).unwrap();
                c.abort();
            }
            Err(e) => say("reopen error", format!("{n}: {e} / {e:?}")),
        }
    }
    for (what, spellings) in vector_sentinels() {
        for form in spellings {
            assert!(!out.contains(&form), "{what} sentinel `{form}` appears in the output of the vector types:\n{out}");
        }
    }
}

/// The text of the errors the codecs give for the descriptor and the vector
/// with each byte changed and each length cut.
fn vector_decode_errors(desc: &[u8], vec: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, _) in desc.iter().enumerate() {
        for cut in [false, true] {
            let mut d = desc.to_vec();
            if cut {
                d.truncate(i);
            } else {
                d[i] ^= 0xff;
            }
            if let Err(e) = kerno_units::VectorDescriptor::decode(&d) {
                out.push(format!("{e} {e:?}"));
            }
        }
    }
    for (i, _) in vec.iter().enumerate() {
        let mut v = vec.to_vec();
        v[i] ^= 0xff;
        if let Err(e) = kerno_units::vector_store::decode_vector(&v) {
            out.push(format!("{e} {e:?}"));
        }
        if let Err(e) = kerno_units::vector_store::decode_vector(&vec[..i]) {
            out.push(format!("{e} {e:?}"));
        }
    }
    out
}

// ---------------------------------------------------------------- E8-06

const S_SUM_PARAM: &str = "SNT-SUMPARAM-4c18e9a2";

/// A summarizer whose method and parameters are sentinels, and whose text is
/// the source payload itself.
struct Echo;

impl Summarizer for Echo {
    fn method(&self) -> &str {
        "SNT-SUMMETHOD-d2a60b71"
    }
    fn version(&self) -> u32 {
        1
    }
    fn params(&self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::from([("k".to_owned(), S_SUM_PARAM.to_owned())])
    }
    fn summarize(&self, sources: &[SourceInput]) -> Option<Vec<u8>> {
        sources.first().map(|s| s.payload.clone())
    }
}

/// E8-06 (I-37, I-43): the text of a summary and the content of its sources
/// appear in no error, no `Debug` or `Display` of the new types.
fn no_sentinel_in_summary_types_or_errors<E: Target>() {
    let disk = SimVfs::new(6);
    let db = open_units::<E>(disk.as_vfs(), &config()).unwrap();
    let dd = open_summaries(db, Vec::new(), &Echo).unwrap();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let u = cs
        .create(NewUnit {
            unit_type: S_TYPE.into(),
            schema_version: 1,
            metadata: S_METADATA.to_vec(),
            payload: S_PAYLOAD.to_vec(),
        })
        .unwrap();
    cs.commit().unwrap();
    let id = dd.build_summary([u], &Echo).unwrap();

    let mut out = String::new();
    let mut say = |what: &str, text: String| out.push_str(&format!("[{what}] {text}\n"));
    // The control: the caller does get its summary text, so the search could find it.
    let served = format!("{:?}", dd.summary(&id));
    assert!(served.contains(&forms(S_PAYLOAD)[1]), "the search must be able to find the sentinel: {served}");
    say("descriptor", format!("{:?}", dd.descriptor(&id)));
    say("state", format!("{:?}", dd.state(&id)));
    say("source", format!("{:?}", dd.get_unit(u).map(|o| o.map(|u| u.head))));

    let other = dd.build([u], "plain", 1, Default::default(), &|_, _| Some(S_PAYLOAD.to_vec())).unwrap();
    let results = [
        dd.build_summary(Vec::<UnitId>::new(), &Echo),
        dd.build_summary([UnitId([7; 16])], &Echo),
        dd.build_summary([u], &Echo), // duplicate
        dd.rebuild_summary(&other, &Echo),
        dd.supersede_summary(&other, &Echo),
        dd.rebuild_summary(&id, &TruncatingSummarizer::new(3, 1)),
        dd.supersede_summary(&DerivedId([1; 32]), &Echo),
    ];
    for r in results {
        match r {
            Ok(_) => {}
            Err(e) => say("error", format!("{e} / {e:?}")),
        }
    }
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(u, None).unwrap();
    cs.commit().unwrap();
    say("after delete", format!("{:?} {:?}", dd.state(&id), dd.descriptor(&id)));
    for (what, spellings) in sentinels()
        .into_iter()
        .filter(|(n, _)| matches!(*n, "payload" | "metadata" | "unit type"))
        .chain([("summary method", forms(b"SNT-SUMMETHOD-d2a60b71")), ("summary param", forms(S_SUM_PARAM.as_bytes()))])
    {
        for form in spellings {
            assert!(
                !out.contains(&form),
                "{what} sentinel `{form}` appears in the output of the summary types:\n{out}"
            );
        }
    }
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn no_sentinel_in_any_observability_explanation_or_error_output() {
                super::no_sentinel_in_any_observability_explanation_or_error_output::<E>();
            }
            #[test]
            fn no_sentinel_in_any_associative_plan_observation_or_error() {
                super::no_sentinel_in_any_associative_plan_observation_or_error::<E>();
            }
            #[test]
            fn no_sentinel_in_vector_types_errors_or_reports() {
                super::no_sentinel_in_vector_types_errors_or_reports::<E>();
            }
            #[test]
            fn no_sentinel_in_summary_types_or_errors() {
                super::no_sentinel_in_summary_types_or_errors::<E>();
            }
            #[test]
            fn levels_below_diagnostic_carry_no_shape_text() {
                super::levels_below_diagnostic_carry_no_shape_text::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
