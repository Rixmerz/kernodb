//! `inspect()`: what the headers of a database's files say (v1.md §9.5, RO-1).
//!
//! It takes the exclusive `LOCK`, reads the first bytes of every engine file
//! and writes nothing. It says which format versions and feature bits the
//! files carry, so a caller can tell what a database needs before opening it.

use std::io;
use std::path::Path;
use std::sync::Arc;

use kerno_core::vfs::{File, OpenMode, Vfs};
use kerno_core::{Error, Result};

use crate::files::{FileSource, format_error};
use crate::format::ByteSource;
use crate::format::header::{FileKind, HEADER_LEN, Header, HeaderExpect, decode_header};
use crate::format::limits::MAX_HEADER_LEN;
use crate::format::{FormatError, ReadError};
use crate::manifest::MANIFEST;
use crate::recovery::{self, Listing};
use crate::sst::sst_name;
use crate::wal::wal_name;

/// Which kind of engine file a header belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Manifest,
    Wal,
    Sst,
}

/// The fields of a valid common header (v1.md §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeaderInfo {
    pub format_version: u32,
    pub compat: u64,
    pub ro_compat: u64,
    pub incompat: u64,
    pub db_id: [u8; 16],
    /// WAL and SST: the number in the file name. MANIFEST: the generation.
    pub file_number: u64,
}

/// One engine file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    pub name: String,
    pub file_type: FileType,
    /// Length of the file in bytes.
    pub len: u64,
    /// `None` when the header does not decode (`problem` says why). A WAL
    /// whose creation a crash cut short legitimately has no valid header
    /// (WAL-R6); a damaged file of any kind does too.
    pub header: Option<HeaderInfo>,
    /// The rule the header violates, with its offset: never key or value bytes.
    pub problem: Option<String>,
}

/// What [`inspect`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormatInfo {
    /// Every canonical engine file in the directory: the MANIFEST first,
    /// then the WALs and the SSTs by number.
    pub files: Vec<FileInfo>,
}

impl FileType {
    fn kind(self) -> FileKind {
        match self {
            FileType::Manifest => FileKind::Manifest,
            FileType::Wal => FileKind::Wal,
            FileType::Sst => FileKind::Sst,
        }
    }
}

/// Reads and checks the header of an open file: HDR-1..HDR-10 against
/// `expect`. Reads 96 bytes, or the longer `header_len` a future version
/// declares, which its CRC needs before the version can be rejected.
pub(crate) fn read_header(
    file: &dyn File,
    kind: FileKind,
    expect: &HeaderExpect,
) -> std::result::Result<Header, ReadError<io::Error>> {
    let mut src = FileSource::new(file).map_err(ReadError::Source)?;
    let len = src.len();
    let mut prefix = vec![0; len.min(HEADER_LEN as u64) as usize];
    src.read_exact_at(0, &mut prefix).map_err(ReadError::Source)?;
    if prefix.len() == HEADER_LEN {
        let declared = u64::from(u32::from_le_bytes(prefix[12..16].try_into().expect("four bytes")));
        if declared > HEADER_LEN as u64 && declared <= len.min(u64::from(MAX_HEADER_LEN)) {
            prefix.resize(declared as usize, 0);
            src.read_exact_at(0, &mut prefix).map_err(ReadError::Source)?;
        }
    }
    Ok(decode_header(&prefix, len, kind, expect)?)
}

impl HeaderInfo {
    pub(crate) fn of(h: &Header) -> HeaderInfo {
        HeaderInfo {
            format_version: h.format_version,
            compat: h.compat,
            ro_compat: h.ro_compat,
            incompat: h.incompat,
            db_id: h.db_id,
            file_number: h.file_number,
        }
    }
}

/// The description of one file whose header was read: a header that does not
/// decode is `Corruption` data in the result, an unsupported one (`Incompatible`)
/// or an I/O failure is the error.
pub(crate) fn describe(file: &dyn File, name: &str, file_type: FileType, expect: &HeaderExpect) -> Result<FileInfo> {
    let len = file.len()?;
    match read_header(file, file_type.kind(), expect) {
        Ok(h) => {
            Ok(FileInfo { name: name.to_string(), file_type, len, header: Some(HeaderInfo::of(&h)), problem: None })
        }
        Err(ReadError::Source(e)) => Err(Error::Io(e)),
        Err(ReadError::Format(e)) => header_problem(name, file_type, len, e),
    }
}

fn header_problem(name: &str, file_type: FileType, len: u64, e: FormatError) -> Result<FileInfo> {
    if !e.is_corruption() {
        return Err(format_error(name, e));
    }
    let problem = Some(format!("{}: {} at offset {}: {}", name, e.rule, e.offset, e.detail));
    Ok(FileInfo { name: name.to_string(), file_type, len, header: None, problem })
}

/// The canonical engine files of `listing`, with the type and the name they
/// have on disk.
pub(crate) fn engine_files(listing: &Listing) -> Vec<(String, FileType, Option<u64>)> {
    let mut out = Vec::new();
    if listing.has_manifest {
        out.push((MANIFEST.to_string(), FileType::Manifest, None));
    }
    out.extend(listing.wals.iter().map(|&n| (wal_name(n), FileType::Wal, Some(n))));
    out.extend(listing.ssts.iter().map(|&n| (sst_name(n), FileType::Sst, Some(n))));
    out
}

/// Takes the `LOCK` of `dir` (`Locked` if a handle or another process has
/// it) and lists the directory under it.
pub(crate) fn lock_and_list(vfs: &dyn Vfs, dir: &Path) -> Result<(Box<dyn std::any::Any + Send + Sync>, Listing)> {
    let lock = vfs.lock(&dir.join("LOCK")).map_err(|e| match e.kind() {
        io::ErrorKind::WouldBlock => Error::Locked,
        _ => Error::Io(e),
    })?;
    let listing = recovery::list_dir(vfs, dir)?;
    Ok((lock, listing))
}

/// Reads the headers of every engine file in `dir`. It writes nothing; the
/// only file it may create is `LOCK` (RO-1).
///
/// Errors: `Locked` if the database is open; `InvalidArgument` if `dir` holds
/// no database or is not a Kerno directory; `Incompatible` if a header
/// declares a version or a feature this engine does not support (M0 files
/// included); `Io`. A header that fails a check because of damage is not an
/// error: the file is listed with `header: None` and the rule it violates.
pub fn inspect(vfs: Arc<dyn Vfs>, dir: &Path) -> Result<FormatInfo> {
    let first = recovery::list_dir(&*vfs, dir)?;
    refuse_directory_without_database(&first, dir)?;
    let (_lock, listing) = lock_and_list(&*vfs, dir)?;
    let mut info = FormatInfo::default();
    for (name, file_type, number) in engine_files(&listing) {
        let file = match vfs.open(&dir.join(&name), OpenMode::Existing) {
            Ok(f) => f,
            // A table deleted since the listing was taken: see RO-4.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let expect = HeaderExpect { db_id: None, file_number: number };
        info.files.push(describe(&*file, &name, file_type, &expect)?);
    }
    Ok(info)
}

/// What `open` says before it takes the lock about a directory that cannot be
/// a database: nothing to inspect or verify is `InvalidArgument`, and so is a
/// directory of foreign files. Data files without a MANIFEST are a damaged
/// database: the caller goes on and reports them.
pub(crate) fn refuse_directory_without_database(listing: &Listing, dir: &Path) -> Result<()> {
    if listing.has_manifest {
        return Ok(());
    }
    match recovery::refuse_without_manifest(listing) {
        Some(e @ Error::InvalidArgument(_)) => Err(e),
        Some(_) => Ok(()),
        None => Err(Error::InvalidArgument(format!("{}: no database (no {MANIFEST})", dir.display()))),
    }
}
