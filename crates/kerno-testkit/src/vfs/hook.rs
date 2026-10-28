//! Generic fault-injecting wrapper around any [`Vfs`].
//!
//! [`HookVfs`] calls a hook before and after every operation (file-system
//! and file operations alike, reads included) with a description of it. The
//! hook's [`HookAction`] decides what happens: proceed, fail with a chosen
//! [`io::ErrorKind`], panic, or sleep first. Before the operation, `Fail`
//! means it never happens; after it, `Fail` means it took effect but the
//! caller still gets the error (a device that lies about failure). Tests use
//! it for errors and panics at precise points of an engine's protocol, and
//! for random delays that shake out races.

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::{File, OpenMode, Vfs};

/// Which operation a hook call describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HookOpKind {
    CreateDirAll,
    Open,
    Exists,
    Remove,
    Rename,
    List,
    SyncDir,
    Lock,
    ReadAt,
    WriteAt,
    Len,
    SetLen,
    SyncData,
}

/// Whether the hook runs before or after the operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookPoint {
    Before,
    After,
}

/// One hook call.
#[derive(Clone, Copy, Debug)]
pub struct HookOp<'a> {
    pub kind: HookOpKind,
    pub point: HookPoint,
    /// The file or directory operated on (the source of a rename; for file
    /// operations, the path the file was opened under).
    pub path: &'a Path,
    /// Destination of a rename.
    pub to: Option<&'a Path>,
    /// `Open`: the mode. Otherwise `None`.
    pub mode: Option<OpenMode>,
    /// `ReadAt`/`WriteAt`: the offset. Otherwise 0.
    pub offset: u64,
    /// `ReadAt`/`WriteAt`: bytes read or written. `SetLen`: the new length.
    /// Otherwise 0.
    pub len: u64,
    /// [`kerno_core::probe::current_phase`] of the calling thread.
    pub phase: &'static str,
}

/// What a [`HookVfs`] does at a hook call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookAction {
    Proceed,
    /// Return an error of this kind (before: the operation does not happen;
    /// after: it did, but the caller is told it failed).
    Fail(io::ErrorKind),
    /// Panic in the calling thread.
    Panic,
    /// Sleep, then proceed.
    Delay(Duration),
}

type Hook = Arc<dyn Fn(&HookOp<'_>) -> HookAction + Send + Sync>;

/// See the [module documentation](self).
#[derive(Clone)]
pub struct HookVfs {
    inner: Arc<dyn Vfs>,
    hook: Hook,
}

impl HookVfs {
    /// Wraps `inner`; `hook` is called before and after every operation, on
    /// the calling thread.
    pub fn new(inner: Arc<dyn Vfs>, hook: impl Fn(&HookOp<'_>) -> HookAction + Send + Sync + 'static) -> Self {
        HookVfs { inner, hook: Arc::new(hook) }
    }

    /// [`new`](Self::new), as a shared [`Vfs`].
    pub fn wrap(inner: Arc<dyn Vfs>, hook: impl Fn(&HookOp<'_>) -> HookAction + Send + Sync + 'static) -> Arc<dyn Vfs> {
        Arc::new(HookVfs::new(inner, hook))
    }
}

/// Describes an operation for both hook calls.
#[derive(Clone, Copy)]
struct Call<'a> {
    kind: HookOpKind,
    path: &'a Path,
    to: Option<&'a Path>,
    mode: Option<OpenMode>,
    offset: u64,
    len: u64,
}

impl<'a> Call<'a> {
    fn new(kind: HookOpKind, path: &'a Path) -> Self {
        Call { kind, path, to: None, mode: None, offset: 0, len: 0 }
    }
}

fn act(hook: &Hook, call: Call<'_>, point: HookPoint) -> io::Result<()> {
    let op = HookOp {
        kind: call.kind,
        point,
        path: call.path,
        to: call.to,
        mode: call.mode,
        offset: call.offset,
        len: call.len,
        phase: kerno_core::probe::current_phase(),
    };
    match hook(&op) {
        HookAction::Proceed => Ok(()),
        HookAction::Fail(kind) => Err(io::Error::new(
            kind,
            format!("injected by HookVfs ({:?} {:?} {})", point, call.kind, call.path.display()),
        )),
        HookAction::Panic => panic!("injected panic by HookVfs ({:?} {:?} {})", point, call.kind, call.path.display()),
        HookAction::Delay(d) => {
            std::thread::sleep(d);
            Ok(())
        }
    }
}

/// Runs `f` between the two hook calls.
fn hooked<T>(hook: &Hook, call: Call<'_>, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    act(hook, call, HookPoint::Before)?;
    let result = f();
    act(hook, call, HookPoint::After)?;
    result
}

struct HookFile {
    inner: Arc<dyn File>,
    hook: Hook,
    path: PathBuf,
}

impl HookFile {
    fn call(&self, kind: HookOpKind, offset: u64, len: u64) -> Call<'_> {
        Call { offset, len, ..Call::new(kind, &self.path) }
    }
}

impl File for HookFile {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        let call = self.call(HookOpKind::ReadAt, offset, buf.len() as u64);
        hooked(&self.hook, call, || self.inner.read_at(buf, offset))
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        let call = self.call(HookOpKind::WriteAt, offset, buf.len() as u64);
        hooked(&self.hook, call, || self.inner.write_at(buf, offset))
    }

    fn len(&self) -> io::Result<u64> {
        hooked(&self.hook, self.call(HookOpKind::Len, 0, 0), || self.inner.len())
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        hooked(&self.hook, self.call(HookOpKind::SetLen, 0, len), || self.inner.set_len(len))
    }

    fn sync_data(&self) -> io::Result<()> {
        hooked(&self.hook, self.call(HookOpKind::SyncData, 0, 0), || self.inner.sync_data())
    }
}

impl Vfs for HookVfs {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        hooked(&self.hook, Call::new(HookOpKind::CreateDirAll, path), || self.inner.create_dir_all(path))
    }

    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
        let call = Call { mode: Some(mode), ..Call::new(HookOpKind::Open, path) };
        let inner = hooked(&self.hook, call, || self.inner.open(path, mode))?;
        Ok(Arc::new(HookFile { inner, hook: self.hook.clone(), path: path.to_path_buf() }))
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        hooked(&self.hook, Call::new(HookOpKind::Exists, path), || self.inner.exists(path))
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        hooked(&self.hook, Call::new(HookOpKind::Remove, path), || self.inner.remove_file(path))
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let call = Call { to: Some(to), ..Call::new(HookOpKind::Rename, from) };
        hooked(&self.hook, call, || self.inner.rename(from, to))
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        hooked(&self.hook, Call::new(HookOpKind::List, dir), || self.inner.list(dir))
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        hooked(&self.hook, Call::new(HookOpKind::SyncDir, dir), || self.inner.sync_dir(dir))
    }

    fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
        hooked(&self.hook, Call::new(HookOpKind::Lock, path), || self.inner.lock(path))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Instant;

    use kerno_core::WriteBatch;
    use kerno_core::contract::{Engine, Options};
    use parking_lot::Mutex;

    use super::*;
    use crate::reference::LogEngine;
    use crate::vfs::{CrashKind, SimVfs};

    fn path(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn hook_vfs_fail_and_delay() {
        let sim = SimVfs::new(1);
        let seen = Arc::new(Mutex::new(BTreeSet::new()));
        let log = seen.clone();
        let vfs = HookVfs::wrap(sim.as_vfs(), move |op| {
            log.lock().insert(op.kind);
            match (op.kind, op.point) {
                (HookOpKind::WriteAt, HookPoint::Before) if op.offset >= 100 => {
                    HookAction::Fail(io::ErrorKind::PermissionDenied)
                }
                (HookOpKind::SyncData, HookPoint::After) if op.path.ends_with("b") => {
                    HookAction::Fail(io::ErrorKind::Interrupted)
                }
                (HookOpKind::SyncDir, HookPoint::Before) => HookAction::Delay(Duration::from_millis(50)),
                _ => HookAction::Proceed,
            }
        });
        vfs.create_dir_all(&path("/db")).unwrap();
        let a = vfs.open(&path("/db/a"), OpenMode::CreateNew).unwrap();
        a.write_at(b"0123", 0).unwrap();
        // Before + Fail: the write never happens.
        let ops = sim.mutating_ops();
        let e = a.write_at(b"x", 100).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        assert!(e.to_string().contains("HookVfs"), "{e}");
        assert_eq!(sim.mutating_ops(), ops, "the inner file system never saw the write");
        assert_eq!(a.len().unwrap(), 4);
        a.sync_data().unwrap();
        // After + Fail: the sync happened, but the caller is told it failed.
        let b = vfs.open(&path("/db/b"), OpenMode::CreateNew).unwrap();
        b.write_at(b"synced anyway", 0).unwrap();
        assert_eq!(b.sync_data().unwrap_err().kind(), io::ErrorKind::Interrupted);
        // Delay.
        let t = Instant::now();
        vfs.sync_dir(&path("/db")).unwrap();
        assert!(t.elapsed() >= Duration::from_millis(50));
        let mut buf = [0u8; 4];
        a.read_at(&mut buf, 0).unwrap();
        vfs.exists(&path("/db/a")).unwrap();
        vfs.list(&path("/db")).unwrap();
        vfs.rename(&path("/db/a"), &path("/db/c")).unwrap();
        vfs.remove_file(&path("/db/c")).unwrap();
        a.set_len(2).unwrap();
        drop(vfs.lock(&path("/db/LOCK")).unwrap());
        let image = sim.crash_now(CrashKind::PowerLoss);
        let f = image.open(&path("/db/b"), OpenMode::Existing).unwrap();
        let mut buf = vec![0u8; f.len().unwrap() as usize];
        f.read_at(&mut buf, 0).unwrap();
        assert_eq!(buf, b"synced anyway");
        // The hook saw every kind of operation.
        use HookOpKind::*;
        let all =
            [CreateDirAll, Open, Exists, Remove, Rename, List, SyncDir, Lock, ReadAt, WriteAt, Len, SetLen, SyncData];
        assert_eq!(*seen.lock(), all.into_iter().collect::<BTreeSet<_>>());
    }

    #[test]
    fn hook_vfs_panic_propagates() {
        let sim = SimVfs::new(2);
        let armed = Arc::new(AtomicBool::new(false));
        let trigger = armed.clone();
        let vfs = HookVfs::wrap(sim.as_vfs(), move |op| {
            if trigger.load(Ordering::SeqCst) && op.kind == HookOpKind::WriteAt && op.point == HookPoint::Before {
                HookAction::Panic
            } else {
                HookAction::Proceed
            }
        });
        // Directly.
        vfs.create_dir_all(&path("/x")).unwrap();
        let f = vfs.open(&path("/x/f"), OpenMode::CreateNew).unwrap();
        armed.store(true, Ordering::SeqCst);
        let ops = sim.mutating_ops();
        let r = catch_unwind(AssertUnwindSafe(|| f.write_at(b"x", 0)));
        let msg = *r.unwrap_err().downcast::<String>().unwrap();
        assert!(msg.contains("injected panic by HookVfs") && msg.contains("WriteAt"), "{msg}");
        assert_eq!(sim.mutating_ops(), ops, "the panic happens before the operation");
        armed.store(false, Ordering::SeqCst);
        // Through an engine call, to the engine's caller.
        let db = LogEngine::open(vfs.clone(), Path::new("/db"), &Options::small_for_tests()).unwrap();
        let mut b = WriteBatch::new();
        b.put("k", "v");
        db.commit(&b).unwrap();
        armed.store(true, Ordering::SeqCst);
        let r = catch_unwind(AssertUnwindSafe(|| db.commit(&b)));
        assert!(r.is_err(), "the panic reaches the caller of commit");
    }
}
