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
    b.relate(Endpoint::Stored(0), Endpoint::Stored(1), "next").relate(a, Endpoint::Stored(0), "next");
    let done = db.commit_with(&b, &CommitMeta::default()).unwrap();
    assert_eq!(done.units.len(), 2);
    assert_eq!(done.relations.len(), 2);
    assert_eq!(db.related(a, Direction::Out, Some("next")).unwrap()[0].to, done.units[0]);
    assert_eq!(done.commit_seq, Some(db.last_commit_seq().unwrap()));

    let mut bad = Batch::new();
    bad.relate(a, Endpoint::Stored(5), "next");
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
