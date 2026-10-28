//! In-memory file system that simulates crashes.
//!
//! Every file keeps two images: what the running process sees (`data`) and
//! what is guaranteed to be on stable storage (`durable`), plus the list of
//! writes issued since the last `sync_data`. Directory entries are tracked
//! the same way (`names` vs `durable_names` + pending namespace operations).
//!
//! A crash produces a *new* [`SimVfs`] (the "disk after reboot"):
//!
//! * [`CrashKind::ProcessKill`] keeps everything the process wrote (the OS
//!   page cache survives a process crash) and treats it as durable.
//! * [`CrashKind::ProcessKillKeepPending`] also keeps everything the process
//!   wrote, but what was not synced stays pending: a later power loss on the
//!   image can still lose it, exactly as on a machine that keeps running
//!   after the process dies.
//! * [`CrashKind::PowerLoss`] keeps only durable state plus a random subset
//!   of the pending writes: each one is dropped, applied, or torn at 512-byte
//!   sector granularity (or a finer unit set with [`SimVfs::set_tear_unit`]),
//!   independently and in no particular order. Pending namespace operations
//!   survive as a random *prefix* (journaled metadata).
//!
//! Crashes can be triggered at an exact mutating operation with
//! [`SimVfs::crash_after`], or at the n-th operation that matches a
//! predicate over [`OpInfo`] with [`SimVfs::crash_when`]; from then on the
//! old instance fails every call, like a machine that lost power.
//!
//! A single I/O error with the device still alive can be injected with
//! [`SimVfs::fail_after`] or [`SimVfs::fail_when`]: that one operation
//! returns an error and has no effect, and everything else keeps working.
//! What a failed `sync_data` does to its writes is chosen with
//! [`SimVfs::set_fsync_failure`]: by default they stay pending (not durable,
//! but a later successful sync can still persist them); with
//! [`FsyncFailure::LinuxDropPages`] they are discarded for good, as Linux
//! does after a failed `fsync` (the data stays visible to readers, and the
//! next `sync_data` returns `Ok` without persisting it; pending length
//! changes are metadata and are kept for that next sync). This is how the
//! harness checks that a failed `fsync` is never acknowledged and poisons the
//! handle.
//!
//! Further opt-in models, all off by default (the default behaviour, down to
//! the random draws, is pinned by `default_mode_crash_images_unchanged`):
//!
//! * [`SimVfs::set_dir_durability`] with [`DirDurability::PerDir`]:
//!   `sync_dir(d)` only persists namespace operations on entries of `d` (and
//!   the earlier ones they depend on), and a new directory is durable only
//!   after its parent is synced. By default ([`DirDurability::Global`])
//!   syncing any directory persists every pending namespace operation, and
//!   directories are durable as soon as they are created.
//! * [`SimVfs::set_space_limit`] and [`SimVfs::fill_disk_after`]: a write or
//!   `set_len` that would grow the bytes in use past the limit fails with
//!   [`io::ErrorKind::StorageFull`] and has no effect (ENOSPC). An unlinked
//!   file keeps its space until its last handle is closed.
//! * [`SimVfs::start_trace`]: records every mutating operation with the
//!   phase label of [`kerno_core::probe`] and the calling thread.
//! * [`SimVfs::set_sync_latency`]: `sync_data` sleeps outside the global
//!   lock, so that concurrent writers can make progress meanwhile.
//! * [`SimVfs::dump`], [`SimVfs::restore`] and [`assert_dir_unchanged`]:
//!   byte-level images of the whole file system.
//! * [`SimVfs::corrupt_byte_live`]: media damage while writes are pending.
//!
//! Every crash image inherits the device's failure models (tear unit, fsync
//! failure, directory durability, space limit, sync latency); triggers,
//! faults and traces are not inherited.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use parking_lot::Mutex;

use super::{File, OpenMode, Vfs};
use crate::rng::Rng;

const SECTOR: u64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashKind {
    PowerLoss,
    ProcessKill,
    /// The process dies but the machine keeps running: the image keeps the
    /// visible data, the durable data and the pending (unsynced) writes and
    /// namespace operations exactly, so a later
    /// [`crash_now(PowerLoss)`](SimVfs::crash_now) on the image can still lose
    /// what was never synced. (`ProcessKill` marks everything durable.)
    ProcessKillKeepPending,
}

/// What a failed `sync_data` does to the writes it was asked to persist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FsyncFailure {
    /// They stay pending: not durable, but a later successful sync can
    /// still persist them.
    #[default]
    KeepPending,
    /// Linux semantics: they are discarded for good (the dirty pages are
    /// marked clean), the data stays visible to readers, and the next
    /// `sync_data` returns `Ok` without persisting it. Pending length changes
    /// (`set_len`) are metadata, not dirty pages: they stay pending, in
    /// order, and the next successful sync persists them.
    LinuxDropPages,
}

/// Which namespace operations `sync_dir` makes durable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirDurability {
    /// Syncing any directory persists every pending namespace operation, in
    /// order (a single metadata journal); directories are durable as soon as
    /// they are created.
    #[default]
    Global,
    /// `sync_dir(d)` only persists creations, removals and renames of
    /// entries of `d` (a rename counts for the destination's directory),
    /// together with the earlier operations on the same inodes or names
    /// that they depend on (so a rename across directories also persists
    /// its source's pending creation, and the source name never comes
    /// back); a new directory is durable only once its parent is synced.
    PerDir,
}

/// Kind of a mutating operation (see [`OpInfo`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OpKind {
    WriteAt,
    SetLen,
    SyncData,
    /// Creation of a file (`open` with `CreateNew`/`CreateOrOpen` of a
    /// missing file).
    Create,
    Remove,
    Rename,
    SyncDir,
}

/// One mutating operation, as seen by [`SimVfs::crash_when`],
/// [`SimVfs::fail_when`] and [`SimVfs::trace`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpInfo {
    pub kind: OpKind,
    /// The file operated on: its current name for file operations (the name
    /// it was opened under if it has been unlinked since), the source of a
    /// rename, the directory of a `sync_dir`.
    pub path: PathBuf,
    /// Destination of a rename; `None` for every other kind.
    pub to: Option<PathBuf>,
    /// `WriteAt`: offset written. Otherwise 0.
    pub offset: u64,
    /// `WriteAt`: bytes written. `SetLen`: the new length. Otherwise 0.
    pub len: u64,
    /// [`kerno_core::probe::current_phase`] of the calling thread.
    pub phase: &'static str,
}

/// A traced mutating operation (see [`SimVfs::start_trace`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceEntry {
    /// Its number, as counted by [`SimVfs::mutating_ops`] and
    /// [`SimVfs::crash_after`].
    pub n: u64,
    pub op: OpInfo,
    /// Name of the calling thread (its id if it has no name).
    pub thread: String,
}

/// Byte-level image of a [`SimVfs`] (see [`SimVfs::dump`]).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DirImage {
    pub dirs: BTreeSet<PathBuf>,
    /// Visible bytes of every file, by path.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    /// Visible bytes of files that were unlinked but are still open, in
    /// inode order.
    pub unlinked_open: Vec<Vec<u8>>,
}

impl fmt::Debug for DirImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let files: Vec<String> = self
            .files
            .iter()
            .map(|(p, d)| format!("{} ({} bytes, crc32c {:08x})", p.display(), d.len(), crc32c::crc32c(d)))
            .collect();
        f.debug_struct("DirImage")
            .field("dirs", &self.dirs)
            .field("files", &files)
            .field("unlinked_open", &self.unlinked_open.len())
            .finish()
    }
}

/// Describes the first difference between two images (directories and file
/// bytes; unlinked open files are not compared), ignoring files whose name
/// or full path is in `ignore`. `None` if they match.
pub fn dir_difference(before: &DirImage, after: &DirImage, ignore: &[&str]) -> Option<String> {
    if before.dirs != after.dirs {
        return Some(format!("directories differ: {:?} vs {:?}", before.dirs, after.dirs));
    }
    let ignored = |p: &Path| ignore.iter().any(|i| p.file_name().is_some_and(|n| n == *i) || p.as_os_str() == *i);
    let paths: BTreeSet<&PathBuf> = before.files.keys().chain(after.files.keys()).collect();
    for p in paths.into_iter().filter(|p| !ignored(p)) {
        match (before.files.get(p), after.files.get(p)) {
            (Some(_), None) => return Some(format!("{} was removed", p.display())),
            (None, Some(_)) => return Some(format!("{} was created", p.display())),
            (Some(a), Some(b)) if a != b => {
                let first = a.iter().zip(b.iter()).position(|(x, y)| x != y);
                return Some(match first {
                    Some(off) => format!("{}: byte {off} changed ({:#04x} -> {:#04x})", p.display(), a[off], b[off]),
                    None => format!("{}: length changed from {} to {} bytes", p.display(), a.len(), b.len()),
                });
            }
            _ => {}
        }
    }
    None
}

/// Panics, naming the first difference, unless the two images have the
/// same directories and the same bytes in every file not listed in
/// `ignore` (file names such as `"LOCK"`, or full paths).
#[track_caller]
pub fn assert_dir_unchanged(before: &DirImage, after: &DirImage, ignore: &[&str]) {
    if let Some(d) = dir_difference(before, after, ignore) {
        panic!("directory changed: {d}");
    }
}

#[derive(Clone)]
enum Pending {
    Write { off: u64, data: Vec<u8> },
    SetLen(u64),
}

#[derive(Clone, Default)]
struct Inode {
    data: Vec<u8>,
    durable: Vec<u8>,
    /// Unsynced writes and length changes, in the order they were issued,
    /// each with its push number (see `pushed`).
    pending: Vec<(u64, Pending)>,
    /// Writes and length changes ever pushed to `pending`; the last one
    /// pushed has this number.
    pushed: u64,
}

#[derive(Clone)]
enum NsOp {
    Link {
        path: PathBuf,
        ino: u64,
    },
    Unlink {
        path: PathBuf,
    },
    Rename {
        from: PathBuf,
        to: PathBuf,
        ino: u64,
    },
    /// Directory creation; only recorded with [`DirDurability::PerDir`].
    Mkdir {
        path: PathBuf,
    },
}

impl NsOp {
    /// The directory whose sync persists this operation under
    /// [`DirDurability::PerDir`].
    fn dir(&self) -> Option<&Path> {
        match self {
            NsOp::Link { path, .. } | NsOp::Unlink { path } | NsOp::Mkdir { path } => path.parent(),
            NsOp::Rename { to, .. } => to.parent(),
        }
    }

    fn ino(&self) -> Option<u64> {
        match self {
            NsOp::Link { ino, .. } | NsOp::Rename { ino, .. } => Some(*ino),
            NsOp::Unlink { .. } | NsOp::Mkdir { .. } => None,
        }
    }

    /// The names it creates or removes.
    fn paths(&self) -> [Option<&Path>; 2] {
        match self {
            NsOp::Link { path, .. } | NsOp::Unlink { path } | NsOp::Mkdir { path } => [Some(path), None],
            NsOp::Rename { from, to, .. } => [Some(from), Some(to)],
        }
    }

    /// Whether `earlier`, issued before `self`, must be durable before
    /// `self` can be: both act on the same inode or on the same name.
    fn depends_on(&self, earlier: &NsOp) -> bool {
        let same_ino = self.ino().is_some() && self.ino() == earlier.ino();
        same_ino || self.paths().into_iter().flatten().any(|p| earlier.paths().into_iter().flatten().any(|q| p == q))
    }
}

fn apply_ns(names: &mut BTreeMap<PathBuf, u64>, dirs: &mut BTreeSet<PathBuf>, op: &NsOp) {
    match op {
        NsOp::Link { path, ino } => {
            names.insert(path.clone(), *ino);
        }
        NsOp::Unlink { path } => {
            names.remove(path);
        }
        NsOp::Rename { from, to, ino } => {
            // Applied in order, or as a `PerDir` sync's dependency closure
            // (which includes every earlier operation on `ino` or `from`),
            // `from` names `ino` here; the check only guards that.
            if names.get(from) == Some(ino) {
                names.remove(from);
            }
            names.insert(to.clone(), *ino);
        }
        NsOp::Mkdir { path } => {
            dirs.insert(path.clone());
        }
    }
}

fn write_into(buf: &mut Vec<u8>, off: u64, data: &[u8]) {
    let off = off as usize;
    let end = off + data.len();
    if buf.len() < end {
        buf.resize(end, 0);
    }
    buf[off..end].copy_from_slice(data);
}

fn apply_pending(buf: &mut Vec<u8>, p: &Pending) {
    match p {
        Pending::Write { off, data } => write_into(buf, *off, data),
        Pending::SetLen(len) => buf.resize(*len as usize, 0),
    }
}

type OpPredicate = Arc<dyn Fn(&OpInfo) -> bool + Send + Sync>;

/// A predicate-filtered crash or fault (see [`SimVfs::crash_when`] and
/// [`SimVfs::fail_when`]).
struct WhenArm<T> {
    pred: OpPredicate,
    /// Matching operations still to go before it fires (>= 1).
    remaining: u64,
    action: T,
}

/// Why a mutating operation was refused before taking effect.
enum Refused {
    /// The device is gone (crash).
    Dead,
    /// An injected one-shot I/O error.
    Injected,
}

impl Refused {
    fn into_io(self) -> io::Error {
        match self {
            Refused::Dead => dead_error(),
            Refused::Injected => injected_error(),
        }
    }
}

/// What a mutating operation targets, resolved into an [`OpInfo`] only when
/// a trace or a predicate needs it.
enum Target<'a> {
    Path(&'a Path),
    Rename(&'a Path, &'a Path),
    File { ino: u64, opened_as: &'a Path },
}

struct OpDesc<'a> {
    kind: OpKind,
    target: Target<'a>,
    offset: u64,
    len: u64,
}

struct State {
    inodes: HashMap<u64, Inode>,
    next_ino: u64,
    names: BTreeMap<PathBuf, u64>,
    durable_names: BTreeMap<PathBuf, u64>,
    pending_ns: Vec<NsOp>,
    dirs: BTreeSet<PathBuf>,
    /// Directories that survive a power loss (equal to `dirs` unless
    /// [`DirDurability::PerDir`]).
    durable_dirs: BTreeSet<PathBuf>,
    locks: BTreeSet<PathBuf>,
    /// One token per inode while some handle has it open.
    handles: HashMap<u64, Weak<()>>,
    mutating_ops: u64,
    trigger: Option<(u64, CrashKind)>,
    fault_at: Option<u64>,
    fault_then: Option<CrashKind>,
    fault_injected: bool,
    when_crash: Option<WhenArm<CrashKind>>,
    when_fault: Option<WhenArm<Option<CrashKind>>>,
    dead: bool,
    image: Option<Box<State>>,
    /// Granularity at which a pending write can be torn by a power loss.
    tear_unit: u64,
    fsync_failure: FsyncFailure,
    dir_durability: DirDurability,
    space_limit: Option<u64>,
    /// Mutating operation number at which the disk becomes full.
    fill_at: Option<u64>,
    sync_latency: Option<Duration>,
    trace: Option<Vec<TraceEntry>>,
    rng: Rng,
}

fn dead_error() -> io::Error {
    io::Error::other("simulated crash: device is gone")
}

fn injected_error() -> io::Error {
    io::Error::other("injected I/O error (EIO)")
}

fn disk_full_error() -> io::Error {
    io::Error::new(io::ErrorKind::StorageFull, "simulated disk full (ENOSPC)")
}

fn thread_label() -> String {
    let t = std::thread::current();
    t.name().map(str::to_owned).unwrap_or_else(|| format!("{:?}", t.id()))
}

impl State {
    fn new(rng: Rng) -> Self {
        State {
            inodes: HashMap::new(),
            next_ino: 1,
            names: BTreeMap::new(),
            durable_names: BTreeMap::new(),
            pending_ns: Vec::new(),
            dirs: BTreeSet::new(),
            durable_dirs: BTreeSet::new(),
            locks: BTreeSet::new(),
            handles: HashMap::new(),
            mutating_ops: 0,
            trigger: None,
            fault_at: None,
            fault_then: None,
            fault_injected: false,
            when_crash: None,
            when_fault: None,
            dead: false,
            image: None,
            tear_unit: SECTOR,
            fsync_failure: FsyncFailure::default(),
            dir_durability: DirDurability::default(),
            space_limit: None,
            fill_at: None,
            sync_latency: None,
            trace: None,
            rng,
        }
    }

    fn check_alive(&self) -> io::Result<()> {
        if self.dead { Err(dead_error()) } else { Ok(()) }
    }

    fn op_info(&self, op: &OpDesc<'_>) -> OpInfo {
        let (path, to) = match op.target {
            Target::Path(p) => (p.to_path_buf(), None),
            Target::Rename(from, to) => (from.to_path_buf(), Some(to.to_path_buf())),
            Target::File { ino, opened_as } => {
                let current = self.names.iter().find(|(_, i)| **i == ino).map(|(p, _)| p.clone());
                (current.unwrap_or_else(|| opened_as.to_path_buf()), None)
            }
        };
        OpInfo { kind: op.kind, path, to, offset: op.offset, len: op.len, phase: kerno_core::probe::current_phase() }
    }

    /// Accounts one mutating operation; fires the crash trigger or the
    /// injected fault if due. Either way the operation is *not* performed.
    fn tick(&mut self, op: OpDesc<'_>) -> Result<(), Refused> {
        if self.dead {
            return Err(Refused::Dead);
        }
        self.mutating_ops += 1;
        let n = self.mutating_ops;
        let info =
            (self.trace.is_some() || self.when_crash.is_some() || self.when_fault.is_some()).then(|| self.op_info(&op));
        if let (Some(trace), Some(info)) = (self.trace.as_mut(), &info) {
            trace.push(TraceEntry { n, op: info.clone(), thread: thread_label() });
        }
        if self.fill_at == Some(n) {
            self.fill_at = None;
            self.space_limit = Some(self.used_bytes());
        }
        if let Some((at, kind)) = self.trigger
            && n >= at
        {
            self.crash(kind);
            return Err(Refused::Dead);
        }
        if let (Some(arm), Some(info)) = (self.when_crash.as_mut(), &info)
            && (arm.pred)(info)
        {
            arm.remaining -= 1;
            if arm.remaining == 0 {
                let kind = arm.action;
                self.when_crash = None;
                self.crash(kind);
                return Err(Refused::Dead);
            }
        }
        if self.fault_at == Some(n) {
            self.fault_at = None;
            self.fault_injected = true;
            if let Some(kind) = self.fault_then.take() {
                self.trigger = Some((n + 1, kind));
            }
            return Err(Refused::Injected);
        }
        if let (Some(arm), Some(info)) = (self.when_fault.as_mut(), &info)
            && (arm.pred)(info)
        {
            arm.remaining -= 1;
            if arm.remaining == 0 {
                let then = arm.action;
                self.when_fault = None;
                self.fault_injected = true;
                if let Some(kind) = then {
                    self.trigger = Some((n + 1, kind));
                }
                return Err(Refused::Injected);
            }
        }
        Ok(())
    }

    fn crash(&mut self, kind: CrashKind) {
        let image = self.build_image(kind);
        self.image = Some(Box::new(image));
        self.dead = true;
    }

    fn build_image(&mut self, kind: CrashKind) -> State {
        let mut rng = self.rng.fork();
        let per_dir = self.dir_durability == DirDurability::PerDir;
        let (names, dirs) = match kind {
            CrashKind::ProcessKill | CrashKind::ProcessKillKeepPending => (self.names.clone(), self.dirs.clone()),
            CrashKind::PowerLoss => {
                let mut names = self.durable_names.clone();
                let mut dirs = self.durable_dirs.clone();
                let keep = rng.below(self.pending_ns.len() as u64 + 1) as usize;
                for op in &self.pending_ns[..keep] {
                    apply_ns(&mut names, &mut dirs, op);
                }
                if per_dir {
                    // A directory whose own entry was lost takes its whole
                    // subtree with it.
                    let all = dirs.clone();
                    dirs.retain(|d| d.ancestors().skip(1).all(|a| all.contains(a)));
                    names.retain(|p, _| p.parent().is_none_or(|d| dirs.contains(d)));
                    (names, dirs)
                } else {
                    // Directories are durable as soon as they are created.
                    (names, self.dirs.clone())
                }
            }
        };
        let mut inodes = HashMap::new();
        if kind == CrashKind::ProcessKillKeepPending {
            // Every inode a later power loss on the image could still reach.
            let mut reachable: BTreeSet<u64> = names.values().chain(self.durable_names.values()).copied().collect();
            for op in &self.pending_ns {
                if let NsOp::Link { ino, .. } | NsOp::Rename { ino, .. } = op {
                    reachable.insert(*ino);
                }
            }
            for ino in reachable {
                inodes.insert(ino, self.inodes[&ino].clone());
            }
        }
        for &ino in names.values() {
            if kind == CrashKind::ProcessKillKeepPending || inodes.contains_key(&ino) {
                continue;
            }
            let inode = &self.inodes[&ino];
            let content = match kind {
                CrashKind::ProcessKill | CrashKind::ProcessKillKeepPending => inode.data.clone(),
                CrashKind::PowerLoss => {
                    let mut content = inode.durable.clone();
                    for (_, p) in &inode.pending {
                        match p {
                            Pending::SetLen(_) => {
                                if rng.chance(1, 2) {
                                    apply_pending(&mut content, p);
                                }
                            }
                            Pending::Write { off, data } => match rng.below(8) {
                                0..=2 => {}
                                3..=5 => apply_pending(&mut content, p),
                                _ => {
                                    // Torn: every sector-aligned piece of the
                                    // write independently made it or not.
                                    let end = off + data.len() as u64;
                                    let mut pos = *off;
                                    while pos < end {
                                        let unit = self.tear_unit;
                                        let next = ((pos / unit) + 1) * unit;
                                        let piece_end = next.min(end);
                                        if rng.chance(1, 2) {
                                            let a = (pos - off) as usize;
                                            let b = (piece_end - off) as usize;
                                            write_into(&mut content, pos, &data[a..b]);
                                        }
                                        pos = piece_end;
                                    }
                                }
                            },
                        }
                    }
                    content
                }
            };
            inodes.insert(ino, Inode { data: content.clone(), durable: content, pending: Vec::new(), pushed: 0 });
        }
        let mut image = State::new(rng.fork());
        image.inodes = inodes;
        image.next_ino = self.next_ino;
        if kind == CrashKind::ProcessKillKeepPending {
            image.durable_names = self.durable_names.clone();
            image.pending_ns = self.pending_ns.clone();
            image.durable_dirs = self.durable_dirs.clone();
        } else {
            image.durable_names = names.clone();
            image.durable_dirs = dirs.clone();
        }
        image.names = names;
        image.dirs = dirs;
        image.tear_unit = self.tear_unit;
        image.fsync_failure = self.fsync_failure;
        image.dir_durability = self.dir_durability;
        image.space_limit = self.space_limit;
        image.sync_latency = self.sync_latency;
        image
    }

    fn inode(&mut self, ino: u64) -> &mut Inode {
        self.inodes.get_mut(&ino).expect("open file refers to a live inode")
    }

    fn check_parent(&self, path: &Path) -> io::Result<()> {
        match path.parent() {
            Some(p) if !self.dirs.contains(p) => {
                Err(io::Error::new(io::ErrorKind::NotFound, format!("directory {} does not exist", p.display())))
            }
            _ => Ok(()),
        }
    }

    /// Visible bytes of every file that is reachable by name or still open:
    /// as on Linux, an unlinked file keeps its space until its last handle
    /// is closed.
    fn used_bytes(&self) -> u64 {
        let mut live: BTreeSet<u64> = self.names.values().copied().collect();
        live.extend(self.handles.iter().filter(|(_, token)| token.strong_count() > 0).map(|(ino, _)| *ino));
        live.iter().map(|ino| self.inodes[ino].data.len() as u64).sum()
    }

    /// Fails with `StorageFull` if growing `ino` to `new_len` bytes would
    /// exceed the space limit.
    fn check_space(&self, ino: u64, new_len: u64) -> io::Result<()> {
        let Some(limit) = self.space_limit else { return Ok(()) };
        let current = self.inodes[&ino].data.len() as u64;
        if new_len > current && self.used_bytes() + (new_len - current) > limit {
            return Err(disk_full_error());
        }
        Ok(())
    }

    /// A token that marks `ino` as open for as long as it lives.
    fn open_handle(&mut self, ino: u64) -> Arc<()> {
        if let Some(token) = self.handles.get(&ino).and_then(Weak::upgrade) {
            return token;
        }
        let token = Arc::new(());
        self.handles.insert(ino, Arc::downgrade(&token));
        token
    }
}

/// See the [module documentation](self).
#[derive(Clone)]
pub struct SimVfs {
    state: Arc<Mutex<State>>,
}

impl SimVfs {
    pub fn new(seed: u64) -> Self {
        SimVfs::from_state(State::new(Rng::new(seed)))
    }

    fn from_state(state: State) -> Self {
        SimVfs { state: Arc::new(Mutex::new(state)) }
    }

    pub fn as_vfs(&self) -> Arc<dyn Vfs> {
        Arc::new(self.clone())
    }

    /// Sets the granularity (bytes, >= 1) at which power loss can tear a
    /// pending write. The default, 512, models sector-atomic devices; 1
    /// models devices that guarantee nothing, which catches designs that
    /// rely on small records being written atomically.
    pub fn set_tear_unit(&self, unit: u64) {
        self.state.lock().tear_unit = unit.max(1);
    }

    /// Chooses what a failed `sync_data` does to its pending writes (see
    /// [`FsyncFailure`]).
    pub fn set_fsync_failure(&self, mode: FsyncFailure) {
        self.state.lock().fsync_failure = mode;
    }

    /// Chooses which namespace operations `sync_dir` makes durable (see
    /// [`DirDurability`]). Set it before creating directories.
    pub fn set_dir_durability(&self, mode: DirDurability) {
        self.state.lock().dir_durability = mode;
    }

    /// Caps the bytes in use (visible bytes of every file that is linked or
    /// still open): a write or `set_len` that would grow past it fails with
    /// [`io::ErrorKind::StorageFull`] and has no effect. Shrinking a file
    /// frees space; removing or replacing one frees it once its last handle
    /// is closed, as on Linux. `None` removes the cap. Either way it also
    /// cancels a pending [`fill_disk_after`](Self::fill_disk_after).
    pub fn set_space_limit(&self, limit: Option<u64>) {
        let mut s = self.state.lock();
        s.space_limit = limit;
        s.fill_at = None;
    }

    /// The disk becomes full right before the `n`-th mutating operation from
    /// now (`n >= 1`): the space limit is set to the bytes in use at that
    /// moment, so that operation and every later one that grows a file fail
    /// with `StorageFull` until space is freed or the limit is lifted.
    pub fn fill_disk_after(&self, n: u64) {
        let mut s = self.state.lock();
        s.fill_at = Some(s.mutating_ops + n.max(1));
    }

    /// Bytes in use, as counted for [`set_space_limit`](Self::set_space_limit).
    pub fn space_used(&self) -> u64 {
        self.state.lock().used_bytes()
    }

    /// Makes `sync_data` take at least `latency` (zero disables it). The
    /// sleep happens before the sync, outside the global lock, so other
    /// threads keep reading and writing meanwhile; the sync then takes
    /// effect atomically, as one mutating operation, and persists exactly
    /// the file's writes issued before `sync_data` was called, in order.
    /// Writes issued while it sleeps stay pending: a later sync must cover
    /// them.
    pub fn set_sync_latency(&self, latency: Duration) {
        self.state.lock().sync_latency = (!latency.is_zero()).then_some(latency);
    }

    /// Number of mutating operations (writes, syncs, creates, renames, ...)
    /// performed so far.
    pub fn mutating_ops(&self) -> u64 {
        self.state.lock().mutating_ops
    }

    /// Crash right before the `n`-th mutating operation from now (`n >= 1`).
    pub fn crash_after(&self, n: u64, kind: CrashKind) {
        let mut s = self.state.lock();
        let at = s.mutating_ops + n.max(1);
        s.trigger = Some((at, kind));
    }

    /// Crash right before the `nth` (`>= 1`) mutating operation from now for
    /// which `pred` returns `true`. `pred` runs under the file system's lock
    /// and must not call back into it.
    pub fn crash_when(&self, pred: impl Fn(&OpInfo) -> bool + Send + Sync + 'static, nth: u64, kind: CrashKind) {
        self.state.lock().when_crash = Some(WhenArm { pred: Arc::new(pred), remaining: nth.max(1), action: kind });
    }

    /// Makes the `n`-th mutating operation from now (`n >= 1`) fail with an
    /// I/O error, once. With `then = None` the device keeps working; with
    /// `Some(kind)` it crashes right before the next mutating operation, so
    /// no later sync can make up for the failed one.
    pub fn fail_after(&self, n: u64, then: Option<CrashKind>) {
        let mut s = self.state.lock();
        s.fault_at = Some(s.mutating_ops + n.max(1));
        s.fault_then = then;
        s.fault_injected = false;
    }

    /// Like [`fail_after`](Self::fail_after), for the `nth` (`>= 1`)
    /// mutating operation from now for which `pred` returns `true`. `pred`
    /// runs under the file system's lock and must not call back into it.
    pub fn fail_when(&self, pred: impl Fn(&OpInfo) -> bool + Send + Sync + 'static, nth: u64, then: Option<CrashKind>) {
        let mut s = self.state.lock();
        s.when_fault = Some(WhenArm { pred: Arc::new(pred), remaining: nth.max(1), action: then });
        s.fault_injected = false;
    }

    /// Whether a fault armed by [`fail_after`](Self::fail_after) or
    /// [`fail_when`](Self::fail_when) fired.
    pub fn fault_injected(&self) -> bool {
        self.state.lock().fault_injected
    }

    /// Cancels every pending trigger that has not fired yet:
    /// [`crash_after`](Self::crash_after), [`crash_when`](Self::crash_when),
    /// [`fail_after`](Self::fail_after), [`fail_when`](Self::fail_when) and
    /// [`fill_disk_after`](Self::fill_disk_after).
    pub fn disarm(&self) {
        let mut s = self.state.lock();
        s.trigger = None;
        s.fault_at = None;
        s.fault_then = None;
        s.when_crash = None;
        s.when_fault = None;
        s.fill_at = None;
    }

    /// Starts (or restarts, discarding earlier entries) recording every
    /// mutating operation; see [`trace`](Self::trace).
    pub fn start_trace(&self) {
        self.state.lock().trace = Some(Vec::new());
    }

    /// The operations recorded since [`start_trace`](Self::start_trace), in
    /// order, numbered like [`crash_after`](Self::crash_after) counts them.
    /// An operation that a trigger refused (crash or injected fault) is
    /// included: it is the point that trigger hit. Empty if tracing was
    /// never started.
    pub fn trace(&self) -> Vec<TraceEntry> {
        self.state.lock().trace.clone().unwrap_or_default()
    }

    /// Crashes immediately (if not already crashed) and returns the disk as
    /// it looks after reboot.
    pub fn crash_now(&self, kind: CrashKind) -> SimVfs {
        let mut s = self.state.lock();
        if !s.dead {
            s.crash(kind);
        }
        let image = s.image.take().expect("crash image");
        SimVfs::from_state(*image)
    }

    /// The post-reboot disk if a triggered crash already happened.
    pub fn take_crash_image(&self) -> Option<SimVfs> {
        let mut s = self.state.lock();
        s.image.take().map(|image| SimVfs::from_state(*image))
    }

    pub fn has_crashed(&self) -> bool {
        self.state.lock().dead
    }

    /// All files and their current lengths.
    pub fn files(&self) -> Vec<(PathBuf, u64)> {
        let s = self.state.lock();
        s.names.iter().map(|(p, ino)| (p.clone(), s.inodes[ino].data.len() as u64)).collect()
    }

    /// The visible bytes of every file, the directories, and the bytes of
    /// files that were unlinked while still open.
    pub fn dump(&self) -> DirImage {
        let s = self.state.lock();
        let files = s.names.iter().map(|(p, ino)| (p.clone(), s.inodes[ino].data.clone())).collect();
        let linked: BTreeSet<u64> = s.names.values().copied().collect();
        let mut unlinked: Vec<u64> = s
            .handles
            .iter()
            .filter(|(ino, token)| token.strong_count() > 0 && !linked.contains(*ino))
            .map(|(ino, _)| *ino)
            .collect();
        unlinked.sort_unstable();
        DirImage {
            dirs: s.dirs.clone(),
            files,
            unlinked_open: unlinked.iter().map(|ino| s.inodes[ino].data.clone()).collect(),
        }
    }

    /// A new file system holding `image` (its directories and files;
    /// unlinked open files are gone), with everything durable.
    pub fn restore(image: &DirImage) -> SimVfs {
        SimVfs::restore_seeded(image, 0)
    }

    /// [`restore`](Self::restore) with the given seed for later crashes.
    pub fn restore_seeded(image: &DirImage, seed: u64) -> SimVfs {
        let mut s = State::new(Rng::new(seed));
        s.dirs = image.dirs.clone();
        for path in image.files.keys() {
            if let Some(parent) = path.parent() {
                s.dirs.extend(parent.ancestors().map(Path::to_path_buf));
            }
        }
        s.durable_dirs = s.dirs.clone();
        for (path, data) in &image.files {
            let ino = s.next_ino;
            s.next_ino += 1;
            s.inodes.insert(ino, Inode { data: data.clone(), durable: data.clone(), pending: Vec::new(), pushed: 0 });
            s.names.insert(path.clone(), ino);
        }
        s.durable_names = s.names.clone();
        SimVfs::from_state(s)
    }

    /// XORs `mask` into the byte at `offset` of `path`, both in the visible
    /// and the durable image (simulates media corruption).
    pub fn corrupt_byte(&self, path: &Path, offset: u64, mask: u8) {
        let mut s = self.state.lock();
        let ino = s.names[path];
        let inode = s.inode(ino);
        assert!(inode.pending.is_empty(), "corrupt only quiescent files");
        inode.data[offset as usize] ^= mask;
        inode.durable[offset as usize] ^= mask;
    }

    /// Like [`corrupt_byte`](Self::corrupt_byte), but allowed while the file
    /// has pending writes (damage during use): the byte is flipped in the
    /// visible image, in the durable image (if that long) and in every
    /// pending write that covers it, so the damage survives any crash.
    pub fn corrupt_byte_live(&self, path: &Path, offset: u64, mask: u8) {
        let mut s = self.state.lock();
        let ino = *s.names.get(path).unwrap_or_else(|| panic!("no file {}", path.display()));
        let inode = s.inode(ino);
        let at = offset as usize;
        assert!(at < inode.data.len(), "offset {offset} is past the end of {}", path.display());
        inode.data[at] ^= mask;
        if at < inode.durable.len() {
            inode.durable[at] ^= mask;
        }
        for (_, p) in &mut inode.pending {
            if let Pending::Write { off, data } = p
                && offset >= *off
                && offset < *off + data.len() as u64
            {
                data[(offset - *off) as usize] ^= mask;
            }
        }
    }
}

struct SimFile {
    state: Arc<Mutex<State>>,
    ino: u64,
    /// The path it was opened under (for traces once it is unlinked).
    path: PathBuf,
    _handle: Arc<()>,
}

impl SimFile {
    fn op(&self, kind: OpKind, offset: u64, len: u64) -> OpDesc<'_> {
        OpDesc { kind, target: Target::File { ino: self.ino, opened_as: &self.path }, offset, len }
    }
}

impl File for SimFile {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let mut s = self.state.lock();
        s.check_alive()?;
        let data = &s.inode(self.ino).data;
        let start = offset as usize;
        let end = start + buf.len();
        if end > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "read past end of file"));
        }
        buf.copy_from_slice(&data[start..end]);
        Ok(())
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let mut s = self.state.lock();
        s.tick(self.op(OpKind::WriteAt, offset, buf.len() as u64)).map_err(Refused::into_io)?;
        s.check_space(self.ino, offset + buf.len() as u64)?;
        let inode = s.inode(self.ino);
        write_into(&mut inode.data, offset, buf);
        inode.pushed += 1;
        inode.pending.push((inode.pushed, Pending::Write { off: offset, data: buf.to_vec() }));
        Ok(())
    }

    fn len(&self) -> io::Result<u64> {
        let mut s = self.state.lock();
        s.check_alive()?;
        Ok(s.inode(self.ino).data.len() as u64)
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        let mut s = self.state.lock();
        s.tick(self.op(OpKind::SetLen, 0, len)).map_err(Refused::into_io)?;
        s.check_space(self.ino, len)?;
        let inode = s.inode(self.ino);
        inode.data.resize(len as usize, 0);
        inode.pushed += 1;
        inode.pending.push((inode.pushed, Pending::SetLen(len)));
        Ok(())
    }

    fn sync_data(&self) -> io::Result<()> {
        let mut s = self.state.lock();
        // A sync covers the writes issued before it was called. Without a
        // latency that is every pending write; with one, writes issued while
        // it sleeps stay pending (the sync may not have seen them).
        let mut issued_before = None;
        if let Some(latency) = s.sync_latency {
            s.check_alive()?;
            issued_before = Some(s.inode(self.ino).pushed);
            drop(s);
            std::thread::sleep(latency);
            s = self.state.lock();
        }
        let result = s.tick(self.op(OpKind::SyncData, 0, 0));
        let drop_pages = s.fsync_failure == FsyncFailure::LinuxDropPages;
        let inode = s.inode(self.ino);
        // `pending` is in push order, so the covered entries are a prefix.
        let covered = match issued_before {
            None => inode.pending.len(),
            Some(upto) => inode.pending.partition_point(|(n, _)| *n <= upto),
        };
        match result {
            Ok(()) => {}
            Err(Refused::Injected) => {
                if drop_pages {
                    // The dirty pages are marked clean: these writes stay
                    // visible but will never reach the disk. A length change
                    // is metadata, not a dirty page: it stays pending, in
                    // order, and the next successful sync persists it.
                    let mut kept: Vec<_> =
                        inode.pending.drain(..covered).filter(|(_, p)| matches!(p, Pending::SetLen(_))).collect();
                    kept.append(&mut inode.pending);
                    inode.pending = kept;
                }
                return Err(injected_error());
            }
            Err(Refused::Dead) => return Err(dead_error()),
        }
        for (_, p) in inode.pending.drain(..covered) {
            apply_pending(&mut inode.durable, &p);
        }
        Ok(())
    }
}

struct SimLock {
    state: Arc<Mutex<State>>,
    path: PathBuf,
}

impl Drop for SimLock {
    fn drop(&mut self) {
        self.state.lock().locks.remove(&self.path);
    }
}

impl Vfs for SimVfs {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        let mut s = self.state.lock();
        s.check_alive()?;
        match s.dir_durability {
            DirDurability::Global => {
                for a in path.ancestors() {
                    s.dirs.insert(a.to_path_buf());
                    s.durable_dirs.insert(a.to_path_buf());
                }
            }
            DirDurability::PerDir => {
                let missing: Vec<PathBuf> =
                    path.ancestors().filter(|a| !s.dirs.contains(*a)).map(Path::to_path_buf).collect();
                for dir in missing.into_iter().rev() {
                    s.dirs.insert(dir.clone());
                    if dir.parent().is_none() {
                        s.durable_dirs.insert(dir);
                    } else {
                        s.pending_ns.push(NsOp::Mkdir { path: dir });
                    }
                }
            }
        }
        Ok(())
    }

    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
        let mut s = self.state.lock();
        s.check_alive()?;
        let existing = s.names.get(path).copied();
        let ino = match (existing, mode) {
            (Some(_), OpenMode::CreateNew) => {
                return Err(io::Error::new(io::ErrorKind::AlreadyExists, "file exists"));
            }
            (Some(ino), _) => ino,
            (None, OpenMode::Existing) => {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("{} not found", path.display())));
            }
            (None, _) => {
                s.check_parent(path)?;
                s.tick(OpDesc { kind: OpKind::Create, target: Target::Path(path), offset: 0, len: 0 })
                    .map_err(Refused::into_io)?;
                let ino = s.next_ino;
                s.next_ino += 1;
                s.inodes.insert(ino, Inode::default());
                s.names.insert(path.to_path_buf(), ino);
                s.pending_ns.push(NsOp::Link { path: path.to_path_buf(), ino });
                ino
            }
        };
        let handle = s.open_handle(ino);
        Ok(Arc::new(SimFile { state: self.state.clone(), ino, path: path.to_path_buf(), _handle: handle }))
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        let s = self.state.lock();
        s.check_alive()?;
        Ok(s.names.contains_key(path) || s.dirs.contains(path))
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let mut s = self.state.lock();
        s.check_alive()?;
        if !s.names.contains_key(path) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
        }
        s.tick(OpDesc { kind: OpKind::Remove, target: Target::Path(path), offset: 0, len: 0 })
            .map_err(Refused::into_io)?;
        s.names.remove(path);
        s.pending_ns.push(NsOp::Unlink { path: path.to_path_buf() });
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut s = self.state.lock();
        s.check_alive()?;
        if !s.names.contains_key(from) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such file"));
        }
        s.check_parent(to)?;
        s.tick(OpDesc { kind: OpKind::Rename, target: Target::Rename(from, to), offset: 0, len: 0 })
            .map_err(Refused::into_io)?;
        let ino = s.names.remove(from).unwrap();
        s.names.insert(to.to_path_buf(), ino);
        s.pending_ns.push(NsOp::Rename { from: from.to_path_buf(), to: to.to_path_buf(), ino });
        Ok(())
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        let s = self.state.lock();
        s.check_alive()?;
        if !s.dirs.contains(dir) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such directory"));
        }
        Ok(s.names
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect())
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        let mut s = self.state.lock();
        s.check_alive()?;
        if !s.dirs.contains(dir) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such directory"));
        }
        s.tick(OpDesc { kind: OpKind::SyncDir, target: Target::Path(dir), offset: 0, len: 0 })
            .map_err(Refused::into_io)?;
        let mode = s.dir_durability;
        let ops = std::mem::take(&mut s.pending_ns);
        let State { durable_names, durable_dirs, pending_ns, .. } = &mut *s;
        match mode {
            DirDurability::Global => {
                // Journaled metadata: syncing a directory commits every
                // namespace operation issued before it, in order.
                for op in &ops {
                    apply_ns(durable_names, durable_dirs, op);
                }
                debug_assert!(s.durable_names == s.names);
            }
            DirDurability::PerDir => {
                // The operations on entries of `dir`, plus every earlier
                // operation they depend on, transitively (see
                // `NsOp::depends_on`), applied in journal order. Without the
                // dependencies, a rename into `dir` would be persisted
                // before the creation of its source in another directory,
                // and syncing that directory later would bring the source
                // name back next to the destination.
                let mut take: Vec<bool> = ops.iter().map(|op| op.dir() == Some(dir)).collect();
                for i in (0..ops.len()).rev() {
                    // Only later operations mark earlier ones, so `take[i]`
                    // is final here.
                    if take[i] {
                        for j in 0..i {
                            if !take[j] && ops[i].depends_on(&ops[j]) {
                                take[j] = true;
                            }
                        }
                    }
                }
                for (op, take) in ops.into_iter().zip(take) {
                    if take {
                        apply_ns(durable_names, durable_dirs, &op);
                    } else {
                        pending_ns.push(op);
                    }
                }
            }
        }
        Ok(())
    }

    fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
        let mut s = self.state.lock();
        s.check_alive()?;
        if s.locks.contains(path) {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "lock held"));
        }
        s.check_parent(path)?;
        if !s.names.contains_key(path) {
            // The lock file behaves like a pre-existing durable file.
            let ino = s.next_ino;
            s.next_ino += 1;
            s.inodes.insert(ino, Inode::default());
            s.names.insert(path.to_path_buf(), ino);
            s.durable_names.insert(path.to_path_buf(), ino);
        }
        s.locks.insert(path.to_path_buf());
        Ok(Box::new(SimLock { state: self.state.clone(), path: path.to_path_buf() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn synced_data_survives_power_loss() {
        for seed in 0..50 {
            let vfs = SimVfs::new(seed);
            vfs.create_dir_all(&path("/db")).unwrap();
            let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
            f.write_at(b"hello", 0).unwrap();
            f.sync_data().unwrap();
            vfs.sync_dir(&path("/db")).unwrap();
            f.write_at(b"world", 5).unwrap();
            let after = vfs.crash_now(CrashKind::PowerLoss);
            let g = after.open(&path("/db/a"), OpenMode::Existing).unwrap();
            let mut buf = [0u8; 5];
            g.read_at(&mut buf, 0).unwrap();
            assert_eq!(&buf, b"hello");
            assert!(g.len().unwrap() == 5 || g.len().unwrap() == 10);
        }
    }

    #[test]
    fn unsynced_directory_entries_may_vanish() {
        let mut vanished = 0;
        for seed in 0..50 {
            let vfs = SimVfs::new(seed);
            vfs.create_dir_all(&path("/db")).unwrap();
            let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
            f.write_at(b"x", 0).unwrap();
            f.sync_data().unwrap();
            let after = vfs.crash_now(CrashKind::PowerLoss);
            if !after.exists(&path("/db/a")).unwrap() {
                vanished += 1;
            }
        }
        assert!(vanished > 0 && vanished < 50);
    }

    #[test]
    fn process_kill_keeps_everything_written() {
        let vfs = SimVfs::new(1);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        f.write_at(b"abc", 0).unwrap();
        let after = vfs.crash_now(CrashKind::ProcessKill);
        let g = after.open(&path("/db/a"), OpenMode::Existing).unwrap();
        let mut buf = [0u8; 3];
        g.read_at(&mut buf, 0).unwrap();
        assert_eq!(&buf, b"abc");
    }

    #[test]
    fn trigger_stops_the_device() {
        let vfs = SimVfs::new(3);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        vfs.crash_after(2, CrashKind::PowerLoss);
        f.write_at(b"1", 0).unwrap();
        assert!(f.write_at(b"2", 1).is_err());
        assert!(f.read_at(&mut [0u8; 1], 0).is_err());
        assert!(vfs.has_crashed());
        assert!(vfs.take_crash_image().is_some());
    }

    #[test]
    fn injected_fault_fails_one_op_and_device_survives() {
        let vfs = SimVfs::new(5);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        vfs.sync_dir(&path("/db")).unwrap();
        f.write_at(b"abc", 0).unwrap();
        vfs.fail_after(1, None);
        assert!(f.sync_data().is_err());
        assert!(vfs.fault_injected());
        // The write is still pending (not durable) and the device works.
        let lost = (0..20).any(|seed| {
            let v = SimVfs::new(seed);
            v.create_dir_all(&path("/db")).unwrap();
            let g = v.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
            v.sync_dir(&path("/db")).unwrap();
            g.write_at(b"abc", 0).unwrap();
            v.fail_after(1, None);
            let _ = g.sync_data();
            v.crash_now(CrashKind::PowerLoss).open(&path("/db/a"), OpenMode::Existing).unwrap().len().unwrap() == 0
        });
        assert!(lost, "a write whose sync failed must not be guaranteed durable");
        f.write_at(b"d", 3).unwrap();
        f.sync_data().unwrap();
        let after = vfs.crash_now(CrashKind::PowerLoss);
        let g = after.open(&path("/db/a"), OpenMode::Existing).unwrap();
        let mut buf = [0u8; 4];
        g.read_at(&mut buf, 0).unwrap();
        assert_eq!(&buf, b"abcd");
    }

    fn pattern(tag: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| tag.wrapping_mul(31).wrapping_add((i % 251) as u8)).collect()
    }

    /// A fixed mix of writes (overlapping, extending, sparse, crossing sector
    /// boundaries), length changes, syncs, creations, renames over an open
    /// file and removals, with namespace operations both synced and pending.
    /// Stops at the first error so that it can also run into a trigger.
    fn fixed_sequence(vfs: &SimVfs) -> io::Result<()> {
        let d = path("/db");
        vfs.create_dir_all(&d)?;
        let a = vfs.open(&path("/db/a"), OpenMode::CreateNew)?;
        a.write_at(&pattern(1, 3000), 0)?;
        a.sync_data()?;
        vfs.sync_dir(&d)?;
        a.write_at(&pattern(2, 1500), 1000)?;
        a.write_at(&pattern(3, 700), 3000)?;
        a.set_len(3500)?;
        a.write_at(&pattern(4, 100), 4000)?;
        let b = vfs.open(&path("/db/b"), OpenMode::CreateNew)?;
        b.write_at(&pattern(5, 2000), 0)?;
        b.sync_data()?;
        let c = vfs.open(&path("/db/c.tmp"), OpenMode::CreateOrOpen)?;
        c.write_at(&pattern(6, 900), 0)?;
        c.sync_data()?;
        vfs.rename(&path("/db/c.tmp"), &path("/db/c"))?;
        vfs.sync_dir(&d)?;
        b.write_at(&pattern(7, 1200), 1500)?;
        let e = vfs.open(&path("/db/e"), OpenMode::CreateNew)?;
        e.write_at(&pattern(8, 600), 0)?;
        vfs.rename(&path("/db/e"), &path("/db/b"))?;
        vfs.remove_file(&path("/db/c"))?;
        a.write_at(&pattern(9, 513), 511)?;
        let f = vfs.open(&path("/db/f"), OpenMode::CreateNew)?;
        f.write_at(&pattern(10, 5000), 100)?;
        b.write_at(&pattern(11, 64), 3000)?;
        e.write_at(&pattern(12, 1100), 200)?;
        e.sync_data()?;
        Ok(())
    }

    /// crc32c over every file's name, length and bytes, in name order.
    fn fingerprint(vfs: &SimVfs) -> u32 {
        let mut crc = 0u32;
        for (p, len) in vfs.files() {
            crc = crc32c::crc32c_append(crc, p.as_os_str().as_encoded_bytes());
            crc = crc32c::crc32c_append(crc, &len.to_le_bytes());
            let mut buf = vec![0u8; len as usize];
            vfs.open(&p, OpenMode::Existing).unwrap().read_at(&mut buf, 0).unwrap();
            crc = crc32c::crc32c_append(crc, &buf);
        }
        crc
    }

    fn fresh() -> SimVfs {
        let vfs = SimVfs::new(42);
        vfs.set_tear_unit(1);
        vfs
    }

    /// Guards the default failure model: every opt-in extension must leave
    /// the default behaviour, including the number and order of RNG draws,
    /// bit-identical. The constants were recorded from the code before any
    /// extension existed; a change here silently shifts every fingerprint
    /// of every randomized crash test.
    #[test]
    fn default_mode_crash_images_unchanged() {
        let mut got = Vec::new();
        for kind in [CrashKind::PowerLoss, CrashKind::ProcessKill] {
            // Crash after the whole sequence.
            let vfs = fresh();
            fixed_sequence(&vfs).unwrap();
            let image = vfs.crash_now(kind);
            got.push(fingerprint(&image));
            // A second power loss on the post-reboot disk, after more writes
            // (exercises the image's own forked RNG).
            let g = image.open(&path("/db/a"), OpenMode::Existing).unwrap();
            g.write_at(&pattern(13, 700), 200).unwrap();
            g.sync_data().unwrap();
            g.write_at(&pattern(14, 333), 4000).unwrap();
            image.rename(&path("/db/a"), &path("/db/a2")).unwrap();
            got.push(fingerprint(&image.crash_now(CrashKind::PowerLoss)));
            // Crash triggered right before a given mutating operation.
            for at in [3, 9, 17, 22] {
                let vfs = fresh();
                vfs.crash_after(at, kind);
                assert!(fixed_sequence(&vfs).is_err());
                got.push(vfs.mutating_ops() as u32);
                got.push(fingerprint(&vfs.take_crash_image().unwrap()));
            }
            // A single injected fault, then a crash before the next operation.
            for at in [2, 5, 12] {
                let vfs = fresh();
                vfs.fail_after(at, Some(kind));
                assert!(fixed_sequence(&vfs).is_err());
                assert!(vfs.fault_injected());
                got.push(fingerprint(&vfs.crash_now(kind)));
            }
        }
        #[rustfmt::skip]
        let expected: [u32; 26] = [
            // PowerLoss: full sequence, chained power loss, 4 triggers
            // (ops count, image), 3 faults.
            3345036778, 3486118355,
            3, 0, 9, 3856496367, 17, 2477431979, 22, 1686931902,
            0, 92148084, 3856496367,
            // ProcessKill: same layout.
            1394652051, 1401971547,
            3, 92148084, 9, 1937810802, 17, 3645635133, 22, 4050293556,
            927471099, 92148084, 3739657026,
        ];
        assert_eq!(got, expected, "default SimVfs crash images changed");
    }

    #[test]
    fn torn_writes_happen() {
        let mut torn = 0;
        for seed in 0..200 {
            let vfs = SimVfs::new(seed);
            vfs.create_dir_all(&path("/d")).unwrap();
            let f = vfs.open(&path("/d/f"), OpenMode::CreateNew).unwrap();
            f.write_at(&[0u8; 4096], 0).unwrap();
            f.sync_data().unwrap();
            vfs.sync_dir(&path("/d")).unwrap();
            f.write_at(&[1u8; 4096], 0).unwrap();
            let after = vfs.crash_now(CrashKind::PowerLoss);
            let g = after.open(&path("/d/f"), OpenMode::Existing).unwrap();
            let mut buf = vec![0u8; 4096];
            g.read_at(&mut buf, 0).unwrap();
            let ones = buf.iter().filter(|&&b| b == 1).count();
            if ones != 0 && ones != 4096 {
                torn += 1;
                assert_eq!(ones % 512, 0);
            }
        }
        assert!(torn > 0);
    }

    fn read_all(vfs: &SimVfs, p: &str) -> Vec<u8> {
        let f = vfs.open(&path(p), OpenMode::Existing).unwrap();
        let mut buf = vec![0u8; f.len().unwrap() as usize];
        f.read_at(&mut buf, 0).unwrap();
        buf
    }

    /// `/db/a` holds "hello" durably (data and entry) and "world" pending;
    /// `/db/b` was created but its entry was never synced.
    fn hello_world(seed: u64) -> SimVfs {
        let vfs = SimVfs::new(seed);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        f.write_at(b"hello", 0).unwrap();
        f.sync_data().unwrap();
        vfs.sync_dir(&path("/db")).unwrap();
        f.write_at(b"world", 5).unwrap();
        let g = vfs.open(&path("/db/b"), OpenMode::CreateNew).unwrap();
        g.write_at(b"b", 0).unwrap();
        g.sync_data().unwrap();
        vfs
    }

    #[test]
    fn kill_keep_pending_then_power_loss_loses_unsynced() {
        let (mut lost_write, mut lost_entry) = (0, 0);
        for seed in 0..64 {
            let image = hello_world(seed).crash_now(CrashKind::ProcessKillKeepPending);
            let after = image.crash_now(CrashKind::PowerLoss);
            let a = read_all(&after, "/db/a");
            assert_eq!(&a[..5], b"hello", "synced data must survive");
            if a.len() == 5 {
                lost_write += 1;
            }
            if !after.exists(&path("/db/b")).unwrap() {
                lost_entry += 1;
            }
            // `ProcessKill` marks everything durable instead.
            let image = hello_world(seed).crash_now(CrashKind::ProcessKill);
            let after = image.crash_now(CrashKind::PowerLoss);
            assert_eq!(read_all(&after, "/db/a"), b"helloworld");
            assert!(after.exists(&path("/db/b")).unwrap());
        }
        assert!(lost_write > 0 && lost_write < 64, "unsynced write lost in {lost_write}/64 power losses");
        assert!(lost_entry > 0 && lost_entry < 64, "unsynced entry lost in {lost_entry}/64 power losses");
    }

    #[test]
    fn kill_keep_pending_preserves_visible_data() {
        let vfs = hello_world(3);
        let before = vfs.dump();
        let image = vfs.crash_now(CrashKind::ProcessKillKeepPending);
        assert_dir_unchanged(&before, &image.dump(), &[]);
        assert_eq!(read_all(&image, "/db/a"), b"helloworld");
        // The pending state is intact: syncing on the image persists it.
        for _ in 0..8 {
            let image = hello_world(3).crash_now(CrashKind::ProcessKillKeepPending);
            image.open(&path("/db/a"), OpenMode::Existing).unwrap().sync_data().unwrap();
            image.sync_dir(&path("/db")).unwrap();
            let after = image.crash_now(CrashKind::PowerLoss);
            assert_eq!(read_all(&after, "/db/a"), b"helloworld");
            assert_eq!(read_all(&after, "/db/b"), b"b");
        }
    }

    /// `/db/a` holds "abc" durably and "def" pending; the next mutating
    /// operation (the sync) fails.
    fn failed_sync(seed: u64, mode: FsyncFailure) -> (SimVfs, Arc<dyn File>) {
        let vfs = SimVfs::new(seed);
        vfs.set_fsync_failure(mode);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        f.write_at(b"abc", 0).unwrap();
        f.sync_data().unwrap();
        vfs.sync_dir(&path("/db")).unwrap();
        f.write_at(b"def", 3).unwrap();
        vfs.fail_after(1, None);
        assert!(f.sync_data().is_err());
        assert!(vfs.fault_injected());
        (vfs, f)
    }

    #[test]
    fn linux_drop_pages_failed_sync_loses_writes() {
        let mut kept_by_default = 0;
        for seed in 0..32 {
            let (vfs, _f) = failed_sync(seed, FsyncFailure::LinuxDropPages);
            assert_eq!(read_all(&vfs, "/db/a"), b"abcdef", "visible data is unchanged");
            let after = vfs.crash_now(CrashKind::PowerLoss);
            assert_eq!(read_all(&after, "/db/a"), b"abc", "dropped pages never reach the disk");
            let (vfs, _f) = failed_sync(seed, FsyncFailure::KeepPending);
            if read_all(&vfs.crash_now(CrashKind::PowerLoss), "/db/a") == b"abcdef" {
                kept_by_default += 1;
            }
        }
        assert!(kept_by_default > 0, "by default the writes stay pending and may still land");
    }

    #[test]
    fn linux_drop_pages_next_sync_succeeds() {
        let (vfs, f) = failed_sync(9, FsyncFailure::LinuxDropPages);
        f.sync_data().expect("the error was reported once; the next sync succeeds");
        // ... without persisting the dropped writes.
        let image = vfs.crash_now(CrashKind::PowerLoss);
        assert_eq!(read_all(&image, "/db/a"), b"abc");
        // Writing the data again is what makes it durable.
        let (vfs, f) = failed_sync(9, FsyncFailure::LinuxDropPages);
        f.sync_data().unwrap();
        f.write_at(b"def", 3).unwrap();
        f.sync_data().unwrap();
        assert_eq!(read_all(&vfs.crash_now(CrashKind::PowerLoss), "/db/a"), b"abcdef");
    }

    /// A failed sync drops dirty pages, not length changes: `ftruncate` is
    /// metadata, so the next successful sync still persists it.
    #[test]
    fn linux_drop_pages_keeps_length_changes() {
        for seed in 0..32 {
            let vfs = SimVfs::new(seed);
            vfs.set_fsync_failure(FsyncFailure::LinuxDropPages);
            vfs.create_dir_all(&path("/db")).unwrap();
            let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
            f.write_at(&pattern(1, 100), 0).unwrap();
            f.sync_data().unwrap();
            vfs.sync_dir(&path("/db")).unwrap();
            f.set_len(10).unwrap();
            f.write_at(b"lost", 2).unwrap();
            vfs.fail_after(1, None);
            assert!(f.sync_data().is_err());
            f.sync_data().unwrap();
            f.write_at(b"new", 10).unwrap();
            f.sync_data().unwrap();
            let mut expected = pattern(1, 10);
            expected[2..6].copy_from_slice(b"lost");
            expected.extend(b"new");
            assert_eq!(read_all(&vfs, "/db/a"), expected, "visible data is unchanged");
            let after = read_all(&vfs.crash_now(CrashKind::PowerLoss), "/db/a");
            let mut durable = pattern(1, 10);
            durable.extend(b"new");
            assert_eq!(after, durable, "seed {seed}: the truncation is durable, the dropped write is not");
        }
        // The length change stays pending, in order, until a sync succeeds.
        let mut truncated = 0;
        for seed in 0..32 {
            let vfs = SimVfs::new(seed);
            vfs.set_fsync_failure(FsyncFailure::LinuxDropPages);
            vfs.create_dir_all(&path("/db")).unwrap();
            let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
            f.write_at(&pattern(2, 100), 0).unwrap();
            f.sync_data().unwrap();
            vfs.sync_dir(&path("/db")).unwrap();
            f.set_len(10).unwrap();
            vfs.fail_after(1, None);
            assert!(f.sync_data().is_err());
            let len = read_all(&vfs.crash_now(CrashKind::PowerLoss), "/db/a").len();
            assert!(len == 10 || len == 100, "seed {seed}: length {len}");
            truncated += usize::from(len == 10);
        }
        assert!(truncated > 0 && truncated < 32, "an unsynced truncation may land or not: {truncated}/32");
    }

    fn two_dirs(seed: u64, mode: DirDurability) -> SimVfs {
        let vfs = SimVfs::new(seed);
        vfs.set_dir_durability(mode);
        vfs.create_dir_all(&path("/db")).unwrap();
        vfs.create_dir_all(&path("/wal")).unwrap();
        vfs.sync_dir(&path("/")).unwrap();
        vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        vfs.open(&path("/wal/b"), OpenMode::CreateNew).unwrap();
        vfs.sync_dir(&path("/db")).unwrap();
        vfs
    }

    #[test]
    fn per_dir_sync_commits_only_that_dir() {
        let mut lost = 0;
        for seed in 0..32 {
            let after = two_dirs(seed, DirDurability::PerDir).crash_now(CrashKind::PowerLoss);
            assert!(after.exists(&path("/db/a")).unwrap(), "the synced directory's entry is durable");
            if !after.exists(&path("/wal/b")).unwrap() {
                lost += 1;
            }
            let after = two_dirs(seed, DirDurability::Global).crash_now(CrashKind::PowerLoss);
            assert!(after.exists(&path("/wal/b")).unwrap(), "global: any sync_dir persists every entry");
        }
        assert!(lost > 0 && lost < 32, "unsynced directory's entry lost in {lost}/32 power losses");
        // Syncing the other directory persists it too.
        for seed in 0..8 {
            let vfs = two_dirs(seed, DirDurability::PerDir);
            vfs.sync_dir(&path("/wal")).unwrap();
            assert!(vfs.crash_now(CrashKind::PowerLoss).exists(&path("/wal/b")).unwrap());
        }
    }

    #[test]
    fn per_dir_new_dir_needs_parent_sync() {
        let setup = |seed| {
            let vfs = SimVfs::new(seed);
            vfs.set_dir_durability(DirDurability::PerDir);
            vfs.create_dir_all(&path("/db/sub")).unwrap();
            let f = vfs.open(&path("/db/sub/a"), OpenMode::CreateNew).unwrap();
            f.write_at(b"x", 0).unwrap();
            f.sync_data().unwrap();
            vfs.sync_dir(&path("/db/sub")).unwrap();
            vfs
        };
        let mut vanished = 0;
        for seed in 0..32 {
            let after = setup(seed).crash_now(CrashKind::PowerLoss);
            let sub = after.exists(&path("/db/sub")).unwrap();
            assert_eq!(after.exists(&path("/db/sub/a")).unwrap(), sub, "entries vanish with their directory");
            if !sub {
                vanished += 1;
                assert!(after.list(&path("/db/sub")).is_err());
            }
        }
        assert!(vanished > 0, "a directory whose parent was never synced must be able to vanish");
        for seed in 0..16 {
            let vfs = setup(seed);
            vfs.sync_dir(&path("/db")).unwrap();
            vfs.sync_dir(&path("/")).unwrap();
            let after = vfs.crash_now(CrashKind::PowerLoss);
            assert_eq!(read_all(&after, "/db/sub/a"), b"x");
        }
    }

    /// A rename across directories counts for the destination; syncing the
    /// destination also persists the earlier operations the rename depends
    /// on (the source's creation), so the source name never comes back.
    #[test]
    fn per_dir_cross_dir_rename_does_not_resurrect_source() {
        let setup = |seed| {
            let vfs = SimVfs::new(seed);
            vfs.set_dir_durability(DirDurability::PerDir);
            vfs.create_dir_all(&path("/a")).unwrap();
            vfs.create_dir_all(&path("/b")).unwrap();
            vfs.sync_dir(&path("/")).unwrap();
            let f = vfs.open(&path("/a/x"), OpenMode::CreateNew).unwrap();
            f.write_at(b"x", 0).unwrap();
            f.sync_data().unwrap();
            vfs.rename(&path("/a/x"), &path("/b/x")).unwrap();
            vfs
        };
        for seed in 0..32 {
            for syncs in [&["/b", "/a"][..], &["/a", "/b"], &["/b"]] {
                let vfs = setup(seed);
                for d in syncs {
                    vfs.sync_dir(&path(d)).unwrap();
                }
                let after = vfs.crash_now(CrashKind::PowerLoss);
                assert!(!after.exists(&path("/a/x")).unwrap(), "seed {seed}, syncs {syncs:?}: /a/x resurrected");
                assert_eq!(read_all(&after, "/b/x"), b"x", "seed {seed}, syncs {syncs:?}");
            }
            // A chain through a third directory, then an unlink in it.
            let vfs = setup(seed);
            vfs.create_dir_all(&path("/c")).unwrap();
            vfs.sync_dir(&path("/")).unwrap();
            vfs.rename(&path("/b/x"), &path("/c/x")).unwrap();
            vfs.open(&path("/b/y"), OpenMode::CreateNew).unwrap();
            vfs.sync_dir(&path("/c")).unwrap();
            let after = vfs.crash_now(CrashKind::PowerLoss);
            assert_eq!(after.list(&path("/a")).unwrap(), Vec::<String>::new(), "seed {seed}");
            assert!(!after.exists(&path("/b/x")).unwrap(), "seed {seed}");
            assert_eq!(read_all(&after, "/c/x"), b"x", "seed {seed}");
        }
        // Syncing only the source persists nothing of the rename: the file
        // survives under its old name or not at all (its creation was never
        // synced), never under both.
        for seed in 0..32 {
            let vfs = setup(seed);
            vfs.sync_dir(&path("/a")).unwrap();
            let after = vfs.crash_now(CrashKind::PowerLoss);
            let (a, b) = (after.exists(&path("/a/x")).unwrap(), after.exists(&path("/b/x")).unwrap());
            assert!(!(a && b), "seed {seed}: both names exist");
        }
    }

    #[test]
    fn space_limit_fails_growth_without_effect() {
        let vfs = SimVfs::new(1);
        vfs.create_dir_all(&path("/db")).unwrap();
        vfs.set_space_limit(Some(1000));
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        f.write_at(&[1u8; 600], 0).unwrap();
        let ops = vfs.mutating_ops();
        let e = f.write_at(&[2u8; 500], 600).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::StorageFull);
        assert_eq!(vfs.mutating_ops(), ops + 1, "a refused write is still a mutating operation");
        assert_eq!(read_all(&vfs, "/db/a"), vec![1u8; 600], "a refused write has no effect");
        f.write_at(&[3u8; 100], 0).unwrap(); // overwrite: no growth
        assert_eq!(f.set_len(1200).unwrap_err().kind(), io::ErrorKind::StorageFull);
        f.set_len(300).unwrap(); // shrinking always works
        f.write_at(&[4u8; 700], 300).unwrap(); // exactly at the limit
        assert_eq!(vfs.space_used(), 1000);
        assert_eq!(f.write_at(&[5u8], 1000).unwrap_err().kind(), io::ErrorKind::StorageFull);
        let g = vfs.open(&path("/db/b"), OpenMode::CreateNew).unwrap(); // creating takes no space
        assert_eq!(g.write_at(b"x", 0).unwrap_err().kind(), io::ErrorKind::StorageFull);
        f.sync_data().unwrap();
        // The refused writes never became pending either.
        let image = vfs.crash_now(CrashKind::ProcessKillKeepPending);
        let mut expected = vec![3u8; 100];
        expected.extend([1u8; 200]);
        expected.extend([4u8; 700]);
        assert_eq!(read_all(&image, "/db/a"), expected);
        assert_eq!(read_all(&image.crash_now(CrashKind::PowerLoss), "/db/a"), expected);
        // fill_disk_after: the disk is full from the n-th operation on.
        let vfs = SimVfs::new(2);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        vfs.fill_disk_after(2);
        f.write_at(b"one", 0).unwrap();
        assert_eq!(f.write_at(b"two", 3).unwrap_err().kind(), io::ErrorKind::StorageFull);
        f.write_at(b"ONE", 0).unwrap();
        vfs.set_space_limit(None);
        f.write_at(b"two", 3).unwrap();
        assert_eq!(read_all(&vfs, "/db/a"), b"ONEtwo");
    }

    #[test]
    fn space_limit_freed_by_remove() {
        let vfs = SimVfs::new(1);
        vfs.create_dir_all(&path("/db")).unwrap();
        vfs.set_space_limit(Some(1000));
        let a = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        a.write_at(&[1u8; 800], 0).unwrap();
        let b = vfs.open(&path("/db/b"), OpenMode::CreateNew).unwrap();
        assert_eq!(b.write_at(&[2u8; 300], 0).unwrap_err().kind(), io::ErrorKind::StorageFull);
        // Replacing a file by rename frees the old one's space too, once its
        // last handle is closed (as on Linux, an unlinked file keeps its
        // blocks while it is open).
        let c = vfs.open(&path("/db/c"), OpenMode::CreateNew).unwrap();
        c.write_at(&[3u8; 100], 0).unwrap();
        vfs.rename(&path("/db/c"), &path("/db/a")).unwrap();
        assert_eq!(vfs.space_used(), 900, "the replaced file is still open");
        assert_eq!(b.write_at(&[2u8; 300], 0).unwrap_err().kind(), io::ErrorKind::StorageFull);
        drop(a);
        assert_eq!(vfs.space_used(), 100);
        b.write_at(&[2u8; 300], 0).unwrap();
        // remove_file of an open file frees nothing until its last handle
        // is closed.
        let c_again = vfs.open(&path("/db/a"), OpenMode::Existing).unwrap();
        vfs.remove_file(&path("/db/a")).unwrap();
        assert_eq!(vfs.space_used(), 400, "an unlinked file holds its space while open");
        assert_eq!(b.write_at(&[2u8; 700], 300).unwrap_err().kind(), io::ErrorKind::StorageFull);
        drop(c);
        assert_eq!(vfs.space_used(), 400, "another handle still has it open");
        drop(c_again);
        assert_eq!(vfs.space_used(), 300);
        b.write_at(&[2u8; 700], 300).unwrap();
        assert_eq!(vfs.space_used(), 1000);
        // A closed file is freed at once.
        drop(b);
        vfs.remove_file(&path("/db/b")).unwrap();
        assert_eq!(vfs.space_used(), 0);
    }

    #[test]
    fn crash_when_matches_predicate() {
        let vfs = SimVfs::new(4);
        vfs.create_dir_all(&path("/db")).unwrap();
        // The second sync of the file now named /db/b (opened as b.tmp).
        vfs.crash_when(|op| op.kind == OpKind::SyncData && op.path == Path::new("/db/b"), 2, CrashKind::PowerLoss);
        let a = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        let b = vfs.open(&path("/db/b.tmp"), OpenMode::CreateNew).unwrap();
        a.write_at(b"a", 0).unwrap();
        a.sync_data().unwrap();
        b.write_at(b"b1", 0).unwrap();
        b.sync_data().unwrap(); // named b.tmp: no match
        vfs.rename(&path("/db/b.tmp"), &path("/db/b")).unwrap();
        vfs.sync_dir(&path("/db")).unwrap();
        b.sync_data().unwrap(); // first match
        a.sync_data().unwrap();
        b.write_at(b"b2", 2).unwrap();
        let before = vfs.mutating_ops();
        assert!(b.sync_data().is_err(), "second match: crash before it");
        assert!(vfs.has_crashed());
        assert_eq!(vfs.mutating_ops(), before + 1);
        assert!(a.sync_data().is_err(), "the device is gone");
        let image = vfs.take_crash_image().unwrap();
        assert_eq!(&read_all(&image, "/db/b")[..2], b"b1");
    }

    #[test]
    fn fail_when_matches_predicate() {
        let vfs = SimVfs::new(5);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        vfs.fail_when(|op| op.kind == OpKind::WriteAt && op.offset >= 100, 1, None);
        f.write_at(b"x", 0).unwrap();
        assert!(!vfs.fault_injected());
        assert!(f.write_at(b"y", 100).is_err());
        assert!(vfs.fault_injected());
        assert_eq!(f.len().unwrap(), 1, "the failed write had no effect");
        f.write_at(b"z", 200).unwrap(); // one-shot; the device keeps working
        f.sync_data().unwrap();
        vfs.sync_dir(&path("/db")).unwrap();
        assert!(!vfs.has_crashed());
        // With `then`, the device crashes right before the next operation.
        vfs.fail_when(|op| op.kind == OpKind::Rename, 1, Some(CrashKind::PowerLoss));
        f.write_at(b"w", 1).unwrap();
        assert!(vfs.rename(&path("/db/a"), &path("/db/b")).is_err());
        assert!(!vfs.has_crashed());
        assert!(f.sync_data().is_err());
        assert!(vfs.has_crashed());
        let image = vfs.take_crash_image().unwrap();
        assert!(image.exists(&path("/db/a")).unwrap());
        assert!(!image.exists(&path("/db/b")).unwrap());
    }

    #[test]
    fn trace_records_phase_and_thread() {
        let vfs = SimVfs::new(6);
        vfs.create_dir_all(&path("/db")).unwrap();
        let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        assert!(vfs.trace().is_empty(), "nothing is recorded before start_trace");
        vfs.start_trace();
        let start = vfs.mutating_ops();
        std::thread::Builder::new()
            .name("trace-writer".into())
            .spawn(move || {
                let _phase = kerno_core::probe::phase("flush");
                f.write_at(b"0123456789", 4).unwrap();
                f.set_len(20).unwrap();
                f.sync_data().unwrap();
            })
            .unwrap()
            .join()
            .unwrap();
        {
            let _phase = kerno_core::probe::phase("checkpoint");
            vfs.rename(&path("/db/a"), &path("/db/b")).unwrap();
        }
        vfs.sync_dir(&path("/db")).unwrap();
        let trace = vfs.trace();
        let main = std::thread::current().name().unwrap().to_owned();
        let summary: Vec<_> = trace
            .iter()
            .map(|t| (t.n - start, t.op.kind, t.op.path.clone(), t.op.offset, t.op.len, t.op.phase, t.thread.clone()))
            .collect();
        let writer = "trace-writer".to_owned();
        assert_eq!(
            summary,
            vec![
                (1, OpKind::WriteAt, path("/db/a"), 4, 10, "flush", writer.clone()),
                (2, OpKind::SetLen, path("/db/a"), 0, 20, "flush", writer.clone()),
                (3, OpKind::SyncData, path("/db/a"), 0, 0, "flush", writer),
                (4, OpKind::Rename, path("/db/a"), 0, 0, "checkpoint", main.clone()),
                (5, OpKind::SyncDir, path("/db"), 0, 0, "", main),
            ]
        );
        assert_eq!(trace[3].op.to.as_deref(), Some(Path::new("/db/b")));
        assert_eq!(trace.last().unwrap().n, vfs.mutating_ops());
    }

    #[test]
    fn dump_restore_round_trip() {
        let vfs = SimVfs::new(7);
        vfs.create_dir_all(&path("/db/sub")).unwrap();
        let a = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        a.write_at(b"durable", 0).unwrap();
        a.sync_data().unwrap();
        a.write_at(b" and pending", 7).unwrap();
        let b = vfs.open(&path("/db/sub/b"), OpenMode::CreateNew).unwrap();
        b.write_at(&[9u8; 3000], 0).unwrap();
        let gone = vfs.open(&path("/db/gone"), OpenMode::CreateNew).unwrap();
        gone.write_at(b"still open", 0).unwrap();
        vfs.remove_file(&path("/db/gone")).unwrap();
        let closed = vfs.open(&path("/db/closed"), OpenMode::CreateNew).unwrap();
        closed.write_at(b"closed", 0).unwrap();
        drop(closed);
        vfs.remove_file(&path("/db/closed")).unwrap();

        let image = vfs.dump();
        assert_eq!(image.files[&path("/db/a")], b"durable and pending");
        assert_eq!(image.files.len(), 2);
        assert_eq!(image.unlinked_open, vec![b"still open".to_vec()], "unlinked files only while open");
        assert!(image.dirs.contains(&path("/db/sub")));

        let restored = SimVfs::restore(&image);
        let again = restored.dump();
        assert_eq!(again.files, image.files);
        assert_eq!(again.dirs, image.dirs);
        assert!(again.unlinked_open.is_empty());
        assert_dir_unchanged(&image, &again, &[]);
        // Everything restored is durable.
        for seed in 0..8 {
            let r = SimVfs::restore_seeded(&image, seed);
            assert_dir_unchanged(&image, &r.crash_now(CrashKind::PowerLoss).dump(), &[]);
        }
        drop((a, b, gone));
    }

    #[test]
    fn assert_dir_unchanged_detects_byte_change() {
        let vfs = SimVfs::new(8);
        vfs.create_dir_all(&path("/db")).unwrap();
        for (name, len) in [("/db/MANIFEST", 100), ("/db/LOCK", 0), ("/db/000001.sst", 5000)] {
            let f = vfs.open(&path(name), OpenMode::CreateNew).unwrap();
            f.write_at(&pattern(len as u8, len), 0).unwrap();
            f.sync_data().unwrap();
        }
        vfs.sync_dir(&path("/db")).unwrap();
        let before = vfs.dump();
        assert_dir_unchanged(&before, &vfs.dump(), &[]);
        vfs.corrupt_byte(&path("/db/000001.sst"), 4097, 0x10);
        let after = vfs.dump();
        let msg = dir_difference(&before, &after, &[]).expect("a flipped bit is a change");
        assert!(msg.contains("000001.sst") && msg.contains("byte 4097"), "{msg}");
        let r = std::panic::catch_unwind(|| assert_dir_unchanged(&before, &after, &["LOCK"]));
        assert!(r.is_err());
        assert_dir_unchanged(&before, &after, &["000001.sst"]);
        assert_dir_unchanged(&before, &after, &["/db/000001.sst"]);
        // Length changes, creations and removals are changes too.
        vfs.open(&path("/db/LOCK"), OpenMode::Existing).unwrap().write_at(b"pid", 0).unwrap();
        assert!(dir_difference(&after, &vfs.dump(), &[]).unwrap().contains("LOCK"));
        assert_dir_unchanged(&after, &vfs.dump(), &["LOCK"]);
        vfs.open(&path("/db/new"), OpenMode::CreateNew).unwrap();
        assert!(dir_difference(&after, &vfs.dump(), &["LOCK"]).unwrap().contains("created"));
        vfs.remove_file(&path("/db/new")).unwrap();
        vfs.remove_file(&path("/db/MANIFEST")).unwrap();
        assert!(dir_difference(&after, &vfs.dump(), &["LOCK"]).unwrap().contains("removed"));
    }

    #[test]
    fn sync_latency_does_not_hold_global_lock() {
        use std::sync::Barrier;
        use std::time::Instant;

        let latency = Duration::from_millis(1000);
        let vfs = SimVfs::new(9);
        vfs.create_dir_all(&path("/db")).unwrap();
        let a = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        let b = vfs.open(&path("/db/b"), OpenMode::CreateNew).unwrap();
        vfs.sync_dir(&path("/db")).unwrap();
        a.write_at(b"before the sync", 0).unwrap();
        vfs.set_sync_latency(latency);
        let barrier = Arc::new(Barrier::new(2));
        let syncer = {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let t = Instant::now();
                a.sync_data().unwrap();
                t.elapsed()
            })
        };
        barrier.wait();
        std::thread::sleep(Duration::from_millis(100));
        // The syncer is asleep inside sync_data: everything else proceeds,
        // including a write to the very file being synced.
        let t = Instant::now();
        b.write_at(b"meanwhile", 0).unwrap();
        b.len().unwrap();
        vfs.files();
        let a_again = vfs.open(&path("/db/a"), OpenMode::Existing).unwrap();
        a_again.write_at(b"during the sync", 100).unwrap();
        let blocked = t.elapsed();
        let synced_in = syncer.join().unwrap();
        assert!(blocked < latency / 2, "a write waited {blocked:?} behind a sleeping sync");
        assert!(synced_in >= latency, "the sync returned after {synced_in:?}");
        // The sync persisted exactly what was written before it was called,
        // in order; the write issued while it slept is still pending.
        {
            let s = vfs.state.lock();
            let inode = &s.inodes[&s.names[&path("/db/a")]];
            assert_eq!(inode.durable, b"before the sync");
            assert_eq!(inode.pending.len(), 1);
        }
        vfs.set_sync_latency(Duration::ZERO);
        let image = vfs.crash_now(CrashKind::PowerLoss);
        assert_eq!(&read_all(&image, "/db/a")[..15], b"before the sync");
    }

    #[test]
    fn corrupt_byte_live_with_pending_writes() {
        let setup = |seed| {
            let vfs = SimVfs::new(seed);
            vfs.create_dir_all(&path("/db")).unwrap();
            let f = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
            f.write_at(b"abcdef", 0).unwrap();
            f.sync_data().unwrap();
            vfs.sync_dir(&path("/db")).unwrap();
            f.write_at(b"XYZ", 2).unwrap(); // pending over bytes 2..5
            vfs.corrupt_byte_live(&path("/db/a"), 3, 0x01);
            vfs.corrupt_byte_live(&path("/db/a"), 0, 0x20);
            vfs
        };
        let vfs = setup(1);
        assert_eq!(read_all(&vfs, "/db/a"), b"AbXXZf", "the reader sees the damage at once");
        assert_eq!(read_all(&vfs.crash_now(CrashKind::ProcessKill), "/db/a"), b"AbXXZf");
        let mut images = BTreeSet::new();
        for seed in 0..32 {
            let a = read_all(&setup(seed).crash_now(CrashKind::PowerLoss), "/db/a");
            assert_eq!(a[0], b'A', "damage outside the pending write survives");
            assert!(a[3] == b'X' || a[3] == b'e', "byte 3 is damaged in every version: {:?}", a[3] as char);
            images.insert(a);
        }
        assert!(images.len() > 1, "the pending write still lands or not");
    }
}
