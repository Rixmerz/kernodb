//! Configuration of a [`KernoDb`](crate::KernoDb) handle.
//!
//! Phase 1 API, explicitly unstable (phase1-design.md §3). The fields are the
//! ones the M0 engine already has; later work packages add the rest. The
//! default configuration reproduces the M0 engine exactly.

use kerno_core::contract::Options;
use kerno_core::{Durability, Error, Result};

use crate::compaction::MAX_IMMUTABLE;

/// Whether a handle may write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessMode {
    /// Reads and writes.
    #[default]
    ReadWrite,
    /// Reads only. Not implemented yet: [`KernoDb::open`](crate::KernoDb::open)
    /// rejects it with [`Error::InvalidArgument`].
    ReadOnly,
}

/// Options of [`KernoDb::open`](crate::KernoDb::open).
#[derive(Clone, Debug)]
pub struct Config {
    /// Read-write (default) or read-only access.
    pub access: AccessMode,
    /// Create the database if `dir` holds none (default `true`). When
    /// `false` and `dir` has no `MANIFEST`, `open` creates and changes
    /// nothing and returns [`Error::Corruption`] if `dir` holds WAL or SST
    /// files (a damaged database, as with `true`), or
    /// [`Error::InvalidArgument`] otherwise (no database).
    pub create_if_missing: bool,
    /// Durability of every commit of this handle (default
    /// [`Durability::Sync`]).
    pub durability: Durability,
    /// Capacity of the block cache in bytes (default 64 MiB).
    pub cache_bytes: usize,
    /// Size of the active memtable that triggers a switch to a new one and a
    /// flush of the old one to L0 (default 8 MiB; at least 4 KiB is used).
    /// Also the target size of SST files.
    pub write_buffer_bytes: usize,
    /// Writers wait while this many memtables are waiting to be flushed
    /// (default 4, the M0 value; at least 1).
    pub max_immutable_memtables: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            access: AccessMode::ReadWrite,
            create_if_missing: true,
            durability: Durability::Sync,
            cache_bytes: 64 << 20,
            write_buffer_bytes: 8 << 20,
            max_immutable_memtables: MAX_IMMUTABLE,
        }
    }
}

impl From<&Options> for Config {
    /// Durability, cache and write buffer come from `opts`; every other field
    /// keeps its default.
    fn from(opts: &Options) -> Self {
        Config {
            durability: opts.durability,
            cache_bytes: opts.cache_bytes,
            write_buffer_bytes: opts.write_buffer_bytes,
            ..Config::default()
        }
    }
}

impl Config {
    /// Rejects values the engine cannot run with.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.max_immutable_memtables == 0 {
            return Err(Error::InvalidArgument("max_immutable_memtables must be at least 1".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::io;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::KernoDb;
    use kerno_core::WriteBatch;
    use kerno_core::contract::{Engine, Snapshot};
    use kerno_core::vfs::{File, OpenMode, Vfs};
    use kerno_testkit::vfs::SimVfs;
    use parking_lot::{Condvar, Mutex};

    #[test]
    fn default_config_matches_m0_options() {
        let c = Config::default();
        let o = Options::default();
        assert_eq!(c.access, AccessMode::ReadWrite);
        assert!(c.create_if_missing);
        assert_eq!(c.durability, o.durability);
        assert_eq!(c.cache_bytes, o.cache_bytes);
        assert_eq!(c.write_buffer_bytes, o.write_buffer_bytes);
        assert_eq!(c.max_immutable_memtables, 4);
    }

    #[test]
    fn config_from_options_takes_engine_fields() {
        let o = Options { durability: Durability::Buffered, cache_bytes: 123, write_buffer_bytes: 4567 };
        let c = Config::from(&o);
        assert_eq!(c.durability, Durability::Buffered);
        assert_eq!(c.cache_bytes, 123);
        assert_eq!(c.write_buffer_bytes, 4567);
        assert_eq!(c.access, AccessMode::ReadWrite);
        assert!(c.create_if_missing);
        assert_eq!(c.max_immutable_memtables, 4);
    }

    #[test]
    fn read_only_access_is_rejected_until_implemented() {
        let sim = SimVfs::new(1);
        let cfg = Config { access: AccessMode::ReadOnly, ..Config::default() };
        match KernoDb::open(sim.as_vfs(), Path::new("/db"), &cfg) {
            Err(Error::InvalidArgument(m)) => assert_eq!(m, "read-only mode not implemented yet"),
            other => panic!("expected InvalidArgument, got {:?}", other.err()),
        }
        assert_eq!(sim.mutating_ops(), 0);
        assert!(!sim.as_vfs().exists(Path::new("/db")).unwrap());
    }

    #[test]
    fn zero_immutable_memtables_is_rejected() {
        let sim = SimVfs::new(1);
        let cfg = Config { max_immutable_memtables: 0, ..Config::default() };
        assert!(matches!(KernoDb::open(sim.as_vfs(), Path::new("/db"), &cfg), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), 0);
    }

    #[test]
    fn create_if_missing_false_touches_nothing() {
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        let cfg = Config { create_if_missing: false, ..Config::default() };
        // Missing directory.
        assert!(matches!(KernoDb::open(vfs.clone(), Path::new("/db"), &cfg), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), 0);
        assert!(!vfs.exists(Path::new("/db")).unwrap());
        // Existing directory without a database: still untouched.
        vfs.create_dir_all(Path::new("/db")).unwrap();
        let before = sim.mutating_ops();
        assert!(matches!(KernoDb::open(vfs.clone(), Path::new("/db"), &cfg), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), before);
        assert!(vfs.list(Path::new("/db")).unwrap().is_empty());
    }

    #[test]
    fn create_if_missing_false_opens_an_existing_database() {
        let vfs = SimVfs::new(1).as_vfs();
        let dir = Path::new("/db");
        let db = KernoDb::open(vfs.clone(), dir, &Config::default()).unwrap();
        let mut b = WriteBatch::new();
        b.put("k", "v");
        db.commit(&b).unwrap();
        db.close().unwrap();
        let cfg = Config { create_if_missing: false, ..Config::default() };
        let db = KernoDb::open(vfs, dir, &cfg).unwrap();
        assert_eq!(db.snapshot().unwrap().get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        db.close().unwrap();
    }

    #[test]
    fn create_if_missing_false_reports_damage_as_corruption() {
        let dir = Path::new("/db");
        let no_create = Config { create_if_missing: false, ..Config::default() };
        // WAL or SST files without a MANIFEST are a damaged database, not a
        // missing one: the error must not depend on create_if_missing, and
        // the directory stays untouched.
        let check = |sim: &SimVfs| {
            let before = sim.mutating_ops();
            let msg = match KernoDb::open(sim.as_vfs(), dir, &no_create) {
                Err(Error::Corruption(m)) => m,
                other => panic!("expected Corruption, got {:?}", other.err()),
            };
            assert_eq!(sim.mutating_ops(), before);
            match KernoDb::open(sim.as_vfs(), dir, &Config::default()) {
                Err(Error::Corruption(m)) => assert_eq!(m, msg),
                other => panic!("expected Corruption, got {:?}", other.err()),
            }
        };

        // A cleanly closed database (LOCK, MANIFEST, SSTs) without its MANIFEST.
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        let db = KernoDb::open(vfs.clone(), dir, &Config::default()).unwrap();
        let mut b = WriteBatch::new();
        b.put("k", "v");
        db.commit(&b).unwrap();
        db.close().unwrap();
        assert!(vfs.list(dir).unwrap().iter().any(|n| n.ends_with(".sst")));
        vfs.remove_file(&dir.join("MANIFEST")).unwrap();
        vfs.sync_dir(dir).unwrap();
        check(&sim);

        // Nothing but a canonical WAL.
        let sim = SimVfs::new(2);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(dir).unwrap();
        vfs.open(&dir.join("000005.wal"), OpenMode::CreateNew).unwrap().write_at(b"junk", 0).unwrap();
        check(&sim);

        // Names that are not canonical WAL or SST names are not data files.
        let sim = SimVfs::new(3);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(dir).unwrap();
        for name in ["1.wal", "5.sst", "notes.txt"] {
            vfs.open(&dir.join(name), OpenMode::CreateNew).unwrap();
        }
        let before = sim.mutating_ops();
        assert!(matches!(KernoDb::open(vfs, dir, &no_create), Err(Error::InvalidArgument(_))));
        assert_eq!(sim.mutating_ops(), before);
    }

    /// Holds every SST creation until [`SstGate::release`], so that no flush
    /// can finish.
    struct SstGate {
        inner: Arc<dyn Vfs>,
        state: Mutex<GateState>,
        cv: Condvar,
    }

    #[derive(Default)]
    struct GateState {
        open: bool,
        held: usize,
    }

    impl SstGate {
        fn held(&self) -> usize {
            self.state.lock().held
        }

        fn release(&self) {
            self.state.lock().open = true;
            self.cv.notify_all();
        }
    }

    impl Vfs for SstGate {
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.inner.create_dir_all(path)
        }
        fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
            if mode == OpenMode::CreateNew && path.extension().is_some_and(|e| e == "sst") {
                let mut s = self.state.lock();
                s.held += 1;
                while !s.open {
                    self.cv.wait(&mut s);
                }
            }
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
            self.inner.sync_dir(dir)
        }
        fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
            self.inner.lock(path)
        }
    }

    /// Opens the gate when dropped, also when an assertion fails, so that the
    /// threads blocked behind it finish and the test does not hang.
    struct ReleaseOnDrop<'a>(&'a SstGate);

    impl Drop for ReleaseOnDrop<'_> {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    fn stat(db: &KernoDb, name: &str) -> u64 {
        db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap()
    }

    #[test]
    fn max_immutable_memtables_bounds_pending_flushes() {
        let gate = Arc::new(SstGate { inner: SimVfs::new(1).as_vfs(), state: Mutex::default(), cv: Condvar::new() });
        let cfg = Config { max_immutable_memtables: 1, write_buffer_bytes: 4 << 10, ..Config::default() };
        let db = KernoDb::open(gate.clone(), Path::new("/db"), &cfg).unwrap();
        // Every commit fills a memtable. Commit 1 fills the first one.
        // Commit 2 switches to a second one (the first waits for its flush,
        // held at the gate) and fills it. Commit 3 then finds one memtable
        // pending, the limit, and must wait for that flush; with the M0
        // limit of 4 it would switch again and return.
        let value = vec![7u8; 5 << 10];
        let done = AtomicUsize::new(0);
        std::thread::scope(|s| {
            let _release = ReleaseOnDrop(&gate);
            s.spawn(|| {
                for i in 0..3u8 {
                    let mut b = WriteBatch::new();
                    b.put([i], value.as_slice());
                    db.commit(&b).unwrap();
                    done.fetch_add(1, Ordering::SeqCst);
                }
            });
            let deadline = Instant::now() + Duration::from_secs(60);
            loop {
                let n = done.load(Ordering::SeqCst);
                assert!(n <= 2, "commit {n} returned while a flush was held and a memtable was pending");
                assert!(stat(&db, "immutable_memtables") <= 1);
                if n == 2 && gate.held() == 1 && stat(&db, "write_stalls") > 0 {
                    break;
                }
                // A stall is counted when the writer wakes up: wake it so that
                // it counts the stall and waits again.
                db.wake_waiters();
                assert!(Instant::now() < deadline, "the writer never stalled");
                std::thread::sleep(Duration::from_millis(1));
            }
            // Dropping `_release` lets the flush finish and commit 3 return.
        });
        assert_eq!(done.load(Ordering::SeqCst), 3);
        let snap = db.snapshot().unwrap();
        for i in 0..3u8 {
            assert_eq!(snap.get(&[i]).unwrap().as_deref(), Some(value.as_slice()));
        }
        drop(snap);
        db.close().unwrap();
    }
}
