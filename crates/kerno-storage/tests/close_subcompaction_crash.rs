//! A crash at every compaction operation of a close that runs its compaction
//! as parallel subcompactions (failure-matrix.md, row SC; the default close:
//! `Background::Threads` with `close_subcompactions: 8`).
//!
//! The other sweeps run inline, which is one subcompaction. Here the
//! compaction of the close writes its outputs from several threads as orphan
//! tables and installs them with a single `COMPACTION` record; a crash at any
//! operation before or after that record must recover a prefix of the
//! commits with every acknowledged one in it. With threads the operation
//! count varies a little between runs; the sweep goes past it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use kerno_core::contract::{Engine, Options};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, WriteBatch};
use kerno_storage::testing::{Profile, Profiled};
use kerno_storage::{Background, CloseBudget, Config};
use kerno_testkit::harness::crash::{OpFilter, Step, count_matching, every_matching_point};
use kerno_testkit::vfs::{CrashKind, HookAction, HookOp, HookOpKind, HookPoint, HookVfs, SimVfs};

/// The default close: background threads, subcompactions of the close floor,
/// an unbounded close, and a low L0 trigger so that the close has work.
struct Threaded;

impl Profile for Threaded {
    const NAME: &'static str = "kerno-threaded";

    fn adjust(cfg: &mut Config) {
        cfg.test.background = Background::Threads;
        cfg.close_budget = CloseBudget::Unbounded;
        cfg.compaction.l0_trigger = 2;
        assert_eq!(cfg.compaction.close_subcompactions, 8, "the default close splits its compaction");
    }
}

type Db = Profiled<Threaded>;

/// `n` batches of 8 keys each. The keys of a batch are 37 apart (mod 400), so a write buffer's
/// worth of batches spans the whole key space and every L0 file overlaps every L1 file. The values
/// are 300 incompressible bytes: with a value that compresses (`vec![i; 300]`) all 400 keys fit in
/// one table, L1 is then a single file, and a close compaction cannot be split (a split cuts at L1
/// file boundaries) whenever the background compaction has not got to rewrite L1 before the close
/// stops it, which is a race.
fn batches(n: u64) -> Vec<WriteBatch> {
    (0..n)
        .map(|i| {
            let mut b = WriteBatch::new();
            for j in 0..8 {
                // xorshift64 seeded by the key's turn: reproducible bytes LZ4 cannot shrink.
                let mut x = (i * 8 + j) * 0x9E37_79B9_7F4A_7C15 + 1;
                let value = (0..300)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        x as u8
                    })
                    .collect::<Vec<u8>>();
                b.put(format!("k{:04}", (i * 8 + j) * 37 % 400), value);
            }
            b
        })
        .collect()
}

/// Open, commit, close, reopen, commit, close.
fn steps() -> Vec<Step<Db>> {
    let all = batches(160);
    let mut steps = vec![Step::Open];
    steps.extend(all[..110].iter().cloned().map(Step::Commit));
    steps.extend([Step::Close, Step::Open]);
    steps.extend(all[110..].iter().cloned().map(Step::Commit));
    steps.push(Step::Close);
    steps
}

fn opts() -> Options {
    Options { durability: Durability::Sync, ..Options::small_for_tests() }
}

fn compact_op() -> OpFilter {
    Arc::new(|op| op.phase == "compact")
}

/// The sweep means something only if the close compaction really is split:
/// count the threads that write compaction output while the close runs. A split runs each of its
/// ranges on its own unnamed scoped thread; the background compactors (`kerno-compact-<i>`) are
/// not the close and do not count.
///
/// One close cannot be required to split: the close stops the background threads, and a
/// background compaction that finishes the L0 backlog just before it stops leaves the close with
/// nothing to merge from L0 (about 1 close in 100). What the default close owes is that it splits
/// when it has L0 work over several L1 files, so the test looks at up to `ROUNDS` closes of the
/// same backlog and fails only if none of them did.
#[test]
fn the_close_compaction_writes_from_several_threads() {
    const ROUNDS: usize = 10;
    let disk = SimVfs::new(1);
    let closing = Arc::new(AtomicBool::new(false));
    let writers = Arc::new(Mutex::new(HashSet::new()));
    let (flag, seen) = (closing.clone(), writers.clone());
    let vfs: Arc<dyn Vfs> = HookVfs::wrap(disk.as_vfs(), move |op: &HookOp<'_>| {
        let thread = std::thread::current();
        let background = thread.name().is_some_and(|n| n.starts_with("kerno-compact"));
        if flag.load(Ordering::SeqCst)
            && !background
            && op.phase == "compact"
            && op.kind == HookOpKind::WriteAt
            && op.point == HookPoint::Before
        {
            seen.lock().unwrap().insert(thread.id());
        }
        HookAction::Proceed
    });
    // The same two sessions as `steps`: the first close fills the levels
    // below L0, the second one is the one that must split.
    let all = batches(160);
    let dir = std::path::Path::new("/db");
    let db = Db::open(vfs.clone(), dir, &opts()).unwrap();
    for b in &all[..110] {
        db.commit(b).unwrap();
    }
    db.close().unwrap();
    let mut threads = 0;
    for _ in 0..ROUNDS {
        let db = Db::open(vfs.clone(), dir, &opts()).unwrap();
        for b in &all[110..] {
            db.commit(b).unwrap();
        }
        writers.lock().unwrap().clear();
        closing.store(true, Ordering::SeqCst);
        db.close().unwrap();
        closing.store(false, Ordering::SeqCst);
        threads = writers.lock().unwrap().len();
        if threads >= 2 {
            return;
        }
    }
    panic!("no close in {ROUNDS} wrote from more than {threads} thread(s): the sweep would not cover subcompactions");
}

#[test]
fn close_with_subcompactions_crash_at_every_compaction_op() {
    let steps = steps();
    let n = count_matching::<Db>(&steps, &opts(), &|_| {}, &compact_op());
    assert!(n > 10, "the scenario performs {n} compaction operations");
    for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill, CrashKind::ProcessKillKeepPending] {
        every_matching_point::<Db>(kind, Durability::Sync, &steps, compact_op());
    }
}
