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
use kerno_core::{Durability, Error, Op, Result, WriteBatch};
use parking_lot::{Condvar, Mutex};

use crate::compaction::{self, BLOCK_SIZE, NUM_LEVELS, Plan, Tuning};
use crate::config::{AccessMode, Background, Config, Limits, SlowdownPolicy};
use crate::format::header::{HEADER_LEN, new_db_id};
use crate::format::limits::{MAX_FILE_NUMBER, MAX_SEQ};
use crate::format::manifest::ManifestState as RecoveredState;
use crate::format::props::{REASON_FLUSH, REASON_RECOVERY};
use crate::format::wal::{CanonicalCommit, CommitOp, canonicalize};
use crate::format::{End, wal::WalHeaderState};
use crate::iter::{InternalIterator, MergeIter};
use crate::manifest::{self, MANIFEST, MANIFEST_TMP, ManifestWriter, Snapshot as ManifestSnapshot};
use crate::memtable::Memtable;
use crate::metrics::{LevelMetrics, Metrics, ShapeMetrics};
use crate::sst::{Table, TableBuilder, TableCtx};
use crate::stats::{BackgroundIo, StallEpisode, StallReason, Stats, bump, bump_max, nanos};
use crate::version::{Levels, Version};
use crate::wal::{self, WalWriter, wal_name};

enum FileKind {
    Manifest,
    ManifestTmp,
    Lock,
    Wal(u64),
    Sst(u64),
    Other,
}

/// Classifies a directory entry (DIR-1, §2). Only canonical names
/// (`{n:06}.wal` / `{n:06}.sst` with `1 <= n <= 2^48-1`, as produced by
/// `wal_name` / `sst_name`) are engine files: a stray `1.wal` would
/// otherwise be opened and deleted as `000001.wal`. Everything else is
/// unknown: ignored and never deleted.
fn parse_name(name: &str) -> FileKind {
    let number = |s: &str| {
        let n: u64 = s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse().ok()).flatten()?;
        ((1..=MAX_FILE_NUMBER).contains(&n) && format!("{n:06}") == s).then_some(n)
    };
    if name == MANIFEST {
        FileKind::Manifest
    } else if name == MANIFEST_TMP {
        FileKind::ManifestTmp
    } else if name == "LOCK" {
        FileKind::Lock
    } else if let Some(n) = name.strip_suffix(".wal").and_then(number) {
        FileKind::Wal(n)
    } else if let Some(n) = name.strip_suffix(".sst").and_then(number) {
        FileKind::Sst(n)
    } else {
        FileKind::Other
    }
}

/// What a directory holds, by kind.
#[derive(Default)]
struct Listing {
    has_manifest: bool,
    /// Canonical WAL numbers, ascending.
    wals: Vec<u64>,
    /// Canonical SST numbers, ascending.
    ssts: Vec<u64>,
    /// Entries that are not engine files (`LOCK` and `MANIFEST.tmp` aside).
    foreign: bool,
}

/// Lists `dir`; a directory that does not exist is empty.
fn list_dir(vfs: &dyn Vfs, dir: &Path) -> Result<Listing> {
    let names = match vfs.list(dir) {
        Ok(names) => names,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut l = Listing::default();
    for name in names {
        match parse_name(&name) {
            FileKind::Manifest => l.has_manifest = true,
            FileKind::Wal(n) => l.wals.push(n),
            FileKind::Sst(n) => l.ssts.push(n),
            FileKind::ManifestTmp | FileKind::Lock => {}
            FileKind::Other => l.foreign = true,
        }
    }
    l.wals.sort_unstable();
    l.ssts.sort_unstable();
    Ok(l)
}

/// Why a directory without a MANIFEST cannot become a database (REC-2,
/// REC-3, DIR-2); `None` if it is empty or holds only `LOCK` and
/// `MANIFEST.tmp`, which a crash during creation leaves. WAL or SST files
/// without a MANIFEST are a damaged database. Anything else is not ours.
fn refuse_without_manifest(l: &Listing) -> Option<Error> {
    if !l.wals.is_empty() || !l.ssts.is_empty() {
        return Some(Error::Corruption("data files exist but the MANIFEST is missing".into()));
    }
    l.foreign.then(|| Error::InvalidArgument("not a Kerno database".into()))
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
    /// Which MANIFEST generation is on disk.
    writer: ManifestWriter,
}

struct Inner {
    vfs: Arc<dyn Vfs>,
    dir: PathBuf,
    durability: Durability,
    tuning: Tuning,
    /// Writers wait while this many memtables are waiting to be flushed.
    max_immutable: usize,
    limits: Limits,
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
fn write_memtable(ctx: &Arc<TableCtx>, mem: &Memtable, number: u64, reason: u8) -> Result<Table> {
    let mut b = TableBuilder::create(ctx, number, reason)?;
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

/// The files of `levels` as the MANIFEST records them.
fn level_metas(levels: &Levels) -> Vec<Vec<manifest::FileMeta>> {
    levels.iter().enumerate().map(|(l, files)| files.iter().map(|t| t.meta(l as u8)).collect()).collect()
}

struct Recovered {
    levels: Levels,
    last_seq: u64,
    log_number: u64,
    next_file: u64,
    wal: WalWriter,
    manifest: ManifestWriter,
}

/// Replays the live WALs (those numbered `>= log_number`, in order) into a
/// memtable. Returns it with the last sequence replayed.
///
/// Each log must start at the sequence after the previous one (WAL-R1), and
/// REC-10 decides which may end torn: every log before the newest one with
/// commits is intact; that one may end in a torn tail; the logs after it hold
/// no commits and have a broken header or a valid one that continues the
/// chain (a log created by a session that crashed before its first commit).
fn replay_live_wals(
    ctx: &TableCtx,
    listing: &Listing,
    db_id: [u8; 16],
    state: &RecoveredState,
) -> Result<(Memtable, u64)> {
    let mem = Memtable::new(0);
    // MAN-S1 keeps `last_seq` within 2^63-1: this cannot overflow.
    let mut expected = state.last_seq + 1;
    let mut scans = Vec::new();
    for &n in listing.wals.iter().filter(|&&n| n >= state.log_number) {
        let scan = wal::replay(&*ctx.vfs, &ctx.dir, n, db_id, expected, |seq, commit| mem.apply_commit(seq, commit))?;
        expected = scan.next_seq;
        scans.push((n, scan));
    }
    if let Some(newest_with_commits) = scans.iter().rposition(|(_, scan)| scan.frames > 0) {
        for (n, scan) in &scans[..newest_with_commits] {
            if !matches!(scan.header, WalHeaderState::Valid(_)) || scan.end == End::Torn {
                return Err(Error::Corruption(format!(
                    "{}: REC-10a: broken header or torn tail in a log that is followed by one with commits",
                    wal_name(*n)
                )));
            }
        }
    }
    for (_, scan) in &scans {
        bump(&ctx.stats.last_open_replayed_records, scan.commits);
        bump(&ctx.stats.last_open_replayed_bytes, scan.valid_len.saturating_sub(HEADER_LEN as u64));
    }
    Ok((mem, expected - 1))
}

/// Crash recovery (also run for a clean or brand-new database), on format
/// v1 files:
///
/// 1. open every SST the MANIFEST references (SST-O1..O7);
/// 2. replay the live WALs (see [`replay_live_wals`]);
/// 3. write the replayed commits to a new L0 table;
/// 4. write the next MANIFEST generation, pointing at a fresh WAL number,
///    so every old WAL is obsolete, then delete obsolete WALs and
///    unreferenced SSTs;
/// 5. create the new WAL.
///
/// Every file is read and validated before anything is written, so a
/// rejected directory is left as it was. When this returns, the recovered
/// state is durable on its own.
fn recover(ctx: &Arc<TableCtx>, listing: &Listing, loaded: manifest::Loaded) -> Result<Recovered> {
    let vfs = &*ctx.vfs;
    let dir = ctx.dir.as_path();
    let manifest::Loaded { db_id, state, mut writer } = loaded;
    let mut next_file = listing.wals.iter().chain(&listing.ssts).map(|n| n + 1).fold(state.next_file.max(1), u64::max);

    let present: HashSet<u64> = listing.ssts.iter().copied().collect();
    let mut levels: Levels = vec![Vec::new(); NUM_LEVELS];
    for (l, files) in state.levels.iter().enumerate() {
        for f in files {
            if !present.contains(&f.number) {
                return Err(Error::Corruption(format!("{}: referenced but missing", crate::sst::sst_name(f.number))));
            }
            levels[l].push(Arc::new(Table::open(ctx, f, state.last_seq)?));
        }
    }

    let (mem, last_seq) = replay_live_wals(ctx, listing, db_id, &state)?;
    if !mem.is_empty() {
        let table = write_memtable(ctx, &mem, next_file, REASON_RECOVERY)?;
        next_file += 1;
        vfs.sync_dir(dir)?;
        bump(&ctx.stats.flushes, 1);
        bump(&ctx.stats.flush_bytes_written, table.size);
        levels[0].insert(0, Arc::new(table));
    }

    let wal_number = next_file;
    next_file += 1;
    let metas = level_metas(&levels);
    writer.checkpoint(
        vfs,
        dir,
        &ManifestSnapshot { next_file, last_seq, log_number: wal_number, clean: false, levels: &metas },
    )?;
    let live: HashSet<u64> = levels.iter().flatten().map(|t| t.number).collect();
    // A snapshot that outlived the previous handle may delete one of these
    // obsolete files concurrently; a file that is already gone is fine.
    let remove = |path: &Path| match vfs.remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    };
    {
        let _phase = probe::phase("delete");
        for &n in &listing.wals {
            remove(&dir.join(wal_name(n)))?;
        }
        for &n in listing.ssts.iter().filter(|n| !live.contains(n)) {
            remove(&ctx.path(n))?;
        }
    }
    // The MANIFEST names the new log before it exists, as in M0: its
    // `base_seq` is the `last_seq` that MANIFEST already holds durably, so a
    // crash can never leave a log that starts after commits the recovered
    // state does not have. A crash before this point leaves no live log at
    // all, which recovery accepts.
    let wal = WalWriter::create(vfs, dir, wal_number, db_id, last_seq + 1)?;
    Ok(Recovered { levels, last_seq, log_number: wal_number, next_file, wal, manifest: writer })
}

/// The format-level form of one operation of a batch.
fn commit_op(op: &Op) -> CommitOp<'_> {
    match op {
        Op::Put { key, value } => CommitOp::Put { key, value },
        Op::Delete { key } => CommitOp::Delete { key },
        Op::DeleteRange { start, end } => CommitOp::DeleteRange { start, end },
    }
}

/// The canonical form of `batch` (WAL-C1).
fn canonical(batch: &WriteBatch) -> CanonicalCommit<&[u8]> {
    // The common commit, a single point operation, needs no sorting.
    match batch.ops() {
        [Op::Put { key, value }] => CanonicalCommit { ranges: Vec::new(), points: vec![(key, Some(value))] },
        [Op::Delete { key }] => CanonicalCommit { ranges: Vec::new(), points: vec![(key, None)] },
        ops => canonicalize(ops.iter().map(commit_op)),
    }
}

/// Checks a batch against the configured limits before anything is written
/// (`WriteBatch::validate` checks the fixed limits of the spikes, which the
/// engine's limits replace).
fn validate_batch(batch: &WriteBatch, limits: &Limits) -> Result<()> {
    let over = |what: &str, got: usize, max: usize| {
        Err(Error::InvalidArgument(format!("{what} of {got} exceeds the limit of {max}")))
    };
    if batch.len() > limits.max_batch_ops {
        return over("batch with operations", batch.len(), limits.max_batch_ops);
    }
    let key = |what: &str, k: &[u8]| {
        if k.len() > limits.max_key_len { over(what, k.len(), limits.max_key_len) } else { Ok(()) }
    };
    for op in batch.ops() {
        match op {
            Op::Put { key: k, value } => {
                key("key", k)?;
                if value.len() > limits.max_value_len {
                    return over("value", value.len(), limits.max_value_len);
                }
            }
            Op::Delete { key: k } => key("key", k)?,
            Op::DeleteRange { start, end } => {
                key("range start", start)?;
                key("range end", end)?;
                if start > end {
                    return Err(Error::InvalidArgument("delete_range with start > end".into()));
                }
            }
        }
    }
    // The keys and values alone are a lower bound of the encoding: reject
    // early what cannot fit before canonicalizing it.
    if batch.payload_bytes() > limits.max_batch_bytes {
        return over("batch of payload bytes", batch.payload_bytes(), limits.max_batch_bytes);
    }
    Ok(())
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

    /// Writes the next MANIFEST generation (the manifest lock is held by the
    /// caller, who owns `ms`).
    fn write_manifest(
        &self,
        ms: &mut ManifestState,
        levels: &Levels,
        last_seq: u64,
        log_number: u64,
        clean: bool,
    ) -> Result<()> {
        let metas = level_metas(levels);
        let snapshot = ManifestSnapshot {
            next_file: self.next_file.load(Ordering::SeqCst),
            last_seq,
            log_number,
            clean,
            levels: &metas,
        };
        ms.writer.checkpoint(&*self.vfs, &self.dir, &snapshot)?;
        bump(&self.stats.manifest_writes, 1);
        Ok(())
    }

    /// Publishes a version with new levels (and without the flushed
    /// memtable, if any). Called with the manifest lock held.
    fn install(&self, levels: Levels, flushed: Option<&Arc<Memtable>>) {
        bump_max(&self.stats.l0_files_max, levels[0].len() as u64);
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
        let start = Instant::now();
        validate_batch(batch, &self.limits)?;
        let commit = canonical(batch);
        if commit.encoded_len() > self.limits.max_batch_bytes {
            return Err(Error::InvalidArgument(format!(
                "batch encodes to {} bytes, over the limit of {}",
                commit.encoded_len(),
                self.limits.max_batch_bytes
            )));
        }
        let _phase = probe::phase("commit");
        let mut w = self.writer.lock();
        self.check_poisoned()?;
        if batch.is_empty() {
            return Ok(w.last_seq);
        }
        if w.last_seq >= MAX_SEQ {
            // Nothing is written and nothing is poisoned: reads still work.
            return Err(Error::InvalidArgument("commit sequence exhausted".into()));
        }
        let room_start = Instant::now();
        self.make_room(&mut w)?;
        let write_start = Instant::now();
        let seq = w.last_seq + 1;
        let sync = self.durability == Durability::Sync;
        let io = w.wal.append(seq, &commit);
        let sync_start = Instant::now();
        // Whether a flush or compaction was writing during the sync (H3).
        let background = || self.stats.background_active.load(Ordering::Relaxed) > 0;
        let busy = sync && background();
        let io = io.and_then(|len| if sync { w.wal.sync().map(|()| len) } else { Ok(len) });
        let frame_len = match io {
            Ok(len) => len,
            Err(e) => {
                self.poison(format!("WAL write failed: {e}"));
                return Err(e.into());
            }
        };
        let apply_start = Instant::now();
        let busy = busy || (sync && background());
        w.mem.apply_commit(seq, &commit);
        w.last_seq = seq;
        self.state.lock().last_seq = seq;
        let end = Instant::now();
        let s = &*self.stats;
        bump(&s.commits, 1);
        bump(&s.wal_bytes, frame_len as u64);
        s.commit_make_room.record_single_writer(nanos(room_start, write_start));
        s.commit_wal_write.record_single_writer(nanos(write_start, sync_start));
        if sync {
            bump(&s.wal_syncs, 1);
            let h = if busy { &s.commit_wal_sync_busy } else { &s.commit_wal_sync_quiet };
            h.record_single_writer(nanos(sync_start, apply_start));
        }
        s.commit_apply.record_single_writer(nanos(apply_start, end));
        s.commit_total.record_single_writer(nanos(start, end));
        Ok(seq)
    }

    /// Applies backpressure and switches to a new memtable when the active
    /// one is full (LevelDB's `MakeRoomForWrite`).
    fn make_room(&self, w: &mut Writer) -> Result<()> {
        if self.inline() {
            return self.make_room_inline(w);
        }
        let mut slowed = false;
        // Counted once, however many times the writer wakes up; timed until
        // the loop ends.
        let mut stall = StallEpisode::new(&self.stats);
        loop {
            self.check_poisoned()?;
            let mut st = self.state.lock();
            let l0 = st.version.levels[0].len();
            if !slowed
                && l0 >= self.tuning.l0_slowdown
                && let SlowdownPolicy::Sleep { micros } = self.tuning.slowdown
            {
                // Give compaction a moment instead of stopping hard later.
                slowed = true;
                let start = Instant::now();
                self.cv.wait_for(&mut st, Duration::from_micros(micros));
                let slept = nanos(start, Instant::now());
                bump(&self.stats.slowdown_commits, 1);
                bump(&self.stats.slowdown_nanos, slept);
                // L0 may reach the slowdown during a stall: count it once.
                stall.exclude(slept);
                continue;
            }
            if w.mem.approx_bytes() < self.tuning.write_buffer {
                return Ok(());
            }
            let reason = if st.version.imms.len() >= self.max_immutable {
                Some(StallReason::Memtable)
            } else if l0 >= self.tuning.l0_stop {
                Some(StallReason::L0)
            } else {
                None
            };
            if let Some(reason) = reason {
                stall.waiting(reason);
                if st.poisoned.is_none() {
                    self.cv.wait(&mut st);
                }
                continue;
            }
            drop(st);
            drop(stall);
            let start = Instant::now();
            let r = self.or_poison("memtable switch failed", || self.rotate(w));
            let took = nanos(start, Instant::now());
            bump(&self.stats.memtable_rotation_nanos, took);
            bump_max(&self.stats.memtable_rotation_nanos_max, took);
            return r;
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
        let wal = WalWriter::create(&*self.vfs, &self.dir, number, self.ctx.db_id, w.last_seq + 1)?;
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
        let _io = BackgroundIo::start(&self.stats);
        let start = Instant::now();
        let table = if mem.is_empty() {
            None
        } else {
            let number = self.next_file.fetch_add(1, Ordering::SeqCst);
            let table = write_memtable(&self.ctx, mem, number, REASON_FLUSH)?;
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
        self.write_manifest(&mut ms, &levels, last_seq, log_number, false)?;
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
        bump_max(&self.stats.flush_nanos_max, nanos(start, Instant::now()));
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
        while let Some(((plan, l0_files), running)) = self.wait_for_work(Job::Compaction, |st| {
            compaction::pick(&st.version.levels, &self.tuning, &pointers).map(|p| (p, st.version.levels[0].len()))
        }) {
            if plan.level > 0 && l0_files >= self.tuning.l0_slowdown {
                // L0 is slowing writers down, yet a deeper level won (H2).
                bump(&self.stats.compactions_while_l0_slowdown, 1);
            }
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
            let (last_seq, log_number) = (ms.last_seq, ms.log_number);
            self.write_manifest(&mut ms, &levels, last_seq, log_number, false)?;
            ms.levels = levels.clone();
            self.install(levels, None);
            bump(&self.stats.trivial_moves, 1);
            return Ok(true);
        }
        let _io = BackgroundIo::start(&self.stats);
        let start = Instant::now();
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
        // Only this thread changes L1 and deeper: what it holds now is what
        // `pick` saw.
        let next_level_files = ms.levels[plan.level + 1].len() as u64;
        let levels = compaction::apply(&ms.levels, plan, &outputs)?;
        let (last_seq, log_number) = (ms.last_seq, ms.log_number);
        self.write_manifest(&mut ms, &levels, last_seq, log_number, false)?;
        ms.levels = levels.clone();
        for t in plan.all_inputs() {
            t.mark_obsolete();
        }
        self.install(levels, None);
        drop(ms);
        let read: u64 = plan.all_inputs().map(|t| t.size).sum();
        let s = &*self.stats;
        bump(&s.compactions, 1);
        bump(&s.compaction_bytes_read, read);
        bump(&s.compaction_bytes_written, outputs.iter().map(|t| t.size).sum());
        bump_max(&s.compaction_nanos_max, nanos(start, Instant::now()));
        if plan.level == 0 {
            let l1_files = plan.next_inputs.len() as u64;
            bump(&s.l0_compactions, 1);
            bump(&s.l0_compaction_input_bytes, read);
            bump_max(&s.l0_compaction_input_bytes_max, read);
            bump(&s.l0_compaction_l1_files, l1_files);
            bump_max(&s.l0_compaction_l1_files_max, l1_files);
            bump(&s.l0_compaction_l1_files_present, next_level_files);
        }
        Ok(true)
    }

    // ---- shutdown ----

    /// Runs compactions in the calling thread until no level is over its
    /// target. Used by `close` (after the background threads stopped and
    /// the final flush), so that a database is never left with a growing
    /// L0 backlog: before, close abandoned the running compaction and then
    /// added one more L0 file, so many short sessions grew L0 without bound.
    /// If any compaction ran, the clean MANIFEST that `close` could not write
    /// before it (W-9) is written after it.
    fn compact_until_settled(&self) -> Result<()> {
        // The background threads are joined; compaction must not see the
        // shutdown flag and abandon its work.
        self.shutting_down.store(false, Ordering::SeqCst);
        let mut pointers = vec![Vec::new(); NUM_LEVELS];
        let mut compacted = false;
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
                    compacted = true;
                }
                None => break,
            }
        }
        if compacted {
            let _checkpoint = probe::phase("checkpoint");
            let mut ms = self.manifest.lock();
            let log_number = self.next_file.load(Ordering::SeqCst);
            let (levels, last_seq) = (ms.levels.clone(), ms.last_seq);
            self.write_manifest(&mut ms, &levels, last_seq, log_number, true)?;
            ms.log_number = log_number;
        }
        Ok(())
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
                let number = self.next_file.fetch_add(1, Ordering::SeqCst);
                tables.push(Arc::new(write_memtable(&self.ctx, mem, number, REASON_FLUSH)?));
            }
        }
        if !tables.is_empty() {
            self.vfs.sync_dir(&self.dir)?;
        }
        bump(&self.stats.flushes, tables.len() as u64);
        bump(&self.stats.flush_bytes_written, tables.iter().map(|t| t.size).sum());
        let mut ms = self.manifest.lock();
        let mut levels = ms.levels.clone();
        for t in tables {
            levels[0].insert(0, t);
        }
        // W-9: a compaction that is due runs before the CLEAN checkpoint, so
        // this MANIFEST is the clean one only when none is (the pick is the
        // one `compact_until_settled` starts with); otherwise it only needs
        // no WAL, and `compact_until_settled` writes the clean one.
        let clean = !self.compaction_due(&levels);
        let log_number =
            if clean { self.next_file.load(Ordering::SeqCst) } else { self.next_file.fetch_add(1, Ordering::SeqCst) };
        {
            let _checkpoint = probe::phase("checkpoint");
            self.write_manifest(&mut ms, &levels, w.last_seq, log_number, clean)?;
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
    /// writer re-checks its condition and waits again; the stall episode is
    /// not counted again.
    pub(crate) fn wake_waiters(&self) {
        self.inner.cv.notify_all();
    }

    /// The engine's own counters.
    pub(crate) fn raw_stats(&self) -> &Stats {
        &self.inner.stats
    }
}

fn no_database(dir: &Path) -> Error {
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
/// directory that holds `dir` and the one that holds each directory created
/// now (DIR-3: only when a database is created, never when one is opened): entries this open made, so failing to
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
    /// Opens the database stored in `dir` (creating it if `dir` holds none
    /// and `cfg.create_if_missing` is set), running crash recovery if the
    /// previous handle was not closed cleanly.
    ///
    /// Everything that can reject the directory (a foreign one, an unknown
    /// version or feature, damage) is checked before the first write. Only
    /// creating the database directory and its `LOCK` come before the lock.
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &Config) -> Result<KernoDb> {
        cfg.validate()?;
        if cfg.access == AccessMode::ReadOnly {
            return Err(Error::InvalidArgument("read-only mode not implemented yet".into()));
        }
        // Without a MANIFEST there is no database to open: nothing is
        // created, locked or changed unless this open may create one (DIR-2).
        let first = list_dir(&*vfs, dir)?;
        let creating = !first.has_manifest;
        if creating {
            if let Some(e) = refuse_without_manifest(&first) {
                return Err(e);
            }
            if !cfg.create_if_missing {
                return Err(no_database(dir));
            }
        }
        let _phase = probe::phase(if creating { "create" } else { "session" });
        if creating {
            let created = missing_dirs(&*vfs, dir)?;
            vfs.create_dir_all(dir)?;
            sync_dir_entries(&*vfs, dir, &created, true)?;
        }
        let lock = vfs.lock(&dir.join("LOCK")).map_err(|e| match e.kind() {
            io::ErrorKind::WouldBlock => Error::Locked,
            _ => Error::Io(e),
        })?;
        // Under the lock the directory cannot change: this listing is the
        // one recovery works from.
        let mut listing = list_dir(&*vfs, dir)?;
        if !listing.has_manifest {
            if let Some(e) = refuse_without_manifest(&listing) {
                return Err(e);
            }
            if !cfg.create_if_missing {
                return Err(no_database(dir));
            }
            manifest::create(&*vfs, dir, cfg.test.db_id.unwrap_or_else(new_db_id))?;
            listing.has_manifest = true;
        }
        let loaded = manifest::read(&*vfs, dir)?;
        let stats = Arc::new(Stats::default());
        let ctx = Arc::new(
            TableCtx::new(vfs.clone(), dir.to_path_buf(), cfg.cache_bytes, stats.clone(), BLOCK_SIZE)
                .with_bytes_per_sync(cfg.compaction.bytes_per_sync)
                .with_db_id(loaded.db_id),
        );
        let r = recover(&ctx, &listing, loaded)?;
        // The L0 found at open counts as seen by this handle.
        bump_max(&stats.l0_files_max, r.levels[0].len() as u64);
        let mem = Arc::new(Memtable::new(r.wal.number()));
        let version = Arc::new(Version { mem: mem.clone(), imms: Vec::new(), levels: r.levels.clone() });
        let inner = Arc::new(Inner {
            vfs,
            dir: dir.to_path_buf(),
            durability: cfg.durability,
            tuning: Tuning::new(cfg),
            max_immutable: cfg.max_immutable_memtables,
            limits: cfg.limits,
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
            manifest: Mutex::new(ManifestState {
                levels: r.levels,
                last_seq: r.last_seq,
                log_number: r.log_number,
                writer: r.manifest,
            }),
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

    /// The metrics of this handle now (phase1-design.md §3).
    pub fn metrics(&self) -> Metrics {
        let inner = &*self.inner;
        let (v, last_seq) = {
            let st = inner.state.lock();
            (st.version.clone(), st.last_seq)
        };
        let shape = ShapeMetrics {
            levels: v
                .levels
                .iter()
                .map(|l| LevelMetrics { files: l.len() as u64, bytes: l.iter().map(|t| t.size).sum() })
                .collect(),
            l0_depth: compaction::l0_depth(&v.levels[0]) as u64,
            l0_files_max: 0,
            memtable_bytes: v.mem.approx_bytes() as u64,
            immutable_memtables: v.imms.len() as u64,
            last_seq,
            block_cache_bytes: inner.ctx.cache.weight(),
        };
        inner.stats.metrics(shape, compaction::pending_bytes(&v.levels, &inner.tuning))
    }

    /// Closes the handle like `Engine::close` and returns its metrics after
    /// the close (its flushes and compactions included). Until `close`
    /// returns its own report (WP05), this is how tests observe the work of
    /// a close.
    #[doc(hidden)]
    pub fn close_with_metrics(self) -> Result<Metrics> {
        self.close_inner()?;
        Ok(self.metrics())
    }

    fn close_inner(&self) -> Result<()> {
        self.stop_threads();
        self.inner.check_poisoned()?;
        self.inner.or_poison("close failed", || {
            self.inner.close()?;
            self.inner.compact_until_settled()
        })
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
        self.close_inner()
    }

    /// [`KernoDb::metrics`], flattened ([`Metrics::flatten`]).
    fn stats(&self) -> Vec<(String, u64)> {
        self.metrics().flatten()
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
