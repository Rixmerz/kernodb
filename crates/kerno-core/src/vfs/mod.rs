//! File-system abstraction used by the candidate engines.
//!
//! Engines never touch `std::fs` directly. This lets the same engine code run
//! on the real file system ([`StdVfs`]), on a simulated one that models power
//! loss and torn writes (`kerno_testkit::vfs::SimVfs`), and under an I/O
//! accounting wrapper (`kerno_testkit::vfs::CountingVfs`).
//!
//! # Durability model (what engines may rely on)
//!
//! * `File::write_at` data is durable only after `File::sync_data` on that
//!   file returns `Ok`.
//! * Creating, removing and renaming directory entries is durable only after
//!   `Vfs::sync_dir` on the parent directory returns `Ok`.
//! * Nothing else is ordered: before a sync, any subset of pending writes may
//!   reach the disk, in any order, and a single write may be torn at 512-byte
//!   sector boundaries.
//! * `rename` atomically replaces the destination (POSIX semantics).

mod std_fs;

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub use std_fs::StdVfs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// Open an existing file for reading and writing.
    Existing,
    /// Open for reading and writing, creating it (empty) if missing.
    CreateOrOpen,
    /// Create a new file; fails if it exists.
    CreateNew,
}

#[allow(clippy::len_without_is_empty)]
pub trait File: Send + Sync {
    /// Reads exactly `buf.len()` bytes at `offset`
    /// (`ErrorKind::UnexpectedEof` if the file is shorter).
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
    /// Writes all of `buf` at `offset`, extending the file if needed.
    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()>;
    fn len(&self) -> io::Result<u64>;
    fn set_len(&self, len: u64) -> io::Result<()>;
    /// `fdatasync`: makes previous writes and length changes durable.
    fn sync_data(&self) -> io::Result<()>;
}

pub trait Vfs: Send + Sync + 'static {
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn open(&self, path: &Path, mode: OpenMode) -> io::Result<Arc<dyn File>>;
    fn exists(&self, path: &Path) -> io::Result<bool>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// Atomically renames `from` to `to`, replacing `to` if it exists.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Names (not full paths) of the files directly inside `dir`.
    fn list(&self, dir: &Path) -> io::Result<Vec<String>>;
    /// Makes entry creations, removals and renames in `dir` durable.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    /// Takes an exclusive advisory lock on `path` (created if missing),
    /// held until the returned guard is dropped. Fails with
    /// `ErrorKind::WouldBlock` if someone else holds it.
    fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>>;
}

/// Total size in bytes of the files directly inside `dir`.
///
/// Safe to call while an engine's background threads delete or rename
/// files: an entry that disappears between listing and opening no longer
/// takes space and is skipped.
pub fn dir_size(vfs: &dyn Vfs, dir: &Path) -> io::Result<u64> {
    let mut total = 0;
    for name in vfs.list(dir)? {
        let path: PathBuf = dir.join(name);
        match vfs.open(&path, OpenMode::Existing) {
            Ok(f) => total += f.len()?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}
