//! Test profiles: the harness suites run against [`KernoDb`] with a
//! different [`Config`].
//!
//! The harness contract opens engines with [`Options`], which only carries
//! durability, cache and write buffer. A [`Profile`] adjusts the rest of the
//! [`Config`], and [`Profiled<P>`] is a distinct engine type, so the shared
//! macros (`kerno_testkit::engine_test_suite!` and the like) can be
//! instantiated once per profile without touching `Options`
//! (phase1-design.md §5.1). Not a public API.
//!
//! Profiles:
//!
//! * [`Inline`] (`kerno-inline`): flushes and compactions run in the
//!   committing thread instead of background threads
//!   ([`Background::Inline`]), so the sequence of file-system operations is
//!   deterministic.

use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Engine, Options, TestHooks};
use kerno_core::vfs::Vfs;
use kerno_core::{Result, WriteBatch};

use crate::{Background, Config, KernoDb, KernoSnapshot};

/// A named adjustment of the [`Config`] derived from the harness
/// [`Options`].
pub trait Profile: Send + Sync + 'static {
    /// What [`Engine::name`] of [`Profiled<Self>`] returns (`kerno-…`).
    const NAME: &'static str;

    /// Changes `cfg`, which starts as `Config::from(opts)`.
    fn adjust(cfg: &mut Config);
}

/// [`KernoDb`] opened with the configuration of profile `P`.
pub struct Profiled<P>(KernoDb, PhantomData<P>);

impl<P> Profiled<P> {
    /// The underlying handle.
    pub fn db(&self) -> &KernoDb {
        &self.0
    }

    /// Test-only control over flushes and compactions (see [`TestHooks`]).
    pub fn hooks(&self) -> &dyn TestHooks {
        self.0.hooks()
    }
}

/// Flushes and compactions run in the calling thread, without background
/// threads ([`Background::Inline`]).
pub struct Inline;

impl Profile for Inline {
    const NAME: &'static str = "kerno-inline";

    fn adjust(cfg: &mut Config) {
        cfg.test.background = Background::Inline;
    }
}

impl<P: Profile> Engine for Profiled<P> {
    type Snapshot = KernoSnapshot;

    fn name() -> &'static str {
        P::NAME
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        let mut cfg = Config::from(opts);
        P::adjust(&mut cfg);
        KernoDb::open(vfs, dir, &cfg).map(|db| Profiled(db, PhantomData))
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        self.0.commit(batch)
    }

    fn snapshot(&self) -> Result<KernoSnapshot> {
        self.0.snapshot()
    }

    fn close(self) -> Result<()> {
        self.0.close()
    }

    fn stats(&self) -> Vec<(String, u64)> {
        self.0.stats()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::AccessMode;
    use kerno_core::Durability;
    use kerno_core::contract::Snapshot;
    use kerno_testkit::vfs::SimVfs;

    /// The configuration `Probe::adjust` received.
    static SEEN: Mutex<Option<Config>> = Mutex::new(None);

    /// Records its input and switches to `Buffered`, which is observable
    /// through `stats()`: a `Buffered` commit does not sync the WAL.
    struct Probe;

    impl Profile for Probe {
        const NAME: &'static str = "kerno-test-probe";

        fn adjust(cfg: &mut Config) {
            *SEEN.lock().unwrap() = Some(cfg.clone());
            cfg.durability = Durability::Buffered;
        }
    }

    fn stat(db: &impl Engine, name: &str) -> u64 {
        db.stats().into_iter().find(|(k, _)| k == name).map(|(_, v)| v).unwrap()
    }

    #[test]
    fn profiled_passes_options_through() {
        assert_eq!(Profiled::<Probe>::name(), "kerno-test-probe");
        let opts = Options { durability: Durability::Sync, cache_bytes: 96 << 10, write_buffer_bytes: 24 << 10 };
        let db = Profiled::<Probe>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &opts).unwrap();

        // `adjust` starts from `Config::from(opts)`.
        let seen = SEEN.lock().unwrap().take().expect("Profile::adjust was not called");
        assert_eq!(seen.durability, Durability::Sync);
        assert_eq!(seen.cache_bytes, 96 << 10);
        assert_eq!(seen.write_buffer_bytes, 24 << 10);
        assert_eq!(seen.access, AccessMode::ReadWrite);
        assert!(seen.create_if_missing);
        assert_eq!(seen.max_immutable_memtables, Config::default().max_immutable_memtables);

        // The engine runs with the adjusted configuration.
        let mut b = WriteBatch::new();
        b.put("k", "v");
        assert_eq!(db.commit(&b).unwrap(), 1);
        assert_eq!(stat(&db, "commits"), 1);
        assert_eq!(stat(&db, "wal_syncs"), 0, "the profile's Buffered durability was not applied");
        let snap = db.snapshot().unwrap();
        assert_eq!(snap.seq(), 1);
        assert_eq!(snap.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        drop(snap);
        db.close().unwrap();
    }

    #[test]
    fn inline_profile_runs_without_threads() {
        assert_eq!(Profiled::<Inline>::name(), "kerno-inline");
        let sim = SimVfs::new(1);
        sim.start_trace();
        let db = Profiled::<Inline>::open(sim.as_vfs(), Path::new("/db"), &Options::small_for_tests()).unwrap();
        for i in 0..200u32 {
            let mut b = WriteBatch::new();
            b.put(i.to_be_bytes(), [7u8; 300]);
            db.commit(&b).unwrap();
        }
        assert!(stat(&db, "flushes") > 0, "200 commits of 300 bytes must fill a 16 KiB memtable");
        assert_eq!(stat(&db, "immutable_memtables"), 0, "inline flushes finish inside the commit");
        db.hooks().wait_idle().unwrap();
        db.close().unwrap();
        // Every file-system operation ran on this thread.
        let me = std::thread::current().name().unwrap().to_string();
        let trace = sim.trace();
        assert!(!trace.is_empty());
        let other = trace.iter().find(|e| e.thread != me);
        assert!(other.is_none(), "operation from another thread: {other:?}");
    }
}
