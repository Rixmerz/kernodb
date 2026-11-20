//! Reading the engine's files through the [`Vfs`](kerno_core::vfs::Vfs) and
//! reporting what the format codec found as `kerno_core::Error`.

use std::io;

use kerno_core::Error;
use kerno_core::vfs::File;

use crate::format::{ByteSource, ErrorClass, FormatError, ReadError};

/// A [`ByteSource`] over an open file; its length is the one at creation.
pub struct FileSource<'a> {
    file: &'a dyn File,
    len: u64,
}

impl<'a> FileSource<'a> {
    pub fn new(file: &'a dyn File) -> io::Result<Self> {
        Ok(FileSource { file, len: file.len()? })
    }
}

impl ByteSource for FileSource<'_> {
    type Error = io::Error;

    fn len(&self) -> u64 {
        self.len
    }

    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.file.read_at(buf, offset)
    }
}

/// A violated format rule in `file` as an engine error: `Corruption`, or
/// `Incompatible` for a version or feature this engine does not support. The
/// message names the file, the rule and the offset, never key or value bytes.
pub fn format_error(file: &str, e: FormatError) -> Error {
    let msg = format!("{file}: {} at offset {}: {}", e.rule, e.offset, e.detail);
    match e.class {
        ErrorClass::Corruption => Error::Corruption(msg),
        ErrorClass::Incompatible => Error::Incompatible(msg),
    }
}

/// [`format_error`] for the error of a scanner over a [`FileSource`].
pub fn read_error(file: &str, e: ReadError<io::Error>) -> Error {
    match e {
        ReadError::Format(e) => format_error(file, e),
        ReadError::Source(e) => Error::Io(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_errors_keep_their_class() {
        let c = format_error("000001.wal", FormatError::corruption("WAL-R3", 40, "count out of range"));
        assert!(
            matches!(&c, Error::Corruption(m) if m == "000001.wal: WAL-R3 at offset 40: count out of range"),
            "{c:?}"
        );
        let i = format_error("000003.sst", FormatError::incompatible("HDR-2", 0, "M0 prototype format"));
        assert!(matches!(&i, Error::Incompatible(m) if m.contains("M0 prototype format")), "{i:?}");
    }
}
