//! `KernoDb`: recovery, the commit path, background flush/compaction
//! threads (or their inline equivalent), snapshots and clean shutdown.

use std::any::Any;
use std::io;
use std::ops::Bound;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use kerno_core::contract::{Engine, Options, RangeDeleteEngine, Snapshot, SyncEngine, TestHooks};
use kerno_core::probe;
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error, Op, Result, WriteBatch};
use parking_lot::{Condvar, Mutex};

use crate::commit_queue::{CommitQueue, Request, Wake, replicate};
use crate::compaction::{self, BLOCK_SIZE, Busy, Claim, NUM_LEVELS, Plan, RangeStep, RateController, Tuning};
use crate::config::{AccessMode, Background, CloseBudget, Config, Limits, ReadOnlyReason, SlowdownPolicy};
use crate::deleter::Deleter;
use crate::flush;
use crate::format::header::new_db_id;
use crate::format::limits::MAX_SEQ;
use crate::format::manifest::{Edit, FileMeta, REASON_BACKGROUND, REASON_CLOSE, REASON_DIRECTED, REASON_TRIVIAL_MOVE};
use crate::format::props::REASON_FLUSH;
use crate::format::wal::{CanonicalCommit, CommitOp, canonicalize};
use crate::manifest::{self, CHECKPOINT_FLOOR, MANIFEST, ManifestWriter, Snapshot as ManifestSnapshot};
use crate::memtable::Memtable;
use crate::metrics::{LevelMetrics, Metrics, ShapeMetrics};
use crate::recovery::{self, FileKind, list_dir, parse_name, refuse_without_manifest};
use crate::sst::{Table, TableCtx};
use crate::stats::{BackgroundIo, StallEpisode, StallReason, Stats, bump, bump_max, nanos};
use crate::version::{Levels, Version, level_metas};
use crate::wal::{self, SPARE_WAL, WalWriter, wal_name};

/// Owned by the (single) writer: the active WAL and memtable.
struct Writer {
    wal: WalWriter,
    mem: Arc<Memtable>,
    last_seq: u64,
    /// Inline mode: round-robin position per level for the compactions run
    /// under the writer lock (largest key of the last compaction out of it).
    /// The background compactor keeps its own.
    pointers: Vec<Vec<u8>>,
    /// The rate of `SlowdownPolicy::Rate`.
    rate: RateController,
}

/// Round-robin positions that choose nothing: `compaction::pick` decides
/// whether a compaction is due regardless of them.
static NO_POINTERS: [Vec<u8>; NUM_LEVELS] = [const { Vec::new() }; NUM_LEVELS];

/// Published state, read by snapshots and background threads.
struct State {
    version: Arc<Version>,
    /// Last commit visible to new snapshots. In `Sync` mode it is published
    /// only after the `fsync` of the frame that holds it (W-12, I-16).
    last_seq: u64,
    /// Every commit up to this sequence survives a power loss. Equal to
    /// `last_seq` in `Sync` mode; in `Buffered` mode it follows `sync()` and
    /// WAL rotations.
    durable_seq: u64,
    poisoned: Option<String>,
    shutdown: bool,
    /// A background thread is working on a flush or a compaction (including
    /// the deletions that follow it). See `Inner::wait_idle`.
    flush_running: bool,
    /// The writer asked the flush thread to prepare the spare WAL.
    spare_wanted: bool,
    /// Compactions in progress (at most `max_background_compactions`).
    compactions_running: usize,
    /// What they read and write, for choosing the next one.
    busy: Busy,
    /// Round-robin position per level for the compaction threads (largest
    /// key of the last compaction chosen out of it).
    pointers: Vec<Vec<u8>>,
}

/// The kinds of background work.
#[derive(Clone, Copy)]
enum Job {
    Flush,
    Compaction,
}

/// What the flush thread was woken for.
enum FlusherWork {
    Flush(Arc<Memtable>),
    /// Prepare the spare WAL (`Config::wal_preallocate`).
    Spare,
}

/// Marks a background job as running until dropped (also on a panic).
struct Running<'a> {
    inner: &'a Inner,
    job: Job,
    /// The compaction's claim on its inputs and output range.
    claim: Option<Claim>,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let mut st = self.inner.state.lock();
        match self.job {
            Job::Flush => st.flush_running = false,
            Job::Compaction => st.compactions_running -= 1,
        }
        if let Some(claim) = &self.claim {
            st.busy.release(claim);
        }
        drop(st);
        self.inner.cv.notify_all();
    }
}

/// Durable LSM shape as of the last MANIFEST record written. Held while
/// computing and appending a record, so that flushes and compactions apply
/// their edits one at a time.
struct ManifestState {
    levels: Levels,
    last_seq: u64,
    log_number: u64,
    /// The MANIFEST on disk: its generation and the append position.
    writer: ManifestWriter,
}

struct Inner {
    vfs: Arc<dyn Vfs>,
    dir: PathBuf,
    durability: Durability,
    tuning: Tuning,
    /// Compaction threads (`max_background_compactions`).
    max_compactions: usize,
    close_budget: CloseBudget,
    /// Writers wait while this many memtables are waiting to be flushed.
    max_immutable: usize,
    limits: Limits,
    /// Background threads, or flushes and compactions inline.
    background: Background,
    ctx: Arc<TableCtx>,
    stats: Arc<Stats>,
    /// Why this handle cannot write, if it cannot (v1.md §9.5).
    read_only: Option<ReadOnlyReason>,
    /// The files recovery found that nothing needs: deleted by a read-write
    /// open, left alone by a read-only one.
    orphans: Vec<String>,
    next_file: AtomicU64,
    /// Bytes a WAL is preallocated with (`Config::wal_preallocate`); 0 when
    /// off.
    wal_prealloc: u64,
    /// Whether the spare WAL (`wal::SPARE_WAL`) is complete and durable. The
    /// lock is held while the spare is prepared and while a rotation takes
    /// it, so that neither sees the other's half-done file; a rotation that
    /// finds it held does not wait for it (leaf lock: nothing is taken under
    /// it).
    spare: Mutex<bool>,
    /// The active memtable already asked for the spare (reset by the
    /// rotation), so that it is asked for once per memtable.
    spare_requested: AtomicBool,
    /// The write slot (D-03): a blind commit holds it only to enqueue.
    txn_gate: Mutex<()>,
    queue: CommitQueue<Work>,
    // Lock order: txn_gate -> queue -> writer -> manifest -> state (the
    // queue's own lock is never held while taking another).
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

/// What [`KernoDb::close`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseReport {
    /// Wall-clock time of the close.
    pub duration: Duration,
    /// Bytes of the tables written from memtables.
    pub flushed_bytes: u64,
    /// Bytes read and written by the compactions of the close.
    pub compacted_bytes: u64,
    /// The [`CloseBudget`] stopped compaction work that was still due.
    pub budget_exhausted: bool,
    /// L0 files left.
    pub l0_files: u64,
    /// L0 depth left (see [`L0Metric::Depth`](crate::config::L0Metric::Depth)).
    pub l0_depth: u64,
}

/// What [`KernoDb::compact_range`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompactRangeReport {
    /// Files read and replaced by the compactions (files that only moved to
    /// the next level are not counted).
    pub files_rewritten: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    /// Replaced files that are still on disk because a snapshot (or a
    /// version a snapshot holds) refers to them, and their size. They go
    /// when the last of those goes: until then they may hold what the range
    /// deletions covered.
    pub pinned_files: u64,
    pub pinned_bytes: u64,
}

/// What a compaction was chosen for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// The score of a level.
    Background,
    /// The close of the handle: its settings (`close_subcompactions`, no
    /// intermediate syncs).
    Close,
    /// `compact_range`.
    Directed,
}

/// The bytes a compaction read and wrote.
#[derive(Clone, Copy, Default)]
struct Compacted {
    read: u64,
    written: u64,
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

/// What a queued request asks the leader for.
enum Work {
    /// A non-empty commit, already canonical, and when its call began.
    Commit { commit: CanonicalCommit<Vec<u8>>, started: Instant },
    /// `sync()`: make everything acknowledged so far durable.
    Sync,
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

    /// Makes `edit` durable: appends it to the MANIFEST (W-3) or, when a
    /// checkpoint is due (W-10), writes the state after the edit (`after`,
    /// `last_seq`, `log_number`) as a new generation instead. The caller
    /// holds the manifest lock (it owns `ms`) and updates `ms` afterwards.
    fn log_edit(
        &self,
        ms: &mut ManifestState,
        edit: Edit,
        after: &Levels,
        last_seq: u64,
        log_number: u64,
    ) -> Result<()> {
        if ms.writer.checkpoint_due() {
            return self.checkpoint_state(ms, after, last_seq, log_number, false);
        }
        ms.writer.append(edit)?;
        let s = &self.stats;
        bump(&s.manifest_records, 1);
        bump(&s.manifest_writes, 1);
        s.manifest_bytes.store(ms.writer.len(), Ordering::Relaxed);
        Ok(())
    }

    /// Replaces the MANIFEST with a new generation holding one `SNAPSHOT` of
    /// the given state (W-10); `clean` marks a closed database, which
    /// requires `log_number == next_file`.
    fn checkpoint_state(
        &self,
        ms: &mut ManifestState,
        levels: &Levels,
        last_seq: u64,
        log_number: u64,
        clean: bool,
    ) -> Result<()> {
        let _phase = probe::phase("checkpoint");
        let metas = level_metas(levels);
        let snapshot = ManifestSnapshot {
            next_file: self.next_file.load(Ordering::SeqCst),
            last_seq,
            log_number,
            clean,
            levels: &metas,
        };
        ms.writer.checkpoint(&*self.vfs, &self.dir, &snapshot)?;
        let s = &self.stats;
        bump(&s.manifest_checkpoints, 1);
        bump(&s.manifest_writes, 1);
        s.manifest_bytes.store(ms.writer.len(), Ordering::Relaxed);
        Ok(())
    }

    /// The WAL number the oldest live log has once `mem`, the oldest
    /// immutable memtable, is flushed: that of the next-oldest memtable, or
    /// of the active one (v1.md §8.3, tag 3: the exact oldest live WAL).
    fn oldest_wal_after(&self, mem: &Arc<Memtable>) -> u64 {
        let st = self.state.lock();
        let v = &st.version;
        match v.imms.iter().position(|m| Arc::ptr_eq(m, mem)) {
            Some(i) if i > 0 => v.imms[i - 1].wal_number(),
            _ => v.mem.wal_number(),
        }
    }

    fn check_writable(&self) -> Result<()> {
        match &self.read_only {
            None => Ok(()),
            Some(reason) => Err(Error::ReadOnly(match reason {
                ReadOnlyReason::Requested => "the handle was opened read-only".into(),
                ReadOnlyReason::UnknownRoCompat { file, bits } => {
                    format!("{file} has ro_compat bits {bits:#x} that this engine does not know")
                }
            })),
        }
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
        self.check_writable()?;
        let started = Instant::now();
        validate_batch(batch, &self.limits)?;
        let commit = canonical(batch);
        let bytes = commit.encoded_len();
        if bytes > self.limits.max_batch_bytes {
            return Err(Error::InvalidArgument(format!(
                "batch encodes to {bytes} bytes, over the limit of {}",
                self.limits.max_batch_bytes
            )));
        }
        self.check_poisoned()?;
        if batch.is_empty() {
            return Ok(self.state.lock().last_seq);
        }
        // ponytail: the commit is copied into the request so the leader can
        // read it while this thread waits; borrow it for a leader's own
        // commit if profiling ever shows the copy.
        self.submit(Work::Commit { commit: commit.to_owned_commit(), started }, bytes)
    }

    /// [`SyncEngine::sync`]: makes every commit acknowledged so far durable
    /// and returns the durable sequence.
    fn sync(&self) -> Result<u64> {
        self.check_writable()?;
        self.check_poisoned()?;
        self.submit(Work::Sync, 0)
    }

    /// Queues `work`, leads the group it ends up in or waits for the leader
    /// that serves it, and returns its result.
    fn submit(&self, work: Work, bytes: usize) -> Result<u64> {
        let request = Request::new(work, bytes);
        let queued = {
            let _gate = self.txn_gate.lock();
            self.queue.enqueue(&request)?
        };
        bump_max(&self.stats.queued_bytes_max, queued.queued_bytes as u64);
        if !queued.leader
            && let Wake::Done(result) = request.wait()
        {
            return result;
        }
        self.lead();
        match request.wait() {
            Wake::Done(result) => result,
            Wake::Lead => unreachable!("a leader that completed its group is not promoted again"),
        }
    }

    /// Runs the group at the front of the queue, whose first request is the
    /// caller's, and completes it. A panic in the leader poisons the handle
    /// and fails the group, so nobody waits for a leader that is gone.
    fn lead(&self) {
        let mut w = self.writer.lock();
        let group = self.queue.take_group();
        let _phase =
            probe::phase(if group.iter().all(|r| matches!(r.payload(), Work::Sync)) { "sync" } else { "commit" });
        let (results, poison) =
            catch_unwind(AssertUnwindSafe(|| self.run_group(&mut w, &group))).unwrap_or_else(|_| {
                const MSG: &str = "commit leader panicked";
                self.poison(MSG.into());
                (group.iter().map(|_| Err(Error::Io(io::Error::other(MSG)))).collect(), Some(MSG.into()))
            });
        drop(w);
        self.queue.complete(&group, results, poison);
    }

    /// W-11/W-12 for one group, under the writer lock: one memtable switch
    /// check, consecutive sequences, one frame, one `write_at`, one `fsync`
    /// (in `Sync` mode, or when a `sync()` is in the group and something is
    /// unsynced), the batches applied in sequence order, and the sequences
    /// published once, after the `fsync`. Returns one result per member, and
    /// the poison message if the handle must fail for good.
    fn run_group(
        &self,
        w: &mut Writer,
        group: &[crate::sync::Arc<Request<Work>>],
    ) -> (Vec<Result<u64>>, Option<String>) {
        let s = &*self.stats;
        let started = Instant::now();
        let fail_all = |e: Error, poison: Option<String>| (group.iter().map(|_| Err(replicate(&e))).collect(), poison);
        let commits: Vec<&CanonicalCommit<Vec<u8>>> = group
            .iter()
            .filter_map(|r| match r.payload() {
                Work::Commit { commit, .. } => Some(commit),
                Work::Sync => None,
            })
            .collect();
        for r in group {
            if matches!(r.payload(), Work::Commit { .. }) {
                s.commit_queue.record_single_writer(nanos(r.enqueued(), started));
            }
        }
        if let Err(e) = self.check_poisoned() {
            return fail_all(e, None);
        }
        let wants_sync = group.iter().any(|r| matches!(r.payload(), Work::Sync));
        // Sequences are exhausted before they wrap: the commits that do not
        // fit are refused, nothing is poisoned and reads still work.
        let room = usize::try_from(MAX_SEQ - w.last_seq).unwrap_or(usize::MAX);
        let accepted = commits.len().min(room);
        let first = w.last_seq + 1;
        let mut frame_len = 0;
        let mut synced = self.durability == Durability::Sync;
        if accepted > 0 {
            let room_start = Instant::now();
            // The rate slowdown charges the group's encoded bytes.
            let group_bytes = commits[..accepted].iter().map(|c| c.encoded_len() as u64).sum();
            if let Err(e) = self.make_room(w, group_bytes) {
                return fail_all(e, None);
            }
            let write_start = Instant::now();
            let io = w.wal.append_group(first, &commits[..accepted]);
            let sync_start = Instant::now();
            // Whether a flush or compaction was writing during the sync (H3).
            let background = || s.background_active.load(Ordering::Relaxed) > 0;
            let fsync = synced || wants_sync;
            let busy = fsync && background();
            let io = io.and_then(|len| if fsync { w.wal.sync().map(|()| len) } else { Ok(len) });
            frame_len = match io {
                Ok(len) => len,
                Err(e) => {
                    let msg = format!("WAL write failed: {e}");
                    self.poison(msg.clone());
                    return fail_all(Error::Io(e), Some(msg));
                }
            };
            synced |= wants_sync;
            let apply_start = Instant::now();
            let busy = busy || (fsync && background());
            for (i, commit) in commits[..accepted].iter().enumerate() {
                w.mem.apply_commit(first + i as u64, commit);
            }
            s.commit_make_room.record_single_writer(nanos(room_start, write_start));
            s.commit_wal_write.record_single_writer(nanos(write_start, sync_start));
            if fsync {
                bump(&s.wal_syncs, 1);
                let h = if busy { &s.commit_wal_sync_busy } else { &s.commit_wal_sync_quiet };
                h.record_single_writer(nanos(sync_start, apply_start));
            }
            s.commit_apply.record_single_writer(nanos(apply_start, Instant::now()));
        } else if wants_sync {
            // Only `sync()` requests (or commits refused for the sequence
            // limit): an `fsync` only if a frame is unsynced.
            if w.wal.needs_sync() {
                let sync_start = Instant::now();
                if let Err(e) = w.wal.sync() {
                    let msg = format!("WAL sync failed: {e}");
                    self.poison(msg.clone());
                    return fail_all(Error::Io(e), Some(msg));
                }
                bump(&s.wal_syncs, 1);
                s.commit_wal_sync_quiet.record_single_writer(nanos(sync_start, Instant::now()));
            }
            synced = true;
        }
        let last = first - 1 + accepted as u64;
        w.last_seq = last;
        {
            // Published together, once: a reader never sees a sequence that
            // a power loss could take back (Sync mode).
            let mut st = self.state.lock();
            st.last_seq = last;
            if synced {
                st.durable_seq = last;
            }
        }
        let end = Instant::now();
        if accepted > 0 {
            bump(&s.commits, accepted as u64);
            bump(&s.commit_groups, 1);
            bump_max(&s.group_commits_max, accepted as u64);
            bump(&s.wal_bytes, frame_len as u64);
        }
        let mut next = 0;
        let results = group
            .iter()
            .map(|r| match r.payload() {
                Work::Commit { started, .. } => {
                    next += 1;
                    if next > accepted {
                        return Err(Error::InvalidArgument("commit sequence exhausted".into()));
                    }
                    s.commit_total.record_single_writer(nanos(*started, end));
                    Ok(first + next as u64 - 1)
                }
                Work::Sync => Ok(last),
            })
            .collect();
        (results, None)
    }

    /// Applies backpressure and switches to a new memtable when the active
    /// one is full (LevelDB's `MakeRoomForWrite`).
    fn make_room(&self, w: &mut Writer, batch_bytes: u64) -> Result<()> {
        if self.inline() {
            return self.make_room_inline(w);
        }
        let mut slowed = false;
        let mut observed = false;
        // Counted once, however many times the writer wakes up; timed until
        // the loop ends.
        let mut stall = StallEpisode::new(&self.stats);
        loop {
            self.check_poisoned()?;
            let mut st = self.state.lock();
            let l0 = self.tuning.l0_metric(&st.version.levels[0]);
            if !observed {
                observed = true;
                w.rate.observe(l0, self.tuning.l0_slowdown);
            }
            if !slowed && l0 >= self.tuning.l0_slowdown && self.tuning.slowdown != SlowdownPolicy::Off {
                // Give compaction a moment instead of stopping hard later.
                slowed = true;
                let start = Instant::now();
                match self.tuning.slowdown {
                    SlowdownPolicy::Sleep { micros } => {
                        self.cv.wait_for(&mut st, Duration::from_micros(micros));
                    }
                    _ => {
                        // Until the delay is served or L0 is relieved.
                        let deadline = start + w.rate.delay(batch_bytes);
                        while st.poisoned.is_none()
                            && !st.shutdown
                            && !self.cv.wait_until(&mut st, deadline).timed_out()
                            && self.tuning.l0_metric(&st.version.levels[0]) >= self.tuning.l0_slowdown
                        {
                        }
                    }
                }
                let slept = nanos(start, Instant::now());
                bump(&self.stats.slowdown_commits, 1);
                bump(&self.stats.slowdown_nanos, slept);
                // L0 may reach the slowdown during a stall: count it once.
                stall.exclude(slept);
                continue;
            }
            if !w.mem.is_full(self.tuning.write_buffer) {
                if self.spare_due(w.mem.approx_bytes()) {
                    st.spare_wanted = true;
                    drop(st);
                    self.cv.notify_all();
                }
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
        if !w.mem.is_full(self.tuning.write_buffer) {
            if self.spare_due(w.mem.approx_bytes()) {
                let _phase = probe::phase("wal_spare");
                self.prepare_spare_if_absent();
            }
            return Ok(());
        }
        self.or_poison("memtable switch failed", || self.rotate(w))?;
        self.flush_inline()?;
        while self.compact_inline(w)? {}
        Ok(())
    }

    /// Whether the active memtable, `bytes` full, should have the spare WAL
    /// prepared now (once per memtable): half way to the rotation leaves the
    /// preparation time to finish before the next log is needed, and keeps a
    /// handle that is only read or barely written from creating one.
    fn spare_due(&self, bytes: usize) -> bool {
        self.wal_prealloc > 0
            && bytes >= self.tuning.write_buffer / 2
            && !self.spare_requested.swap(true, Ordering::SeqCst)
    }

    /// Prepares a new spare unless one is ready (or being prepared).
    fn prepare_spare_if_absent(&self) {
        if let Some(mut ready) = self.spare.try_lock()
            && !*ready
        {
            *ready = self.make_spare(None);
        }
    }

    fn rotate(&self, w: &mut Writer) -> Result<()> {
        let _phase = probe::phase("rotate");
        self.spare_requested.store(false, Ordering::SeqCst);
        w.mem.set_wal_len(w.wal.written());
        if self.durability == Durability::Buffered {
            // Every older log must be complete before a newer one exists,
            // so that recovery's "only the newest log may be torn" holds.
            w.wal.sync()?;
            // ...and everything acknowledged so far is durable.
            self.state.lock().durable_seq = w.last_seq;
        }
        let number = self.next_file.fetch_add(1, Ordering::SeqCst);
        // Without a spare the new log grows like any other: waiting here for
        // a preallocation would stall the commit that rotates.
        let mut spare = self.spare.try_lock();
        let wal = match spare.as_deref_mut() {
            Some(ready @ true) => {
                *ready = false;
                bump(&self.stats.wal_recycled, 1);
                WalWriter::create_from_spare(&*self.vfs, &self.dir, number, self.ctx.db_id, w.last_seq + 1)?
            }
            _ => WalWriter::create(&*self.vfs, &self.dir, number, self.ctx.db_id, w.last_seq + 1, 0)?,
        };
        drop(spare);
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
    /// job counts as running until the returned guard is dropped. For a
    /// compaction, `next` also returns the claim on what it will touch, taken
    /// in the same critical section that chose it.
    fn wait_for_work<T>(
        &self,
        job: Job,
        mut next: impl FnMut(&mut State) -> Option<(T, Option<Claim>)>,
    ) -> Option<(T, Running<'_>)> {
        let mut st = self.state.lock();
        loop {
            if st.shutdown || st.poisoned.is_some() {
                return None;
            }
            if let Some((work, claim)) = next(&mut st) {
                match job {
                    Job::Flush => st.flush_running = true,
                    Job::Compaction => {
                        st.compactions_running += 1;
                        bump_max(&self.stats.compactions_concurrent_max, st.compactions_running as u64);
                    }
                }
                return Some((work, Running { inner: self, job, claim }));
            }
            self.cv.wait(&mut st);
        }
    }

    /// Gets rid of the WAL `number`, which the MANIFEST no longer needs: with
    /// preallocation and no spare yet it becomes the spare (zeroed, see
    /// [`wal::prepare_spare`]), otherwise it is deleted. Best effort either
    /// way: the spare only saves work later, and a leftover log below
    /// `log_number` is deleted by the next recovery.
    fn retire_wal(&self, number: u64, written: u64) {
        if self.wal_prealloc > 0 {
            let mut ready = self.spare.lock();
            if !*ready {
                *ready = self.make_spare(Some((number, written)));
                if *ready {
                    return;
                }
            }
        }
        let _ = self.vfs.remove_file(&self.dir.join(wal_name(number)));
    }

    /// Prepares the spare WAL from the retired log `retired`, or from nothing,
    /// and returns whether it is ready (the caller holds `spare`). A failure (a full disk, say) leaves
    /// no spare behind and is not an error of the engine: rotations create
    /// their logs as before.
    fn make_spare(&self, retired: Option<(u64, u64)>) -> bool {
        let abort = || self.shutting_down.load(Ordering::SeqCst);
        match wal::prepare_spare(&*self.vfs, &self.dir, retired, self.wal_prealloc, &abort) {
            Ok(ready) => ready,
            Err(_) => {
                // The half-made spare is useless, and it holds the space the
                // failed write wanted.
                let _ = self.vfs.remove_file(&self.dir.join(SPARE_WAL));
                false
            }
        }
    }

    fn flusher(&self) {
        while let Some((work, _running)) = self.wait_for_work(Job::Flush, |st| {
            if let Some(mem) = st.version.imms.last() {
                return Some((FlusherWork::Flush(mem.clone()), None));
            }
            std::mem::take(&mut st.spare_wanted).then_some((FlusherWork::Spare, None))
        }) {
            match work {
                FlusherWork::Flush(mem) => {
                    if self.or_poison("flush failed", || self.flush(&mem)).is_err() {
                        return;
                    }
                }
                FlusherWork::Spare => {
                    let _phase = probe::phase("wal_spare");
                    self.prepare_spare_if_absent();
                }
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

    /// The files of the level L0 compacts into (L1, or L2 when
    /// `dynamic_base_level` skips an empty L1) that a flush cuts around
    /// (`flush_split`); none if it is off.
    fn l1_for_split(&self) -> Vec<Arc<Table>> {
        if !self.tuning.flush_split {
            return Vec::new();
        }
        let st = self.state.lock();
        let base = compaction::l0_output_level(&st.version.levels, &self.tuning, &st.busy);
        st.version.levels[base].clone()
    }

    /// [`flush::write_memtable`] with file numbers from this engine.
    fn write_memtable(&self, mem: &Memtable, l1: &[Arc<Table>]) -> Result<Vec<Arc<Table>>> {
        let mut number = || self.next_file.fetch_add(1, Ordering::SeqCst);
        Ok(flush::write_memtable(&self.ctx, mem, REASON_FLUSH, l1, &mut number)?.into_iter().map(Arc::new).collect())
    }

    /// Writes the oldest immutable memtable to L0 and retires its WAL.
    fn flush(&self, mem: &Arc<Memtable>) -> Result<()> {
        let _phase = probe::phase("flush");
        let _io = BackgroundIo::start(&self.stats);
        let start = Instant::now();
        let tables = if mem.is_empty() {
            Vec::new()
        } else {
            let tables = self.write_memtable(mem, &self.l1_for_split())?;
            self.vfs.sync_dir(&self.dir)?;
            bump(&self.stats.flush_bytes_written, tables.iter().map(|t| t.size).sum());
            tables
        };
        let mut ms = self.manifest.lock();
        let mut levels = ms.levels.clone();
        let files: Vec<FileMeta> = tables.iter().map(|t| t.meta(0)).collect();
        // The pieces of one flush are key-disjoint: they go in as a group.
        levels[0].splice(0..0, tables);
        let last_seq = ms.last_seq.max(mem.max_seq());
        let log_number = ms.log_number.max(self.oldest_wal_after(mem));
        let edit =
            Edit::Flush { next_file: self.next_file.load(Ordering::SeqCst), last_seq, log_number, reason: None, files };
        self.log_edit(&mut ms, edit, &levels, last_seq, log_number)?;
        ms.levels = levels.clone();
        ms.last_seq = last_seq;
        ms.log_number = log_number;
        self.install(levels, Some(mem));
        drop(ms);
        // The MANIFEST no longer needs this log (W-5; best effort: a leftover
        // log below `log_number` is deleted by the next recovery).
        let _delete = probe::phase("delete");
        self.retire_wal(mem.wal_number(), mem.wal_len());
        bump(&self.stats.flushes, 1);
        bump_max(&self.stats.flush_nanos_max, nanos(start, Instant::now()));
        Ok(())
    }

    /// Whether some level is over its target (what `compaction::pick`
    /// decides; the round-robin positions only choose the input file).
    fn compaction_due(&self, levels: &Levels) -> bool {
        compaction::pick(levels, &self.tuning, &NO_POINTERS, &Busy::default()).is_some()
    }

    fn compactor(&self) {
        // The round-robin positions are shared by the compaction threads
        // (`State::pointers`); `compact` only needs a place to write its own.
        let mut unused = vec![Vec::new(); NUM_LEVELS];
        while let Some(((plan, l0_files), running)) = self.wait_for_work(Job::Compaction, |st| {
            if st.compactions_running >= self.max_compactions {
                return None;
            }
            let plan = compaction::pick(&st.version.levels, &self.tuning, &st.pointers, &st.busy)?;
            let claim = st.busy.claim(&plan);
            st.pointers[plan.level] = plan.largest_input_key().to_vec();
            Some(((plan, self.tuning.l0_metric(&st.version.levels[0])), Some(claim)))
        }) {
            if plan.level > 0 && l0_files >= self.tuning.l0_slowdown {
                // L0 is slowing writers down, yet a deeper level won (H2).
                bump(&self.stats.compactions_while_l0_slowdown, 1);
            }
            let r = self.or_poison("compaction failed", || self.compact(&plan, &mut unused, false));
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
            compaction::pick(&st.version.levels, &self.tuning, &w.pointers, &Busy::default())
        };
        match plan {
            Some(plan) => self.or_poison("compaction failed", || self.compact(&plan, &mut w.pointers, false)),
            None => Ok(false),
        }
    }

    /// Returns `false` if the compaction was abandoned for shutdown. A
    /// compaction of a `closing` session runs with the close-time settings
    /// (`close_subcompactions`, no intermediate syncs).
    fn compact(&self, plan: &Plan, pointers: &mut [Vec<u8>], closing: bool) -> Result<bool> {
        let purpose = if closing { Purpose::Close } else { Purpose::Background };
        Ok(self.compact_for(plan, pointers, purpose)?.is_some())
    }

    /// [`compact`](Self::compact) for any purpose; `None` if the compaction
    /// was abandoned for shutdown.
    fn compact_for(&self, plan: &Plan, pointers: &mut [Vec<u8>], purpose: Purpose) -> Result<Option<Compacted>> {
        let _phase = probe::phase(if purpose == Purpose::Directed { "compact_range" } else { "compact" });
        let closing = purpose == Purpose::Close;
        pointers[plan.level] = plan.largest_input_key().to_vec();
        if plan.is_trivial_move() {
            let mut ms = self.manifest.lock();
            let t = &plan.inputs[0];
            let mut levels = ms.levels.clone();
            levels[plan.level].retain(|x| x.number != t.number);
            let next = &mut levels[plan.output];
            let at = next.partition_point(|x| x.smallest < t.smallest);
            next.insert(at, t.clone());
            let (last_seq, log_number) = (ms.last_seq, ms.log_number);
            let edit = Edit::Compaction {
                reason: REASON_TRIVIAL_MOVE,
                next_file: Some(self.next_file.load(Ordering::SeqCst)),
                add: vec![t.meta(plan.output as u8)],
                delete: vec![(plan.level as u8, t.number)],
            };
            self.log_edit(&mut ms, edit, &levels, last_seq, log_number)?;
            ms.levels = levels.clone();
            self.install(levels, None);
            bump(&self.stats.trivial_moves, 1);
            return Ok(Some(Compacted::default()));
        }
        let _io = BackgroundIo::start(&self.stats);
        let start = Instant::now();
        let closing_tuning;
        let tuning = if closing {
            closing_tuning = Tuning {
                max_subcompactions: self.tuning.close_subcompactions,
                intermediate_syncs: false,
                ..self.tuning.clone()
            };
            &closing_tuning
        } else {
            &self.tuning
        };
        let Some(outputs) =
            compaction::run(&self.ctx, plan, tuning, &self.next_file, &self.shutting_down, &self.stats)?
        else {
            return Ok(None);
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
        let next_level_files = ms.levels[plan.output].len() as u64;
        let levels = compaction::apply(&ms.levels, plan, &outputs)?;
        let (last_seq, log_number) = (ms.last_seq, ms.log_number);
        let out_level = plan.output as u8;
        let edit = Edit::Compaction {
            reason: match purpose {
                Purpose::Background => REASON_BACKGROUND,
                Purpose::Close => REASON_CLOSE,
                Purpose::Directed => REASON_DIRECTED,
            },
            next_file: Some(self.next_file.load(Ordering::SeqCst)),
            add: outputs.iter().map(|t| t.meta(out_level)).collect(),
            delete: plan
                .inputs
                .iter()
                .map(|t| (plan.level as u8, t.number))
                .chain(plan.next_inputs.iter().map(|t| (out_level, t.number)))
                .collect(),
        };
        self.log_edit(&mut ms, edit, &levels, last_seq, log_number)?;
        ms.levels = levels.clone();
        for t in plan.all_inputs() {
            t.mark_obsolete();
        }
        self.install(levels, None);
        drop(ms);
        let read: u64 = plan.all_inputs().map(|t| t.size).sum();
        let written: u64 = outputs.iter().map(|t| t.size).sum();
        let s = &*self.stats;
        bump(&s.compactions, 1);
        bump(&s.compaction_bytes_read, read);
        bump(&s.compaction_bytes_written, written);
        bump_max(&s.compaction_nanos_max, nanos(start, Instant::now()));
        if plan.level == 0 {
            let l1_files = plan.next_inputs.len() as u64;
            bump(&s.l0_compactions, 1);
            bump(&s.l0_compaction_input_bytes, read);
            bump_max(&s.l0_compaction_input_bytes_max, read);
            bump(&s.l0_compaction_l1_files, l1_files);
            bump_max(&s.l0_compaction_l1_files_max, l1_files);
            bump(&s.l0_compaction_l1_files_present, next_level_files);
            if plan.output > 1 {
                bump(&s.l0_base_level_skips, 1);
            }
        }
        Ok(Some(Compacted { read, written }))
    }

    // ---- directed compaction ----

    /// Takes the next step of the directed compaction of the range, claiming
    /// what it touches, waiting while it conflicts with a compaction in
    /// flight. `None` once no file above the last level overlaps the range.
    fn claim_directed(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<Option<(Plan, Running<'_>)>> {
        let mut st = self.state.lock();
        loop {
            if let Some(msg) = &st.poisoned {
                return Err(Error::Poisoned(msg.clone()));
            }
            match compaction::pick_range(&st.version.levels, &st.busy, start, end) {
                RangeStep::Done => return Ok(None),
                RangeStep::Blocked => self.cv.wait(&mut st),
                RangeStep::Plan(plan) => {
                    let claim = st.busy.claim(&plan);
                    st.compactions_running += 1;
                    bump_max(&self.stats.compactions_concurrent_max, st.compactions_running as u64);
                    return Ok(Some((plan, Running { inner: self, job: Job::Compaction, claim: Some(claim) })));
                }
            }
        }
    }

    /// [`KernoDb::compact_range`]: (1) the active memtable, if it holds
    /// anything, is rotated and every memtable written out, so that the range
    /// tombstones and the data they cover are in files; (2) the files that
    /// overlap the range are compacted level by level, each step in the
    /// calling thread, claiming its files like a background compaction, until
    /// no level above the last overlaps it; (3) a checkpoint replaces the
    /// MANIFEST, whose old generation still names the keys of the files that
    /// went; (4) the cache forgets their blocks; (5) the report counts the
    /// replaced files that a snapshot keeps on disk.
    fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<CompactRangeReport> {
        self.check_writable()?;
        let _phase = probe::phase("compact_range");
        self.flush_all()?;
        let mut report = CompactRangeReport::default();
        let mut replaced: Vec<Arc<Table>> = Vec::new();
        let mut pointers = vec![Vec::new(); NUM_LEVELS];
        while let Some((plan, running)) = self.claim_directed(start, end)? {
            let r =
                self.or_poison("compaction failed", || self.compact_for(&plan, &mut pointers, Purpose::Directed))?;
            let Some(done) = r else {
                return Err(Error::Io(io::Error::other("compaction abandoned: the handle is closing")));
            };
            if !plan.is_trivial_move() {
                report.bytes_read += done.read;
                report.bytes_written += done.written;
                let Plan { inputs, next_inputs, .. } = plan;
                replaced.extend(inputs.into_iter().chain(next_inputs));
            }
            drop(running);
        }
        report.files_rewritten = replaced.len() as u64;
        self.checkpoint_now()?;
        for t in &replaced {
            t.purge_cache();
            // The table is dropped below (and its file deleted) unless
            // something else still holds it.
            if Arc::strong_count(t) > 1 {
                report.pinned_files += 1;
                report.pinned_bytes += t.size;
            }
        }
        drop(replaced);
        self.ctx.deleter.settle();
        Ok(report)
    }

    // ---- shutdown ----

    /// The compactions of a clean close, in the calling thread (the
    /// background threads are joined, and the final flush is done).
    ///
    /// First, always: compact until the L0 metric is below `l0_trigger`, so
    /// that many short sessions cannot grow L0 without bound (before, close
    /// abandoned the running compaction and then added one more L0 file).
    /// Then, one compaction at a time while the close budget allows it, until
    /// no level is over its target. Returns the bytes compacted (read plus
    /// written) and whether the budget stopped work that was still due.
    fn compact_for_close(&self, start: Instant) -> Result<(u64, bool)> {
        // The compaction must not see the shutdown flag and abandon its work.
        self.shutting_down.store(false, Ordering::SeqCst);
        // A close writes into L1 even if it is empty: merging the L0 files
        // with L1 rewrites far less than merging them with all of L2, which is
        // what skipping L1 would do, and the next session starts from the
        // usual shape (`dynamic_base_level` needs no transition).
        let tuning = Tuning { dynamic_base_level: false, ..self.tuning.clone() };
        let mut pointers = vec![Vec::new(); NUM_LEVELS];
        let mut compacted = 0u64;
        // Time spent in those compactions, the deletion of their inputs
        // included.
        let mut compacting = Duration::ZERO;
        let run = |plan: Plan, pointers: &mut [Vec<u8>]| -> Result<(u64, Duration)> {
            let (before, began) = (self.compaction_io(), Instant::now());
            self.compact(&plan, pointers, true)?;
            drop(plan);
            Ok((self.compaction_io() - before, began.elapsed()))
        };
        loop {
            let plan = {
                let st = self.state.lock();
                if tuning.l0_metric(&st.version.levels[0]) < tuning.l0_trigger {
                    break;
                }
                compaction::pick_l0_floor(&st.version.levels, &tuning)
            };
            let Some(plan) = plan else { break };
            let (bytes, took) = run(plan, &mut pointers)?;
            (compacted, compacting) = (compacted + bytes, compacting + took);
        }
        let exhausted = loop {
            let plan = {
                let st = self.state.lock();
                compaction::pick(&st.version.levels, &tuning, &pointers, &Busy::default())
            };
            let Some(plan) = plan else { break false };
            if !self.close_budget.allows(compacted, start.elapsed(), compacting, plan.io_bytes()) {
                break true;
            }
            let (bytes, took) = run(plan, &mut pointers)?;
            (compacted, compacting) = (compacted + bytes, compacting + took);
        };
        Ok((compacted, exhausted))
    }

    /// Bytes read and written by compactions so far.
    fn compaction_io(&self) -> u64 {
        self.stats.compaction_bytes_read.load(Ordering::Relaxed)
            + self.stats.compaction_bytes_written.load(Ordering::Relaxed)
    }

    /// W-9 steps 2 and 3: rotates the active memtable if it received commits
    /// (so that every `FLUSH` record can name a WAL that exists as the oldest
    /// live one), then flushes every immutable memtable, oldest first, one
    /// `FLUSH` record each. Background threads must already be stopped.
    /// Returns the bytes of the tables written.
    fn close_flush(&self) -> Result<u64> {
        let _phase = probe::phase("close");
        let before = self.stats.flush_bytes_written.load(Ordering::Relaxed);
        let mut w = self.writer.lock();
        if w.mem.max_seq() > 0 {
            self.rotate(&mut w)?;
        }
        self.flush_inline()?;
        drop(w);
        Ok(self.stats.flush_bytes_written.load(Ordering::Relaxed) - before)
    }

    /// W-9 steps 5 and 6, after every memtable is flushed and the
    /// compactions are done: the `CLEAN` checkpoint (`log_number =
    /// next_file`), then every WAL goes (W-5) and the directory is synced.
    fn close_clean(&self) -> Result<()> {
        let w = self.writer.lock();
        let mut ms = self.manifest.lock();
        if ms.last_seq != w.last_seq {
            return Err(Error::Corruption(format!(
                "internal: closing with commits up to {} while the MANIFEST holds {}",
                w.last_seq, ms.last_seq
            )));
        }
        let log_number = self.next_file.load(Ordering::SeqCst);
        let (levels, last_seq) = (ms.levels.clone(), ms.last_seq);
        self.checkpoint_state(&mut ms, &levels, last_seq, log_number, true)?;
        ms.log_number = log_number;
        drop(ms);
        drop(w);
        {
            // Drop the memtables; later snapshots of this handle are not possible.
            let mut st = self.state.lock();
            let mem = Arc::new(Memtable::new(log_number));
            st.version = Arc::new(Version { mem, imms: Vec::new(), levels });
        }
        // W-9 step 6: nothing needs the WALs any more.
        let _phase = probe::phase("close");
        *self.spare.lock() = false;
        for name in self.vfs.list(&self.dir)? {
            if let FileKind::Wal(_) | FileKind::SpareWal = parse_name(&name) {
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
        if self.read_only.is_some() {
            // Nothing runs, and what is over its target stays that way.
            return Ok(());
        }
        let mut st = self.state.lock();
        loop {
            if let Some(msg) = &st.poisoned {
                return Err(Error::Poisoned(msg.clone()));
            }
            let idle = self.inline()
                || st.shutdown
                || (!st.flush_running
                    && st.compactions_running == 0
                    && st.version.imms.is_empty()
                    && !st.spare_wanted
                    && !self.compaction_due(&st.version.levels));
            if idle {
                break;
            }
            self.cv.wait(&mut st);
        }
        drop(st);
        // Deferred deletion is background work too: the files the work above
        // made obsolete are gone (or their failures counted) when this returns.
        self.ctx.deleter.settle();
        Ok(())
    }

    /// [`TestHooks::checkpoint`]: replaces the MANIFEST with a new
    /// generation holding the current state (W-10).
    fn checkpoint_now(&self) -> Result<()> {
        self.check_writable()?;
        self.check_poisoned()?;
        let mut ms = self.manifest.lock();
        let (levels, last_seq, log_number) = (ms.levels.clone(), ms.last_seq, ms.log_number);
        self.or_poison("checkpoint failed", || self.checkpoint_state(&mut ms, &levels, last_seq, log_number, false))
    }

    /// [`TestHooks::flush`]: rotates a non-empty active memtable and waits
    /// until every immutable memtable is on disk (inline: writes them).
    fn flush_all(&self) -> Result<()> {
        self.check_writable()?;
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
        self.check_writable()?;
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
        // The engine is gone: what is still queued is deleted now, and a
        // snapshot that outlives the handle deletes inline from here on.
        self.inner.ctx.deleter.shutdown();
    }
}

impl KernoDb {
    /// Test-only control over flushes and compactions (see [`TestHooks`]).
    #[doc(hidden)]
    pub fn hooks(&self) -> &dyn TestHooks {
        self
    }
}

/// A table of the current version, for tests that check the shape of the
/// LSM tree.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub number: u64,
    pub size: u64,
    pub entries: u64,
    /// `(start, end, seq)` of the range tombstone fragments.
    pub fragments: Vec<(Vec<u8>, Vec<u8>, u64)>,
    pub smallest: Vec<u8>,
    pub largest: Vec<u8>,
    pub largest_exclusive: bool,
}

impl KernoDb {
    /// The tables of every level of the current version (L0 newest first).
    #[doc(hidden)]
    pub fn level_tables(&self) -> Vec<Vec<TableInfo>> {
        let v = self.inner.state.lock().version.clone();
        v.levels
            .iter()
            .map(|l| {
                l.iter()
                    .map(|t| TableInfo {
                        number: t.number,
                        size: t.size,
                        entries: t.entries(),
                        fragments: t.fragments().iter().map(|f| (f.start.clone(), f.end.clone(), f.seq)).collect(),
                        smallest: t.smallest.clone(),
                        largest: t.largest.clone(),
                        largest_exclusive: t.largest_exclusive,
                    })
                    .collect()
            })
            .collect()
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

    fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<()> {
        KernoDb::compact_range(self, start, end).map(drop)
    }

    fn checkpoint(&self) -> Result<()> {
        self.inner.checkpoint_now()
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
    /// previous handle was not closed cleanly (v1.md §9.4, S-1..S-6).
    ///
    /// Everything that can reject the directory (a foreign one, an unknown
    /// version or feature, damage) is checked before the first write. Only
    /// creating the database directory and its `LOCK` come before the lock.
    ///
    /// With [`AccessMode::ReadOnly`], or when a file has `ro_compat` bits
    /// this engine does not know, the handle only reads (v1.md §9.5): it
    /// takes the exclusive `LOCK`, replays the live WALs in memory and
    /// changes nothing else.
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &Config) -> Result<KernoDb> {
        cfg.validate()?;
        let requested_read_only = cfg.access == AccessMode::ReadOnly;
        // Without a MANIFEST there is no database to open: nothing is
        // created, locked or changed unless this open may create one (DIR-2).
        let first = list_dir(&*vfs, dir)?;
        let creating = !first.has_manifest;
        if creating {
            if let Some(e) = refuse_without_manifest(&first) {
                return Err(e);
            }
            if requested_read_only || !cfg.create_if_missing {
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
            if requested_read_only || !cfg.create_if_missing {
                return Err(no_database(dir));
            }
            manifest::create(&*vfs, dir, cfg.test.db_id.unwrap_or_else(new_db_id))?;
            listing.has_manifest = true;
        }
        // S-1: classify and validate everything, writing nothing.
        let (loaded, plan) = recovery::plan(&*vfs, dir, &listing)?;
        let db_id = loaded.replay.header.db_id;
        let stats = Arc::new(Stats::default());
        let deleter = Deleter::inline(vfs.clone(), stats.clone());
        let ctx = Arc::new(
            TableCtx::new(vfs.clone(), dir.to_path_buf(), cfg.cache_bytes, stats.clone(), BLOCK_SIZE)
                .with_bytes_per_sync(cfg.compaction.bytes_per_sync)
                .with_bytes_per_writeback(cfg.compaction.bytes_per_writeback)
                .with_db_id(db_id)
                .with_deleter(deleter),
        );
        let (levels, table_ro_compat) = recovery::open_tables(&ctx, &loaded.replay.state)?;
        let read_only = if requested_read_only {
            Some(ReadOnlyReason::Requested)
        } else {
            recovery::unknown_ro_compat_reason(&plan.unknown_ro_compat, table_ro_compat)
        };
        let tuning = Tuning::new(cfg);
        // A read-only handle never writes a log, so it never prepares or
        // recycles one either.
        let wal_prealloc = if cfg.wal_preallocate && read_only.is_none() { tuning.write_buffer as u64 } else { 0 };
        let floor = CHECKPOINT_FLOOR;
        let (levels, last_seq, log_number, next_file, wal, manifest_writer, mem) = if read_only.is_some() {
            let (mem, last_seq) = recovery::replay_in_memory(&ctx, &plan, db_id, &loaded.replay.state)?;
            let writer = loaded.writer(floor);
            let state = &loaded.replay.state;
            (levels, last_seq, state.log_number, plan.next_file, WalWriter::read_only(0), writer, Arc::new(mem))
        } else {
            let r = recovery::apply(&ctx, &plan, &loaded, levels, tuning.write_buffer, floor, wal_prealloc)?;
            let mem = Arc::new(Memtable::new(r.wal.number()));
            (r.levels, r.last_seq, r.log_number, r.next_file, r.wal, r.manifest, mem)
        };
        // The L0 found at open counts as seen by this handle.
        bump_max(&stats.l0_files_max, levels[0].len() as u64);
        let version = Arc::new(Version { mem: mem.clone(), imms: Vec::new(), levels: levels.clone() });
        let inner = Arc::new(Inner {
            vfs,
            dir: dir.to_path_buf(),
            durability: cfg.durability,
            tuning,
            max_compactions: cfg.compaction.max_background_compactions,
            close_budget: cfg.close_budget,
            max_immutable: cfg.max_immutable_memtables,
            limits: cfg.limits,
            background: cfg.test.background,
            ctx,
            stats,
            read_only,
            orphans: plan.orphans,
            next_file: AtomicU64::new(next_file),
            wal_prealloc,
            spare: Mutex::new(false),
            spare_requested: AtomicBool::new(false),
            txn_gate: Mutex::new(()),
            queue: CommitQueue::new(cfg.group_commit.queue_limits()),
            writer: Mutex::new(Writer {
                wal,
                mem,
                last_seq,
                pointers: vec![Vec::new(); NUM_LEVELS],
                rate: RateController::new(match cfg.compaction.slowdown {
                    SlowdownPolicy::Rate { initial_bytes_per_sec } => initial_bytes_per_sec,
                    _ => 0,
                }),
            }),
            manifest: Mutex::new(ManifestState { levels, last_seq, log_number, writer: manifest_writer }),
            state: Mutex::new(State {
                version,
                last_seq,
                // Recovery made what it recovered durable.
                durable_seq: last_seq,
                poisoned: None,
                shutdown: false,
                flush_running: false,
                spare_wanted: false,
                compactions_running: 0,
                busy: Busy::default(),
                pointers: vec![Vec::new(); NUM_LEVELS],
            }),
            cv: Condvar::new(),
            poisoned: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            _lock: lock,
        });
        let engine = KernoDb { inner: inner.clone(), threads: Mutex::new(Vec::new()) };
        // S-6.
        if cfg.test.background == Background::Threads && inner.read_only.is_none() {
            let compactors = (0..cfg.compaction.max_background_compactions)
                .map(|i| (format!("kerno-compact-{i}"), Inner::compactor as fn(&Inner)));
            for (name, body) in
                [("kerno-flush".to_string(), Inner::flusher as fn(&Inner))].into_iter().chain(compactors)
            {
                let handle = Self::spawn(&inner, &name, body)?;
                engine.threads.lock().push(handle);
            }
            inner.ctx.deleter.spawn()?;
        }
        Ok(engine)
    }

    /// Why this handle cannot write, if it cannot: it was opened with
    /// [`AccessMode::ReadOnly`], or a file has `ro_compat` bits this engine
    /// does not know (v1.md HDR-7).
    pub fn is_read_only(&self) -> Option<ReadOnlyReason> {
        self.inner.read_only.clone()
    }

    /// The names of the files recovery found that nothing needs (orphans,
    /// v1.md REC-7, REC-8, REC-11). A read-write open has deleted them; a
    /// read-only one leaves them in place.
    #[doc(hidden)]
    pub fn recovery_orphans(&self) -> &[String] {
        &self.inner.orphans
    }

    /// The metrics of this handle now (phase1-design.md §3).
    pub fn metrics(&self) -> Metrics {
        let inner = &*self.inner;
        let (v, last_seq, durable_seq) = {
            let st = inner.state.lock();
            (st.version.clone(), st.last_seq, st.durable_seq)
        };
        let shape = ShapeMetrics {
            levels: v
                .levels
                .iter()
                .map(|l| LevelMetrics { files: l.len() as u64, bytes: l.iter().map(|t| t.size).sum() })
                .collect(),
            l0_depth: compaction::l0_depth(&v.levels[0]) as u64,
            range_fragments: v.levels.iter().flatten().map(|t| t.fragments().len() as u64).sum(),
            l0_files_max: 0,
            memtable_bytes: v.mem.approx_bytes() as u64,
            immutable_memtables: v.imms.len() as u64,
            last_seq,
            durable_seq,
            block_cache_bytes: inner.ctx.cache.weight(),
        };
        inner.stats.metrics(shape, compaction::pending_bytes(&v.levels, &inner.tuning))
    }

    /// Makes every commit acknowledged so far durable and returns the durable
    /// sequence (ADR-0002, I-17). With [`Durability::Sync`](kerno_core::Durability::Sync)
    /// every acknowledged commit is durable already and this does no I/O; in
    /// `Buffered` mode it costs one `fsync` of the WAL if frames are
    /// unsynced. A failed `fsync` poisons the handle. Shares the commit queue:
    /// it is served by the leader of the group it joins.
    pub fn sync(&self) -> Result<u64> {
        self.inner.sync()
    }

    /// Pushes the data and the range deletions between the bounds `start` and
    /// `end` to the last level and out of the files: the active write
    /// buffer is written out, then the files that overlap the range are
    /// compacted level by level until none above the last does, and the
    /// compactions drop every tombstone, point or range, that has nothing
    /// deeper to hide, and the points a range tombstone covers. The MANIFEST
    /// is checkpointed, so that its history no longer names the keys of the
    /// files that went. Returns when that is done.
    ///
    /// This is a mechanism, not a guarantee of erasure: a snapshot keeps the
    /// files of the version it holds (the report counts them), WAL files, free
    /// space in the file system and the memory of the OS and the block cache
    /// are not scrubbed, and a table that background compactions moved to
    /// the last level with a point tombstone in it keeps it (`DESIGN.md`).
    pub fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<CompactRangeReport> {
        self.inner.compact_range(start, end)
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

    /// Closes the handle: flushes every memtable, compacts within the
    /// [`CloseBudget`] (always enough to bring the L0 metric below
    /// `l0_trigger`) and leaves a MANIFEST that needs no WAL. Dropping the
    /// handle instead is a crash as far as the files are concerned.
    pub fn close(self) -> Result<CloseReport> {
        let _phase = probe::phase("close");
        self.close_inner()
    }

    fn close_inner(&self) -> Result<CloseReport> {
        let start = Instant::now();
        let inner = &*self.inner;
        if inner.read_only.is_some() {
            // Nothing was opened for writing, so nothing is written.
            let l0 = inner.state.lock().version.levels[0].clone();
            return Ok(CloseReport {
                duration: start.elapsed(),
                flushed_bytes: 0,
                compacted_bytes: 0,
                budget_exhausted: false,
                l0_files: l0.len() as u64,
                l0_depth: compaction::l0_depth(&l0) as u64,
            });
        }
        self.stop_threads();
        inner.check_poisoned()?;
        // W-9: rotate and flush, compact, write the CLEAN checkpoint, delete
        // the WALs.
        let (flushed_bytes, (compacted_bytes, budget_exhausted)) = inner.or_poison("close failed", || {
            let flushed = inner.close_flush()?;
            let compacted = inner.compact_for_close(start)?;
            inner.close_clean()?;
            Ok((flushed, compacted))
        })?;
        let l0 = inner.state.lock().version.levels[0].clone();
        Ok(CloseReport {
            duration: start.elapsed(),
            flushed_bytes,
            compacted_bytes,
            budget_exhausted,
            l0_files: l0.len() as u64,
            l0_depth: compaction::l0_depth(&l0) as u64,
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
        KernoDb::close(self).map(drop)
    }

    /// [`KernoDb::metrics`], flattened ([`Metrics::flatten`]).
    fn stats(&self) -> Vec<(String, u64)> {
        self.metrics().flatten()
    }
}

impl SyncEngine for KernoDb {
    fn sync(&self) -> Result<u64> {
        KernoDb::sync(self)
    }
}

impl RangeDeleteEngine for KernoDb {}
