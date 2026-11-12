//! E8-04: persistence of vectors as derived data (ADR-0020, layer version 2,
//! `docs/file-format/units-v2.md`).
//!
//! Every test runs on both engines (`emulated_log` and `kerno_db`), except
//! `derived_sentinel_survival`, which reads the files of the database and so
//! needs the engine that has them. The numbers in the names are the criteria
//! of ADR-0020 ("Criterios de aceptación de E8-04"): `t01_…` is T1, and so on.
//! T5 (a binary built from the last commit before E8-04) and T25/T26 (a `git
//! diff` of the engine paths and of the manifests) are commands, not tests;
//! T6 simulates the old reader the way the ADR says.
//!
//! The compute function is a deterministic function of the sources, supplied
//! here; nothing depends on time and no model runs.

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::sync::Arc;

use harness::units::{Dump, Target, open_units, raw};
use kerno_core::contract::{Options, Snapshot};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::{CrashKind, OpKind, SimVfs, assert_dir_unchanged};
use kerno_units::codec::{
    Key, LAYER_VERSION, LayerMeta, MAX_LAYER_VERSION, Record, TAG_DERIVED_DESC, TAG_DERIVED_SRC, TAG_DERIVED_VEC,
};
use kerno_units::vector_store::{
    DerivedDecodeError, DerivedKey, DerivedRule, VectorDescriptor, VectorState, decode_src_entry, decode_vector,
    encode_src_entry, encode_vector, output_hash,
};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg() -> UnitConfig {
    cfg_at(1)
}

/// A configuration whose unit ids start after `start` (a reopened database
/// must not mint the ids of the session before).
fn cfg_at(start: u64) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(start)));
    c.engine = Options::small_for_tests();
    c
}

fn open_db<E: Target>(disk: &SimVfs) -> UnitDb<E> {
    open_units::<E>(disk.as_vfs(), &cfg()).expect("open")
}

fn open_dd<E: Target>(disk: &SimVfs) -> (DerivedDb<E>, ReopenReport) {
    DerivedDb::open_persisted(open_db::<E>(disk)).expect("open_persisted")
}

/// Opening this image as a derived database leaves the files as a plain open
/// of the same engine leaves them (the engine starts a log of its own when
/// it opens; the layer adds no write), and the store reads the same.
fn opening_adds_nothing<E: Target>(image: &kerno_testkit::vfs::DirImage, ctx: &str) {
    let plain = SimVfs::restore(image);
    let db = open_db::<E>(&plain);
    let store = dump_of(&db);
    drop(db);
    let derived = SimVfs::restore(image);
    let (dd, _) = open_dd::<E>(&derived);
    drop(dd);
    if let Some(d) = kerno_testkit::vfs::dir_difference(&plain.dump(), &derived.dump(), &["LOCK"]) {
        panic!("{ctx}: opening the derived layer wrote more than the engine does: {d}");
    }
    let db = open_db::<E>(&derived);
    assert_eq!(dump_of(&db), store, "{ctx}: the store changed");
}

fn params() -> BTreeMap<String, String> {
    BTreeMap::from([("dim".to_owned(), "4".to_owned()), ("norm".to_owned(), "l2".to_owned())])
}

fn recipe(version: u32, profile: &str) -> VectorRecipe {
    VectorRecipe { method: "embed".into(), version, profile: profile.into(), params: params() }
}

/// The vector of a payload: its length, the sum of its bytes, a constant and a
/// negative zero (which the format stores as a positive one).
fn embed(payload: &[u8]) -> Vec<f32> {
    vec![payload.len() as f32, payload.iter().map(|b| f32::from(*b)).sum(), 0.5, -0.0]
}

/// What `embed` becomes once stored: `-0.0` is `+0.0`.
fn stored(payload: &[u8]) -> Vec<f32> {
    let mut v = embed(payload);
    v[3] = 0.0;
    v
}

fn vcompute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<f32>> {
    inputs.first().map(|i| embed(&i.payload))
}

fn make<E: Target>(dd: &DerivedDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn edit<E: Target>(dd: &DerivedDb<E>, id: UnitId, payload: &[u8]) -> CommitInfo {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap()
}

fn build<E: Target>(dd: &DerivedDb<E>, unit: UnitId) -> DerivedId {
    dd.build_vector(unit, &recipe(1, "model-a"), ChangeMeta::default(), &vcompute).unwrap()
}

fn spec(unit: UnitId, rev: u64, payload: &[u8]) -> VectorSpec {
    VectorSpec {
        source: RevisionId { unit, rev: RevNo(rev) },
        method: "embed".into(),
        method_version: 1,
        profile: "model-a".into(),
        params: params(),
        vector: embed(payload),
    }
}

/// A database with one unit holding `payload` and its vector, closed.
fn fixture<E: Target>(disk: &SimVfs, payload: &[u8]) -> (UnitId, DerivedId) {
    let (dd, _) = open_dd::<E>(disk);
    let u = make(&dd, payload);
    let id = build(&dd, u);
    dd.close().unwrap();
    (u, id)
}

fn dump_of<E: Target>(db: &UnitDb<E>) -> Dump {
    raw(db)
}

fn scan_tag<E: Target>(db: &UnitDb<E>, tag: u8) -> Vec<(Vec<u8>, Vec<u8>)> {
    db.raw_snapshot().unwrap().scan(Bound::Included(&[tag][..]), Bound::Excluded(&[tag + 1][..]), usize::MAX).unwrap()
}

fn derived_keys<E: Target>(db: &UnitDb<E>) -> usize {
    [TAG_DERIVED_DESC, TAG_DERIVED_VEC, TAG_DERIVED_SRC].iter().map(|t| scan_tag(db, *t).len()).sum()
}

fn get_raw<E: Target>(db: &UnitDb<E>, k: &[u8]) -> Option<Vec<u8>> {
    db.raw_get(k).unwrap()
}

fn put_raw<E: Target>(db: &UnitDb<E>, k: &[u8], v: &[u8]) {
    db.raw_commit(vec![(k.to_vec(), Some(v.to_vec()))]).unwrap();
}

fn delete_raw<E: Target>(db: &UnitDb<E>, k: &[u8]) {
    db.raw_commit(vec![(k.to_vec(), None)]).unwrap();
}

fn desc_key(id: DerivedId) -> Vec<u8> {
    DerivedKey::Desc(id).encode()
}

fn vec_key(id: DerivedId) -> Vec<u8> {
    DerivedKey::Vec(id).encode()
}

fn src_key(unit: UnitId, rev: u64, id: DerivedId) -> Vec<u8> {
    DerivedKey::Src { unit, rev: RevNo(rev), id }.encode()
}

fn mark_version<E: Target>(db: &UnitDb<E>) -> Option<u32> {
    db.snapshot().unwrap().layer_version().unwrap()
}

fn rules<E: Target>(db: &UnitDb<E>) -> BTreeSet<String> {
    db.verify_units().unwrap().rules()
}

fn clean<E: Target>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().unwrap();
    assert!(r.is_clean(), "{ctx}: verify_units found {:?}", r.findings);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ------------------------------------------------------------ T1: vectors

/// The inputs of the committed vectors (`units-v2.md`, "Vectores").
fn unit_a() -> UnitId {
    UnitId(std::array::from_fn(|i| i as u8 + 1))
}

fn desc_v() -> VectorDescriptor {
    let source = RevisionId { unit: unit_a(), rev: RevNo(2) };
    let value = encode_vector(&[1.0, -2.5, 0.0, 3.75]).unwrap();
    VectorDescriptor {
        state: VectorState::Ready,
        source,
        method: "embed".into(),
        method_version: 3,
        profile: "model-a".into(),
        params: params(),
        input_hash: input_hash(&[SourceInput {
            id: source,
            content_hash: ContentHash::blake3(0, b""),
            payload: Vec::new(),
        }]),
        output_hash: output_hash(&value),
    }
}

/// One row of the document: bytes of a kind and what the decoder says.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    name: String,
    kind: &'static str,
    bytes: Vec<u8>,
    /// `ok`, or the id of the rule that rejects it.
    result: String,
}

fn decode_row(kind: &str, bytes: &[u8]) -> Result<(), DerivedRule> {
    let rule = |e: DerivedDecodeError| e.rule;
    match kind {
        "key" => DerivedKey::decode(bytes).map(drop).map_err(rule),
        "desc" => VectorDescriptor::decode(bytes).map(drop).map_err(rule),
        "vec" => decode_vector(bytes).map(drop).map_err(rule),
        "src" => decode_src_entry(bytes).map_err(rule),
        "mark" => LayerMeta::decode_upto(bytes, MAX_LAYER_VERSION).map(drop).map_err(|e| DerivedRule::Unit(e.rule)),
        "mark1" => LayerMeta::decode(bytes).map(drop).map_err(|e| DerivedRule::Unit(e.rule)),
        "changeset" => {
            ChangeSetRecord::decode_upto(bytes, MAX_LAYER_VERSION).map(drop).map_err(|e| DerivedRule::Unit(e.rule))
        }
        "changeset1" => ChangeSetRecord::decode(bytes).map(drop).map_err(|e| DerivedRule::Unit(e.rule)),
        other => panic!("unknown kind {other}"),
    }
}

fn result_of(kind: &str, bytes: &[u8]) -> String {
    match decode_row(kind, bytes) {
        Ok(()) => "ok".into(),
        Err(r) => r.id(),
    }
}

fn row(name: &str, kind: &'static str, bytes: Vec<u8>) -> Row {
    let result = result_of(kind, &bytes);
    Row { name: name.into(), kind, bytes, result }
}

fn changeset_derived() -> Vec<u8> {
    ChangeSetRecord {
        created_at: T0,
        member_count: 0,
        purpose: Purpose::Derived,
        reason: Vec::new(),
        actor_ref: Vec::new(),
        source_ref: Vec::new(),
        correlation: Vec::new(),
    }
    .encode()
    .unwrap()
}

/// Positive and negative vectors, produced by the production encoders and by
/// editing their bytes.
fn rows() -> Vec<Row> {
    let d = desc_v();
    let id = d.id();
    let good = d.encode().unwrap();
    let vec_good = encode_vector(&[1.0, -2.5, 0.0, 3.75]).unwrap();
    let mut out = vec![
        row("key_desc", "key", desc_key(id)),
        row("key_vec", "key", vec_key(id)),
        row("key_src", "key", src_key(unit_a(), 2, id)),
        row("desc_ready", "desc", good.clone()),
        row("desc_stale", "desc", VectorDescriptor { state: VectorState::Stale, ..d.clone() }.encode().unwrap()),
        row(
            "desc_superseded",
            "desc",
            VectorDescriptor { state: VectorState::Superseded, ..d.clone() }.encode().unwrap(),
        ),
        row("desc_no_params", "desc", VectorDescriptor { params: BTreeMap::new(), ..d.clone() }.encode().unwrap()),
        row("vec_dim4", "vec", vec_good.clone()),
        row("vec_dim1", "vec", encode_vector(&[0.5]).unwrap()),
        row("src_entry", "src", encode_src_entry()),
        row(
            "mark_v2",
            "mark",
            LayerMeta { layer_version: 2, history_policy: HistoryPolicy::Versioned }.encode().unwrap(),
        ),
        row("changeset_derived", "changeset", changeset_derived()),
    ];
    let mut key_short = desc_key(id);
    key_short.pop();
    out.push(row("key_dv3_short", "key", key_short));
    let mut key_src_short = src_key(unit_a(), 2, id);
    key_src_short.pop();
    out.push(row("key_dv3_src_short", "key", key_src_short));
    out.push(row("key_unit5_rev0", "key", src_key(unit_a(), 0, id)));
    let mut key_new = desc_key(id);
    key_new[0] = 0x63;
    out.push(row("key_unit2_reserved", "key", key_new));

    // Descriptor offsets: env 0..2, kind 2, state 3, unit 4..20, rev 20..28,
    // method len 28..32, "embed" 32..37, version 37..41, profile len 41..45.
    let edit = |at: usize, v: u8| {
        let mut b = good.clone();
        b[at] = v;
        b
    };
    out.push(row("desc_unit6_env0", "desc", edit(0, 0)));
    out.push(row("desc_unit7_env2", "desc", edit(0, 2)));
    out.push(row("desc_unit8_flags", "desc", edit(1, 1)));
    out.push(row("desc_dv1_kind0", "desc", edit(2, 0)));
    out.push(row("desc_dv1_kind2", "desc", edit(2, 2)));
    out.push(row("desc_dv2_state0", "desc", edit(3, 0)));
    out.push(row("desc_dv2_state4", "desc", edit(3, 4)));
    let mut rev0 = good.clone();
    rev0[20..28].fill(0);
    out.push(row("desc_dv4_rev0", "desc", rev0));
    let mut method_empty = good.clone();
    method_empty.splice(28..37, [0, 0, 0, 0]);
    out.push(row("desc_dv4_method_empty", "desc", method_empty));
    let mut method_long = good.clone();
    method_long.splice(28..37, [&65u32.to_be_bytes()[..], &[b'm'; 65][..]].concat());
    out.push(row("desc_dv4_method_65", "desc", method_long));
    out.push(row("desc_dv4_method_not_utf8", "desc", edit(32, 0xff)));
    let mut profile_empty = good.clone();
    profile_empty.splice(41..52, [0, 0, 0, 0]);
    out.push(row("desc_dv4_profile_empty", "desc", profile_empty));
    let count_at = 45 + 7;
    out.push(row("desc_dv5_count17", "desc", edit(count_at, 17)));
    let mut key_empty = good.clone();
    key_empty[count_at + 1..count_at + 5].fill(0);
    out.push(row("desc_dv5_key_empty", "desc", key_empty));
    // Params "dim"="4" then "norm"="l2": swap the first key's text so the order breaks.
    let mut unsorted = good.clone();
    let first_key_at = count_at + 5;
    unsorted[first_key_at] = b'z';
    out.push(row("desc_dv5_not_increasing", "desc", unsorted));
    // The second key "norm" becomes "dim": a repeated key.
    let mut repeated = good.clone();
    let second_key_at = first_key_at + 3 + 4 + 1 + 4;
    repeated.splice(second_key_at - 4..second_key_at + 4, [&3u32.to_be_bytes()[..], b"dim"].concat());
    out.push(row("desc_dv5_repeated_key", "desc", repeated));
    let mut value_long = good.clone();
    let value_len_at = first_key_at + 3;
    value_long.splice(value_len_at..value_len_at + 5, [&257u32.to_be_bytes()[..], &[b'v'; 257][..]].concat());
    out.push(row("desc_dv5_value_257", "desc", value_long));
    out.push(row("desc_dv5_count_255", "desc", edit(count_at, 255)));
    out.push(row("desc_unit9_truncated", "desc", good[..good.len() - 1].to_vec()));
    let mut trailing = good.clone();
    trailing.push(0);
    out.push(row("desc_unit10_trailing", "desc", trailing));

    // Vector: env 0..2, dim 2..6, elem_type 6, data 7..
    let vedit = |at: usize, v: u8| {
        let mut b = vec_good.clone();
        b[at] = v;
        b
    };
    let mut dim0 = vec_good.clone();
    dim0[2..6].fill(0);
    out.push(row("vec_dv6_dim0", "vec", dim0));
    let mut dim_big = vec_good.clone();
    dim_big[2..6].copy_from_slice(&4097u32.to_be_bytes());
    out.push(row("vec_dv6_dim4097", "vec", dim_big));
    out.push(row("vec_dv6_length", "vec", vec_good[..vec_good.len() - 1].to_vec()));
    out.push(row("vec_dv7_elem2", "vec", vedit(6, 2)));
    let mut nan = vec_good.clone();
    nan[7..11].copy_from_slice(&f32::NAN.to_bits().to_be_bytes());
    out.push(row("vec_dv8_nan", "vec", nan));
    let mut inf = vec_good.clone();
    inf[7..11].copy_from_slice(&f32::INFINITY.to_bits().to_be_bytes());
    out.push(row("vec_dv8_inf", "vec", inf));
    let mut neg_zero = vec_good.clone();
    neg_zero[15..19].copy_from_slice(&0x8000_0000u32.to_be_bytes());
    out.push(row("vec_dv8_negative_zero", "vec", neg_zero));
    out.push(row("vec_unit9_short", "vec", vec_good[..5].to_vec()));
    out.push(row("vec_unit7_env2", "vec", vedit(0, 2)));

    out.push(row("src_unit10_trailing", "src", vec![1, 0, 0]));
    out.push(row("src_unit8_flags", "src", vec![1, 1]));
    out.push(row("src_unit9_empty", "src", Vec::new()));

    out.push(row("mark_unit16_v3", "mark", vec![1, 0, 0, 0, 0, 3, 2]));
    out.push(row("mark_v1_reader_unit16", "mark1", vec![1, 0, 0, 0, 0, 2, 2]));
    out.push(row("mark_unit15_v0", "mark", vec![1, 0, 0, 0, 0, 0, 2]));
    out.push(row("changeset_v1_reader_unit13", "changeset1", changeset_derived()));
    out
}

const DOC: &str = include_str!("../../../docs/file-format/units-v2.md");

/// The rows of the document: `| \`name\` | kind | hex | result |`.
fn doc_rows() -> BTreeMap<String, (String, String, String)> {
    let mut out = BTreeMap::new();
    // Only the table of section 8 lists vectors.
    let table = DOC.split_once("| Nombre | Tipo |").map_or("", |(_, t)| t);
    for line in table.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        if cells.len() == 6 && cells[1].starts_with('`') && cells[1].ends_with('`') {
            let name = cells[1].trim_matches('`').to_owned();
            let prev = out.insert(
                name.clone(),
                (cells[2].to_owned(), cells[3].trim_matches('`').to_owned(), cells[4].to_owned()),
            );
            assert!(prev.is_none(), "vector {name} listed twice in the document");
        }
    }
    out
}

/// `PRINT_DOC_VECTORS=1 cargo test … -- --nocapture t01` prints the table.
#[test]
fn print_doc_vectors() {
    if std::env::var_os("PRINT_DOC_VECTORS").is_some() {
        for r in rows() {
            println!("| `{}` | {} | `{}` | {} |", r.name, r.kind, hex(&r.bytes), r.result);
        }
    }
}

/// T1: the bytes of the document are the bytes of the production codec, and
/// every positive value decodes and encodes back to the same bytes.
#[test]
fn t01_vectors_of_the_document_match_the_codec_byte_for_byte() {
    let doc = doc_rows();
    let rows = rows();
    let names: BTreeSet<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names.len(), rows.len(), "duplicate vector names");
    assert_eq!(
        doc.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        names,
        "document and test list different vectors"
    );
    for r in &rows {
        let (kind, bytes, result) = &doc[&r.name];
        assert_eq!(kind, r.kind, "{}", r.name);
        assert_eq!(*bytes, hex(&r.bytes), "{}: the document's bytes differ", r.name);
        assert_eq!(*result, r.result, "{}: the document's result differs", r.name);
        // Every vector of the rows is also decoded from the document's own hex.
        assert_eq!(result_of(kind_of(kind), &unhex(bytes)), r.result, "{}", r.name);
        if r.result == "ok" {
            assert_encodes_back(r);
        }
    }
    // Positive vectors exist for every record type, negative ones for every rule.
    for kind in ["key", "desc", "vec", "src", "mark", "changeset"] {
        assert!(rows.iter().any(|r| r.kind == kind && r.result == "ok"), "no positive vector of {kind}");
    }
    let rejected: BTreeSet<&str> = rows.iter().map(|r| r.result.as_str()).collect();
    for want in [
        "DV-1", "DV-2", "DV-3", "DV-4", "DV-5", "DV-6", "DV-7", "DV-8", "UNIT-2", "UNIT-5", "UNIT-6", "UNIT-7",
        "UNIT-8", "UNIT-9", "UNIT-10", "UNIT-13", "UNIT-15", "UNIT-16",
    ] {
        assert!(rejected.contains(want), "no negative vector for {want}");
    }
}

fn kind_of(k: &str) -> &'static str {
    ["key", "desc", "vec", "src", "mark", "mark1", "changeset", "changeset1"]
        .into_iter()
        .find(|c| *c == k)
        .unwrap_or_else(|| panic!("unknown kind {k}"))
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn assert_encodes_back(r: &Row) {
    match r.kind {
        "key" => assert_eq!(DerivedKey::decode(&r.bytes).unwrap().encode(), r.bytes, "{}", r.name),
        "desc" => assert_eq!(VectorDescriptor::decode(&r.bytes).unwrap().encode().unwrap(), r.bytes, "{}", r.name),
        "vec" => assert_eq!(encode_vector(&decode_vector(&r.bytes).unwrap()).unwrap(), r.bytes, "{}", r.name),
        "src" => assert_eq!(encode_src_entry(), r.bytes, "{}", r.name),
        "mark" => {
            let m = LayerMeta::decode_upto(&r.bytes, MAX_LAYER_VERSION).unwrap();
            assert_eq!(m.encode().unwrap(), r.bytes, "{}", r.name);
        }
        "changeset" => {
            let c = ChangeSetRecord::decode_upto(&r.bytes, MAX_LAYER_VERSION).unwrap();
            assert_eq!(c.encode().unwrap(), r.bytes, "{}", r.name);
        }
        other => panic!("{other}"),
    }
}

/// The document's constants: derived id, input hash and output hash of the
/// first descriptor are spelled out so they can be checked without the codec.
#[test]
fn t01_hashes_of_the_document() {
    let d = desc_v();
    let value = encode_vector(&[1.0, -2.5, 0.0, 3.75]).unwrap();
    assert_eq!(output_hash(&value), *blake3::hash(&value).as_bytes());
    for (label, bytes) in [
        ("derived_id", d.id().0.to_vec()),
        ("input_hash", d.input_hash.to_vec()),
        ("output_hash", d.output_hash.to_vec()),
        ("vector_value", value),
    ] {
        let line = DOC
            .lines()
            .find(|l| l.starts_with(&format!("| `{label}` |")))
            .unwrap_or_else(|| panic!("the document has no line for {label}"));
        assert!(line.contains(&hex(&bytes)), "{label}: the document's bytes differ; expected {}", hex(&bytes));
    }
}

/// An independent identity, written from §C.1 alone (no `F(·)` helper shared
/// with the code under test).
#[test]
fn t03_identity_follows_the_specification() {
    let d = desc_v();
    let mut pre = b"kerno.derived.identity.v2".to_vec();
    let mut f = |x: &[u8]| {
        pre.extend_from_slice(&(x.len() as u64).to_le_bytes());
        pre.extend_from_slice(x);
    };
    f(&1u64.to_le_bytes());
    f(&d.source.unit.0);
    f(&d.source.rev.0.to_le_bytes());
    f(d.method.as_bytes());
    f(&d.method_version.to_le_bytes());
    f(d.profile.as_bytes());
    f(&(d.params.len() as u64).to_le_bytes());
    for (k, v) in &d.params {
        f(k.as_bytes());
        f(v.as_bytes());
    }
    assert_eq!(d.id().0, *blake3::hash(&pre).as_bytes());
    // The input hash of §C.2 has no length prefix and is little-endian.
    let mut input = b"kerno.derived.input.v1".to_vec();
    input.extend_from_slice(&1u64.to_le_bytes());
    input.extend_from_slice(&d.source.unit.0);
    input.extend_from_slice(&d.source.rev.0.to_le_bytes());
    assert_eq!(d.input_hash, *blake3::hash(&input).as_bytes());
    // The profile is part of the identity.
    let other = VectorDescriptor { profile: "model-b".into(), ..d.clone() };
    assert_ne!(d.id(), other.id());
}

// ----------------------------------------------------- T2: spec decoder

/// A second decoder, written only from ADR-0020 §A–§D. It returns the id of
/// the first rule broken.
mod spec {
    pub struct Desc {
        pub state: u8,
        pub unit: [u8; 16],
        pub rev: u64,
        pub method: String,
        pub method_version: u32,
        pub profile: String,
        pub params: Vec<(String, String)>,
        pub input_hash: [u8; 32],
        pub output_hash: [u8; 32],
    }

    type R<T> = Result<T, &'static str>;

    struct Cur<'a>(&'a [u8], usize);

    impl<'a> Cur<'a> {
        fn take(&mut self, n: usize) -> R<&'a [u8]> {
            if self.0.len() - self.1 < n {
                return Err("UNIT-9");
            }
            self.1 += n;
            Ok(&self.0[self.1 - n..self.1])
        }
        fn u8(&mut self) -> R<u8> {
            Ok(self.take(1)?[0])
        }
        fn u32(&mut self) -> R<u32> {
            Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
        }
        fn u64(&mut self) -> R<u64> {
            Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
        }
        fn envelope(&mut self) -> R<()> {
            match self.u8()? {
                0 => return Err("UNIT-6"),
                1 => {}
                _ => return Err("UNIT-7"),
            }
            if self.u8()? != 0 {
                return Err("UNIT-8");
            }
            Ok(())
        }
        fn text(&mut self, max: u32, empty_ok: bool, rule: &'static str) -> R<String> {
            let n = self.u32()?;
            if n > max || (n == 0 && !empty_ok) {
                return Err(rule);
            }
            String::from_utf8(self.take(n as usize)?.to_vec()).map_err(|_| rule)
        }
        fn end(&self) -> R<()> {
            if self.1 == self.0.len() { Ok(()) } else { Err("UNIT-10") }
        }
    }

    pub fn desc(b: &[u8]) -> R<Desc> {
        let mut c = Cur(b, 0);
        c.envelope()?;
        if c.u8()? != 1 {
            return Err("DV-1");
        }
        let state = c.u8()?;
        if !(1..=3).contains(&state) {
            return Err("DV-2");
        }
        let unit: [u8; 16] = c.take(16)?.try_into().unwrap();
        let rev = c.u64()?;
        if rev == 0 {
            return Err("DV-4");
        }
        let method = c.text(64, false, "DV-4")?;
        let method_version = c.u32()?;
        let profile = c.text(64, false, "DV-4")?;
        let n = c.u8()?;
        if n > 16 {
            return Err("DV-5");
        }
        let mut params: Vec<(String, String)> = Vec::new();
        for _ in 0..n {
            let k = c.text(64, false, "DV-5")?;
            let v = c.text(256, true, "DV-5")?;
            if params.last().is_some_and(|(p, _)| p.as_bytes() >= k.as_bytes()) {
                return Err("DV-5");
            }
            params.push((k, v));
        }
        let input_hash: [u8; 32] = c.take(32)?.try_into().unwrap();
        let output_hash: [u8; 32] = c.take(32)?.try_into().unwrap();
        c.end()?;
        Ok(Desc { state, unit, rev, method, method_version, profile, params, input_hash, output_hash })
    }

    pub fn vector(b: &[u8]) -> R<Vec<u32>> {
        let mut c = Cur(b, 0);
        c.envelope()?;
        let dim = c.u32()?;
        if dim == 0 || dim > 4096 {
            return Err("DV-6");
        }
        if c.u8()? != 1 {
            return Err("DV-7");
        }
        if b.len() != 7 + 4 * dim as usize {
            return Err("DV-6");
        }
        let mut out = Vec::new();
        for _ in 0..dim {
            let bits = c.u32()?;
            if (bits >> 23) & 0xff == 0xff || bits == 0x8000_0000 {
                return Err("DV-8");
            }
            out.push(bits);
        }
        Ok(out)
    }

    pub fn src(b: &[u8]) -> R<()> {
        let mut c = Cur(b, 0);
        c.envelope()?;
        c.end()
    }

    pub fn key(b: &[u8]) -> R<(u8, Vec<u8>)> {
        let Some((&tag, rest)) = b.split_first() else { return Err("UNIT-1") };
        match tag {
            0x60 | 0x61 if b.len() != 33 => Err("DV-3"),
            0x62 if b.len() != 57 => Err("DV-3"),
            0x60..=0x62 if tag == 0x62 && rest[16..24] == [0; 8] => Err("UNIT-5"),
            0x60..=0x62 => Ok((tag, rest.to_vec())),
            _ => Err("UNIT-2"),
        }
    }

    pub fn mark(b: &[u8], max: u32) -> R<u32> {
        let mut c = Cur(b, 0);
        c.envelope()?;
        let v = c.u32()?;
        if v == 0 {
            return Err("UNIT-15");
        }
        if v > max {
            return Err("UNIT-16");
        }
        if c.u8()? > 2 {
            return Err("UNIT-13");
        }
        c.end()?;
        Ok(v)
    }
}

/// Edits of a value: a bit flipped in each byte, every truncation, an extra byte.
fn edits(good: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for i in 0..good.len() {
        for mask in [0x01u8, 0x80, 0xff] {
            let mut b = good.to_vec();
            b[i] ^= mask;
            out.push(b);
        }
        out.push(good[..i].to_vec());
    }
    let mut longer = good.to_vec();
    longer.push(0);
    out.push(longer);
    out
}

fn agree(kind: &str, bytes: &[u8]) {
    let prod = decode_row(kind, bytes).map_err(|r| r.id());
    match kind {
        "desc" => {
            let spec = spec::desc(bytes);
            assert_eq!(prod.as_ref().err().map(String::as_str), spec.as_ref().err().copied(), "desc {}", hex(bytes));
            if let (Ok(()), Ok(s)) = (&prod, &spec) {
                let p = VectorDescriptor::decode(bytes).unwrap();
                assert_eq!(p.state.to_byte(), s.state);
                assert_eq!((p.source.unit.0, p.source.rev.0), (s.unit, s.rev));
                assert_eq!(
                    (p.method.as_str(), p.method_version, p.profile.as_str()),
                    (s.method.as_str(), s.method_version, s.profile.as_str())
                );
                assert_eq!(p.params.into_iter().collect::<Vec<_>>(), s.params);
                assert_eq!((p.input_hash, p.output_hash), (s.input_hash, s.output_hash));
            }
        }
        "vec" => {
            let spec = spec::vector(bytes);
            assert_eq!(prod.as_ref().err().map(String::as_str), spec.as_ref().err().copied(), "vec {}", hex(bytes));
            if let Ok(bits) = spec {
                let p: Vec<u32> = decode_vector(bytes).unwrap().iter().map(|x| x.to_bits()).collect();
                assert_eq!(p, bits);
            }
        }
        "src" => assert_eq!(prod.err().as_deref(), spec::src(bytes).err()),
        "key" => {
            let spec = spec::key(bytes);
            assert_eq!(prod.as_ref().err().map(String::as_str), spec.as_ref().err().copied(), "key {}", hex(bytes));
            if let Ok((tag, rest)) = spec {
                assert_eq!(DerivedKey::decode(bytes).unwrap().encode(), [&[tag][..], &rest].concat());
            }
        }
        "mark" => assert_eq!(prod.err().as_deref(), spec::mark(bytes, 2).err(), "mark {}", hex(bytes)),
        "mark1" => assert_eq!(prod.err().as_deref(), spec::mark(bytes, 1).err(), "mark1 {}", hex(bytes)),
        _ => {}
    }
}

/// T2: the independent decoder agrees with the codec on rule, class and
/// content, over the vectors and over edits of their bytes.
#[test]
fn spec_decoder_agrees_derived_v2() {
    let mut checked = 0;
    for r in rows() {
        agree(r.kind, &r.bytes);
        checked += 1;
        if r.result == "ok" {
            for e in edits(&r.bytes) {
                agree(r.kind, &e);
                checked += 1;
            }
        }
    }
    // The classes: only UNIT-2, UNIT-7 and UNIT-16 are IncompatibleLayer.
    assert_eq!(DerivedRule::Dv(3).class(), ErrorClass::Corruption);
    assert_eq!(DerivedRule::Unit(UnitRule::Unit16).class(), ErrorClass::IncompatibleLayer);
    assert_eq!(DerivedRule::Unit(UnitRule::Unit2).class(), ErrorClass::IncompatibleLayer);
    assert!(checked > 1000, "{checked}");
}

/// A vector element that is `-0.0` is written as `+0.0`, and a non-finite
/// element or a bad dimension is refused by the writer (DV-6, DV-8).
#[test]
fn t01_writer_normalizes_negative_zero_and_refuses_bad_vectors() {
    let v = encode_vector(&[-0.0, 1.0]).unwrap();
    assert_eq!(decode_vector(&v).unwrap().iter().map(|x| x.to_bits()).collect::<Vec<_>>(), vec![0, 1.0f32.to_bits()]);
    assert_eq!(encode_vector(&[0.0, 1.0]).unwrap(), v);
    for bad in [vec![f32::NAN], vec![f32::INFINITY], vec![f32::NEG_INFINITY], vec![], vec![0.0; 4097]] {
        assert!(matches!(encode_vector(&bad), Err(UnitError::InvalidArgument(_))));
    }
    assert!(encode_vector(&vec![1.0; 4096]).is_ok());
    // Descriptor limits at the writer.
    let d = desc_v();
    for bad in [
        VectorDescriptor { method: String::new(), ..d.clone() },
        VectorDescriptor { method: "m".repeat(65), ..d.clone() },
        VectorDescriptor { profile: String::new(), ..d.clone() },
        VectorDescriptor { source: RevisionId { rev: RevNo(0), ..d.source }, ..d.clone() },
        VectorDescriptor { params: (0..17).map(|i| (format!("k{i:02}"), String::new())).collect(), ..d.clone() },
        VectorDescriptor { params: BTreeMap::from([(String::new(), "v".into())]), ..d.clone() },
        VectorDescriptor { params: BTreeMap::from([("k".into(), "v".repeat(257))]), ..d.clone() },
    ] {
        assert!(matches!(bad.encode(), Err(UnitError::InvalidArgument(_))), "{bad:?}");
    }
    let edge = VectorDescriptor {
        method: "m".repeat(64),
        profile: "p".repeat(64),
        params: (0..16).map(|i| (format!("{i:064}"), "v".repeat(256))).collect(),
        ..d
    };
    assert_eq!(VectorDescriptor::decode(&edge.encode().unwrap()).unwrap(), edge);
}

// ------------------------------------------- T3, T4: identity and output hash

fn t03_identity_mismatch_is_dv10_and_not_loaded<E: Target>() {
    for (what, patch) in [
        ("source_rev", Box::new(|v: &mut Vec<u8>| v[27] = 3) as Box<dyn Fn(&mut Vec<u8>)>),
        ("profile", Box::new(|v: &mut Vec<u8>| v[45] = b'X')),
    ] {
        let disk = SimVfs::new(3);
        let (_, id) = fixture::<E>(&disk, b"hello");
        let db = open_db::<E>(&disk);
        let mut v = get_raw(&db, &desc_key(id)).unwrap();
        patch(&mut v);
        put_raw(&db, &desc_key(id), &v);
        db.close().unwrap();
        let (dd, report) = open_dd::<E>(&disk);
        assert_eq!(dd.state(&id), None, "{what}: a descriptor with another identity is not loaded");
        assert_eq!(dd.vector(&id), None, "{what}");
        assert_eq!(report.served, 0, "{what}");
        assert!(report.findings.iter().any(|f| f.rule == DerivedRule::Dv(10)), "{what}: {:?}", report.findings);
        dd.close().unwrap();
        let db = open_db::<E>(&disk);
        assert!(rules(&db).contains("DV-10"), "{what}");
        db.close().unwrap();
    }
}

fn vector_hash_mismatch_is_not_served<E: Target>() {
    let disk = SimVfs::new(4);
    let (u, id) = fixture::<E>(&disk, b"hello");
    let db = open_db::<E>(&disk);
    // Another finite vector of the same dimension: it decodes, it does not hash.
    put_raw(&db, &vec_key(id), &encode_vector(&[9.0, 9.0, 9.0, 9.0]).unwrap());
    db.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(dd.state(&id), Some(DerivedState::Stale));
    assert_eq!(dd.vector(&id), None, "a vector that does not hash is never served");
    assert_eq!(report.demoted, vec![(id, StaleCause::VectorMismatch)]);
    assert!(report.findings.iter().any(|f| f.rule == DerivedRule::Dv(12)));
    // A rebuild recomputes it.
    let again = dd.rebuild_vector(&id, ChangeMeta::default(), &vcompute).unwrap();
    assert_eq!(again, id, "the source did not move: the same identity");
    assert_eq!(dd.vector(&again), Some(stored(b"hello")));
    let _ = u;
    dd.close().unwrap();
    clean(&open_db::<E>(&disk), "after the rebuild");
}

// ----------------------------------------------- T6, T7: old readers, no writes

fn layer1_reader_refuses_derived_fixture<E: Target>() {
    let disk = SimVfs::new(6);
    fixture::<E>(&disk, b"hello");
    let db = open_db::<E>(&disk);
    let pairs = raw(&db);
    let mark = pairs.iter().find(|(k, _)| k == &Key::LayerMeta { name: "layer".into() }.encode().unwrap()).unwrap();
    // The reader of layer 1 reads the mark first and refuses it: UNIT-16, with the version found.
    let e = LayerMeta::decode(&mark.1).unwrap_err();
    assert_eq!((e.rule, e.found_layer_version), (UnitRule::Unit16, Some(2)));
    assert_eq!(e.class(), ErrorClass::IncompatibleLayer);
    let as_error: UnitError = e.into();
    assert!(matches!(as_error, UnitError::IncompatibleLayer { found: 2, supported: 1 }), "{as_error:?}");
    assert_eq!(LAYER_VERSION, 1);
    // And every other derived record is unknown to it: nothing is misread.
    let mut derived = 0;
    for (k, v) in &pairs {
        if (0x60..=0x62).contains(&k[0]) {
            derived += 1;
            let e = Record::decode(k, v).unwrap_err();
            assert_eq!((e.rule, e.class()), (UnitRule::Unit2, ErrorClass::IncompatibleLayer));
        }
        if k[0] == 0x30 {
            // The change set of the vector has purpose 5, outside the table of layer 1.
            if let Err(e) = Record::decode(k, v) {
                assert_eq!(e.rule, UnitRule::Unit13);
            }
        }
    }
    assert_eq!(derived, 3);
    // This build reads the mark.
    assert_eq!(LayerMeta::decode_upto(&mark.1, MAX_LAYER_VERSION).unwrap().layer_version, 2);
    db.close().unwrap();
}

/// A database that holds no vector is of layer 1 and the reader of layer 1
/// opens it (the other half of the declaration).
fn a_database_without_vectors_stays_at_version_1<E: Target>() {
    let disk = SimVfs::new(7);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"x");
    edit(&dd, u, b"y");
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(mark_version(&db), Some(1));
    let mark = get_raw(&db, &Key::LayerMeta { name: "layer".into() }.encode().unwrap()).unwrap();
    assert_eq!(LayerMeta::decode(&mark).unwrap().layer_version, 1, "the layer 1 reader accepts it");
    assert_eq!(derived_keys(&db), 0);
    db.set_policy("doc", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    assert_eq!(mark_version(&db), Some(1), "no vector: still version 1");
    db.close().unwrap();
}

fn open_derived_fixture_writes_nothing<E: Target>() {
    let disk = SimVfs::new(8);
    let (u, _) = fixture::<E>(&disk, b"hello");
    let image = disk.dump();
    opening_adds_nothing::<E>(&image, "the fixture");
    let (dd, report) = open_dd::<E>(&SimVfs::restore(&image));
    assert_eq!(report.served, 1);
    assert!(report.demoted.is_empty() && report.findings.is_empty(), "{report:?}");
    assert!(dd.get_unit(u).unwrap().is_some());
    // A plain UnitDb open is a read, too, and verifies clean.
    let db = open_db::<E>(&SimVfs::restore(&image));
    assert!(db.verify_units().unwrap().is_clean());
}

fn unit_id(n: u64) -> UnitId {
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&n.to_be_bytes());
    UnitId(id)
}

// --------------------------------------------- T8, T10: crash sweeps

fn sweep_cfg() -> UnitConfig {
    cfg()
}

/// Opens, writes `steps` and returns on the first error (the crash).
fn first_vector_scenario<E: Target>(disk: &SimVfs) -> Option<()> {
    let db = open_units::<E>(disk.as_vfs(), &sweep_cfg()).ok()?;
    let (dd, _) = DerivedDb::open_persisted(db).ok()?;
    let mut cs = dd.begin_changeset(ChangeMeta::default()).ok()?;
    let u = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"hello".to_vec() })
        .ok()?;
    cs.commit().ok()?;
    dd.build_vector(u, &recipe(1, "model-a"), ChangeMeta::default(), &vcompute).ok()?;
    dd.close().ok()
}

fn count_points<E: Target>(scenario: fn(&SimVfs) -> Option<()>) -> u64 {
    let dry = SimVfs::new(0);
    assert!(scenario(&dry).is_some(), "the dry run must complete");
    dry.mutating_ops() + 5
}

fn check_mark_atomic<E: Target>(image: &SimVfs, ctx: &str) {
    let db = open_units::<E>(image.as_vfs(), &cfg()).unwrap_or_else(|e| panic!("{ctx}: recovery: {e}"));
    let n = [TAG_DERIVED_DESC, TAG_DERIVED_VEC, TAG_DERIVED_SRC].map(|t| scan_tag(&db, t).len());
    let mark = mark_version(&db);
    match n {
        [0, 0, 0] => assert!(matches!(mark, None | Some(1)), "{ctx}: no derived key but the mark is {mark:?}"),
        [1, 1, 1] => assert_eq!(mark, Some(2), "{ctx}: derived keys with the mark {mark:?}"),
        other => panic!("{ctx}: descriptor, vector and entry are not all there or all absent: {other:?}"),
    }
    clean(&db, ctx);
    let (dd, report) = DerivedDb::open_persisted(db).unwrap_or_else(|e| panic!("{ctx}: {e}"));
    assert!(report.findings.is_empty() && report.demoted.is_empty(), "{ctx}: {report:?}");
    assert_eq!(report.served, n[0], "{ctx}");
    dd.close().unwrap();
}

fn derived_mark_every_point<E: Target>(kind: CrashKind) {
    let total = count_points::<E>(first_vector_scenario::<E>);
    for point in 1..=total {
        let ctx = format!("{} {kind:?}: crash before mutating op #{point} of {total}", E::NAME);
        let disk = SimVfs::new(point);
        disk.crash_after(point, kind);
        let _ = first_vector_scenario::<E>(&disk);
        let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(kind));
        check_mark_atomic::<E>(&image, &ctx);
    }
}

fn update_scenario<E: Target>(disk: &SimVfs) -> Option<()> {
    let db = open_units::<E>(disk.as_vfs(), &sweep_cfg()).ok()?;
    let (dd, _) = DerivedDb::open_persisted(db).ok()?;
    let mut cs = dd.begin_changeset(ChangeMeta::default()).ok()?;
    let u = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"hello".to_vec() })
        .ok()?;
    cs.commit().ok()?;
    dd.build_vector(u, &recipe(1, "model-a"), ChangeMeta::default(), &vcompute).ok()?;
    let mut cs = dd.begin_changeset(ChangeMeta::default()).ok()?;
    cs.update(u, UnitUpdate { payload: b"hello2".to_vec(), schema_version: None, metadata: None }, None).ok()?;
    cs.commit().ok()?;
    dd.close().ok()
}

/// No `Ready` descriptor has a source other than the head of its unit (I-23, I-25, I-38).
fn check_no_ready_on_an_old_source<E: Target>(image: &SimVfs, ctx: &str) {
    let db = open_units::<E>(image.as_vfs(), &cfg()).unwrap_or_else(|e| panic!("{ctx}: recovery: {e}"));
    for (_, v) in scan_tag(&db, TAG_DERIVED_DESC) {
        let d = VectorDescriptor::decode(&v).unwrap();
        let head = db.get_unit(d.source.unit).unwrap().expect("the source exists").head;
        if d.state == VectorState::Ready {
            assert_eq!(
                head, d.source.rev,
                "{ctx}: a Ready descriptor over revision {} while the head is {head}",
                d.source.rev
            );
        }
        // A vector is persisted only with a Ready descriptor.
        assert_eq!(get_raw(&db, &vec_key(d.id())).is_some(), d.state == VectorState::Ready, "{ctx}");
    }
    clean(&db, ctx);
    let (dd, report) = DerivedDb::open_persisted(db).unwrap();
    assert!(report.demoted.is_empty() && report.findings.is_empty(), "{ctx}: {report:?}");
    dd.close().unwrap();
}

fn stale_mark_every_point<E: Target>(kind: CrashKind) {
    let total = count_points::<E>(update_scenario::<E>);
    for point in 1..=total {
        let ctx = format!("{} {kind:?}: crash before mutating op #{point} of {total}", E::NAME);
        let disk = SimVfs::new(point);
        disk.crash_after(point, kind);
        let _ = update_scenario::<E>(&disk);
        let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(kind));
        check_no_ready_on_an_old_source::<E>(&image, &ctx);
    }
}

// ----------------------------------------------- T11: a failed commit

fn a_commit_that_fails_marks_nothing<E: Target>() {
    let disk = SimVfs::new(21);
    let (dd, _) = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a"), make(&dd, b"b"));
    let on_a = build(&dd, a);
    let on_b = build(&dd, b);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(a, UnitUpdate { payload: b"a2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    disk.fail_when(|op| op.kind == OpKind::SyncData, 1, None);
    let result = cs.commit();
    assert!(disk.fault_injected(), "the fault did not fire");
    assert!(result.is_err(), "a failed data sync went unreported");
    assert_eq!(dd.state(&on_a), Some(DerivedState::Ready));
    assert_eq!(dd.state(&on_b), Some(DerivedState::Ready));
    assert_eq!(dd.vector(&on_a), Some(stored(b"a")));
    // The disk after a power loss: whole change set or none; here the sync failed, so none of it
    // may be partly there: the descriptor is not Stale unless the unit moved too.
    let image = disk.crash_now(CrashKind::PowerLoss);
    let db = open_db::<E>(&image);
    let head = db.get_unit(a).unwrap().unwrap().head;
    let d = VectorDescriptor::decode(&get_raw(&db, &desc_key(on_a)).unwrap()).unwrap();
    assert_eq!(d.state == VectorState::Ready, head == RevNo(1), "the stale mark and the source change are one commit");
    assert_eq!(get_raw(&db, &vec_key(on_a)).is_some(), d.state == VectorState::Ready);
    assert_eq!(VectorDescriptor::decode(&get_raw(&db, &desc_key(on_b)).unwrap()).unwrap().state, VectorState::Ready);
    clean(&db, "after the failed commit");
}

fn a_failed_first_vector_writes_no_derived_key_and_no_mark<E: Target>() {
    let disk = SimVfs::new(22);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"a");
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.put_vector(&spec(u, 1, b"a")).unwrap();
    disk.fail_when(|op| op.kind == OpKind::SyncData, 1, None);
    assert!(cs.commit().is_err());
    assert!(disk.fault_injected());
    assert_eq!(dd.vector(&DerivedId([0; 32])), None);
    let image = disk.crash_now(CrashKind::PowerLoss);
    let db = open_db::<E>(&image);
    let n = derived_keys(&db);
    match mark_version(&db) {
        Some(2) => assert_eq!(n, 3),
        _ => assert_eq!(n, 0),
    }
    clean(&db, "after the failed first vector");
}

// ----------------------------------------- T9: the mark never goes down

fn mark_never_lowers<E: Target>() {
    let disk = SimVfs::new(9);
    let (u, id) = fixture::<E>(&disk, b"hello");
    let db = open_units::<E>(disk.as_vfs(), &cfg_at(100)).unwrap();
    assert_eq!(mark_version(&db), Some(2));
    // set_policy, an update, an archive, a delete: none writes a lower version.
    db.set_policy("doc", HistoryPolicy::Audit, ChangeMeta::default()).unwrap();
    assert_eq!(mark_version(&db), Some(2));
    let policy = get_raw(&db, &Key::LayerMeta { name: "policy/doc".into() }.encode().unwrap()).unwrap();
    assert_eq!(
        LayerMeta::decode_upto(&policy, MAX_LAYER_VERSION).unwrap().layer_version,
        2,
        "the policy record is not lower"
    );
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(u, UnitUpdate { payload: b"x".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(mark_version(&db), Some(2));
    // The history policy of the mark is kept when a later change set raises the version.
    db.create_unit(
        NewUnit { unit_type: "other".into(), schema_version: 1, metadata: vec![], payload: vec![1] },
        ChangeMeta::default(),
    )
    .unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(u, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(derived_keys(&db), 0, "the delete took every derived record");
    assert_eq!(mark_version(&db), Some(2), "a database that had a vector stays at version 2");
    let _ = id;
    db.close().unwrap();
    // It reopens, and a vector written later needs no second raise.
    let dd = DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg_at(200)).unwrap()).unwrap().0;
    let v = make(&dd, b"again");
    build(&dd, v);
    dd.close().unwrap();
    assert_eq!(mark_version(&open_db::<E>(&disk)), Some(2));
}

/// The first change set of a new database that already persists a vector
/// writes the mark at version 2 directly, and keeps the configured policy.
fn a_new_database_with_a_first_vector_starts_at_version_2<E: Target>() {
    let disk = SimVfs::new(10);
    let mut c = cfg();
    c.default_policy = HistoryPolicy::Audit;
    let db = open_units::<E>(disk.as_vfs(), &c).unwrap();
    let (dd, _) = DerivedDb::open_persisted(db).unwrap();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let u = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"p".to_vec() })
        .unwrap();
    let id = cs.put_vector(&spec(u, 1, b"p")).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Ready));
    dd.close().unwrap();
    let db = open_units::<E>(disk.as_vfs(), &c).unwrap();
    assert_eq!(mark_version(&db), Some(2));
    assert_eq!(db.policy("anything").unwrap(), HistoryPolicy::Audit);
    clean(&db, "new database");
    // An upgrade of an existing database keeps its policy too.
    let disk2 = SimVfs::new(11);
    let db = open_units::<E>(disk2.as_vfs(), &c).unwrap();
    let (first, _) = db
        .create_unit(
            NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"p".to_vec() },
            ChangeMeta::default(),
        )
        .unwrap();
    db.close().unwrap();
    let mut other = cfg();
    other.default_policy = HistoryPolicy::None;
    let db = open_units::<E>(disk2.as_vfs(), &other).unwrap();
    let (dd, _) = DerivedDb::open_persisted(db).unwrap();
    dd.build_vector(first, &recipe(1, "model-a"), ChangeMeta::default(), &vcompute).unwrap();
    dd.close().unwrap();
    let db = open_units::<E>(disk2.as_vfs(), &other).unwrap();
    assert_eq!(mark_version(&db), Some(2));
    assert_eq!(db.policy("doc").unwrap(), HistoryPolicy::Audit, "the policy of the old mark, not the new config's");
}

// ----------------------------------- T13, T14, T15, T16: reopen and loss

/// T13: a rebuild after a source change gives the identity, the hashes and the
/// bytes of a build over the same sources.
fn rebuilt_equals_fresh<E: Target>() {
    let disk = SimVfs::new(13);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"v1");
    let first = build(&dd, u);
    edit(&dd, u, b"v2");
    assert_eq!(dd.state(&first), Some(DerivedState::Stale));
    assert_eq!(dd.vector(&first), None);
    let rebuilt = dd.rebuild_vector(&first, ChangeMeta::default(), &vcompute).unwrap();
    assert_ne!(rebuilt, first, "the source moved to revision 2: another identity");
    assert_eq!(dd.state(&first), None, "the old entry is replaced");
    assert_eq!(dd.state(&rebuilt), Some(DerivedState::Ready));
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    let rebuilt_records = records_of(&db, rebuilt);
    clean(&db, "after the rebuild");
    db.close().unwrap();

    let fresh_disk = SimVfs::new(14);
    let (dd, _) = open_dd::<E>(&fresh_disk);
    let u2 = make(&dd, b"v1");
    edit(&dd, u2, b"v2");
    let fresh = build(&dd, u2);
    dd.close().unwrap();
    let db = open_db::<E>(&fresh_disk);
    assert_eq!(u, u2);
    assert_eq!(fresh, rebuilt, "same derived_id");
    assert_eq!(
        records_of(&db, fresh),
        rebuilt_records,
        "same descriptor (input_hash, output_hash) and vector bytes and entry"
    );
}

type Records = (Vec<u8>, Vec<u8>, Vec<u8>);

fn records_of<E: Target>(db: &UnitDb<E>, id: DerivedId) -> Records {
    let d = VectorDescriptor::decode(&get_raw(db, &desc_key(id)).unwrap()).unwrap();
    (
        get_raw(db, &desc_key(id)).unwrap(),
        get_raw(db, &vec_key(id)).unwrap(),
        get_raw(db, &src_key(d.source.unit, d.source.rev.0, id)).unwrap(),
    )
}

fn derived_loss_leaves_units_unchanged<E: Target>() {
    let disk = SimVfs::new(15);
    let (dd, _) = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a"), make(&dd, b"b"));
    build(&dd, a);
    build(&dd, b);
    edit(&dd, b, b"b2");
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    let without = |d: Dump| -> Dump { d.into_iter().filter(|(k, _)| !(0x60..=0x62).contains(&k[0])).collect() };
    let before = without(dump_of(&db));
    let reads: Vec<_> = [a, b].iter().map(|u| db.get_unit(*u).unwrap()).collect();
    for tag in [TAG_DERIVED_DESC, TAG_DERIVED_VEC, TAG_DERIVED_SRC] {
        for (k, _) in scan_tag(&db, tag) {
            delete_raw(&db, &k);
        }
    }
    assert_eq!(derived_keys(&db), 0);
    db.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(report.served, 0);
    assert!(report.findings.is_empty());
    assert_eq!([a, b].map(|u| dd.get_unit(u).unwrap()).to_vec(), reads);
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    // The units' records are the ones they were (the commits after the loss are the deletes only).
    assert_eq!(
        without(dump_of(&db)).into_iter().filter(|(k, _)| k[0] != 0x30).collect::<Dump>(),
        before.into_iter().filter(|(k, _)| k[0] != 0x30).collect::<Dump>()
    );
    clean(&db, "after the loss of every derived record");
}

type Damage<E> = Box<dyn Fn(&UnitDb<E>, UnitId, DerivedId)>;

struct Case<E: Target> {
    name: &'static str,
    damage: Damage<E>,
    /// `Some(state)` the registry holds after opening; `None` it holds nothing.
    state: Option<DerivedState>,
    served: bool,
    demoted: Option<StaleCause>,
    /// The rule `verify_units` reports (and the report of opening), if any.
    rule: Option<&'static str>,
    /// Whether the explicit repair writes something.
    repairs: bool,
}

fn cases<E: Target + 'static>() -> Vec<Case<E>> {
    vec![
        Case {
            name: "vector without descriptor",
            damage: Box::new(|db, _, id| delete_raw(db, &desc_key(id))),
            state: None,
            served: false,
            demoted: None,
            rule: Some("DV-9"),
            repairs: true,
        },
        Case {
            name: "descriptor without 0x62",
            damage: Box::new(|db, u, id| delete_raw(db, &src_key(u, 1, id))),
            state: Some(DerivedState::Ready),
            served: true,
            demoted: None,
            rule: Some("DV-9"),
            repairs: true,
        },
        Case {
            name: "0x62 entry of another revision",
            damage: Box::new(|db, u, id| {
                delete_raw(db, &src_key(u, 1, id));
                put_raw(db, &src_key(u, 9, id), &encode_src_entry());
            }),
            state: Some(DerivedState::Ready),
            served: true,
            demoted: None,
            rule: Some("DV-9"),
            repairs: true,
        },
        Case {
            name: "derived_id that does not match",
            damage: Box::new(|db, _, id| {
                let mut d = VectorDescriptor::decode(&get_raw(db, &desc_key(id)).unwrap()).unwrap();
                d.method_version += 1;
                put_raw(db, &desc_key(id), &d.encode().unwrap());
            }),
            state: None,
            served: false,
            demoted: None,
            rule: Some("DV-10"),
            repairs: true,
        },
        Case {
            name: "Ready without vector",
            damage: Box::new(|db, _, id| delete_raw(db, &vec_key(id))),
            state: Some(DerivedState::Stale),
            served: false,
            demoted: Some(StaleCause::VectorMissing),
            rule: Some("DV-12"),
            repairs: true,
        },
        Case {
            name: "output_hash that differs",
            damage: Box::new(|db, _, id| {
                let mut d = VectorDescriptor::decode(&get_raw(db, &desc_key(id)).unwrap()).unwrap();
                d.output_hash[0] ^= 1;
                put_raw(db, &desc_key(id), &d.encode().unwrap());
            }),
            state: Some(DerivedState::Stale),
            served: false,
            demoted: Some(StaleCause::VectorMismatch),
            rule: Some("DV-12"),
            repairs: true,
        },
        Case {
            name: "vector that does not decode",
            damage: Box::new(|db, _, id| {
                let mut v = get_raw(db, &vec_key(id)).unwrap();
                v[2] = 0xff;
                put_raw(db, &vec_key(id), &v);
            }),
            state: Some(DerivedState::Stale),
            served: false,
            demoted: Some(StaleCause::VectorUndecodable),
            rule: Some("DV-6"),
            repairs: true,
        },
        Case {
            name: "stale descriptor that keeps a vector",
            damage: Box::new(|db, _, id| {
                let mut d = VectorDescriptor::decode(&get_raw(db, &desc_key(id)).unwrap()).unwrap();
                d.state = VectorState::Stale;
                put_raw(db, &desc_key(id), &d.encode().unwrap());
            }),
            state: Some(DerivedState::Stale),
            served: false,
            demoted: None,
            rule: Some("DV-12"),
            repairs: true,
        },
    ]
}

fn reopen_refuses_or_stales_each_damage_case<E: Target + 'static>() {
    for case in cases::<E>() {
        let ctx = case.name;
        let disk = SimVfs::new(30);
        let (u, id) = fixture::<E>(&disk, b"hello");
        let db = open_db::<E>(&disk);
        (case.damage)(&db, u, id);
        db.close().unwrap();
        opening_adds_nothing::<E>(&disk.dump(), ctx);
        let (dd, report) = open_dd::<E>(&disk);
        assert_eq!(dd.state(&id), case.state, "{ctx}: registry state");
        assert_eq!(dd.vector(&id).is_some(), case.served, "{ctx}: served");
        assert_eq!(report.demoted.first().map(|d| d.1), case.demoted, "{ctx}: demotion");
        assert_eq!(report.served, usize::from(case.served), "{ctx}");
        let reported: BTreeSet<String> = report.findings.iter().map(|f| f.rule.id()).collect();
        assert_eq!(reported, case.rule.iter().map(|r| r.to_string()).collect(), "{ctx}: opening reports {report:?}");
        dd.close().unwrap();
        let db = open_db::<E>(&disk);
        assert_eq!(rules(&db), reported, "{ctx}: verify_units reports the same rules");
        db.close().unwrap();

        // The explicit repair: a Derived change set; afterwards nothing is reported.
        let (dd, _) = open_dd::<E>(&disk);
        let r = dd.repair_derived(ChangeMeta::default()).unwrap();
        assert_eq!(r.writes > 0, case.repairs, "{ctx}");
        dd.close().unwrap();
        let (dd, report) = open_dd::<E>(&disk);
        assert!(report.findings.is_empty(), "{ctx}: after the repair {report:?}");
        assert_eq!(
            dd.repair_derived(ChangeMeta::default()).unwrap().writes,
            0,
            "{ctx}: a repaired database needs none"
        );
        dd.close().unwrap();
        clean(&open_db::<E>(&disk), ctx);
    }
}

/// A damage case the table marks "ninguno": a `Ready` descriptor whose source
/// moved on is `Stale` at opening and is not a finding.
fn reopen_after_source_change_is_stale<E: Target>() {
    // Through the layer: the persisted state is Stale.
    let disk = SimVfs::new(31);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"hello");
    let id = build(&dd, u);
    edit(&dd, u, b"world");
    dd.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(dd.state(&id), Some(DerivedState::Stale));
    assert_eq!(dd.vector(&id), None);
    assert!(report.demoted.is_empty() && report.findings.is_empty(), "{report:?}");
    dd.close().unwrap();

    // Through the plain database: it marks the same way, in its own transaction.
    let disk = SimVfs::new(32);
    let (u, id) = fixture::<E>(&disk, b"hello");
    let db = open_db::<E>(&disk);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(u, UnitUpdate { payload: b"world".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(VectorDescriptor::decode(&get_raw(&db, &desc_key(id)).unwrap()).unwrap().state, VectorState::Stale);
    assert!(get_raw(&db, &vec_key(id)).is_none());
    db.close().unwrap();

    // A Ready descriptor and its vector put back over a moved source (the damage
    // the table lists): Stale, before serving anything, and no finding.
    let disk = SimVfs::new(33);
    let (u, id) = fixture::<E>(&disk, b"hello");
    let db = open_db::<E>(&disk);
    let (d, v) = (get_raw(&db, &desc_key(id)).unwrap(), get_raw(&db, &vec_key(id)).unwrap());
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(u, UnitUpdate { payload: b"world".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    put_raw(&db, &desc_key(id), &d);
    put_raw(&db, &vec_key(id), &v);
    assert!(db.verify_units().unwrap().is_clean(), "a Ready over an old source is a valid state");
    db.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(dd.state(&id), Some(DerivedState::Stale));
    assert_eq!(dd.vector(&id), None);
    assert_eq!(report.demoted, vec![(id, StaleCause::SourceMoved)]);
    assert!(report.findings.is_empty());
    let r = dd.repair_derived(ChangeMeta::default()).unwrap();
    assert!(r.writes > 0, "the repair persists the Stale state");
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(VectorDescriptor::decode(&get_raw(&db, &desc_key(id)).unwrap()).unwrap().state, VectorState::Stale);
}

/// DV-11 and DV-14, and the damaged mark.
fn a_version_1_mark_with_derived_keys_and_a_dead_source_are_damage<E: Target>() {
    let disk = SimVfs::new(34);
    let (_, id) = fixture::<E>(&disk, b"hello");
    let db = open_db::<E>(&disk);
    let mark_key = Key::LayerMeta { name: "layer".into() }.encode().unwrap();
    put_raw(
        &db,
        &mark_key,
        &LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned }.encode().unwrap(),
    );
    assert!(rules(&db).contains("DV-11"));
    db.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(report.served, 0);
    assert_eq!(dd.state(&id), None, "nothing is loaded from a database of version 1");
    assert!(report.findings.iter().any(|f| f.rule == DerivedRule::Dv(11)));
    assert_eq!(dd.repair_derived(ChangeMeta::default()).unwrap().writes, 0, "a mark cannot be repaired from here");
    dd.close().unwrap();

    // DV-14: the derived keys of a unit that was deleted, put back.
    let disk = SimVfs::new(35);
    let (u, id) = fixture::<E>(&disk, b"hello");
    let db = open_db::<E>(&disk);
    let saved: Vec<(Vec<u8>, Vec<u8>)> = [desc_key(id), vec_key(id), src_key(u, 1, id)]
        .into_iter()
        .map(|k| {
            let v = get_raw(&db, &k).unwrap();
            (k, v)
        })
        .collect();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(u, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(derived_keys(&db), 0);
    for (k, v) in &saved {
        put_raw(&db, k, v);
    }
    assert!(rules(&db).contains("DV-14"));
    db.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(dd.state(&id), None);
    assert!(report.findings.iter().any(|f| f.rule == DerivedRule::Dv(14)), "{report:?}");
    assert!(dd.repair_derived(ChangeMeta::default()).unwrap().writes > 0);
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 0);
    clean(&db, "after removing the derived data of a dead unit");
}

// ------------------------------------- T17, T18, T20: deletion and archive

fn delete_hides_derived_at_once<E: Target>() {
    let disk = SimVfs::new(40);
    let (dd, _) = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a"), make(&dd, b"b"));
    let (on_a, on_b) = (build(&dd, a), build(&dd, b));
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(a, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&on_a), None, "a deleted source's derived datum is removed, not marked");
    assert_eq!(dd.vector(&on_a), None);
    assert_eq!(dd.state(&on_b), Some(DerivedState::Ready), "the other unit's is untouched");
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    let snap = db.raw_snapshot().unwrap();
    for k in [desc_key(on_a), vec_key(on_a), src_key(a, 1, on_a)] {
        assert_eq!(snap.get(&k).unwrap(), None, "get returns a deleted derived key");
    }
    for tag in [TAG_DERIVED_DESC, TAG_DERIVED_VEC, TAG_DERIVED_SRC] {
        let keys: Vec<Vec<u8>> = scan_tag(&db, tag).into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys.len(), 1, "scan returns only the other unit's record");
        assert!(keys[0].ends_with(&on_b.0), "{keys:?}");
    }
    clean(&db, "after the delete");
}

fn snapshot_keeps_derived_until_released<E: Target>() {
    let disk = SimVfs::new(41);
    let (a, id) = fixture::<E>(&disk, b"hello");
    let db = open_db::<E>(&disk);
    let held = db.raw_snapshot().unwrap();
    let want: Vec<_> = [desc_key(id), vec_key(id), src_key(a, 1, id)]
        .into_iter()
        .map(|k| (held.get(&k).unwrap().unwrap(), k))
        .collect();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(a, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(derived_keys(&db), 0, "a new reader sees none");
    for (v, k) in &want {
        assert_eq!(held.get(k).unwrap().as_ref(), Some(v), "the snapshot taken before the commit still reads it");
    }
    drop(held);
}

fn archive_is_stale_delete_is_removed<E: Target>() {
    let disk = SimVfs::new(42);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"hello");
    let id = build(&dd, u);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Stale), "archiving marks, it does not delete");
    assert_eq!(dd.vector(&id), None);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.unarchive(u, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Stale), "unarchiving does not bring a stale datum back");
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 2, "descriptor and entry are kept, the vector is not");
    assert_eq!(VectorDescriptor::decode(&get_raw(&db, &desc_key(id)).unwrap()).unwrap().state, VectorState::Stale);
    db.close().unwrap();
    // A rebuild over the unarchived unit, then the delete removes without marking.
    let (dd, _) = open_dd::<E>(&disk);
    let id2 = dd.rebuild_vector(&id, ChangeMeta::default(), &vcompute).unwrap();
    assert_eq!(dd.state(&id2), Some(DerivedState::Ready));
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(u, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id2), None);
    dd.close().unwrap();
    assert_eq!(derived_keys(&open_db::<E>(&disk)), 0);
}

/// Retention that purges a revision takes the derived data computed from it,
/// in its own change set.
fn retention_purges_the_derived_data_of_a_purged_revision<E: Target>() {
    let disk = SimVfs::new(43);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"r1");
    let old = build(&dd, u);
    edit(&dd, u, b"r2");
    edit(&dd, u, b"r3");
    let current = build(&dd, u);
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 5, "the stale datum keeps descriptor and entry, the current one has three records");
    let report =
        db.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default()).unwrap();
    assert!(report.complete);
    assert!(get_raw(&db, &desc_key(old)).is_none(), "the datum computed from a purged revision is removed");
    assert!(get_raw(&db, &vec_key(old)).is_none() && get_raw(&db, &src_key(u, 1, old)).is_none());
    assert!(get_raw(&db, &desc_key(current)).is_some(), "the datum of the retained revision stays");
    assert_eq!(derived_keys(&db), 3);
    assert_eq!(mark_version(&db), Some(2));
    clean(&db, "after retention");
    db.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(report.served, 1);
    assert_eq!(dd.state(&current), Some(DerivedState::Ready));
    dd.close().unwrap();
}

fn supersede_is_one_changeset<E: Target>() {
    let disk = SimVfs::new(44);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"hello");
    let old = build(&dd, u);
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    let before = db.snapshot().unwrap().seq();
    db.close().unwrap();
    let (dd, _) = open_dd::<E>(&disk);
    let new = dd.supersede_vector(&old, &recipe(2, "model-a"), ChangeMeta::default(), &vcompute).unwrap();
    assert_ne!(new, old);
    assert_eq!(dd.state(&old), Some(DerivedState::Superseded));
    assert_eq!(dd.vector(&old), None);
    assert_eq!(dd.state(&new), Some(DerivedState::Ready));
    assert_eq!(dd.vector(&new), Some(stored(b"hello")));
    assert!(matches!(
        dd.supersede_vector(&old, &recipe(3, "model-a"), ChangeMeta::default(), &vcompute),
        Err(DerivedStaleError::Derived(DerivedError::IllegalTransition { .. }))
    ));
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(db.snapshot().unwrap().seq().0, before.0 + 1, "the new datum and the old one's change are one commit");
    assert_eq!(
        VectorDescriptor::decode(&get_raw(&db, &desc_key(old)).unwrap()).unwrap().state,
        VectorState::Superseded
    );
    assert!(get_raw(&db, &vec_key(old)).is_none(), "its vector is deleted");
    assert!(get_raw(&db, &src_key(u, 1, old)).is_some(), "its 0x62 entry is kept, like every descriptor's");
    assert!(get_raw(&db, &vec_key(new)).is_some());
    clean(&db, "after the supersede");
    // The change set is of purpose Derived with no members.
    let last = Key::ChangeSet { seq: db.snapshot().unwrap().seq() }.encode().unwrap();
    let rec = ChangeSetRecord::decode_upto(&get_raw(&db, &last).unwrap(), MAX_LAYER_VERSION).unwrap();
    assert_eq!((rec.purpose, rec.member_count), (Purpose::Derived, 0));
    drop(db);
    let (dd, report) = open_dd::<E>(&disk);
    assert!(report.findings.is_empty());
    assert_eq!(dd.state(&old), Some(DerivedState::Superseded), "reopening keeps it superseded");
    dd.close().unwrap();
}

fn stale_source_write_is_refused<E: Target>() {
    let disk = SimVfs::new(45);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"v1");
    edit(&dd, u, b"v2");
    let before = disk.dump();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let e = cs.put_vector(&spec(u, 1, b"v1")).unwrap_err();
    assert!(matches!(e, DerivedStaleError::Unit(UnitError::InvalidArgument(_))), "{e:?}");
    // The change set stays usable and writes nothing for the refused vector.
    cs.abort();
    assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    // In one change set: an update first moves the head, so the old revision is refused…
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let rev = cs.update(u, UnitUpdate { payload: b"v3".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    assert!(cs.put_vector(&spec(u, 2, b"v2")).is_err());
    // …and the new head is accepted, and stays Ready.
    let id = cs.put_vector(&spec(u, rev.0, b"v3")).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Ready));
    // A vector first, then the update, in one change set: the update marks it.
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id4 = cs.put_vector(&spec(u, rev.0, b"v3")).unwrap_err();
    assert!(matches!(id4, DerivedStaleError::Derived(DerivedError::Duplicate(_))));
    cs.update(u, UnitUpdate { payload: b"v4".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Stale));
    // A missing or deleted source.
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    assert!(matches!(
        cs.put_vector(&spec(unit_id(999), 1, b"x")),
        Err(DerivedStaleError::Unit(UnitError::NotFound { .. }))
    ));
    cs.delete(u, None).unwrap();
    assert!(matches!(cs.put_vector(&spec(u, 5, b"x")), Err(DerivedStaleError::Unit(UnitError::Deleted { .. }))));
    cs.abort();
    dd.close().unwrap();
    clean(&open_db::<E>(&disk), "after the refusals");
}

/// A change set that persists a vector and also writes units is an
/// `Application` change set; one that only persists derived data is `Derived`
/// and has no members (DV-13). The verifier rejects the other shape.
fn derived_purpose_has_no_members<E: Target>() {
    let disk = SimVfs::new(46);
    let (dd, _) = open_dd::<E>(&disk);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let u = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: b"p".to_vec() })
        .unwrap();
    cs.put_vector(&spec(u, 1, b"p")).unwrap();
    let with_members = cs.commit().unwrap().commit_seq;
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    let record = |seq: CommitSeq| {
        ChangeSetRecord::decode_upto(
            &get_raw(&db, &Key::ChangeSet { seq }.encode().unwrap()).unwrap(),
            MAX_LAYER_VERSION,
        )
        .unwrap()
    };
    let first = record(with_members);
    assert_eq!((first.purpose, first.member_count), (Purpose::Application, 1));
    clean(&db, "members and a vector");

    // A Derived change set with members is DV-13.
    let disk = SimVfs::new(47);
    let (u, id) = fixture::<E>(&disk, b"hello");
    let _ = (u, id);
    let db = open_db::<E>(&disk);
    let seq = db.snapshot().unwrap().seq();
    let k = Key::ChangeSet { seq }.encode().unwrap();
    let mut rec = ChangeSetRecord::decode_upto(&get_raw(&db, &k).unwrap(), MAX_LAYER_VERSION).unwrap();
    assert_eq!((rec.purpose, rec.member_count), (Purpose::Derived, 0));
    clean(&db, "derived change set");
    rec.member_count = 1;
    put_raw(&db, &k, &rec.encode().unwrap());
    assert!(rules(&db).contains("DV-13"));
}

/// More derived data than one scan page (256): every page is read by the
/// change that marks them and by the opening that loads them.
fn more_than_a_page_of_derived_data_is_all_seen<E: Target>() {
    let disk = SimVfs::new(70);
    let (dd, _) = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a"), make(&dd, b"b"));
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    for i in 0..300 {
        for (u, p) in [(a, &b"a"[..]), (b, &b"b"[..])] {
            let s = VectorSpec { method_version: i, ..spec(u, 1, p) };
            cs.put_vector(&s).unwrap();
        }
    }
    cs.commit().unwrap();
    edit(&dd, a, b"a2");
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(scan_tag(&db, TAG_DERIVED_DESC).len(), 600);
    assert_eq!(scan_tag(&db, TAG_DERIVED_VEC).len(), 300, "the 300 of the unit that changed lost their vector");
    assert_eq!(scan_tag(&db, TAG_DERIVED_SRC).len(), 600);
    clean(&db, "600 derived data");
    let stale = scan_tag(&db, TAG_DERIVED_DESC)
        .into_iter()
        .filter(|(_, v)| VectorDescriptor::decode(v).unwrap().state == VectorState::Stale)
        .count();
    assert_eq!(stale, 300);
    db.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(report.served, 300);
    assert!(report.demoted.is_empty() && report.findings.is_empty());
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(b, None).unwrap();
    cs.commit().unwrap();
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 300 * 2, "the delete removed the 900 records of the 300 vectors of b, in every page");
    clean(&db, "after the delete");
}

/// A descriptor that does not decode is never served and never blocks a
/// change to its source; the healthy data of the same unit are still marked.
fn a_source_change_goes_through_a_damaged_descriptor<E: Target>() {
    let disk = SimVfs::new(71);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"hello");
    let bad = build(&dd, u);
    let good = dd.build_vector(u, &recipe(1, "model-b"), ChangeMeta::default(), &vcompute).unwrap();
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    // Which of the two sorts first decides whether a `return` would skip the other.
    let (first, second) = if bad < good { (bad, good) } else { (good, bad) };
    for damaged in [first, second] {
        let healthy = if damaged == first { second } else { first };
        let disk = SimVfs::new(72);
        let (dd, _) = open_dd::<E>(&disk);
        let u = make(&dd, b"hello");
        let a = build(&dd, u);
        let b = dd.build_vector(u, &recipe(1, "model-b"), ChangeMeta::default(), &vcompute).unwrap();
        assert_eq!((a, b), (bad, good), "identities are deterministic");
        dd.close().unwrap();
        let db2 = open_db::<E>(&disk);
        let mut v = get_raw(&db2, &desc_key(damaged)).unwrap();
        v.truncate(v.len() - 1);
        put_raw(&db2, &desc_key(damaged), &v);
        let mut cs = db2.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(u, UnitUpdate { payload: b"world".to_vec(), schema_version: None, metadata: None }, None).unwrap();
        cs.commit().expect("a damaged derived record does not stop a change to the source");
        assert_eq!(
            VectorDescriptor::decode(&get_raw(&db2, &desc_key(healthy)).unwrap()).unwrap().state,
            VectorState::Stale,
            "the healthy datum is marked whichever side of the damaged one it sorts"
        );
        assert!(get_raw(&db2, &vec_key(healthy)).is_none());
        assert_eq!(get_raw(&db2, &desc_key(damaged)).unwrap(), v, "the damaged record is left for the repair");
        assert!(rules(&db2).contains("UNIT-9"));
        db2.close().unwrap();
    }
    db.close().unwrap();
}

// ------------------------------------------------------- T19: survival

/// The sentinel of a vector, as the two f32 whose big-endian bits spell it.
const SENTINEL: &[u8; 8] = b"VSENT-DV";

fn sentinel_vector() -> Vec<f32> {
    SENTINEL.as_chunks::<4>().0.iter().map(|c| f32::from_bits(u32::from_be_bytes(*c))).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The files that hold the sentinel.
fn holders(disk: &SimVfs) -> Vec<String> {
    let image = disk.dump();
    let mut out: Vec<String> = image
        .files
        .iter()
        .filter(|(_, bytes)| contains(bytes, SENTINEL))
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    if image.unlinked_open.iter().any(|b| contains(b, SENTINEL)) {
        out.push("(unlinked, open)".into());
    }
    out.sort();
    out
}

/// Predictions, written before the first run (never edited afterwards):
///
/// - **P1.** After the delete of the source, without a flush, the vector is
///   still in the write-ahead log.
/// - **P2.** After a flush, `compact_range` over the derived keys and a clean
///   close, with no snapshot held, the sentinel is in no live file.
/// - **P3.** With a snapshot taken before the delete, the sentinel stays in
///   some live file until the snapshot is released, and is gone after.
///
/// Not promised, and so not asserted: that the `unit_id` of the source leaves
/// the disk. It is in `UNIT_HEAD` and `REV_META` (a deleted unit keeps its
/// tombstone) and in no derived record that survives the delete.
fn derived_sentinel_survival() {
    for hold_snapshot in [false, true] {
        let disk = SimVfs::new(60);
        let db = open_units::<KernoDb>(disk.as_vfs(), &cfg()).unwrap();
        let (dd, _) = DerivedDb::open_persisted(db).unwrap();
        let u = make(&dd, b"hello");
        let id = dd
            .build_vector(
                u,
                &recipe(1, "model-a"),
                ChangeMeta::default(),
                &|_: &DerivedDescriptor, _: &[SourceInput]| Some(sentinel_vector()),
            )
            .unwrap();
        dd.close().unwrap();
        let db = open_units::<KernoDb>(disk.as_vfs(), &cfg()).unwrap();
        assert!(!holders(&disk).is_empty(), "the sentinel is on disk before the delete");
        let held = hold_snapshot.then(|| db.raw_snapshot().unwrap());
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.delete(u, None).unwrap();
        cs.commit().unwrap();
        assert_eq!(derived_keys(&db), 0, "I-1: nothing visible after the commit");
        let in_log = holders(&disk);
        println!("[survival] hold_snapshot={hold_snapshot} after the delete: {in_log:?}");
        assert!(in_log.iter().any(|f| f.ends_with(".wal") || f.ends_with(".sst")), "P1: {in_log:?}");
        db.hook_flush().unwrap();
        db.hook_compact_range(Bound::Included(&[TAG_DERIVED_DESC]), Bound::Excluded(&[TAG_DERIVED_SRC + 1])).unwrap();
        db.hook_wait_idle().unwrap();
        let after_compaction = holders(&disk);
        println!("[survival] hold_snapshot={hold_snapshot} after compact_range: {after_compaction:?}");
        if let Some(snap) = held {
            assert!(snap.get(&vec_key(id)).unwrap().is_some(), "I-2: the snapshot still reads the vector");
            assert!(!after_compaction.is_empty(), "P3: a held snapshot keeps the sentinel in a live file");
            drop(snap);
            db.hook_wait_idle().unwrap();
        }
        db.close().unwrap();
        let closed = holders(&disk);
        println!("[survival] hold_snapshot={hold_snapshot} after the close: {closed:?}");
        assert!(
            closed.is_empty(),
            "P2/P3: the sentinel is in no live file after compaction, release and a clean close: {closed:?}"
        );
    }
}

// ------------------------------------------------------ misc behaviour

fn put_vector_refuses_what_the_format_refuses<E: Target>() {
    let disk = SimVfs::new(50);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"p");
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let bad = [
        VectorSpec { vector: vec![f32::NAN], ..spec(u, 1, b"p") },
        VectorSpec { vector: vec![], ..spec(u, 1, b"p") },
        VectorSpec { profile: String::new(), ..spec(u, 1, b"p") },
        VectorSpec { method: String::new(), ..spec(u, 1, b"p") },
    ];
    for s in &bad {
        assert!(matches!(cs.put_vector(s), Err(DerivedStaleError::Unit(UnitError::InvalidArgument(_)))), "{s:?}");
    }
    cs.abort();
    let id = build(&dd, u);
    assert!(matches!(
        dd.build_vector(u, &recipe(1, "model-a"), ChangeMeta::default(), &vcompute),
        Err(DerivedStaleError::Derived(DerivedError::Duplicate(d))) if d == id
    ));
    assert!(matches!(
        dd.build_vector(
            u,
            &recipe(2, "model-a"),
            ChangeMeta::default(),
            &|_: &DerivedDescriptor, _: &[SourceInput]| None
        ),
        Err(DerivedStaleError::ComputeFailed(_))
    ));
    // The in-memory methods refuse a persisted datum: it changes through the vector methods.
    assert!(matches!(
        dd.rebuild(&id, &|_: &DerivedDescriptor, _: &[SourceInput]| None),
        Err(DerivedStaleError::Persisted(_))
    ));
    // The value is not a number list: a vector is served as the f32 it was.
    assert_eq!(dd.vector(&id), Some(stored(b"p")));
    dd.close().unwrap();
}

/// The profile is part of the identity: the same method over the same source
/// with another profile is another datum (ADR-0018, I-25).
fn another_profile_is_another_datum<E: Target>() {
    let disk = SimVfs::new(51);
    let (dd, _) = open_dd::<E>(&disk);
    let u = make(&dd, b"p");
    let a = build(&dd, u);
    let b = dd.build_vector(u, &recipe(1, "model-b"), ChangeMeta::default(), &vcompute).unwrap();
    assert_ne!(a, b);
    assert_eq!(dd.descriptor(&b).unwrap().profile(), "model-b");
    dd.close().unwrap();
    let (dd, report) = open_dd::<E>(&disk);
    assert_eq!(report.served, 2);
    assert_eq!(dd.descriptor(&a).unwrap().profile(), "model-a");
    dd.close().unwrap();
}

/// Opening a damaged vector never panics and never serves it, whatever byte
/// is changed.
fn every_byte_flip_of_the_records_is_handled<E: Target>() {
    let disk = SimVfs::new(52);
    let (_, id) = fixture::<E>(&disk, b"hello");
    let image = disk.dump();
    let db = open_db::<E>(&disk);
    let (dv, vv) = (get_raw(&db, &desc_key(id)).unwrap(), get_raw(&db, &vec_key(id)).unwrap());
    db.close().unwrap();
    for (key, value) in [(desc_key(id), dv), (vec_key(id), vv)] {
        for i in 0..value.len() {
            let disk = SimVfs::restore(&image);
            let db = open_db::<E>(&disk);
            let mut v = value.clone();
            v[i] ^= 0x10;
            put_raw(&db, &key, &v);
            let report = db.verify_units().unwrap();
            db.close().unwrap();
            let (dd, opened) = open_dd::<E>(&disk);
            // Either the record still means the same (a flipped bit in a text field of
            // the vector's padding cannot happen: every byte counts), or something is reported
            // or the datum is not served as Ready.
            let served = dd.vector(&id).is_some();
            assert!(
                !served || (report.is_clean() && opened.findings.is_empty() && opened.demoted.is_empty()),
                "byte {i} of {:02x?}: served despite damage",
                key[0]
            );
            dd.close().unwrap();
        }
    }
}

// ------------- one change set that persists a vector and changes the source

/// Persisted states of every descriptor, by source unit.
fn persisted_states<E: Target>(db: &UnitDb<E>) -> Vec<(UnitId, VectorState)> {
    scan_tag(db, TAG_DERIVED_DESC)
        .into_iter()
        .map(|(_, v)| {
            let d = VectorDescriptor::decode(&v).unwrap();
            (d.source.unit, d.state)
        })
        .collect()
}

/// A database still at layer version 1 holding the units `payloads`.
fn v1_units<E: Target>(disk: &SimVfs, payloads: &[&[u8]]) -> Vec<UnitId> {
    let (dd, _) = open_dd::<E>(disk);
    let ids = payloads.iter().map(|p| make(&dd, p)).collect();
    dd.close().unwrap();
    let db = open_db::<E>(disk);
    assert_eq!(mark_version(&db), Some(1));
    ids
}

fn close_and_check<E: Target>(dd: DerivedDb<E>, disk: &SimVfs, ctx: &str) {
    dd.close().unwrap();
    check_no_ready_on_an_old_source::<E>(disk, ctx);
}

fn cs_persist_then_update<E: Target>() {
    let disk = SimVfs::new(60);
    let u = v1_units::<E>(&disk, &[b"a"])[0];
    let (dd, _) = open_dd::<E>(&disk);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.put_vector(&spec(u, 1, b"a")).unwrap();
    cs.update(u, UnitUpdate { payload: b"b".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Stale));
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(persisted_states(&db), vec![(u, VectorState::Stale)]);
    assert_eq!(mark_version(&db), Some(2));
    drop(db);
    check_no_ready_on_an_old_source::<E>(&disk, "persist then update");
}

fn cs_persist_then_delete<E: Target>() {
    let disk = SimVfs::new(61);
    let u = v1_units::<E>(&disk, &[b"a"])[0];
    let (dd, _) = open_dd::<E>(&disk);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.put_vector(&spec(u, 1, b"a")).unwrap();
    cs.delete(u, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), None);
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 0, "no descriptor, vector or inverse-index entry");
    clean(&db, "persist then delete");
}

fn cs_update_then_persist<E: Target>() {
    let disk = SimVfs::new(62);
    let u = v1_units::<E>(&disk, &[b"a"])[0];
    let (dd, _) = open_dd::<E>(&disk);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(u, UnitUpdate { payload: b"b".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    let id = cs.put_vector(&spec(u, 2, b"b")).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&id), Some(DerivedState::Ready));
    close_and_check::<E>(dd, &disk, "update then persist");
    let db = open_db::<E>(&disk);
    assert_eq!(persisted_states(&db), vec![(u, VectorState::Ready)]);
}

fn cs_delete_then_persist<E: Target>() {
    let disk = SimVfs::new(63);
    let u = v1_units::<E>(&disk, &[b"a"])[0];
    let (dd, _) = open_dd::<E>(&disk);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(u, None).unwrap();
    assert!(cs.put_vector(&spec(u, 1, b"a")).is_err(), "no vector over a deleted unit");
    cs.commit().unwrap();
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 0);
    assert_eq!(mark_version(&db), Some(1), "nothing derived was written: the mark stays");
    clean(&db, "delete then persist");
}

fn cs_persist_two_change_one<E: Target>() {
    let disk = SimVfs::new(64);
    let us = v1_units::<E>(&disk, &[b"a", b"b"]);
    let (dd, _) = open_dd::<E>(&disk);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let ia = cs.put_vector(&spec(us[0], 1, b"a")).unwrap();
    let ib = cs.put_vector(&spec(us[1], 1, b"b")).unwrap();
    cs.update(us[0], UnitUpdate { payload: b"c".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&ia), Some(DerivedState::Stale));
    assert_eq!(dd.state(&ib), Some(DerivedState::Ready));
    close_and_check::<E>(dd, &disk, "persist two, change one");
    let db = open_db::<E>(&disk);
    let mut got = persisted_states(&db);
    got.sort_by_key(|(u, _)| u.0);
    let mut want = vec![(us[0], VectorState::Stale), (us[1], VectorState::Ready)];
    want.sort_by_key(|(u, _)| u.0);
    assert_eq!(got, want);
}

macro_rules! tests {
    ($e:ty; $($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                super::$name::<$e>();
            }
        )*
        #[test]
        fn derived_mark_every_point_power_loss() {
            super::derived_mark_every_point::<$e>(kerno_testkit::vfs::CrashKind::PowerLoss);
        }
        #[test]
        fn derived_mark_every_point_process_kill() {
            super::derived_mark_every_point::<$e>(kerno_testkit::vfs::CrashKind::ProcessKill);
        }
        #[test]
        fn stale_mark_every_point_power_loss() {
            super::stale_mark_every_point::<$e>(kerno_testkit::vfs::CrashKind::PowerLoss);
        }
        #[test]
        fn stale_mark_every_point_process_kill() {
            super::stale_mark_every_point::<$e>(kerno_testkit::vfs::CrashKind::ProcessKill);
        }
    };
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            tests!(
                $e;
                t03_identity_mismatch_is_dv10_and_not_loaded,
                vector_hash_mismatch_is_not_served,
                layer1_reader_refuses_derived_fixture,
                a_database_without_vectors_stays_at_version_1,
                open_derived_fixture_writes_nothing,
                a_commit_that_fails_marks_nothing,
                a_failed_first_vector_writes_no_derived_key_and_no_mark,
                mark_never_lowers,
                a_new_database_with_a_first_vector_starts_at_version_2,
                rebuilt_equals_fresh,
                derived_loss_leaves_units_unchanged,
                reopen_refuses_or_stales_each_damage_case,
                reopen_after_source_change_is_stale,
                a_version_1_mark_with_derived_keys_and_a_dead_source_are_damage,
                delete_hides_derived_at_once,
                snapshot_keeps_derived_until_released,
                archive_is_stale_delete_is_removed,
                retention_purges_the_derived_data_of_a_purged_revision,
                supersede_is_one_changeset,
                stale_source_write_is_refused,
                derived_purpose_has_no_members,
                put_vector_refuses_what_the_format_refuses,
                another_profile_is_another_datum,
                every_byte_flip_of_the_records_is_handled,
                more_than_a_page_of_derived_data_is_all_seen,
                a_source_change_goes_through_a_damaged_descriptor,
                cs_persist_then_update,
                cs_persist_then_delete,
                cs_update_then_persist,
                cs_delete_then_persist,
                cs_persist_two_change_one,
            );
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);

#[test]
fn derived_sentinel_survival_on_kerno_db() {
    derived_sentinel_survival();
}
