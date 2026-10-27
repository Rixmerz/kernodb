use std::any::Any;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use super::{File, OpenMode, Vfs};

/// Cumulative I/O issued through a [`CountingVfs`].
#[derive(Debug, Default)]
pub struct IoCounters {
    pub bytes_written: AtomicU64,
    pub bytes_read: AtomicU64,
    pub writes: AtomicU64,
    pub reads: AtomicU64,
    pub file_syncs: AtomicU64,
    pub dir_syncs: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IoSnapshot {
    pub bytes_written: u64,
    pub bytes_read: u64,
    pub writes: u64,
    pub reads: u64,
    pub file_syncs: u64,
    pub dir_syncs: u64,
}

impl IoSnapshot {
    pub fn since(&self, earlier: &IoSnapshot) -> IoSnapshot {
        IoSnapshot {
            bytes_written: self.bytes_written - earlier.bytes_written,
            bytes_read: self.bytes_read - earlier.bytes_read,
            writes: self.writes - earlier.writes,
            reads: self.reads - earlier.reads,
            file_syncs: self.file_syncs - earlier.file_syncs,
            dir_syncs: self.dir_syncs - earlier.dir_syncs,
        }
    }
}

impl IoCounters {
    pub fn snapshot(&self) -> IoSnapshot {
        IoSnapshot {
            bytes_written: self.bytes_written.load(Relaxed),
            bytes_read: self.bytes_read.load(Relaxed),
            writes: self.writes.load(Relaxed),
            reads: self.reads.load(Relaxed),
            file_syncs: self.file_syncs.load(Relaxed),
            dir_syncs: self.dir_syncs.load(Relaxed),
        }
    }
}

/// Wraps another [`Vfs`] and counts the I/O that goes through it, so that
/// write/read amplification can be measured per engine.
pub struct CountingVfs {
    inner: Arc<dyn Vfs>,
    counters: Arc<IoCounters>,
}

impl CountingVfs {
    pub fn wrap(inner: Arc<dyn Vfs>) -> (Arc<dyn Vfs>, Arc<IoCounters>) {
        let counters = Arc::new(IoCounters::default());
        (Arc::new(CountingVfs { inner, counters: counters.clone() }), counters)
    }
}

struct CountingFile {
    inner: Arc<dyn File>,
    counters: Arc<IoCounters>,
}

impl File for CountingFile {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.counters.reads.fetch_add(1, Relaxed);
        self.counters.bytes_read.fetch_add(buf.len() as u64, Relaxed);
        self.inner.read_at(buf, offset)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        self.counters.writes.fetch_add(1, Relaxed);
        self.counters.bytes_written.fetch_add(buf.len() as u64, Relaxed);
        self.inner.write_at(buf, offset)
    }

    fn len(&self) -> io::Result<u64> {
        self.inner.len()
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        self.inner.set_len(len)
    }

    fn sync_data(&self) -> io::Result<()> {
        self.counters.file_syncs.fetch_add(1, Relaxed);
        self.inner.sync_data()
    }
}

impl Vfs for CountingVfs {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.inner.create_dir_all(path)
    }

    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
        let inner = self.inner.open(path, mode)?;
        Ok(Arc::new(CountingFile { inner, counters: self.counters.clone() }))
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
        self.counters.dir_syncs.fetch_add(1, Relaxed);
        self.inner.sync_dir(dir)
    }

    fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
        self.inner.lock(path)
    }
}
