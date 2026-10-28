//! The inline background mode (`Background::Inline`, profile
//! `kerno-inline`): no threads, flushes and compactions in the commit path,
//! and a deterministic sequence of file-system operations
//! (phase1-design.md §1 item 6, §5.1).

use std::collections::BTreeSet;
use std::path::Path;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options};
use kerno_storage::testing::{Inline, Profiled};
use kerno_storage::{Background, CompactionConfig, Config, KernoDb};
use kerno_testkit::harness::crash::{commits, run_scenario, scenario};
use kerno_testkit::vfs::{OpKind, SimVfs, TraceEntry};

type InlineDb = Profiled<Inline>;

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap()
}

fn this_thread() -> String {
    std::thread::current().name().expect("test threads are named").to_string()
}

/// Runs scenario A (seed 7, the one every `every_point` sweep uses) on a
/// fresh device and returns its trace.
fn trace_scenario_a() -> Vec<TraceEntry> {
    let steps = scenario::<InlineDb>(7);
    let disk = SimVfs::new(7);
    disk.start_trace();
    let (acked, attempted) = run_scenario::<InlineDb>(&disk, &steps, &Options::small_for_tests());
    assert_eq!((acked, attempted), (commits(&steps).len() as u64, commits(&steps).len() as u64));
    let trace = disk.trace();
    assert_eq!(trace.len() as u64, disk.mutating_ops(), "every mutating operation is traced");
    trace
}

#[test]
fn inline_io_trace_is_deterministic() {
    let first = trace_scenario_a();
    let second = trace_scenario_a();
    assert!(!first.is_empty());
    if let Some(i) = (0..first.len().min(second.len())).find(|&i| first[i] != second[i]) {
        panic!("traces differ at operation {}: {:?} vs {:?}", i + 1, first[i], second[i]);
    }
    assert_eq!(first.len(), second.len(), "traces have different lengths");

    // No background thread touched the device: every operation ran on the
    // thread that called the engine.
    let me = this_thread();
    let foreign = first.iter().find(|e| e.thread != me);
    assert!(foreign.is_none(), "operation from another thread: {foreign:?}");

    // Every probe phase of the M0 engine shows up in scenario A.
    let phases: BTreeSet<&str> = first.iter().map(|e| e.op.phase).collect();
    for phase in ["create", "session", "commit", "rotate", "flush", "compact", "close", "checkpoint", "delete"] {
        assert!(phases.contains(phase), "no operation labelled {phase:?}; labels seen: {phases:?}");
    }
    assert!(
        !phases.contains(""),
        "an operation ran outside any phase: {:?}",
        first.iter().find(|e| e.op.phase.is_empty())
    );
}

#[test]
fn inline_flush_happens_in_commit_path() {
    let disk = SimVfs::new(1);
    let cfg = Config {
        write_buffer_bytes: 16 << 10,
        test: kerno_storage::TestConfig { background: Background::Inline, db_id: None },
        ..Config::default()
    };
    let db = KernoDb::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap();
    let me = this_thread();
    let mut seq = 0;
    // Fill the first memtable; the commit after it is full rotates.
    let flushing_commit = loop {
        disk.start_trace();
        let mut b = WriteBatch::new();
        b.put(format!("key-{seq:04}"), vec![seq as u8; 700]);
        seq = db.commit(&b).unwrap();
        if stat(&db, "flushes") > 0 {
            break disk.trace();
        }
        assert!(seq < 100, "no flush after {seq} commits of 700 bytes into a 16 KiB memtable");
        assert!(
            disk.trace().iter().all(|e| e.op.phase == "commit"),
            "a commit that does not rotate only writes its WAL"
        );
    };
    // When the commit returns, the full memtable is already an L0 table.
    assert_eq!(stat(&db, "flushes"), 1);
    assert_eq!(stat(&db, "memtable_rotations"), 1);
    assert_eq!(stat(&db, "immutable_memtables"), 0);
    assert_eq!(stat(&db, "l0_files"), 1);

    // All in this thread, in protocol order: rotate (new WAL), flush (SST,
    // MANIFEST), delete (the old WAL), and only then the commit's own record
    // in the new WAL.
    assert!(flushing_commit.iter().all(|e| e.thread == me), "{flushing_commit:#?}");
    let mut order: Vec<&str> = flushing_commit.iter().map(|e| e.op.phase).collect();
    order.dedup();
    assert_eq!(order, ["rotate", "flush", "delete", "commit"], "{flushing_commit:#?}");
    let first_flush = flushing_commit.iter().find(|e| e.op.phase == "flush").unwrap();
    assert_eq!(first_flush.op.kind, OpKind::Create);
    assert!(first_flush.op.path.extension().is_some_and(|x| x == "sst"), "{first_flush:?}");
    let last = flushing_commit.last().unwrap();
    assert_eq!(last.op.kind, OpKind::SyncData, "Sync commit ends with the WAL sync");
    let new_wal =
        &flushing_commit.iter().find(|e| e.op.phase == "rotate" && e.op.kind == OpKind::Create).unwrap().op.path;
    assert_eq!(&last.op.path, new_wal, "the commit's record goes to the WAL created by the rotation");

    db.close().unwrap();
}

/// `TestHooks` in both modes: `flush` and `compact_once` step the engine
/// inline, and with threads `flush` and `wait_idle` wait for the background
/// work.
mod hooks {
    use super::*;
    use kerno_core::contract::Snapshot;
    use kerno_storage::TestConfig;

    fn open(disk: &SimVfs, background: Background) -> KernoDb {
        // The M0 compaction policy: four L0 files make a compaction due,
        // whatever their key ranges.
        let cfg = Config {
            write_buffer_bytes: 16 << 10,
            test: TestConfig { background, db_id: None },
            compaction: CompactionConfig::m0(16 << 10),
            ..Config::default()
        };
        KernoDb::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap()
    }

    fn put(db: &KernoDb, i: u32) {
        let mut b = WriteBatch::new();
        b.put(format!("key-{:05}", i % 300), vec![i as u8; 200]);
        db.commit(&b).unwrap();
    }

    #[test]
    fn inline_hooks_flush_and_compact_one_step_at_a_time() {
        let disk = SimVfs::new(1);
        let db = open(&disk, Background::Inline);
        // An empty memtable is not rotated.
        db.hooks().flush().unwrap();
        assert_eq!(stat(&db, "memtable_rotations"), 0);
        // Four flushed memtables: L0 reaches the compaction trigger, and
        // `flush` itself does not compact.
        for round in 0..4u32 {
            for i in 0..20 {
                put(&db, round * 20 + i);
            }
            disk.start_trace();
            db.hooks().flush().unwrap();
            let trace = disk.trace();
            assert!(trace.iter().all(|e| ["rotate", "flush", "delete"].contains(&e.op.phase)), "{trace:#?}");
            assert_eq!(stat(&db, "l0_files"), round as u64 + 1);
            assert_eq!(stat(&db, "immutable_memtables"), 0);
            assert_eq!(stat(&db, "memtable_bytes"), 0);
        }
        assert_eq!(stat(&db, "compactions"), 0);
        disk.start_trace();
        assert!(db.hooks().compact_once().unwrap(), "L0 holds 4 files: a compaction is due");
        let trace = disk.trace();
        assert!(trace.iter().all(|e| e.op.phase == "compact" || e.op.phase == "delete"), "{trace:#?}");
        assert_eq!(stat(&db, "compactions"), 1);
        assert_eq!(stat(&db, "l0_files"), 0);
        assert!(stat(&db, "l1_files") > 0);
        assert!(!db.hooks().compact_once().unwrap(), "nothing left to compact");
        db.hooks().wait_idle().unwrap();
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), 80);
        assert_eq!(snap.get(b"key-00079").unwrap(), Some(vec![79u8; 200]));
        drop(snap);
        db.close().unwrap();
    }

    #[test]
    fn threads_hooks_wait_for_background_work() {
        let disk = SimVfs::new(2);
        let db = open(&disk, Background::Threads);
        for i in 0..400 {
            put(&db, i);
        }
        db.hooks().flush().unwrap();
        // Every memtable written, including the active one.
        assert_eq!(stat(&db, "immutable_memtables"), 0);
        assert_eq!(stat(&db, "memtable_bytes"), 0);
        assert!(stat(&db, "flushes") >= 4, "{:?}", db.stats());
        db.hooks().wait_idle().unwrap();
        // Idle: no compaction due (L0 below its trigger), and nothing
        // touches the device any more.
        assert!(stat(&db, "l0_files") < 4, "{:?}", db.stats());
        let before = disk.mutating_ops();
        assert!(!db.hooks().compact_once().unwrap(), "idle: no compaction was due, so none ran");
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(disk.mutating_ops(), before, "background work after wait_idle returned");
        assert_eq!(db.snapshot().unwrap().seq(), 400);
        db.close().unwrap();
    }

    /// With threads, the background thread runs the compactions, but
    /// `compact_once` still reports one that was running when it was called
    /// (`TestHooks` contract: `true` if one ran).
    #[test]
    fn threads_compact_once_reports_a_running_compaction() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Condvar, Mutex};
        use std::time::Duration;

        use kerno_testkit::vfs::{HookAction, HookPoint, HookVfs};

        // Holds the first compaction at its first I/O until released:
        // (entered, released).
        let gate = Arc::new((Mutex::new((false, false)), Condvar::new()));
        let hook_gate = gate.clone();
        let disk = SimVfs::new(4);
        let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
            if op.phase == "compact" && op.point == HookPoint::Before {
                let (lock, cv) = &*hook_gate;
                let mut g = lock.lock().unwrap();
                if !g.1 {
                    g.0 = true;
                    cv.notify_all();
                    while !g.1 {
                        g = cv.wait(g).unwrap();
                    }
                }
            }
            HookAction::Proceed
        });
        let cfg = Config {
            write_buffer_bytes: 16 << 10,
            test: TestConfig { background: Background::Threads, db_id: None },
            compaction: CompactionConfig::m0(16 << 10),
            ..Config::default()
        };
        let db = KernoDb::open(vfs, Path::new("/db"), &cfg).unwrap();
        // Four flushed memtables: L0 reaches the compaction trigger and the
        // background thread starts a compaction, which the hook holds.
        for round in 0..4u32 {
            for i in 0..20 {
                put(&db, round * 20 + i);
            }
            db.hooks().flush().unwrap();
        }
        {
            let (lock, cv) = &*gate;
            let _held = cv.wait_while(lock.lock().unwrap(), |g| !g.0).unwrap();
        }
        assert_eq!(stat(&db, "compactions") + stat(&db, "trivial_moves"), 0);
        // Release it once `compact_once` is (almost certainly) waiting.
        let calling = Arc::new(AtomicBool::new(false));
        let releaser = {
            let (gate, calling) = (gate.clone(), calling.clone());
            std::thread::spawn(move || {
                while !calling.load(Ordering::SeqCst) {
                    std::thread::yield_now();
                }
                std::thread::sleep(Duration::from_millis(200));
                let (lock, cv) = &*gate;
                lock.lock().unwrap().1 = true;
                cv.notify_all();
            })
        };
        calling.store(true, Ordering::SeqCst);
        assert!(db.hooks().compact_once().unwrap(), "a compaction was running when compact_once was called");
        releaser.join().unwrap();
        assert!(stat(&db, "compactions") >= 1, "{:?}", db.stats());
        assert!(stat(&db, "l0_files") < 4, "{:?}", db.stats());
        assert!(!db.hooks().compact_once().unwrap(), "idle: no compaction was due, so none ran");
        assert_eq!(db.snapshot().unwrap().seq(), 80);
        db.close().unwrap();
    }
}
