//! `sync()` and group commit against `KernoDb`: the shared `sync()` and
//! multi-writer suites (with background threads and with the inline profile)
//! and the checks that need the engine's own counters.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Engine, Options, Snapshot, SyncEngine};
use kerno_core::{Durability, Error, WriteBatch};
use kerno_storage::{Config, KernoDb};
use kerno_testkit::vfs::{SimVfs, assert_dir_unchanged};

kerno_testkit::sync_test_suite!(kerno_storage::KernoDb);
kerno_testkit::group_commit_test_suite!(kerno_storage::KernoDb);

/// The same suites with flushes and compactions inline (profile
/// `kerno-inline`).
mod inline {
    kerno_testkit::sync_test_suite!(kerno_storage::testing::Profiled<kerno_storage::testing::Inline>);
    kerno_testkit::group_commit_test_suite!(kerno_storage::testing::Profiled<kerno_storage::testing::Inline>);
}

/// Group commit off: the same multi-writer guarantees with one commit per
/// frame.
mod serial {
    kerno_testkit::sync_test_suite!(kerno_storage::testing::Profiled<kerno_storage::testing::Serial>);
    kerno_testkit::group_commit_test_suite!(kerno_storage::testing::Profiled<kerno_storage::testing::Serial>);
}

fn put(db: &KernoDb, key: &str) -> u64 {
    let mut b = WriteBatch::new();
    b.put(key, "v");
    db.commit(&b).unwrap()
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap_or_else(|| panic!("no stat {name}"))
}

fn open(disk: &SimVfs, durability: Durability) -> KernoDb {
    let cfg = Config::from(&Options { durability, ..Options::small_for_tests() });
    KernoDb::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap()
}

/// `sync()` returns the durable sequence: the last acknowledged commit,
/// published as `durable_seq`; it costs one `fsync` in `Buffered` mode only
/// when frames are unsynced, and none in `Sync` mode.
#[test]
fn sync_returns_durable_seq() {
    let disk = SimVfs::new(1);
    let db = open(&disk, Durability::Buffered);
    assert_eq!(db.sync().unwrap(), 0, "a new database has nothing to sync");
    assert_eq!(stat(&db, "wal_syncs"), 0, "a log with only its header is not synced");
    for i in 1..=5 {
        assert_eq!(put(&db, &format!("k{i}")), i);
    }
    assert_eq!(stat(&db, "durable_seq"), 0, "Buffered commits are not durable until a sync");
    assert_eq!(db.sync().unwrap(), 5);
    assert_eq!((stat(&db, "durable_seq"), stat(&db, "wal_syncs")), (5, 1));
    // Nothing new to make durable: no I/O.
    let ops = disk.mutating_ops();
    assert_eq!(db.sync().unwrap(), 5);
    assert_eq!(disk.mutating_ops(), ops);
    assert_eq!(stat(&db, "wal_syncs"), 1);
    // The sync survives a power loss.
    put(&db, "k6");
    let image = disk.crash_now(kerno_testkit::vfs::CrashKind::PowerLoss);
    drop(db);
    let db = open(&image, Durability::Buffered);
    assert!(db.snapshot().unwrap().seq() >= 5);
    assert_eq!(stat(&db, "durable_seq"), db.snapshot().unwrap().seq(), "recovery makes what it found durable");

    // Sync mode: every acknowledged commit is durable, sync does no I/O.
    let disk = SimVfs::new(2);
    let db = open(&disk, Durability::Sync);
    assert_eq!(put(&db, "a"), 1);
    assert_eq!(stat(&db, "durable_seq"), 1);
    let ops = disk.mutating_ops();
    assert_eq!(db.sync().unwrap(), 1);
    assert_eq!(disk.mutating_ops(), ops, "Sync mode: sync is free");
}

/// A sync on a handle poisoned by a failed commit fails without queueing.
#[test]
fn sync_after_a_failed_commit_is_poisoned() {
    let disk = SimVfs::new(3);
    let db = Arc::new(open(&disk, Durability::Sync));
    put(&db, "a");
    disk.fail_when(|op| op.kind == kerno_testkit::vfs::OpKind::SyncData, 1, None);
    let mut b = WriteBatch::new();
    b.put("b", "v");
    assert!(matches!(db.commit(&b), Err(Error::Io(_))));
    assert!(matches!(db.sync(), Err(Error::Poisoned(_))));
    assert!(matches!(SyncEngine::sync(&*db), Err(Error::Poisoned(_))));
}

/// `sync()` on a read-only handle returns `ReadOnly`, like every other
/// mutation, and writes nothing.
#[test]
fn sync_on_read_only_returns_read_only() {
    let disk = SimVfs::new(1);
    let db = open(&disk, Durability::Buffered);
    put(&db, "k1");
    db.close().unwrap();
    for durability in [Durability::Buffered, Durability::Sync] {
        let cfg = Config {
            access: kerno_storage::AccessMode::ReadOnly,
            ..Config::from(&Options { durability, ..Options::small_for_tests() })
        };
        let before = disk.dump();
        let db = KernoDb::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap();
        assert!(matches!(db.sync(), Err(Error::ReadOnly(_))), "{durability:?}: sync on a read-only handle");
        db.close().unwrap();
        assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
    }
}
