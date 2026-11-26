use std::any::Any;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::FileExt;
#[cfg(windows)]
use std::os::windows::fs::FileExt;
use std::path::Path;
use std::sync::Arc;

use super::{File, OpenMode, Vfs};

/// The real file system: `pread`/`pwrite` on Unix, `seek_read`/`seek_write`
/// on Windows. Durability is verified on Linux only (kerno-storage refuses
/// `Sync` elsewhere); this type makes no durability claim on Windows.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdVfs;

impl StdVfs {
    pub fn shared() -> Arc<dyn Vfs> {
        Arc::new(StdVfs)
    }
}

struct StdFile(fs::File, std::sync::OnceLock<Option<Mapping>>);

/// A read-only `mmap` of a whole file, unmapped on drop. Address and length as integers so
/// the file stays `Send + Sync`: the mapping is never written through.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Mapping {
    addr: usize,
    len: usize,
}

#[cfg(target_os = "linux")]
impl Mapping {
    fn new(file: &fs::File) -> Option<Mapping> {
        use std::os::fd::AsRawFd;

        let len = usize::try_from(file.metadata().ok()?.len()).ok()?;
        if len == 0 {
            return None;
        }
        // SAFETY: a fresh read-only shared mapping of `len` bytes of an open descriptor; the
        // kernel picks the address. Nothing is read until the call returns.
        let addr = unsafe {
            linux::mmap(std::ptr::null_mut(), len, linux::PROT_READ, linux::MAP_SHARED, file.as_raw_fd(), 0)
        };
        if addr as isize == -1 {
            return None;
        }
        Some(Mapping { addr: addr as usize, len })
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `addr..addr + len` is mapped readable for as long as `self` lives (it is
        // unmapped in `drop`), and the files mapped are not written again (`File::map_read_only`).
        unsafe { std::slice::from_raw_parts(self.addr as *const u8, self.len) }
    }
}

#[cfg(target_os = "linux")]
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: unmaps exactly the mapping `new` made; no slice of it outlives `self`.
        unsafe {
            linux::munmap(self.addr as *mut std::ffi::c_void, self.len);
        }
    }
}

/// `read_exact_at` over a positional read that may return fewer bytes than
/// asked (Windows `seek_read`): loops until `buf` is full, retries
/// `Interrupted`, and fails with `UnexpectedEof` when the read returns 0.
#[cfg(any(windows, test))]
fn read_exact_with(
    mut read: impl FnMut(&mut [u8], u64) -> io::Result<usize>,
    mut buf: &mut [u8],
    mut offset: u64,
) -> io::Result<()> {
    while !buf.is_empty() {
        match read(buf, offset) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "failed to fill whole buffer")),
            Ok(n) => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// `write_all_at` over a positional write that may write fewer bytes than
/// given (Windows `seek_write`): loops until `buf` is written, retries
/// `Interrupted`, and fails with `WriteZero` when the write returns 0.
#[cfg(any(windows, test))]
fn write_all_with(
    mut write: impl FnMut(&[u8], u64) -> io::Result<usize>,
    mut buf: &[u8],
    mut offset: u64,
) -> io::Result<()> {
    while !buf.is_empty() {
        match write(buf, offset) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "failed to write whole buffer")),
            Ok(n) => {
                buf = &buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl File for StdFile {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        #[cfg(unix)]
        return self.0.read_exact_at(buf, offset);
        #[cfg(windows)]
        return read_exact_with(|b, o| self.0.seek_read(b, o), buf, offset);
    }

    fn write_at(&self, buf: &[u8], offset: u64) -> io::Result<()> {
        #[cfg(unix)]
        return self.0.write_all_at(buf, offset);
        #[cfg(windows)]
        return write_all_with(|b, o| self.0.seek_write(b, o), buf, offset);
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

    #[cfg(target_os = "linux")]
    fn start_writeback(&self, offset: u64, len: u64) -> io::Result<()> {
        self.sync_range(offset, len, linux::SYNC_FILE_RANGE_WRITE)
    }

    #[cfg(target_os = "linux")]
    fn wait_writeback(&self, offset: u64, len: u64) -> io::Result<()> {
        self.sync_range(offset, len, linux::SYNC_FILE_RANGE_WAIT_BEFORE | linux::SYNC_FILE_RANGE_WAIT_AFTER)
    }

    #[cfg(target_os = "linux")]
    fn map_read_only(&self) -> Option<&[u8]> {
        self.1.get_or_init(|| Mapping::new(&self.0)).as_ref().map(Mapping::bytes)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    // Declared here instead of depending on `libc`: kerno-core has no
    // dependencies (CONTRIBUTING). std already links the C library that
    // exports it.
    unsafe extern "C" {
        pub fn sync_file_range(fd: i32, offset: i64, nbytes: i64, flags: u32) -> i32;
        pub fn mmap(
            addr: *mut std::ffi::c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut std::ffi::c_void;
        pub fn munmap(addr: *mut std::ffi::c_void, len: usize) -> i32;
    }
    pub const PROT_READ: i32 = 1;
    pub const MAP_SHARED: i32 = 1;
    /// Wait for the writeback already under way in the range.
    pub const SYNC_FILE_RANGE_WAIT_BEFORE: u32 = 1;
    /// Start the writeback of the dirty pages of the range, without waiting.
    pub const SYNC_FILE_RANGE_WRITE: u32 = 2;
    /// Wait for the writeback that is under way when the call ends.
    pub const SYNC_FILE_RANGE_WAIT_AFTER: u32 = 4;
}

#[cfg(target_os = "linux")]
impl StdFile {
    /// `sync_file_range(2)` on `[offset, offset + len)` (0 bytes: to the end
    /// of the file).
    fn sync_range(&self, offset: u64, len: u64, flags: u32) -> io::Result<()> {
        use std::os::fd::AsRawFd;

        let (Ok(offset), Ok(len)) = (i64::try_from(offset), i64::try_from(len)) else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "writeback range out of bounds"));
        };
        // SAFETY: plain system call on a file descriptor `self.0` owns for
        // the duration of the call; it reads no memory of ours.
        let rc = unsafe { linux::sync_file_range(self.0.as_raw_fd(), offset, len, flags) };
        if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
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
        Ok(Arc::new(StdFile(o.open(path)?, std::sync::OnceLock::new())))
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

    #[cfg(not(windows))]
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        fs::File::open(dir)?.sync_all()
    }

    // Windows cannot open a directory as a plain file (`Access is denied`),
    // and NTFS journals directory entries itself: there is nothing to flush.
    // Durability on Windows stays unverified (`allow_unverified_platform`).
    #[cfg(windows)]
    fn sync_dir(&self, _dir: &Path) -> io::Result<()> {
        Ok(())
    }

    fn lock(&self, path: &Path) -> io::Result<Box<dyn Any + Send + Sync>> {
        let f = match fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path) {
            Ok(f) => f,
            // A read-only medium or directory (a backup, a read-only mount):
            // lock the existing file without write access, or the directory
            // when `LOCK` was never created, so a read-only open still works.
            // ponytail: with no `LOCK` file the lock is on the directory, so
            // it does not exclude a writer that later creates `LOCK`; fine
            // while the directory is not writable, revisit if that changes.
            Err(e) if matches!(e.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem) => {
                match fs::File::open(path) {
                    Ok(f) => f,
                    Err(nf) if nf.kind() == io::ErrorKind::NotFound => {
                        fs::File::open(path.parent().unwrap_or(Path::new(".")))?
                    }
                    Err(_) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };
        match f.try_lock() {
            Ok(()) => Ok(Box::new(f)),
            Err(fs::TryLockError::WouldBlock) => Err(io::Error::new(io::ErrorKind::WouldBlock, "lock held")),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn start_writeback_runs_on_a_real_file_and_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("kerno-core-writeback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let vfs = StdVfs;
        vfs.create_dir_all(&dir).unwrap();
        let f = vfs.open(&dir.join("f"), OpenMode::CreateNew).unwrap();
        let data: Vec<u8> = (0..(1 << 20)).map(|i| (i % 251) as u8).collect();
        f.write_at(&data, 0).unwrap();
        f.start_writeback(0, 1 << 19).unwrap();
        // 0 bytes means "to the end of the file" for the system call; ranges
        // past the end are not an error either.
        f.start_writeback(1 << 19, 0).unwrap();
        f.start_writeback(1 << 40, 10).unwrap();
        f.wait_writeback(0, 1 << 19).unwrap();
        f.wait_writeback(0, 0).unwrap();
        // A range no `off_t` holds is refused before the call.
        assert_eq!(f.start_writeback(u64::MAX, 1).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(f.start_writeback(0, u64::MAX).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(f.wait_writeback(u64::MAX, 1).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        // Nothing about the file changed, and the data still syncs.
        assert_eq!(f.len().unwrap(), data.len() as u64);
        let mut back = vec![0u8; data.len()];
        f.read_at(&mut back, 0).unwrap();
        assert!(back == data);
        f.sync_data().unwrap();
        drop(f);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lock_works_in_a_directory_without_write_permission() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("kerno-core-ro-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let vfs = StdVfs;
        vfs.create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores the mode: then the directory is writable and there is nothing to show.
        if fs::File::create(dir.join("probe")).is_err() {
            let first = vfs.lock(&dir.join("LOCK")).expect("lock without LOCK in a read-only directory");
            assert!(!dir.join("LOCK").exists(), "a read-only lock must not create LOCK");
            // Held: a second lock on the same directory is refused.
            assert_eq!(vfs.lock(&dir.join("LOCK")).unwrap_err().kind(), io::ErrorKind::WouldBlock);
            drop(first);
            vfs.lock(&dir.join("LOCK")).expect("released lock can be taken again");
        }
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}

/// The Windows read/write loops, run on every platform against an in-memory
/// "file" that returns short reads and writes and an `Interrupted` now and then.
#[cfg(test)]
mod positional_loops {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// An in-memory file: each call moves at most 3 bytes, every 4th call
    /// fails with `Interrupted`.
    #[derive(Default)]
    struct Fake {
        data: RefCell<Vec<u8>>,
        calls: Cell<u32>,
    }

    impl Fake {
        fn interrupted(&self) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.calls.get().is_multiple_of(4)
        }

        fn read(&self, b: &mut [u8], o: u64) -> io::Result<usize> {
            if self.interrupted() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let d = self.data.borrow();
            let o = o as usize;
            let n = b.len().min(3).min(d.len().saturating_sub(o));
            b[..n].copy_from_slice(&d[o..o + n]);
            Ok(n)
        }

        fn write(&self, b: &[u8], o: u64) -> io::Result<usize> {
            if self.interrupted() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let mut d = self.data.borrow_mut();
            let o = o as usize;
            let n = b.len().min(3);
            if d.len() < o + n {
                d.resize(o + n, 0);
            }
            d[o..o + n].copy_from_slice(&b[..n]);
            Ok(n)
        }
    }

    #[test]
    fn short_reads_and_writes_are_looped_until_the_buffer_is_done() {
        let f = Fake::default();
        let payload: Vec<u8> = (0..100u8).collect();
        write_all_with(|b, o| f.write(b, o), &payload, 5).unwrap();
        assert_eq!(&f.data.borrow()[5..], &payload[..]);
        let mut back = vec![0u8; payload.len()];
        read_exact_with(|b, o| f.read(b, o), &mut back, 5).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn read_past_the_end_is_unexpected_eof_and_empty_buffers_do_nothing() {
        let f = Fake { data: RefCell::new(vec![1, 2, 3, 4]), ..Fake::default() };
        let mut buf = [0u8; 8];
        assert_eq!(read_exact_with(|b, o| f.read(b, o), &mut buf, 0).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        read_exact_with(|b, o| f.read(b, o), &mut [], 1 << 40).unwrap();
        write_all_with(|b, o| f.write(b, o), &[], 1 << 40).unwrap();
        assert_eq!(f.data.borrow().len(), 4);
    }

    #[test]
    fn a_zero_write_is_write_zero_and_other_errors_pass_through() {
        let err = write_all_with(|_, _| Ok(0), b"x", 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WriteZero);
        let err = read_exact_with(|_, _| Err(io::ErrorKind::PermissionDenied.into()), &mut [0u8; 1], 0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }
}
