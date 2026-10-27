//! Reusable test suites. Every candidate engine runs exactly the same tests
//! through [`engine_test_suite!`](crate::engine_test_suite).
//!
//! Randomized tests are reproducible: they print the failing seed, and the
//! `SPIKE_SEED=<n>` environment variable re-runs only that seed.
//! `SPIKE_SEEDS=<n>` changes how many seeds the randomized suites run.

pub mod corruption;
pub mod crash;
pub mod functional;

use std::collections::BTreeMap;
use std::ops::{Bound, Range};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::{Engine, MAX_KEY_LEN, Op, Options, Snapshot, WriteBatch};
use crate::error::Result;
use crate::rng::Rng;
use crate::vfs::Vfs;

/// Expected database contents.
pub type Model = BTreeMap<Vec<u8>, Vec<u8>>;

pub const DB_DIR: &str = "/db";

pub fn db_dir() -> PathBuf {
    PathBuf::from(DB_DIR)
}

pub fn open<E: Engine>(vfs: &Arc<dyn Vfs>, opts: &Options) -> Result<E> {
    E::open(vfs.clone(), Path::new(DB_DIR), opts)
}

pub fn apply(model: &mut Model, batch: &WriteBatch) {
    for op in batch.ops() {
        match op {
            Op::Put { key, value } => {
                model.insert(key.clone(), value.clone());
            }
            Op::Delete { key } => {
                model.remove(key);
            }
        }
    }
}

/// Reads the whole snapshot through paginated scans (odd page size on
/// purpose, to exercise `Excluded` restarts).
pub fn read_all<S: Snapshot>(snap: &S) -> Result<Model> {
    let mut out = Model::new();
    let mut last: Option<Vec<u8>> = None;
    loop {
        let start = match &last {
            None => Bound::Unbounded,
            Some(k) => Bound::Excluded(k.as_slice()),
        };
        let page = snap.scan(start, Bound::Unbounded, 97)?;
        let done = page.len() < 97;
        for (k, v) in page {
            if let Some(prev) = &last {
                assert!(k > *prev, "scan returned keys out of order");
            }
            last = Some(k.clone());
            out.insert(k, v);
        }
        if done {
            return Ok(out);
        }
    }
}

/// Describes the first difference between two models (for assertion messages).
pub fn diff(expected: &Model, actual: &Model) -> String {
    for (k, v) in expected {
        match actual.get(k) {
            None => return format!("missing key {}", show(k)),
            Some(a) if a != v => {
                return format!("wrong value for {} ({} bytes, expected {} bytes)", show(k), a.len(), v.len());
            }
            _ => {}
        }
    }
    for k in actual.keys() {
        if !expected.contains_key(k) {
            return format!("unexpected key {}", show(k));
        }
    }
    "no difference".into()
}

pub fn show(key: &[u8]) -> String {
    let s = String::from_utf8_lossy(&key[..key.len().min(40)]).into_owned();
    if key.len() > 40 { format!("{s}…({} bytes)", key.len()) } else { s }
}

/// Full check of a snapshot against a model: full scan, point reads of
/// every key, a few absent keys and some random bounded scans.
pub fn assert_snapshot_matches<S: Snapshot>(snap: &S, model: &Model, rng: &mut Rng, ctx: &str) {
    let all = read_all(snap).unwrap_or_else(|e| panic!("{ctx}: full scan failed: {e}"));
    assert!(all == *model, "{ctx}: full scan differs from model: {}", diff(model, &all));
    for (k, v) in model {
        let got = snap.get(k).unwrap_or_else(|e| panic!("{ctx}: get failed: {e}"));
        assert!(got.as_deref() == Some(v.as_slice()), "{ctx}: get({}) returned wrong value", show(k));
    }
    for _ in 0..20 {
        let k = random_key(rng, 1 << 20);
        if !model.contains_key(&k) {
            assert_eq!(snap.get(&k).unwrap(), None, "{ctx}: absent key {} found", show(&k));
        }
    }
    for _ in 0..10 {
        let mut a = random_key(rng, 4000);
        let mut b = random_key(rng, 4000);
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        let limit = rng.range(1, 50) as usize;
        let got = snap.scan(Bound::Included(&a), Bound::Excluded(&b), limit).unwrap();
        let want: Vec<_> = model
            .range::<Vec<u8>, _>((Bound::Included(&a), Bound::Excluded(&b)))
            .take(limit)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        assert!(got == want, "{ctx}: bounded scan [{}, {}) limit {limit} differs", show(&a), show(&b));
    }
}

pub fn random_key(rng: &mut Rng, key_space: u64) -> Vec<u8> {
    format!("key-{:08}", rng.below(key_space)).into_bytes()
}

/// Random write batches with a realistic mix of sizes: mostly small values,
/// some multi-page values, occasional very long keys, empty values and
/// deletes of existing keys.
pub struct Workload {
    pub rng: Rng,
    pub key_space: u64,
    pub max_batch: u64,
    /// Probability (in 1/1000) that a value is "large" (1 KiB .. 20 KiB).
    pub large_per_mille: u64,
}

impl Workload {
    pub fn new(seed: u64) -> Self {
        Workload { rng: Rng::new(seed), key_space: 4000, max_batch: 12, large_per_mille: 40 }
    }

    pub fn key(&mut self) -> Vec<u8> {
        if self.rng.chance(1, 100) {
            let mut k = format!("long-{:06}-", self.rng.below(self.key_space)).into_bytes();
            let len = self.rng.range(64, MAX_KEY_LEN as u64) as usize;
            k.resize(len, b'x');
            k
        } else {
            random_key(&mut self.rng, self.key_space)
        }
    }

    pub fn value(&mut self) -> Vec<u8> {
        let len = if self.rng.chance(self.large_per_mille, 1000) {
            self.rng.range(1024, 20 * 1024)
        } else if self.rng.chance(1, 50) {
            0
        } else {
            self.rng.range(1, 300)
        };
        self.rng.bytes(len as usize)
    }

    pub fn batch(&mut self, model: &Model) -> WriteBatch {
        let mut b = WriteBatch::new();
        let n = self.rng.range(1, self.max_batch);
        for _ in 0..n {
            if self.rng.chance(1, 4) && !model.is_empty() {
                let probe = self.key();
                let key = model.range(probe..).next().or_else(|| model.iter().next()).map(|(k, _)| k.clone()).unwrap();
                b.delete(key);
            } else if self.rng.chance(1, 30) {
                let k = self.key();
                b.delete(k);
            } else {
                let k = self.key();
                let v = self.value();
                b.put(k, v);
            }
        }
        b
    }
}

/// Seeds for randomized suites: `SPIKE_SEED` (single) or
/// `0..SPIKE_SEEDS` (default `default_count`).
pub fn seeds(default_count: u64) -> Range<u64> {
    if let Ok(s) = std::env::var("SPIKE_SEED") {
        let s: u64 = s.parse().expect("SPIKE_SEED must be a number");
        return s..s + 1;
    }
    let n = std::env::var("SPIKE_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(default_count);
    0..n
}

/// Runs `f(seed)` for each seed, reporting the seed on panic.
pub fn for_each_seed(name: &str, seeds: Range<u64>, f: impl Fn(u64)) {
    for seed in seeds {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(seed)));
        if let Err(e) = r {
            eprintln!("\n*** {name} failed for seed {seed}; reproduce with SPIKE_SEED={seed}\n");
            std::panic::resume_unwind(e);
        }
    }
}

/// Generates the shared test suite for an engine type.
///
/// ```ignore
/// spike_core::engine_test_suite!(MyEngine, buffered_power_loss_prefix = true);
/// ```
///
/// `buffered_power_loss_prefix` declares whether the engine guarantees that,
/// in [`Durability::Buffered`](crate::Durability::Buffered) mode, a power
/// loss leaves a consistent prefix of commits. If `false`, buffered mode is
/// only tested against process crashes.
#[macro_export]
macro_rules! engine_test_suite {
    ($engine:ty, buffered_power_loss_prefix = $bpl:expr) => {
        mod shared_functional {
            use super::*;
            use $crate::harness::functional as f;
            #[test]
            fn empty_database() {
                f::empty_database::<$engine>();
            }
            #[test]
            fn put_get_overwrite_delete() {
                f::put_get_overwrite_delete::<$engine>();
            }
            #[test]
            fn batch_semantics() {
                f::batch_semantics::<$engine>();
            }
            #[test]
            fn scan_bounds_and_pagination() {
                f::scan_bounds_and_pagination::<$engine>();
            }
            #[test]
            fn size_limits() {
                f::size_limits::<$engine>();
            }
            #[test]
            fn reopen_after_clean_close() {
                f::reopen_after_clean_close::<$engine>();
            }
            #[test]
            fn reopen_after_drop_without_close() {
                f::reopen_after_drop_without_close::<$engine>();
            }
            #[test]
            fn commit_sequence_is_monotonic_and_persistent() {
                f::commit_sequence::<$engine>();
            }
            #[test]
            fn exclusive_open() {
                f::exclusive_open::<$engine>();
            }
            #[test]
            fn snapshot_isolation_under_churn() {
                f::snapshot_isolation_under_churn::<$engine>();
            }
            #[test]
            fn concurrent_readers_see_atomic_batches() {
                f::concurrent_readers::<$engine>();
            }
            #[test]
            fn randomized_model_check() {
                f::randomized_model_check::<$engine>();
            }
            #[test]
            fn space_is_reclaimed() {
                f::space_is_reclaimed::<$engine>();
            }
            #[test]
            fn real_filesystem_smoke() {
                f::real_filesystem_smoke::<$engine>();
            }
        }
        mod shared_crash {
            use super::*;
            use $crate::Durability;
            use $crate::harness::crash as c;
            use $crate::vfs::CrashKind;
            #[test]
            fn power_loss_sync() {
                c::run::<$engine>(CrashKind::PowerLoss, Durability::Sync);
            }
            #[test]
            fn process_kill_sync() {
                c::run::<$engine>(CrashKind::ProcessKill, Durability::Sync);
            }
            #[test]
            fn process_kill_buffered() {
                c::run::<$engine>(CrashKind::ProcessKill, Durability::Buffered);
            }
            #[test]
            fn power_loss_buffered() {
                if $bpl {
                    c::run::<$engine>(CrashKind::PowerLoss, Durability::Buffered);
                } else {
                    eprintln!("engine does not claim prefix consistency for buffered commits on power loss");
                }
            }
        }
        mod shared_corruption {
            use super::*;
            #[test]
            fn bit_flips_are_detected_or_harmless() {
                $crate::harness::corruption::bit_flips::<$engine>();
            }
        }
    };
}
