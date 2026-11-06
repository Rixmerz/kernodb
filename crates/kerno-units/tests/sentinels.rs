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
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
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
    units.chain(relations).map(|q| q.fingerprint(key)).collect()
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
    for (what, needles) in wanted.iter().filter(|(w, _)| !["oversize type", "provenance origin"].contains(w)) {
        assert!(
            needles.iter().any(|n| answer.contains(n.as_str())),
            "control: the answer to the caller must hold the {what} sentinel"
        );
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
        let records = db.observer().records();
        match level {
            ObservationLevel::Off | ObservationLevel::Aggregate => assert!(records.is_empty(), "{level:?}"),
            ObservationLevel::Fingerprint => {
                assert_eq!(records.len(), 2);
                assert!(records.iter().all(|r| r.fingerprint_text.is_none()), "no shape text at {level:?}");
            }
            ObservationLevel::Diagnostic => {
                let texts: Vec<_> = records.iter().map(|r| r.fingerprint_text.as_deref()).collect();
                assert_eq!(
                    texts,
                    [
                        Some("units[unit_type=?text,schema_version=?u32]"),
                        Some("relations[out:unit=?unit_id,relation_type=?text]")
                    ]
                );
            }
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
            fn levels_below_diagnostic_carry_no_shape_text() {
                super::levels_below_diagnostic_carry_no_shape_text::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
