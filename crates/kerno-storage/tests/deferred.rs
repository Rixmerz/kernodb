//! Deferred commits (`WriteTxn::commit_deferred`, decision 12c of
//! `etapa3-design.md`): in `Sync` mode a deferred commit asks for no `fsync`
//! of its own and becomes durable with the next `fsync` of the log (a later
//! durable commit, `sync()`, a log rotation or `close`). A crash at any
//! point recovers a prefix of the commits that holds every durable commit
//! acknowledged before it.

use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::Path;

use kerno_core::contract::{Engine, Options, Snapshot, Txn, TxnEngine};
use kerno_core::{Durability, WriteBatch};
use kerno_storage::{Config, KernoDb};
use kerno_testkit::vfs::{CrashKind, SimVfs};

type State = BTreeMap<Vec<u8>, Vec<u8>>;

fn open(disk: &SimVfs, durability: Durability) -> KernoDb {
    let cfg = Config::from(&Options { durability, ..Options::small_for_tests() });
    KernoDb::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap()
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap_or_else(|| panic!("no stat {name}"))
}

fn dump(db: &KernoDb) -> State {
    db.snapshot().unwrap().scan(Bound::Unbounded, Bound::Unbounded, usize::MAX).unwrap().into_iter().collect()
}

/// The workload: commit `i` puts `k{i}` (3 KiB, so that the 16 KiB write
/// buffer rotates while deferred frames are unsynced), every fourth one
/// also deletes a range of older keys; commits with `i % 3 != 0` are
/// deferred. Returns whether each was deferred.
fn plan() -> Vec<bool> {
    (0..14).map(|i| i % 3 != 0).collect()
}

fn key(i: usize) -> Vec<u8> {
    format!("k{i:02}").into_bytes()
}

fn apply(state: &mut State, i: usize) {
    if i % 4 == 3 {
        let (a, b) = (key(i - 3), key(i - 1));
        state.retain(|k, _| !(a <= *k && *k < b));
    }
    state.insert(key(i), vec![i as u8; 3 << 10]);
}

/// Runs commit `i` of the workload; `Ok(seq)` once acknowledged.
fn commit(db: &KernoDb, i: usize, deferred: bool) -> kerno_core::Result<u64> {
    let mut t = TxnEngine::begin_write(db)?;
    if i % 4 == 3 {
        t.delete_range(&key(i - 3), &key(i - 1))?;
    }
    t.put(&key(i), &vec![i as u8; 3 << 10])?;
    if deferred {
        Txn::commit_deferred(t).map(|(s, waited)| {
            assert!(!waited, "KernoDb does not wait for a deferred commit");
            s
        })
    } else {
        Txn::commit(t)
    }
}

/// A deferred commit does no `fsync` and does not move `durable_seq`; the
/// next durable commit (or `sync`) makes it durable with one `fsync`.
#[test]
fn deferred_commit_waits_for_the_next_sync() {
    let disk = SimVfs::new(1);
    let db = open(&disk, Durability::Sync);
    let mut b = WriteBatch::new();
    b.put("a", "1");
    assert_eq!(db.commit(&b).unwrap(), 1);
    let syncs = stat(&db, "wal_syncs");
    assert_eq!(stat(&db, "durable_seq"), 1);
    let mut t = db.begin_write().unwrap();
    t.put(b"b", b"2").unwrap();
    assert_eq!(t.commit_deferred().unwrap(), 2);
    assert_eq!(stat(&db, "wal_syncs"), syncs, "a deferred commit has no fsync of its own");
    assert_eq!(stat(&db, "durable_seq"), 1, "not durable yet");
    assert_eq!(db.snapshot().unwrap().get(b"b").unwrap().as_deref(), Some(&b"2"[..]), "visible at once");
    // A power loss now may take it back, never the durable commit.
    let image = disk.crash_now(CrashKind::PowerLoss);
    drop(db);
    let db = open(&image, Durability::Sync);
    let got = dump(&db);
    assert_eq!(got.get(&b"a"[..]).map(Vec::as_slice), Some(&b"1"[..]));
    drop(db);

    // The next durable commit makes it durable with the same fsync.
    let disk = SimVfs::new(2);
    let db = open(&disk, Durability::Sync);
    let mut t = db.begin_write().unwrap();
    t.put(b"b", b"2").unwrap();
    assert_eq!(t.commit_deferred().unwrap(), 1);
    let syncs = stat(&db, "wal_syncs");
    let mut b = WriteBatch::new();
    b.put("c", "3");
    assert_eq!(db.commit(&b).unwrap(), 2);
    assert_eq!((stat(&db, "wal_syncs"), stat(&db, "durable_seq")), (syncs + 1, 2));
    // sync() on a log with nothing unsynced does no I/O again.
    let ops = disk.mutating_ops();
    assert_eq!(kerno_core::contract::SyncEngine::sync(&db).unwrap(), 2);
    assert_eq!(disk.mutating_ops(), ops);
    let image = disk.crash_now(CrashKind::PowerLoss);
    drop(db);
    let db = open(&image, Durability::Sync);
    assert_eq!(dump(&db).len(), 2, "both survive the power loss");
    drop(db);

    // sync() makes a deferred commit durable with one fsync.
    let disk = SimVfs::new(3);
    let db = open(&disk, Durability::Sync);
    let mut t = db.begin_write().unwrap();
    t.put(b"b", b"2").unwrap();
    t.commit_deferred().unwrap();
    let syncs = stat(&db, "wal_syncs");
    assert_eq!(kerno_core::contract::SyncEngine::sync(&db).unwrap(), 1);
    assert_eq!((stat(&db, "wal_syncs"), stat(&db, "durable_seq")), (syncs + 1, 1));
    let image = disk.crash_now(CrashKind::PowerLoss);
    drop(db);
    let db = open(&image, Durability::Sync);
    assert_eq!(dump(&db).len(), 1);
}

/// A crash before every mutating I/O of the workload (deferred and durable
/// commits mixed, rotations with unsynced deferred frames, then `close`)
/// recovers exactly the state after some prefix of the commits, which
/// holds every durable commit acknowledged before the crash; after a
/// process kill (and in `Buffered` mode, the same rules as ever) nothing
/// acknowledged is lost.
fn every_point(kind: CrashKind, durability: Durability) {
    let plan = plan();
    let mut states = vec![State::new()];
    for i in 0..plan.len() {
        let mut s = states.last().unwrap().clone();
        apply(&mut s, i);
        states.push(s);
    }
    let disk = SimVfs::new(7);
    let db = open(&disk, durability);
    let start = disk.mutating_ops();
    for (i, d) in plan.iter().enumerate() {
        assert_eq!(commit(&db, i, *d).unwrap(), i as u64 + 1);
    }
    assert!(stat(&db, "memtable_rotations") > 0, "the workload must rotate the write buffer");
    assert_eq!(dump(&db), *states.last().unwrap());
    db.close().unwrap();
    let total = disk.mutating_ops() - start;
    for point in 1..=total {
        let ctx = format!("{kind:?} {durability:?} before mutating op #{point} of {total}");
        let disk = SimVfs::new(7);
        let db = open(&disk, durability);
        disk.crash_after(point, kind);
        let mut acked = 0usize;
        let mut acked_durable = 0usize;
        for (i, d) in plan.iter().enumerate() {
            if commit(&db, i, *d).is_err() {
                break;
            }
            acked = i + 1;
            if !*d {
                acked_durable = i + 1;
            }
        }
        let closed = acked == plan.len() && db.close().is_ok();
        assert!(disk.has_crashed(), "{ctx}: the crash did not fire");
        let image = disk.take_crash_image().expect("crash image");
        let db = open(&image, durability);
        let n = db.snapshot().unwrap().seq() as usize;
        assert!(n <= plan.len(), "{ctx}: recovered {n} commits");
        assert_eq!(dump(&db), states[n], "{ctx}: not the state after the first {n} commits");
        if durability == Durability::Sync {
            assert!(n >= acked_durable, "{ctx}: durable commit {acked_durable} lost (recovered {n})");
        }
        if kind == CrashKind::ProcessKill || closed {
            assert!(n >= acked, "{ctx}: acknowledged commit {acked} lost (recovered {n})");
        }
        db.close().unwrap();
    }
}

#[test]
fn deferred_every_point_power_loss_sync() {
    every_point(CrashKind::PowerLoss, Durability::Sync);
}

#[test]
fn deferred_every_point_process_kill_sync() {
    every_point(CrashKind::ProcessKill, Durability::Sync);
}

#[test]
fn deferred_every_point_power_loss_buffered() {
    every_point(CrashKind::PowerLoss, Durability::Buffered);
}
