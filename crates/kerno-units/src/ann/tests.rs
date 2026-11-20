//! Tests of the HNSW index. Every one runs on `Emulated<LogEngine>` over a
//! `SimVfs`, so they run the same on macOS and Linux (no real disk, no
//! durability probe). The data are random vectors from a fixed seed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;

use super::*;
use crate::clock::{CounterIds, FixedClock};
use crate::db::{ChangeMeta, NewUnit, UnitConfig, UnitUpdate};
use crate::derived_stale::DerivedDb;
use crate::types::Timestamp;
use crate::vector_store::VectorRecipe;
use crate::{DerivedId, UnitDb};

type Eng = Emulated<LogEngine>;
type Ann = AnnIndex<Eng>;

const K: usize = 10;

fn unit(i: u64) -> UnitId {
    let mut b = [0u8; 16];
    b[8..].copy_from_slice(&i.to_be_bytes());
    UnitId(b)
}

fn vectors(rng: &mut Rng, n: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..n).map(|_| (0..dim).map(|_| (rng.below(2_000_001) as f32 - 1_000_000.0) / 1_000_000.0).collect()).collect()
}

fn cfg(dim: usize, metric: AnnMetric) -> AnnConfig {
    AnnConfig { seed: 7, ..AnnConfig::new(dim, metric) }
}

fn open_ann(disk: &SimVfs, cfg: AnnConfig) -> Result<Ann, UnitError> {
    Ann::open(disk.as_vfs(), Path::new("ann"), &Options::small_for_tests(), cfg)
}

fn exact(metric: AnnMetric, a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| f64::from(*x) * f64::from(*y)).sum();
    match metric {
        AnnMetric::L2 => a.iter().zip(b).map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2)).sum::<f64>().sqrt(),
        AnnMetric::Cosine => {
            let n = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
            1.0 - dot / (n(a) * n(b))
        }
    }
}

/// The `k` nearest of `data` by brute force.
fn brute(metric: AnnMetric, data: &BTreeMap<UnitId, Vec<f32>>, q: &[f32], k: usize) -> Vec<UnitId> {
    let mut all: Vec<(f64, UnitId)> = data.iter().map(|(u, v)| (exact(metric, q, v), *u)).collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    all.into_iter().take(k).map(|(_, u)| u).collect()
}

/// Mean recall@`K` of `idx` against brute force over `queries`; also checks
/// that every distance reported is the true one.
fn recall(idx: &Ann, data: &BTreeMap<UnitId, Vec<f32>>, queries: &[Vec<f32>]) -> f64 {
    let metric = idx.metric();
    let mut found = 0;
    for q in queries {
        let want: BTreeSet<UnitId> = brute(metric, data, q, K).into_iter().collect();
        let hits = idx.search(q, K).unwrap();
        assert_eq!(hits.len(), K.min(data.len()));
        for h in &hits {
            let d = exact(metric, q, &data[&h.unit]);
            assert!((f64::from(h.distance) - d).abs() < 1e-3, "distance {} for a true {d}", h.distance);
        }
        assert!(hits.windows(2).all(|w| w[0].distance <= w[1].distance), "hits are nearest first");
        found += hits.iter().filter(|h| want.contains(&h.unit)).count();
    }
    found as f64 / (queries.len() * K.min(data.len())) as f64
}

fn build(idx: &mut Ann, rng: &mut Rng, n: usize, dim: usize) -> BTreeMap<UnitId, Vec<f32>> {
    let mut data = BTreeMap::new();
    for (i, v) in vectors(rng, n, dim).into_iter().enumerate() {
        idx.insert(unit(i as u64), [i as u8; 32], &v).unwrap();
        data.insert(unit(i as u64), v);
    }
    data
}

fn recall_on_2k(metric: AnnMetric) {
    let disk = SimVfs::new(1);
    let mut idx = open_ann(&disk, cfg(16, metric)).unwrap();
    let mut rng = Rng::new(0xA11);
    let data = build(&mut idx, &mut rng, 2000, 16);
    assert_eq!(idx.len(), 2000);
    let queries = vectors(&mut rng, 100, 16);
    let r = recall(&idx, &data, &queries);
    assert!(r >= 0.95, "recall@{K} is {r} under {metric:?}");
}

#[test]
fn recall_at_10_l2_emulated_log() {
    recall_on_2k(AnnMetric::L2);
}

#[test]
fn recall_at_10_cosine_emulated_log() {
    recall_on_2k(AnnMetric::Cosine);
}

#[test]
fn a_query_of_an_indexed_vector_finds_it_first() {
    let disk = SimVfs::new(2);
    let mut idx = open_ann(&disk, cfg(8, AnnMetric::L2)).unwrap();
    let data = build(&mut idx, &mut Rng::new(5), 300, 8);
    for (u, v) in data.iter().take(40) {
        let top = idx.search(v, 1).unwrap();
        assert_eq!((top[0].unit, top[0].distance), (*u, 0.0));
    }
    assert!(idx.search(&[0.0; 8], 0).unwrap().is_empty(), "k = 0 is empty");
    let all = idx.search(&[0.0; 8], 1000).unwrap();
    assert_eq!(all.len(), 300, "k above the size returns everything reachable");
}

#[test]
fn deletes_leave_no_trace_and_keep_recall() {
    let disk = SimVfs::new(3);
    let mut idx = open_ann(&disk, cfg(16, AnnMetric::L2)).unwrap();
    let mut rng = Rng::new(0xDE1);
    let mut data = build(&mut idx, &mut rng, 1000, 16);
    let gone: Vec<UnitId> = (0..1000u64).filter(|i| i % 3 != 0).map(unit).collect();
    for u in &gone {
        assert!(idx.remove(u).unwrap());
        assert!(!idx.remove(u).unwrap(), "a second remove finds nothing");
        data.remove(u);
    }
    assert_eq!(idx.len(), data.len());
    assert!(gone.iter().all(|u| !idx.contains(u)));
    let queries = vectors(&mut rng, 100, 16);
    for q in &queries {
        assert!(idx.search(q, K).unwrap().iter().all(|h| data.contains_key(&h.unit)), "a deleted unit was returned");
    }
    let r = recall(&idx, &data, &queries);
    assert!(r >= 0.95, "recall@{K} after deletes is {r}");

    // Insert again over the holes, then empty the index completely.
    for (i, v) in vectors(&mut rng, 200, 16).into_iter().enumerate() {
        let u = unit(5000 + i as u64);
        idx.insert(u, [9; 32], &v).unwrap();
        data.insert(u, v);
    }
    assert!(recall(&idx, &data, &queries) >= 0.95);
    for u in data.keys().copied().collect::<Vec<_>>() {
        assert!(idx.remove(&u).unwrap());
    }
    assert!(idx.is_empty());
    assert!(idx.search(&queries[0], K).unwrap().is_empty());
    idx.insert(unit(1), [1; 32], &queries[0]).unwrap();
    assert_eq!(idx.search(&queries[1], K).unwrap().len(), 1, "an emptied index takes inserts again");
}

#[test]
fn insert_over_a_unit_replaces_its_vector() {
    let disk = SimVfs::new(4);
    let mut idx = open_ann(&disk, cfg(2, AnnMetric::L2)).unwrap();
    idx.insert(unit(1), [1; 32], &[0.0, 0.0]).unwrap();
    idx.insert(unit(2), [2; 32], &[5.0, 5.0]).unwrap();
    idx.insert(unit(1), [3; 32], &[9.0, 9.0]).unwrap();
    assert_eq!(idx.len(), 2);
    assert_eq!(idx.stamp(&unit(1)), Some([3; 32]));
    let top = idx.search(&[9.0, 9.0], 1).unwrap();
    assert_eq!((top[0].unit, top[0].distance), (unit(1), 0.0));
}

#[test]
fn reopen_serves_the_same_answers_and_goes_on() {
    let disk = SimVfs::new(5);
    let c = cfg(16, AnnMetric::Cosine);
    let mut idx = open_ann(&disk, c).unwrap();
    let mut rng = Rng::new(0xBEE);
    let mut data = build(&mut idx, &mut rng, 600, 16);
    for i in (0..600u64).step_by(4) {
        idx.remove(&unit(i)).unwrap();
        data.remove(&unit(i));
    }
    let queries = vectors(&mut rng, 50, 16);
    let before: Vec<Vec<AnnHit>> = queries.iter().map(|q| idx.search(q, K).unwrap()).collect();
    idx.close().unwrap();

    let mut idx = open_ann(&disk, c).unwrap();
    assert_eq!(idx.len(), data.len());
    for (q, want) in queries.iter().zip(&before) {
        assert_eq!(&idx.search(q, K).unwrap(), want, "a reopened index answers as it did");
    }
    assert!(recall(&idx, &data, &queries) >= 0.95);

    // It keeps being written after the reopen, and survives a second one.
    for (i, v) in vectors(&mut rng, 100, 16).into_iter().enumerate() {
        let u = unit(10_000 + i as u64);
        idx.insert(u, [7; 32], &v).unwrap();
        data.insert(u, v);
    }
    idx.close().unwrap();
    let idx = open_ann(&disk, c).unwrap();
    assert_eq!(idx.len(), data.len());
    assert!(recall(&idx, &data, &queries) >= 0.95);
}

#[test]
fn reopen_with_other_parameters_is_refused() {
    let disk = SimVfs::new(6);
    let mut idx = open_ann(&disk, cfg(4, AnnMetric::L2)).unwrap();
    idx.insert(unit(1), [0; 32], &[1.0, 2.0, 3.0, 4.0]).unwrap();
    idx.close().unwrap();
    let other = [
        cfg(5, AnnMetric::L2),
        cfg(4, AnnMetric::Cosine),
        AnnConfig { m: 8, ef_construction: 100, ..cfg(4, AnnMetric::L2) },
    ];
    for c in other {
        assert!(matches!(open_ann(&disk, c), Err(UnitError::InvalidArgument(_))), "{c:?}");
    }
    // ef_search and ef_construction are not stored: they may change.
    let c = AnnConfig { ef_search: 5, ef_construction: 20, ..cfg(4, AnnMetric::L2) };
    assert_eq!(open_ann(&disk, c).unwrap().len(), 1);
}

#[test]
fn a_damaged_record_is_corruption_not_a_misread() {
    let disk = SimVfs::new(7);
    let c = cfg(4, AnnMetric::L2);
    let mut idx = open_ann(&disk, c).unwrap();
    idx.insert(unit(1), [0; 32], &[1.0, 2.0, 3.0, 4.0]).unwrap();
    idx.close().unwrap();
    let engine = Eng::open(disk.as_vfs(), Path::new("ann"), &Options::small_for_tests()).unwrap();
    let mut b = WriteBatch::new();
    b.put(node_key(0), vec![FORMAT, 1, 2, 3]);
    engine.commit(&b).unwrap();
    engine.close().unwrap();
    let err = open_ann(&disk, c).unwrap_err();
    assert!(matches!(err, UnitError::Storage(kerno_core::Error::Corruption(_))), "{err:?}");
}

#[test]
fn parameters_and_vectors_are_validated_and_never_printed() {
    let disk = SimVfs::new(8);
    let bad = [
        AnnConfig { dim: 0, ..cfg(4, AnnMetric::L2) },
        AnnConfig { m: 1, ..cfg(4, AnnMetric::L2) },
        AnnConfig { ef_construction: 3, ..cfg(4, AnnMetric::L2) },
        AnnConfig { ef_search: 0, ..cfg(4, AnnMetric::L2) },
    ];
    for c in bad {
        assert!(matches!(open_ann(&disk, c), Err(UnitError::InvalidArgument(_))), "{c:?}");
    }
    let mut idx = open_ann(&disk, cfg(3, AnnMetric::Cosine)).unwrap();
    let refused = |r: Result<(), UnitError>| matches!(r, Err(UnitError::InvalidArgument(_)));
    assert!(refused(idx.insert(unit(1), [0; 32], &[1.0, 2.0])), "wrong dimension");
    assert!(refused(idx.insert(unit(1), [0; 32], &[1.0, f32::NAN, 0.0])), "NaN");
    assert!(refused(idx.insert(unit(1), [0; 32], &[f32::INFINITY, 0.0, 0.0])), "infinity");
    assert!(refused(idx.insert(unit(1), [0; 32], &[0.0; 3])), "zero under cosine");
    assert!(refused(idx.search(&[0.0; 3], 1).map(drop)), "a zero query under cosine");
    assert!(idx.is_empty(), "a refused insert changes nothing");
    idx.insert(unit(1), [0; 32], &[1.5, 2.5, 3.5]).unwrap();
    assert!(!format!("{idx:?}").contains("1.5"), "Debug names numbers of the index, not elements");
}

#[test]
fn reconcile_makes_the_index_equal_to_what_is_wanted() {
    let disk = SimVfs::new(9);
    let mut idx = open_ann(&disk, cfg(2, AnnMetric::L2)).unwrap();
    let v = |x: f32| vec![x, 0.0];
    let (a, b, c, bad) = (v(1.0), v(2.0), v(3.0), vec![1.0]);
    let want = |items: &[(u64, u8, &Vec<f32>)]| -> Vec<(UnitId, [u8; 32], Vec<f32>)> {
        items.iter().map(|(u, s, v)| (unit(*u), [*s; 32], (*v).clone())).collect()
    };
    let run = |idx: &mut Ann, w: Vec<(UnitId, [u8; 32], Vec<f32>)>| {
        idx.reconcile(w.iter().map(|(u, s, v)| (*u, *s, v.as_slice()))).unwrap()
    };
    let r = run(&mut idx, want(&[(1, 1, &a), (2, 1, &b), (3, 1, &bad)]));
    assert_eq!(r, SyncReport { added: 2, skipped: 1, ..SyncReport::default() });
    let r = run(&mut idx, want(&[(1, 1, &a), (2, 2, &c), (4, 1, &c)]));
    assert_eq!(r, SyncReport { added: 1, replaced: 1, unchanged: 1, ..SyncReport::default() });
    assert_eq!(idx.search(&[3.0, 0.0], 3).unwrap().iter().filter(|h| h.distance == 0.0).count(), 2);
    // A bad vector for an indexed unit removes it; a later entry for a unit wins.
    let r = run(&mut idx, want(&[(1, 1, &a), (2, 3, &bad), (4, 1, &b), (4, 5, &c)]));
    assert_eq!(r, SyncReport { replaced: 1, removed: 1, skipped: 1, unchanged: 1, ..SyncReport::default() });
    assert_eq!(idx.len(), 2);
    assert_eq!(idx.stamp(&unit(4)), Some([5; 32]));
    let r = run(&mut idx, want(&[(1, 1, &a), (4, 5, &c)]));
    assert_eq!(r, SyncReport { unchanged: 2, ..SyncReport::default() });
    assert_eq!(run(&mut idx, Vec::new()), SyncReport { removed: 2, ..SyncReport::default() });
    assert!(idx.is_empty());
}

// ------------------------------------------------------------- DerivedDb

fn ucfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(Timestamp(1))), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

/// A four-element vector from a four-byte payload.
fn embed(_: &crate::derived::DerivedDescriptor, inputs: &[crate::derived_stale::SourceInput]) -> Option<Vec<f32>> {
    Some(inputs.first()?.payload.iter().map(|b| f32::from(*b)).collect())
}

fn recipe() -> VectorRecipe {
    VectorRecipe { method: "bytes".into(), version: 1, profile: "test".into(), params: BTreeMap::new() }
}

fn new_unit(db: &DerivedDb<Eng>, payload: [u8; 4]) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn nearest(idx: &Ann, q: [f32; 4]) -> (UnitId, f32) {
    let h = idx.search(&q, 1).unwrap();
    (h[0].unit, h[0].distance)
}

#[test]
fn sync_ann_follows_inserts_updates_and_deletes_of_the_database() {
    let disk = SimVfs::new(10);
    let unit_db = UnitDb::<Eng>::open(disk.as_vfs(), Path::new("db"), &ucfg()).unwrap();
    let (db, _) = DerivedDb::open_persisted(unit_db).unwrap();
    let mut idx = open_ann(&disk, cfg(4, AnnMetric::L2)).unwrap();
    let r = recipe();
    let none = SyncReport::default();

    let (a, b, c) = (new_unit(&db, [10, 0, 0, 0]), new_unit(&db, [0, 10, 0, 0]), new_unit(&db, [0, 0, 10, 0]));
    let ids: Vec<DerivedId> =
        [a, b, c].iter().map(|u| db.build_vector(*u, &r, ChangeMeta::default(), &embed).unwrap()).collect();
    assert_eq!(db.sync_ann(&mut idx, &r).unwrap(), SyncReport { added: 3, ..none });
    assert_eq!(idx.stamp(&a), Some(ids[0].0), "the stamp is the id of the datum");
    assert_eq!(nearest(&idx, [10.0, 0.0, 0.0, 0.0]), (a, 0.0));
    assert_eq!(db.sync_ann(&mut idx, &r).unwrap(), SyncReport { unchanged: 3, ..none }, "nothing changed");

    // Another recipe is another set of vectors: this index sees none of them.
    let other = VectorRecipe { profile: "other".into(), ..recipe() };
    assert_eq!(db.sync_ann(&mut idx, &other).unwrap(), SyncReport { removed: 3, ..none });
    assert!(idx.is_empty());
    assert_eq!(db.sync_ann(&mut idx, &r).unwrap(), SyncReport { added: 3, ..none });

    // An update makes the vector stale: it leaves the index until rebuilt.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(a, UnitUpdate { payload: vec![0, 0, 0, 10], schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.sync_ann(&mut idx, &r).unwrap(), SyncReport { removed: 1, unchanged: 2, ..none });
    assert!(!idx.contains(&a));
    db.rebuild_vector(&ids[0], ChangeMeta::default(), &embed).unwrap();
    assert_eq!(db.sync_ann(&mut idx, &r).unwrap(), SyncReport { added: 1, unchanged: 2, ..none });
    assert_eq!(nearest(&idx, [0.0, 0.0, 0.0, 10.0]), (a, 0.0));

    // A delete takes the unit out of the index.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(b, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.sync_ann(&mut idx, &r).unwrap(), SyncReport { removed: 1, unchanged: 2, ..none });
    assert!(idx.search(&[0.0, 10.0, 0.0, 0.0], 5).unwrap().iter().all(|h| h.unit != b));

    // Both reopen, and the index is still equal to the database.
    idx.close().unwrap();
    drop(db);
    let unit_db = UnitDb::<Eng>::open(disk.as_vfs(), Path::new("db"), &ucfg()).unwrap();
    let (db, _) = DerivedDb::open_persisted(unit_db).unwrap();
    let mut idx = open_ann(&disk, cfg(4, AnnMetric::L2)).unwrap();
    assert_eq!(idx.len(), 2);
    assert_eq!(db.sync_ann(&mut idx, &r).unwrap(), SyncReport { unchanged: 2, ..none });
    assert_eq!(nearest(&idx, [0.0, 0.0, 10.0, 0.0]), (c, 0.0));
}

#[test]
fn sync_ann_skips_vectors_of_another_dimension() {
    let disk = SimVfs::new(11);
    let unit_db = UnitDb::<Eng>::open(disk.as_vfs(), Path::new("db"), &ucfg()).unwrap();
    let (db, _) = DerivedDb::open_persisted(unit_db).unwrap();
    let mut idx = open_ann(&disk, cfg(3, AnnMetric::L2)).unwrap();
    let u = new_unit(&db, [1, 2, 3, 4]);
    db.build_vector(u, &recipe(), ChangeMeta::default(), &embed).unwrap();
    assert_eq!(db.sync_ann(&mut idx, &recipe()).unwrap(), SyncReport { skipped: 1, ..SyncReport::default() });
    assert!(idx.is_empty());
}
