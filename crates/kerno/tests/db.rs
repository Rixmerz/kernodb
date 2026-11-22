//! The facade through its public API only, on real directories.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use kerno::units::{AnnConfig, AnnIndex, AnnMetric};
use std::ops::Bound;

use kerno::{
    AccessMode, AssocQuery, Batch, Config, Db, Direction, Document, Error, FieldDef, FieldType, FieldValue,
    HybridQuery, Lifecycle, UnitFilter, VectorLeg,
};
use kerno_core::Durability;
use kerno_core::contract::Options;
use kerno_core::vfs::StdVfs;

/// A fresh directory, removed when the test ends, pass or fail.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kerno-facade-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("create the test directory");
        TempDir(dir)
    }

    fn db(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn of_type(unit_type: &str) -> UnitFilter {
    UnitFilter { unit_type: Some(unit_type.to_owned()), ..UnitFilter::default() }
}

#[test]
fn store_then_get_returns_the_unit() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let id = db.store("note", "hello").unwrap();
    let unit = db.get(id).unwrap().expect("the unit exists");
    assert_eq!(unit.id, id);
    assert_eq!(unit.unit_type, "note");
    assert_eq!(unit.lifecycle, Lifecycle::Active);
    assert_eq!(unit.payload.as_deref(), Some(&b"hello"[..]));
}

#[test]
fn get_of_an_unknown_id_is_none() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let other = TempDir::new();
    let foreign = {
        let db2 = Db::open(other.db()).unwrap();
        let id = db2.store("note", "elsewhere").unwrap();
        db2.close().unwrap();
        id
    };
    assert!(db.get(foreign).unwrap().is_none());
}

#[test]
fn units_survive_close_and_reopen() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let a = db.store("note", b"bytes".to_vec()).unwrap();
    let b = db.store("topic", "t").unwrap();
    db.relate(a, b, "about").unwrap();
    db.close().unwrap();

    let db = Db::open(dir.db()).unwrap();
    assert_eq!(db.get(a).unwrap().unwrap().payload.as_deref(), Some(&b"bytes"[..]));
    assert_eq!(db.related(a, Direction::Out, None).unwrap().len(), 1);
}

#[test]
fn relate_is_visible_from_both_ends_and_filters_by_type() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let note = db.store("note", "n").unwrap();
    let topic = db.store("topic", "t").unwrap();
    let edge = db.relate(note, topic, "about").unwrap();
    db.relate(note, topic, "cites").unwrap();

    let out = db.related(note, Direction::Out, Some("about")).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!((out[0].id, out[0].from, out[0].to), (edge, note, topic));

    let incoming = db.related(topic, Direction::In, None).unwrap();
    assert_eq!(incoming.len(), 2);
    assert!(db.related(topic, Direction::Out, None).unwrap().is_empty());
}

#[test]
fn query_filters_by_type_and_lists_every_active_unit_without_a_filter() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let n1 = db.store("note", "1").unwrap();
    let n2 = db.store("note", "2").unwrap();
    db.store("topic", "t").unwrap();

    let mut notes: Vec<_> = db.query(&of_type("note")).unwrap().into_iter().map(|u| u.id).collect();
    notes.sort();
    let mut expected = vec![n1, n2];
    expected.sort();
    assert_eq!(notes, expected);
    assert_eq!(db.query(&UnitFilter::default()).unwrap().len(), 3);
    assert!(db.query(&of_type("absent")).unwrap().is_empty());
}

#[test]
fn invalid_input_is_an_error_and_writes_nothing() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    assert!(db.store("", "x").is_err());
    assert!(db.store(&"t".repeat(65), "x").is_err());
    let a = db.store("note", "a").unwrap();
    assert!(db.relate(a, a, "").is_err());
    assert!(db.related(a, Direction::Out, None).unwrap().is_empty());
    assert_eq!(db.query(&UnitFilter::default()).unwrap().len(), 1);
}

#[test]
fn relating_a_unit_that_does_not_exist_fails() {
    let dir = TempDir::new();
    let other = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let a = db.store("note", "a").unwrap();
    let foreign = Db::open(other.db()).unwrap().store("note", "b").unwrap();
    assert!(db.relate(a, foreign, "about").is_err());
    assert!(db.related(a, Direction::Out, None).unwrap().is_empty());
}

#[test]
fn a_second_open_of_a_held_directory_fails() {
    let dir = TempDir::new();
    let _held = Db::open(dir.db()).unwrap();
    assert!(Db::open(dir.db()).is_err());
}

#[test]
fn open_with_read_only_reads_and_refuses_a_missing_database() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let id = db.store("note", "kept").unwrap();
    db.close().unwrap();

    let read_only = Config {
        access: AccessMode::ReadOnly,
        create_if_missing: false,
        allow_unverified_platform: true,
        ..Config::default()
    };
    let db = Db::open_with(dir.db(), read_only.clone()).unwrap();
    assert_eq!(db.get(id).unwrap().unwrap().payload.as_deref(), Some(&b"kept"[..]));
    assert!(db.store("note", "no").is_err());
    db.close().unwrap();

    let empty = TempDir::new();
    assert!(Db::open_with(empty.db().join("none"), read_only).is_err());
}

#[test]
fn the_layers_stay_reachable_through_the_facade() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let id = db.store("note", "v1").unwrap();
    let snapshot = db.unit_db().snapshot().unwrap();
    assert_eq!(snapshot.get_unit(id).unwrap().unwrap().head, kerno::units::RevNo(1));
}

#[test]
fn hybrid_search_fuses_text_hnsw_and_graph() {
    let dir = TempDir::new();
    let db = Db::open(dir.db().join("db")).unwrap();
    db.set_fulltext("note", true).unwrap(); // opt-in per type (ADR-0023)
    let a = db.store("note", "rust borrow checker").unwrap();
    let b = db.store("note", "garbage collector").unwrap();
    let c = db.store("note", "rust ownership").unwrap();
    let s = db.store("topic", "memory").unwrap();
    db.relate(b, s, "about").unwrap();

    let opts = Options { durability: Durability::Buffered, ..Options::default() };
    let cfg = AnnConfig { seed: 1, ..AnnConfig::new(3, AnnMetric::Cosine) };
    let mut index =
        AnnIndex::<kerno::storage::KernoDb>::open(StdVfs::shared(), &dir.db().join("ann"), &opts, cfg).unwrap();
    index.insert(a, [0; 32], &[1.0, 0.0, 0.0]).unwrap();
    index.insert(b, [0; 32], &[0.0, 1.0, 0.0]).unwrap();
    index.insert(c, [0; 32], &[0.9, 0.1, 0.0]).unwrap();

    let query_vector = [1.0, 0.0, 0.0];
    let mut query = HybridQuery::new(10);
    query.text = Some("rust");
    query.vector = Some(VectorLeg { index: &index, query: &query_vector });
    query.graph = Some(AssocQuery::new(vec![s], 1, 10, 1000));
    let hits = db.hybrid_search(&query).unwrap();

    let ranks = |unit| hits.iter().find(|h| h.unit == unit).map(|h| (h.ranks.keyword, h.ranks.vector, h.ranks.graph));
    assert_eq!(ranks(a), Some((Some(2), Some(1), None)));
    assert_eq!(ranks(c), Some((Some(1), Some(2), None)));
    assert_eq!(ranks(b), Some((None, Some(3), Some(1))));
    assert_eq!(ranks(s), None, "the seed is not a hit");
    let scores: Vec<u64> = hits.iter().map(|h| h.score).collect();
    assert_eq!(scores, [32_522_474, 32_522_474, 32_266_457]);
    let order: Vec<_> = hits.iter().map(|h| h.unit).collect();
    assert_eq!(order, [a.min(c), a.max(c), b]);

    index.close().unwrap();
    db.close().unwrap();
}

#[test]
fn hybrid_search_refuses_an_invalid_query() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let mut query = HybridQuery::new(0);
    query.text = Some("rust");
    assert!(matches!(db.hybrid_search(&query), Err(Error::InvalidArgument(_))));
}

fn user(email: &str, age: i64) -> Document {
    Document::new().with("email", FieldValue::Text(email.into())).with("age", FieldValue::Int(age))
}

fn user_fields() -> Vec<FieldDef> {
    vec![FieldDef::new("email", FieldType::Text).unique().required(), FieldDef::new("age", FieldType::Int).indexed()]
}

#[test]
fn a_schema_validates_writes_and_serves_lookups_through_a_reopen() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("user", user_fields()).unwrap();
    let ada = db.store_document("user", &user("ada@x", 36)).unwrap();
    let bob = db.store_document("user", &user("bob@x", 41)).unwrap();
    let cy = db.store_document("user", &user("cy@x", 36)).unwrap();
    let ids = |units: Vec<kerno::Unit>| units.into_iter().map(|u| u.id).collect::<Vec<_>>();

    assert_eq!(ids(db.find("user", "age", &FieldValue::Int(36)).unwrap()), vec![ada, cy]);
    assert_eq!(ids(db.find("user", "email", &FieldValue::Text("bob@x".into())).unwrap()), vec![bob]);
    let (lo, hi) = (FieldValue::Int(37), FieldValue::Int(99));
    let range = db.find_range("user", "age", Bound::Included(&lo), Bound::Included(&hi), 10).unwrap();
    assert_eq!(ids(range), vec![bob]);

    // Refused writes: a repeated unique value, a missing required field, a wrong
    // type, bytes that are not a document. None of them stores a unit.
    let before = db.query(&of_type("user")).unwrap().len();
    let dup = db.store_document("user", &user("ada@x", 1));
    assert!(matches!(dup, Err(Error::SchemaViolation { .. })), "{dup:?}");
    let no_email = db.store_document("user", &Document::new().with("age", FieldValue::Int(1)));
    assert!(matches!(no_email, Err(Error::SchemaViolation { .. })), "{no_email:?}");
    let wrong = Document::new().with("email", FieldValue::Int(1));
    assert!(matches!(db.store_document("user", &wrong), Err(Error::SchemaViolation { .. })));
    assert!(matches!(db.store("user", "free text"), Err(Error::SchemaViolation { .. })));
    assert_eq!(db.query(&of_type("user")).unwrap().len(), before);

    // A type without a schema is as opaque as before.
    db.store("note", "free text").unwrap();
    assert!(db.schema("note").unwrap().is_none());
    let schema = db.schema("user").unwrap().expect("the schema");
    assert_eq!(schema.fields().len(), 2);
    assert!(schema.field("email").is_some_and(|f| f.unique && f.indexed && f.required));

    // Closed cleanly and dropped without closing: the schema, the units and the
    // index entries are all there, and the checks find nothing.
    db.close().unwrap();
    let db = Db::open(dir.db()).unwrap();
    assert_eq!(ids(db.find("user", "age", &FieldValue::Int(36)).unwrap()), vec![ada, cy]);
    assert!(db.unit_db().verify_units().unwrap().is_clean());
    drop(db);
    let db = Db::open(dir.db()).unwrap();
    assert!(matches!(db.store_document("user", &user("cy@x", 2)), Err(Error::SchemaViolation { .. })));
    assert_eq!(ids(db.find("user", "age", &FieldValue::Int(36)).unwrap()), vec![ada, cy]);
    assert!(db.unit_db().verify_units().unwrap().is_clean());
}

#[test]
fn definitions_over_existing_units_backfill_or_refuse() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    // Free text under the type: no schema accepts it.
    db.store("user", "free text").unwrap();
    let refused = db.define_schema("user", user_fields());
    assert!(matches!(refused, Err(Error::SchemaViolation { .. })), "{refused:?}");
    assert!(db.schema("user").unwrap().is_none(), "a refused definition stores nothing");

    let other = TempDir::new();
    let db = {
        db.close().unwrap();
        Db::open(other.db()).unwrap()
    };
    let ada = db.store_document("user", &user("ada@x", 36)).unwrap();
    db.define_schema("user", vec![FieldDef::new("email", FieldType::Text), FieldDef::new("age", FieldType::Int)])
        .unwrap();
    // `age` was not indexed: asking is an error, not an empty answer.
    assert!(matches!(db.find("user", "age", &FieldValue::Int(36)), Err(Error::NotIndexed { .. })));
    db.create_index("user", "age").unwrap();
    assert_eq!(db.find("user", "age", &FieldValue::Int(36)).unwrap()[0].id, ada);
    assert!(matches!(db.create_index("user", "age"), Err(Error::InvalidArgument(_))));
    assert!(matches!(db.create_index("user", "nope"), Err(Error::InvalidArgument(_))));
    db.drop_index("user", "age").unwrap();
    assert!(matches!(db.find("user", "age", &FieldValue::Int(36)), Err(Error::NotIndexed { .. })));
    assert!(matches!(db.define_schema("user", user_fields()), Err(Error::SchemaExists { .. })));
    assert!(db.unit_db().verify_units().unwrap().is_clean());
}

#[test]
fn the_query_language_joins_filters_orders_and_explains_through_a_reopen() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("user", user_fields()).unwrap();
    db.define_schema("team", vec![FieldDef::new("name", FieldType::Text).unique().required()]).unwrap();
    let team =
        |name: &str| db.store_document("team", &Document::new().with("name", FieldValue::Text(name.into()))).unwrap();
    let (core, ops) = (team("core"), team("ops"));
    let mut users = Vec::new();
    for (email, age, team) in [("ada@x", 36, core), ("bob@x", 41, core), ("cy@x", 29, ops), ("dee@x", 36, ops)] {
        let id = db.store_document("user", &user(email, age)).unwrap();
        db.relate(id, team, "member_of").unwrap();
        users.push(id);
    }
    let ids =
        |rows: &kerno::QueryResult| rows.rows.iter().map(|r| r.iter().map(|u| u.id).collect()).collect::<Vec<Vec<_>>>();

    let q = "FIND user AS u JOIN team AS t VIA member_of WHERE u.age >= 36 ORDER BY u.age DESC, u.email LIMIT 2";
    let rows = db.execute(q).unwrap();
    assert_eq!(rows.aliases, ["u", "t"]);
    assert_eq!(ids(&rows), vec![vec![users[1], core], vec![users[0], core]]);
    // The other direction, with a condition on the joined side.
    let rows =
        db.execute("FIND team AS t JOIN user AS u VIA member_of IN WHERE t.name = \"ops\" ORDER BY u.email").unwrap();
    assert_eq!(ids(&rows), vec![vec![ops, users[2]], vec![ops, users[3]]]);

    // The index narrows and the plan says so; the answer is the same.
    let plan = db.explain("FIND user WHERE email = \"cy@x\"").unwrap();
    assert_eq!(plan.chosen().access, kerno::units::Access::FieldIndex { field: "email".into() });
    assert_eq!(plan.candidates.len(), 2);
    assert_eq!(ids(&db.execute("FIND user WHERE email = \"cy@x\"").unwrap()), vec![vec![users[2]]]);
    assert_eq!(db.explain("FIND user JOIN team VIA member_of").unwrap().joins.len(), 1);

    // Errors keep the layer's error type: the byte of a parse error, the field of a schema error.
    let parse = db.execute("FIND user WHERE").unwrap_err();
    assert!(matches!(&parse, Error::InvalidArgument(m) if m.contains("at byte 15")), "{parse:?}");
    assert!(matches!(db.explain("FIND user WHERE").unwrap_err(), Error::InvalidArgument(_)));
    let field = db.execute("FIND user WHERE nope = 1").unwrap_err();
    assert!(matches!(&field, Error::InvalidArgument(m) if m.contains("declares no such field")), "{field:?}");

    db.close().unwrap();
    let db = Db::open(dir.db()).unwrap();
    assert_eq!(ids(&db.execute(q).unwrap()), vec![vec![users[1], core], vec![users[0], core]]);
}

fn keyword_hits(db: &Db, query: &str) -> Vec<kerno::UnitId> {
    let snapshot = db.unit_db().snapshot().unwrap();
    snapshot.search_text(query, 10, None).unwrap().into_iter().map(|hit| hit.unit).collect()
}

#[test]
fn update_replaces_the_payload_and_keeps_the_full_text_index_in_step() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.set_fulltext("note", true).unwrap(); // opt-in per type (ADR-0023)
    let id = db.store("note", "alpha bravo").unwrap();
    assert_eq!(keyword_hits(&db, "alpha"), [id]);

    db.update(id, "charlie delta").unwrap();

    let unit = db.get(id).unwrap().expect("still there");
    assert_eq!(unit.payload.as_deref(), Some(&b"charlie delta"[..]));
    assert_eq!(unit.unit_type, "note");
    assert_eq!(unit.head.0, 2);
    assert!(keyword_hits(&db, "alpha").is_empty(), "the old terms are gone");
    assert_eq!(keyword_hits(&db, "charlie"), [id]);
    db.unit_db().snapshot().unwrap().verify_units().expect("the indexes agree with the units");
}

#[test]
fn update_of_an_unknown_or_deleted_unit_is_refused() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let id = db.store("note", "x").unwrap();
    db.delete(id).unwrap();
    assert!(db.update(id, "y").is_err());
    let other = Db::open(TempDir::new().db()).unwrap().store("note", "z").unwrap();
    assert!(matches!(db.update(other, "y"), Err(Error::NotFound { .. })));
}

#[test]
fn delete_leaves_a_tombstone_and_drops_the_unit_from_queries_and_keyword_search() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.set_fulltext("note", true).unwrap(); // opt-in per type (ADR-0023)
    let keep = db.store("note", "shared words").unwrap();
    let gone = db.store("note", "shared words").unwrap();

    db.delete(gone).unwrap();

    let unit = db.get(gone).unwrap().expect("the tombstone is readable");
    assert_eq!(unit.lifecycle, Lifecycle::Deleted);
    assert_eq!(unit.payload, None);
    let ids: Vec<_> = db.query(&of_type("note")).unwrap().into_iter().map(|u| u.id).collect();
    assert_eq!(ids, [keep]);
    assert_eq!(keyword_hits(&db, "shared"), [keep]);
    assert!(db.delete(gone).is_err(), "a second delete is refused");
    db.unit_db().snapshot().unwrap().verify_units().expect("the indexes agree with the units");
}

#[test]
fn delete_of_a_related_unit_is_refused_until_the_relation_is_removed() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let a = db.store("note", "a").unwrap();
    let b = db.store("note", "b").unwrap();
    let edge = db.relate(a, b, "about").unwrap();

    assert!(matches!(db.delete(a), Err(Error::UnitHasRelations { .. })));
    assert_eq!(db.get(a).unwrap().unwrap().lifecycle, Lifecycle::Active);

    db.unrelate(edge).unwrap();
    assert!(db.related(a, Direction::Out, None).unwrap().is_empty());
    db.delete(a).unwrap();
    assert!(db.unrelate(edge).is_err(), "the relation is already gone");
}

#[test]
fn update_document_checks_the_schema_and_a_refusal_changes_nothing() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("user", vec![FieldDef::new("age", FieldType::Int).indexed().required()]).unwrap();
    let id = db.store_document("user", &Document::new().with("age", FieldValue::Int(30))).unwrap();

    db.update_document(id, &Document::new().with("age", FieldValue::Int(31))).unwrap();
    assert!(db.find("user", "age", &FieldValue::Int(30)).unwrap().is_empty());
    assert_eq!(db.find("user", "age", &FieldValue::Int(31)).unwrap()[0].id, id);

    let bad = Document::new().with("age", FieldValue::Text("old".into()));
    assert!(matches!(db.update_document(id, &bad), Err(Error::SchemaViolation { .. })));
    assert_eq!(db.find("user", "age", &FieldValue::Int(31)).unwrap()[0].id, id);
}

#[test]
fn a_batch_applies_every_operation_in_one_commit() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.set_fulltext("note", true).unwrap(); // opt-in per type (ADR-0023)
    db.set_fulltext("topic", true).unwrap();
    let edited = db.store("note", "before").unwrap();
    let removed = db.store("note", "doomed").unwrap();
    let before = db.unit_db().raw_seq().unwrap();

    let mut batch = Batch::new();
    batch.store("note", "fresh one").update(edited, "after").delete(removed).store("topic", "fresh two");
    let stored = db.commit(&batch).unwrap();

    assert_eq!(stored.len(), 2);
    assert_eq!(db.unit_db().raw_seq().unwrap(), before + 1, "one commit for the whole batch");
    assert_eq!(db.get(stored[0]).unwrap().unwrap().unit_type, "note");
    assert_eq!(db.get(stored[1]).unwrap().unwrap().unit_type, "topic");
    assert_eq!(db.get(edited).unwrap().unwrap().payload.as_deref(), Some(&b"after"[..]));
    assert_eq!(db.get(removed).unwrap().unwrap().lifecycle, Lifecycle::Deleted);
    assert_eq!(keyword_hits(&db, "fresh"), [stored[0], stored[1]]);
    assert!(keyword_hits(&db, "doomed").is_empty());
    db.unit_db().snapshot().unwrap().verify_units().expect("the indexes agree with the units");
}

#[test]
fn a_batch_that_fails_writes_nothing() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.set_fulltext("note", true).unwrap(); // so the keyword asserts below can see a leak
    let id = db.store("note", "original").unwrap();
    let before = db.unit_db().raw_seq().unwrap();

    // The store and the update are valid; the delete of a unit that does not exist is not.
    let ghost = Db::open(TempDir::new().db()).unwrap().store("note", "ghost").unwrap();
    let mut batch = Batch::new();
    batch.store("note", "never").update(id, "changed").delete(ghost);
    assert!(matches!(db.commit(&batch), Err(Error::NotFound { .. })));

    assert_eq!(db.unit_db().raw_seq().unwrap(), before);
    assert_eq!(db.get(id).unwrap().unwrap().payload.as_deref(), Some(&b"original"[..]));
    assert_eq!(db.query(&of_type("note")).unwrap().len(), 1);
    assert!(keyword_hits(&db, "never").is_empty());
    assert!(keyword_hits(&db, "changed").is_empty());

    // Two operations on one unit are refused whole too.
    let mut twice = Batch::new();
    twice.update(id, "one").update(id, "two");
    assert!(matches!(db.commit(&twice), Err(Error::InvalidArgument(_))));
    assert_eq!(db.get(id).unwrap().unwrap().payload.as_deref(), Some(&b"original"[..]));
}

#[test]
fn an_empty_batch_writes_nothing() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let before = db.unit_db().raw_seq().unwrap();
    assert!(db.commit(Batch::new()).unwrap().is_empty());
    assert_eq!(db.unit_db().raw_seq().unwrap(), before);
}

#[test]
fn a_batch_can_unrelate_and_delete_together() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let a = db.store("note", "a").unwrap();
    let b = db.store("note", "b").unwrap();
    let edge = db.relate(a, b, "about").unwrap();

    db.commit(Batch::new().unrelate(edge).delete(a)).unwrap();
    assert_eq!(db.get(a).unwrap().unwrap().lifecycle, Lifecycle::Deleted);
    assert!(db.related(b, Direction::In, None).unwrap().is_empty());
}

/// An open change set from `unit_db()` makes a write on the same thread wait for the write slot
/// forever. `Batch` holds nothing, so building one between other writes must not hang; a worker
/// thread and a timeout turn a regression into a failure instead of a stuck test run.
#[test]
fn the_facade_writes_do_not_deadlock_on_one_thread() {
    use std::sync::mpsc;
    use std::time::Duration;

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let dir = TempDir::new();
        let db = Db::open(dir.db()).unwrap();
        let a = db.store("note", "a").unwrap();

        let mut batch = Batch::new();
        batch.update(a, "a2").store("note", "b");
        // Writes, reads and a second batch between building and committing the first.
        let c = db.store("note", "c").unwrap();
        db.update(c, "c2").unwrap();
        assert_eq!(db.get(a).unwrap().unwrap().payload.as_deref(), Some(&b"a"[..]));
        let mut other = Batch::new();
        other.delete(c);
        db.commit(&other).unwrap();
        let stored = db.commit(&batch).unwrap();
        db.update(stored[0], "b2").unwrap();
        db.delete(stored[0]).unwrap();
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(60)).expect("same-thread facade writes finished without deadlock");
}

#[test]
fn verify_reports_a_clean_database_clean() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let a = db.store("note", "a").unwrap();
    let b = db.store("note", "b").unwrap();
    db.relate(a, b, "cites").unwrap();
    let report = db.verify().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
    db.close().unwrap();
}

#[test]
fn verify_reports_a_removed_adjacency_entry() {
    use kerno::units::codec::Key;

    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let a = db.store("note", "a").unwrap();
    let b = db.store("note", "b").unwrap();
    let edge = db.relate(a, b, "cites").unwrap();
    assert!(db.verify().unwrap().is_clean());

    // Remove the incoming entry below the layer, as a lost write would.
    let key = Key::RelationIn { unit: b, relation_type: "cites".into(), edge }.encode().unwrap();
    db.unit_db().raw_commit(vec![(key, None)]).unwrap();

    let report = db.verify().unwrap();
    assert!(!report.is_clean());
    assert!(report.rules().contains("UNIT-36"), "{:?}", report.findings);
    db.close().unwrap();
}

#[test]
fn refresh_shows_a_read_only_handle_the_commits_made_since_it_opened() {
    let dir = TempDir::new();
    let writer = Db::open(dir.db()).unwrap();
    let first = writer.store("note", "one").unwrap();

    let read_only = Config { access: AccessMode::ReadOnly, allow_unverified_platform: true, ..Config::default() };
    let reader = Db::open_with(dir.db(), read_only).unwrap();
    let opened_at = reader.last_commit_seq().unwrap();
    assert!(reader.get(first).unwrap().is_some());

    let second = writer.store("note", "two").unwrap();
    assert!(reader.get(second).unwrap().is_none(), "not visible before refresh");
    assert_eq!(reader.last_commit_seq().unwrap(), opened_at);

    let seq = reader.refresh().unwrap();
    assert_eq!(seq, writer.last_commit_seq().unwrap());
    assert_eq!(reader.last_commit_seq().unwrap(), seq);
    assert_eq!(reader.get(second).unwrap().unwrap().payload.as_deref(), Some(&b"two"[..]));

    assert!(writer.refresh().is_err(), "a read-write handle is always current");
}

#[test]
fn run_retention_prunes_old_revisions_and_they_answer_pruned() {
    use kerno::units::{HistoryQuery, RevNo, UnitAt, UnitError};
    use kerno::{RetentionPolicy, RetentionRule};

    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let note = db.store("note", "v1").unwrap();
    let first_seq = db.last_commit_seq().unwrap();
    db.update(note, "v2").unwrap();
    db.update(note, "v3").unwrap();
    db.update(note, "v4").unwrap();
    let keep = db.store("log", "l1").unwrap();
    db.update(keep, "l2").unwrap();
    let before = db.last_commit_seq().unwrap();

    // Nothing is pruned until retention runs, and a refused policy writes nothing.
    assert!(matches!(db.read_as_of(note, first_seq).unwrap(), UnitAt::Present(_)));
    assert!(db.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(0))).is_err());
    assert_eq!(db.last_commit_seq().unwrap(), before);

    // `note` keeps its newest two revisions; `log` has no rule and keeps everything.
    let policy = RetentionPolicy::default().with_type("note", RetentionRule::KeepLast(2));
    let report = db.run_retention(&policy).unwrap();
    assert!(report.complete);
    assert_eq!((report.units_retained, report.revisions_pruned), (1, 2), "{report:?}");
    assert_eq!(db.get(note).unwrap().unwrap().oldest, RevNo(3));
    assert_eq!(db.get(keep).unwrap().unwrap().oldest, RevNo(1));

    assert!(matches!(db.read_as_of(note, first_seq).unwrap(), UnitAt::Pruned { oldest } if oldest == RevNo(3)));
    assert!(matches!(
        db.unit_db().history(note, HistoryQuery { from: Some(RevNo(1)), ..HistoryQuery::oldest(10) }),
        Err(UnitError::Pruned { .. })
    ));
    let revisions: Vec<u64> =
        db.unit_db().history(note, HistoryQuery::oldest(10)).unwrap().iter().map(|r| r.id.rev.0).collect();
    assert_eq!(revisions, [3, 4]);
    assert_eq!(db.get(note).unwrap().unwrap().payload.as_deref(), Some(&b"v4"[..]));
    assert!(db.verify().unwrap().is_clean());

    // A second run has nothing left to do and commits nothing.
    let seq = db.last_commit_seq().unwrap();
    let again = db.run_retention(&policy).unwrap();
    assert_eq!((again.units_retained, again.change_set), (0, None));
    assert_eq!(db.last_commit_seq().unwrap(), seq);

    // A duration rule: `ZERO` keeps only the revision in force now.
    db.run_retention(&RetentionPolicy::all(RetentionRule::KeepFor(std::time::Duration::ZERO))).unwrap();
    assert_eq!(db.get(note).unwrap().unwrap().oldest, RevNo(4));
    assert_eq!(db.get(keep).unwrap().unwrap().oldest, RevNo(2));
    assert!(db.verify().unwrap().is_clean());
}

// ---- gaps an outside user found in 1.0.0-alpha.3 ----

#[test]
fn compact_reports_the_size_at_call_time_not_the_size_at_the_last_flush() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    let blob = |n: usize| "x".repeat(1000 + n);
    for i in 0..100 {
        db.store("blob", blob(i)).unwrap();
    }
    // Nothing was flushed yet: every commit is still in the write buffer. (The payloads
    // compress, so the stored size is a fraction of the 100 KB written.)
    let first = db.compact().unwrap();
    let (before1, after1) = (first.stored_bytes_before.unwrap(), first.stored_bytes_after.unwrap());
    assert!(before1 > 5_000, "100 units are stored when compact is called, not {before1} bytes");
    assert!(before1.abs_diff(after1) < before1, "before {before1} and after {after1} measure the same data");

    for i in 0..1000 {
        db.store("blob", blob(i)).unwrap();
    }
    let before2 = db.compact().unwrap().stored_bytes_before.unwrap();
    assert!(before2 > 5 * after1, "1000 more units must show in before ({before2}, after {after1} units)");
}

#[test]
fn defining_the_same_schema_again_is_a_no_op_and_a_different_one_is_schema_exists() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("user", user_fields()).unwrap();
    let seq = db.last_commit_seq().unwrap();

    // The same fields, listed in another order, are the same schema: nothing is written.
    db.define_schema("user", user_fields().into_iter().rev().collect()).unwrap();
    assert_eq!(db.last_commit_seq().unwrap(), seq);

    let more = user_fields().into_iter().chain([FieldDef::new("name", FieldType::Text)]).collect();
    let changed = db.define_schema("user", more);
    assert!(matches!(changed, Err(Error::SchemaExists { .. })), "{changed:?}");
    assert_eq!(db.last_commit_seq().unwrap(), seq);

    // Once a field is indexed later, the schema in force is not the one first defined.
    db.define_schema("tag", vec![FieldDef::new("label", FieldType::Text)]).unwrap();
    db.create_index("tag", "label").unwrap();
    let old = db.define_schema("tag", vec![FieldDef::new("label", FieldType::Text)]);
    assert!(matches!(old, Err(Error::SchemaExists { .. })), "{old:?}");
    db.define_schema("tag", vec![FieldDef::new("label", FieldType::Text).indexed()]).unwrap();
}

#[test]
fn the_indexed_text_limit_is_254_bytes_and_the_error_says_so() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("lim", vec![FieldDef::new("key", FieldType::Text).unique().required()]).unwrap();
    let doc = |n: usize| Document::new().with("key", FieldValue::Text("a".repeat(n)));
    db.store_document("lim", &doc(254)).unwrap();
    let Err(Error::SchemaViolation { detail }) = db.store_document("lim", &doc(255)) else {
        panic!("255 bytes must be refused");
    };
    assert!(detail.contains("254 bytes"), "the error names the limit: {detail}");
    // A text of 254 bytes and one of 254 two-byte characters differ: the limit is in bytes.
    assert_eq!(kerno::units::schema::MAX_INDEXED_TEXT_LEN, 254);
}

#[test]
fn a_lookup_that_can_never_match_is_an_error_not_an_empty_answer() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("lim", vec![FieldDef::new("key", FieldType::Text).unique().required()]).unwrap();
    db.store_document("lim", &Document::new().with("key", FieldValue::Text("a".repeat(254)))).unwrap();

    let long = FieldValue::Text("b".repeat(300));
    assert!(matches!(db.find("lim", "key", &long), Err(Error::InvalidArgument(_))));
    let longest = FieldValue::Text("a".repeat(254));
    assert_eq!(db.find("lim", "key", &longest).unwrap().len(), 1);
    // A range bound this long still means something: everything up to it.
    let hits = db.find_range("lim", "key", Bound::Unbounded, Bound::Included(&long), 10).unwrap();
    assert_eq!(hits.len(), 1);

    // Keyword search with no type switched on can never match.
    let mut query = HybridQuery::new(5);
    query.text = Some("ada");
    assert!(matches!(db.hybrid_search(&query), Err(Error::NotIndexed { .. })));
    db.set_fulltext("lim", true).unwrap();
    assert!(db.hybrid_search(&query).unwrap().is_empty(), "a keyword-indexed type with no match is just empty");
}

#[test]
fn search_text_refuses_a_search_that_could_never_match() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.store("note", "needle in a haystack").unwrap();
    assert!(matches!(db.search_text("needle", 10, None), Err(Error::NotIndexed { .. })), "nothing is indexed");
    db.set_fulltext("note", true).unwrap();
    assert_eq!(db.search_text("needle", 10, None).unwrap().len(), 1);
    assert!(db.search_text("absent", 10, None).unwrap().is_empty(), "a real empty answer");
    for (query, limit) in [("needle", 0), ("", 10), (" ,, ", 10)] {
        assert!(matches!(db.search_text(query, limit, None), Err(Error::InvalidArgument(_))), "{query:?} {limit}");
    }
    assert!(matches!(db.search_text("needle", 10, Some(&[])), Err(Error::InvalidArgument(_))));
}

#[test]
fn search_text_and_hybrid_search_restrict_to_unit_types() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    for t in ["note", "memo"] {
        db.set_fulltext(t, true).unwrap();
    }
    let note = db.store("note", "shared words here").unwrap();
    let memo = db.store("memo", "shared words there").unwrap();
    db.store("plain", "shared words nowhere").unwrap(); // exists, never indexed

    let ids = |types: Option<&[&str]>| -> Vec<kerno::UnitId> {
        db.search_text("shared", 10, types).unwrap().into_iter().map(|h| h.unit).collect()
    };
    assert_eq!(ids(Some(&["note"])), vec![note]);
    assert_eq!(ids(Some(&["memo"])), vec![memo]);
    assert_eq!(ids(Some(&["note", "memo"])), ids(None));
    assert_eq!(ids(None).len(), 2);
    assert!(matches!(db.search_text("shared", 10, Some(&["ghost"])), Err(Error::UnknownType { .. })));
    assert!(matches!(db.search_text("shared", 10, Some(&["plain"])), Err(Error::NotIndexed { .. })));

    let types: &[&str] = &["note"];
    let mut query = HybridQuery::new(5);
    query.text = Some("shared");
    query.text_types = Some(types);
    assert_eq!(db.hybrid_search(&query).unwrap().iter().map(|h| h.unit).collect::<Vec<_>>(), vec![note]);
    let ghost: &[&str] = &["ghost"];
    query.text_types = Some(ghost);
    assert!(matches!(db.hybrid_search(&query), Err(Error::UnknownType { .. })));
    // `text_types` with no keyword leg to apply it to is refused, not ignored.
    let mut no_leg = HybridQuery::new(5);
    no_leg.text_types = Some(types);
    no_leg.graph = Some(AssocQuery::new(vec![note], 1, 10, 100));
    assert!(matches!(db.hybrid_search(&no_leg), Err(Error::InvalidArgument(_))));
}

#[test]
fn a_range_or_name_nothing_can_match_is_an_error_not_an_empty_answer() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("user", vec![FieldDef::new("age", FieldType::Int).indexed()]).unwrap();
    let (ten, thirty) = (FieldValue::Int(10), FieldValue::Int(30));
    let range = |lo, hi, limit| db.find_range("user", "age", lo, hi, limit);
    assert!(range(Bound::Included(&ten), Bound::Included(&thirty), 5).unwrap().is_empty(), "a possible range");
    assert!(matches!(range(Bound::Included(&thirty), Bound::Included(&ten), 5), Err(Error::InvalidArgument(_))));
    assert!(matches!(range(Bound::Excluded(&ten), Bound::Excluded(&ten), 5), Err(Error::InvalidArgument(_))));
    assert!(matches!(range(Bound::Unbounded, Bound::Unbounded, 0), Err(Error::InvalidArgument(_))));
    let a = db.store("a", "x").unwrap();
    assert!(matches!(db.related(a, Direction::Out, Some("")), Err(Error::InvalidArgument(_))));
    let long = "r".repeat(300);
    assert!(matches!(db.related(a, Direction::Out, Some(&long)), Err(Error::InvalidArgument(_))));
    assert!(matches!(db.related(a, Direction::In, Some("a\0b")), Err(Error::InvalidArgument(_))));
    assert!(matches!(db.relations(Some("")), Err(Error::InvalidArgument(_))));
    assert!(matches!(db.relations(Some(&long)), Err(Error::InvalidArgument(_))));
    assert!(db.relations(Some("ghost")).unwrap().is_empty(), "a valid name nothing holds");
    // An id no unit ever had is `NotFound`; a deleted unit is known and has no relations.
    let never = kerno::UnitId([0xde; 16]);
    assert!(matches!(db.related(never, Direction::Out, None), Err(Error::NotFound { .. })));
    assert!(matches!(db.related(never, Direction::In, Some("about")), Err(Error::NotFound { .. })));
    assert!(db.related(a, Direction::Out, None).unwrap().is_empty());
    let gone = db.store("a", "y").unwrap();
    db.delete(gone).unwrap();
    assert!(db.related(gone, Direction::Out, None).unwrap().is_empty());
    // The type scan: a name no unit can have is refused, a valid one nothing holds answers [].
    assert!(matches!(db.query(&of_type("")), Err(Error::InvalidArgument(_))));
    assert!(matches!(db.query(&of_type(&"t".repeat(65))), Err(Error::InvalidArgument(_))));
    assert!(db.query(&of_type("ghost")).unwrap().is_empty());
}

#[test]
fn distinct_refusals_have_distinct_errors() {
    let dir = TempDir::new();
    let db = Db::open(dir.db()).unwrap();
    db.define_schema("tag", vec![FieldDef::new("label", FieldType::Text)]).unwrap();
    let one = Document::new().with("label", FieldValue::Text("a".into()));
    let upsert = db.upsert_document("tag", "label", &one);
    assert!(matches!(upsert, Err(Error::NotUnique { .. })), "{upsert:?}");
    let find = db.find("tag", "label", &FieldValue::Text("a".into()));
    assert!(matches!(find, Err(Error::NotIndexed { .. })), "{find:?}");
    let range = db.find_range("tag", "label", Bound::Unbounded, Bound::Unbounded, 1);
    assert!(matches!(range, Err(Error::NotIndexed { .. })), "{range:?}");

    let mut big = Batch::new();
    for _ in 0..3 {
        big.store("n", "x");
    }
    let small = TempDir::new();
    let tiny = Db::open_with_limits(small.db(), Config::default(), 2).unwrap();
    assert!(matches!(tiny.commit(&big), Err(Error::ChangeSetTooLarge { .. })));
}

#[test]
fn a_write_on_a_read_only_handle_and_a_missing_database_are_named() {
    let dir = TempDir::new();
    Db::open(dir.db()).unwrap().close().unwrap();
    let read_only = Config {
        access: AccessMode::ReadOnly,
        create_if_missing: false,
        allow_unverified_platform: true,
        ..Config::default()
    };
    let db = Db::open_with(dir.db(), read_only.clone()).unwrap();
    let err = db.store("note", "no").unwrap_err();
    assert!(err.is_read_only(), "{err:?}");
    assert!(!err.is_no_database() && !err.is_locked());
    let refresh = Db::open(dir.db()).unwrap();
    // refresh on a handle that is not read-only is the engine refusing an argument.
    assert!(refresh.refresh().unwrap_err().is_engine_argument());
    drop(refresh);

    let empty = TempDir::new();
    let missing = Db::open_with(empty.db().join("none"), read_only).err().expect("no database there");
    assert!(missing.is_no_database(), "{missing:?}");
    assert!(!missing.to_string().contains("create_if_missing"), "{missing}");
}
