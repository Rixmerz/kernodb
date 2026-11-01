//! Production codecs, errors, clock/id sources and the reference model
//! (E3-02). The compatibility with the committed structure vectors lives in
//! `struct_vectors.rs` (`production_codec_matches_committed_vectors`).

use std::sync::Arc;

use kerno_units::codec::{
    MAX_CHANGESET_FIELD_LEN, MAX_MEMBER_COUNT, MAX_METADATA_LEN, MAX_REV_REF_LEN, MAX_UNIT_TYPE_LEN,
};
use kerno_units::*;
use proptest::prelude::*;

const U: UnitId = UnitId([7; 16]);

fn hash() -> ContentHash {
    ContentHash::blake3(0, b"x")
}

fn head() -> UnitHead {
    UnitHead {
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
        metadata: Vec::new(),
    }
}

fn rev_meta() -> RevisionMeta {
    RevisionMeta {
        kind: RevisionKind::Update,
        parent_rev: Some(RevNo(1)),
        commit_seq: CommitSeq(9),
        created_at: Timestamp(0),
        schema_version: 1,
        content_hash: hash(),
        payload_len: 1,
        payload_present: true,
        restored_from: None,
        metadata: Vec::new(),
        actor_ref: Vec::new(),
        source_ref: Vec::new(),
    }
}

fn changeset() -> ChangeSetRecord {
    ChangeSetRecord {
        created_at: Timestamp(0),
        member_count: 1,
        purpose: Purpose::Application,
        reason: Vec::new(),
        actor_ref: Vec::new(),
        source_ref: Vec::new(),
        correlation: Vec::new(),
    }
}

fn rule_of<T: std::fmt::Debug>(r: Result<T, DecodeError>) -> u8 {
    r.expect_err("must reject").rule.number()
}

fn bytes_upto(max: u32) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=max as usize)
}

fn content_hash() -> impl Strategy<Value = ContentHash> {
    prop_oneof![
        any::<[u8; 32]>().prop_map(|digest| ContentHash { alg: HashAlg::Blake3, digest }),
        any::<u32>().prop_map(|crc| {
            let mut digest = [0; 32];
            digest[..4].copy_from_slice(&crc.to_be_bytes());
            ContentHash { alg: HashAlg::Crc32cProvisional, digest }
        }),
    ]
}

fn unit_id() -> impl Strategy<Value = UnitId> {
    any::<[u8; 16]>().prop_map(UnitId)
}

fn kind() -> impl Strategy<Value = RevisionKind> {
    proptest::sample::select(RevisionKind::ALL)
}

fn valid_key() -> impl Strategy<Value = Key> {
    prop_oneof![
        "[a-z\u{e9}\u{4e2d}]{1,16}"
            .prop_filter("≤ 64 bytes", |s| s.len() <= 64)
            .prop_map(|name| Key::LayerMeta { name }),
        unit_id().prop_map(|unit| Key::UnitHead { unit }),
        (unit_id(), 1..=u64::MAX).prop_map(|(unit, r)| Key::RevMeta { unit, rev: RevNo(r) }),
        (unit_id(), 1..=u64::MAX).prop_map(|(unit, r)| Key::RevPayload { unit, rev: RevNo(r) }),
        (1..=u64::MAX).prop_map(|s| Key::ChangeSet { seq: CommitSeq(s) }),
        (1..=u64::MAX, any::<u32>()).prop_map(|(s, ordinal)| Key::CsMember { seq: CommitSeq(s), ordinal }),
    ]
}

fn valid_head() -> impl Strategy<Value = UnitHead> {
    (
        "[a-zA-Z0-9_]{1,64}",
        any::<u32>(),
        proptest::sample::select(Lifecycle::ALL),
        (1..=u64::MAX, 1..=u64::MAX, 1..=u64::MAX, 1..=u64::MAX),
        (any::<i64>(), any::<i64>()),
        content_hash(),
        0..=MAX_PAYLOAD_LEN,
        bytes_upto(MAX_METADATA_LEN),
    )
        .prop_map(
            |(unit_type, schema_version, lifecycle, (a, b, c, d), (t0, t1), content_hash, payload_len, metadata)| {
                UnitHead {
                    unit_type,
                    schema_version,
                    lifecycle,
                    head_rev: RevNo(a.max(b)),
                    oldest_rev: RevNo(a.min(b)),
                    created_seq: CommitSeq(c.min(d)),
                    updated_seq: CommitSeq(c.max(d)),
                    created_at: Timestamp(t0),
                    updated_at: Timestamp(t1),
                    content_hash,
                    payload_len,
                    metadata,
                }
            },
        )
}

fn valid_rev_meta() -> impl Strategy<Value = (RevNo, RevisionMeta)> {
    (
        kind(),
        2..=u64::MAX,
        any::<u64>(),
        any::<u64>(),
        1..=u64::MAX,
        any::<i64>(),
        any::<u32>(),
        content_hash(),
        (0..=MAX_PAYLOAD_LEN, any::<bool>()),
        (bytes_upto(MAX_METADATA_LEN), bytes_upto(MAX_REV_REF_LEN), bytes_upto(MAX_REV_REF_LEN)),
    )
        .prop_map(|(kind, rev, p, s, commit, t, schema_version, content_hash, (len, present), (m, a, src))| {
            let create = kind == RevisionKind::Create;
            let rev = if create { 1 + rev % 3 } else { rev };
            let delete = kind == RevisionKind::Delete;
            (
                RevNo(rev),
                RevisionMeta {
                    kind,
                    parent_rev: (!create).then(|| RevNo(1 + p % (rev - 1))),
                    commit_seq: CommitSeq(commit),
                    created_at: Timestamp(t),
                    schema_version,
                    content_hash,
                    payload_len: if delete { 0 } else { len },
                    payload_present: present && !delete,
                    restored_from: (kind == RevisionKind::Restore).then(|| RevNo(1 + s % (rev - 1))),
                    metadata: m,
                    actor_ref: a,
                    source_ref: src,
                },
            )
        })
}

fn valid_changeset() -> impl Strategy<Value = ChangeSetRecord> {
    (
        any::<i64>(),
        0..=MAX_MEMBER_COUNT,
        proptest::sample::select(Purpose::ALL),
        (
            bytes_upto(MAX_CHANGESET_FIELD_LEN),
            bytes_upto(MAX_CHANGESET_FIELD_LEN),
            bytes_upto(MAX_CHANGESET_FIELD_LEN),
            bytes_upto(MAX_CHANGESET_FIELD_LEN),
        ),
    )
        .prop_map(|(t, member_count, purpose, (reason, actor_ref, source_ref, correlation))| ChangeSetRecord {
            created_at: Timestamp(t),
            member_count,
            purpose,
            reason,
            actor_ref,
            source_ref,
            correlation,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn key_round_trip(k in valid_key()) {
        let bytes = k.encode().unwrap();
        prop_assert_eq!(Key::decode(&bytes).unwrap(), k);
    }

    /// Lexicographic order of the encoded keys is the numeric (derived) order.
    #[test]
    fn key_byte_order_is_numeric_order(a in valid_key(), b in valid_key()) {
        prop_assert_eq!(a.encode().unwrap().cmp(&b.encode().unwrap()), a.cmp(&b));
    }

    /// Same-tag keys with numeric fields: byte order follows the numbers.
    #[test]
    fn rev_and_seq_keys_order_numerically(u in unit_id(), r1 in 1..=u64::MAX, r2 in 1..=u64::MAX, o1: u32, o2: u32) {
        let k = |r| Key::RevMeta { unit: u, rev: RevNo(r) }.encode().unwrap();
        prop_assert_eq!(k(r1).cmp(&k(r2)), r1.cmp(&r2));
        let m = |o| Key::CsMember { seq: CommitSeq(r1), ordinal: o }.encode().unwrap();
        prop_assert_eq!(m(o1).cmp(&m(o2)), o1.cmp(&o2));
        let c = |s| Key::ChangeSet { seq: CommitSeq(s) }.encode().unwrap();
        prop_assert_eq!(c(r1).cmp(&c(r2)), r1.cmp(&r2));
    }

    #[test]
    fn head_round_trip(h in valid_head(), u in unit_id()) {
        let rec = Record::UnitHead { unit: u, head: h };
        let (k, v) = rec.encode().unwrap();
        prop_assert_eq!(Record::decode(&k, &v).unwrap(), rec);
    }

    #[test]
    fn rev_meta_round_trip((rev, meta) in valid_rev_meta(), u in unit_id()) {
        let rec = Record::RevMeta { unit: u, rev, meta };
        let (k, v) = rec.encode().unwrap();
        prop_assert_eq!(Record::decode(&k, &v).unwrap(), rec);
    }

    #[test]
    fn changeset_round_trip(c in valid_changeset(), s in 1..=u64::MAX) {
        let rec = Record::ChangeSet { seq: CommitSeq(s), record: c };
        let (k, v) = rec.encode().unwrap();
        prop_assert_eq!(Record::decode(&k, &v).unwrap(), rec);
    }

    #[test]
    fn small_records_round_trip(u in unit_id(), s in 1..=u64::MAX, o: u32, r in 1..=u64::MAX, kind in kind(),
                                p in bytes_upto(64), pol in proptest::sample::select(HistoryPolicy::ALL)) {
        for rec in [
            Record::CsMember { seq: CommitSeq(s), ordinal: o, member: CsMember { unit: u, rev_no: RevNo(r), kind } },
            Record::RevPayload { unit: u, rev: RevNo(r), payload: p.clone() },
            Record::LayerMeta { name: "layer".into(), meta: LayerMeta { layer_version: 1, history_policy: pol } },
        ] {
            let (k, v) = rec.encode().unwrap();
            prop_assert_eq!(Record::decode(&k, &v).unwrap(), rec);
        }
    }

    /// Arbitrary bytes never panic and are either accepted or one rule.
    #[test]
    fn arbitrary_input_never_panics(tag in prop_oneof![Just(0x01u8), Just(0x10), Just(0x20), Just(0x21), Just(0x30), Just(0x31), any::<u8>()],
                                    tail in proptest::collection::vec(any::<u8>(), 0..30),
                                    value in proptest::collection::vec(any::<u8>(), 0..200)) {
        let key = [vec![tag], tail].concat();
        if let Err(e) = Record::decode(&key, &value) {
            prop_assert!((1..=25).contains(&e.rule.number()));
        }
    }
}

#[test]
fn limits_are_accepted_exactly_and_refused_one_above() {
    let at =
        |unit_type: String, metadata: Vec<u8>, payload_len| UnitHead { unit_type, metadata, payload_len, ..head() };
    let ok = at("x".repeat(MAX_UNIT_TYPE_LEN as usize), vec![0; MAX_METADATA_LEN as usize], MAX_PAYLOAD_LEN);
    assert_eq!(UnitHead::decode(&ok.encode().unwrap()).unwrap(), ok);
    for bad in [
        at("x".repeat(65), Vec::new(), 1),
        at("x".into(), vec![0; 4097], 1),
        at("x".into(), Vec::new(), MAX_PAYLOAD_LEN + 1),
        at(String::new(), Vec::new(), 1),
    ] {
        assert!(matches!(bad.encode(), Err(UnitError::InvalidArgument(_))));
    }
    let rm = RevisionMeta { actor_ref: vec![0; 257], ..rev_meta() };
    assert!(rm.encode(RevNo(2)).is_err());
    let rm = RevisionMeta { actor_ref: vec![0; 256], source_ref: vec![0; 256], ..rev_meta() };
    assert!(rm.encode(RevNo(2)).is_ok());
    let cs = ChangeSetRecord { member_count: MAX_MEMBER_COUNT, correlation: vec![0; 1024], ..changeset() };
    assert_eq!(ChangeSetRecord::decode(&cs.encode().unwrap()).unwrap(), cs);
    assert!(ChangeSetRecord { member_count: MAX_MEMBER_COUNT + 1, ..changeset() }.encode().is_err());
    assert!(ChangeSetRecord { reason: vec![0; 1025], ..changeset() }.encode().is_err());
    assert!(Key::LayerMeta { name: "n".repeat(64) }.encode().is_ok());
    assert!(Key::LayerMeta { name: "n".repeat(65) }.encode().is_err());
    assert!(Key::LayerMeta { name: String::new() }.encode().is_err());
    assert!(Key::RevMeta { unit: U, rev: RevNo(0) }.encode().is_err());
    assert!(Key::ChangeSet { seq: CommitSeq(0) }.encode().is_err());
    // Some(0) would encode like None: refused rather than silently changed.
    assert!(
        RevisionMeta { parent_rev: Some(RevNo(0)), kind: RevisionKind::Create, ..rev_meta() }.encode(RevNo(1)).is_err()
    );
}

#[test]
fn length_prefix_u32_max_is_unit_11_not_9() {
    let mut v = UnitHead { metadata: Vec::new(), ..head() }.encode().unwrap();
    let n = v.len();
    v[n - 4..].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(rule_of(UnitHead::decode(&v)), 11);
    // Same at the first prefix (unit_type), with nothing after it.
    assert_eq!(rule_of(UnitHead::decode(&[1, 0, 0xFF, 0xFF, 0xFF, 0xFF])), 11);
    // A prefix within the maximum but past the end is UNIT-9.
    assert_eq!(rule_of(UnitHead::decode(&[1, 0, 0, 0, 0, 64, b'x'])), 9);
}

#[test]
fn empty_and_truncated_values() {
    assert_eq!(rule_of(LayerMeta::decode(&[])), 9);
    assert_eq!(rule_of(UnitHead::decode(&[])), 9);
    assert_eq!(rule_of(RevisionMeta::decode(RevNo(2), &[])), 9);
    assert_eq!(rule_of(ChangeSetRecord::decode(&[])), 9);
    assert_eq!(rule_of(CsMember::decode(&[1])), 9);
    // An empty REV_PAYLOAD value is a valid, empty payload (§3.4).
    let k = Key::RevPayload { unit: U, rev: RevNo(1) }.encode().unwrap();
    assert_eq!(Record::decode(&k, &[]).unwrap(), Record::RevPayload { unit: U, rev: RevNo(1), payload: Vec::new() });
}

/// One hand-made case per rule, checked for rule and class, so the
/// production decoder is exercised on every UNIT-n independently of vectors.
#[test]
fn every_rule_is_rejected_with_its_class() {
    let hk = Key::UnitHead { unit: U }.encode().unwrap();
    let rk = Key::RevMeta { unit: U, rev: RevNo(2) }.encode().unwrap();
    let hv = head().encode().unwrap();
    let rv = rev_meta().encode(RevNo(2)).unwrap();
    let lv = LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned }.encode().unwrap();
    let lk = Key::LayerMeta { name: "layer".into() }.encode().unwrap();
    let ck = Key::ChangeSet { seq: CommitSeq(3) }.encode().unwrap();
    let mk = Key::CsMember { seq: CommitSeq(3), ordinal: 0 }.encode().unwrap();
    let mv = CsMember { unit: U, rev_no: RevNo(1), kind: RevisionKind::Create }.encode().unwrap();
    let set = |v: &[u8], at: usize, b: u8| {
        let mut v = v.to_vec();
        v[at] = b;
        v
    };
    // Offsets into REV_META: kind 2, parent_rev 3..11, commit_seq 11..19,
    // hash alg 31, payload_len 64..68, codec 68, present 69, restored_from 70..78.
    // Offsets into UNIT_HEAD with unit_type "note": oldest_rev 23..31, created_seq 31..39.
    let cases: Vec<(u8, Vec<u8>, Vec<u8>)> = vec![
        (1, vec![], lv.clone()),
        (1, vec![0x11; 17], hv.clone()),
        (2, vec![0x40], vec![]),
        (3, hk[..16].to_vec(), hv.clone()),
        (4, vec![0x01, 0xC3, 0x28], lv.clone()),
        (5, [&rk[..17], &[0; 8][..]].concat(), rv.clone()),
        (6, rk.clone(), set(&rv, 0, 0)),
        (7, rk.clone(), vec![9]),
        (8, rk.clone(), set(&rv, 1, 4)),
        (9, rk.clone(), rv[..rv.len() - 1].to_vec()),
        (10, rk.clone(), [rv.clone(), vec![0]].concat()),
        (11, hk.clone(), [&hv[..2], &[0, 0, 0, 65][..], &hv[6..]].concat()),
        (12, hk.clone(), [&hv[..2], &[0, 0, 0, 0][..], &hv[10..]].concat()),
        (13, rk.clone(), set(&rv, 2, 7)),
        (13, rk.clone(), set(&rv, 68, 1)),
        (13, rk.clone(), set(&rv, 31, 2)),
        (14, rk.clone(), {
            let mut v = set(&rv, 31, 0);
            v[63] = 1;
            v
        }),
        (15, lk.clone(), set(&lv, 5, 0)),
        (16, lk.clone(), set(&lv, 5, 2)),
        (
            17,
            hk.clone(),
            set(&UnitHead { head_rev: RevNo(1), oldest_rev: RevNo(1), ..head() }.encode().unwrap(), 30, 2),
        ),
        (18, hk.clone(), set(&hv, 38, 0)),    // created_seq := 0
        (19, rk.clone(), set(&rv, 10, 2)),    // parent_rev := 2 = rev_no
        (20, rk.clone(), set(&rv, 77, 1)),    // restored_from := 1 on Update
        (21, rk.clone(), set(&rv, 69, 2)),    // payload_present := 2
        (22, rk.clone(), set(&rv, 64, 0x10)), // payload_len high byte
        (23, rk.clone(), set(&rv, 18, 0)),    // commit_seq := 0
        (24, ck.clone(), set(&changeset().encode().unwrap(), 10, 0x20)),
        (25, mk.clone(), set(&mv, 25, 0)),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (want, key, value) in cases {
        let err = Record::decode(&key, &value).expect_err(&format!("UNIT-{want} case accepted"));
        assert_eq!(err.rule.number(), want, "{err}");
        let class = if matches!(want, 2 | 7 | 16) { ErrorClass::IncompatibleLayer } else { ErrorClass::Corruption };
        assert_eq!(err.class(), class);
        seen.insert(want);
        // Deterministic message and mapping into kerno_core::Error.
        assert_eq!(Record::decode(&key, &value).unwrap_err().to_string(), err.to_string());
        assert!(err.to_string().starts_with(&format!("UNIT-{want} ")));
        match (kerno_core::Error::from(err.clone()), class) {
            (kerno_core::Error::Corruption(m), ErrorClass::Corruption)
            | (kerno_core::Error::Incompatible(m), ErrorClass::IncompatibleLayer) => assert_eq!(m, err.to_string()),
            (other, _) => panic!("UNIT-{want} mapped to {other:?}"),
        }
        assert!(matches!(UnitError::from(err), UnitError::Storage(_)));
    }
    assert_eq!(seen.len(), 25);
    assert_eq!(UnitRule::ALL.iter().map(|r| r.number()).collect::<Vec<_>>(), (1..=25).collect::<Vec<_>>());
}

#[test]
fn messages_carry_no_content() {
    let secret = b"SECRET-CONTENT".to_vec();
    let mut v = UnitHead { metadata: secret.clone(), ..head() }.encode().unwrap();
    v.push(0);
    let msg = UnitHead::decode(&v).unwrap_err().to_string();
    assert!(!msg.contains("SECRET"), "{msg}");
    assert_eq!(msg, "UNIT-10 (Corruption): UNIT_HEAD has 1 trailing bytes");
}

#[test]
fn rev_meta_feeds_payload_verification() {
    let payload = b"hello";
    let meta = RevisionMeta { content_hash: ContentHash::blake3(0, payload), payload_len: 5, ..rev_meta() };
    assert_eq!(verify_payload(&meta.payload_decl(), Some(payload)).unwrap().bytes(), Some(&payload[..]));
    assert!(verify_payload(&meta.payload_decl(), Some(b"hellO")).is_err());
}

#[test]
fn fixed_clock_and_counter_ids_are_deterministic() {
    let c = FixedClock::stepping(Timestamp(100), 10);
    assert_eq!([c.now(), c.now(), c.now()], [Timestamp(100), Timestamp(110), Timestamp(120)]);
    c.set(Timestamp(-5));
    assert_eq!(c.now(), Timestamp(-5));
    let still = FixedClock::new(Timestamp(7));
    assert_eq!(still.now(), still.now());
    let ids = CounterIds::new(1);
    let a = ids.next_id();
    let b = ids.next_id();
    assert_eq!(a.0[15], 1);
    assert!(a < b);
}

fn v7_ids(clock_steps: &[i64], seed: u64) -> Vec<UnitId> {
    let clock = Arc::new(FixedClock::new(Timestamp(0)));
    let src = UuidV7Source::with_seed(clock.clone(), seed);
    clock_steps
        .iter()
        .map(|t| {
            clock.set(Timestamp(*t));
            src.next_id()
        })
        .collect()
}

#[test]
fn uuid_v7_is_deterministic_well_formed_and_increasing() {
    // Clock forwards, stalled, and backwards.
    let times = [1_761_000_000_000_000, 1_761_000_000_000_000, 1_761_000_000_500_000, 5, 1_761_000_001_000_000];
    let a = v7_ids(&times, 42);
    assert_eq!(a, v7_ids(&times, 42));
    assert_ne!(a, v7_ids(&times, 43));
    for id in &a {
        assert_eq!(id.0[6] >> 4, 7, "version");
        assert_eq!(id.0[8] >> 6, 0b10, "variant");
    }
    assert!(a.windows(2).all(|w| w[0] < w[1]), "{a:?}");
    let ms = u64::from_be_bytes([0, 0, a[0].0[0], a[0].0[1], a[0].0[2], a[0].0[3], a[0].0[4], a[0].0[5]]);
    assert_eq!(ms, 1_761_000_000_000);
    // Counter overflow within one millisecond still increases.
    let many = v7_ids(&[1_000_000; 10_000], 1);
    assert!(many.windows(2).all(|w| w[0] < w[1]));
    // The default source works and produces distinct ids.
    let d = UuidV7Source::new();
    assert_ne!(d.next_id(), d.next_id());
    assert!(SystemClock.now().0 > 1_700_000_000_000_000);
}

fn change(unit: UnitId, kind: RevisionKind, lifecycle: Lifecycle, payload: &[u8]) -> ModelChange {
    ModelChange {
        unit,
        kind,
        unit_type: "note".into(),
        schema_version: 1,
        lifecycle,
        restored_from: None,
        metadata: Vec::new(),
        payload: Some(payload.to_vec()),
    }
}

#[test]
fn unit_model_applies_and_answers_as_of() {
    let (a, b) = (UnitId([1; 16]), UnitId([2; 16]));
    let mut m = UnitModel::new();
    assert!(m.apply(CommitSeq(1), Purpose::Application, vec![]).unwrap().is_none());
    m.apply(
        CommitSeq(3),
        Purpose::Application,
        vec![
            change(a, RevisionKind::Create, Lifecycle::Active, b"a1"),
            change(b, RevisionKind::Create, Lifecycle::Active, b"b1"),
        ],
    )
    .unwrap();
    m.apply(CommitSeq(5), Purpose::Application, vec![change(a, RevisionKind::Update, Lifecycle::Active, b"a2")])
        .unwrap();
    m.apply(CommitSeq(8), Purpose::Application, vec![change(a, RevisionKind::Delete, Lifecycle::Deleted, b"")])
        .unwrap();
    let restore =
        ModelChange { restored_from: Some(RevNo(1)), ..change(a, RevisionKind::Restore, Lifecycle::Active, b"a1") };
    m.apply(CommitSeq(9), Purpose::Application, vec![restore]).unwrap();

    // Refusals leave the model unchanged.
    let before = m.clone();
    assert!(matches!(
        m.apply(CommitSeq(9), Purpose::Application, vec![change(b, RevisionKind::Update, Lifecycle::Active, b"")]),
        Err(ModelError::SeqNotIncreasing { .. })
    ));
    assert!(matches!(
        m.apply(CommitSeq(10), Purpose::Application, vec![change(b, RevisionKind::Create, Lifecycle::Active, b"")]),
        Err(ModelError::BadKind { .. })
    ));
    assert!(matches!(
        m.apply(
            CommitSeq(10),
            Purpose::Application,
            vec![change(UnitId([3; 16]), RevisionKind::Update, Lifecycle::Active, b"")]
        ),
        Err(ModelError::BadKind { .. })
    ));
    assert!(matches!(
        m.apply(CommitSeq(10), Purpose::Application, vec![change(b, RevisionKind::Restore, Lifecycle::Active, b"")]),
        Err(ModelError::BadRestore { .. })
    ));
    assert!(matches!(
        m.apply(
            CommitSeq(10),
            Purpose::Application,
            vec![
                change(b, RevisionKind::Update, Lifecycle::Active, b""),
                change(b, RevisionKind::Update, Lifecycle::Active, b"")
            ]
        ),
        Err(ModelError::DuplicateUnit { .. })
    ));
    assert_eq!(m, before);

    assert_eq!(m.last_seq(), CommitSeq(9));
    assert_eq!(m.head(a).unwrap().rev, RevNo(4));
    assert_eq!(m.history(a).iter().map(|r| r.commit_seq.0).collect::<Vec<_>>(), [3, 5, 8, 9]);
    assert_eq!(m.revision(a, RevNo(2)).unwrap().payload.as_deref(), Some(&b"a2"[..]));
    assert!(m.revision(a, RevNo(0)).is_none());
    assert_eq!(m.unit_as_of(a, CommitSeq(2)), ModelUnitAt::Absent);
    assert!(matches!(m.unit_as_of(a, CommitSeq(4)), ModelUnitAt::Present(r) if r.rev == RevNo(1)));
    assert!(matches!(m.unit_as_of(a, CommitSeq(8)), ModelUnitAt::DeletedAt(r) if r.rev == RevNo(3)));
    assert!(matches!(m.unit_as_of(a, CommitSeq(100)), ModelUnitAt::Present(r) if r.restored_from == Some(RevNo(1))));
    let since: Vec<u64> = m.change_sets_since(CommitSeq(3), 2).iter().map(|c| c.seq.0).collect();
    assert_eq!(since, [5, 8]);
    assert_eq!(m.change_set(CommitSeq(3)).unwrap().members.len(), 2);
    assert!(m.change_set(CommitSeq(4)).is_none());
    assert_eq!(m.unit_ids().collect::<Vec<_>>(), [a, b]);
}
