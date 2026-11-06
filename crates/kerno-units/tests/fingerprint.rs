//! E5-01: the normalized fingerprint of a query (ADR-0015, etapa5-acta.md §3).
//!
//! * `vectors_are_fixed` — fixed shapes and digests under a fixed key. The
//!   digests were derived with an independent BLAKE3 (checked against the
//!   official keyed test vector), not read back from this crate: a change of
//!   algorithm, label, truncation or shape grammar fails here and must bump
//!   `FINGERPRINT_VERSION`.
//! * `same_shape_same_fingerprint_whatever_the_literals` — literals do not
//!   reach the hash.
//! * `no_literal_in_shape_or_fingerprint` — sentinel literals appear in
//!   neither the shape text nor any rendering of the fingerprint.
//! * `shapes_are_told_apart` — every condition that is set, the direction and
//!   the kind change the fingerprint.
//! * `key_*` — the key is part of the hash, is hidden by `Debug`, and a
//!   generated one differs from the next.

use kerno_units::*;

fn key() -> FingerprintKey {
    FingerprintKey::from_bytes(std::array::from_fn(|i| i as u8))
}

fn filter(unit_type: Option<&str>, schema_version: Option<u32>) -> UnitFilter {
    UnitFilter { unit_type: unit_type.map(Into::into), schema_version }
}

fn rel(direction: Direction, relation_type: Option<&str>) -> RelationQuery {
    RelationQuery { unit: UnitId([0xA7; 16]), direction, relation_type: relation_type.map(Into::into) }
}

fn hex(f: &Fingerprint) -> String {
    f.digest().iter().map(|b| format!("{b:02x}")).collect()
}

/// The queries of the vectors, with the shape and digest each must give.
fn vectors() -> Vec<(NormalizedQuery, &'static str, &'static str)> {
    vec![
        (NormalizedQuery::of_filter(&filter(None, None)), "units[]", "49d262d25e578305c84d37a0f9ce706b"),
        (
            NormalizedQuery::of_filter(&filter(Some("invoice"), None)),
            "units[unit_type=?text]",
            "e8bcfadb75887d86b4c5213f744bd294",
        ),
        (
            NormalizedQuery::of_filter(&filter(None, Some(3))),
            "units[schema_version=?u32]",
            "6a72f11972d572a9443efc1d82fadcd7",
        ),
        (
            NormalizedQuery::of_filter(&filter(Some("invoice"), Some(3))),
            "units[unit_type=?text,schema_version=?u32]",
            "19d3354d6f4441af05296c69fd0571c0",
        ),
        (
            NormalizedQuery::of_relations(&rel(Direction::Out, None)),
            "relations[out:unit=?unit_id]",
            "8e6e61c2c9cf14b31edb8fb6807581c6",
        ),
        (
            NormalizedQuery::of_relations(&rel(Direction::In, None)),
            "relations[in:unit=?unit_id]",
            "fc4cb315402815b341b00394d639da3b",
        ),
        (
            NormalizedQuery::of_relations(&rel(Direction::Out, Some("cites"))),
            "relations[out:unit=?unit_id,relation_type=?text]",
            "a6695efc3d1eeace81f9dc52d1f5f3ac",
        ),
        (
            NormalizedQuery::of_relations(&rel(Direction::In, Some("cites"))),
            "relations[in:unit=?unit_id,relation_type=?text]",
            "39a4681931a3fd4c3af589d16066751f",
        ),
    ]
}

#[test]
fn vectors_are_fixed() {
    for (query, shape, digest) in vectors() {
        assert_eq!(query.text(), shape);
        let f = query.fingerprint(&key());
        assert_eq!(hex(&f), digest, "digest of {shape}");
        assert_eq!(f.kind(), query.kind());
        assert_eq!(f.alg(), FingerprintAlg::Blake3Keyed);
        assert_eq!(f.version(), 1);
        assert_eq!(FINGERPRINT_VERSION, 1);
        let kind = if query.kind() == QueryKind::Units { "units" } else { "relations" };
        assert_eq!(f.to_string(), format!("{kind}/blake3-keyed/v1/{digest}"));
    }
}

#[test]
fn same_shape_same_fingerprint_whatever_the_literals() {
    let k = key();
    let a = NormalizedQuery::of_filter(&filter(Some("invoice"), Some(3))).fingerprint(&k);
    let b = NormalizedQuery::of_filter(&filter(Some("receipt"), Some(9))).fingerprint(&k);
    assert_eq!(a, b);
    let a = NormalizedQuery::of_relations(&rel(Direction::In, Some("cites"))).fingerprint(&k);
    let other =
        RelationQuery { unit: UnitId([0x11; 16]), direction: Direction::In, relation_type: Some("blocks".into()) };
    assert_eq!(a, NormalizedQuery::of_relations(&other).fingerprint(&k));
}

#[test]
fn no_literal_in_shape_or_fingerprint() {
    let k = key();
    let unit = UnitId([0xA7; 16]);
    let queries = [
        NormalizedQuery::of_filter(&filter(Some("SENTINEL-type-7f3a"), Some(987_654))),
        NormalizedQuery::of_relations(&RelationQuery {
            unit,
            direction: Direction::Out,
            relation_type: Some("SENTINEL-rel-91bc".into()),
        }),
    ];
    for q in &queries {
        let f = q.fingerprint(&k);
        let shown = [q.text().to_owned(), f.to_string(), format!("{f:?}")];
        for text in &shown {
            for literal in ["SENTINEL", "987654", "a7a7a7a7", &unit.to_string()] {
                assert!(!text.contains(literal), "{literal:?} leaked into {text:?}");
            }
        }
    }
}

#[test]
fn shapes_are_told_apart() {
    let k = key();
    let all: Vec<Fingerprint> = vectors().iter().map(|(q, _, _)| q.fingerprint(&k)).collect();
    for (i, a) in all.iter().enumerate() {
        for b in &all[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn key_is_part_of_the_hash() {
    let q = NormalizedQuery::of_filter(&filter(Some("invoice"), None));
    let other = FingerprintKey::from_bytes([7; 32]);
    assert_ne!(q.fingerprint(&key()), q.fingerprint(&other));
    assert_eq!(q.fingerprint(&key()), q.fingerprint(&key()));
}

#[test]
fn key_is_hidden_by_debug() {
    assert_eq!(format!("{:?}", FingerprintKey::from_bytes([0x5E; 32])), "FingerprintKey(..)");
}

#[test]
fn generated_keys_differ() {
    let q = NormalizedQuery::of_filter(&filter(None, None));
    let a = FingerprintKey::generate().expect("a key");
    let b = FingerprintKey::generate().expect("a key");
    assert_ne!(q.fingerprint(&a), q.fingerprint(&b));
    assert_ne!(q.fingerprint(&a), q.fingerprint(&FingerprintKey::from_bytes([0; 32])));
}
