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
//!   page cache survives a process crash).
//! * [`CrashKind::PowerLoss`] keeps only durable state plus a random subset
//!   of the pending writes: each one is dropped, applied, or torn at 512-byte
//!   sector granularity (or a finer unit set with [`SimVfs::set_tear_unit`]),
//!   independently and in no particular order. Pending namespace operations
//!   survive as a random *prefix* (journaled metadata).
//!
//! Crashes can be triggered at an exact mutating operation with
//! [`SimVfs::crash_after`]; from then on the old instance fails every call,
//! like a machine that lost power.
//!
//! A single I/O error with the device still alive can be injected with
//! [`SimVfs::fail_after`]: that one operation returns an error and has no
//! effect (a failed `sync_data` leaves its writes pending, i.e. not
//! durable), and everything else keeps working. This is how the harness
//! checks that a failed `fsync` is never acknowledged and poisons the
//! handle. Linux's stronger "dirty pages are dropped after a failed fsync"
//! behaviour is not modelled.
//!
//! Simplification: directories are durable as soon as they are created.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;

use super::{File, OpenMode, Vfs};
use crate::rng::Rng;

const SECTOR: u64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashKind {
    PowerLoss,
    ProcessKill,
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
    pending: Vec<Pending>,
}

#[derive(Clone)]
enum NsOp {
    Link { path: PathBuf, ino: u64 },
    Unlink { path: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
}

fn apply_ns(names: &mut BTreeMap<PathBuf, u64>, op: &NsOp) {
    match op {
        NsOp::Link { path, ino } => {
            names.insert(path.clone(), *ino);
        }
        NsOp::Unlink { path } => {
            names.remove(path);
        }
        NsOp::Rename { from, to } => {
            if let Some(ino) = names.remove(from) {
                names.insert(to.clone(), ino);
            }
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

struct State {
    inodes: HashMap<u64, Inode>,
    next_ino: u64,
    names: BTreeMap<PathBuf, u64>,
    durable_names: BTreeMap<PathBuf, u64>,
    pending_ns: Vec<NsOp>,
    dirs: BTreeSet<PathBuf>,
    locks: BTreeSet<PathBuf>,
    mutating_ops: u64,
    trigger: Option<(u64, CrashKind)>,
    fault_at: Option<u64>,
    fault_then: Option<CrashKind>,
    fault_injected: bool,
    dead: bool,
    image: Option<Box<State>>,
    /// Granularity at which a pending write can be torn by a power loss.
    tear_unit: u64,
    rng: Rng,
}

fn dead_error() -> io::Error {
    io::Error::other("simulated crash: device is gone")
}

fn injected_error() -> io::Error {
    io::Error::other("injected I/O error (EIO)")
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
            locks: BTreeSet::new(),
            mutating_ops: 0,
            trigger: None,
            fault_at: None,
            fault_then: None,
            fault_injected: false,
            dead: false,
            image: None,
            tear_unit: SECTOR,
            rng,
        }
    }

    fn check_alive(&self) -> io::Result<()> {
        if self.dead { Err(dead_error()) } else { Ok(()) }
    }

    /// Accounts one mutating operation; fires the crash trigger or the
    /// injected fault if due. Either way the operation is *not* performed.
    fn tick(&mut self) -> io::Result<()> {
        self.check_alive()?;
        self.mutating_ops += 1;
        if let Some((at, kind)) = self.trigger
            && self.mutating_ops >= at
        {
            self.crash(kind);
            return Err(dead_error());
        }
        if self.fault_at == Some(self.mutating_ops) {
            self.fault_at = None;
            self.fault_injected = true;
            if let Some(kind) = self.fault_then.take() {
                self.trigger = Some((self.mutating_ops + 1, kind));
            }
            return Err(injected_error());
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
        let names = match kind {
            CrashKind::ProcessKill => self.names.clone(),
            CrashKind::PowerLoss => {
                let mut names = self.durable_names.clone();
                let keep = rng.below(self.pending_ns.len() as u64 + 1) as usize;
                for op in &self.pending_ns[..keep] {
                    apply_ns(&mut names, op);
                }
                names
            }
        };
        let mut inodes = HashMap::new();
        for &ino in names.values() {
            if inodes.contains_key(&ino) {
                continue;
            }
            let inode = &self.inodes[&ino];
            let content = match kind {
                CrashKind::ProcessKill => inode.data.clone(),
                CrashKind::PowerLoss => {
                    let mut content = inode.durable.clone();
                    for p in &inode.pending {
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
            inodes.insert(ino, Inode { data: content.clone(), durable: content, pending: Vec::new() });
        }
        let mut image = State::new(rng.fork());
        image.inodes = inodes;
        image.next_ino = self.next_ino;
        image.durable_names = names.clone();
        image.names = names;
        image.dirs = self.dirs.clone();
        image.tear_unit = self.tear_unit;
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
}

/// See the [module documentation](self).
#[derive(Clone)]
pub struct SimVfs {
    state: Arc<Mutex<State>>,
}

impl SimVfs {
    pub fn new(seed: u64) -> Self {
        SimVfs { state: Arc::new(Mutex::new(State::new(Rng::new(seed)))) }
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

    /// Whether the fault armed by [`fail_after`](Self::fail_after) fired.
    pub fn fault_injected(&self) -> bool {
        self.state.lock().fault_injected
    }

    /// Cancels a pending [`crash_after`](Self::crash_after) trigger and a
    /// pending [`fail_after`](Self::fail_after) fault (no effect on ones
    /// that already fired).
    pub fn disarm(&self) {
        let mut s = self.state.lock();
        s.trigger = None;
        s.fault_at = None;
        s.fault_then = None;
    }

    /// Crashes immediately (if not already crashed) and returns the disk as
    /// it looks after reboot.
    pub fn crash_now(&self, kind: CrashKind) -> SimVfs {
        let mut s = self.state.lock();
        if !s.dead {
            s.crash(kind);
        }
        let image = s.image.take().expect("crash image");
        SimVfs { state: Arc::new(Mutex::new(*image)) }
    }

    /// The post-reboot disk if a triggered crash already happened.
    pub fn take_crash_image(&self) -> Option<SimVfs> {
        let mut s = self.state.lock();
        s.image.take().map(|image| SimVfs { state: Arc::new(Mutex::new(*image)) })
    }

    pub fn has_crashed(&self) -> bool {
        self.state.lock().dead
    }

    /// All files and their current lengths.
    pub fn files(&self) -> Vec<(PathBuf, u64)> {
        let s = self.state.lock();
        s.names.iter().map(|(p, ino)| (p.clone(), s.inodes[ino].data.len() as u64)).collect()
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
}

struct SimFile {
    state: Arc<Mutex<State>>,
    ino: u64,
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
        s.tick()?;
        let inode = s.inode(self.ino);
        write_into(&mut inode.data, offset, buf);
        inode.pending.push(Pending::Write { off: offset, data: buf.to_vec() });
        Ok(())
    }

    fn len(&self) -> io::Result<u64> {
        let mut s = self.state.lock();
        s.check_alive()?;
        Ok(s.inode(self.ino).data.len() as u64)
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        let mut s = self.state.lock();
        s.tick()?;
        let inode = s.inode(self.ino);
        inode.data.resize(len as usize, 0);
        inode.pending.push(Pending::SetLen(len));
        Ok(())
    }

    fn sync_data(&self) -> io::Result<()> {
        let mut s = self.state.lock();
        s.tick()?;
        let inode = s.inode(self.ino);
        let pending = std::mem::take(&mut inode.pending);
        for p in &pending {
            apply_pending(&mut inode.durable, p);
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
        for a in path.ancestors() {
            s.dirs.insert(a.to_path_buf());
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
                s.tick()?;
                let ino = s.next_ino;
                s.next_ino += 1;
                s.inodes.insert(ino, Inode::default());
                s.names.insert(path.to_path_buf(), ino);
                s.pending_ns.push(NsOp::Link { path: path.to_path_buf(), ino });
                ino
            }
        };
        Ok(Arc::new(SimFile { state: self.state.clone(), ino }))
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
        s.tick()?;
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
        s.tick()?;
        let ino = s.names.remove(from).unwrap();
        s.names.insert(to.to_path_buf(), ino);
        s.pending_ns.push(NsOp::Rename { from: from.to_path_buf(), to: to.to_path_buf() });
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
        s.tick()?;
        // Journaled metadata: syncing a directory commits every namespace
        // operation issued before it, in order.
        let ops = std::mem::take(&mut s.pending_ns);
        for op in &ops {
            apply_ns(&mut s.durable_names, op);
        }
        debug_assert!(s.durable_names == s.names);
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
}
