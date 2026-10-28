//! Test profiles: the harness suites run against [`KernoDb`] with a
//! different [`Config`].
//!
//! The harness contract opens engines with [`Options`], which only carries
//! durability, cache and write buffer. A [`Profile`] adjusts the rest of the
//! [`Config`], and [`Profiled<P>`] is a distinct engine type, so the shared
//! macros (`kerno_testkit::engine_test_suite!` and the like) can be
//! instantiated once per profile without touching `Options`
//! (phase1-design.md §5.1). Not a public API.

use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::{Engine, Options};
use kerno_core::vfs::Vfs;
use kerno_core::{Result, WriteBatch};

use crate::{Config, KernoDb, KernoSnapshot};

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
}
