//! The batch operations added for applications that mirror their own data: relations inside a
//! batch, unique relations, cascading deletes, updates that skip unchanged payloads, archive and
//! restore, provenance, the last commit sequence and clamped as-of reads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use kerno::units::UnitAt;
use kerno::{Batch, CommitMeta, Db, Direction, Endpoint, Lifecycle};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kerno-batch-ext-{}-{}",
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

#[test]
fn relations_commit_with_their_units() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("n", "a").unwrap();
    let mut b = Batch::new();
    b.store("n", "b").store("n", "c");
    b.relate(Endpoint::Op(0), Endpoint::Op(1), "next").relate(a, Endpoint::Op(0), "next");
    let done = db.commit_with(&b, &CommitMeta::default()).unwrap();
    assert_eq!(done.units.len(), 2);
    assert_eq!(done.relations.len(), 2);
    assert_eq!(db.related(a, Direction::Out, Some("next")).unwrap()[0].to, done.units[0]);
    assert_eq!(done.commit_seq, Some(db.last_commit_seq().unwrap()));

    let mut bad = Batch::new();
    bad.relate(a, Endpoint::Op(5), "next");
    assert!(db.commit(&bad).is_err());
    assert_eq!(db.related(a, Direction::Out, None).unwrap().len(), 1, "a refused batch writes nothing");
}

#[test]
fn unique_relate_reuses_and_cascade_removes_once() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("n", "a").unwrap();
    let b = db.store("n", "b").unwrap();
    let first = db.relate(a, b, "r").unwrap();
    let mut batch = Batch::new();
    batch.relate_unique(a, b, "r").relate_unique(a, b, "r").relate_unique(b, a, "r");
    let done = db.commit_with(&batch, &CommitMeta::default()).unwrap();
    assert_eq!(done.relations[0], first);
    assert_eq!(done.relations[1], first);
    assert_eq!(db.related(a, Direction::Out, Some("r")).unwrap().len(), 1);

    // a <-> b both ways; deleting both cascades every relation exactly once.
    let mut del = Batch::new();
    del.delete_cascade(a).delete_cascade(b);
    db.commit(&del).unwrap();
    assert_eq!(db.get(a).unwrap().unwrap().lifecycle, Lifecycle::Deleted);
    assert_eq!(db.get(b).unwrap().unwrap().lifecycle, Lifecycle::Deleted);
    assert!(db.related(a, Direction::Out, None).unwrap().is_empty());
}

#[test]
fn unchanged_updates_add_no_revision() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("n", "same").unwrap();
    let mut batch = Batch::new();
    batch.update_if_changed(a, "same");
    let done = db.commit_with(&batch, &CommitMeta::default()).unwrap();
    assert_eq!(done.unchanged, 1);
    assert_eq!(done.commit_seq, None, "nothing written, no commit");
    assert_eq!(db.get(a).unwrap().unwrap().head.0, 1);
    let mut batch = Batch::new();
    batch.update_if_changed(a, "new");
    db.commit(&batch).unwrap();
    assert_eq!(db.get(a).unwrap().unwrap().head.0, 2);
}

#[test]
fn archive_restore_and_provenance() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("n", "v1").unwrap();
    db.update(a, "v2").unwrap();
    let meta = CommitMeta { reason: Some("rollback".into()), actor: Some(b"tester".to_vec()), source: None };
    let mut batch = Batch::new();
    batch.restore(a, 1);
    db.commit_with(&batch, &meta).unwrap();
    assert_eq!(db.get(a).unwrap().unwrap().payload.as_deref(), Some(&b"v1"[..]));
    let rev = db.unit_db().get_revision(a, kerno::units::RevNo(3)).unwrap();
    assert_eq!(rev.provenance.actor.as_deref(), Some(&b"tester"[..]));

    let mut batch = Batch::new();
    batch.archive(a);
    db.commit(&batch).unwrap();
    assert_eq!(db.get(a).unwrap().unwrap().lifecycle, Lifecycle::Archived);
    let mut batch = Batch::new();
    batch.unarchive(a);
    db.commit(&batch).unwrap();
    assert_eq!(db.get(a).unwrap().unwrap().lifecycle, Lifecycle::Active);
}

#[test]
fn as_of_a_future_sequence_reads_now() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("n", "v1").unwrap();
    db.update(a, "v2").unwrap();
    match db.read_as_of(a, u64::MAX).unwrap() {
        UnitAt::Present(r) => assert_eq!(r.payload.as_deref(), Some(&b"v2"[..])),
        other => panic!("expected the current revision, got {other:?}"),
    }
    match db.read_as_of(a, 0).unwrap() {
        UnitAt::Absent => {}
        other => panic!("expected absent before creation, got {other:?}"),
    }
}

#[test]
fn bulk_relations_by_type() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("n", "a").unwrap();
    let b = db.store("n", "b").unwrap();
    db.relate(a, b, "calls").unwrap();
    db.relate(b, a, "other").unwrap();
    assert_eq!(db.relations(None).unwrap().len(), 2);
    let calls = db.relations(Some("calls")).unwrap();
    assert_eq!((calls[0].from, calls[0].to), (a, b));
}

#[test]
fn a_unit_changed_earlier_in_the_batch_cannot_take_a_relation() {
    // `require_active` caches units it found Active within a change set; a
    // delete or an archive in the same batch must still be seen.
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("n", "a").unwrap();
    let b = db.store("n", "b").unwrap();
    let mut batch = Batch::new();
    batch.relate(a, b, "r").delete_cascade(b).relate(a, b, "r");
    assert!(matches!(db.commit(&batch), Err(kerno::Error::Deleted { .. })));
    let mut batch = Batch::new();
    batch.relate(a, b, "r").archive(b).relate(a, b, "r");
    assert!(db.commit(&batch).is_err(), "an archived unit cannot take a relation");
    assert!(db.related(a, Direction::Out, None).unwrap().is_empty(), "refused batches wrote nothing");
    // Counts stay right across many relations of one unit in one batch.
    let mut batch = Batch::new();
    for _ in 0..50 {
        batch.relate(a, b, "r");
    }
    db.commit(&batch).unwrap();
    assert_eq!(db.related(a, Direction::Out, Some("r")).unwrap().len(), 50);
    assert!(db.verify().unwrap().is_clean());
}

// ---- the format cut (ADR-0023): keyword index opt-in per type -------------

fn layer_version(dir: &std::path::Path) -> u32 {
    let db = Db::open(dir).unwrap();
    let raw = db.unit_db().raw_get(b"\x01layer").unwrap();
    db.close().unwrap();
    let raw = raw.expect("a layer mark");
    kerno::units::LayerMeta::decode_upto(&raw, kerno::units::codec::MAX_LAYER_VERSION).unwrap().layer_version
}

fn search(db: &Db, q: &str) -> Vec<kerno::UnitId> {
    db.search_text(q, 10, None).unwrap().iter().map(|h| h.unit).collect()
}

#[test]
fn keyword_index_is_off_by_default_and_on_per_type() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("sym", "alpha beta").unwrap();
    let off = db.search_text("alpha", 10, None);
    assert!(matches!(off, Err(kerno::Error::NotIndexed { .. })), "no type is indexed by default: {off:?}");
    db.set_fulltext("note", true).unwrap();
    db.set_fulltext("note", true).unwrap(); // same state: nothing written
    let n = db.store("note", "alpha beta").unwrap();
    assert_eq!(search(&db, "alpha"), vec![n]);
    db.update(a, "delta").unwrap();
    assert!(db.verify().unwrap().is_clean(), "{:?}", db.verify().unwrap().findings);
    db.close().unwrap();
    assert_eq!(
        layer_version(&dir.0),
        kerno::units::codec::MIN_LAYER_VERSION,
        "a new database starts at the cut version"
    );
    let db = Db::open(&dir.0).unwrap(); // the setting survives a reopen
    let m = db.store("note", "epsilon").unwrap();
    assert_eq!(search(&db, "epsilon"), vec![m]);
    assert!(db.verify().unwrap().is_clean());
}

#[test]
fn switching_a_type_with_units_reindexes_them_in_one_commit() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("note", "rust ownership").unwrap();
    let b = db.store("note", "rust borrow").unwrap();
    let c = db.store("note", "archived rust").unwrap();
    db.commit(Batch::new().archive(c)).unwrap();
    db.set_fulltext("note", true).unwrap();
    let mut hits = search(&db, "rust");
    hits.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(hits, want, "active units indexed, the archived one not");
    assert!(db.verify().unwrap().is_clean(), "{:?}", db.verify().unwrap().findings);
    db.commit(Batch::new().unarchive(c)).unwrap();
    assert_eq!(search(&db, "archived"), vec![c]);
    db.set_fulltext("note", false).unwrap();
    let off = db.search_text("rust", 10, None);
    assert!(matches!(off, Err(kerno::Error::NotIndexed { .. })), "turning it off removes the entries: {off:?}");
    assert!(db.verify().unwrap().is_clean(), "{:?}", db.verify().unwrap().findings);
}

#[test]
fn a_database_from_before_the_cut_is_refused() {
    // The marks of 1.0.0-alpha.1 (6), of the alpha.2 cut (7) and of the trigram index (8): their
    // adjacency and keyword postings are one record per entry, which this build does not read.
    // The mark of the posting-block cut (9), whose heads do not carry their payloads, and of the
    // head-payload cut (10), whose head revisions store a small payload twice, and of the cut that
    // stored a payload once (11), whose keyword postings name unit ids: this build opens one
    // layer, the one it writes (`units-v12.md` §1).
    for old in [6u8, 7, 8, 9, 10, 11] {
        let dir = TempDir::new();
        let db = Db::open(&dir.0).unwrap();
        db.store("n", "a").unwrap();
        let mut mark = db.unit_db().raw_get(b"\x01layer").unwrap().unwrap();
        mark[5] = old;
        db.unit_db().raw_commit(vec![(b"\x01layer".to_vec(), Some(mark))]).unwrap();
        db.close().unwrap();
        match Db::open(&dir.0) {
            Err(kerno::Error::LayerTooOld { found, minimum: 12 }) if found == u32::from(old) => {}
            Err(e) => panic!("layer {old}: wrong error: {e}"),
            Ok(_) => panic!("a database of layer {old} must be refused"),
        }
    }
}

#[test]
fn record_layout_is_pinned() {
    let (t, o) = kerno::units::type_options::check_shape(b"\x77\x04note", &[1, 0, 1]).unwrap();
    assert_eq!((t.as_str(), o), ("note", kerno::units::type_options::FULLTEXT));
    assert!(kerno::units::type_options::check_shape(b"\x77\x04note", &[1, 0, 0]).is_err(), "no empty options");
    let (_, o) = kerno::units::type_options::check_shape(b"\x77\x04note", &[1, 0, 3]).unwrap();
    assert_eq!(o, kerno::units::type_options::FULLTEXT | kerno::units::type_options::SUBSTRING);
    assert!(kerno::units::type_options::check_shape(b"\x77\x04note", &[1, 0, 4]).is_err(), "unknown bit");
    assert!(kerno::units::type_options::check_shape(b"\x77\x04note", &[2, 0, 1]).is_err(), "newer envelope");
    assert!(kerno::units::type_options::check_shape(b"\x77\x05note", &[1, 0, 1]).is_err(), "bad length");
}

fn user_schema(db: &Db) {
    let fields = vec![
        kerno::FieldDef::new("email", kerno::FieldType::Text).unique().required(),
        kerno::FieldDef::new("name", kerno::FieldType::Text),
    ];
    db.define_schema("user", fields).expect("define the schema");
}

fn user(email: &str) -> Vec<u8> {
    kerno::Document::new().with("email", kerno::FieldValue::Text(email.into())).encode().expect("encode")
}

#[test]
fn an_upsert_is_a_relate_end_in_the_same_batch() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    user_schema(&db);

    // store A, upsert g, store B, relate(0, 1) must link A to the upserted unit, not to B.
    let mut b = Batch::new();
    b.store("n", "A").upsert("user", "email", user("g@x.io")).store("n", "B");
    b.relate(Endpoint::Op(0), Endpoint::Op(1), "meant_for_g");
    let done = db.commit_with(&b, &CommitMeta::default()).unwrap();
    let edge = &db.related(done.units[0], Direction::Out, Some("meant_for_g")).unwrap()[0];
    assert_eq!(edge.to, done.upserts[0].id, "index 1 is the upsert, not the second store");

    // two typed upserts and a relation between them, in one commit.
    let mut b = Batch::new();
    b.upsert("user", "email", user("a@x.io")).upsert("user", "email", user("b@x.io"));
    b.relate(Endpoint::Op(0), Endpoint::Op(1), "knows");
    let done = db.commit_with(&b, &CommitMeta::default()).unwrap();
    let edge = &db.related(done.upserts[0].id, Direction::Out, Some("knows")).unwrap()[0];
    assert_eq!(edge.to, done.upserts[1].id);
}

#[test]
fn a_relate_end_that_is_not_an_earlier_unit_creating_op_is_an_error_and_writes_nothing() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    user_schema(&db);
    let a = db.store("n", "a").unwrap();
    let seq = db.last_commit_seq().unwrap();
    let refused = |build: &dyn Fn(&mut Batch)| {
        let mut b = Batch::new();
        build(&mut b);
        let err = db.commit(&b).unwrap_err();
        assert!(matches!(err, kerno::Error::BadEndpoint { .. }), "{err:?}");
        assert_eq!(db.last_commit_seq().unwrap(), seq, "a refused batch writes nothing");
        err.to_string()
    };

    // A later operation, the relation itself, and an index past the end.
    refused(&|b| {
        b.relate(a, Endpoint::Op(1), "r").store("n", "x");
    });
    refused(&|b| {
        b.store("n", "x").relate(Endpoint::Op(1), Endpoint::Op(0), "r");
    });
    refused(&|b| {
        b.store("n", "x").relate(Endpoint::Op(0), Endpoint::Op(9), "r");
    });
    // Operations that create no unit: an update, a relation, a delete.
    let said = refused(&|b| {
        b.update(a, "again").store("n", "x").relate(Endpoint::Op(0), Endpoint::Op(1), "r");
    });
    assert!(said.contains("creates no unit"), "{said}");
    refused(&|b| {
        b.store("n", "x").relate(a, Endpoint::Op(0), "r").relate(a, Endpoint::Op(1), "r");
    });
    refused(&|b| {
        b.store("n", "x").delete(a).relate(Endpoint::Op(0), Endpoint::Op(1), "r");
    });
}

#[test]
fn an_upsert_of_an_existing_unit_is_a_relate_end_too() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    user_schema(&db);
    let known = db
        .upsert_document(
            "user",
            "email",
            &kerno::Document::new().with("email", kerno::FieldValue::Text("k@x.io".into())),
        )
        .unwrap();

    // The second upsert of the key finds the unit (unchanged); a repeated key is one unit.
    let mut b = Batch::new();
    b.store("n", "first").upsert("user", "email", user("k@x.io")).upsert("user", "email", user("k@x.io"));
    b.relate(Endpoint::Op(0), Endpoint::Op(1), "to_known").relate(Endpoint::Op(0), Endpoint::Op(2), "to_known_again");
    let done = db.commit_with(&b, &CommitMeta::default()).unwrap();
    assert_eq!(done.upserts[0].id, known.id);
    assert_eq!(done.upserts[1].id, known.id);
    let out = db.related(done.units[0], Direction::Out, None).unwrap();
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|r| r.to == known.id));
}

/// A batch of many stores cuts their texts into terms on several threads ahead of the creates:
/// the index is the one the stores make one by one (same search answers, same scores, `verify`
/// clean), including stores of a type without the index mixed in.
#[test]
fn a_batch_of_stores_indexes_its_texts_as_single_stores_do() {
    let texts: Vec<String> = (0..300)
        .map(|i| format!("Doc{i} alpha{} Beta gamma{} ÉTÉ {}", i % 7, i % 13, "delta ".repeat(1 + i % 5)))
        .collect();
    let (one_dir, batch_dir) = (TempDir::new(), TempDir::new());
    let (one, batch) = (Db::open(&one_dir.0).unwrap(), Db::open(&batch_dir.0).unwrap());
    for db in [&one, &batch] {
        db.set_fulltext("note", true).unwrap();
    }
    for t in &texts {
        one.store("note", t.as_str()).unwrap();
        one.store("plain", t.as_str()).unwrap();
    }
    let mut b = Batch::new();
    for t in &texts {
        b.store("note", t.as_str());
        b.store("plain", t.as_str());
    }
    batch.commit(&b).unwrap();
    for q in ["alpha3", "beta", "gamma12 delta", "été", "doc299"] {
        let score = |db: &Db| -> Vec<(usize, String)> {
            let hits = db.search_text(q, 500, None).unwrap();
            let mut ids: Vec<kerno::UnitId> = hits.iter().map(|h| h.unit).collect();
            ids.sort();
            // Unit ids differ between the two databases; their rank in id order does not.
            hits.iter().map(|h| (ids.binary_search(&h.unit).unwrap(), format!("{:.9}", h.score))).collect()
        };
        assert_eq!(score(&one), score(&batch), "query {q:?}");
    }
    assert!(batch.verify().unwrap().is_clean(), "{:?}", batch.verify().unwrap().findings);
}
