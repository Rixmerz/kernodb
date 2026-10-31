//! Typed metrics and their flattened `stats()` names (phase1-design.md §3,
//! WP04): stall episodes, commit-phase histograms, flush accounting,
//! pending compaction bytes, the M0 names and the sharded read counters.

use std::any::Any;
use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options, Snapshot};
use kerno_core::vfs::{File, OpenMode, Vfs};
use kerno_core::{Durability, WriteBatch};
use kerno_storage::metrics::{READ_COUNTER_SHARDS, ReadMetrics};
use kerno_storage::{CompactionConfig, Config, KernoDb, Metrics, SlowdownPolicy};
use kerno_testkit::vfs::SimVfs;
use parking_lot::{Condvar, Mutex};

const DIR: &str = "/db";

fn open(vfs: &Arc<dyn Vfs>, cfg: &Config) -> KernoDb {
    KernoDb::open(vfs.clone(), Path::new(DIR), cfg).unwrap()
}

fn stats(db: &KernoDb) -> HashMap<String, u64> {
    let list = db.stats();
    let map: HashMap<String, u64> = list.iter().cloned().collect();
    assert_eq!(map.len(), list.len(), "duplicate stat names");
    map
}

fn stat(db: &KernoDb, name: &str) -> u64 {
    *stats(db).get(name).unwrap_or_else(|| panic!("no stat {name}"))
}

fn put(db: &KernoDb, key: &[u8], value: &[u8]) {
    let mut b = WriteBatch::new();
    b.put(key, value);
    db.commit(&b).unwrap();
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(1));
    }
}

// ---------------------------------------------------------------- gates

#[derive(Default)]
struct GateState {
    /// Thread-name prefixes whose file creations are held.
    held_threads: Vec<&'static str>,
    /// Creations allowed through per prefix.
    permits: HashMap<&'static str, usize>,
    /// Threads waiting per prefix.
    waiting: HashMap<&'static str, usize>,
    open: bool,
    /// Hold every file creation, not only SSTs.
    all_files: bool,
}

/// Holds the SST creations (or, with `all_files`, every `CreateNew`: SSTs
/// and `MANIFEST.tmp`) made by the engine threads named with a held prefix,
/// until permits are granted or the gate opens. The writer and the test
/// thread are never held.
struct GatedVfs {
    inner: Arc<dyn Vfs>,
    state: Mutex<GateState>,
    cv: Condvar,
}

impl GatedVfs {
    fn new(inner: Arc<dyn Vfs>, held_threads: &[&'static str]) -> Arc<Self> {
        let state = GateState { held_threads: held_threads.to_vec(), ..GateState::default() };
        Arc::new(GatedVfs { inner, state: Mutex::new(state), cv: Condvar::new() })
    }

    fn holding_all_files(inner: Arc<dyn Vfs>, held_threads: &[&'static str]) -> Arc<Self> {
        let gate = GatedVfs::new(inner, held_threads);
        gate.state.lock().all_files = true;
        gate
    }

    fn waiting(&self, prefix: &str) -> usize {
        self.state.lock().waiting.get(prefix).copied().unwrap_or(0)
    }

    fn allow(&self, prefix: &'static str, n: usize) {
        *self.state.lock().permits.entry(prefix).or_default() += n;
        self.cv.notify_all();
    }

    fn open_all(&self) {
        self.state.lock().open = true;
        self.cv.notify_all();
    }
}

/// Opens the gate when dropped, also when an assertion fails, so that the
/// engine threads behind it finish and the test does not hang. It must drop
/// before anything that joins those threads: declare it after the `KernoDb`
/// and as the first statement of every `std::thread::scope` closure.
struct OpenOnDrop(Arc<GatedVfs>);

impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open_all();
    }
}

impl Vfs for GatedVfs {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.inner.create_dir_all(path)
    }
    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
        if mode == OpenMode::CreateNew {
            let name = std::thread::current().name().unwrap_or("").to_string();
            let sst = path.extension().is_some_and(|e| e == "sst");
            let mut s = self.state.lock();
            let held = s.held_threads.iter().copied().find(|p| name.starts_with(p));
            if let Some(prefix) = held.filter(|_| sst || s.all_files) {
                *s.waiting.entry(prefix).or_default() += 1;
                loop {
                    if s.open {
                        break;
                    }
                    let permits = s.permits.entry(prefix).or_default();
                    if *permits > 0 {
                        *permits -= 1;
                        break;
                    }
                    self.cv.wait(&mut s);
                }
                *s.waiting.get_mut(prefix).unwrap() -= 1;
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

const FLUSH: &str = "kerno-flush";
const COMPACT: &str = "kerno-compact";

// ---------------------------------------------------------------- tests

#[test]
fn stall_episodes_counted_once_per_reason() {
    // Every 5 KiB value fills a 4 KiB memtable, so the next commit needs a
    // new one.
    let value = vec![7u8; 5 << 10];

    // L0 reason: L0 stops writers at its first file, and that file's
    // compaction is held. Flushes go on while the writer waits: each one
    // wakes it up, and it must stay one episode.
    let gate = GatedVfs::new(SimVfs::new(1).as_vfs(), &[FLUSH, COMPACT]);
    let cfg = Config {
        write_buffer_bytes: 4 << 10,
        compaction: CompactionConfig {
            l0_trigger: 1,
            l0_stop: 1,
            slowdown: SlowdownPolicy::Off,
            ..CompactionConfig::m0(4 << 10)
        },
        ..Config::default()
    };
    let vfs: Arc<dyn Vfs> = gate.clone();
    let db = open(&vfs, &cfg);
    // Declared after `db`, so that it opens the gate before `db` joins the
    // background threads on unwind.
    let _open = OpenOnDrop(gate.clone());
    // Memtables A, B, C, D; A is held in the flush, B and C wait.
    for i in 0..4u8 {
        put(&db, &[i], &value);
    }
    wait_until("three immutable memtables", || stat(&db, "immutable_memtables") == 3 && gate.waiting(FLUSH) == 1);
    assert_eq!(stat(&db, "write_stalls"), 0);
    // A reaches L0 (= l0_stop); its compaction is held.
    gate.allow(FLUSH, 1);
    wait_until("A in L0 and its compaction held", || {
        stat(&db, "l0_files") == 1 && gate.waiting(COMPACT) == 1 && gate.waiting(FLUSH) == 1
    });
    std::thread::scope(|s| {
        // Opens the gate before the scope joins the writer on unwind.
        let _open = OpenOnDrop(gate.clone());
        // D is full and L0 is at its stop: the writer waits.
        let writer = s.spawn(|| put(&db, &[4], &value));
        wait_until("an L0 stall", || stat(&db, "stall_episodes_l0") == 1);
        // B and C reach L0: two installs, two wake-ups of the writer.
        gate.allow(FLUSH, 2);
        wait_until("B and C in L0", || stat(&db, "flushes") == 3 && stat(&db, "l0_files") == 3);
        std::thread::sleep(Duration::from_millis(20));
        assert!(!writer.is_finished(), "the writer went past the L0 stop");
        let st = stats(&db);
        assert_eq!(st["stall_episodes_l0"], 1, "wake-ups counted as episodes");
        assert_eq!(st["stall_episodes_memtable"], 0);
        assert_eq!(st["write_stalls"], 1);
        // Compactions drain L0 and the writer goes on.
        gate.open_all();
        writer.join().unwrap();
    });
    let m = db.metrics();
    assert_eq!((m.stalls.episodes_l0, m.stalls.episodes_memtable), (1, 0));
    assert!(m.stalls.micros_l0 >= 20_000, "{:?}", m.stalls);
    assert_eq!(m.stalls.micros_memtable, 0);
    assert!(m.stalls.micros_max >= m.stalls.micros_l0, "{:?}", m.stalls);
    assert_eq!(stat(&db, "write_stall_micros"), m.stalls.micros_l0);
    assert_eq!(m.stalls.slowdown_commits, 0, "slowdown is off");
    db.close().unwrap();

    // Memtable reason: one immutable memtable allowed, its flush held.
    let gate = GatedVfs::new(SimVfs::new(2).as_vfs(), &[FLUSH]);
    let cfg = Config { write_buffer_bytes: 4 << 10, max_immutable_memtables: 1, ..Config::default() };
    let vfs: Arc<dyn Vfs> = gate.clone();
    let db = open(&vfs, &cfg);
    let _open = OpenOnDrop(gate.clone());
    put(&db, b"a", &value);
    put(&db, b"b", &value);
    wait_until("the flush of the first memtable held", || gate.waiting(FLUSH) == 1);
    std::thread::scope(|s| {
        let _open = OpenOnDrop(gate.clone());
        let writer = s.spawn(|| put(&db, b"c", &value));
        wait_until("a memtable stall", || stat(&db, "stall_episodes_memtable") == 1);
        std::thread::sleep(Duration::from_millis(20));
        assert!(!writer.is_finished());
        gate.open_all();
        writer.join().unwrap();
    });
    let m = db.metrics();
    assert_eq!((m.stalls.episodes_memtable, m.stalls.episodes_l0), (1, 0));
    assert!(m.stalls.micros_memtable >= 20_000, "{:?}", m.stalls);
    assert_eq!(m.stalls.micros_l0, 0);
    assert_eq!(stat(&db, "write_stalls"), 1);
    db.close().unwrap();
}

/// A file whose `sync_data` takes `delay` longer.
struct SlowSyncFile {
    inner: Arc<dyn File>,
    delay: Duration,
}

impl File for SlowSyncFile {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.inner.read_at(buf, offset)
    }
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        self.inner.write_at(buf, offset)
    }
    fn len(&self) -> io::Result<u64> {
        self.inner.len()
    }
    fn set_len(&self, len: u64) -> io::Result<()> {
        self.inner.set_len(len)
    }
    fn sync_data(&self) -> io::Result<()> {
        std::thread::sleep(self.delay);
        self.inner.sync_data()
    }
}

/// WAL files get a slow `sync_data`.
struct SlowWalSync {
    inner: Arc<dyn Vfs>,
    delay: Duration,
}

impl Vfs for SlowWalSync {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.inner.create_dir_all(path)
    }
    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
        let f = self.inner.open(path, mode)?;
        if path.extension().is_some_and(|e| e == "wal") {
            return Ok(Arc::new(SlowSyncFile { inner: f, delay: self.delay }));
        }
        Ok(f)
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

#[test]
fn commit_phase_histograms_populated() {
    let vfs: Arc<dyn Vfs> = Arc::new(SlowWalSync { inner: SimVfs::new(3).as_vfs(), delay: Duration::from_millis(2) });
    let db = open(&vfs, &Config::default());
    for i in 0..50u32 {
        put(&db, &i.to_be_bytes(), b"value");
    }
    // An empty batch is not a commit.
    db.commit(&WriteBatch::new()).unwrap();
    let c = db.metrics().commit_phases;
    for (phase, l) in
        [("make_room", c.make_room), ("wal_write", c.wal_write), ("wal_sync", c.wal_sync), ("apply", c.apply)]
    {
        assert_eq!(l.count, 50, "{phase}");
        assert!(l.p50_micros <= l.p99_micros && l.p99_micros <= l.max_micros, "{phase}: {l:?}");
    }
    assert_eq!(c.total.count, 50);
    // One writer: every commit leads its own group, so it never waits for a
    // leader, only for the lock and the queue bookkeeping.
    assert_eq!(c.queue.count, 50);
    assert!(c.queue.max_micros < 1_000, "{:?}", c.queue);
    // Every sync took at least 2 ms; histogram percentiles are within 25 %
    // of the truth (the upper bound leaves room for a loaded machine).
    assert!((1_500..=4_200).contains(&c.wal_sync.p50_micros), "{:?}", c.wal_sync);
    assert!(c.wal_sync.p99_micros >= 1_500, "{:?}", c.wal_sync);
    assert!(c.wal_sync.max_micros >= 2_000, "{:?}", c.wal_sync);
    assert!(c.wal_sync.total_micros >= 50 * 2_000);
    assert_eq!(c.wal_sync_busy.count + c.wal_sync_quiet.count, 50);
    assert_eq!(c.wal_sync_busy.total_micros + c.wal_sync_quiet.total_micros, c.wal_sync.total_micros);
    // A commit lasts at least as long as its phases.
    let phases = c.make_room.total_micros + c.wal_write.total_micros + c.wal_sync.total_micros + c.apply.total_micros;
    assert!(c.total.total_micros >= phases, "total {} < phases {phases}", c.total.total_micros);
    assert!(c.total.p99_micros >= 2_000);

    let st = stats(&db);
    for phase in ["queue", "make_room", "wal_write", "wal_sync", "apply"] {
        assert!(st.contains_key(&format!("commit_{phase}_p99_micros")), "{phase}");
    }
    assert_eq!(st["commit_queue_count"], 50);
    assert_eq!(st["commit_queue_p99_micros"], c.queue.p99_micros);
    assert_eq!(st["commit_wal_sync_p99_micros"], c.wal_sync.p99_micros);
    assert_eq!(st["commit_wal_sync_count"], 50);
    db.close().unwrap();

    // Buffered commits do not sync the WAL.
    let vfs = SimVfs::new(4).as_vfs();
    let db = open(&vfs, &Config { durability: Durability::Buffered, ..Config::default() });
    for i in 0..10u32 {
        put(&db, &i.to_be_bytes(), b"value");
    }
    let c = db.metrics().commit_phases;
    assert_eq!((c.total.count, c.wal_write.count, c.wal_sync.count), (10, 10, 0));
    db.close().unwrap();
}

#[test]
fn close_and_recovery_flushes_counted() {
    let vfs = SimVfs::new(5).as_vfs();
    let cfg = Config::from(&Options::small_for_tests());

    // A new database: nothing replayed, nothing flushed yet.
    let db = open(&vfs, &cfg);
    let m = db.metrics();
    assert!(m.last_open.clean);
    assert_eq!((m.last_open.replayed_bytes, m.background.flushes), (0, 0));
    for i in 0..10u32 {
        put(&db, &i.to_be_bytes(), b"small");
    }
    assert_eq!(db.metrics().writes.memtable_rotations, 0);
    // A clean close flushes the one memtable.
    let m = db.close_with_metrics().unwrap();
    assert_eq!(m.background.flushes, 1);
    assert!(m.background.flush_bytes_written > 0);

    // A clean reopen replays and flushes nothing.
    let db = open(&vfs, &cfg);
    let st = stats(&db);
    assert_eq!((st["last_open_clean"], st["last_open_replayed_bytes"], st["flushes"]), (1, 0, 0));
    for i in 10..20u32 {
        put(&db, &i.to_be_bytes(), b"small");
    }
    let wal_bytes = stat(&db, "wal_bytes");
    assert!(wal_bytes > 0);
    drop(db); // unclean: the commits are only in the WAL

    // Recovery replays that WAL and flushes it to L0.
    let db = open(&vfs, &cfg);
    let m = db.metrics();
    assert!(!m.last_open.clean);
    assert_eq!(m.last_open.replayed_bytes, wal_bytes);
    assert_eq!(m.background.flushes, 1);
    assert!(m.background.flush_bytes_written > 0);
    let st = stats(&db);
    assert_eq!((st["last_open_clean"], st["last_open_replayed_bytes"]), (0, wal_bytes));

    // Every memtable that received commits is flushed once: by the flush
    // thread, or by the close.
    let recovered = m.background.flush_bytes_written;
    for i in 0..60u32 {
        put(&db, &i.to_be_bytes(), &[1u8; 1500]);
    }
    let rotations = db.metrics().writes.memtable_rotations;
    assert!(rotations >= 3, "{rotations} rotations");
    let m = db.close_with_metrics().unwrap();
    assert_eq!(m.background.flushes, 1 + rotations + 1);
    assert!(m.background.flush_bytes_written > recovered + 60 * 1500);
}

/// The stat names of the M0 engine (`spike-lsm`).
const M0_NAMES: &[&str] = &[
    "l0_files",
    "l0_bytes",
    "l1_files",
    "l1_bytes",
    "l2_files",
    "l2_bytes",
    "l3_files",
    "l3_bytes",
    "l4_files",
    "l4_bytes",
    "l5_files",
    "l5_bytes",
    "l6_files",
    "l6_bytes",
    "memtable_bytes",
    "immutable_memtables",
    "last_seq",
    "block_cache_bytes",
    "commits",
    "wal_bytes",
    "wal_syncs",
    "memtable_rotations",
    "flushes",
    "flush_bytes_written",
    "compactions",
    "trivial_moves",
    "compaction_bytes_read",
    "compaction_bytes_written",
    "tombstones_dropped",
    "write_stalls",
    "write_stall_micros",
    "write_slowdowns",
    "bloom_checks",
    "bloom_negatives",
    "bloom_false_positives",
    "block_cache_hits",
    "block_cache_misses",
    "manifest_writes",
    "obsolete_files_deleted",
];

/// The names phase1-design.md §3 adds in WP04.
const WP04_NAMES: &[&str] = &[
    "l0_depth",
    "commit_queue_p99_micros",
    "commit_make_room_p99_micros",
    "commit_wal_write_p99_micros",
    "commit_wal_sync_p99_micros",
    "commit_apply_p99_micros",
    "stall_episodes_memtable",
    "stall_episodes_l0",
    "stall_micros_memtable",
    "stall_micros_l0",
    "stall_micros_max",
    "slowdown_commits",
    "slowdown_micros",
    "flush_micros_max",
    "compaction_micros_max",
    "l0_compaction_input_bytes_max",
    "l0_compaction_l1_files_max",
    "pending_compaction_bytes",
    "last_open_clean",
    "last_open_replayed_bytes",
];

/// The names the condition-1 follow-up levers add (`bytes_per_writeback` and
/// `dynamic_base_level`).
const FOLLOW_UP_NAMES: &[&str] = &["sst_writeback_hints", "l0_base_level_skips"];

#[test]
fn legacy_stat_names_present() {
    let vfs = SimVfs::new(6).as_vfs();
    let db = open(&vfs, &Config::from(&Options::small_for_tests()));
    for i in 0..400u32 {
        put(&db, format!("key-{i:06}").as_bytes(), &[3u8; 300]);
    }
    let snap = db.snapshot().unwrap();
    for i in 0..100u32 {
        snap.get(format!("key-{i:06}").as_bytes()).unwrap();
        snap.get(format!("absent-{i}").as_bytes()).unwrap();
    }
    drop(snap);
    let st = stats(&db);
    for name in M0_NAMES.iter().chain(WP04_NAMES).chain(FOLLOW_UP_NAMES) {
        assert!(st.contains_key(*name), "missing stat {name}");
    }
    assert_eq!(st["commits"], 400);
    assert!(st["flushes"] > 0 && st["bloom_checks"] > 0);
    // The M0 names are derived from the new ones.
    assert_eq!(st["write_stalls"], st["stall_episodes_memtable"] + st["stall_episodes_l0"]);
    assert_eq!(st["write_stall_micros"], st["stall_micros_memtable"] + st["stall_micros_l0"]);
    assert_eq!(st["write_slowdowns"], st["slowdown_commits"]);

    // `stats()` is `metrics()` flattened.
    let m = db.close_with_metrics().unwrap();
    let flat: HashMap<String, u64> = m.flatten().into_iter().collect();
    assert_eq!(flat["commits"], 400);
    assert_eq!(flat["bloom_checks"], m.reads.bloom_checks);
    assert_eq!(flat["l0_files"], m.shape.levels[0].files);
    assert_eq!(flat["l6_bytes"], m.shape.levels[6].bytes);
    assert_eq!(flat["commit_wal_sync_p99_micros"], m.commit_phases.wal_sync.p99_micros);
    assert_eq!(flat["pending_compaction_bytes"], m.background.pending_compaction_bytes);
    assert_eq!(flat.len(), st.len(), "the set of names does not depend on the state");
}

/// Leveled compaction targets of `cfg`, as the engine computes them.
fn expected_pending(m: &Metrics, cfg: &CompactionConfig) -> u64 {
    let levels = &m.shape.levels;
    let mut pending = 0;
    if levels[0].files >= cfg.l0_trigger as u64 {
        pending += levels[0].bytes;
    }
    for (n, l) in levels.iter().enumerate().take(levels.len() - 1).skip(1) {
        let target = cfg.level_base_bytes.unwrap() * cfg.level_multiplier.pow(n as u32 - 1);
        pending += l.bytes.saturating_sub(target);
    }
    pending
}

#[test]
fn pending_compaction_bytes_reported() {
    let sim = SimVfs::new(7);
    // The M0 policy: this test counts L0 files against the trigger.
    let cfg = Config { compaction: CompactionConfig::m0(16 << 10), ..Config::from(&Options::small_for_tests()) };
    assert_eq!(cfg.compaction.l0_trigger, 4);
    let value = vec![9u8; 17 << 10]; // fills a 16 KiB memtable

    // L0 at its trigger, its compaction held: all of L0 is pending.
    let gate = GatedVfs::new(sim.as_vfs(), &[COMPACT]);
    let vfs: Arc<dyn Vfs> = gate.clone();
    let db = open(&vfs, &cfg);
    let open_on_drop = OpenOnDrop(gate.clone());
    assert_eq!(db.metrics().background.pending_compaction_bytes, 0);
    for i in 0..5u8 {
        put(&db, &[i], &value);
    }
    wait_until("four L0 files and their compaction held", || {
        stat(&db, "l0_files") == 4 && stat(&db, "immutable_memtables") == 0 && gate.waiting(COMPACT) == 1
    });
    let m = db.metrics();
    assert!(m.shape.levels[0].bytes > 4 * (17 << 10));
    assert_eq!(m.background.pending_compaction_bytes, m.shape.levels[0].bytes);
    assert_eq!(stat(&db, "pending_compaction_bytes"), m.shape.levels[0].bytes);

    // Once compaction catches up, nothing is pending.
    gate.open_all();
    wait_until("compaction to settle", || stat(&db, "pending_compaction_bytes") == 0);
    let m = db.close_with_metrics().unwrap();
    assert_eq!(m.background.pending_compaction_bytes, 0);
    assert!(m.shape.levels[1].bytes > 0);
    drop(open_on_drop);

    // Reopened with smaller level targets and compaction held (also a
    // trivial move, which writes only the MANIFEST): the bytes above each
    // target are pending.
    let gate = GatedVfs::holding_all_files(sim.as_vfs(), &[COMPACT]);
    let vfs: Arc<dyn Vfs> = gate.clone();
    let small = CompactionConfig { level_base_bytes: Some(1 << 10), level_multiplier: 1, ..cfg.compaction.clone() };
    let db = open(&vfs, &Config { compaction: small.clone(), ..cfg.clone() });
    let _open = OpenOnDrop(gate.clone());
    let m = db.metrics();
    let pending = m.background.pending_compaction_bytes;
    // Trivial moves may have carried the files to the last level, which has no
    // target; only a file in L1..L5 is guaranteed to be above the 1 KiB one.
    let l1_to_l5 = m.shape.levels[1..m.shape.levels.len() - 1].iter().map(|l| l.bytes).sum::<u64>();
    assert_eq!(pending > 0, l1_to_l5 > 0);
    assert_eq!(pending, expected_pending(&m, &small));
    assert!(m.shape.levels[0].files < 4, "L0 is below its trigger after a clean close");
    gate.open_all();
    db.close().unwrap();
}

fn read_delta(after: &ReadMetrics, before: &ReadMetrics) -> [u64; 4] {
    [
        after.bloom_checks - before.bloom_checks,
        after.bloom_negatives - before.bloom_negatives,
        after.bloom_false_positives - before.bloom_false_positives,
        (after.block_cache_hits + after.block_cache_misses) - (before.block_cache_hits + before.block_cache_misses),
    ]
}

#[test]
fn l0_files_max_includes_the_l0_found_at_open() {
    let vfs = SimVfs::new(11).as_vfs();
    let small = Config { compaction: CompactionConfig::m0(16 << 10), ..Config::from(&Options::small_for_tests()) };
    let deep = Config {
        compaction: CompactionConfig { l0_trigger: 100, l0_slowdown: 100, l0_stop: 100, ..small.compaction.clone() },
        ..small.clone()
    };
    let db = open(&vfs, &deep);
    let value = vec![3u8; 17 << 10]; // fills a 16 KiB memtable
    for i in 0..7u8 {
        put(&db, &[i], &value);
    }
    let depth = db.close_with_metrics().unwrap().shape.levels[0].files;
    assert!(depth >= 6, "{depth}");

    // Reopened with the default trigger: compaction drains that L0, and the
    // maximum stays at the depth this handle found.
    let db = open(&vfs, &small);
    assert_eq!(db.metrics().shape.l0_files_max, depth);
    wait_until("L0 drained", || stat(&db, "l0_files") == 0 && stat(&db, "pending_compaction_bytes") == 0);
    assert_eq!(db.metrics().shape.l0_files_max, depth);
    assert_eq!(stat(&db, "l0_files_max"), depth);
    db.close().unwrap();
}

#[test]
fn read_counters_are_sharded_and_summed() {
    assert_eq!(READ_COUNTER_SHARDS, 16);
    let vfs = SimVfs::new(8).as_vfs();
    let cfg = Config::from(&Options::small_for_tests());
    let db = open(&vfs, &cfg);
    for chunk in 0..40u32 {
        let mut b = WriteBatch::new();
        for i in 0..50 {
            b.put(format!("key-{:06}", chunk * 50 + i), [5u8; 200]);
        }
        db.commit(&b).unwrap();
    }
    db.close().unwrap();
    // Reopened after a clean close: every key is in an SST, the block cache
    // is empty and no background work is pending, so every lookup of a key
    // checks the same filters and reads the same blocks in any thread.
    let db = open(&vfs, &cfg);
    let keys: Vec<Vec<u8>> = (0..300)
        .map(|i| if i % 3 == 0 { format!("key-{:06}-absent", i * 6) } else { format!("key-{:06}", i * 6) }.into_bytes())
        .collect();
    let lookups = |db: &KernoDb| {
        let snap = db.snapshot().unwrap();
        for k in &keys {
            snap.get(k).unwrap();
        }
    };

    let start = db.metrics().reads;
    lookups(&db);
    let one = db.metrics().reads;
    let per_thread = read_delta(&one, &start);
    assert!(per_thread[0] > 0 && per_thread[1] > 0 && per_thread[3] > 0, "{per_thread:?}");

    let threads = 8u64;
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| lookups(&db));
        }
    });
    let all = db.metrics().reads;
    let got = read_delta(&all, &one);
    assert_eq!(got, per_thread.map(|n| n * threads), "the sum over the thread slots lost or invented counts");

    // `stats()` reports the same sums.
    let st = stats(&db);
    assert_eq!(st["bloom_checks"], all.bloom_checks);
    assert_eq!(st["bloom_negatives"], all.bloom_negatives);
    assert_eq!(st["block_cache_hits"] + st["block_cache_misses"], all.block_cache_hits + all.block_cache_misses);
    db.close().unwrap();
}
