//! The embedding path over the emulated VFS, so it runs on every platform.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Options;
use kerno_core::vfs::Vfs;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;

use super::*;
use crate::clock::{CounterIds, FixedClock};
use crate::db::{NewUnit, UnitConfig, UnitUpdate};
use crate::derived::{DerivedError, DerivedState};
use crate::types::CommitSeq;
use crate::types::Timestamp;

type Eng = Emulated<LogEngine>;

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(Timestamp(1))), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open() -> DerivedDb<Eng> {
    let vfs: Arc<dyn Vfs> = Arc::new(SimVfs::new(1));
    let db = UnitDb::<Eng>::open(vfs, Path::new("/db"), &cfg()).unwrap();
    DerivedDb::open_persisted(db).unwrap().0
}

fn open_with(provider: impl EmbeddingProvider + 'static) -> DerivedDb<Eng> {
    let db = open();
    db.db.set_embedding_provider(Arc::new(provider));
    db
}

fn create(db: &DerivedDb<Eng>, payload: &[u8]) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let unit = NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() };
    let id = cs.create(unit).unwrap();
    cs.commit().unwrap();
    id
}

fn edit(db: &DerivedDb<Eng>, unit: UnitId, payload: &[u8]) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(unit, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

fn seq_of(db: &DerivedDb<Eng>) -> CommitSeq {
    db.db.snapshot().unwrap().seq()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn hashing(dim: usize) -> HashingProvider {
    HashingProvider::new(dim).unwrap()
}

/// A provider that answers with whatever it is given, to test the checks.
struct Fixed {
    dim: usize,
    out: Result<Vec<Vec<f32>>, String>,
    profile: &'static str,
}

impl EmbeddingProvider for Fixed {
    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.out.clone().map_err(EmbedError::Provider)
    }

    fn profile(&self) -> &str {
        self.profile
    }
}

#[test]
fn hashing_provider_is_deterministic_and_unit_length() {
    let p = hashing(64);
    let a = p.embed(&["The cat sat", "the CAT sat!", "", "zzz qqq"]).unwrap();
    assert_eq!(a[0], a[1], "case and punctuation do not matter");
    assert_eq!(a[0], p.embed(&["The cat sat"]).unwrap()[0], "the same on every call");
    assert!(a.iter().all(|v| v.len() == 64));
    assert!((cosine(&a[0], &a[0]) - 1.0).abs() < 1e-5);
    assert!(a[2].iter().all(|x| *x == 0.0), "an empty text is the zero vector");
    let near = p.embed(&["the cat slept"]).unwrap();
    assert!(cosine(&a[0], &near[0]) > cosine(&a[0], &a[3]), "shared words are closer than none");
}

#[test]
fn hashing_provider_checks_its_dimension() {
    assert!(HashingProvider::new(0).is_err());
    assert!(HashingProvider::new(4097).is_err());
    assert_eq!(HashingProvider::new(4096).unwrap().dim(), 4096);
}

#[test]
fn embed_units_persists_one_ready_vector_per_unit() {
    let db = open_with(hashing(32));
    let (a, b) = (create(&db, b"red apple pie"), create(&db, b"blue sky"));
    let ids = db.embed_units(&[b, a], ChangeMeta::default()).unwrap();
    assert_eq!(ids.len(), 2);
    let want = hashing(32).embed(&["blue sky", "red apple pie"]).unwrap();
    for (id, v) in ids.iter().zip(&want) {
        assert_eq!(db.state(id), Some(DerivedState::Ready));
        assert_eq!(db.vector(id).as_ref(), Some(v));
        let d = db.descriptor(id).unwrap();
        assert_eq!((d.method(), d.version(), d.profile()), (METHOD, METHOD_VERSION, HashingProvider::PROFILE));
        assert_eq!(d.params().get("dim").map(String::as_str), Some("32"));
    }
}

#[test]
fn embeddings_survive_a_reopen_of_the_registry() {
    let db = open_with(hashing(16));
    let u = create(&db, b"persist me");
    let id = db.embed_units(&[u], ChangeMeta::default()).unwrap()[0];
    let vector = db.vector(&id).unwrap();
    let (db, report) = DerivedDb::open_persisted(db.db).unwrap();
    assert_eq!(report.served, 1);
    assert!(report.demoted.is_empty());
    assert_eq!(db.vector(&id), Some(vector));
}

#[test]
fn no_provider_is_refused_and_writes_nothing() {
    let db = open();
    let u = create(&db, b"text");
    let seq = seq_of(&db);
    assert!(matches!(db.embed_units(&[u], ChangeMeta::default()), Err(EmbedError::NoProvider)));
    assert_eq!(seq_of(&db), seq);
    assert!(db.db.embedding_provider().is_none());
}

#[test]
fn a_unit_that_is_not_text_refuses_the_whole_batch() {
    let db = open_with(hashing(8));
    let (good, bad) = (create(&db, b"fine"), create(&db, &[0xff, 0xfe, 0x00]));
    let seq = seq_of(&db);
    let err = db.embed_units(&[good, bad], ChangeMeta::default()).unwrap_err();
    assert!(matches!(err, EmbedError::NotText(u) if u == bad), "{err}");
    assert_eq!(seq_of(&db), seq, "nothing was written");
}

#[test]
fn an_empty_payload_is_not_text_and_writes_nothing() {
    let db = open_with(hashing(8));
    let (good, empty) = (create(&db, b"fine"), create(&db, b""));
    let seq = seq_of(&db);
    let err = db.embed_units(&[good, empty], ChangeMeta::default()).unwrap_err();
    assert!(matches!(err, EmbedError::NotText(u) if u == empty), "{err}");
    assert_eq!(seq_of(&db), seq, "nothing was written");
}

#[test]
fn a_deleted_unit_is_a_missing_source_and_writes_nothing() {
    let db = open_with(hashing(8));
    let u = create(&db, b"soon gone");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(u, None).unwrap();
    cs.commit().unwrap();
    let seq = seq_of(&db);
    let err = db.embed_units(&[u], ChangeMeta::default()).unwrap_err();
    assert!(matches!(err, EmbedError::Derived(DerivedStaleError::MissingSource(m)) if m == u), "{err}");
    assert_eq!(seq_of(&db), seq, "nothing was written");
}

#[test]
fn a_missing_unit_is_an_error_naming_it() {
    let db = open_with(hashing(8));
    let ghost = UnitId([9; 16]);
    let err = db.embed_units(&[ghost], ChangeMeta::default()).unwrap_err();
    assert!(matches!(err, EmbedError::Derived(DerivedStaleError::MissingSource(u)) if u == ghost), "{err}");
}

#[test]
fn provider_output_is_checked() {
    let u8s = |out| Fixed { dim: 4, out, profile: "t" };
    for (out, what) in [
        (Ok(vec![]), "no vectors for one text"),
        (Ok(vec![vec![1.0; 4], vec![1.0; 4]]), "two vectors for one text"),
        (Ok(vec![vec![1.0; 3]]), "a vector of the wrong length"),
        (Ok(vec![vec![f32::NAN; 4]]), "a NaN element"),
        (Ok(vec![vec![1.0, f32::INFINITY, 1.0, 1.0]]), "an infinite element"),
    ] {
        let db = open_with(u8s(out));
        let u = create(&db, b"x");
        let seq = seq_of(&db);
        let err = db.embed_units(&[u], ChangeMeta::default());
        assert!(matches!(err, Err(EmbedError::BadOutput(_))), "{what}: {err:?}");
        assert_eq!(seq_of(&db), seq, "{what}: nothing was written");
    }
    let db = open_with(u8s(Err("boom".into())));
    let u = create(&db, b"x");
    assert!(matches!(db.embed_units(&[u], ChangeMeta::default()), Err(EmbedError::Provider(m)) if m == "boom"));
}

#[test]
fn editing_a_unit_stales_its_vector_and_embedding_again_writes_a_new_one() {
    let db = open_with(hashing(32));
    let u = create(&db, b"first text");
    let old = db.embed_units(&[u], ChangeMeta::default()).unwrap()[0];
    assert!(matches!(
        db.embed_units(&[u], ChangeMeta::default()),
        Err(EmbedError::Derived(DerivedStaleError::Derived(DerivedError::Duplicate(i)))) if i == old
    ));
    edit(&db, u, b"second text");
    assert_eq!(db.state(&old), Some(DerivedState::Stale));
    let new = db.embed_units(&[u], ChangeMeta::default()).unwrap()[0];
    assert_ne!(new, old);
    assert_eq!(db.vector(&new), Some(hashing(32).embed(&["second text"]).unwrap().remove(0)));
}

#[test]
fn a_provider_with_another_profile_is_another_datum() {
    let db = open_with(hashing(8));
    let u = create(&db, b"same text");
    let a = db.embed_units(&[u], ChangeMeta::default()).unwrap()[0];
    let previous =
        db.db.set_embedding_provider(Arc::new(Fixed { dim: 8, out: Ok(vec![vec![0.5; 8]]), profile: "other" }));
    assert!(previous.is_some(), "the replaced provider is returned");
    let b = db.embed_units(&[u], ChangeMeta::default()).unwrap()[0];
    assert_ne!(a, b);
    assert_eq!(db.state(&a), Some(DerivedState::Ready));
    assert_eq!(db.vector(&b), Some(vec![0.5; 8]));
}
