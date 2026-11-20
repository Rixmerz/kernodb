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

// ---- layer 7: per-type keyword-index opt-out (ADR-0022) -------------------

fn layer_version(dir: &std::path::Path) -> u32 {
    let db = Db::open(dir).unwrap();
    let raw = db.unit_db().raw_get(b"\x01layer").unwrap();
    db.close().unwrap();
    let raw = raw.expect("a layer mark");
    kerno::units::LayerMeta::decode_upto(&raw, kerno::units::codec::MAX_LAYER_VERSION).unwrap().layer_version
}

#[test]
fn a_type_without_fulltext_is_not_indexed_and_verifies() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    db.set_fulltext("sym", false).unwrap();
    db.set_fulltext("sym", false).unwrap(); // same state: nothing written
    let a = db.store("sym", "alpha beta gamma").unwrap();
    let n = db.store("note", "alpha beta gamma").unwrap();
    let hits: Vec<_> = db.unit_db().snapshot().unwrap().search_text("alpha", 10).unwrap();
    let ids: Vec<_> = hits.iter().map(|h| h.unit).collect();
    assert_eq!(ids, vec![n], "only the indexed type is found");
    db.update(a, "delta").unwrap();
    assert!(db.verify().unwrap().is_clean(), "{:?}", db.verify().unwrap().findings);
    db.close().unwrap();
    assert_eq!(layer_version(&dir.0), 7);
    // The setting survives a reopen.
    let db = Db::open(&dir.0).unwrap();
    let b = db.store("sym", "epsilon").unwrap();
    assert!(db.unit_db().snapshot().unwrap().search_text("epsilon", 10).unwrap().is_empty());
    assert!(db.verify().unwrap().is_clean());
    let _ = b;
}

#[test]
fn the_setting_changes_only_while_the_type_is_empty() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    let a = db.store("sym", "alpha").unwrap();
    assert!(matches!(db.set_fulltext("sym", false), Err(kerno::Error::InvalidArgument(_))));
    db.delete(a).unwrap();
    db.set_fulltext("sym", false).unwrap();
    let b = db.store("sym", "beta").unwrap();
    assert!(db.set_fulltext("sym", true).is_err(), "an active unit blocks turning it back on");
    db.commit(Batch::new().archive(b)).unwrap();
    db.set_fulltext("sym", true).unwrap(); // archived units are not indexed either way
    db.commit(Batch::new().unarchive(b)).unwrap();
    let found: Vec<_> = db.unit_db().snapshot().unwrap().search_text("beta", 10).unwrap();
    assert_eq!(found.len(), 1, "unarchived under the index is indexed");
    assert!(db.verify().unwrap().is_clean(), "{:?}", db.verify().unwrap().findings);
}

#[test]
fn a_layer_6_reader_refuses_a_layer_7_database() {
    let dir = TempDir::new();
    let db = Db::open(&dir.0).unwrap();
    db.set_fulltext("sym", false).unwrap();
    let raw = db.unit_db().raw_get(b"\x01layer").unwrap().unwrap();
    let err = kerno::units::LayerMeta::decode_upto(&raw, 6).unwrap_err();
    assert_eq!(err.rule, kerno::units::UnitRule::Unit16);
}

#[test]
fn record_layout_is_pinned() {
    let (t, o) = kerno::units::type_options::check_shape(b"\x77\x03sym", &[1, 0, 1]).unwrap();
    assert_eq!((t.as_str(), o), ("sym", 1));
    assert!(kerno::units::type_options::check_shape(b"\x77\x03sym", &[1, 0, 0]).is_err(), "no empty options");
    assert!(kerno::units::type_options::check_shape(b"\x77\x03sym", &[1, 0, 2]).is_err(), "unknown bit");
    assert!(kerno::units::type_options::check_shape(b"\x77\x03sym", &[2, 0, 1]).is_err(), "newer envelope");
    assert!(kerno::units::type_options::check_shape(b"\x77\x04sym", &[1, 0, 1]).is_err(), "bad length");
}
