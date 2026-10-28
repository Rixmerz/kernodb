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

use crate::{Background, Config, KernoDb, KernoSnapshot, SlowdownPolicy};

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

/// The environment variable read by [`Tuned`].
pub const TUNING_ENV: &str = "KERNO_TUNING";

/// Profile for A/B experiments with `spike-bench` (engine `kerno-tuned`,
/// phase1-design.md §5.4): the configuration from the harness options,
/// changed by the environment variable [`TUNING_ENV`] as parsed by
/// [`apply_tuning`]. Never used by the test suites. An invalid value panics
/// with the reason.
pub struct Tuned;

impl Profile for Tuned {
    const NAME: &'static str = "kerno-tuned";

    fn adjust(cfg: &mut Config) {
        if let Ok(spec) = std::env::var(TUNING_ENV)
            && let Err(e) = apply_tuning(&spec, cfg)
        {
            panic!("{TUNING_ENV}={spec:?}: {e}");
        }
    }
}

/// Applies `spec`, a comma-separated list of `field=value`, to `cfg`, for
/// example `slowdown=off,bytes_per_sync=1048576,l0_slowdown=12`. Fields:
///
/// * of [`CompactionConfig`](crate::CompactionConfig): `l0_trigger`,
///   `l0_slowdown`, `l0_stop`, `level_multiplier`, `level_base_bytes`,
///   `target_file_size` (integers), `slowdown` (`off`, or `sleep:<micros>`),
///   `bytes_per_sync` (bytes, or `none`);
/// * of [`Config`]: `max_immutable_memtables`.
///
/// `KernoDb::open` validates the result as usual.
pub fn apply_tuning(spec: &str, cfg: &mut Config) -> std::result::Result<(), String> {
    for item in spec.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        let (key, value) = item.split_once('=').ok_or_else(|| format!("`{item}` is not field=value"))?;
        let (key, value) = (key.trim(), value.trim());
        let int = || value.parse::<u64>().map_err(|_| format!("{key}: `{value}` is not an integer"));
        let c = &mut cfg.compaction;
        match key {
            "l0_trigger" => c.l0_trigger = int()? as usize,
            "l0_slowdown" => c.l0_slowdown = int()? as usize,
            "l0_stop" => c.l0_stop = int()? as usize,
            "level_multiplier" => c.level_multiplier = int()?,
            "level_base_bytes" => c.level_base_bytes = Some(int()?),
            "target_file_size" => c.target_file_size = Some(int()?),
            "max_immutable_memtables" => cfg.max_immutable_memtables = int()? as usize,
            "bytes_per_sync" => c.bytes_per_sync = if value == "none" { None } else { Some(int()?) },
            "slowdown" => {
                c.slowdown = if value == "off" {
                    SlowdownPolicy::Off
                } else if let Some(us) = value.strip_prefix("sleep:") {
                    let micros = us.parse().map_err(|_| format!("slowdown: `{us}` is not an integer"))?;
                    SlowdownPolicy::Sleep { micros }
                } else {
                    return Err(format!("slowdown: `{value}` is neither off nor sleep:<micros>"));
                }
            }
            other => return Err(format!("unknown field `{other}`")),
        }
    }
    Ok(())
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

    #[test]
    fn tuning_spec_changes_the_named_fields_only() {
        let base = Config::default();
        let mut cfg = base.clone();
        apply_tuning("", &mut cfg).unwrap();
        assert_eq!(cfg.compaction, base.compaction);

        apply_tuning(" slowdown=off, bytes_per_sync=1048576 ,l0_slowdown=12", &mut cfg).unwrap();
        assert_eq!(cfg.compaction.slowdown, SlowdownPolicy::Off);
        assert_eq!(cfg.compaction.bytes_per_sync, Some(1 << 20));
        assert_eq!(cfg.compaction.l0_slowdown, 12);
        assert_eq!(cfg.compaction.l0_stop, base.compaction.l0_stop);

        let spec = "slowdown=sleep:250,bytes_per_sync=none,l0_trigger=2,l0_stop=20,level_multiplier=8,\
                    level_base_bytes=1000,target_file_size=500,max_immutable_memtables=6";
        apply_tuning(spec, &mut cfg).unwrap();
        let c = &cfg.compaction;
        assert_eq!(c.slowdown, SlowdownPolicy::Sleep { micros: 250 });
        assert_eq!(c.bytes_per_sync, None);
        assert_eq!((c.l0_trigger, c.l0_stop, c.level_multiplier), (2, 20, 8));
        assert_eq!((c.level_base_bytes, c.target_file_size), (Some(1000), Some(500)));
        assert_eq!(cfg.max_immutable_memtables, 6);

        for bad in ["l0_stop", "l0_stop=x", "nope=1", "slowdown=fast", "slowdown=sleep:x"] {
            assert!(apply_tuning(bad, &mut cfg).is_err(), "{bad} was accepted");
        }
    }

    #[test]
    fn tuned_profile_without_the_variable_keeps_the_configuration() {
        assert_eq!(Profiled::<Tuned>::name(), "kerno-tuned");
        if std::env::var(TUNING_ENV).is_ok() {
            return; // set by hand for an experiment
        }
        let mut cfg = Config::default();
        Tuned::adjust(&mut cfg);
        assert_eq!(cfg.compaction, Config::default().compaction);
    }
}
