//! `KernoDb`: recovery, the commit path, background flush/compaction
//! threads (or their inline equivalent), snapshots and clean shutdown.

use std::any::Any;
use std::collections::HashSet;
use std::io;
use std::ops::Bound;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options, Snapshot, TestHooks};
use kerno_core::probe;
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, Result, WriteBatch};
use parking_lot::{Condvar, Mutex};

use crate::compaction::{self, BLOCK_SIZE, L0_SLOWDOWN, L0_STOP, NUM_LEVELS, Plan, Tuning};
use crate::config::{AccessMode, Background, Config};
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
    /// Inline mode: round-robin position per level for the compactions run
    /// under the writer lock (largest key of the last compaction out of it).
    /// The background compactor keeps its own.
    pointers: Vec<Vec<u8>>,
}

/// Round-robin positions that choose nothing: `compaction::pick` decides
/// whether a compaction is due regardless of them.
static NO_POINTERS: [Vec<u8>; NUM_LEVELS] = [const { Vec::new() }; NUM_LEVELS];

/// Published state, read by snapshots and background threads.
struct State {
    version: Arc<Version>,
    /// Last commit visible to new snapshots.
    last_seq: u64,
    poisoned: Option<String>,
    shutdown: bool,
    /// A background thread is working on a flush or a compaction (including
    /// the deletions that follow it). See `Inner::wait_idle`.
    flush_running: bool,
    compaction_running: bool,
}

/// The kinds of background work.
#[derive(Clone, Copy)]
enum Job {
    Flush,
    Compaction,
}

/// Marks a background job as running until dropped (also on a panic).
struct Running<'a> {
    inner: &'a Inner,
    job: Job,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let mut st = self.inner.state.lock();
        match self.job {
            Job::Flush => st.flush_running = false,
            Job::Compaction => st.compaction_running = false,
        }
        drop(st);
        self.inner.cv.notify_all();
    }
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
    /// Writers wait while this many memtables are waiting to be flushed.
    max_immutable: usize,
    /// Background threads, or flushes and compactions inline.
    background: Background,
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
pub struct KernoDb {
    inner: Arc<Inner>,
    threads: Mutex<Vec<JoinHandle<()>>>,
}

pub struct KernoSnapshot {
    version: Arc<Version>,
    seq: u64,
}

impl Snapshot for KernoSnapshot {
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
        let file = vfs.open(&dir.join(&name), kerno_core::vfs::OpenMode::Existing)?;
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
    {
        let _phase = probe::phase("delete");
        for &n in &wals {
            remove(&dir.join(wal_name(n)))?;
        }
        for &n in ssts.iter().filter(|n| !live.contains(n)) {
            remove(&ctx.path(n))?;
        }
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

    fn inline(&self) -> bool {
        self.background == Background::Inline
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
        let _phase = probe::phase("commit");
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
        if self.inline() {
            return self.make_room_inline(w);
        }
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
            if st.version.imms.len() >= self.max_immutable || l0 >= L0_STOP {
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

    /// [`make_room`](Self::make_room) without background threads: there is
    /// nothing to wait for, so no slowdown and no stall. A full memtable is
    /// rotated, flushed and followed by every due compaction, all in the
    /// calling thread, before the commit writes its own record.
    fn make_room_inline(&self, w: &mut Writer) -> Result<()> {
        if w.mem.approx_bytes() < self.tuning.write_buffer {
            return Ok(());
        }
        self.or_poison("memtable switch failed", || self.rotate(w))?;
        self.flush_inline()?;
        while self.compact_inline(w)? {}
        Ok(())
    }

    fn rotate(&self, w: &mut Writer) -> Result<()> {
        let _phase = probe::phase("rotate");
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

    /// Waits for `next` to produce work; `None` on shutdown or poison. The
    /// job counts as running until the returned guard is dropped.
    fn wait_for_work<T>(&self, job: Job, mut next: impl FnMut(&State) -> Option<T>) -> Option<(T, Running<'_>)> {
        let mut st = self.state.lock();
        loop {
            if st.shutdown || st.poisoned.is_some() {
                return None;
            }
            if let Some(work) = next(&st) {
                match job {
                    Job::Flush => st.flush_running = true,
                    Job::Compaction => st.compaction_running = true,
                }
                return Some((work, Running { inner: self, job }));
            }
            self.cv.wait(&mut st);
        }
    }

    fn flusher(&self) {
        while let Some((mem, _running)) = self.wait_for_work(Job::Flush, |st| st.version.imms.last().cloned()) {
            if self.or_poison("flush failed", || self.flush(&mem)).is_err() {
                return;
            }
        }
    }

    /// Inline mode: flushes every immutable memtable, oldest first, in the
    /// calling thread (which holds the writer lock).
    fn flush_inline(&self) -> Result<()> {
        loop {
            let Some(mem) = self.state.lock().version.imms.last().cloned() else {
                return Ok(());
            };
            self.or_poison("flush failed", || self.flush(&mem))?;
        }
    }

    /// Writes the oldest immutable memtable to L0 and retires its WAL.
    fn flush(&self, mem: &Arc<Memtable>) -> Result<()> {
        let _phase = probe::phase("flush");
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
        let _delete = probe::phase("delete");
        let _ = self.vfs.remove_file(&self.dir.join(wal_name(mem.wal_number())));
        bump(&self.stats.flushes, 1);
        Ok(())
    }

    /// Whether some level is over its target (what `compaction::pick`
    /// decides; the round-robin positions only choose the input file).
    fn compaction_due(&self, levels: &Levels) -> bool {
        compaction::pick(levels, &self.tuning, &NO_POINTERS).is_some()
    }

    fn compactor(&self) {
        // Round-robin position per level: largest key of the last compaction.
        let mut pointers = vec![Vec::new(); NUM_LEVELS];
        while let Some((plan, running)) =
            self.wait_for_work(Job::Compaction, |st| compaction::pick(&st.version.levels, &self.tuning, &pointers))
        {
            let r = self.or_poison("compaction failed", || self.compact(&plan, &mut pointers));
            // The inputs it made obsolete are deleted while the job still
            // counts as running.
            drop(plan);
            drop(running);
            match r {
                Ok(true) => {}
                Ok(false) | Err(_) => return,
            }
        }
    }

    /// Inline mode: picks the most urgent compaction and runs it in the
    /// calling thread, which holds the writer lock. `false` if none is due.
    /// The inputs it made obsolete are deleted before this returns (unless a
    /// snapshot still uses them).
    fn compact_inline(&self, w: &mut Writer) -> Result<bool> {
        let plan = {
            let st = self.state.lock();
            compaction::pick(&st.version.levels, &self.tuning, &w.pointers)
        };
        match plan {
            Some(plan) => self.or_poison("compaction failed", || self.compact(&plan, &mut w.pointers)),
            None => Ok(false),
        }
    }

    /// Returns `false` if the compaction was abandoned for shutdown.
    fn compact(&self, plan: &Plan, pointers: &mut [Vec<u8>]) -> Result<bool> {
        let _phase = probe::phase("compact");
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
        let _phase = probe::phase("close");
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
        {
            // The M0 counterpart of the clean checkpoint: a MANIFEST that
            // needs no WAL.
            let _checkpoint = probe::phase("checkpoint");
            self.write_manifest(&levels, w.last_seq, log_number)?;
        }
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
        let _delete = probe::phase("delete");
        for name in self.vfs.list(&self.dir)? {
            if let FileKind::Wal(_) = parse_name(&name) {
                self.vfs.remove_file(&self.dir.join(name))?;
            }
        }
        self.vfs.sync_dir(&self.dir)?;
        Ok(())
    }

    // ---- test hooks ----

    /// [`TestHooks::wait_idle`]: with threads, waits until no flush is
    /// pending or running and no compaction is due or running.
    fn wait_idle(&self) -> Result<()> {
        let mut st = self.state.lock();
        loop {
            if let Some(msg) = &st.poisoned {
                return Err(Error::Poisoned(msg.clone()));
            }
            let idle = self.inline()
                || st.shutdown
                || (!st.flush_running
                    && !st.compaction_running
                    && st.version.imms.is_empty()
                    && !self.compaction_due(&st.version.levels));
            if idle {
                return Ok(());
            }
            self.cv.wait(&mut st);
        }
    }

    /// [`TestHooks::flush`]: rotates a non-empty active memtable and waits
    /// until every immutable memtable is on disk (inline: writes them).
    fn flush_all(&self) -> Result<()> {
        let mut w = self.writer.lock();
        self.check_poisoned()?;
        if !w.mem.is_empty() {
            self.or_poison("memtable switch failed", || self.rotate(&mut w))?;
        }
        if self.inline() {
            return self.flush_inline();
        }
        drop(w);
        let mut st = self.state.lock();
        loop {
            if let Some(msg) = &st.poisoned {
                return Err(Error::Poisoned(msg.clone()));
            }
            if st.shutdown || (st.version.imms.is_empty() && !st.flush_running) {
                return Ok(());
            }
            self.cv.wait(&mut st);
        }
    }

    /// [`TestHooks::compact_once`]. With background threads, compactions
    /// run on their own: this waits until none is due or running and
    /// returns whether at least one finished meanwhile. Every compaction
    /// that was due or running when it was called is among those (the
    /// counters are read before waiting, and a compaction counts itself
    /// before it stops counting as running); more than one may have run.
    fn compact_once(&self) -> Result<bool> {
        if !self.inline() {
            let finished =
                || self.stats.compactions.load(Ordering::SeqCst) + self.stats.trivial_moves.load(Ordering::SeqCst);
            let before = finished();
            self.wait_idle()?;
            return Ok(finished() > before);
        }
        let mut w = self.writer.lock();
        self.check_poisoned()?;
        self.compact_inline(&mut w)
    }
}

impl KernoDb {
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

impl Drop for KernoDb {
    fn drop(&mut self) {
        self.stop_threads();
    }
}

impl KernoDb {
    /// Test-only control over flushes and compactions (see [`TestHooks`]).
    #[doc(hidden)]
    pub fn hooks(&self) -> &dyn TestHooks {
        self
    }
}

impl TestHooks for KernoDb {
    fn wait_idle(&self) -> Result<()> {
        self.inner.wait_idle()
    }

    fn flush(&self) -> Result<()> {
        self.inner.flush_all()
    }

    fn compact_once(&self) -> Result<bool> {
        self.inner.compact_once()
    }
}

#[cfg(test)]
impl KernoDb {
    /// Wakes every thread waiting on the engine's condition variable. Every
    /// wait re-checks its condition, so the only effect is that a stalled
    /// writer counts the stall and waits again.
    pub(crate) fn wake_waiters(&self) {
        self.inner.cv.notify_all();
    }
}

/// The error of opening `dir`, which has no MANIFEST, when the database may
/// not be created. WAL or SST files without a MANIFEST are a damaged
/// database, not a missing one: the same `Corruption` as in [`recover`].
/// Anything else (no directory, an empty one, only other files) is
/// `InvalidArgument`. Only reads `dir`.
fn missing_manifest(vfs: &dyn Vfs, dir: &Path) -> Error {
    let names = match vfs.list(dir) {
        Ok(names) => names,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Error::Io(e),
    };
    if names.iter().any(|n| matches!(parse_name(n), FileKind::Wal(_) | FileKind::Sst(_))) {
        return Error::Corruption("data files exist but the MANIFEST is missing".into());
    }
    Error::InvalidArgument(format!("{}: no database (no {MANIFEST}) and create_if_missing is false", dir.display()))
}

/// The directory that holds `path`'s entry. For a relative path with a
/// single component that is the current directory, `.` (where
/// `Path::parent` returns an empty path, which no real file system opens).
/// `None` for a root, and for `.` or `..` alone, whose parent this process
/// does not name.
fn parent_dir(path: &Path) -> Option<&Path> {
    let parent = path.parent()?;
    if !parent.as_os_str().is_empty() {
        return Some(parent);
    }
    match path.components().next() {
        Some(Component::Normal(_)) => Some(Path::new(".")),
        _ => None,
    }
}

/// `dir` and those of its ancestors that do not exist, deepest first: what
/// `create_dir_all(dir)` is about to create. Only reads.
fn missing_dirs(vfs: &dyn Vfs, dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut next = Some(dir);
    while let Some(d) = next {
        if vfs.exists(d)? {
            break;
        }
        missing.push(d.to_path_buf());
        next = parent_dir(d);
    }
    Ok(missing)
}

/// Makes the database directory `dir` durable where it lives, after
/// `create_dir_all`. `created` is what that call created (see
/// [`missing_dirs`]); `new` is whether the database is new (no MANIFEST).
///
/// On file systems without a single metadata journal, an entry is durable
/// only once the directory that holds it is synced. So this syncs the
/// directory that holds `dir` (on every open, as in M0) and the one that
/// holds each directory created now: entries this open made, so failing to
/// sync them fails the open. Directories above those are not touched in
/// general: their entries did not change, and syncing them can fail for
/// reasons that do not concern the database (an ancestor that may only be
/// traversed, as with home directories in mode 0711 or app data on Android;
/// a read-only file system without directory fsync).
///
/// Except for a new database: an earlier open may have created some of
/// these directories (`create_dir_all` makes one level at a time, so even a
/// partial chain, without the database directory itself) and failed or died
/// before syncing them. Which ones is not known, so the ancestors above the
/// highest directory synced strictly are synced best effort, bottom up. The
/// walk stops quietly only where this process cannot sync the directory at
/// all (see [`not_ours`]): one it may not read was not created by it, and
/// neither was anything above it. Any other error (an `EIO`) fails the open,
/// as a failed fsync must never be taken as durable (ADR-0002).
fn sync_dir_entries(vfs: &dyn Vfs, dir: &Path, created: &[PathBuf], new: bool) -> io::Result<()> {
    let mut synced: Vec<&Path> = Vec::new();
    for d in std::iter::once(dir).chain(created.iter().map(PathBuf::as_path)) {
        if let Some(parent) = parent_dir(d)
            && !synced.contains(&parent)
        {
            vfs.sync_dir(parent)?;
            synced.push(parent);
        }
    }
    if new {
        let highest = created.last().map_or(dir, PathBuf::as_path);
        let mut ancestor = parent_dir(highest).and_then(parent_dir);
        while let Some(a) = ancestor {
            match vfs.sync_dir(a) {
                Ok(()) => ancestor = parent_dir(a),
                Err(e) if not_ours(&e) => break,
                Err(e) => return Err(e),
            }
        }
    }
    Ok(())
}

/// Whether a failed `sync_dir` of an ancestor shows a directory this process
/// cannot sync at all (so did not create): no read permission, or a file
/// system that is read-only or does not support directory fsync.
fn not_ours(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem | io::ErrorKind::Unsupported)
}

impl KernoDb {
    /// Opens the database stored in `dir` (creating it if it does not exist
    /// and `cfg.create_if_missing` is set), running crash recovery if the
    /// previous handle was not closed cleanly.
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &Config) -> Result<KernoDb> {
        cfg.validate()?;
        if cfg.access == AccessMode::ReadOnly {
            return Err(Error::InvalidArgument("read-only mode not implemented yet".into()));
        }
        // Checked before anything is created: without a MANIFEST there is no
        // database to open, and the directory must stay untouched.
        let exists = vfs.exists(&dir.join(MANIFEST))?;
        if !cfg.create_if_missing && !exists {
            return Err(missing_manifest(&*vfs, dir));
        }
        let _phase = probe::phase(if exists { "session" } else { "create" });
        let created = if exists { Vec::new() } else { missing_dirs(&*vfs, dir)? };
        vfs.create_dir_all(dir)?;
        sync_dir_entries(&*vfs, dir, &created, !exists)?;
        let lock = vfs.lock(&dir.join("LOCK")).map_err(|e| match e.kind() {
            io::ErrorKind::WouldBlock => Error::Locked,
            _ => Error::Io(e),
        })?;
        let stats = Arc::new(Stats::default());
        let ctx = Arc::new(TableCtx::new(vfs.clone(), dir.to_path_buf(), cfg.cache_bytes, stats.clone(), BLOCK_SIZE));
        let r = recover(&ctx)?;
        let mem = Arc::new(Memtable::new(r.wal.number()));
        let version = Arc::new(Version { mem: mem.clone(), imms: Vec::new(), levels: r.levels.clone() });
        let inner = Arc::new(Inner {
            vfs,
            dir: dir.to_path_buf(),
            durability: cfg.durability,
            tuning: Tuning::new(cfg),
            max_immutable: cfg.max_immutable_memtables,
            background: cfg.test.background,
            ctx,
            stats,
            next_file: AtomicU64::new(r.next_file),
            writer: Mutex::new(Writer {
                wal: r.wal,
                mem,
                last_seq: r.last_seq,
                pointers: vec![Vec::new(); NUM_LEVELS],
            }),
            manifest: Mutex::new(ManifestState { levels: r.levels, last_seq: r.last_seq, log_number: r.log_number }),
            state: Mutex::new(State {
                version,
                last_seq: r.last_seq,
                poisoned: None,
                shutdown: false,
                flush_running: false,
                compaction_running: false,
            }),
            cv: Condvar::new(),
            poisoned: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            _lock: lock,
        });
        let engine = KernoDb { inner: inner.clone(), threads: Mutex::new(Vec::new()) };
        if cfg.test.background == Background::Threads {
            for (name, body) in [("kerno-flush", Inner::flusher as fn(&Inner)), ("kerno-compact-0", Inner::compactor)] {
                let handle = Self::spawn(&inner, name, body)?;
                engine.threads.lock().push(handle);
            }
        }
        Ok(engine)
    }
}

impl Engine for KernoDb {
    type Snapshot = KernoSnapshot;

    fn name() -> &'static str {
        "kerno"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        KernoDb::open(vfs, dir, &Config::from(opts))
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        // Range deletions are not implemented yet (WP10b replaces this):
        // reject them before anything is written or a sequence is assigned.
        if batch.has_range_deletes() {
            return Err(Error::InvalidArgument("delete_range not yet supported".into()));
        }
        self.inner.commit(batch)
    }

    fn snapshot(&self) -> Result<KernoSnapshot> {
        let st = self.inner.state.lock();
        if let Some(msg) = &st.poisoned {
            return Err(Error::Poisoned(msg.clone()));
        }
        Ok(KernoSnapshot { version: st.version.clone(), seq: st.last_seq })
    }

    fn close(self) -> Result<()> {
        let _phase = probe::phase("close");
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

/// Range deletions are rejected until WP10b implements them.
#[cfg(test)]
mod range_delete_rejection {
    use kerno_core::contract::{Engine, Options, Snapshot};
    use kerno_testkit::vfs::SimVfs;

    use super::*;

    fn stat(db: &KernoDb, name: &str) -> u64 {
        db.stats().into_iter().find(|(n, _)| n == name).map(|(_, v)| v).unwrap()
    }

    #[test]
    fn range_delete_is_rejected_without_consuming_a_sequence() {
        let disk = SimVfs::new(1);
        let opts = Options::small_for_tests();
        let db = <KernoDb as Engine>::open(disk.as_vfs(), Path::new("/db"), &opts).unwrap();
        let mut b = WriteBatch::new();
        b.put("a", "1").put("b", "2");
        assert_eq!(db.commit(&b).unwrap(), 1);
        // A full write buffer: the next commit that gets past the entry check
        // switches memtables (and WAL) before it encodes anything.
        let mut b = WriteBatch::new();
        b.put("big", vec![b'x'; opts.write_buffer_bytes]);
        assert_eq!(db.commit(&b).unwrap(), 2);
        let counters = ["wal_bytes", "commits", "memtable_rotations", "flushes"];
        let before: Vec<u64> = counters.iter().map(|c| stat(&db, c)).collect();
        let ops = disk.mutating_ops();

        let mut ranges = vec![WriteBatch::new(), WriteBatch::new(), WriteBatch::new()];
        ranges[0].delete_range("a", "z");
        ranges[1].put("c", "3").delete_range("a", "b").put("d", "4");
        ranges[2].delete_range("same", "same");
        for b in &ranges {
            let r = db.commit(b);
            assert!(matches!(&r, Err(Error::InvalidArgument(m)) if m == "delete_range not yet supported"), "got {r:?}");
        }
        // Nothing was written, switched, applied or numbered, and the handle
        // is healthy.
        assert_eq!(disk.mutating_ops(), ops, "a rejected batch touched the disk");
        assert_eq!(counters.iter().map(|c| stat(&db, c)).collect::<Vec<_>>(), before, "{counters:?}");
        let s = db.snapshot().unwrap();
        assert_eq!(s.seq(), 2);
        assert_eq!(s.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap().len(), 3);
        assert_eq!(s.get(b"c").unwrap(), None);
        let mut b = WriteBatch::new();
        b.put("e", "5");
        assert_eq!(db.commit(&b).unwrap(), 3, "the next commit gets the next sequence");
        assert_eq!(stat(&db, "memtable_rotations"), before[2] + 1, "the setup must make this commit switch");
        db.close().unwrap();
        let db = <KernoDb as Engine>::open(disk.as_vfs(), Path::new("/db"), &opts).unwrap();
        let s = db.snapshot().unwrap();
        assert_eq!(s.seq(), 3);
        assert_eq!(s.scan(Bound::Unbounded, Bound::Unbounded, 10).unwrap().len(), 4);
    }
}
