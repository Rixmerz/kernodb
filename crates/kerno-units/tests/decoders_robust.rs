//! Robustness of the `kerno-units` decoders against damaged bytes (E9-13, D21).
//!
//! For each record type (unit records, relation records, persisted derived
//! descriptors, persisted vectors) and each mode, 2 000 cases are run:
//!
//! * `mutate`: a valid encoding with 1 to 4 byte edits (flip, overwrite,
//!   truncate, append, insert, delete) applied to its key and to its value;
//! * `random`: arbitrary bytes of a bounded length.
//!
//! A decoder must never panic (a panic fails the property) and may only
//! refuse with an error of class `Corruption` or `IncompatibleLayer`. The
//! counters assert that every property really ran 2 000 cases. No engine is
//! opened, so there is nothing to run on a second engine.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use kerno_units::codec::{Adjacency, MAX_LAYER_VERSION};
use kerno_units::vector_store::{decode_src_entry, decode_vector, encode_src_entry, encode_vector};
use kerno_units::*;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

const CASES: u32 = 2000;

type Pair = (Vec<u8>, Vec<u8>);

fn unit(n: u8) -> UnitId {
    UnitId([n; 16])
}

fn hash() -> ContentHash {
    ContentHash::blake3(0, b"x")
}

fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    r.expect("seed must encode")
}

// ------------------------------------------------------------------ seeds

fn unit_seeds() -> Vec<Pair> {
    let head = UnitHead {
        unit_type: "note".into(),
        schema_version: 1,
        lifecycle: Lifecycle::Active,
        head_rev: RevNo(2),
        oldest_rev: RevNo(1),
        created_seq: CommitSeq(1),
        updated_seq: CommitSeq(2),
        created_at: Timestamp(-1),
        updated_at: Timestamp(5),
        content_hash: hash(),
        payload_len: 1,
        metadata: b"meta".to_vec(),
    };
    let rev = RevisionMeta {
        kind: RevisionKind::Update,
        parent_rev: Some(RevNo(1)),
        commit_seq: CommitSeq(9),
        created_at: Timestamp(0),
        schema_version: 1,
        content_hash: hash(),
        payload_len: 1,
        payload_present: true,
        restored_from: None,
        metadata: b"m".to_vec(),
        actor_ref: b"actor".to_vec(),
        source_ref: b"source".to_vec(),
    };
    let change_set = ChangeSetRecord {
        created_at: Timestamp(0),
        member_count: 1,
        purpose: Purpose::Application,
        reason: b"why".to_vec(),
        actor_ref: Vec::new(),
        source_ref: Vec::new(),
        correlation: Vec::new(),
    };
    let records = [
        Record::LayerMeta {
            name: "layer".into(),
            meta: LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned },
        },
        Record::UnitHead { unit: unit(7), head },
        Record::RevMeta { unit: unit(7), rev: RevNo(2), meta: rev },
        Record::RevPayload { unit: unit(7), rev: RevNo(2), payload: b"payload".to_vec() },
        Record::ChangeSet { seq: CommitSeq(9), record: change_set },
        Record::CsMember {
            seq: CommitSeq(9),
            ordinal: 0,
            member: CsMember { unit: unit(7), rev_no: RevNo(2), kind: RevisionKind::Update },
        },
        Record::IndexType { unit_type: "note".into(), schema_version: 1, unit: unit(7) },
    ];
    records.iter().map(|r| ok(r.encode())).collect()
}

fn relation_seeds() -> Vec<Pair> {
    let edge = RelationId([3; 16]);
    let provenances = [
        RelationProvenance::Declared,
        RelationProvenance::Imported { source: b"feed".to_vec() },
        RelationProvenance::Algorithmic { origin: b"rule".to_vec() },
        RelationProvenance::Inferred { origin: b"guess".to_vec(), confidence_permille: 640 },
    ];
    let mut out = Vec::new();
    for provenance in provenances {
        let record = codec::RelationRecord {
            from: unit(1),
            to: unit(2),
            relation_type: "cites".into(),
            created_seq: CommitSeq(4),
            provenance,
        };
        out.push(ok(Record::Relation { edge, record }.encode()));
    }
    let adjacency = Adjacency { other: unit(2) };
    out.push(ok(Record::RelationOut { unit: unit(1), relation_type: "cites".into(), edge, adjacency }.encode()));
    out.push(ok(Record::RelationIn { unit: unit(2), relation_type: "cites".into(), edge, adjacency }.encode()));
    out
}

fn descriptor_seeds() -> Vec<Vec<u8>> {
    let base = VectorDescriptor {
        state: VectorState::Ready,
        source: RevisionId { unit: unit(5), rev: RevNo(2) },
        method: "embed".into(),
        method_version: 3,
        profile: "model-a".into(),
        params: BTreeMap::from([("alpha".to_string(), "1".to_string()), ("beta".to_string(), String::new())]),
        input_hash: [1; 32],
        output_hash: [2; 32],
    };
    let mut out = Vec::new();
    for state in [VectorState::Ready, VectorState::Stale, VectorState::Superseded] {
        out.push(ok(VectorDescriptor { state, ..base.clone() }.encode()));
    }
    out.push(ok(VectorDescriptor { params: BTreeMap::new(), ..base }.encode()));
    out
}

fn vector_seeds() -> Vec<Vec<u8>> {
    vec![ok(encode_vector(&[1.0, -2.5, 0.0, 3.75])), ok(encode_vector(&[0.5])), encode_src_entry()]
}

// -------------------------------------------------------------- mutation

/// One edit: `(kind, position, byte)`; the position wraps to the length.
type Edit = (u8, usize, u8);

fn edits() -> impl Strategy<Value = Vec<Edit>> {
    proptest::collection::vec((0u8..6, any::<usize>(), any::<u8>()), 0..=4)
}

fn apply(bytes: &[u8], edits: &[Edit]) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for &(kind, pos, byte) in edits {
        let at = if out.is_empty() { 0 } else { pos % out.len() };
        match kind {
            0 if !out.is_empty() => out[at] ^= byte | 1,
            1 if !out.is_empty() => out[at] = byte,
            2 => out.truncate(at),
            3 => out.push(byte),
            4 => out.insert(at, byte),
            5 if !out.is_empty() => {
                out.remove(at);
            }
            _ => {}
        }
    }
    out
}

fn random_bytes() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=256)
}

/// The only classes a refusal may have.
fn class_ok(class: ErrorClass) -> Result<(), TestCaseError> {
    prop_assert!(matches!(class, ErrorClass::Corruption | ErrorClass::IncompatibleLayer));
    Ok(())
}

/// Runs exactly `CASES` cases and checks the counter saw them all.
fn run<S: Strategy>(strategy: S, body: impl Fn(S::Value) -> Result<(), TestCaseError>) {
    let seen = AtomicU32::new(0);
    let mut runner = TestRunner::new(Config { cases: CASES, failure_persistence: None, ..Config::default() });
    runner
        .run(&strategy, |v| {
            seen.fetch_add(1, Ordering::Relaxed);
            body(v)
        })
        .unwrap();
    assert_eq!(seen.load(Ordering::Relaxed), CASES, "the property did not run every case");
}

// ------------------------------------------------------------- checkers

fn check_record(key: &[u8], value: &[u8]) -> Result<(), TestCaseError> {
    match Record::decode_upto(key, value, MAX_LAYER_VERSION) {
        Ok(_) => Ok(()),
        Err(e) => class_ok(e.rule.class()),
    }
}

fn check_descriptor(value: &[u8]) -> Result<(), TestCaseError> {
    match VectorDescriptor::decode(value) {
        Ok(d) => {
            let again = ok(d.encode());
            prop_assert_eq!(again.as_slice(), value);
            Ok(())
        }
        Err(e) => class_ok(e.class()),
    }
}

fn check_vector(value: &[u8]) -> Result<(), TestCaseError> {
    let vec = decode_vector(value);
    let src = decode_src_entry(value);
    if let Ok(v) = &vec {
        let again = ok(encode_vector(v));
        prop_assert_eq!(again.as_slice(), value);
    }
    for e in [vec.err(), src.err()].into_iter().flatten() {
        class_ok(e.class())?;
    }
    Ok(())
}

fn check_derived_key(key: &[u8]) -> Result<(), TestCaseError> {
    match DerivedKey::decode(key) {
        Ok(_) => Ok(()),
        Err(e) => class_ok(e.class()),
    }
}

// ------------------------------------------------------------ properties

fn pair_mutations(seeds: Vec<Pair>) -> impl Strategy<Value = (Pair, Vec<Edit>, Vec<Edit>)> {
    let n = seeds.len();
    (0..n, edits(), edits()).prop_map(move |(i, ek, ev)| (seeds[i].clone(), ek, ev))
}

fn pair_check(((key, value), ek, ev): (Pair, Vec<Edit>, Vec<Edit>)) -> Result<(), TestCaseError> {
    check_record(&apply(&key, &ek), &apply(&value, &ev))
}

#[test]
fn seeds_decode() {
    for (k, v) in unit_seeds().into_iter().chain(relation_seeds()) {
        Record::decode(&k, &v).expect("a seed must decode");
    }
    for d in descriptor_seeds() {
        VectorDescriptor::decode(&d).expect("a descriptor seed must decode");
    }
    for v in &vector_seeds()[..2] {
        decode_vector(v).expect("a vector seed must decode");
    }
}

#[test]
fn unit_records_mutated() {
    run(pair_mutations(unit_seeds()), pair_check);
}

#[test]
fn unit_records_random() {
    let seeds = unit_seeds();
    let n = seeds.len();
    // A valid key reaches the value decoder; a random key is refused at the key.
    run((0..n, random_bytes(), any::<bool>(), random_bytes(), any::<bool>()), move |(i, v, raw_key, k, envelope)| {
        // Half the values start with the seed's envelope so the field readers run.
        let v = if envelope { [&seeds[i].1[..2], &v].concat() } else { v };
        check_record(if raw_key { &k } else { &seeds[i].0 }, &v)
    });
}

#[test]
fn relation_records_mutated() {
    run(pair_mutations(relation_seeds()), pair_check);
}

#[test]
fn relation_records_random() {
    let seeds = relation_seeds();
    let n = seeds.len();
    run((0..n, random_bytes(), any::<bool>(), random_bytes(), any::<bool>()), move |(i, v, raw_key, k, envelope)| {
        // Half the values start with the seed's envelope so the field readers run.
        let v = if envelope { [&seeds[i].1[..2], &v].concat() } else { v };
        check_record(if raw_key { &k } else { &seeds[i].0 }, &v)
    });
}

#[test]
fn descriptors_mutated() {
    let seeds = descriptor_seeds();
    let n = seeds.len();
    run((0..n, edits()), move |(i, e)| check_descriptor(&apply(&seeds[i], &e)));
}

#[test]
fn descriptors_random() {
    run(random_bytes(), |v| check_descriptor(&v));
}

#[test]
fn vectors_mutated() {
    let seeds = vector_seeds();
    let n = seeds.len();
    run((0..n, edits()), move |(i, e)| check_vector(&apply(&seeds[i], &e)));
}

#[test]
fn vectors_random() {
    run(random_bytes(), |v| check_vector(&v));
}

#[test]
fn derived_keys_mutated_and_random() {
    let keys: Vec<Vec<u8>> = [0x60u8, 0x61, 0x62]
        .iter()
        .map(|&tag| {
            let len = if tag == 0x62 { 57 } else { 33 };
            let mut k = vec![7u8; len];
            k[0] = tag;
            k
        })
        .collect();
    let n = keys.len();
    run((0..n, edits()), move |(i, e)| check_derived_key(&apply(&keys[i], &e)));
    run(random_bytes(), |k| check_derived_key(&k));
}
