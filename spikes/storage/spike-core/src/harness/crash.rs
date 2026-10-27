//! Crash-consistency suite.
//!
//! Each seed runs several rounds of: open (recovery) → verify → random
//! commits → crash at a random mutating I/O operation. Crashes can also hit
//! *during* recovery. After every recovery the database must be exactly the
//! state after some commit `s` (its reported sequence), and `s` must be
//! within what the durability mode allows:
//!
//! | mode              | allowed recovered sequence `s`                  |
//! |-------------------|--------------------------------------------------|
//! | Sync (any crash)  | last acknowledged, or the one in flight          |
//! | Buffered + kill   | last acknowledged, or the one in flight          |
//! | Buffered + power  | anything from the round's starting point upward  |

use super::{Model, Workload, apply, assert_snapshot_matches, for_each_seed, open, read_all, seeds};
use crate::engine::{Durability, Engine, Options, Snapshot, WriteBatch};
use crate::rng::Rng;
use crate::vfs::{CrashKind, SimVfs};

const ROUNDS: usize = 5;
const COMMITS_PER_ROUND: u64 = 80;

pub fn run<E: Engine>(kind: CrashKind, durability: Durability) {
    let name = format!("crash {kind:?} {durability:?}");
    for_each_seed(&name, seeds(60), |seed| one_seed::<E>(seed, kind, durability));
}

fn one_seed<E: Engine>(seed: u64, kind: CrashKind, durability: Durability) {
    let mut rng = Rng::new(seed);
    let mut w = Workload::new(seed ^ 0xC0FFEE);
    // Small key spaces mean heavy overwrite churn, which is what drives
    // page reuse, flushes, compactions and log rewrites.
    w.key_space = rng.range(40, 800);
    let mut opts = Options::small_for_tests();
    opts.durability = durability;

    let mut disk = SimVfs::new(seed);
    // History of every batch ever attempted, indexed by sequence - 1.
    let mut history: Vec<WriteBatch> = Vec::new();
    // State known to be recovered at the start of the round, and its seq.
    let mut base_model = Model::new();
    let mut base_seq = 0u64;
    // Bounds for the next recovered sequence.
    let mut min_seq = 0u64;
    let mut max_seq = 0u64;
    let mut ops_per_commit_estimate = 20u64;

    for round in 0..ROUNDS {
        let ctx = format!("seed {seed} round {round}");
        let vfs = disk.as_vfs();
        if rng.chance(1, 5) {
            // Crash during recovery itself.
            disk.crash_after(rng.range(1, 40), kind);
        }
        let db: E = match open(&vfs, &opts) {
            Ok(db) => db,
            Err(e) => {
                assert!(disk.has_crashed(), "{ctx}: open failed without a crash: {e}");
                disk = disk.take_crash_image().expect("crash image");
                continue;
            }
        };
        let snap = match db.snapshot() {
            Ok(s) => s,
            Err(e) => {
                assert!(disk.has_crashed(), "{ctx}: snapshot failed without a crash: {e}");
                drop(db);
                disk = disk.take_crash_image().expect("crash image");
                continue;
            }
        };
        let s = snap.seq();
        assert!(
            s >= min_seq && s <= max_seq,
            "{ctx}: recovered sequence {s} outside the allowed range {min_seq}..={max_seq}"
        );
        // Rebuild the model at `s` from the last verified (durable) base.
        assert!(s >= base_seq, "{ctx}: recovered sequence {s} is older than verified base {base_seq}");
        let mut model = base_model.clone();
        for b in &history[base_seq as usize..s as usize] {
            apply(&mut model, b);
        }
        if !disk.has_crashed() {
            assert_snapshot_matches(&snap, &model, &mut rng, &format!("{ctx}: after recovery at seq {s}"));
        } else {
            // The crash fired in background work started by `open`; reads
            // that miss the cache now fail, so verify what can be read.
            if let Ok(all) = read_all(&snap) {
                assert!(all == model, "{ctx}: recovered state differs: {}", super::diff(&model, &all));
            }
        }
        drop(snap);
        history.truncate(s as usize);
        base_model = model.clone();
        base_seq = s;

        if !disk.has_crashed() {
            let budget = COMMITS_PER_ROUND * ops_per_commit_estimate;
            disk.crash_after(rng.range(1, budget.max(2)), kind);
        }
        let ops_before = disk.mutating_ops();
        let mut acked = s;
        let mut attempted = s;
        let mut commits_done = 0u64;
        for _ in 0..COMMITS_PER_ROUND {
            if disk.has_crashed() {
                break;
            }
            let b = w.batch(&model);
            history.push(b.clone());
            attempted += 1;
            match db.commit(&b) {
                Ok(seq) => {
                    assert_eq!(seq, attempted, "{ctx}: unexpected commit sequence");
                    acked = seq;
                    apply(&mut model, &b);
                    commits_done += 1;
                }
                Err(_) => break,
            }
        }
        if let Some(used) = disk.mutating_ops().saturating_sub(ops_before).checked_div(commits_done) {
            ops_per_commit_estimate = (ops_per_commit_estimate + used.max(1)) / 2 + 1;
        }
        let image = if disk.has_crashed() {
            drop(db);
            disk.take_crash_image().expect("crash image")
        } else {
            let image = disk.crash_now(kind);
            drop(db);
            image
        };
        disk = image;

        // Allowed range for the next recovery.
        min_seq = match (durability, kind) {
            (Durability::Buffered, CrashKind::PowerLoss) => base_seq,
            _ => acked,
        };
        max_seq = attempted;
    }
}
