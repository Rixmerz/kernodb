//! E9-09: reads through `DerivedDb` (`snapshot`, `query`, `history`,
//! `derived_of`) and the vector search that builds its set once per batch.
//! Every test runs on both engines (`emulated_log` and `kerno_db`). Answers
//! are compared with the reference path on the same snapshot (I-32).

mod harness;

use std::collections::BTreeMap;
use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::assoc_rank::{VectorError, VectorMetric, VectorQuery, VectorSet};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn open_dd<E: Target>() -> DerivedDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    let disk = SimVfs::new(90);
    let db = open_units::<E>(disk.as_vfs(), &c).expect("open");
    DerivedDb::open_persisted(db).expect("open_persisted").0
}

fn make<E: Target>(dd: &DerivedDb<E>, unit_type: &str, payload: &[u8]) -> UnitId {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: unit_type.into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn edit<E: Target>(dd: &DerivedDb<E>, id: UnitId, payload: &[u8]) {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

fn recipe() -> VectorRecipe {
    VectorRecipe {
        method: "embed".into(),
        version: 1,
        profile: "model-a".into(),
        params: BTreeMap::from([("dim".to_owned(), "3".to_owned())]),
    }
}

/// A vector of dimension 3 that depends on the payload, never zero.
fn embed(payload: &[u8]) -> Vec<f32> {
    let s: f32 = payload.iter().map(|b| f32::from(*b)).sum();
    vec![1.0 + payload.len() as f32, s + 1.0, (s % 7.0) + 0.5]
}

fn vcompute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<f32>> {
    inputs.first().map(|i| embed(&i.payload))
}

fn build<E: Target>(dd: &DerivedDb<E>, unit: UnitId) -> DerivedId {
    dd.build_vector(unit, &recipe(), ChangeMeta::default(), &vcompute).unwrap()
}

fn cosine(v: Vec<f32>) -> VectorQuery {
    VectorQuery::new(VectorMetric::Cosine, v).unwrap()
}

fn query_and_snapshot_agree_with_the_reference<E: Target>() {
    let dd = open_dd::<E>();
    let a = make(&dd, "note", b"a");
    let b = make(&dd, "doc", b"bb");
    let c = make(&dd, "note", b"ccc");
    edit(&dd, a, b"a2");

    let snap = dd.snapshot().unwrap();
    assert_eq!(snap.get_unit(b).unwrap(), dd.get_unit(b).unwrap());
    for filter in [
        UnitFilter::default(),
        UnitFilter { unit_type: Some("note".into()), schema_version: None },
        UnitFilter { unit_type: Some("none".into()), schema_version: None },
    ] {
        let out = dd.query(&filter).unwrap();
        assert_eq!(out.units, snap.reference_filter(&filter).unwrap(), "{filter:?}");
    }
    let notes = dd.query(&UnitFilter { unit_type: Some("note".into()), schema_version: None }).unwrap();
    assert_eq!(notes.units.iter().map(|u| u.id).collect::<Vec<_>>(), vec![a, c]);
}

fn history_lists_every_revision_newest_first<E: Target>() {
    let dd = open_dd::<E>();
    let a = make(&dd, "note", b"a");
    let other = make(&dd, "note", b"x");
    edit(&dd, a, b"a2");
    edit(&dd, a, b"a3");
    edit(&dd, other, b"y");

    let h = dd.history(a).unwrap();
    assert_eq!(h.iter().map(|r| r.id.rev.0).collect::<Vec<_>>(), vec![3, 2, 1]);
    assert!(h.iter().all(|r| r.id.unit == a));
    let reference = dd.snapshot().unwrap().history(a, HistoryQuery::latest(100)).unwrap();
    assert_eq!(h, reference);
    assert_eq!(dd.history(other).unwrap().len(), 2);
}

fn derived_of_finds_the_data_of_one_unit<E: Target>() {
    let dd = open_dd::<E>();
    let a = make(&dd, "note", b"a");
    let b = make(&dd, "note", b"b");
    let lonely = make(&dd, "note", b"z");
    let va = build(&dd, a);
    let vb = build(&dd, b);

    assert_eq!(dd.derived_of(a), vec![va]);
    assert_eq!(dd.derived_of(b), vec![vb]);
    assert!(dd.derived_of(lonely).is_empty());
    // A stale datum is still the unit's derived datum.
    edit(&dd, a, b"a2");
    assert_eq!(dd.derived_of(a), vec![va]);
    assert_eq!(dd.state(&va), Some(DerivedState::Stale));
}

/// The set an outsider would build by hand from `vector()`.
fn by_hand<E: Target>(dd: &DerivedDb<E>, pairs: &[(UnitId, DerivedId)]) -> VectorSet {
    let mut set = VectorSet::new(3, VectorMetric::Cosine).unwrap();
    for (u, id) in pairs {
        set.insert(*u, dd.vector(id).unwrap()).unwrap();
    }
    set
}

fn search_equals_the_reference_over_persisted_vectors<E: Target>() {
    let dd = open_dd::<E>();
    let mut pairs = Vec::new();
    for p in [&b"alpha"[..], b"bravo!", b"c", b"delta-delta", b"e"] {
        let u = make(&dd, "note", p);
        pairs.push((u, build(&dd, u)));
    }
    let queries = [cosine(vec![1.0, 2.0, 3.0]), cosine(vec![5.0, 1.0, 0.5]), cosine(embed(b"c"))];
    let reference = by_hand(&dd, &pairs);

    let got = dd.search_vectors(VectorMetric::Cosine, &queries, 3).unwrap();
    assert_eq!(got.len(), queries.len());
    for (q, hits) in queries.iter().zip(&got) {
        assert_eq!(hits, &reference.search(q, 3).unwrap());
        assert_eq!(hits.len(), 3);
    }
    // k above the count returns all of them.
    assert_eq!(dd.search_vectors(VectorMetric::Cosine, &queries[..1], 99).unwrap()[0].len(), pairs.len());
}

fn search_serves_only_ready_vectors<E: Target>() {
    let dd = open_dd::<E>();
    let a = make(&dd, "note", b"a");
    let b = make(&dd, "note", b"bb");
    let va = build(&dd, a);
    let vb = build(&dd, b);
    edit(&dd, a, b"a2");
    assert_eq!(dd.state(&va), Some(DerivedState::Stale));

    let set = dd.vector_set(3, VectorMetric::Cosine).unwrap();
    assert_eq!(set.len(), 1);
    let q = cosine(embed(b"bb"));
    let hits = dd.search_vectors(VectorMetric::Cosine, std::slice::from_ref(&q), 5).unwrap();
    assert_eq!(hits[0].iter().map(|s| s.unit).collect::<Vec<_>>(), vec![b]);
    assert_eq!(hits[0], by_hand(&dd, &[(b, vb)]).search(&q, 5).unwrap());
}

fn search_refuses_what_the_set_refuses<E: Target>() {
    let dd = open_dd::<E>();
    let a = make(&dd, "note", b"a");
    build(&dd, a);
    assert!(dd.search_vectors(VectorMetric::Cosine, &[], 3).unwrap().is_empty());
    // Another dimension in the batch.
    let mixed = [cosine(vec![1.0, 1.0, 1.0]), cosine(vec![1.0, 1.0])];
    assert!(matches!(
        dd.search_vectors(VectorMetric::Cosine, &mixed, 3),
        Err(VectorError::DimensionMismatch { expected: 3, got: 2 })
    ));
    // Another metric.
    let dot = VectorQuery::new(VectorMetric::Dot, vec![1.0, 1.0, 1.0]).unwrap();
    assert!(matches!(dd.search_vectors(VectorMetric::Cosine, &[dot], 3), Err(VectorError::MetricMismatch { .. })));
    // Nothing of that dimension: an empty answer, not an error.
    let none = dd.search_vectors(VectorMetric::Cosine, &[cosine(vec![1.0; 5])], 3).unwrap();
    assert_eq!(none, vec![vec![]]);
}

macro_rules! tests {
    ($e:ty; $($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                super::$name::<$e>();
            }
        )*
    };
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            tests!(
                $e;
                query_and_snapshot_agree_with_the_reference,
                history_lists_every_revision_newest_first,
                derived_of_finds_the_data_of_one_unit,
                search_equals_the_reference_over_persisted_vectors,
                search_serves_only_ready_vectors,
                search_refuses_what_the_set_refuses,
            );
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
