//! Lifecycle stress of `kerno-storage` (acta of the Etapa 9 advance, E9A-05,
//! sections 3.2 and 5): many cycles of
//!
//! 1. concurrent commits from four writer threads (puts, deletes and
//!    `delete_range`, each writer on its own key prefix, calling `sync()` now
//!    and then),
//! 2. `compact_range`, once while the writers run and once after they stop
//!    (on the full range or on one writer's prefix),
//! 3. a clean close, then `verify()` of the closed directory,
//! 4. a reopen, whose contents are compared with a model.
//!
//! Every tenth cycle replaces the clean close with a power loss (a
//! `SimVfs::crash_now` image taken while the last commits are still in the
//! WAL, with no compaction after them): that is the only way "no confirmed
//! commit is lost" says anything, because after a clean close the recovered
//! sequence is trivially the last one.
//!
//! Thresholds (acta section 5, never lowered here):
//!
//! * 200 cycles by default; `KERNO_SOAK_CYCLES` overrides it.
//! * Zero confirmed commits lost: the recovered sequence is at least the
//!   highest sequence a writer was told, and the contents equal the model.
//! * `verify()` has no findings at every closed state; after a clean close it
//!   also says `clean`, and leaves no orphan, unknown file, WAL or
//!   `MANIFEST.tmp`; the reopen after a clean close replays no WAL byte.
//!
//! Reproducibility: the workload of every writer in every cycle is a pure
//! function of `KERNO_SOAK_SEED` (default [`DEFAULT_SEED`]), so the model is
//! the same on every run; only the interleaving of the threads varies. A
//! failure prints the seed and the cycle, and `KERNO_SOAK_SEED=<seed>
//! KERNO_SOAK_CYCLES=<cycle + 1>` replays the history up to it.
//!
//! Disk: `SimVfs` (in memory, `Durability::Sync`), engine threads on, 16 KiB
//! write buffer so that flushes and compactions happen in every cycle.
//!
//! MEASURED RUN TIME (2026-11-11, 4 cores, `cargo test -p kerno-storage
//! --all-features --release --test lifecycle_soak`, 200 cycles, 3 runs): 1.1
//! to 1.4 s of test time (1000 cycles: 6.1 s). Far under the two minutes the
//! acta worries about, so the full run is the default and nothing is gated.
//! In a debug build it is much slower; CI runs the suite with `--release`.
//!
//! MUTATION (acta 3.2), applied to a copy of the tree with its own target
//! directory, never to the working tree: `close_clean` in `src/db.rs` calls
//! `checkpoint_state(.., log_number, false)` instead of `true` (no CLEAN
//! checkpoint at close). The test fails at cycle 0:
//!
//! ```text
//! seed 0x50ace9a5 cycle 0: verify() found [Issue { file: "000040.wal", offset: None,
//! rule: "REC-9", detail: "required WAL missing (the MANIFEST says it is the oldest live log)" }]
//! ```
//!
//! Skipping the checkpoint call altogether fails the same way (REC-9). The
//! CLEAN flag itself is asserted too (`VerifyReport::clean`, and the reopen
//! after a clean close replays no WAL byte).

use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use kerno_core::contract::{Engine, Snapshot};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, WriteBatch};
use kerno_storage::{Background, CloseBudget, Config, KernoDb, TestConfig, VerifyOptions, VerifyReport, verify};
use kerno_testkit::harness::{Model, apply, assert_snapshot_matches};
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, SimVfs};

const DIR: &str = "/db";
const DB_ID: [u8; 16] = [0x50; 16];
const WRITERS: usize = 4;
/// Commits each writer makes in one cycle.
const COMMITS_PER_WRITER: u64 = 25;
/// Keys in each writer's prefix: small, so that deletes and rewrites hit keys
/// that earlier cycles left in tables.
const KEYS_PER_WRITER: u64 = 120;
/// Cycle `c` ends in a power loss instead of a clean close when
/// `c % CRASH_EVERY == CRASH_EVERY - 1`.
const CRASH_EVERY: u64 = 10;
const DEFAULT_CYCLES: u64 = 200;
const DEFAULT_SEED: u64 = 0x50AC_E9A5;
/// A cycle that takes longer than this is a hang (a follower waiting for a
/// leader that is gone): the process stops and says where.
const CYCLE_LIMIT: Duration = Duration::from_secs(300);

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn config() -> Config {
    Config {
        durability: Durability::Sync,
        write_buffer_bytes: 16 << 10,
        cache_bytes: 1 << 20,
        test: TestConfig { background: Background::Threads, db_id: Some(DB_ID) },
        close_budget: CloseBudget::Unbounded,
        ..Config::default()
    }
}

/// The counters that show the cycles did the work they are for; they restart
/// with every session, so each session adds its own.
const WORK_STATS: [&str; 5] =
    ["flushes", "compactions", "tombstones_dropped", "range_tombstones_dropped", "covered_points_dropped"];

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("no stat {name}")).1
}

fn prefix(writer: usize) -> String {
    format!("w{writer}/")
}

fn key(writer: usize, k: u64) -> String {
    format!("w{writer}/k{k:03}")
}

/// The bounds of one writer's prefix (`/` is followed by `0` in ASCII).
fn prefix_bounds(writer: usize) -> (Vec<u8>, Vec<u8>) {
    (prefix(writer).into_bytes(), format!("w{writer}0").into_bytes())
}

/// One batch of one to three operations on `writer`'s keys: mostly small puts,
/// some values of several KiB (they fill the write buffer), point deletes and
/// range deletions of a few keys.
fn random_batch(rng: &mut Rng, writer: usize) -> WriteBatch {
    let mut b = WriteBatch::new();
    for _ in 0..rng.range(1, 3) {
        let k = rng.below(KEYS_PER_WRITER);
        match rng.below(100) {
            0..=54 => {
                let len = rng.range(8, 300) as usize;
                b.put(key(writer, k), rng.bytes(len));
            }
            55..=64 => {
                let len = rng.range(2_000, 5_000) as usize;
                b.put(key(writer, k), rng.bytes(len));
            }
            65..=84 => {
                b.delete(key(writer, k));
            }
            _ => {
                let end = (k + rng.range(1, 20)).min(KEYS_PER_WRITER);
                b.delete_range(key(writer, k), key(writer, end));
            }
        }
    }
    b
}

/// What one writer thread did in a cycle.
#[derive(Default)]
struct WriterOutcome {
    /// Every acknowledged commit, in the writer's order.
    acked: Vec<(u64, WriteBatch)>,
    /// The first thing that went wrong, if anything.
    problem: Option<String>,
}

/// Commits `COMMITS_PER_WRITER` batches. Every few commits it calls `sync()`,
/// which must return at least the sequence of every commit acknowledged to any
/// writer before the call (`newest_acked` is read before the call).
fn run_writer(db: &KernoDb, writer: usize, mut rng: Rng, newest_acked: &AtomicU64) -> WriterOutcome {
    let mut out = WriterOutcome::default();
    for n in 1..=COMMITS_PER_WRITER {
        let batch = random_batch(&mut rng, writer);
        let seq = match db.commit(&batch) {
            Ok(seq) => seq,
            Err(e) => {
                out.problem = Some(format!("writer {writer} commit {n}: {e}"));
                return out;
            }
        };
        newest_acked.fetch_max(seq, Ordering::SeqCst);
        out.acked.push((seq, batch));
        if (n + writer as u64).is_multiple_of(6) {
            let floor = newest_acked.load(Ordering::SeqCst);
            match db.sync() {
                Ok(durable) if durable >= floor => {}
                Ok(durable) => {
                    out.problem = Some(format!("writer {writer}: sync returned {durable}, below acknowledged {floor}"));
                    return out;
                }
                Err(e) => {
                    out.problem = Some(format!("writer {writer} sync after commit {n}: {e}"));
                    return out;
                }
            }
        }
    }
    out
}

/// The state that survives from one cycle to the next.
struct Soak {
    seed: u64,
    disk: SimVfs,
    model: Model,
    /// The highest sequence any writer was told, over all cycles.
    confirmed: u64,
    /// The last cycle ended in a clean close.
    closed_clean: bool,
    created: bool,
    /// Engine work counted over all sessions ([`WORK_STATS`]).
    work: [u64; WORK_STATS.len()],
}

impl Soak {
    fn vfs(&self) -> Arc<dyn Vfs> {
        self.disk.as_vfs()
    }

    /// Opens the directory and compares what it holds with the model.
    fn reopen(&self, ctx: &str) -> KernoDb {
        let db = KernoDb::open(self.vfs(), Path::new(DIR), &config()).unwrap_or_else(|e| panic!("{ctx}: open: {e}"));
        let snap = db.snapshot().unwrap_or_else(|e| panic!("{ctx}: snapshot: {e}"));
        assert!(
            snap.seq() >= self.confirmed,
            "{ctx}: recovered sequence {} is below the last confirmed commit {}",
            snap.seq(),
            self.confirmed
        );
        let mut rng = Rng::new(self.seed ^ self.confirmed);
        assert_snapshot_matches(&snap, &self.model, &mut rng, &format!("{ctx}: reopened contents"));
        if self.created {
            if self.closed_clean {
                assert_eq!(stat(&db, "last_open_replayed_bytes"), 0, "{ctx}: a clean close left WAL bytes to replay");
                assert_eq!(stat(&db, "last_open_clean"), 1, "{ctx}: reopen after a clean close was not clean");
            } else {
                assert!(stat(&db, "last_open_replayed_bytes") > 0, "{ctx}: the power loss left no WAL to replay");
            }
        }
        db
    }

    fn verify_closed(&self, ctx: &str) -> VerifyReport {
        let report = verify(self.vfs(), Path::new(DIR), &VerifyOptions::default())
            .unwrap_or_else(|e| panic!("{ctx}: verify: {e}"));
        assert!(report.issues.is_empty(), "{ctx}: verify() found {:?}", report.issues);
        assert_eq!(
            report.clean,
            self.closed_clean,
            "{ctx}: the MANIFEST is{} CLEAN",
            if report.clean { "" } else { " not" }
        );
        // After a power loss the MANIFEST holds the flushed prefix only; the
        // rest is in the WAL and the reopen proves it is recovered.
        assert!(
            report.last_seq <= self.confirmed,
            "{ctx}: the MANIFEST ends at {} past {}",
            report.last_seq,
            self.confirmed
        );
        if self.closed_clean {
            assert_eq!(report.last_seq, self.confirmed, "{ctx}: a clean close records every commit");
            assert!(report.orphans.is_empty(), "{ctx}: orphans after a clean close: {:?}", report.orphans);
            assert!(report.unknown.is_empty(), "{ctx}: unknown files: {:?}", report.unknown);
            assert!(report.wals.is_empty(), "{ctx}: WALs after a clean close: {:?}", report.wals);
            assert!(!report.manifest_tmp, "{ctx}: MANIFEST.tmp after a clean close");
        }
        report
    }

    /// One cycle: reopen and compare, write concurrently with a
    /// `compact_range`, then end the session (clean close or power loss) and
    /// verify the directory it leaves.
    fn cycle(&mut self, cycle: u64) {
        let ctx = format!("seed {:#x} cycle {cycle}", self.seed);
        let crash = cycle % CRASH_EVERY == CRASH_EVERY - 1;
        let db = Arc::new(self.reopen(&ctx));
        self.created = true;
        let base = db.snapshot().expect("snapshot").seq();
        let newest_acked = AtomicU64::new(base);
        let mut rng = Rng::new(self.seed ^ cycle.wrapping_mul(0x9E37_79B9));
        let victim = rng.below(WRITERS as u64) as usize;
        let (lo, hi) = prefix_bounds(victim);

        let outcomes: Vec<WriterOutcome> = std::thread::scope(|s| {
            let writers: Vec<_> = (0..WRITERS)
                .map(|w| {
                    let (db, newest) = (&db, &newest_acked);
                    let wrng = Rng::new(self.seed ^ (cycle << 8) ^ (w as u64 + 1));
                    s.spawn(move || run_writer(db, w, wrng, newest))
                })
                .collect();
            // Skipped in the cycle that ends in a power loss, which must
            // leave commits in the WAL.
            let compact = (!crash).then(|| {
                let db = &db;
                let (lo, hi) = (&lo, &hi);
                s.spawn(move || db.compact_range(Bound::Included(lo.as_slice()), Bound::Excluded(hi.as_slice())))
            });
            if let Some(c) = compact {
                c.join().expect("compactor thread").unwrap_or_else(|e| panic!("{ctx}: concurrent compact_range: {e}"));
            }
            writers.into_iter().map(|w| w.join().expect("writer thread")).collect()
        });

        // Every acknowledged sequence is unique and the range is contiguous.
        let mut seqs: Vec<u64> = Vec::new();
        for (w, out) in outcomes.iter().enumerate() {
            assert!(out.problem.is_none(), "{ctx}: writer {w}: {:?}", out.problem);
            assert!(out.acked.windows(2).all(|p| p[0].0 < p[1].0), "{ctx}: writer {w} saw sequences go backwards");
            seqs.extend(out.acked.iter().map(|(s, _)| *s));
            for (_, b) in &out.acked {
                apply(&mut self.model, b);
            }
        }
        seqs.sort_unstable();
        let total = WRITERS as u64 * COMMITS_PER_WRITER;
        assert_eq!(seqs, (base + 1..=base + total).collect::<Vec<_>>(), "{ctx}: acknowledged sequences");
        self.confirmed = base + total;

        let snap = db.snapshot().expect("snapshot");
        assert_eq!(snap.seq(), self.confirmed, "{ctx}: live sequence");
        let mut check_rng = Rng::new(self.seed ^ cycle);
        assert_snapshot_matches(&snap, &self.model, &mut check_rng, &format!("{ctx}: live contents"));
        drop(snap);
        assert!(db.sync().expect("final sync") >= self.confirmed, "{ctx}: final sync below the last commit");

        if crash {
            for (total, name) in self.work.iter_mut().zip(WORK_STATS) {
                *total += stat(&db, name);
            }
            // The writers are done and every commit was acknowledged with
            // `Durability::Sync`: a power loss now must lose none of them.
            let rebooted = self.disk.crash_now(CrashKind::PowerLoss);
            drop(db);
            self.disk = rebooted;
            self.closed_clean = false;
        } else {
            // Alternate the full range and one writer's prefix.
            let (start, end) = if cycle.is_multiple_of(2) {
                (Bound::Unbounded, Bound::Unbounded)
            } else {
                (Bound::Included(lo.as_slice()), Bound::Excluded(hi.as_slice()))
            };
            db.compact_range(start, end).unwrap_or_else(|e| panic!("{ctx}: compact_range: {e}"));
            for (total, name) in self.work.iter_mut().zip(WORK_STATS) {
                *total += stat(&db, name);
            }
            let db = Arc::try_unwrap(db).unwrap_or_else(|_| panic!("{ctx}: the handle is still shared"));
            db.close().unwrap_or_else(|e| panic!("{ctx}: close: {e}"));
            self.closed_clean = true;
        }
        self.verify_closed(&ctx);
    }
}

/// Stops the process, naming the cycle, when a cycle takes longer than
/// [`CYCLE_LIMIT`]. Dropping the sender ends it.
fn watchdog(seed: u64) -> mpsc::Sender<u64> {
    let (tx, rx) = mpsc::channel::<u64>();
    std::thread::spawn(move || {
        let mut cycle = 0;
        loop {
            match rx.recv_timeout(CYCLE_LIMIT) {
                Ok(c) => cycle = c,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    eprintln!("lifecycle_soak: HUNG, seed {seed:#x}, cycle {cycle}");
                    std::process::exit(101);
                }
            }
        }
    });
    tx
}

fn soak(seed: u64, cycles: u64, current: &AtomicU64) {
    let progress = watchdog(seed);
    let mut soak = Soak {
        seed,
        disk: SimVfs::new(seed),
        model: Model::new(),
        confirmed: 0,
        closed_clean: true,
        created: false,
        work: [0; WORK_STATS.len()],
    };
    for cycle in 0..cycles {
        current.store(cycle, Ordering::SeqCst);
        progress.send(cycle).expect("watchdog");
        soak.cycle(cycle);
    }
    // A soak that never flushed, compacted or dropped a tombstone proves
    // nothing about the lifecycle it is named for.
    eprintln!(
        "lifecycle_soak: {cycles} cycles, seed {seed:#x}, work {:?}",
        WORK_STATS.iter().zip(soak.work).collect::<Vec<_>>()
    );
    for (name, total) in WORK_STATS.iter().zip(soak.work) {
        assert!(cycles < 10 || total >= cycles, "seed {seed:#x}: {name} happened {total} times in {cycles} cycles");
    }
    // The state the last cycle left must also reopen and equal the model.
    let ctx = format!("seed {seed:#x} final reopen after {cycles} cycles");
    let db = soak.reopen(&ctx);
    db.close().unwrap_or_else(|e| panic!("{ctx}: close: {e}"));
}

#[test]
fn lifecycle_soak_keeps_every_confirmed_commit_and_verifies_clean() {
    let seed = env_u64("KERNO_SOAK_SEED", DEFAULT_SEED);
    let cycles = env_u64("KERNO_SOAK_CYCLES", DEFAULT_CYCLES);
    let current = AtomicU64::new(0);
    if let Err(panic) = catch_unwind(AssertUnwindSafe(|| soak(seed, cycles, &current))) {
        eprintln!(
            "lifecycle_soak FAILED: KERNO_SOAK_SEED={seed} (cycle {}); replay with KERNO_SOAK_SEED={seed} KERNO_SOAK_CYCLES={}",
            current.load(Ordering::SeqCst),
            current.load(Ordering::SeqCst) + 1
        );
        resume_unwind(panic);
    }
}
