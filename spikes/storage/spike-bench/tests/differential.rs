//! SAD §17.6: "the same sequence of operations produces the same logical
//! state in every candidate". Runs one random history (puts, overwrites,
//! deletes, large values, long keys, clean reopens) through every engine
//! and compares full contents and bounded scans at each checkpoint.

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use spike_core::harness::{Model, Workload, apply, read_all};
use spike_core::rng::Rng;
use spike_core::vfs::{SimVfs, StdVfs};
use spike_core::{Engine, Options, Snapshot, Vfs, WriteBatch};

const COMMITS: usize = 500;
const CHECKPOINT_EVERY: usize = 50;

/// Contents and some bounded scans at a checkpoint.
#[derive(PartialEq, Debug)]
struct Observation {
    seq: Option<u64>,
    contents: Model,
    scans: Vec<Vec<(Vec<u8>, Vec<u8>)>>,
}

fn history(seed: u64) -> Vec<WriteBatch> {
    let mut w = Workload::new(seed);
    w.key_space = 1500;
    let mut model = Model::new();
    (0..COMMITS)
        .map(|_| {
            let b = w.batch(&model);
            apply(&mut model, &b);
            b
        })
        .collect()
}

fn scan_ranges(seed: u64) -> Vec<(Vec<u8>, Vec<u8>, usize)> {
    let mut rng = Rng::new(seed);
    (0..20)
        .map(|_| {
            let mut a = format!("key-{:08}", rng.below(1500)).into_bytes();
            let mut b = format!("key-{:08}", rng.below(1500)).into_bytes();
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            (a, b, rng.range(1, 60) as usize)
        })
        .collect()
}

fn observe<S: Snapshot>(s: &S, ranges: &[(Vec<u8>, Vec<u8>, usize)], with_seq: bool) -> Observation {
    Observation {
        seq: with_seq.then(|| s.seq()),
        contents: read_all(s).unwrap(),
        scans: ranges
            .iter()
            .map(|(a, b, limit)| s.scan(Bound::Included(a), Bound::Excluded(b), *limit).unwrap())
            .collect(),
    }
}

/// Runs the history; `persistent_seq` is false for reference engines,
/// whose adapters only track the commit sequence in memory.
fn run<E: Engine>(vfs: Arc<dyn Vfs>, dir: &Path, batches: &[WriteBatch], persistent_seq: bool) -> Vec<Observation> {
    let opts = Options::small_for_tests();
    let ranges = scan_ranges(1);
    let mut out = Vec::new();
    let mut db = E::open(vfs.clone(), dir, &opts).unwrap();
    for (i, b) in batches.iter().enumerate() {
        db.commit(b).unwrap();
        if (i + 1) % CHECKPOINT_EVERY == 0 {
            out.push(observe(&db.snapshot().unwrap(), &ranges, persistent_seq));
            db.close().unwrap();
            db = E::open(vfs.clone(), dir, &opts).unwrap();
            let after = observe(&db.snapshot().unwrap(), &ranges, persistent_seq);
            assert_eq!(after, *out.last().unwrap(), "{}: state changed across a clean reopen", E::name());
        }
    }
    db.close().unwrap();
    out
}

fn sim<E: Engine>(batches: &[WriteBatch]) -> Vec<Observation> {
    run::<E>(SimVfs::new(1).as_vfs(), Path::new("/db"), batches, true)
}

fn real<E: Engine>(batches: &[WriteBatch]) -> Vec<Observation> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir: PathBuf = std::env::temp_dir().join(format!("kerno-diff-{}-{}-{nanos}", E::name(), std::process::id()));
    let out = run::<E>(StdVfs::shared(), &dir, batches, false);
    std::fs::remove_dir_all(&dir).unwrap();
    out
}

#[test]
fn same_history_same_state_in_every_engine() {
    let batches = history(2026);
    let mut expected = Vec::new();
    let mut model = Model::new();
    let ranges = scan_ranges(1);
    for (i, b) in batches.iter().enumerate() {
        apply(&mut model, b);
        if (i + 1) % CHECKPOINT_EVERY == 0 {
            let scans: Vec<Vec<(Vec<u8>, Vec<u8>)>> = ranges
                .iter()
                .map(|(a, z, limit)| {
                    model
                        .range::<Vec<u8>, _>((Bound::Included(a), Bound::Excluded(z)))
                        .take(*limit)
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<Vec<_>>()
                })
                .collect();
            expected.push((i as u64 + 1, model.clone(), scans));
        }
    }

    let mut results: BTreeMap<&str, Vec<Observation>> = BTreeMap::new();
    results.insert("log", sim::<spike_core::reference::LogEngine>(&batches));
    #[cfg(feature = "candidates")]
    {
        results.insert("btree", sim::<spike_btree::BTreeEngine>(&batches));
        results.insert("lsm", sim::<spike_lsm::LsmEngine>(&batches));
    }
    #[cfg(feature = "kerno")]
    results.insert("kerno", sim::<kerno_storage::KernoDb>(&batches));
    results.insert("redb", real::<spike_bench::adapters::RedbEngine>(&batches));
    results.insert("fjall", real::<spike_bench::adapters::FjallEngine>(&batches));
    results.insert("sqlite", real::<spike_bench::adapters::SqliteEngine>(&batches));

    for (engine, observations) in &results {
        assert_eq!(observations.len(), expected.len(), "{engine}: wrong number of checkpoints");
        for (obs, (seq, contents, scans)) in observations.iter().zip(&expected) {
            if let Some(s) = obs.seq {
                assert_eq!(s, *seq, "{engine}: commit sequence differs at checkpoint {seq}");
            }
            assert!(obs.contents == *contents, "{engine}: contents differ from the model at commit {seq}");
            assert!(obs.scans == *scans, "{engine}: bounded scans differ from the model at commit {seq}");
        }
    }
}
