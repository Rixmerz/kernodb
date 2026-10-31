//! Structure vectors of the units layer (`docs/file-format/units-v1.md` §7).
//!
//! [`vectors`] builds every vector in memory: a positive vector is the
//! encoding of a record; a negative vector breaks exactly one `UNIT-n` rule
//! of a valid record and names the rule and class a reader must report. The
//! bytes are committed under `tests/vectors/v1/structs/<name>.bin` as
//! `key_len: u32be ‖ key ‖ value`. To regenerate after a layout change:
//!
//! ```text
//! cargo test -p kerno-units --test struct_vectors -- --ignored regenerate_struct_vectors
//! ```
//!
//! `spec_decoder` was written from the spec alone and shares no code with
//! `vector_encoder`; the tests below require both to agree with the table.

mod spec_decoder;
mod units_model;
mod vector_encoder;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use spec_decoder::Reject;
use units_model::{ChangeSet, Class, Hash, Head, Layer, Member, Rec, RevMeta, RevPayload};
use vector_encoder::encode;

const UNIT: [u8; 16] = [0xA0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const C: Class = Class::Corruption;
const I: Class = Class::IncompatibleLayer;
const RULES: u8 = 25;

enum Expect {
    Ok(Rec),
    Reject(Reject),
}

struct Vector {
    name: String,
    key: Vec<u8>,
    value: Vec<u8>,
    expect: Expect,
}

impl Vector {
    fn file_bytes(&self) -> Vec<u8> {
        let mut out = u32::try_from(self.key.len()).expect("key fits u32").to_be_bytes().to_vec();
        out.extend_from_slice(&self.key);
        out.extend_from_slice(&self.value);
        out
    }
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/v1/structs")
}

fn crc_hash(crc: u32) -> Hash {
    let mut digest = [0u8; 32];
    digest[..4].copy_from_slice(&crc.to_be_bytes());
    Hash { alg: 0, digest }
}

fn blake_hash() -> Hash {
    Hash { alg: 1, digest: std::array::from_fn(|i| u8::try_from(i * 7 + 1).expect("small")) }
}

fn layer() -> Layer {
    Layer { name: b"layer".to_vec(), layer_version: 1, history_policy: 2 }
}

fn head() -> Head {
    Head {
        unit_id: UNIT,
        unit_type: b"note".to_vec(),
        schema_version: 3,
        lifecycle: 1,
        head_rev: 3,
        oldest_rev: 1,
        created_seq: 41,
        updated_seq: 90,
        created_at: 1_761_000_000_000_000,
        updated_at: 1_761_000_500_000_000,
        content_hash: crc_hash(0xE3069283),
        payload_len: 11,
        payload_codec: 0,
        metadata: b"{\"k\":1}".to_vec(),
    }
}

fn rev(kind: u8, rev_no: u64, parent_rev: u64) -> RevMeta {
    RevMeta {
        unit_id: UNIT,
        rev_no,
        kind,
        parent_rev,
        commit_seq: rev_no.saturating_add(40),
        created_at: -5,
        schema_version: 3,
        content_hash: crc_hash(0xE3069283),
        payload_len: 11,
        payload_codec: 0,
        payload_present: 1,
        restored_from: 0,
        metadata: b"m".to_vec(),
        actor_ref: b"actor".to_vec(),
        source_ref: Vec::new(),
    }
}

fn changeset() -> ChangeSet {
    ChangeSet {
        commit_seq: 57,
        created_at: 1_761_000_000_000_000,
        member_count: 2,
        purpose: 0,
        reason: b"import".to_vec(),
        actor_ref: Vec::new(),
        source_ref: b"cli".to_vec(),
        correlation: Vec::new(),
    }
}

fn member() -> Member {
    Member { commit_seq: 57, ordinal: 1, unit_id: UNIT, rev_no: 2, kind: 2 }
}

fn pos(name: &str, rec: Rec) -> Vector {
    let (key, value) = encode(&rec);
    Vector { name: name.into(), key, value, expect: Expect::Ok(rec) }
}

/// A negative vector from a record whose fields already break `rule`.
fn neg_rec(name: &str, rule: u8, class: Class, rec: Rec) -> Vector {
    let (key, value) = encode(&rec);
    neg_bytes(name, rule, class, key, value)
}

fn neg_bytes(name: &str, rule: u8, class: Class, key: Vec<u8>, value: Vec<u8>) -> Vector {
    Vector { name: format!("neg_unit{rule}_{name}"), key, value, expect: Expect::Reject(Reject { rule, class }) }
}

fn vectors() -> Vec<Vector> {
    let mut v = vec![
        pos("layer_default", Rec::Layer(layer())),
        pos("head_active", Rec::Head(head())),
        pos(
            "head_min",
            Rec::Head(Head {
                unit_type: b"t".to_vec(),
                head_rev: 1,
                oldest_rev: 1,
                created_seq: 1,
                updated_seq: 1,
                lifecycle: 0,
                payload_len: 0,
                metadata: Vec::new(),
                ..head()
            }),
        ),
        pos(
            "head_limits",
            Rec::Head(Head {
                unit_type: vec![b'x'; 64],
                metadata: vec![0xEE; 4096],
                payload_len: 1 << 26,
                content_hash: blake_hash(),
                head_rev: u64::MAX,
                oldest_rev: u64::MAX,
                created_seq: 1,
                updated_seq: u64::MAX,
                ..head()
            }),
        ),
        pos("rev_meta_create", Rec::RevMeta(rev(1, 1, 0))),
        pos("rev_meta_update", Rec::RevMeta(rev(2, 2, 1))),
        pos("rev_meta_restore", Rec::RevMeta(RevMeta { restored_from: 1, ..rev(6, 4, 3) })),
        pos("rev_meta_delete", Rec::RevMeta(RevMeta { payload_present: 0, payload_len: 0, ..rev(5, 3, 2) })),
        pos(
            "rev_meta_limits",
            Rec::RevMeta(RevMeta {
                metadata: vec![1; 4096],
                actor_ref: vec![2; 256],
                source_ref: vec![3; 256],
                payload_len: 1 << 26,
                content_hash: blake_hash(),
                ..rev(2, u64::MAX, u64::MAX - 1)
            }),
        ),
        pos(
            "rev_payload_example",
            Rec::RevPayload(RevPayload { unit_id: UNIT, rev_no: 2, payload: b"hello world".to_vec() }),
        ),
        pos("rev_payload_empty", Rec::RevPayload(RevPayload { unit_id: UNIT, rev_no: 1, payload: Vec::new() })),
        pos("changeset_one", Rec::ChangeSet(changeset())),
        pos(
            "changeset_limits",
            Rec::ChangeSet(ChangeSet {
                member_count: 1 << 20,
                purpose: 2,
                reason: vec![b'r'; 1024],
                actor_ref: vec![b'a'; 1024],
                source_ref: vec![b's'; 1024],
                correlation: vec![b'c'; 1024],
                commit_seq: u64::MAX,
                ..changeset()
            }),
        ),
        pos("cs_member_one", Rec::Member(member())),
    ];

    // Key rules (§2).
    let hk = encode(&Rec::Head(head()));
    let (rk, rv) = encode(&Rec::RevMeta(rev(2, 2, 1)));
    let (ck, cv) = encode(&Rec::ChangeSet(changeset()));
    let (lk, lv) = encode(&Rec::Layer(layer()));
    v.push(neg_bytes("tag_0x02", 1, C, vec![0x02, 1], lv.clone()));
    v.push(neg_bytes("tag_0x32", 1, C, [vec![0x32], ck[1..].to_vec()].concat(), cv.clone()));
    v.push(neg_bytes("empty_key", 1, C, Vec::new(), lv.clone()));
    v.push(neg_bytes("tag_0x40", 2, I, [vec![0x40], ck[1..].to_vec()].concat(), cv.clone()));
    v.push(neg_bytes("tag_0xff", 2, I, vec![0xFF], Vec::new()));
    v.push(neg_bytes("head_key_short", 3, C, hk.0[..16].to_vec(), hk.1.clone()));
    v.push(neg_bytes("layer_name_empty", 3, C, vec![0x01], lv.clone()));
    v.push(neg_bytes("layer_name_65", 3, C, [vec![0x01], vec![b'n'; 65]].concat(), lv.clone()));
    v.push(neg_bytes("layer_name_not_utf8", 4, C, vec![0x01, 0xFF, 0xFE], lv.clone()));
    v.push(neg_rec("rev_meta_rev_no_0", 5, C, Rec::RevMeta(rev(2, 0, 1))));
    v.push(neg_bytes("changeset_seq_0", 5, C, [vec![0x30], vec![0; 8]].concat(), cv.clone()));
    v.push(neg_bytes("rev_payload_rev_no_0", 5, C, [vec![0x21], UNIT.to_vec(), vec![0; 8]].concat(), Vec::new()));

    // Envelope (§1.1).
    let patch = |bytes: &[u8], at: usize, to: u8| {
        let mut b = bytes.to_vec();
        b[at] = to;
        b
    };
    v.push(neg_bytes("version_0", 6, C, rk.clone(), patch(&rv, 0, 0)));
    v.push(neg_bytes("version_2", 7, I, rk.clone(), patch(&rv, 0, 2)));
    v.push(neg_bytes("version_2_rest_garbage", 7, I, rk.clone(), vec![2]));
    v.push(neg_bytes("flags_1", 8, C, rk.clone(), patch(&rv, 1, 1)));

    // Truncation, trailing bytes, length prefixes.
    v.push(neg_bytes("empty_value", 9, C, hk.0.clone(), Vec::new()));
    v.push(neg_bytes("head_cut_last_byte", 9, C, hk.0.clone(), hk.1[..hk.1.len() - 1].to_vec()));
    v.push(neg_bytes("rev_meta_cut_in_hash", 9, C, rk.clone(), rv[..40].to_vec()));
    v.push(neg_bytes(
        "member_cut",
        9,
        C,
        encode(&Rec::Member(member())).0,
        encode(&Rec::Member(member())).1[..20].to_vec(),
    ));
    v.push(neg_bytes("changeset_trailing", 10, C, ck.clone(), [cv.clone(), vec![0]].concat()));
    v.push(neg_bytes("layer_trailing", 10, C, lk.clone(), [lv.clone(), vec![0, 0]].concat()));
    v.push(neg_rec("metadata_4097", 11, C, Rec::Head(Head { metadata: vec![0; 4097], ..head() })));
    v.push(neg_rec("actor_ref_257", 11, C, Rec::RevMeta(RevMeta { actor_ref: vec![0; 257], ..rev(2, 2, 1) })));
    v.push(neg_rec("correlation_1025", 11, C, Rec::ChangeSet(ChangeSet { correlation: vec![0; 1025], ..changeset() })));
    let huge = Rec::Head(Head { metadata: Vec::new(), ..head() });
    let (huge_key, mut huge_value) = encode(&huge);
    let n = huge_value.len();
    huge_value[n - 4..].copy_from_slice(&u32::MAX.to_be_bytes());
    v.push(neg_bytes("prefix_u32_max", 11, C, huge_key, huge_value));

    // Field values.
    v.push(neg_rec("unit_type_empty", 12, C, Rec::Head(Head { unit_type: Vec::new(), ..head() })));
    v.push(neg_rec("unit_type_not_utf8", 12, C, Rec::Head(Head { unit_type: vec![0xC3, 0x28], ..head() })));
    v.push(neg_rec("lifecycle_3", 13, C, Rec::Head(Head { lifecycle: 3, ..head() })));
    v.push(neg_rec("kind_0", 13, C, Rec::RevMeta(rev(0, 2, 1))));
    v.push(neg_rec("kind_7", 13, C, Rec::RevMeta(rev(7, 2, 1))));
    v.push(neg_rec("member_kind_7", 13, C, Rec::Member(Member { kind: 7, ..member() })));
    v.push(neg_rec("purpose_3", 13, C, Rec::ChangeSet(ChangeSet { purpose: 3, ..changeset() })));
    v.push(neg_rec("history_policy_3", 13, C, Rec::Layer(Layer { history_policy: 3, ..layer() })));
    v.push(neg_rec("alg_2", 13, C, Rec::Head(Head { content_hash: Hash { alg: 2, digest: [0; 32] }, ..head() })));
    v.push(neg_rec("payload_codec_1", 13, C, Rec::RevMeta(RevMeta { payload_codec: 1, ..rev(2, 2, 1) })));
    let mut padded = crc_hash(7);
    padded.digest[31] = 1;
    v.push(neg_rec("crc_padding_nonzero", 14, C, Rec::Head(Head { content_hash: padded, ..head() })));
    let mut padded_first = crc_hash(7);
    padded_first.digest[4] = 1;
    v.push(neg_rec("crc_padding_first_nonzero", 14, C, Rec::Head(Head { content_hash: padded_first, ..head() })));
    v.push(neg_rec("layer_version_0", 15, C, Rec::Layer(Layer { layer_version: 0, ..layer() })));
    v.push(neg_rec("layer_version_2", 16, I, Rec::Layer(Layer { layer_version: 2, ..layer() })));

    // Rules between fields.
    v.push(neg_rec("oldest_above_head", 17, C, Rec::Head(Head { oldest_rev: 4, ..head() })));
    v.push(neg_rec("oldest_zero", 17, C, Rec::Head(Head { oldest_rev: 0, ..head() })));
    v.push(neg_rec("created_after_updated", 18, C, Rec::Head(Head { created_seq: 91, ..head() })));
    v.push(neg_rec("created_seq_zero", 18, C, Rec::Head(Head { created_seq: 0, ..head() })));
    v.push(neg_rec("create_with_parent", 19, C, Rec::RevMeta(rev(1, 2, 1))));
    v.push(neg_rec("update_parent_is_self", 19, C, Rec::RevMeta(rev(2, 2, 2))));
    v.push(neg_rec("update_without_parent", 19, C, Rec::RevMeta(rev(2, 2, 0))));
    v.push(neg_rec("restore_without_source", 20, C, Rec::RevMeta(rev(6, 4, 3))));
    v.push(neg_rec("restore_from_self", 20, C, Rec::RevMeta(RevMeta { restored_from: 4, ..rev(6, 4, 3) })));
    v.push(neg_rec("update_with_restored_from", 20, C, Rec::RevMeta(RevMeta { restored_from: 1, ..rev(2, 2, 1) })));
    v.push(neg_rec("payload_present_2", 21, C, Rec::RevMeta(RevMeta { payload_present: 2, ..rev(2, 2, 1) })));
    v.push(neg_rec("delete_with_payload", 21, C, Rec::RevMeta(rev(5, 3, 2))));
    v.push(neg_rec("head_payload_len_over", 22, C, Rec::Head(Head { payload_len: (1 << 26) + 1, ..head() })));
    v.push(neg_rec(
        "rev_payload_len_over",
        22,
        C,
        Rec::RevMeta(RevMeta { payload_len: (1 << 26) + 1, ..rev(2, 2, 1) }),
    ));
    v.push(neg_rec("commit_seq_zero", 23, C, Rec::RevMeta(RevMeta { commit_seq: 0, ..rev(2, 2, 1) })));
    v.push(neg_rec(
        "member_count_over",
        24,
        C,
        Rec::ChangeSet(ChangeSet { member_count: (1 << 20) + 1, ..changeset() }),
    ));
    v.push(neg_rec("member_rev_no_0", 25, C, Rec::Member(Member { rev_no: 0, ..member() })));
    v
}

#[test]
fn spec_decoder_accepts_and_rejects_every_vector() {
    for v in vectors() {
        let got = spec_decoder::decode(&v.key, &v.value);
        match &v.expect {
            Expect::Ok(rec) => assert_eq!(got.as_ref(), Ok(rec), "{}", v.name),
            Expect::Reject(r) => assert_eq!(got.as_ref(), Err(r), "{}", v.name),
        }
    }
}

#[test]
fn every_rule_has_a_negative_vector_and_the_spec_lists_it() {
    let covered: BTreeSet<u8> = vectors()
        .iter()
        .filter_map(|v| match &v.expect {
            Expect::Reject(r) => Some(r.rule),
            Expect::Ok(_) => None,
        })
        .collect();
    let spec =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/file-format/units-v1.md"))
            .expect("the spec is in the repository");
    for rule in 1..=RULES {
        assert!(covered.contains(&rule), "no negative vector for UNIT-{rule}");
        assert!(spec.contains(&format!("| UNIT-{rule} |")), "UNIT-{rule} is not in the rules table of units-v1.md");
    }
    assert_eq!(covered.len(), usize::from(RULES));
}

#[test]
fn names_are_unique() {
    let names: BTreeSet<String> = vectors().into_iter().map(|v| v.name).collect();
    assert_eq!(names.len(), vectors().len());
}

#[test]
fn committed_files_are_the_generated_vectors() {
    let all = vectors();
    for v in &all {
        let path = dir().join(format!("{}.bin", v.name));
        let on_disk =
            std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run the regenerate test)", path.display()));
        assert!(on_disk == v.file_bytes(), "{} differs from the generated vector", v.name);
    }
    let expected: BTreeSet<String> = all.iter().map(|v| format!("{}.bin", v.name)).collect();
    let present: BTreeSet<String> = std::fs::read_dir(dir())
        .expect("vector directory")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(present, expected, "files not in the table (or the reverse)");
}

/// A prefix of a valid value is never accepted and never panics: every
/// field either fits or runs past the end (UNIT-9).
#[test]
fn every_truncation_of_a_positive_value_is_unit_9() {
    for v in vectors() {
        let Expect::Ok(rec) = &v.expect else { continue };
        if matches!(rec, Rec::RevPayload(_)) {
            continue;
        }
        for cut in 0..v.value.len() {
            let got = spec_decoder::decode(&v.key, &v.value[..cut]);
            assert_eq!(got, Err(Reject { rule: 9, class: C }), "{} cut at {cut}", v.name);
        }
    }
}

/// Decoding a flipped byte may succeed (a different valid value) or reject,
/// but it never panics.
#[test]
fn single_byte_flips_never_panic() {
    for v in vectors() {
        let Expect::Ok(_) = &v.expect else { continue };
        for at in 0..v.value.len().min(300) {
            let mut value = v.value.clone();
            value[at] ^= 0xFF;
            let _ = spec_decoder::decode(&v.key, &value);
        }
    }
}

#[test]
#[ignore = "writes the committed vectors"]
fn regenerate_struct_vectors() {
    std::fs::create_dir_all(dir()).expect("create the vector directory");
    for entry in std::fs::read_dir(dir()).expect("vector directory") {
        std::fs::remove_file(entry.expect("entry").path()).expect("remove a stale vector");
    }
    for v in vectors() {
        std::fs::write(dir().join(format!("{}.bin", v.name)), v.file_bytes()).expect("write a vector");
    }
}
