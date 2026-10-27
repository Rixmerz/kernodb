use std::any::Any;
use std::fs;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::Arc;

use super::{File, OpenMode, Vfs};

/// The real file system (Unix only: relies on `pread`/`pwrite`).
#[derive(Clone, Copy, Debug, Default)]
pub struct StdVfs;

impl StdVfs {
    pub fn shared() -> Arc<dyn Vfs> {
        Arc::new(StdVfs)
    }
}

struct StdFile(fs::File);

impl File for StdFile {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        self.0.read_exact_at(buf, offset)
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        self.0.write_all_at(buf, offset)
    }

    fn len(&self) -> io::Result<u64> {
        Ok(self.0.metadata()?.len())
    }

    fn set_len(&self, len: u64) -> io::Result<()> {
        self.0.set_len(len)
    }

    fn sync_data(&self) -> io::Result<()> {
        self.0.sync_data()
    }
}

impl Vfs for StdVfs {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>> {
        let mut o = fs::OpenOptions::new();
        o.read(true).write(true);
        match mode {
            OpenMode::Existing => {}
            OpenMode::CreateOrOpen => {
                o.create(true);
            }
            OpenMode::CreateNew => {
                o.create_new(true);
            }
        }
        Ok(Arc::new(StdFile(o.open(path)?)))
    }

    fn exists(&self, path: &Path) -> io::Result<bool> {
        path.try_exists()
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    fn list(&self, dir: &Path) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                out.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        out.sort();
        Ok(out)
    }

    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        fs::File::open(dir)?.sync_all()
    }

    fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
        let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)?;
        match f.try_lock() {
            Ok(()) => Ok(Box::new(f)),
            Err(fs::TryLockError::WouldBlock) => Err(io::Error::new(io::ErrorKind::WouldBlock, "lock held")),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }
}
