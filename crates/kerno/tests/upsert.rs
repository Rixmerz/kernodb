//! `Batch::upsert` and `Db::upsert`: store a unit, or revise the active one that holds the same
//! value of a unique field, atomically with the rest of the batch.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use kerno::units::{HistoryQuery, RevisionKind};
use kerno::{Batch, CommitMeta, Db, Document, Error, FieldDef, FieldType, FieldValue, UnitId, UpsertOutcome};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kerno-upsert-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("create the test directory");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(dir: &TempDir) -> Db {
    let db = Db::open(&dir.0).expect("open");
    if db.unit_db().snapshot().unwrap().schema("user").unwrap().is_none() {
        let fields = vec![
            FieldDef::new("email", FieldType::Text).unique().required(),
            FieldDef::new("name", FieldType::Text),
            FieldDef::new("age", FieldType::Int).indexed(),
        ];
        db.define_schema("user", fields).expect("define the schema");
    }
    db
}

fn user(email: &str, name: &str) -> Document {
    Document::new().with("email", FieldValue::Text(email.into())).with("name", FieldValue::Text(name.into()))
}

fn email(e: &str) -> FieldValue {
    FieldValue::Text(e.into())
}

fn kinds(db: &Db, id: UnitId) -> Vec<RevisionKind> {
    db.unit_db().history(id, HistoryQuery::oldest(10)).unwrap().iter().map(|r| r.kind).collect()
}

#[test]
fn create_update_and_unchanged_report_the_unit_and_what_happened() {
    let dir = TempDir::new();
    let db = open(&dir);

    let made = db.upsert_document("user", "email", &user("a@x.io", "Ada")).unwrap();
    assert_eq!(made.outcome, UpsertOutcome::Created);
    assert_eq!(db.find("user", "email", &email("a@x.io")).unwrap()[0].id, made.id);

    let same = db.upsert_document("user", "email", &user("a@x.io", "Ada")).unwrap();
    assert_eq!((same.id, same.outcome), (made.id, UpsertOutcome::Unchanged));
    assert_eq!(kinds(&db, made.id), [RevisionKind::Create], "an unchanged upsert writes no revision");

    let seq = db.last_commit_seq().unwrap();
    let newer = db.upsert_document("user", "email", &user("a@x.io", "Ada L.")).unwrap();
    assert_eq!((newer.id, newer.outcome), (made.id, UpsertOutcome::Updated));
    assert_eq!(db.last_commit_seq().unwrap(), seq + 1);
    assert_eq!(kinds(&db, made.id), [RevisionKind::Create, RevisionKind::Update]);
    assert_eq!(db.get(made.id).unwrap().unwrap().payload.unwrap(), user("a@x.io", "Ada L.").encode().unwrap());

    let other = db.upsert_document("user", "email", &user("b@x.io", "Bob")).unwrap();
    assert_ne!(other.id, made.id);
    assert_eq!(other.outcome, UpsertOutcome::Created);
    assert!(db.verify().unwrap().is_clean());
}

#[test]
fn an_unchanged_upsert_writes_nothing_and_counts_as_unchanged() {
    let dir = TempDir::new();
    let db = open(&dir);
    db.upsert_document("user", "email", &user("a@x.io", "Ada")).unwrap();
    let seq = db.last_commit_seq().unwrap();

    let mut batch = Batch::new();
    batch.upsert("user", "email", user("a@x.io", "Ada").encode().unwrap());
    let done = db.commit_with(&batch, &CommitMeta::default()).unwrap();
    assert_eq!((done.unchanged, done.commit_seq, done.upserts.len()), (1, None, 1));
    assert!(done.units.is_empty(), "an upsert is not a store");
    assert_eq!(db.last_commit_seq().unwrap(), seq);
}

#[test]
fn a_field_that_is_not_unique_is_refused_and_nothing_is_written() {
    let dir = TempDir::new();
    let db = open(&dir);
    let payload = user("a@x.io", "Ada").encode().unwrap();
    // Indexed but not unique, not indexed, not in the schema, and a type with no schema.
    for (unit_type, field) in [("user", "age"), ("user", "name"), ("user", "nope"), ("bare", "email")] {
        let r = db.upsert(unit_type, field, payload.clone());
        assert!(matches!(r, Err(Error::NotUnique { .. })), "{unit_type}.{field}: {r:?}");
    }
    // In a batch the refusal abandons the stores before it.
    let mut batch = Batch::new();
    batch.store("note", "kept out").upsert("user", "name", payload);
    assert!(matches!(db.commit(&batch), Err(Error::NotUnique { .. })));
    assert!(db.query(&Default::default()).unwrap().is_empty(), "nothing was written");
}

#[test]
fn a_payload_without_the_key_is_a_schema_violation() {
    let dir = TempDir::new();
    let db = open(&dir);
    let no_email = Document::new().with("name", FieldValue::Text("Ada".into())).encode().unwrap();
    for payload in [no_email, b"not a record".to_vec()] {
        let r = db.upsert("user", "email", payload);
        assert!(matches!(r, Err(Error::SchemaViolation { .. })), "{r:?}");
    }
}

#[test]
fn the_same_key_twice_in_one_batch_is_one_unit_and_the_last_payload_wins() {
    let dir = TempDir::new();
    let db = open(&dir);
    let enc = |e: &str, n: &str| user(e, n).encode().unwrap();

    let mut batch = Batch::new();
    batch.upsert("user", "email", enc("a@x.io", "first"));
    batch.upsert("user", "email", enc("b@x.io", "Bob"));
    batch.upsert("user", "email", enc("a@x.io", "second"));
    let done = db.commit_with(&batch, &CommitMeta::default()).unwrap();
    assert_eq!(done.upserts.len(), 3);
    assert_eq!(done.upserts[0], done.upserts[2]);
    assert_ne!(done.upserts[0].id, done.upserts[1].id);
    assert!(done.upserts.iter().all(|u| u.outcome == UpsertOutcome::Created));
    let a = done.upserts[0].id;
    assert_eq!(db.get(a).unwrap().unwrap().payload.unwrap(), enc("a@x.io", "second"));
    assert_eq!(kinds(&db, a), [RevisionKind::Create], "one revision, not one per upsert");
    assert_eq!(db.find("user", "email", &email("a@x.io")).unwrap().len(), 1);

    // Against a unit that exists: still one revision, still one unit.
    let mut again = Batch::new();
    again.upsert("user", "email", enc("a@x.io", "third")).upsert("user", "email", enc("a@x.io", "fourth"));
    let done = db.commit_with(&again, &CommitMeta::default()).unwrap();
    assert_eq!(done.upserts[0].id, a);
    assert_eq!((done.upserts[0], done.upserts[1]), (done.upserts[0], done.upserts[0]));
    assert_eq!(done.upserts[0].outcome, UpsertOutcome::Updated);
    assert_eq!(kinds(&db, a), [RevisionKind::Create, RevisionKind::Update]);
    assert_eq!(db.get(a).unwrap().unwrap().payload.unwrap(), enc("a@x.io", "fourth"));
}

#[test]
fn an_upsert_commits_atomically_with_the_rest_of_the_batch() {
    let dir = TempDir::new();
    let db = open(&dir);
    let note = db.store("note", "v1").unwrap();
    let mut batch = Batch::new();
    batch.upsert("user", "email", user("a@x.io", "Ada").encode().unwrap()).update(note, "v2");
    let done = db.commit_with(&batch, &CommitMeta::default()).unwrap();
    assert_eq!(done.commit_seq, Some(db.last_commit_seq().unwrap()));
    assert_eq!(db.get(note).unwrap().unwrap().payload.unwrap(), b"v2");
    assert_eq!(db.find("user", "email", &email("a@x.io")).unwrap().len(), 1);

    // The upsert applies, then a later operation fails: neither is visible.
    let mut bad = Batch::new();
    bad.upsert("user", "email", user("c@x.io", "Cy").encode().unwrap()).update(UnitId([9; 16]), "gone");
    assert!(db.commit(&bad).is_err());
    assert!(db.find("user", "email", &email("c@x.io")).unwrap().is_empty());
}

#[test]
fn an_upsert_of_a_unit_the_batch_already_changed_is_refused() {
    let dir = TempDir::new();
    let db = open(&dir);
    let id = db.upsert_document("user", "email", &user("a@x.io", "Ada")).unwrap().id;
    let mut batch = Batch::new();
    batch.update(id, user("a@x.io", "Ada L.").encode().unwrap()).upsert(
        "user",
        "email",
        user("a@x.io", "Other").encode().unwrap(),
    );
    assert!(matches!(db.commit(&batch), Err(Error::InvalidArgument(_))));
}

#[test]
fn a_dropped_handle_and_a_reopen_keep_one_unit_and_its_history() {
    let dir = TempDir::new();
    let id = {
        let db = open(&dir);
        let id = db.upsert_document("user", "email", &user("a@x.io", "Ada")).unwrap().id;
        db.upsert_document("user", "email", &user("a@x.io", "Ada L.")).unwrap();
        id // dropped without close: a crash as far as the files go
    };
    let db = open(&dir);
    let again = db.upsert_document("user", "email", &user("a@x.io", "Ada L.")).unwrap();
    assert_eq!((again.id, again.outcome), (id, UpsertOutcome::Unchanged));
    assert_eq!(db.find("user", "email", &email("a@x.io")).unwrap().len(), 1);
    assert_eq!(kinds(&db, id), [RevisionKind::Create, RevisionKind::Update]);
    assert!(db.verify().unwrap().is_clean());
}
