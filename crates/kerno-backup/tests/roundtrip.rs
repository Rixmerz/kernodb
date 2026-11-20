//! Backup, ship and restore against real databases and a file store: what a restore gives back is
//! compared with the original by its logical export, which also holds the commit sequences.

// The no-panic rule is for the library; a test panics to fail.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kerno::units::export::{export, export_snapshot};
use kerno::{Config, Db, Direction};
use kerno_backup::{Error, FsStore, Keeper, MemStore, ObjectStore, Shipper, Target, backup_base, catalog, restore};

struct Dirs(PathBuf);

impl Dirs {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dirs(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fill(db: &Db, from: usize, to: usize) {
    for i in from..to {
        let a = db.store("note", format!("note number {i}")).unwrap();
        let b = db.store("topic", format!("topic {i}")).unwrap();
        db.relate(a, b, "about").unwrap();
    }
}

fn state(db: &Db) -> Vec<u8> {
    export(db.unit_db()).unwrap()
}

fn open_restored(dir: &Path) -> Db {
    let db = Db::open(dir).unwrap();
    assert!(db.unit_db().verify_units().unwrap().is_clean());
    db
}

fn mem() -> Arc<dyn ObjectStore> {
    Arc::new(MemStore::default())
}

#[test]
fn a_base_taken_while_writing_and_the_log_after_it_restore_to_the_same_database() {
    let t = Dirs::new("hot");
    let dir = t.path("db");
    let db = Arc::new(Db::open(&dir).unwrap());
    fill(&db, 0, 20);
    let store = mem();

    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (db, stop) = (Arc::clone(&db), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut n = 1000;
            while !stop.load(Ordering::Relaxed) {
                fill(&db, n, n + 1);
                n += 1;
            }
        })
    };
    let base = backup_base(&dir, db.unit_db(), &*store, 10).unwrap();
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    assert!(base.entries > 0 && base.parts == 1);
    let end_seq = db.unit_db().raw_seq().unwrap();
    assert!(end_seq >= base.seq, "writes went on after the snapshot");

    let mut shipper = Shipper::resume(&dir, Arc::clone(&store)).unwrap();
    assert_eq!(shipper.shipped_seq(), base.seq);
    let report = shipper.ship(20).unwrap();
    assert_eq!((report.commits, report.shipped_seq), (end_seq - base.seq, end_seq));
    let want = state(&db);
    Arc::into_inner(db).unwrap().close().unwrap();

    let out = t.path("restored");
    let restored = restore(&*store, &out, Target::Latest).unwrap();
    assert_eq!((restored.base_seq, restored.seq), (base.seq, end_seq));
    let copy = open_restored(&out);
    assert_eq!(copy.unit_db().raw_seq().unwrap(), end_seq);
    assert_eq!(state(&copy), want);
    // The restored database takes writes: its sequence is the one the layer expects.
    fill(&copy, 5000, 5001);
    assert!(copy.unit_db().verify_units().unwrap().is_clean());
    let note = &copy.query(&kerno::UnitFilter { unit_type: Some("topic".into()), ..Default::default() }).unwrap()[0];
    assert_eq!(copy.related(note.id, Direction::In, Some("about")).unwrap().len(), 1);
}

#[test]
fn restoring_to_a_sequence_gives_the_state_the_database_had_then() {
    let t = Dirs::new("seq");
    let dir = t.path("db");
    let db = Db::open(&dir).unwrap();
    fill(&db, 0, 5);
    let store = mem();
    let base = backup_base(&dir, db.unit_db(), &*store, 10).unwrap();
    let mut shipper = Shipper::resume(&dir, Arc::clone(&store)).unwrap();

    fill(&db, 5, 8);
    // The state, and the sequence it is at, to compare a restore with later.
    let snap = db.unit_db().snapshot().unwrap();
    let (mid_seq, mid_state) = (snap.seq().0, export_snapshot(&snap).unwrap());
    drop(snap);
    fill(&db, 8, 12);
    shipper.ship(20).unwrap();
    db.close().unwrap();

    let out = t.path("mid");
    let r = restore(&*store, &out, Target::Seq(mid_seq)).unwrap();
    assert_eq!((r.base_seq, r.seq, r.replayed), (base.seq, mid_seq, mid_seq - base.seq));
    let copy = open_restored(&out);
    assert_eq!(state(&copy), mid_state);

    // Past what the store holds is an error, not a smaller database.
    let err = restore(&*store, &t.path("far"), Target::Seq(1_000_000)).unwrap_err();
    assert!(matches!(err, Error::Unreachable { wanted: 1_000_000, .. }), "{err}");
    assert!(!t.path("far").exists() && !t.path("far.restoring").exists());
    // Before the first base there is nothing to start from.
    assert!(matches!(restore(&*store, &t.path("early"), Target::Seq(base.seq - 1)), Err(Error::NoBase(_))));
}

#[test]
fn restoring_to_a_time_uses_the_bases_and_segments_of_that_moment() {
    let t = Dirs::new("time");
    let dir = t.path("db");
    let db = Db::open(&dir).unwrap();
    let store = mem();
    fill(&db, 0, 3);
    backup_base(&dir, db.unit_db(), &*store, 1_000).unwrap();
    let mut shipper = Shipper::resume(&dir, Arc::clone(&store)).unwrap();
    let mut at = Vec::new();
    for (i, ms) in [2_000u64, 3_000, 4_000].into_iter().enumerate() {
        fill(&db, 10 * (i + 1), 10 * (i + 1) + 2);
        shipper.ship(ms).unwrap();
        at.push(state(&db));
    }
    db.close().unwrap();

    for (ms, want) in [(2_000, &at[0]), (2_999, &at[0]), (3_000, &at[1]), (9_999, &at[2])] {
        let out = t.path(&format!("at-{ms}"));
        restore(&*store, &out, Target::Time(ms)).unwrap();
        assert_eq!(&state(&open_restored(&out)), want, "restored to {ms}");
    }
    // Before the base was taken there is nothing.
    assert!(matches!(restore(&*store, &t.path("none"), Target::Time(999)), Err(Error::NoBase(_))));
}

#[test]
fn a_hole_in_the_log_is_a_gap_and_a_new_base_mends_it() {
    let t = Dirs::new("gap");
    let dir = t.path("db");
    let store = mem();
    let db = Db::open(&dir).unwrap();
    fill(&db, 0, 3);
    backup_base(&dir, db.unit_db(), &*store, 1).unwrap();
    let mut shipper = Shipper::resume(&dir, Arc::clone(&store)).unwrap();
    fill(&db, 3, 6);
    // Closing and reopening replays the log into tables and deletes it: those commits were never shipped.
    db.close().unwrap();
    let db = Db::open(&dir).unwrap();
    fill(&db, 6, 8);
    let err = shipper.ship(2).unwrap_err();
    assert!(matches!(err, Error::Gap { .. }), "{err}");
    assert!(err.to_string().contains("take a new base backup"));

    backup_base(&dir, db.unit_db(), &*store, 3).unwrap();
    let mut shipper = Shipper::resume(&dir, Arc::clone(&store)).unwrap();
    fill(&db, 8, 10);
    assert_eq!(shipper.ship(4).unwrap().commits, 6);
    let want = state(&db);
    db.close().unwrap();
    let out = t.path("out");
    let r = restore(&*store, &out, Target::Latest).unwrap();
    assert_eq!(r.replayed, 6);
    assert_eq!(state(&open_restored(&out)), want);
}

#[test]
fn the_keeper_takes_a_new_base_after_a_gap_and_a_first_one_when_the_store_is_empty() {
    let t = Dirs::new("rebase");
    let dir = t.path("db");
    let store = mem();
    let db = Db::open(&dir).unwrap();
    fill(&db, 0, 2);
    let mut keeper = Keeper::new(&dir, Arc::clone(&store));
    let first = keeper.tick(db.unit_db(), 1).unwrap();
    assert!(first.base.is_some());
    fill(&db, 2, 4);
    let plain = keeper.tick(db.unit_db(), 2).unwrap();
    assert_eq!((plain.base, plain.shipped.commits), (None, 6));
    // Closing deletes the log with commits in it that were never shipped.
    fill(&db, 4, 6);
    db.close().unwrap();
    let db = Db::open(&dir).unwrap();
    fill(&db, 6, 7);
    let mended = keeper.tick(db.unit_db(), 3).unwrap();
    assert_eq!(mended.base.as_ref().map(|b| b.created_ms), Some(3));
    assert_eq!(mended.shipped.commits, 0);
    let want = state(&db);
    // The base was taken before the last write; the next tick ships it.
    assert_eq!(keeper.tick(db.unit_db(), 4).unwrap().shipped.commits, 0);
    db.close().unwrap();
    let out = t.path("out");
    restore(&*store, &out, Target::Latest).unwrap();
    assert_eq!(state(&open_restored(&out)), want);
}

#[test]
fn shipping_again_ships_only_what_is_new_and_a_new_shipper_goes_on_from_the_store() {
    let t = Dirs::new("again");
    let dir = t.path("db");
    let store = mem();
    let db = Db::open(&dir).unwrap();
    backup_base(&dir, db.unit_db(), &*store, 1).unwrap();
    let mut shipper = Shipper::resume(&dir, Arc::clone(&store)).unwrap();
    assert_eq!(shipper.ship(2).unwrap().commits, 0);
    fill(&db, 0, 2);
    assert_eq!(shipper.ship(3).unwrap().commits, 6);
    assert_eq!(shipper.ship(4).unwrap().commits, 0);
    fill(&db, 2, 3);
    let mut next = Shipper::resume(&dir, Arc::clone(&store)).unwrap();
    assert_eq!(next.shipped_seq(), shipper.shipped_seq());
    assert_eq!(next.ship(5).unwrap().commits, 3);
    let cat = catalog(&*store).unwrap();
    assert_eq!(cat.segments.len(), 2);
    assert_eq!(cat.segments[0].last + 1, cat.segments[1].first);
    db.close().unwrap();
}

#[test]
fn a_shipper_needs_a_base_of_its_own_database() {
    let t = Dirs::new("foreign");
    let store = mem();
    let (a, b) = (t.path("a"), t.path("b"));
    let (da, db) = (Db::open(&a).unwrap(), Db::open(&b).unwrap());
    assert!(matches!(Shipper::resume(&a, Arc::clone(&store)), Err(Error::NoBase(_))));
    fill(&da, 0, 1);
    backup_base(&a, da.unit_db(), &*store, 1).unwrap();
    assert!(matches!(Shipper::resume(&b, Arc::clone(&store)), Err(Error::Refused(_))));
    da.close().unwrap();
    db.close().unwrap();
}

#[test]
fn a_damaged_object_stops_the_restore_and_leaves_nothing_behind() {
    let t = Dirs::new("damage");
    let dir = t.path("db");
    let store_dir = t.path("store");
    let store: Arc<dyn ObjectStore> = Arc::new(FsStore::new(&store_dir));
    let db = Db::open(&dir).unwrap();
    fill(&db, 0, 4);
    backup_base(&dir, db.unit_db(), &*store, 1).unwrap();
    let mut shipper = Shipper::resume(&dir, Arc::clone(&store)).unwrap();
    for i in 0..3 {
        fill(&db, 4 + i, 5 + i);
        shipper.ship(2 + i as u64).unwrap();
    }
    db.close().unwrap();

    let keys = store.list("").unwrap();
    let last_segment = keys.iter().rfind(|k| k.starts_with("wal/")).unwrap().clone();
    for key in &keys {
        let path = store_dir.join(key);
        let good = std::fs::read(&path).unwrap();
        let mut bad = good.clone();
        let middle = bad.len() / 2;
        bad[middle] ^= 0x40;
        std::fs::write(&path, &bad).unwrap();
        let out = t.path("out");
        let err = restore(&*store, &out, Target::Latest).unwrap_err();
        assert!(matches!(err, Error::Corrupt(_)) && err.to_string().contains("damaged"), "{key}: {err}");
        assert!(!out.exists() && !t.path("out.restoring").exists(), "{key}");
        // A missing object is as bad. Without its manifest a base is not one; without the last
        // segment the store simply ends sooner, which only a target past it can notice.
        std::fs::remove_file(&path).unwrap();
        let gone = restore(&*store, &out, Target::Latest);
        if key.ends_with("MANIFEST") {
            assert!(matches!(gone, Err(Error::NoBase(_))), "{key} missing");
        } else if *key == last_segment {
            assert!(gone.is_ok(), "{key} missing");
            std::fs::remove_dir_all(&out).unwrap();
        } else {
            assert!(matches!(gone, Err(Error::Corrupt(_))), "{key} missing: {gone:?}");
        }
        std::fs::write(&path, &good).unwrap();
    }
    restore(&*store, &t.path("fine"), Target::Latest).unwrap();
}

#[test]
fn a_destination_in_use_is_refused() {
    let t = Dirs::new("refuse");
    let dir = t.path("db");
    let store = mem();
    let db = Db::open(&dir).unwrap();
    fill(&db, 0, 1);
    backup_base(&dir, db.unit_db(), &*store, 1).unwrap();
    db.close().unwrap();

    let err = restore(&*store, &dir, Target::Latest).unwrap_err();
    assert!(matches!(err, Error::Refused(_)) && err.to_string().contains("is not empty"), "{err}");
    let left = t.path("left");
    std::fs::create_dir_all(t.path("left.restoring")).unwrap();
    let err = restore(&*store, &left, Target::Latest).unwrap_err();
    assert!(err.to_string().contains("left by an earlier restore"), "{err}");
    // An empty directory is fine and is replaced.
    let empty = t.path("empty");
    std::fs::create_dir_all(&empty).unwrap();
    restore(&*store, &empty, Target::Latest).unwrap();
    assert!(empty.join("MANIFEST").exists());
}

#[test]
fn an_empty_database_and_a_store_without_a_base_restore_or_refuse_cleanly() {
    let t = Dirs::new("empty");
    let dir = t.path("db");
    let store = mem();
    assert!(matches!(restore(&*store, &t.path("x"), Target::Latest), Err(Error::NoBase(_))));
    let db = Db::open(&dir).unwrap();
    let base = backup_base(&dir, db.unit_db(), &*store, 1).unwrap();
    assert_eq!((base.seq, base.parts, base.entries), (0, 0, 0));
    db.close().unwrap();
    let out = t.path("out");
    assert_eq!(restore(&*store, &out, Target::Latest).unwrap().seq, 0);
    let copy = open_restored(&out);
    fill(&copy, 0, 1);
}

#[test]
fn the_keeper_follows_a_database_that_rotates_its_logs_faster_than_it_ships() {
    // A tiny write buffer makes the engine rotate its logs, flush them and delete them while the
    // writer runs: some ticks find a gap, and the keeper answers each with a new base backup.
    let t = Dirs::new("keeper");
    let dir = t.path("db");
    let store = mem();
    let config = Config { allow_unverified_platform: true, write_buffer_bytes: 4096, ..Config::default() };
    let db = Arc::new(Db::open_with(&dir, config).unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (db, stop) = (Arc::clone(&db), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut n = 0;
            while !stop.load(Ordering::Relaxed) && n < 400 {
                fill(&db, n, n + 1);
                n += 1;
            }
        })
    };
    let mut keeper = Keeper::new(&dir, Arc::clone(&store));
    let first = keeper.tick(db.unit_db(), 1).unwrap();
    assert!(first.base.is_some(), "the first tick takes the first base");
    let mut ms = 2;
    while !writer.is_finished() {
        // A writer this fast can outrun even a tick that has just taken a base: that tick reports
        // the gap, and the next one goes on from the base it took.
        match keeper.tick(db.unit_db(), ms) {
            Ok(_) | Err(Error::Gap { .. }) => {}
            Err(e) => panic!("{e}"),
        }
        ms += 1;
    }
    writer.join().unwrap();
    keeper.tick(db.unit_db(), ms).unwrap();
    // Nothing left to ship, and no base taken for it.
    let idle = keeper.tick(db.unit_db(), ms + 1).unwrap();
    assert!(idle.base.is_none());
    assert_eq!((idle.shipped.commits, idle.shipped.shipped_seq), (0, db.unit_db().raw_seq().unwrap()));
    let want = state(&db);
    Arc::into_inner(db).unwrap().close().unwrap();
    let out = t.path("out");
    restore(&*store, &out, Target::Latest).unwrap();
    assert_eq!(state(&open_restored(&out)), want);
}

/// How long a restore takes as the commits pile up: `cargo test -p kerno-backup --release --test
/// roundtrip -- --ignored --nocapture restore_time`. It prints, asserts nothing.
#[test]
#[ignore = "a measurement, not a check"]
fn restore_time_grows_with_the_commits() {
    let t = Dirs::new("time-of-restore");
    for commits in [10_000usize, 100_000, 1_000_000] {
        let dir = t.path(&format!("db-{commits}"));
        let config =
            Config { allow_unverified_platform: true, durability: kerno::Durability::Buffered, ..Config::default() };
        let db = Db::open_with(&dir, config).unwrap();
        let note = db.store("note", "x").unwrap();
        for i in 0..commits {
            let mut changes = db.unit_db().begin_changeset(kerno::units::ChangeMeta::default()).unwrap();
            changes
                .update(
                    note,
                    kerno::units::UnitUpdate {
                        payload: i.to_string().into_bytes(),
                        schema_version: None,
                        metadata: None,
                    },
                    None,
                )
                .unwrap();
            changes.commit().unwrap();
        }
        let seq = db.unit_db().raw_seq().unwrap();
        let store = mem();
        let started = std::time::Instant::now();
        let base = backup_base(&dir, db.unit_db(), &*store, 1).unwrap();
        let took_base = started.elapsed();
        db.close().unwrap();
        let out = t.path(&format!("out-{commits}"));
        let started = std::time::Instant::now();
        restore(&*store, &out, Target::Latest).unwrap();
        eprintln!(
            "{seq} commits, {} entries ({} bytes): base {took_base:?}, restore {:?}",
            base.entries,
            base.bytes,
            started.elapsed()
        );
    }
}

#[test]
fn a_database_holding_the_reserved_key_is_not_backed_up() {
    use kerno_core::WriteBatch;
    use kerno_core::contract::Engine;
    use kerno_core::vfs::StdVfs;
    use kerno_storage::KernoDb;

    let t = Dirs::new("reserved");
    let dir = t.path("db");
    fill(&Db::open(&dir).unwrap(), 0, 1);
    let config = Config { allow_unverified_platform: true, ..Config::default() };
    let engine = KernoDb::open(StdVfs::shared(), &dir, &config).unwrap();
    let mut batch = WriteBatch::new();
    batch.put(b"\xffkerno-backup-pad".to_vec(), b"x".to_vec());
    engine.commit(&batch).unwrap();
    engine.close().unwrap();
    let db = Db::open(&dir).unwrap();
    let err = backup_base(&dir, db.unit_db(), &*mem(), 1).unwrap_err();
    assert!(matches!(err, Error::Refused(_)), "{err}");
    assert!(err.to_string().contains("reserves for restores"), "{err}");
}
