//! Reusable test suites. Every candidate engine runs exactly the same tests
//! through [`engine_test_suite!`](crate::engine_test_suite), the extended
//! failure models through
//! [`extended_failure_suite!`](crate::extended_failure_suite), and range
//! deletions through [`range_delete_test_suite!`](crate::range_delete_test_suite).
//!
//! Randomized tests are reproducible: they print the failing seed, and the
//! `SPIKE_SEED=<n>` environment variable re-runs only that seed.
//! `SPIKE_SEEDS=<n>` changes how many seeds the randomized suites run.
//!
//! Engines with `sync()` also run
//! [`sync_test_suite!`](crate::sync_test_suite), and every engine that takes
//! commits from several threads runs
//! [`group_commit_test_suite!`](crate::group_commit_test_suite) (multiple
//! writers, scenario C).

pub mod concurrency;
pub mod corruption;
pub mod crash;
pub mod ext;
pub mod faults;
pub mod functional;
pub mod sync;

pub mod range;

use std::collections::BTreeMap;
use std::ops::{Bound, Range};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::{MAX_KEY_LEN, Op, Result, WriteBatch};

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

/// Applies `batch` to `model` operation by operation (the contract's
/// semantics: later operations win, a range deletion removes `[start, end)`
/// as it stands at that point of the batch).
pub fn apply(model: &mut Model, batch: &WriteBatch) {
    for op in batch.ops() {
        match op {
            Op::Put { key, value } => {
                model.insert(key.clone(), value.clone());
            }
            Op::Delete { key } => {
                model.remove(key);
            }
            Op::DeleteRange { start, end } => {
                // `start >= end` deletes nothing (and `BTreeMap::range`
                // would panic on `start > end`, which `validate` rejects).
                if start < end {
                    let doomed: Vec<Vec<u8>> = model
                        .range::<[u8], _>((Bound::Included(start.as_slice()), Bound::Excluded(end.as_slice())))
                        .map(|(k, _)| k.clone())
                        .collect();
                    for k in doomed {
                        model.remove(&k);
                    }
                }
            }
        }
    }
}

/// Shape of the random workload of a suite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkloadCfg {
    /// Chance, in thousandths per operation, that [`Workload::batch`] emits a
    /// [`WriteBatch::delete_range`] (bounds from [`Workload::range_bounds`]).
    /// The default, 0, draws exactly the random numbers the M0 workload drew,
    /// so every history, sweep point and fingerprint stays the same.
    pub range_delete_per_mille: u32,
}

/// Configuration of the suites [`engine_test_suite!`](crate::engine_test_suite)
/// generates with its `config = …` arm. The default reproduces the arms
/// without `config` exactly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SuiteConfig {
    pub workload: WorkloadCfg,
}

impl SuiteConfig {
    /// The default suites with `per_mille` thousandths of the random
    /// operations turned into range deletions (the engine must implement
    /// them, natively or through [`Emulated`](crate::emulated::Emulated)).
    pub fn with_range_deletes(per_mille: u32) -> Self {
        SuiteConfig { workload: WorkloadCfg { range_delete_per_mille: per_mille } }
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
/// deletes of existing keys; with [`WorkloadCfg::range_delete_per_mille`],
/// also range deletions.
pub struct Workload {
    pub rng: Rng,
    pub key_space: u64,
    pub max_batch: u64,
    /// Probability (in 1/1000) that a value is "large" (1 KiB .. 20 KiB).
    pub large_per_mille: u64,
    pub cfg: WorkloadCfg,
}

impl Workload {
    pub fn new(seed: u64) -> Self {
        Self::with_config(seed, WorkloadCfg::default())
    }

    pub fn with_config(seed: u64, cfg: WorkloadCfg) -> Self {
        Workload { rng: Rng::new(seed), key_space: 4000, max_batch: 12, large_per_mille: 40, cfg }
    }

    /// Bounds of a random range deletion, always inside one key family
    /// (`key-…` or `long-…`, so that keys outside the workload, such as the
    /// accounts of `concurrent_readers`, are never hit): usually a few
    /// neighbouring keys, sometimes up to a quarter of the key space, and
    /// sometimes empty (`start == end`).
    pub fn range_bounds(&mut self) -> (Vec<u8>, Vec<u8>) {
        let span = match self.rng.below(16) {
            0 => 0,
            1 => self.rng.range(1, self.key_space / 4 + 1),
            _ => self.rng.range(1, 8),
        };
        let lo = self.rng.below(self.key_space);
        assert!(lo + span < 1_000_000, "key space too large for fixed-width range bounds");
        if self.rng.chance(1, 10) {
            (format!("long-{lo:06}").into_bytes(), format!("long-{:06}", lo + span).into_bytes())
        } else {
            (format!("key-{lo:08}").into_bytes(), format!("key-{:08}", lo + span).into_bytes())
        }
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
            // No draw at all when ranges are off: the M0 stream is unchanged.
            let ranges = u64::from(self.cfg.range_delete_per_mille);
            if ranges > 0 && self.rng.chance(ranges, 1000) {
                let (start, end) = self.range_bounds();
                b.delete_range(start, end);
            } else if self.rng.chance(1, 4) && !model.is_empty() {
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
/// kerno_testkit::engine_test_suite!(MyEngine, buffered_power_loss_prefix = true);
/// kerno_testkit::engine_test_suite!(
///     MyEngine,
///     buffered_power_loss_prefix = true,
///     config = kerno_testkit::harness::SuiteConfig::with_range_deletes(20)
/// );
/// ```
///
/// `buffered_power_loss_prefix` declares whether the engine guarantees that,
/// in [`Durability::Buffered`](crate::Durability::Buffered) mode, a power
/// loss leaves a consistent prefix of commits. If `false`, buffered mode is
/// only tested against process crashes.
///
/// `detects_truncation` (default `true`) declares whether truncated files
/// after a clean shutdown are always detected. Only the reference
/// `LogEngine` opts out (a truncated log is indistinguishable from a torn
/// tail).
///
/// `config` (default [`SuiteConfig::default()`](SuiteConfig)) changes the
/// random workload of every test that uses one (reopens, isolation,
/// concurrent readers, the model check, the real file system smoke test,
/// the crash, fault and corruption suites, and the scenario of the
/// `every_point` sweeps). The arms without `config` generate exactly the M0
/// suite. The generated modules have fixed names (`shared_functional`,
/// `shared_crash`, `shared_faults`, `shared_corruption`): a second suite in
/// the same file goes inside its own module.
#[macro_export]
macro_rules! engine_test_suite {
    (@suite $engine:ty, $bpl:expr, $trunc:expr, $cfg:expr) => {
        mod shared_functional {
            use super::*;
            use $crate::harness::functional as f;
            fn cfg() -> $crate::harness::SuiteConfig {
                $cfg
            }
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
                f::reopen_after_clean_close_with_config::<$engine>(&cfg());
            }
            #[test]
            fn reopen_after_drop_without_close() {
                f::reopen_after_drop_without_close_with_config::<$engine>(&cfg());
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
                f::snapshot_isolation_under_churn_with_config::<$engine>(&cfg());
            }
            #[test]
            fn concurrent_readers_see_atomic_batches() {
                f::concurrent_readers_with_config::<$engine>(&cfg());
            }
            #[test]
            fn randomized_model_check() {
                f::randomized_model_check_with_config::<$engine>(&cfg());
            }
            #[test]
            fn space_is_reclaimed() {
                f::space_is_reclaimed::<$engine>();
            }
            #[test]
            fn real_filesystem_smoke() {
                f::real_filesystem_smoke_with_config::<$engine>(&cfg());
            }
        }
        mod shared_crash {
            use super::*;
            use $crate::Durability;
            use $crate::harness::crash as c;
            use $crate::vfs::CrashKind;
            fn cfg() -> $crate::harness::SuiteConfig {
                $cfg
            }
            #[test]
            fn power_loss_sync() {
                c::run_with_config::<$engine>(CrashKind::PowerLoss, Durability::Sync, &cfg());
            }
            #[test]
            fn process_kill_sync() {
                c::run_with_config::<$engine>(CrashKind::ProcessKill, Durability::Sync, &cfg());
            }
            #[test]
            fn process_kill_buffered() {
                c::run_with_config::<$engine>(CrashKind::ProcessKill, Durability::Buffered, &cfg());
            }
            #[test]
            fn power_loss_at_every_io_point() {
                c::every_point_with_config::<$engine>(CrashKind::PowerLoss, Durability::Sync, &cfg());
            }
            #[test]
            fn process_kill_at_every_io_point() {
                c::every_point_with_config::<$engine>(CrashKind::ProcessKill, Durability::Sync, &cfg());
            }
            #[test]
            fn process_kill_at_every_io_point_buffered() {
                c::every_point_with_config::<$engine>(CrashKind::ProcessKill, Durability::Buffered, &cfg());
            }
            #[test]
            fn power_loss_buffered() {
                if $bpl {
                    c::run_with_config::<$engine>(CrashKind::PowerLoss, Durability::Buffered, &cfg());
                } else {
                    eprintln!("engine does not claim prefix consistency for buffered commits on power loss");
                }
            }
        }
        mod shared_faults {
            use super::*;
            #[test]
            fn single_io_failure_at_every_point() {
                $crate::harness::faults::single_failure_at_every_point_with_config::<$engine>(&$cfg);
            }
        }
        mod shared_corruption {
            use super::*;
            #[test]
            fn bit_flips_are_detected_or_harmless() {
                $crate::harness::corruption::bit_flips_with_config::<$engine>(&$cfg);
            }
            #[test]
            fn truncations_are_detected_or_harmless() {
                if $trunc {
                    $crate::harness::corruption::truncations_with_config::<$engine>(&$cfg);
                } else {
                    eprintln!("engine does not claim to detect truncated files");
                }
            }
        }
    };
    ($engine:ty, buffered_power_loss_prefix = $bpl:expr) => {
        $crate::engine_test_suite!($engine, buffered_power_loss_prefix = $bpl, detects_truncation = true);
    };
    ($engine:ty, buffered_power_loss_prefix = $bpl:expr, detects_truncation = $trunc:expr) => {
        $crate::engine_test_suite!(@suite $engine, $bpl, $trunc, $crate::harness::SuiteConfig::default());
    };
    ($engine:ty, buffered_power_loss_prefix = $bpl:expr, config = $cfg:expr) => {
        $crate::engine_test_suite!(@suite $engine, $bpl, true, $cfg);
    };
    ($engine:ty, buffered_power_loss_prefix = $bpl:expr, detects_truncation = $trunc:expr, config = $cfg:expr) => {
        $crate::engine_test_suite!(@suite $engine, $bpl, $trunc, $cfg);
    };
}
