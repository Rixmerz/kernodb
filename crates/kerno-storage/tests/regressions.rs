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
    use kerno_storage::format::header::HEADER_LEN;
    use kerno_storage::format::wal::{CanonicalCommit, FRAME_HEADER_LEN, frame_len};
    use kerno_testkit::vfs::{CrashKind, SimVfs};

    const DIR: &str = "/db";

    /// Bytes of the frame of `put("k<i>", [i; 100])`: the 32-byte head and
    /// the payload (1 op count + 1 op + 1 key length + 2 key + 1 value
    /// length + 100 value), from the format's own length functions.
    fn frame_bytes() -> u64 {
        let commit = CanonicalCommit { ranges: vec![], points: vec![(b"k0".to_vec(), Some(vec![0u8; 100]))] };
        frame_len(commit.encoded_len() as u32, false)
    }

    /// Offset of the value inside a frame: past the head, the op count, the
    /// op, the key length, the key and the value length.
    const VALUE_AT: u64 = FRAME_HEADER_LEN as u64 + 6;
    /// Offset of the second byte of `payload_len`, the first field of a frame.
    const LEN_FIELD_AT: u64 = 1;

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
        // The ten frames, then nothing or the zeros of a preallocated log.
        assert!(len >= HEADER_LEN as u64 + 10 * frame_bytes(), "{label}: unexpected WAL length");
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
        // Case A: power loss, then one bit flips in the value of frame 3 (seq 3).
        // Case B: process kill (drop without close), same damage.
        // Case C: power loss, damage in frame 3's len field.
        let frame3 = HEADER_LEN as u64 + 2 * frame_bytes();
        for (label, kill, off) in [
            ("A powerloss+value-bitflip", CrashKind::PowerLoss, frame3 + VALUE_AT + 50),
            ("B processkill+value-bitflip", CrashKind::ProcessKill, frame3 + VALUE_AT + 50),
            ("C powerloss+len-bitflip", CrashKind::PowerLoss, frame3 + LEN_FIELD_AT),
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
        while stat(db, "immutable_memtables") > 0 || stat(db, "l0_depth") >= 4 {
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
            // The depth is what the trigger bounds; flush split may double
            // the number of files.
            let (depth, files) = (stat(&db, "l0_depth"), stat(&db, "l0_files"));
            assert!(depth < 4, "session {session}: L0 depth {depth} after a clean close");
            assert!(files <= 8, "session {session}: {files} L0 files after a clean close");
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
// as incompatible, not as damage, and the directory is left byte for byte as
// it was (the LOCK file aside).
mod format_version {
    use std::path::Path;

    use kerno_core::contract::{Engine, Options};
    use kerno_core::vfs::OpenMode;
    use kerno_core::{Error, WriteBatch};
    use kerno_storage::KernoDb;
    use kerno_storage::format::header::{FORMAT_VERSION, HEADER_LEN, reseal_header};
    use kerno_testkit::vfs::{SimVfs, assert_dir_unchanged};

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
        // The header: format_version u32 at 8, covered by the CRC in its
        // last four bytes.
        let f = vfs.open(&dir.join("MANIFEST"), OpenMode::Existing).unwrap();
        let mut header = vec![0u8; HEADER_LEN];
        f.read_at(&mut header, 0).unwrap();
        header[8..12].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
        assert!(reseal_header(&mut header));
        f.write_at(&header, 0).unwrap();
        f.sync_data().unwrap();
        let before = disk.dump();
        let r = <KernoDb as Engine>::open(vfs.clone(), dir, &opts);
        assert!(matches!(r, Err(Error::Incompatible(_))), "expected Incompatible, got {:?}", r.err());
        assert_dir_unchanged(&before, &disk.dump(), &["LOCK"]);
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

// Found by the per-directory durability model (`DirDurability::PerDir`) of
// WP03b: a new database synced only the immediate parent of its directory.
// When `create_dir_all` also created that parent (`/data/apps/db` on an
// empty disk), the parent's own entry was never made durable, and a power
// loss after acknowledged `Sync` commits could remove the whole database.
//
// The first fix synced every ancestor up to `/`, and the review of WP03b
// found two defects in it. `sync_dir` opens the directory, which needs read
// permission, so an ancestor the process may only traverse (a home directory
// in mode 0711, app data on Android) made creating a database fail for good.
// And the current directory, the empty parent of a relative path, was never
// synced, so `db` or `data/apps/db` could still vanish. A new database now
// syncs the directory holding each directory it created (`.` included), and
// the higher ancestors only best effort, and only when its directory already
// existed (an earlier open may have created it and failed).
mod nested_new_directory {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use kerno_core::contract::{Engine, Snapshot};
    use kerno_core::{Error, WriteBatch};
    use kerno_storage::{Config, KernoDb};
    use kerno_testkit::vfs::{
        CrashKind, DirDurability, HookAction, HookOpKind, HookPoint, HookVfs, OpKind, SimVfs, StdVfs, Vfs,
    };
    use parking_lot::Mutex;

    fn commit_one(db: &KernoDb) {
        let mut b = WriteBatch::new();
        b.put("k", "v");
        assert_eq!(db.commit(&b).unwrap(), 1);
    }

    /// Reopens `dir` without creating it and checks the one acknowledged
    /// commit.
    fn check_one_commit(vfs: Arc<dyn Vfs>, dir: &Path, label: &str) {
        let reopen = Config { create_if_missing: false, ..Config::default() };
        let db = KernoDb::open(vfs, dir, &reopen).unwrap_or_else(|e| panic!("{label}: the database vanished: {e}"));
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), 1, "{label}: acknowledged commit lost");
        assert_eq!(snap.get(b"k").unwrap().as_deref(), Some(&b"v"[..]), "{label}");
    }

    #[test]
    fn new_nested_database_directory_survives_power_loss_with_per_dir_durability() {
        // Relative paths too: their outermost new directory is an entry of
        // the current directory, `.`.
        for dir in ["/data/apps/db", "db", "data/apps/db", "./db"] {
            for seed in 0..32 {
                let disk = SimVfs::new(seed);
                disk.set_dir_durability(DirDurability::PerDir);
                let db = KernoDb::open(disk.as_vfs(), Path::new(dir), &Config::default()).unwrap();
                commit_one(&db);
                let image = disk.crash_now(CrashKind::PowerLoss);
                drop(db);
                check_one_commit(image.as_vfs(), Path::new(dir), &format!("{dir}, seed {seed}"));
            }
        }
    }

    /// An earlier open created the directories and failed before syncing
    /// them all; the retried open finds the database directory already there
    /// and must still make its ancestors durable.
    #[test]
    fn retried_open_syncs_the_ancestors_an_earlier_open_created() {
        let dir = Path::new("/data/apps/db");
        for seed in 0..32 {
            let disk = SimVfs::new(seed);
            disk.set_dir_durability(DirDurability::PerDir);
            disk.fail_when(|op| op.kind == OpKind::SyncDir && op.path == Path::new("/data"), 1, None);
            let first = KernoDb::open(disk.as_vfs(), dir, &Config::default());
            assert!(matches!(first, Err(Error::Io(_))), "seed {seed}: {:?}", first.err());
            let db = KernoDb::open(disk.as_vfs(), dir, &Config::default()).unwrap();
            commit_one(&db);
            let image = disk.crash_now(CrashKind::PowerLoss);
            drop(db);
            check_one_commit(image.as_vfs(), dir, &format!("seed {seed}"));
        }
    }

    /// An earlier open died inside `create_dir_all`, which (like std's)
    /// makes one level at a time: `/data` and `/data/apps` exist, unsynced,
    /// and `db` does not. The retried open creates only `db`, and must still
    /// make the entries of `/data` and `/data/apps` durable (WP03b review).
    #[test]
    fn retried_open_after_a_partial_create_dir_all_syncs_the_ancestors() {
        let dir = Path::new("/data/apps/db");
        for seed in 0..32 {
            let disk = SimVfs::new(seed);
            disk.set_dir_durability(DirDurability::PerDir);
            let inner = disk.as_vfs();
            let failed = Arc::new(Mutex::new(false));
            let vfs = HookVfs::wrap(disk.as_vfs(), move |op| {
                if op.kind == HookOpKind::CreateDirAll && op.point == HookPoint::Before && op.path == dir {
                    let mut failed = failed.lock();
                    if !*failed {
                        *failed = true;
                        inner.create_dir_all(Path::new("/data/apps")).unwrap();
                        return HookAction::Fail(io::ErrorKind::StorageFull);
                    }
                }
                HookAction::Proceed
            });
            let first = KernoDb::open(vfs.clone(), dir, &Config::default());
            assert!(matches!(first, Err(Error::Io(_))), "seed {seed}: {:?}", first.err());
            let db = KernoDb::open(vfs, dir, &Config::default()).unwrap();
            commit_one(&db);
            let image = disk.crash_now(CrashKind::PowerLoss);
            drop(db);
            check_one_commit(image.as_vfs(), dir, &format!("seed {seed}"));
        }
    }

    /// An ancestor whose fsync fails with an I/O error (not a permission
    /// error) fails every open that needs it, the retried one included: the
    /// failure must not be swallowed by the best-effort walk (WP03b review).
    #[test]
    fn retried_open_does_not_swallow_an_io_error_syncing_an_ancestor() {
        let dir = Path::new("/data/apps/db");
        let disk = SimVfs::new(6);
        disk.set_dir_durability(DirDurability::PerDir);
        let vfs = HookVfs::wrap(disk.as_vfs(), |op| {
            if op.kind == HookOpKind::SyncDir && op.point == HookPoint::Before && op.path == Path::new("/data") {
                HookAction::Fail(io::ErrorKind::Other)
            } else {
                HookAction::Proceed
            }
        });
        for attempt in 0..3 {
            match KernoDb::open(vfs.clone(), dir, &Config::default()) {
                Err(Error::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::Other, "attempt {attempt}"),
                other => panic!("attempt {attempt}: the open must fail, got {:?}", other.err()),
            }
        }
    }

    /// `inner` with `sync_dir` failing as it does for a directory this
    /// process may traverse but not read (`unreadable`); records every
    /// `sync_dir` attempted outside the database directory `db_dir`.
    fn traverse_only(
        inner: Arc<dyn Vfs>,
        db_dir: &'static str,
        unreadable: &'static [&'static str],
    ) -> (Arc<dyn Vfs>, Arc<Mutex<Vec<PathBuf>>>) {
        let synced = Arc::new(Mutex::new(Vec::new()));
        let log = synced.clone();
        let vfs = HookVfs::wrap(inner, move |op| {
            if op.kind != HookOpKind::SyncDir || op.point != HookPoint::Before || op.path.starts_with(db_dir) {
                return HookAction::Proceed;
            }
            log.lock().push(op.path.to_path_buf());
            if unreadable.iter().any(|d| op.path == Path::new(d)) {
                HookAction::Fail(io::ErrorKind::PermissionDenied)
            } else {
                HookAction::Proceed
            }
        });
        (vfs, synced)
    }

    #[test]
    fn traverse_only_ancestors_do_not_prevent_creating_a_database() {
        const DIR: &str = "/home/shared/db";
        // `/home` and `/` may only be traversed; `/home/shared` is readable.
        for (pre_existing, nested, expected) in [
            // A new directory: the directories that hold new entries, then
            // best effort upwards, stopping at the first this process may
            // not read.
            (false, false, &["/home/shared", "/home"][..]),
            (false, true, &["/home/shared/db", "/home/shared", "/home"][..]),
            // An existing one (maybe left by a failed open): the parent, then
            // best effort upwards, stopping at the first that fails.
            (true, false, &["/home/shared", "/home"][..]),
        ] {
            let disk = SimVfs::new(3);
            disk.create_dir_all(Path::new("/home/shared")).unwrap();
            if pre_existing {
                disk.create_dir_all(Path::new(DIR)).unwrap();
            }
            let db_dir = if nested { "/home/shared/db/nested" } else { DIR };
            let (vfs, synced) = traverse_only(disk.as_vfs(), db_dir, &["/home", "/"]);
            let label = format!("pre-existing {pre_existing}, {db_dir}");
            let db = KernoDb::open(vfs.clone(), Path::new(db_dir), &Config::default())
                .unwrap_or_else(|e| panic!("{label}: creating the database failed: {e}"));
            commit_one(&db);
            db.close().unwrap();
            let synced = synced.lock().clone();
            assert_eq!(synced, expected.iter().map(PathBuf::from).collect::<Vec<_>>(), "{label}");
            check_one_commit(vfs, Path::new(db_dir), &label);
        }
    }

    /// The directory that holds the database directory is still synced
    /// strictly: if that fails, so does the open.
    #[test]
    fn unsyncable_parent_fails_the_open() {
        for pre_existing in [false, true] {
            let disk = SimVfs::new(4);
            disk.create_dir_all(Path::new("/home/shared")).unwrap();
            if pre_existing {
                disk.create_dir_all(Path::new("/home/shared/db")).unwrap();
            }
            let (vfs, _) = traverse_only(disk.as_vfs(), "/home/shared/db", &["/home/shared"]);
            match KernoDb::open(vfs, Path::new("/home/shared/db"), &Config::default()) {
                Err(Error::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::PermissionDenied),
                other => panic!("pre-existing {pre_existing}: {:?}", other.err()),
            }
        }
    }

    /// The same on the real file system, under an ancestor in mode 0111.
    /// Skipped where permission bits are not enforced (a process with
    /// CAP_DAC_OVERRIDE or CAP_DAC_READ_SEARCH, such as root): run it there
    /// with `setpriv --bounding-set=-dac_override,-dac_read_search`.
    #[test]
    fn execute_only_ancestor_on_the_real_file_system() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let base =
            Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("kerno-execute-only-ancestor-{}", std::process::id()));
        let outer = base.join("outer");
        let inner = outer.join("inner");
        fs::create_dir_all(inner.join("pre-existing")).unwrap();
        fs::set_permissions(&outer, fs::Permissions::from_mode(0o111)).unwrap();
        let enforced = fs::File::open(&outer).is_err();
        let result = std::panic::catch_unwind(|| {
            if !enforced {
                eprintln!("skipped: permission bits are not enforced for this process");
                return;
            }
            for name in ["db", "pre-existing", "nested/db"] {
                let dir = inner.join(name);
                let db = KernoDb::open(StdVfs::shared(), &dir, &Config::default())
                    .unwrap_or_else(|e| panic!("{name}: creating the database failed: {e}"));
                commit_one(&db);
                db.close().unwrap();
                check_one_commit(StdVfs::shared(), &dir, name);
            }
        });
        fs::set_permissions(&outer, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&base).unwrap();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}
