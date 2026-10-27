//! `LsmEngine`: recovery, the commit path, background flush/compaction
//! threads, snapshots and clean shutdown.

use std::any::Any;
use std::collections::HashSet;
use std::io;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use spike_core::vfs::Vfs;
use spike_core::{Durability, Engine, Error, Options, Result, Snapshot, WriteBatch};

use crate::compaction::{self, BLOCK_SIZE, L0_SLOWDOWN, L0_STOP, MAX_IMMUTABLE, NUM_LEVELS, Plan, Tuning};
use crate::iter::{InternalIterator, MergeIter};
use crate::manifest::{self, MANIFEST, MANIFEST_TMP, ManifestData};
use crate::memtable::Memtable;
use crate::sst::{Table, TableBuilder, TableCtx};
use crate::stats::{Stats, bump};
use crate::version::{Levels, Version};
use crate::wal::{self, WalWriter, wal_name};

enum FileKind {
    Manifest,
    ManifestTmp,
    Wal(u64),
    Sst(u64),
    Other,
}

fn parse_name(name: &str) -> FileKind {
    // Only canonical names (as produced by `wal_name` / `sst_name`) count:
    // a stray "1.wal" would otherwise be opened and deleted as "000001.wal".
    let number = |s: &str| {
        let n: u64 = s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse().ok()).flatten()?;
        (format!("{n:06}") == s).then_some(n)
    };
    if name == MANIFEST {
        FileKind::Manifest
    } else if name == MANIFEST_TMP {
        FileKind::ManifestTmp
    } else if let Some(n) = name.strip_suffix(".wal").and_then(number) {
        FileKind::Wal(n)
    } else if let Some(n) = name.strip_suffix(".sst").and_then(number) {
        FileKind::Sst(n)
    } else {
        FileKind::Other
    }
}

/// Owned by the (single) writer: the active WAL and memtable.
struct Writer {
    wal: WalWriter,
    mem: Arc<Memtable>,
    last_seq: u64,
}

/// Published state, read by snapshots and background threads.
struct State {
    version: Arc<Version>,
    /// Last commit visible to new snapshots.
    last_seq: u64,
    poisoned: Option<String>,
    shutdown: bool,
}

/// Durable LSM shape as of the last MANIFEST written. Held while computing
/// and writing a new MANIFEST, so that flushes and compactions apply their
/// edits one at a time.
struct ManifestState {
    levels: Levels,
    last_seq: u64,
    log_number: u64,
}

struct Inner {
    vfs: Arc<dyn Vfs>,
    dir: PathBuf,
    durability: Durability,
    tuning: Tuning,
    ctx: Arc<TableCtx>,
    stats: Arc<Stats>,
    next_file: AtomicU64,
    // Lock order: writer -> manifest -> state.
    writer: Mutex<Writer>,
    manifest: Mutex<ManifestState>,
    state: Mutex<State>,
    /// Signalled whenever `state` changes in a way someone may wait for.
    cv: Condvar,
    poisoned: AtomicBool,
    shutting_down: AtomicBool,
    _lock: Box<dyn Any + Send + Sync>,
}

/// See the [crate documentation](crate).
pub struct LsmEngine {
    inner: Arc<Inner>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

pub struct LsmSnapshot {
    version: Arc<Version>,
    seq: u64,
}

impl Snapshot for LsmSnapshot {
    fn seq(&self) -> u64 {
        self.seq
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.version.get(key, self.seq)
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.version.scan(self.seq, start, end, limit)
    }
}

/// Writes the newest version of every key in `mem` to a new synced table.
fn write_memtable(ctx: &Arc<TableCtx>, mem: &Memtable, number: u64) -> Result<Table> {
    let mut b = TableBuilder::create(ctx, number)?;
    let fill = (|| {
        let mut it = MergeIter::new(vec![Box::new(mem.iter(u64::MAX))]);
        it.seek_to_first()?;
        while it.valid() {
            b.add(it.key(), it.seq(), it.value())?;
            it.next()?;
        }
        Ok::<_, Error>(())
    })();
    match fill {
        Ok(()) => b.finish(),
        Err(e) => {
            b.abandon();
            Err(e)
        }
    }
}

fn manifest_data(levels: &Levels, last_seq: u64, log_number: u64, next_file: u64) -> ManifestData {
    ManifestData {
        last_seq,
        log_number,
        next_file,
        levels: levels.iter().map(|l| l.iter().map(|t| t.meta()).collect()).collect(),
    }
}

struct Recovered {
    levels: Levels,
    last_seq: u64,
    log_number: u64,
    next_file: u64,
    wal: WalWriter,
}

/// Crash recovery (also run for a clean or brand-new database):
///
/// 1. read and verify the MANIFEST and every SST it references;
/// 2. replay WALs numbered `>= log_number` in order (only the newest may
///    end in a torn record);
/// 3. write the replayed commits to a new L0 table;
/// 4. write a MANIFEST pointing at a fresh WAL number, so every old WAL is
///    obsolete, then delete obsolete WALs and unreferenced SSTs;
/// 5. create the new WAL.
///
/// When this returns, the recovered state is durable on its own.
fn recover(ctx: &Arc<TableCtx>) -> Result<Recovered> {
    let vfs = &*ctx.vfs;
    let dir = ctx.dir.as_path();
    let mut has_manifest = false;
    let (mut wals, mut ssts) = (Vec::new(), Vec::new());
    for name in vfs.list(dir)? {
        match parse_name(&name) {
            FileKind::Manifest => has_manifest = true,
            FileKind::Wal(n) => wals.push(n),
            FileKind::Sst(n) => ssts.push(n),
            FileKind::ManifestTmp | FileKind::Other => {}
        }
    }
    wals.sort_unstable();
    let m = if has_manifest {
        manifest::read(vfs, dir, NUM_LEVELS)?
    } else if !wals.is_empty() || !ssts.is_empty() {
        return Err(Error::Corruption("data files exist but the MANIFEST is missing".into()));
    } else {
        ManifestData::empty(NUM_LEVELS)
    };
    let mut next_file = wals.iter().chain(&ssts).map(|n| n + 1).fold(m.next_file.max(1), u64::max);

    let present: HashSet<u64> = ssts.iter().copied().collect();
    let mut levels: Levels = vec![Vec::new(); NUM_LEVELS];
    for (l, files) in m.levels.iter().enumerate() {
        for f in files {
            if !present.contains(&f.number) {
                return Err(Error::Corruption(format!("{}: referenced but missing", crate::sst::sst_name(f.number))));
            }
            levels[l].push(Arc::new(Table::open(ctx, f)?));
        }
    }

    let mem = Memtable::new(0);
    let mut last_seq = m.last_seq;
    let live_wals: Vec<u64> = wals.iter().copied().filter(|&n| n >= m.log_number).collect();
    for (i, &n) in live_wals.iter().enumerate() {
        let name = wal_name(n);
        let file = vfs.open(&dir.join(&name), spike_core::OpenMode::Existing)?;
        let r = wal::replay(&*file, &name, last_seq + 1, |seq, ops| mem.apply(seq, ops))?;
        if r.torn && i + 1 < live_wals.len() {
            return Err(Error::Corruption(format!(
                "{name}: invalid record at offset {} in a log that is not the newest",
                r.valid_len
            )));
        }
        last_seq = r.last_seq;
    }
    if !mem.is_empty() {
        let table = write_memtable(ctx, &mem, next_file)?;
        next_file += 1;
        vfs.sync_dir(dir)?;
        levels[0].insert(0, Arc::new(table));
    }

    let wal_number = next_file;
    next_file += 1;
    manifest::write(vfs, dir, &manifest_data(&levels, last_seq, wal_number, next_file))?;
    let live: HashSet<u64> = levels.iter().flatten().map(|t| t.number).collect();
    // A snapshot that outlived the previous handle may delete one of these
    // obsolete files concurrently; a file that is already gone is fine.
    let remove = |path: &Path| match vfs.remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    };
    for &n in &wals {
        remove(&dir.join(wal_name(n)))?;
    }
    for &n in ssts.iter().filter(|n| !live.contains(n)) {
        remove(&ctx.path(n))?;
    }
    let wal = WalWriter::create(vfs, dir, wal_number)?;
    vfs.sync_dir(dir)?;
    Ok(Recovered { levels, last_seq, log_number: wal_number, next_file, wal })
}

impl Inner {
    fn poison(&self, msg: String) {
        self.poisoned.store(true, Ordering::SeqCst);
        let mut st = self.state.lock();
        if st.poisoned.is_none() {
            st.poisoned = Some(msg);
        }
        drop(st);
        self.cv.notify_all();
    }

    fn check_poisoned(&self) -> Result<()> {
        if self.poisoned.load(Ordering::SeqCst) {
            let msg = self.state.lock().poisoned.clone().unwrap_or_default();
            return Err(Error::Poisoned(msg));
        }
        Ok(())
    }

    /// Runs `f`; any error poisons the handle.
    fn or_poison<T>(&self, what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        f().inspect_err(|e| self.poison(format!("{what}: {e}")))
    }

    fn write_manifest(&self, levels: &Levels, last_seq: u64, log_number: u64) -> Result<()> {
        let data = manifest_data(levels, last_seq, log_number, self.next_file.load(Ordering::SeqCst));
        manifest::write(&*self.vfs, &self.dir, &data)?;
        bump(&self.stats.manifest_writes, 1);
        Ok(())
    }

    /// Publishes a version with new levels (and without the flushed
    /// memtable, if any). Called with the manifest lock held.
    fn install(&self, levels: Levels, flushed: Option<&Arc<Memtable>>) {
        let old = {
            let mut st = self.state.lock();
            let v = &st.version;
            let imms = match flushed {
                Some(m) => v.imms.iter().filter(|x| !Arc::ptr_eq(x, m)).cloned().collect(),
                None => v.imms.clone(),
            };
            let new = Arc::new(Version { mem: v.mem.clone(), imms, levels });
            std::mem::replace(&mut st.version, new)
        };
        self.cv.notify_all();
        // Dropping the old version may delete obsolete files: not under the lock.
        drop(old);
    }

    // ---- commit path ----

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        batch.validate()?;
        let mut w = self.writer.lock();
        self.check_poisoned()?;
        if batch.is_empty() {
            return Ok(w.last_seq);
        }
        self.make_room(&mut w)?;
        let seq = w.last_seq.checked_add(1).ok_or_else(|| Error::Corruption("commit sequence exhausted".into()))?;
        let sync = self.durability == Durability::Sync;
        let record = wal::encode_record(seq, batch.ops(), sync)?;
        let io = w.wal.append(&record).and_then(|()| if sync { w.wal.sync() } else { Ok(()) });
        if let Err(e) = io {
            self.poison(format!("WAL write failed: {e}"));
            return Err(e.into());
        }
        w.mem.apply(seq, batch.ops());
        w.last_seq = seq;
        self.state.lock().last_seq = seq;
        bump(&self.stats.commits, 1);
        bump(&self.stats.wal_bytes, record.len() as u64);
        if sync {
            bump(&self.stats.wal_syncs, 1);
        }
        Ok(seq)
    }

    /// Applies backpressure and switches to a new memtable when the active
    /// one is full (LevelDB's `MakeRoomForWrite`).
    fn make_room(&self, w: &mut Writer) -> Result<()> {
        let mut slowed = false;
        loop {
            self.check_poisoned()?;
            let mut st = self.state.lock();
            let l0 = st.version.levels[0].len();
            if !slowed && l0 >= L0_SLOWDOWN {
                // Give compaction a moment instead of stopping hard later.
                slowed = true;
                bump(&self.stats.write_slowdowns, 1);
                self.cv.wait_for(&mut st, Duration::from_millis(1));
                continue;
            }
            if w.mem.approx_bytes() < self.tuning.write_buffer {
                return Ok(());
            }
            if st.version.imms.len() >= MAX_IMMUTABLE || l0 >= L0_STOP {
                let start = Instant::now();
                if st.poisoned.is_none() {
                    self.cv.wait(&mut st);
                }
                bump(&self.stats.write_stalls, 1);
                bump(&self.stats.write_stall_micros, start.elapsed().as_micros() as u64);
                continue;
            }
            drop(st);
            return self.or_poison("memtable switch failed", || self.rotate(w));
        }
    }

    fn rotate(&self, w: &mut Writer) -> Result<()> {
        if self.durability == Durability::Buffered {
            // Every older log must be complete before a newer one exists,
            // so that recovery's "only the newest log may be torn" holds.
            w.wal.sync()?;
        }
        let number = self.next_file.fetch_add(1, Ordering::SeqCst);
        let wal = WalWriter::create(&*self.vfs, &self.dir, number)?;
        self.vfs.sync_dir(&self.dir)?;
        let mem = Arc::new(Memtable::new(number));
        let old = {
            let mut st = self.state.lock();
            let v = &st.version;
            let mut imms = Vec::with_capacity(v.imms.len() + 1);
            imms.push(w.mem.clone());
            imms.extend(v.imms.iter().cloned());
            let new = Arc::new(Version { mem: mem.clone(), imms, levels: v.levels.clone() });
            std::mem::replace(&mut st.version, new)
        };
        self.cv.notify_all();
        drop(old);
        w.mem = mem;
        w.wal = wal;
        bump(&self.stats.memtable_rotations, 1);
        Ok(())
    }

    // ---- background work ----

    /// Waits for `next` to produce work; `None` on shutdown or poison.
    fn wait_for_work<T>(&self, mut next: impl FnMut(&State) -> Option<T>) -> Option<T> {
        let mut st = self.state.lock();
        loop {
            if st.shutdown || st.poisoned.is_some() {
                return None;
            }
            if let Some(work) = next(&st) {
                return Some(work);
            }
            self.cv.wait(&mut st);
        }
    }

    fn flusher(&self) {
        while let Some(mem) = self.wait_for_work(|st| st.version.imms.last().cloned()) {
            if self.or_poison("flush failed", || self.flush(&mem)).is_err() {
                return;
            }
        }
    }

    /// Writes the oldest immutable memtable to L0 and retires its WAL.
    fn flush(&self, mem: &Arc<Memtable>) -> Result<()> {
        let table = if mem.is_empty() {
            None
        } else {
            let table = write_memtable(&self.ctx, mem, self.next_file.fetch_add(1, Ordering::SeqCst))?;
            self.vfs.sync_dir(&self.dir)?;
            bump(&self.stats.flush_bytes_written, table.size);
            Some(Arc::new(table))
        };
        let mut ms = self.manifest.lock();
        let mut levels = ms.levels.clone();
        if let Some(t) = table {
            levels[0].insert(0, t);
        }
        let last_seq = ms.last_seq.max(mem.max_seq());
        let log_number = ms.log_number.max(mem.wal_number() + 1);
        self.write_manifest(&levels, last_seq, log_number)?;
        ms.levels = levels.clone();
        ms.last_seq = last_seq;
        ms.log_number = log_number;
        self.install(levels, Some(mem));
        drop(ms);
        // The MANIFEST no longer needs this log (best effort: a leftover
        // log below `log_number` is deleted by the next recovery).
        let _ = self.vfs.remove_file(&self.dir.join(wal_name(mem.wal_number())));
        bump(&self.stats.flushes, 1);
        Ok(())
    }

    fn compactor(&self) {
        // Round-robin position per level: largest key of the last compaction.
        let mut pointers = vec![Vec::new(); NUM_LEVELS];
        while let Some(plan) = self.wait_for_work(|st| compaction::pick(&st.version.levels, &self.tuning, &pointers)) {
            match self.or_poison("compaction failed", || self.compact(&plan, &mut pointers)) {
                Ok(true) => {}
                Ok(false) | Err(_) => return,
            }
        }
    }

    /// Returns `false` if the compaction was abandoned for shutdown.
    fn compact(&self, plan: &Plan, pointers: &mut [Vec<u8>]) -> Result<bool> {
        pointers[plan.level] = plan.largest_input_key().to_vec();
        if plan.is_trivial_move() {
            let mut ms = self.manifest.lock();
            let t = &plan.inputs[0];
            let mut levels = ms.levels.clone();
            levels[plan.level].retain(|x| x.number != t.number);
            let next = &mut levels[plan.level + 1];
            let at = next.partition_point(|x| x.smallest < t.smallest);
            next.insert(at, t.clone());
            self.write_manifest(&levels, ms.last_seq, ms.log_number)?;
            ms.levels = levels.clone();
            self.install(levels, None);
            bump(&self.stats.trivial_moves, 1);
            return Ok(true);
        }
        let Some(outputs) =
            compaction::run(&self.ctx, plan, &self.tuning, &self.next_file, &self.shutting_down, &self.stats)?
        else {
            return Ok(false);
        };
        // On failure the outputs are left on disk: once the MANIFEST write
        // has started, a new MANIFEST that references them may survive (a
        // rename whose `sync_dir` failed). Recovery deletes them if not.
        if !outputs.is_empty() {
            self.vfs.sync_dir(&self.dir)?;
        }
        let mut ms = self.manifest.lock();
        let levels = compaction::apply(&ms.levels, plan, &outputs)?;
        self.write_manifest(&levels, ms.last_seq, ms.log_number)?;
        ms.levels = levels.clone();
        for t in plan.all_inputs() {
            t.mark_obsolete();
        }
        self.install(levels, None);
        drop(ms);
        let read: u64 = plan.all_inputs().map(|t| t.size).sum();
        bump(&self.stats.compactions, 1);
        bump(&self.stats.compaction_bytes_read, read);
        bump(&self.stats.compaction_bytes_written, outputs.iter().map(|t| t.size).sum());
        Ok(true)
    }

    // ---- shutdown ----

    /// Runs compactions in the calling thread until no level is over its
    /// target. Used by `close` (after the background threads stopped and
    /// the final flush), so that a database is never left with a growing
    /// L0 backlog: before, close abandoned the running compaction and then
    /// added one more L0 file, so many short sessions grew L0 without bound.
    fn compact_until_settled(&self) -> Result<()> {
        // The background threads are joined; compaction must not see the
        // shutdown flag and abandon its work.
        self.shutting_down.store(false, Ordering::SeqCst);
        let mut pointers = vec![Vec::new(); NUM_LEVELS];
        loop {
            let plan = {
                let st = self.state.lock();
                compaction::pick(&st.version.levels, &self.tuning, &pointers)
            };
            match plan {
                Some(plan) => {
                    if !self.compact(&plan, &mut pointers)? {
                        return Ok(());
                    }
                }
                None => return Ok(()),
            }
        }
    }

    /// Flushes every memtable, writes a MANIFEST that needs no WAL, removes
    /// the WALs. Background threads must already be stopped.
    fn close(&self) -> Result<()> {
        let w = self.writer.lock();
        let v = self.state.lock().version.clone();
        let mut tables = Vec::new();
        for mem in v.imms.iter().rev().chain(std::iter::once(&v.mem)) {
            if !mem.is_empty() {
                let t = write_memtable(&self.ctx, mem, self.next_file.fetch_add(1, Ordering::SeqCst))?;
                tables.push(Arc::new(t));
            }
        }
        if !tables.is_empty() {
            self.vfs.sync_dir(&self.dir)?;
        }
        let mut ms = self.manifest.lock();
        let mut levels = ms.levels.clone();
        for t in tables {
            levels[0].insert(0, t);
        }
        let log_number = self.next_file.fetch_add(1, Ordering::SeqCst);
        self.write_manifest(&levels, w.last_seq, log_number)?;
        ms.levels = levels.clone();
        ms.last_seq = w.last_seq;
        ms.log_number = log_number;
        drop(ms);
        drop(v);
        {
            // Drop the memtables; later snapshots of this handle are not possible.
            let mut st = self.state.lock();
            let mem = Arc::new(Memtable::new(log_number));
            st.version = Arc::new(Version { mem, imms: Vec::new(), levels });
        }
        for name in self.vfs.list(&self.dir)? {
            if let FileKind::Wal(_) = parse_name(&name) {
                self.vfs.remove_file(&self.dir.join(name))?;
            }
        }
        self.vfs.sync_dir(&self.dir)?;
        Ok(())
    }
}

impl LsmEngine {
    fn spawn(inner: &Arc<Inner>, name: &str, body: fn(&Inner)) -> io::Result<JoinHandle<()>> {
        let inner = inner.clone();
        std::thread::Builder::new().name(name.into()).spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&inner)));
            if r.is_err() {
                inner.poison("background thread panicked".into());
            }
        })
    }

    /// Signals the background threads and waits for them.
    fn stop_threads(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        self.inner.state.lock().shutdown = true;
        self.inner.cv.notify_all();
        for t in self.threads.lock().drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for LsmEngine {
    fn drop(&mut self) {
        self.stop_threads();
    }
}

impl Engine for LsmEngine {
    type Snapshot = LsmSnapshot;

    fn name() -> &'static str {
        "lsm"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        vfs.create_dir_all(dir)?;
        // The directory itself must be durable in its parent, or a power
        // loss can remove a newly created database with everything in it.
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            vfs.sync_dir(parent)?;
        }
        let lock = vfs.lock(&dir.join("LOCK")).map_err(|e| match e.kind() {
            io::ErrorKind::WouldBlock => Error::Locked,
            _ => Error::Io(e),
        })?;
        let stats = Arc::new(Stats::default());
        let ctx = Arc::new(TableCtx::new(vfs.clone(), dir.to_path_buf(), opts.cache_bytes, stats.clone(), BLOCK_SIZE));
        let r = recover(&ctx)?;
        let mem = Arc::new(Memtable::new(r.wal.number()));
        let version = Arc::new(Version { mem: mem.clone(), imms: Vec::new(), levels: r.levels.clone() });
        let inner = Arc::new(Inner {
            vfs,
            dir: dir.to_path_buf(),
            durability: opts.durability,
            tuning: Tuning::new(opts),
            ctx,
            stats,
            next_file: AtomicU64::new(r.next_file),
            writer: Mutex::new(Writer { wal: r.wal, mem, last_seq: r.last_seq }),
            manifest: Mutex::new(ManifestState { levels: r.levels, last_seq: r.last_seq, log_number: r.log_number }),
            state: Mutex::new(State { version, last_seq: r.last_seq, poisoned: None, shutdown: false }),
            cv: Condvar::new(),
            poisoned: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            _lock: lock,
        });
        let engine = LsmEngine { inner: inner.clone(), threads: Mutex::new(Vec::new()) };
        for (name, body) in [("lsm-flush", Inner::flusher as fn(&Inner)), ("lsm-compact", Inner::compactor)] {
            let handle = Self::spawn(&inner, name, body)?;
            engine.threads.lock().push(handle);
        }
        Ok(engine)
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        self.inner.commit(batch)
    }

    fn snapshot(&self) -> Result<LsmSnapshot> {
        let st = self.inner.state.lock();
        if let Some(msg) = &st.poisoned {
            return Err(Error::Poisoned(msg.clone()));
        }
        Ok(LsmSnapshot { version: st.version.clone(), seq: st.last_seq })
    }

    fn close(self) -> Result<()> {
        self.stop_threads();
        self.inner.check_poisoned()?;
        self.inner.or_poison("close failed", || {
            self.inner.close()?;
            self.inner.compact_until_settled()
        })
    }

    fn stats(&self) -> Vec<(String, u64)> {
        let (v, seq) = {
            let st = self.inner.state.lock();
            (st.version.clone(), st.last_seq)
        };
        let mut out = Vec::new();
        for (i, level) in v.levels.iter().enumerate() {
            out.push((format!("l{i}_files"), level.len() as u64));
            out.push((format!("l{i}_bytes"), level.iter().map(|t| t.size).sum()));
        }
        out.push(("memtable_bytes".into(), v.mem.approx_bytes() as u64));
        out.push(("immutable_memtables".into(), v.imms.len() as u64));
        out.push(("last_seq".into(), seq));
        out.push(("block_cache_bytes".into(), self.inner.ctx.cache.weight()));
        out.extend(self.inner.stats.report());
        out
    }
}
