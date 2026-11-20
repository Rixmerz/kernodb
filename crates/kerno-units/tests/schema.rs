//! Phase C: typed schemas per unit type and user-defined secondary indexes
//! (ADR-0021, `kerno_units::schema`).
//!
//! A schema refuses a write whole, an index follows every write that changes a
//! unit's `Active` payload (create, update, archive, unarchive, delete,
//! restore), a definition backfills or removes its entries in its own change
//! set, and `verify_units` sees every way the entries can be wrong. Each test
//! runs on `emulated_log` and on `kerno_db`.

#![cfg(feature = "test-support")]

mod harness;

use std::collections::BTreeSet;
use std::ops::Bound;
use std::sync::Arc;

use harness::units::{Target, open_units, raw};
use kerno_core::contract::Options;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{Key, LayerMeta, MAX_LAYER_VERSION};
use kerno_units::*;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const ACCT: &str = "acct";

fn open<E: Target>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    open_units::<E>(SimVfs::new(5).as_vfs(), &c).expect("open")
}

fn doc(name: &str, age: i64, email: &str) -> Vec<u8> {
    Document::new()
        .with("name", FieldValue::Text(name.into()))
        .with("age", FieldValue::Int(age))
        .with("email", FieldValue::Text(email.into()))
        .encode()
        .expect("encode")
}

fn acct_fields() -> Vec<FieldDef> {
    vec![
        FieldDef::new("name", FieldType::Text).required(),
        FieldDef::new("age", FieldType::Int).indexed(),
        FieldDef::new("email", FieldType::Text).unique(),
        FieldDef::new("score", FieldType::Float).indexed(),
        FieldDef::new("note", FieldType::Bytes),
    ]
}

fn define<E: Target>(db: &UnitDb<E>) {
    db.define_schema(ACCT, acct_fields(), ChangeMeta::default()).expect("define the schema");
}

fn new_unit(unit_type: &str, payload: Vec<u8>) -> NewUnit {
    NewUnit { unit_type: unit_type.into(), schema_version: 1, metadata: vec![], payload }
}

fn create<E: Target>(db: &UnitDb<E>, payload: Vec<u8>) -> Result<UnitId, UnitError> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    let id = cs.create(new_unit(ACCT, payload))?;
    cs.commit()?;
    Ok(id)
}

fn one<E: Target, T>(
    db: &UnitDb<E>,
    f: impl FnOnce(&mut ChangeSet<'_, E>) -> Result<T, UnitError>,
) -> Result<T, UnitError> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    let out = f(&mut cs)?;
    cs.commit()?;
    Ok(out)
}

fn update<E: Target>(db: &UnitDb<E>, id: UnitId, payload: Vec<u8>) -> Result<RevNo, UnitError> {
    one(db, |cs| cs.update(id, UnitUpdate { payload, schema_version: None, metadata: None }, None))
}

fn is_violation<T: std::fmt::Debug>(r: &Result<T, UnitError>) -> bool {
    matches!(r, Err(UnitError::SchemaViolation { .. }))
}

fn is_corruption<T: std::fmt::Debug>(r: &Result<T, UnitError>) -> bool {
    matches!(r, Err(UnitError::Storage(kerno_core::Error::Corruption(_))))
}

#[track_caller]
fn clean<E: Target>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().expect("verify_units");
    assert!(r.is_clean(), "{ctx}: {:?}", r.findings);
}

fn rules<E: Target>(db: &UnitDb<E>) -> BTreeSet<String> {
    db.verify_units().expect("verify_units").rules()
}

fn layer_version<E: Target>(db: &UnitDb<E>) -> u32 {
    let key = Key::LayerMeta { name: "layer".into() }.encode().expect("mark key");
    let mark = raw(db).into_iter().find(|(k, _)| *k == key).expect("mark").1;
    LayerMeta::decode_upto(&mark, MAX_LAYER_VERSION).expect("mark").layer_version
}

/// The stored records of one family.
fn stored<E: Target>(db: &UnitDb<E>, tag: u8) -> Vec<(Vec<u8>, Vec<u8>)> {
    raw(db).into_iter().filter(|(k, _)| k[0] == tag).collect()
}

/// The ids the index answers for `field == value`.
fn found<E: Target>(db: &UnitDb<E>, field: &str, value: &FieldValue) -> Vec<UnitId> {
    let units = db.snapshot().unwrap().find_by_field(ACCT, field, value).expect("find_by_field");
    units.iter().map(|u| u.id).collect()
}

/// The definition of the answer: every `Active` unit of the type, its payload
/// decoded, filtered by `keep`, ordered by (value, id) when `ordered`.
fn reference<E: Target>(db: &UnitDb<E>, field: &str, keep: impl Fn(&FieldValue) -> bool) -> Vec<UnitId> {
    let snap = db.snapshot().unwrap();
    let units = snap.reference_filter(&UnitFilter { unit_type: Some(ACCT.into()), schema_version: None }).unwrap();
    units
        .iter()
        .filter(|u| Document::decode(u.payload.as_deref().unwrap()).unwrap().get(field).is_some_and(&keep))
        .map(|u| u.id)
        .collect()
}

fn text(s: &str) -> FieldValue {
    FieldValue::Text(s.into())
}

// ------------------------------------------------------------ validation

/// Every way a payload fails the schema is `SchemaViolation`, stores nothing and
/// leaves the change set usable; the same payload on a type without a schema is
/// accepted untouched.
fn a_refused_write_stores_nothing<E: Target>() {
    let db = open::<E>();
    // Opaque bytes, before there is a schema.
    one(&db, |cs| cs.create(new_unit("blob", b"not a document".to_vec()))).unwrap();
    define(&db);
    let before = raw(&db);
    let bad: Vec<(&str, Vec<u8>)> = vec![
        ("not a document", b"plain bytes".to_vec()),
        ("required field missing", Document::new().with("age", FieldValue::Int(1)).encode().unwrap()),
        ("wrong type", Document::new().with("name", FieldValue::Int(7)).encode().unwrap()),
        ("undeclared field", Document::new().with("name", text("a")).with("nickname", text("b")).encode().unwrap()),
        (
            "an indexed value too long",
            Document::new().with("name", text("a")).with("email", text(&"e".repeat(300))).encode().unwrap(),
        ),
    ];
    for (what, payload) in bad {
        let r = create(&db, payload);
        assert!(is_violation(&r), "{what}: {r:?}");
        assert_eq!(raw(&db), before, "{what}: a refused create stored something");
    }
    // The refusal leaves the change set usable: the valid ops around it commit.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let a = cs.create(new_unit(ACCT, doc("a", 1, "a@x"))).unwrap();
    assert!(is_violation(&cs.create(new_unit(ACCT, b"junk".to_vec()))));
    let b = cs.create(new_unit(ACCT, doc("b", 2, "b@x"))).unwrap();
    cs.commit().unwrap();
    assert_eq!(found(&db, "age", &FieldValue::Int(1)), vec![a]);
    assert_eq!(found(&db, "age", &FieldValue::Int(2)), vec![b]);
    // Dropping the change set after a refusal stores nothing of it.
    let before = raw(&db);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create(new_unit(ACCT, doc("c", 3, "c@x"))).unwrap();
    assert!(is_violation(&cs.update(
        a,
        UnitUpdate { payload: b"junk".to_vec(), schema_version: None, metadata: None },
        None
    )));
    drop(cs);
    assert_eq!(raw(&db), before);
    // An update that breaks the schema keeps the old revision and entries.
    assert!(is_violation(&update(&db, a, b"junk".to_vec())));
    assert_eq!(found(&db, "age", &FieldValue::Int(1)), vec![a]);
    clean(&db, "after the refusals");
}

/// `unique` is among `Active` units, across change sets and inside one.
fn unique_holds_among_active_units<E: Target>() {
    let db = open::<E>();
    define(&db);
    let a = create(&db, doc("a", 1, "same@x")).unwrap();
    assert!(is_violation(&create(&db, doc("b", 2, "same@x"))), "a second unit with the value");
    // Two creates of one value in one change set.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create(new_unit(ACCT, doc("c", 3, "twin@x"))).unwrap();
    assert!(is_violation(&cs.create(new_unit(ACCT, doc("d", 4, "twin@x")))));
    drop(cs);
    // An update to another unit's value; to its own value it is fine.
    let b = create(&db, doc("b", 2, "b@x")).unwrap();
    assert!(is_violation(&update(&db, b, doc("b", 2, "same@x"))));
    update(&db, a, doc("a2", 1, "same@x")).unwrap();
    // Archiving frees the value; unarchiving takes it back only if still free.
    one(&db, |cs| cs.archive(a, None)).unwrap();
    let c = create(&db, doc("c", 3, "same@x")).unwrap();
    let before = raw(&db);
    assert!(is_violation(&one(&db, |cs| cs.unarchive(a, None))), "another unit took the value");
    assert_eq!(raw(&db), before);
    one(&db, |cs| cs.delete(c, None)).unwrap();
    one(&db, |cs| cs.unarchive(a, None)).unwrap();
    assert_eq!(found(&db, "email", &text("same@x")), vec![a]);
    clean(&db, "unique");
}

// ---------------------------------------------------------- index upkeep

/// The index answers what the payloads say after every kind of write.
fn the_index_follows_every_write<E: Target>() {
    let db = open::<E>();
    define(&db);
    let check = |ctx: &str| {
        clean(&db, ctx);
        for age in 0..6 {
            let v = FieldValue::Int(age);
            assert_eq!(found(&db, "age", &v), reference(&db, "age", |f| *f == v), "{ctx}: age {age}");
        }
    };
    let a = create(&db, doc("a", 1, "a@x")).unwrap();
    let b = create(&db, doc("b", 1, "b@x")).unwrap();
    check("created");
    assert_eq!(found(&db, "age", &FieldValue::Int(1)), vec![a, b]);
    update(&db, a, doc("a", 2, "a@x")).unwrap();
    check("updated");
    one(&db, |cs| cs.archive(b, None)).unwrap();
    check("archived");
    assert!(found(&db, "age", &FieldValue::Int(1)).is_empty());
    one(&db, |cs| cs.unarchive(b, None)).unwrap();
    check("unarchived");
    update(&db, a, doc("a", 3, "a@x")).unwrap();
    one(&db, |cs| cs.restore(a, RevNo(2), None)).unwrap();
    check("restored");
    assert_eq!(found(&db, "age", &FieldValue::Int(2)), vec![a]);
    one(&db, |cs| cs.delete(a, None)).unwrap();
    check("deleted");
    assert_eq!(found(&db, "age", &FieldValue::Int(2)), Vec::<UnitId>::new());
    // A deleted unit's unique value is free again.
    create(&db, doc("a again", 4, "a@x")).unwrap();
    check("reused");
    // Optional fields that are absent have no entry.
    let entries = stored(&db, 0x79).len();
    create(&db, Document::new().with("name", text("bare")).encode().unwrap()).unwrap();
    assert_eq!(stored(&db, 0x79).len(), entries);
    clean(&db, "a unit without indexed fields");
}

/// Ranges are in value order for ints (negative included), floats (`-0.0` is
/// `0.0`), and text (NUL bytes and prefixes included), with every bound kind.
fn ranges_are_in_value_order<E: Target>() {
    let db = open::<E>();
    db.define_schema(
        "t",
        vec![
            FieldDef::new("i", FieldType::Int).indexed(),
            FieldDef::new("f", FieldType::Float).indexed(),
            FieldDef::new("s", FieldType::Text).indexed(),
            FieldDef::new("b", FieldType::Bool).indexed(),
        ],
        ChangeMeta::default(),
    )
    .unwrap();
    let ints = [i64::MIN, -5, -1, 0, 1, 7, i64::MAX];
    let floats = [f64::NEG_INFINITY, -2.5, -0.0, 0.0, 1.5, f64::INFINITY, -1e-300];
    let texts = ["", "a", "a\0", "a\0b", "ab", "b", "é"];
    let mut ids = Vec::new();
    for n in 0..ints.len() {
        let d = Document::new()
            .with("i", FieldValue::Int(ints[n]))
            .with("f", FieldValue::Float(floats[n]))
            .with("s", text(texts[n]))
            .with("b", FieldValue::Bool(n % 2 == 0));
        ids.push(one(&db, |cs| cs.create(new_unit("t", d.encode().unwrap()))).unwrap());
    }
    let snap = db.snapshot().unwrap();
    let range = |field: &str, lo: Bound<&FieldValue>, hi: Bound<&FieldValue>, limit: usize| -> Vec<UnitId> {
        snap.find_range("t", field, lo, hi, limit).unwrap().iter().map(|u| u.id).collect()
    };
    let (all, none) = (Bound::Unbounded, Bound::Unbounded);
    // Ints: ascending value order.
    let mut by_int: Vec<usize> = (0..ints.len()).collect();
    by_int.sort_by_key(|&n| ints[n]);
    assert_eq!(range("i", all, none, usize::MAX), by_int.iter().map(|&n| ids[n]).collect::<Vec<_>>());
    let (lo, hi) = (FieldValue::Int(-1), FieldValue::Int(7));
    assert_eq!(range("i", Bound::Included(&lo), Bound::Included(&hi), 99), vec![ids[2], ids[3], ids[4], ids[5]]);
    assert_eq!(range("i", Bound::Excluded(&lo), Bound::Excluded(&hi), 99), vec![ids[3], ids[4]]);
    assert_eq!(range("i", Bound::Excluded(&lo), Bound::Unbounded, 2), vec![ids[3], ids[4]], "limit");
    assert!(range("i", Bound::Included(&hi), Bound::Included(&lo), 99).is_empty(), "lo above hi");
    // Floats: -0.0 and 0.0 are one value, ties by unit id.
    let zero = FieldValue::Float(0.0);
    assert_eq!(range("f", Bound::Included(&zero), Bound::Included(&zero), 9), vec![ids[2], ids[3]]);
    let order: Vec<UnitId> = [0, 1, 6, 2, 3, 4, 5].iter().map(|&n| ids[n]).collect();
    assert_eq!(range("f", all, none, 99), order);
    // Text: the order of the bytes, a NUL and a prefix included.
    let order: Vec<UnitId> = [0, 1, 2, 3, 4, 5, 6].iter().map(|&n| ids[n]).collect();
    assert_eq!(range("s", all, none, 99), order);
    let a = text("a");
    assert_eq!(snap.find_by_field("t", "s", &a).unwrap().iter().map(|u| u.id).collect::<Vec<_>>(), vec![ids[1]]);
    // Bool.
    let yes = FieldValue::Bool(true);
    assert_eq!(snap.find_by_field("t", "b", &yes).unwrap().len(), 4);
    // What cannot be asked.
    let wrong = FieldValue::Text("x".into());
    assert!(matches!(snap.find_by_field("t", "i", &wrong), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(snap.find_by_field("t", "nope", &wrong), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(snap.find_by_field("none", "i", &wrong), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(snap.find_by_field("t", "f", &FieldValue::Float(f64::NAN)), Err(UnitError::InvalidArgument(_))));
    clean(&db, "ranges");
}

// ------------------------------------------------------------ definitions

/// A schema over units that exist: they are validated and indexed in the
/// change set that defines it; one that does not fit refuses the definition
/// and stores nothing, not even the layer mark.
fn a_definition_backfills_or_refuses<E: Target>() {
    let db = open::<E>();
    let a = one(&db, |cs| cs.create(new_unit(ACCT, doc("a", 1, "a@x")))).unwrap();
    let b = one(&db, |cs| cs.create(new_unit(ACCT, doc("b", 2, "b@x")))).unwrap();
    one(&db, |cs| cs.archive(b, None)).unwrap();
    let layer = layer_version(&db);
    let before = raw(&db);
    // An existing unit the schema refuses (an archived one counts).
    let strict =
        vec![FieldDef::new("name", FieldType::Text).required(), FieldDef::new("rank", FieldType::Int).required()];
    assert!(is_violation(&db.define_schema(ACCT, strict, ChangeMeta::default())));
    // A unique value two existing units hold.
    let c = one(&db, |cs| cs.create(new_unit(ACCT, doc("c", 3, "a@x")))).unwrap();
    let before_dup = raw(&db);
    assert!(is_violation(&db.define_schema(ACCT, acct_fields(), ChangeMeta::default())));
    assert_eq!(raw(&db), before_dup, "a refused definition stores nothing");
    assert_ne!(before, before_dup);
    one(&db, |cs| cs.delete(c, None)).unwrap();
    assert!(stored(&db, 0x78).is_empty());
    // It fits now.
    define(&db);
    assert_eq!(layer_version(&db), 7, "the first schema raises the layer mark to 6");
    assert_eq!(layer, 7);
    assert_eq!(found(&db, "age", &FieldValue::Int(1)), vec![a]);
    assert!(found(&db, "age", &FieldValue::Int(2)).is_empty(), "the archived unit has no entry");
    clean(&db, "backfilled");
    // A second schema for a type is refused.
    assert!(matches!(db.define_schema(ACCT, acct_fields(), ChangeMeta::default()), Err(UnitError::InvalidArgument(_))));
}

/// `create_index` backfills one field and `drop_index` removes its entries; a
/// unique field keeps its index; both are plain change sets.
fn create_and_drop_index<E: Target>() {
    let db = open::<E>();
    define(&db);
    let a = create(&db, doc("alpha", 1, "a@x")).unwrap();
    let b = create(&db, doc("beta", 2, "b@x")).unwrap();
    let entries = stored(&db, 0x79).len();
    let by_name = |db: &UnitDb<E>| db.snapshot().unwrap().find_by_field(ACCT, "name", &text("beta"));
    assert!(matches!(by_name(&db), Err(UnitError::InvalidArgument(_))), "name is not indexed yet");
    db.create_index(ACCT, "name", ChangeMeta::default()).unwrap();
    assert_eq!(stored(&db, 0x79).len(), entries + 2);
    assert_eq!(by_name(&db).unwrap().iter().map(|u| u.id).collect::<Vec<_>>(), vec![b]);
    clean(&db, "indexed after the fact");
    // Kept in step from then on.
    update(&db, a, doc("beta", 1, "a@x")).unwrap();
    assert_eq!(by_name(&db).unwrap().len(), 2);
    clean(&db, "kept");
    assert!(matches!(db.create_index(ACCT, "name", ChangeMeta::default()), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(db.create_index(ACCT, "nope", ChangeMeta::default()), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(db.drop_index(ACCT, "email", ChangeMeta::default()), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(db.drop_index(ACCT, "note", ChangeMeta::default()), Err(UnitError::InvalidArgument(_))));
    db.drop_index(ACCT, "name", ChangeMeta::default()).unwrap();
    assert_eq!(stored(&db, 0x79).len(), entries);
    assert!(matches!(by_name(&db), Err(UnitError::InvalidArgument(_))));
    clean(&db, "dropped");
    // The definition is in the change set that carries it: aborting it drops it.
    let before = raw(&db);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_index(ACCT, "name").unwrap();
    drop(cs);
    assert_eq!(raw(&db), before);
}

/// A database that never defines a schema is what it was before: no record of
/// the family and the layer mark stays where it was. Defining one raises the
/// mark to 6 in the change set that carries it, and aborting that change set
/// leaves the mark at 5.
fn without_a_schema_nothing_changes<E: Target>() {
    let db = open::<E>();
    one(&db, |cs| cs.create(new_unit("blob", vec![1, 2, 3]))).unwrap();
    let layer = layer_version(&db);
    assert!(stored(&db, 0x78).is_empty() && stored(&db, 0x79).is_empty());
    assert_eq!(layer, 7);
    clean(&db, "no schema");
    let before = raw(&db);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.define_schema(ACCT, acct_fields()).unwrap();
    drop(cs);
    assert_eq!(raw(&db), before, "an aborted definition leaves the database as it was");
    assert_eq!(layer_version(&db), 7);
    define(&db);
    assert_eq!(layer_version(&db), 7);
    clean(&db, "layer 6");
    // A schema change set with no unit is an `Index` change set.
    let cs = db.snapshot().unwrap().change_set(db.snapshot().unwrap().seq()).unwrap();
    assert_eq!(cs.expect("the change set").purpose, Purpose::Index);
    // The other type is still opaque.
    one(&db, |cs| cs.create(new_unit("blob", vec![9]))).unwrap();
    clean(&db, "defined");
}

// ---------------------------------------------------------------- damage

fn patch<E: Target>(db: &UnitDb<E>, puts: Vec<(Vec<u8>, Option<Vec<u8>>)>) {
    db.raw_commit(puts).expect("raw_commit");
}

/// Every way the entries can be wrong is a finding, and the read path refuses
/// the entry that lies instead of answering from it.
fn verify_sees_damaged_indexes<E: Target>() {
    let db = open::<E>();
    define(&db);
    let a = create(&db, doc("a", 1, "a@x")).unwrap();
    let b = create(&db, doc("b", 2, "b@x")).unwrap();
    let entries = stored(&db, 0x79);
    let age_of = |unit: UnitId| {
        let want = FieldValue::Int(if unit == a { 1 } else { 2 }).index_bytes_for_test();
        entries
            .iter()
            .find(|(k, _)| k.ends_with(&unit.0) && k.windows(want.len()).any(|w| w == want))
            .expect("entry")
            .clone()
    };
    // A missing entry.
    let (key, value) = age_of(a);
    patch(&db, vec![(key.clone(), None)]);
    assert!(rules(&db).contains("UNIT-50"), "{:?}", rules(&db));
    patch(&db, vec![(key.clone(), Some(value.clone()))]);
    clean(&db, "restored");
    // An entry that names another unit's payload: b under a's value.
    let mut lie = key.clone();
    let n = lie.len();
    lie[n - 16..].copy_from_slice(&b.0);
    patch(&db, vec![(lie.clone(), Some(value.clone()))]);
    assert!(rules(&db).contains("UNIT-50"));
    let looked = db.snapshot().unwrap().find_by_field(ACCT, "age", &FieldValue::Int(1));
    assert!(is_corruption(&looked), "the read path answered from a lying entry: {looked:?}");
    patch(&db, vec![(lie, None)]);
    // An entry of an archived unit: archive through the layer, then put it back.
    one(&db, |cs| cs.archive(b, None)).unwrap();
    let (kb, vb) = age_of(b);
    patch(&db, vec![(kb, Some(vb))]);
    assert!(rules(&db).contains("UNIT-50"));
    let looked = db.snapshot().unwrap().find_by_field(ACCT, "age", &FieldValue::Int(2));
    assert!(is_corruption(&looked), "{looked:?}");
}

/// A value that does not decode is a structure finding; a schema that its own
/// units violate is UNIT-51.
fn verify_sees_a_schema_the_store_breaks<E: Target>() {
    let db = open::<E>();
    define(&db);
    create(&db, doc("a", 1, "a@x")).unwrap();
    let schema_key = stored(&db, 0x78).remove(0).0;
    let entry = stored(&db, 0x79).remove(0);
    // A schema stricter than the units: copy one from another database.
    let other = open::<E>();
    other
        .define_schema(
            ACCT,
            vec![FieldDef::new("name", FieldType::Text).required(), FieldDef::new("rank", FieldType::Int).required()],
            ChangeMeta::default(),
        )
        .unwrap();
    let strict = stored(&other, 0x78).remove(0);
    let original = stored(&db, 0x78).remove(0);
    patch(&db, vec![(schema_key.clone(), Some(strict.1))]);
    assert!(rules(&db).contains("UNIT-51"), "{:?}", rules(&db));
    // A finding never carries content: the units hold the field `age`, which this schema does not declare.
    for f in db.verify_units().unwrap().findings {
        assert!(!f.detail.contains("age") && !f.detail.contains("a@x"), "a finding names payload content: {f}");
    }
    patch(&db, vec![(schema_key.clone(), Some(original.1.clone()))]);
    clean(&db, "schema restored");
    // A value that does not decode.
    patch(&db, vec![(entry.0.clone(), Some(vec![1, 0, 9]))]);
    assert!(rules(&db).iter().any(|r| r == "UNIT-10"), "{:?}", rules(&db));
    patch(&db, vec![(entry.0.clone(), Some(entry.1.clone()))]);
    // An entry of a field the schema does not index (the schema's `note` is not).
    let mut stray = entry.0.clone();
    let at = stray.windows(3).position(|w| w == b"age").expect("the field name");
    stray.splice(at..at + 3, *b"note");
    stray[at - 1] = 4;
    patch(&db, vec![(stray.clone(), Some(entry.1.clone()))]);
    assert!(rules(&db).contains("UNIT-50"));
    patch(&db, vec![(stray, None)]);
    clean(&db, "stray removed");
}

trait IndexBytes {
    fn index_bytes_for_test(&self) -> Vec<u8>;
}

impl IndexBytes for FieldValue {
    /// The encoding of the layout in the module documentation, written out
    /// here independently of the implementation.
    fn index_bytes_for_test(&self) -> Vec<u8> {
        match self {
            FieldValue::Int(i) => ((*i as u64) ^ (1 << 63)).to_be_bytes().to_vec(),
            _ => unreachable!("the damage tests index ints"),
        }
    }
}

/// The stored bytes, pinned (ADR-0021).
fn record_layout_is_pinned<E: Target>() {
    let db = open::<E>();
    db.define_schema(
        "t",
        vec![FieldDef::new("k", FieldType::Text).unique(), FieldDef::new("n", FieldType::Int).required()],
        ChangeMeta::default(),
    )
    .unwrap();
    let d = Document::new().with("k", text("a\0b")).with("n", FieldValue::Int(-2));
    // The document: version, count, then each field by ascending name.
    assert_eq!(
        d.encode().unwrap(),
        vec![
            1, 0, 2, 1, b'k', 4, 0, 0, 0, 3, b'a', 0, b'b', 1, b'n', 2, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE
        ]
    );
    let id = one(&db, |cs| cs.create(new_unit("t", d.encode().unwrap()))).unwrap();
    let schema = stored(&db, 0x78);
    // tag, type_len, type -> envelope, count, (name_len, name, type, flags)...
    assert_eq!(schema, vec![(vec![0x78, 1, b't'], vec![1, 0, 0, 2, 1, b'k', 4, 0b110, 1, b'n', 2, 0b001])]);
    // tag, type_len, type, name_len, name, escaped value, unit -> envelope.
    let mut key = vec![0x79, 1, b't', 1, b'k', b'a', 0, 0xFF, b'b', 0, 0];
    key.extend_from_slice(&id.0);
    assert_eq!(stored(&db, 0x79), vec![(key, vec![1, 0])]);
    clean(&db, "layout");
}

/// A document that is not canonical does not decode.
fn documents_are_canonical<E: Target>() {
    let _ = open::<E>();
    let d = Document::new().with("a", FieldValue::Bool(true)).with("b", FieldValue::Float(0.5));
    let bytes = d.encode().unwrap();
    assert_eq!(Document::decode(&bytes).unwrap(), d);
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Document::decode(&trailing).is_err());
    assert!(Document::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut bad_bool = bytes.clone();
    bad_bool[6] = 2;
    assert!(Document::decode(&bad_bool).is_err());
    // Names out of order.
    assert!(Document::decode(&[1, 0, 2, 1, b'b', 1, 1, 1, b'a', 1, 1]).is_err());
    assert!(Document::new().with("nan", FieldValue::Float(f64::NAN)).encode().is_err());
    assert!(Document::new().with("", FieldValue::Bool(true)).encode().is_err());
}

/// The key of an index entry as ADR-0021 lays it out, written here independently of
/// the implementation (`value` is the already encoded value).
fn entry_at(unit_type: &str, field: &str, value: &[u8], unit: UnitId) -> Vec<u8> {
    let mut k = vec![0x79, unit_type.len() as u8];
    k.extend_from_slice(unit_type.as_bytes());
    k.push(field.len() as u8);
    k.extend_from_slice(field.as_bytes());
    k.extend_from_slice(value);
    k.extend_from_slice(&unit.0);
    k
}

fn int_value(i: i64) -> Vec<u8> {
    FieldValue::Int(i).index_bytes_for_test()
}

const ENTRY_VALUE: [u8; 2] = [1, 0];

/// Two active units that hold one value of a unique field are UNIT-51, once for
/// each repeat, and nothing else (the entries are as the payloads say).
fn verify_sees_a_repeated_unique_value<E: Target>() {
    let fields = |unique: bool| {
        let age = FieldDef::new("age", FieldType::Int);
        vec![FieldDef::new("name", FieldType::Text).required(), if unique { age.unique() } else { age.indexed() }]
    };
    let named = |name: &str, age: i64| {
        Document::new().with("name", text(name)).with("age", FieldValue::Int(age)).encode().expect("encode")
    };
    let db = open::<E>();
    db.define_schema(ACCT, fields(false), ChangeMeta::default()).unwrap();
    for (name, age) in [("a", 7), ("b", 7), ("c", 7), ("d", 8)] {
        create(&db, named(name, age)).unwrap();
    }
    clean(&db, "units may share an indexed value");
    // The stricter schema of another database, over the same two fields.
    let strict = open::<E>();
    strict.define_schema(ACCT, fields(true), ChangeMeta::default()).unwrap();
    let (key, value) = stored(&strict, 0x78).remove(0);
    patch(&db, vec![(key, Some(value))]);
    let report = db.verify_units().unwrap();
    assert_eq!(report.rules(), BTreeSet::from(["UNIT-51".to_string()]), "{:?}", report.findings);
    assert_eq!(report.findings.len(), 2, "one finding per repeat: {:?}", report.findings);
    // Archived units do not count: only the repeat among active units is one.
    let first = db.snapshot().unwrap().find_by_field(ACCT, "age", &FieldValue::Int(7)).unwrap();
    for u in &first[1..] {
        one(&db, |cs| cs.archive(u.id, None)).unwrap();
    }
    clean(&db, "one active unit holds the value");
}

/// A record that does not decode is one structure finding, and the comparison is
/// not repeated on top of it as a pile of UNIT-50.
fn verify_does_not_pile_findings_on_damage<E: Target>() {
    let db = open::<E>();
    define(&db);
    let a = create(&db, doc("a", 1, "a@x")).unwrap();
    let (skey, sval) = stored(&db, 0x78).remove(0);
    let not_cross = |db: &UnitDb<E>, want: &str| {
        let r = rules(db);
        assert!(r.contains(want), "{want} expected in {r:?}");
        assert!(!r.contains("UNIT-50") && !r.contains("UNIT-51"), "a cross finding on top of the damage: {r:?}");
    };
    // A schema value that does not decode: its entries are not "entries of a field without a definition".
    patch(&db, vec![(skey.clone(), Some(vec![1, 0, 0, 5]))]);
    not_cross(&db, "UNIT-9");
    patch(&db, vec![(skey.clone(), Some(sval.clone()))]);
    // Keys that are not a schema key or an entry key.
    patch(&db, vec![(vec![0x78, 9, b'x'], Some(sval.clone()))]);
    not_cross(&db, "UNIT-3");
    patch(&db, vec![(vec![0x78, 9, b'x'], None), (vec![0x79, 1, b'a'], Some(ENTRY_VALUE.to_vec()))]);
    not_cross(&db, "UNIT-3");
    patch(&db, vec![(vec![0x79, 1, b'a'], None)]);
    clean(&db, "damage removed");
    // A head that does not decode: its type is unknown, so its entries cannot be judged.
    let head = Key::UnitHead { unit: a }.encode().unwrap();
    let good = db.raw_get(&head).unwrap();
    patch(&db, vec![(head.clone(), Some(vec![1, 0, 1]))]);
    let r = rules(&db);
    assert!(!r.is_empty() && !r.contains("UNIT-50") && !r.contains("UNIT-51"), "{r:?}");
    patch(&db, vec![(head, good)]);
    clean(&db, "head restored");
    // Entries of a unit type that has no schema, and of a field the schema does not declare.
    let ghost = entry_at("ghost", "age", &int_value(1), a);
    patch(&db, vec![(ghost.clone(), Some(ENTRY_VALUE.to_vec()))]);
    assert!(rules(&db).contains("UNIT-50"));
    patch(&db, vec![(ghost, None)]);
    let unknown = entry_at(ACCT, "unknown", &int_value(1), a);
    patch(&db, vec![(unknown.clone(), Some(ENTRY_VALUE.to_vec()))]);
    assert!(rules(&db).contains("UNIT-50"));
    patch(&db, vec![(unknown, None)]);
    clean(&db, "strays removed");
}

/// An entry the read path meets is checked against its unit whatever it lies
/// about: the unit's type, its existence, the value its payload holds.
fn the_read_path_refuses_every_lying_entry<E: Target>() {
    let db = open::<E>();
    define(&db);
    let a = create(&db, doc("a", 1, "a@x")).unwrap();
    // A unit of another type that holds a document with the same value.
    let other = one(&db, |cs| cs.create(new_unit("other", doc("x", 1, "x@x")))).unwrap();
    let lies = [
        ("a unit of another type", entry_at(ACCT, "age", &int_value(1), other)),
        ("a unit that does not exist", entry_at(ACCT, "age", &int_value(1), UnitId([9; 16]))),
        ("a value the unit does not hold", entry_at(ACCT, "age", &int_value(2), a)),
    ];
    for (what, key) in lies {
        patch(&db, vec![(key.clone(), Some(ENTRY_VALUE.to_vec()))]);
        let looked = db.snapshot().unwrap().find_range(
            ACCT,
            "age",
            std::ops::Bound::Unbounded,
            std::ops::Bound::Unbounded,
            usize::MAX,
        );
        assert!(is_corruption(&looked), "{what}: {looked:?}");
        assert!(rules(&db).contains("UNIT-50"), "{what}: {:?}", rules(&db));
        patch(&db, vec![(key, None)]);
        clean(&db, what);
    }
}

/// A lookup reads the entries a page at a time and stops at `limit` inside a page.
fn lookups_page_through_many_entries<E: Target>() {
    let db = open::<E>();
    define(&db);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let ids: Vec<UnitId> =
        (0..300).map(|n| cs.create(new_unit(ACCT, doc("n", 5, &format!("{n:04}@x")))).unwrap()).collect();
    cs.commit().unwrap();
    let mut by_id = ids.clone();
    by_id.sort();
    assert_eq!(found(&db, "age", &FieldValue::Int(5)), by_id);
    let snap = db.snapshot().unwrap();
    let (all, none) = (std::ops::Bound::Unbounded, std::ops::Bound::Unbounded);
    let limited = snap.find_range(ACCT, "age", all, none, 257).unwrap();
    assert_eq!(limited.iter().map(|u| u.id).collect::<Vec<_>>(), by_id[..257]);
    // Distinct values across pages come in value order.
    let by_email = snap.find_range(ACCT, "email", all, none, usize::MAX).unwrap();
    assert_eq!(by_email.iter().map(|u| u.id).collect::<Vec<_>>(), ids);
    assert!(snap.find_range(ACCT, "age", all, none, 0).unwrap().is_empty());
    clean(&db, "many entries");
    // A definition that reads the heads of many units a page at a time.
    let db = open::<E>();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for n in 0..300 {
        cs.create(new_unit(ACCT, doc("n", 5, &format!("{n:04}@x")))).unwrap();
    }
    cs.commit().unwrap();
    define(&db);
    assert_eq!(found(&db, "age", &FieldValue::Int(5)).len(), 300);
    clean(&db, "many units backfilled");
}

/// A definition validates the archived units too (so `unarchive` can only fail on
/// uniqueness) and ignores the deleted ones.
fn a_definition_checks_archived_units_and_ignores_deleted_ones<E: Target>() {
    let db = open::<E>();
    one(&db, |cs| cs.create(new_unit(ACCT, doc("a", 1, "a@x")))).unwrap();
    let junk = one(&db, |cs| cs.create(new_unit(ACCT, b"junk".to_vec()))).unwrap();
    one(&db, |cs| cs.archive(junk, None)).unwrap();
    let before = raw(&db);
    assert!(
        is_violation(&db.define_schema(ACCT, acct_fields(), ChangeMeta::default())),
        "an archived unit that does not fit"
    );
    assert_eq!(raw(&db), before);
    one(&db, |cs| cs.delete(junk, None)).unwrap();
    define(&db);
    clean(&db, "the deleted unit is ignored");
}

/// What a definition refuses as an argument stores nothing.
fn definitions_refuse_bad_arguments<E: Target>() {
    let db = open::<E>();
    one(&db, |cs| cs.create(new_unit("blob", vec![1]))).unwrap();
    let before = raw(&db);
    let meta = ChangeMeta::default;
    let invalid = |r: Result<CommitInfo, UnitError>| matches!(r, Err(UnitError::InvalidArgument(_)));
    let ok_fields = || vec![FieldDef::new("a", FieldType::Int)];
    assert!(invalid(db.define_schema("", ok_fields(), meta())));
    assert!(invalid(db.define_schema(&"t".repeat(65), ok_fields(), meta())));
    let twice = vec![FieldDef::new("a", FieldType::Int), FieldDef::new("a", FieldType::Text)];
    assert!(invalid(db.define_schema("t", twice, meta())));
    let many = (0..65).map(|i| FieldDef::new(format!("f{i}"), FieldType::Int)).collect();
    assert!(invalid(db.define_schema("t", many, meta())));
    assert!(invalid(db.define_schema("t", vec![FieldDef::new("", FieldType::Int)], meta())));
    // No schema to index or unindex.
    assert!(invalid(db.create_index("t", "a", meta())));
    assert!(invalid(db.drop_index("t", "a", meta())));
    assert!(invalid(db.create_index("", "a", meta())));
    assert_eq!(raw(&db), before);
    // A field the schema does not have indexes nothing (the first declared one is not indexed here).
    db.define_schema("t", vec![FieldDef::new("a", FieldType::Int), FieldDef::new("b", FieldType::Int)], meta())
        .unwrap();
    let defined = raw(&db);
    assert!(invalid(db.create_index("t", "nope", meta())));
    assert!(invalid(db.drop_index("t", "nope", meta())));
    assert_eq!(raw(&db), defined);
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            macro_rules! each {
                ($f:ident) => {
                    #[test]
                    fn $f() {
                        super::$f::<$e>();
                    }
                };
            }
            each!(a_refused_write_stores_nothing);
            each!(unique_holds_among_active_units);
            each!(the_index_follows_every_write);
            each!(ranges_are_in_value_order);
            each!(a_definition_backfills_or_refuses);
            each!(create_and_drop_index);
            each!(without_a_schema_nothing_changes);
            each!(verify_sees_damaged_indexes);
            each!(verify_sees_a_schema_the_store_breaks);
            each!(record_layout_is_pinned);
            each!(documents_are_canonical);
            each!(verify_sees_a_repeated_unique_value);
            each!(verify_does_not_pile_findings_on_damage);
            each!(the_read_path_refuses_every_lying_entry);
            each!(lookups_page_through_many_entries);
            each!(a_definition_checks_archived_units_and_ignores_deleted_ones);
            each!(definitions_refuse_bad_arguments);
        }
    )*};
}

engines!(emulated_log: kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>, kerno_db: kerno_storage::KernoDb);
