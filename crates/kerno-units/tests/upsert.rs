//! `ChangeSet::upsert`: store a unit, or update the active one that already holds the same value
//! of a unique field, in one change set. Each test runs on `emulated_log` and on `kerno_db`.

#![cfg(feature = "test-support")]

mod harness;

use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const ACCT: &str = "acct";

fn open<E: Target>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    let db = open_units::<E>(SimVfs::new(5).as_vfs(), &c).expect("open");
    let fields = vec![
        FieldDef::new("email", FieldType::Text).unique(),
        FieldDef::new("name", FieldType::Text),
        FieldDef::new("age", FieldType::Int).indexed(),
    ];
    db.define_schema(ACCT, fields, ChangeMeta::default()).expect("define the schema");
    db
}

fn doc(email: &str, name: &str) -> Vec<u8> {
    Document::new()
        .with("email", FieldValue::Text(email.into()))
        .with("name", FieldValue::Text(name.into()))
        .encode()
        .expect("encode")
}

fn new_unit(payload: Vec<u8>) -> NewUnit {
    NewUnit { unit_type: ACCT.into(), schema_version: 1, metadata: vec![], payload }
}

/// One upsert in its own change set.
fn upsert<E: Target>(db: &UnitDb<E>, field: &str, payload: Vec<u8>) -> Result<Upserted, UnitError> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    let done = cs.upsert(new_unit(payload), field)?;
    cs.commit()?;
    Ok(done)
}

fn revisions<E: Target>(db: &UnitDb<E>, id: UnitId) -> Vec<RevisionKind> {
    db.history(id, HistoryQuery::oldest(10)).expect("history").iter().map(|r| r.kind).collect()
}

fn create_update_unchanged<E: Target>() {
    let db = open::<E>();
    let made = upsert(&db, "email", doc("a@x.io", "Ada")).expect("create");
    assert_eq!(made.outcome, UpsertOutcome::Created);

    let same = upsert(&db, "email", doc("a@x.io", "Ada")).expect("unchanged");
    assert_eq!((same.id, same.outcome), (made.id, UpsertOutcome::Unchanged));
    assert_eq!(revisions(&db, made.id), [RevisionKind::Create], "an unchanged upsert writes no revision");

    let newer = upsert(&db, "email", doc("a@x.io", "Ada L.")).expect("update");
    assert_eq!((newer.id, newer.outcome), (made.id, UpsertOutcome::Updated));
    assert_eq!(revisions(&db, made.id), [RevisionKind::Create, RevisionKind::Update]);
    assert_eq!(db.get_unit(made.id).unwrap().unwrap().payload.unwrap(), doc("a@x.io", "Ada L."));

    let other = upsert(&db, "email", doc("b@x.io", "Bob")).expect("another key");
    assert_ne!(other.id, made.id);
    assert_eq!(other.outcome, UpsertOutcome::Created);
    assert!(db.verify_units().expect("verify").is_clean());
}

fn a_field_that_is_not_unique_is_refused<E: Target>() {
    let db = open::<E>();
    let invalid = |r: Result<Upserted, UnitError>| matches!(r, Err(UnitError::NotUnique { .. }));
    // Indexed but not unique, not indexed at all, not in the schema.
    for field in ["age", "name", "nope"] {
        assert!(invalid(upsert(&db, field, doc("a@x.io", "Ada"))), "{field}");
    }
    // A type with no schema.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let bare = NewUnit { unit_type: "bare".into(), ..new_unit(doc("a@x.io", "Ada")) };
    assert!(invalid(cs.upsert(bare, "email")));
    drop(cs);
    assert!(db.snapshot().unwrap().reference_filter(&UnitFilter::default()).unwrap().is_empty(), "nothing written");
}

fn a_payload_without_the_key_is_refused<E: Target>() {
    let db = open::<E>();
    let no_email = Document::new().with("name", FieldValue::Text("Ada".into())).encode().unwrap();
    for payload in [no_email, b"not a record".to_vec()] {
        let r = upsert(&db, "email", payload);
        assert!(matches!(r, Err(UnitError::SchemaViolation { .. })), "{r:?}");
    }
}

fn a_key_stored_earlier_in_the_change_set_is_found<E: Target>() {
    let db = open::<E>();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let first = cs.upsert(new_unit(doc("a@x.io", "Ada")), "email").unwrap();
    // The same payload resolves to the unit just stored.
    let again = cs.upsert(new_unit(doc("a@x.io", "Ada")), "email").unwrap();
    assert_eq!((again.id, again.outcome), (first.id, UpsertOutcome::Unchanged));
    // A different payload would be a second revision of one unit in one change set.
    let r = cs.upsert(new_unit(doc("a@x.io", "Other")), "email");
    assert!(matches!(r, Err(UnitError::InvalidArgument(_))), "{r:?}");
    cs.commit().unwrap();
    let all = db.snapshot().unwrap().reference_filter(&UnitFilter::default()).unwrap();
    assert_eq!(all.len(), 1);
}

fn only_an_active_unit_holds_the_key<E: Target>() {
    let db = open::<E>();
    let old = upsert(&db, "email", doc("a@x.io", "Ada")).unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(old.id, None).unwrap();
    cs.commit().unwrap();
    let fresh = upsert(&db, "email", doc("a@x.io", "Ada")).expect("an archived unit does not hold the key");
    assert_eq!(fresh.outcome, UpsertOutcome::Created);
    assert_ne!(fresh.id, old.id);
}

fn a_refused_upsert_writes_nothing<E: Target>() {
    let db = open::<E>();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.upsert(new_unit(doc("a@x.io", "Ada")), "email").unwrap();
    assert!(cs.upsert(new_unit(doc("b@x.io", "Bob")), "name").is_err());
    drop(cs);
    assert!(db.snapshot().unwrap().reference_filter(&UnitFilter::default()).unwrap().is_empty());
}

/// Within one change set, a unique value its own writes freed can be taken, and one they took
/// cannot: what the change set wrote counts over what was stored before it.
fn values_freed_and_taken_in_the_change_set_count<E: Target>() {
    let db = open::<E>();
    let a = upsert(&db, "email", doc("a@x.io", "Ada")).unwrap().id;
    let b = upsert(&db, "email", doc("b@x.io", "Bob")).unwrap().id;
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let upd = UnitUpdate { payload: doc("c@x.io", "Ada"), schema_version: None, metadata: None };
    cs.update(a, upd, None).unwrap();
    // a@ is free now: a new unit takes it.
    let fresh = cs.upsert(new_unit(doc("a@x.io", "Another")), "email").unwrap();
    assert_eq!(fresh.outcome, UpsertOutcome::Created);
    assert_ne!(fresh.id, a);
    // c@ is taken by this change set's update, b@ by the stored unit.
    assert!(matches!(cs.create(new_unit(doc("c@x.io", "X"))), Err(UnitError::SchemaViolation { .. })));
    let mut cs = {
        drop(cs);
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.update(a, UnitUpdate { payload: doc("c@x.io", "Ada"), schema_version: None, metadata: None }, None).unwrap();
        cs
    };
    assert!(matches!(cs.create(new_unit(doc("b@x.io", "X"))), Err(UnitError::SchemaViolation { .. })));
    let mut cs = {
        drop(cs);
        db.begin_changeset(ChangeMeta::default()).unwrap()
    };
    cs.delete(b, None).unwrap();
    let again = cs.upsert(new_unit(doc("b@x.io", "Bea")), "email").unwrap();
    assert_eq!(again.outcome, UpsertOutcome::Created, "the deletion freed b@");
    assert!(matches!(cs.create(new_unit(doc("b@x.io", "X"))), Err(UnitError::SchemaViolation { .. })));
    drop(cs);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(b, None).unwrap();
    let bea = cs.upsert(new_unit(doc("b@x.io", "Bea")), "email").unwrap().id;
    cs.commit().unwrap();
    assert_eq!(upsert(&db, "email", doc("b@x.io", "Bea")).unwrap().id, bea);
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
            each!(create_update_unchanged);
            each!(a_field_that_is_not_unique_is_refused);
            each!(a_payload_without_the_key_is_refused);
            each!(a_key_stored_earlier_in_the_change_set_is_found);
            each!(only_an_active_unit_holds_the_key);
            each!(a_refused_upsert_writes_nothing);
            each!(values_freed_and_taken_in_the_change_set_count);
        }
    )*};
}

engines!(emulated_log: kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>, kerno_db: kerno_storage::KernoDb);
