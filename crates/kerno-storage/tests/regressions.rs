//! Regression tests for defects found by the adversarial review of M0
//! (each one was first reproduced by an independent verifier; the first two
//! modules are those reproductions, unchanged, asserting correct behaviour).

// Damage in the middle of a synced WAL was taken for a torn tail (acknowledged commits silently dropped, WAL deleted).
mod wal_damage {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use kerno_core::contract::{Engine, Options, Snapshot};
    use kerno_core::vfs::Vfs;
    use kerno_core::{Durability, WriteBatch};
    use kerno_storage::KernoDb;
    use kerno_testkit::vfs::{CrashKind, SimVfs};

    const DIR: &str = "/db";
    // Record layout for put("k<i>", [i;100]): header 16 + body (1 count + 1 tag + 1 klen + 2 key + 1 vlen + 100) = 122.
    const REC: u64 = 122;

    fn open(vfs: &Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<KernoDb> {
        <KernoDb as Engine>::open(vfs.clone(), Path::new(DIR), opts)
    }

    fn wal_path(sim: &SimVfs) -> PathBuf {
        let wals: Vec<PathBuf> =
            sim.files().into_iter().map(|(p, _)| p).filter(|p| p.extension().is_some_and(|e| e == "wal")).collect();
        assert_eq!(wals.len(), 1, "expected exactly one WAL, got {wals:?}");
        wals[0].clone()
    }

    fn write_ten(sim: &SimVfs, opts: &Options) -> KernoDb {
        let vfs = sim.as_vfs();
        let db = open(&vfs, opts).unwrap();
        for i in 0..10u8 {
            let mut b = WriteBatch::new();
            b.put(format!("k{i}"), vec![i; 100]);
            assert_eq!(db.commit(&b).unwrap(), i as u64 + 1);
        }
        db
    }

    fn check(label: &str, disk: &SimVfs, opts: &Options, corrupt_off: Option<(u64, u8)>) {
        let wal = wal_path(disk);
        let len = disk.files().into_iter().find(|(p, _)| *p == wal).unwrap().1;
        assert_eq!(len, 10 * REC, "{label}: unexpected WAL length");
        if let Some((off, mask)) = corrupt_off {
            disk.corrupt_byte(&wal, off, mask);
        }
        let vfs = disk.as_vfs();
        match open(&vfs, opts) {
            Err(e) => eprintln!("{label}: open reported error (GOOD): {e}"),
            Ok(db) => {
                let snap = db.snapshot().unwrap();
                let k9 = snap.get(b"k9").unwrap();
                let k1 = snap.get(b"k1").unwrap();
                let mut names = vfs.list(Path::new(DIR)).unwrap();
                names.sort();
                eprintln!(
                    "{label}: open OK, seq={} get(k1)={:?} get(k9)={:?} files={names:?}",
                    snap.seq(),
                    k1.map(|v| v.len()),
                    k9.as_ref().map(|v| v.len())
                );
                drop(snap);
                db.close().unwrap();
                assert_eq!(
                    (k9.is_some(), vfs.exists(&wal).unwrap()),
                    (true, false),
                    "{label}: acknowledged Sync commits silently lost"
                );
            }
        }
    }

    #[test]
    fn mid_wal_damage_after_unclean_shutdown() {
        let mut opts = Options::small_for_tests();
        opts.durability = Durability::Sync;

        // Control: power loss without damage keeps all 10 synced commits.
        {
            let sim = SimVfs::new(7);
            let db = write_ten(&sim, &opts);
            let after = sim.crash_now(CrashKind::PowerLoss);
            drop(db);
            let vfs = after.as_vfs();
            let snap_db = open(&vfs, &opts).unwrap();
            let s = snap_db.snapshot().unwrap();
            eprintln!("control (power loss, no damage): seq={}", s.seq());
            assert_eq!(s.seq(), 10);
            drop(s);
            snap_db.close().unwrap();
        }

        let mut failures = Vec::new();
        // Case A: power loss, then one bit flips in the value of record 3 (seq 3).
        // Case B: process kill (drop without close), same damage.
        // Case C: power loss, damage in record 3's len field.
        for (label, kill, off) in [
            ("A powerloss+value-bitflip", CrashKind::PowerLoss, 2 * REC + 16 + 50),
            ("B processkill+value-bitflip", CrashKind::ProcessKill, 2 * REC + 16 + 50),
            ("C powerloss+len-bitflip", CrashKind::PowerLoss, 2 * REC + 1),
        ] {
            let sim = SimVfs::new(11);
            let db = write_ten(&sim, &opts);
            let disk = match kill {
                CrashKind::PowerLoss => {
                    let after = sim.crash_now(CrashKind::PowerLoss);
                    drop(db);
                    after
                }
                CrashKind::ProcessKill | CrashKind::ProcessKillKeepPending => {
                    drop(db); // unclean shutdown: no close()
                    sim
                }
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                check(label, &disk, &opts, Some((off, 0x04)))
            }));
            if let Err(e) = r {
                let msg = e
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                failures.push(format!("{label}: {msg}"));
            }
        }
        assert!(failures.is_empty(), "DEFECT REPRODUCED:\n{}", failures.join("\n"));
    }
}

// A snapshot that outlived its handle deleted an obsolete SST while a new handle was recovering (open failed with NotFound).
mod outliving_snapshot {
    use std::any::Any;
    use std::io;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use kerno_core::WriteBatch;
    use kerno_core::contract::{Engine, Options};
    use kerno_core::vfs::{File, OpenMode, Vfs};
    use kerno_storage::KernoDb;
    use kerno_testkit::harness::{Model, apply, read_all};
    use kerno_testkit::vfs::SimVfs;
    use parking_lot::Mutex;

    const DIR: &str = "/db";

    type Hook = Box<dyn FnOnce() + Send>;

    /// Delegating Vfs that runs a one-shot hook right after `list()` returns.
    struct HookVfs {
        inner: Arc<dyn Vfs>,
        after_list: Mutex<Option<Hook>>,
    }

    impl Vfs for HookVfs {
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.inner.create_dir_all(path)
        }
        fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
            self.inner.open(path, mode)
        }
        fn exists(&self, path: &Path) -> io::Result<bool> {
            self.inner.exists(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.inner.remove_file(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.inner.rename(from, to)
        }
        fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
            let r = self.inner.list(dir);
            let hook = self.after_list.lock().take();
            if let Some(h) = hook {
                h();
            }
            r
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            self.inner.sync_dir(dir)
        }
        fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
            self.inner.lock(path)
        }
    }

    fn stat(db: &KernoDb, name: &str) -> u64 {
        db.stats().into_iter().find(|(n, _)| n == name).unwrap().1
    }

    fn settle(db: &KernoDb) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while stat(db, "immutable_memtables") > 0 || stat(db, "l0_files") >= 4 {
            assert!(Instant::now() < deadline, "background work did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn fill(db: &KernoDb, model: &mut Model, round: u32, n: u32) {
        for chunk in 0..n / 50 {
            let mut b = WriteBatch::new();
            for i in 0..50 {
                let k = format!("key-{:06}", (chunk * 50 + i) * 7 % n);
                b.put(k, format!("value-{round}-{i}-{}", "x".repeat((i % 200) as usize)));
            }
            db.commit(&b).unwrap();
            apply(model, &b);
        }
    }

    fn ssts(vfs: &dyn Vfs) -> Vec<String> {
        let mut v: Vec<String> =
            vfs.list(Path::new(DIR)).unwrap().into_iter().filter(|n| n.ends_with(".sst")).collect();
        v.sort();
        v
    }

    #[test]
    fn snapshot_outliving_handle_breaks_reopen() {
        let sim = SimVfs::new(7).as_vfs();
        let hook_vfs = Arc::new(HookVfs { inner: sim.clone(), after_list: Mutex::new(None) });
        let vfs: Arc<dyn Vfs> = hook_vfs.clone();
        let opts = Options::small_for_tests();

        // Handle A: create L0/L1 tables, pin them with a snapshot, then let
        // compaction replace (and mark obsolete) every file the snapshot uses.
        let a = <KernoDb as Engine>::open(vfs.clone(), Path::new(DIR), &opts).unwrap();
        let mut model = Model::new();
        fill(&a, &mut model, 0, 2000);
        settle(&a);
        let old = a.snapshot().unwrap();
        let old_model = model.clone();
        let pinned_ssts = ssts(&*sim);
        for round in 1..6 {
            fill(&a, &mut model, round, 2000);
        }
        settle(&a);
        assert!(stat(&a, "compactions") > 0);
        a.close().unwrap();

        // The old snapshot still works after its handle was closed, and still
        // pins files no MANIFEST references.
        assert!(read_all(&old).unwrap() == old_model);
        let after_close = ssts(&*sim);
        let still_pinned: Vec<&String> = pinned_ssts.iter().filter(|n| after_close.contains(n)).collect();
        println!("ssts pinned by old snapshot: {pinned_ssts:?}");
        println!("ssts present after close of A: {after_close:?}");
        println!("pinned-by-snapshot files still present after close: {still_pinned:?}");
        assert!(!still_pinned.is_empty(), "test setup: snapshot should keep some obsolete SST alive");

        // Simulate another thread dropping the old snapshot right after B's
        // recovery lists the directory (Table::drop runs on whatever thread
        // drops the last reference).
        let slot = Arc::new(Mutex::new(Some(old)));
        let slot2 = slot.clone();
        *hook_vfs.after_list.lock() = Some(Box::new(move || {
            let s = slot2.lock().take();
            drop(s);
        }));

        // Handle B: exclusive owner of the directory now (A's LOCK is released).
        let b = <KernoDb as Engine>::open(vfs.clone(), Path::new(DIR), &opts);
        match &b {
            Ok(_) => println!("B opened OK"),
            Err(e) => println!("B open failed: {e}"),
        }
        assert!(slot.lock().is_none(), "hook should have dropped the snapshot");
        let err = b.err().map(|e| e.to_string());
        assert!(err.is_none(), "open of a cleanly closed database failed: {err:?}");
    }

    /// Without the precise interleaving: B deletes the files of a still-live
    /// snapshot that belonged to a previous handle; the snapshot's later drop
    /// tries to unlink them again in B's directory.
    #[test]
    fn new_handle_deletes_files_of_live_old_snapshot() {
        let sim = SimVfs::new(8).as_vfs();
        let opts = Options::small_for_tests();
        let a = <KernoDb as Engine>::open(sim.clone(), Path::new(DIR), &opts).unwrap();
        let mut model = Model::new();
        fill(&a, &mut model, 0, 2000);
        settle(&a);
        let old = a.snapshot().unwrap();
        let old_model = model.clone();
        for round in 1..6 {
            fill(&a, &mut model, round, 2000);
        }
        settle(&a);
        a.close().unwrap();
        let before = ssts(&*sim);
        let b = <KernoDb as Engine>::open(sim.clone(), Path::new(DIR), &opts).unwrap();
        let after = ssts(&*sim);
        println!("before B open: {before:?}\nafter B open:  {after:?}");
        // Old snapshot reads through open SimFile handles.
        println!("old snapshot read after B removed its files: {:?}", read_all(&old).map(|m| m == old_model));
        drop(old);
        assert!(read_all(&b.snapshot().unwrap()).unwrap() == model);
        b.close().unwrap();
    }
}

// close() abandoned the running compaction and then added one more L0
// file, so many short sessions grew L0 without bound.
mod short_sessions {
    use std::path::Path;

    use kerno_core::contract::{Engine, Options};
    use kerno_storage::KernoDb;
    use kerno_testkit::harness::{Model, Workload, apply, read_all};
    use kerno_testkit::vfs::SimVfs;

    fn stat(db: &KernoDb, name: &str) -> u64 {
        db.stats().into_iter().find(|(k, _)| k == name).map(|(_, v)| v).unwrap()
    }

    #[test]
    fn l0_stays_bounded_across_many_short_sessions() {
        let vfs = SimVfs::new(3).as_vfs();
        let opts = Options::small_for_tests();
        let dir = Path::new("/db");
        let mut model = Model::new();
        let mut w = Workload::new(3);
        // A sizeable L1 first, so L0 compactions have real work to do.
        let db = <KernoDb as Engine>::open(vfs.clone(), dir, &opts).unwrap();
        for _ in 0..400 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        db.close().unwrap();
        for session in 0..40 {
            let db = <KernoDb as Engine>::open(vfs.clone(), dir, &opts).unwrap();
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
            db.close().unwrap();
            let db = <KernoDb as Engine>::open(vfs.clone(), dir, &opts).unwrap();
            let l0 = stat(&db, "l0_files");
            assert!(l0 < 4, "session {session}: {l0} L0 files after a clean close");
            drop(db);
        }
        let db = <KernoDb as Engine>::open(vfs.clone(), dir, &opts).unwrap();
        assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
    }
}

// The database directory was never synced into its parent.
mod directory {
    use std::any::Any;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use kerno_core::contract::{Engine, Options};
    use kerno_core::vfs::{File, OpenMode, Vfs};
    use kerno_storage::KernoDb;
    use kerno_testkit::vfs::SimVfs;
    use parking_lot::Mutex;

    struct Recorder {
        inner: Arc<dyn Vfs>,
        dir_syncs: Mutex<Vec<PathBuf>>,
    }

    impl Vfs for Recorder {
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.inner.create_dir_all(path)
        }
        fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
            self.inner.open(path, mode)
        }
        fn exists(&self, path: &Path) -> io::Result<bool> {
            self.inner.exists(path)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            self.inner.remove_file(path)
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.inner.rename(from, to)
        }
        fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
            self.inner.list(dir)
        }
        fn sync_dir(&self, dir: &Path) -> io::Result<()> {
            self.dir_syncs.lock().push(dir.to_path_buf());
            self.inner.sync_dir(dir)
        }
        fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
            self.inner.lock(path)
        }
    }

    #[test]
    fn open_makes_the_database_directory_durable_in_its_parent() {
        let rec = Arc::new(Recorder { inner: SimVfs::new(1).as_vfs(), dir_syncs: Mutex::new(Vec::new()) });
        let db =
            <KernoDb as Engine>::open(rec.clone(), Path::new("/data/apps/db"), &Options::small_for_tests()).unwrap();
        db.close().unwrap();
        let syncs = rec.dir_syncs.lock().clone();
        assert!(syncs.contains(&PathBuf::from("/data/apps")), "parent directory never synced: {syncs:?}");
    }
}

// I-15: a checksum-valid MANIFEST of an unknown format version is rejected
// and nothing in the directory is modified.
mod format_version {
    use std::path::Path;

    use kerno_core::contract::{Engine, Options};
    use kerno_core::vfs::OpenMode;
    use kerno_core::{Error, WriteBatch};
    use kerno_storage::KernoDb;
    use kerno_testkit::vfs::SimVfs;

    #[test]
    fn unknown_manifest_version_is_rejected_without_touching_files() {
        let disk = SimVfs::new(5);
        let vfs = disk.as_vfs();
        let dir = Path::new("/db");
        let opts = Options::small_for_tests();
        let db = <KernoDb as Engine>::open(vfs.clone(), dir, &opts).unwrap();
        let mut b = WriteBatch::new();
        b.put("k", "v");
        db.commit(&b).unwrap();
        db.close().unwrap();
        // MANIFEST = magic(8) | body_len u32 | crc u32 | body, body = format u32 | ...
        let f = vfs.open(&dir.join("MANIFEST"), OpenMode::Existing).unwrap();
        let mut buf = vec![0u8; f.len().unwrap() as usize];
        f.read_at(&mut buf, 0).unwrap();
        buf[16..20].copy_from_slice(&2u32.to_le_bytes());
        let crc = crc32c::crc32c(&buf[16..]);
        buf[12..16].copy_from_slice(&crc.to_le_bytes());
        f.write_at(&buf, 0).unwrap();
        f.sync_data().unwrap();
        let before = disk.files();
        let r = <KernoDb as Engine>::open(vfs.clone(), dir, &opts);
        assert!(matches!(r, Err(Error::Corruption(_))), "expected Corruption");
        assert_eq!(disk.files(), before, "a rejected open must not create, delete or resize files");
    }
}

// Only canonical names are engine files: a stray "1.wal" used to be opened
// and deleted as "000001.wal".
mod file_names {
    use std::path::Path;

    use kerno_core::contract::{Engine, Options};
    use kerno_core::vfs::OpenMode;
    use kerno_storage::KernoDb;
    use kerno_testkit::harness::{Model, Workload, apply, read_all};
    use kerno_testkit::vfs::SimVfs;

    #[test]
    fn non_canonical_names_are_left_alone() {
        let disk = SimVfs::new(8);
        let vfs = disk.as_vfs();
        let dir = Path::new("/db");
        let opts = Options::small_for_tests();
        let mut model = Model::new();
        let mut w = Workload::new(8);
        let db = <KernoDb as Engine>::open(vfs.clone(), dir, &opts).unwrap();
        for _ in 0..100 {
            let b = w.batch(&model);
            db.commit(&b).unwrap();
            apply(&mut model, &b);
        }
        drop(db); // unclean: leaves a live WAL behind
        for name in ["1.wal", "7.sst", "0000001.wal", "notes.txt"] {
            let f = vfs.open(&dir.join(name), OpenMode::CreateNew).unwrap();
            f.write_at(b"not an engine file", 0).unwrap();
            f.sync_data().unwrap();
        }
        vfs.sync_dir(dir).unwrap();
        let db = <KernoDb as Engine>::open(vfs.clone(), dir, &opts).unwrap();
        assert!(read_all(&db.snapshot().unwrap()).unwrap() == model);
        db.close().unwrap();
        for name in ["1.wal", "7.sst", "0000001.wal", "notes.txt"] {
            assert!(vfs.exists(&dir.join(name)).unwrap(), "{name} was removed");
        }
    }
}
