//! On-disk format v1 (`docs/file-format/v1.md`) as pure code.
//!
//! Every function here works on byte slices: there is no file access, no
//! `Vfs` and no engine state. The engine (WP07 onwards) reads bytes through
//! the `Vfs`, hands them to these decoders and maps a [`FormatError`] to a
//! `kerno_core::Error` (`Corruption` or `Incompatible`).
//!
//! * [`coding`], [`crc`], [`tlv`]: integers, varints (VAR-1..3), context
//!   CRCs (§1.2) and TLV fields (§1.3).
//! * [`limits`], [`features`]: the format limits of §1.4 and the feature
//!   bits of §4.
//! * [`header`]: the 96-byte common header (HDR-1..10).
//! * [`wal`]: commits in canonical form (WAL-C1..C4), frames and the replay
//!   scanner (WAL-R1..R6).
//! * [`block`], [`bloom`], [`filter`], [`rangedel`], [`props`], [`sst`]: the
//!   SST blocks, footer and the SST-O*, SST-R*, SST-V* checks.
//! * [`manifest`]: records, fields, `FileMeta`, application, state
//!   validation (MAN-S1..S5) and replay (MAN-R1..R4).
//!
//! Every decoder returns a [`FormatError`] naming the rule it enforces; the
//! rule each check reports is listed in v1.md §13.1. Error values never carry
//! key or value bytes (threat T-08).

pub mod block;
pub mod bloom;
pub mod coding;
pub mod crc;
pub mod features;
pub mod filter;
pub mod header;
pub mod limits;
pub mod manifest;
pub mod props;
pub mod rangedel;
pub mod sst;
pub mod tlv;
pub mod wal;

pub use header::new_db_id;

use std::convert::Infallible;
use std::fmt;

/// How a reader must treat a rejected structure (v1.md §0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorClass {
    /// Damage or a fabricated file.
    Corruption,
    /// An unknown or unsupported version or feature: not damage.
    Incompatible,
}

/// A violated format rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatError {
    /// The rule id from v1.md (`HDR-3`, `WAL-C4`, `MAN-S4`, ...).
    pub rule: &'static str,
    /// Byte offset of the offending structure or field, relative to the
    /// input the decoder received (plus the base offset it was given, for
    /// decoders that take one).
    pub offset: u64,
    pub class: ErrorClass,
    /// Short fixed description (never contains key or value bytes).
    pub detail: &'static str,
}

impl FormatError {
    pub const fn corruption(rule: &'static str, offset: u64, detail: &'static str) -> Self {
        FormatError { rule, offset, class: ErrorClass::Corruption, detail }
    }

    pub const fn incompatible(rule: &'static str, offset: u64, detail: &'static str) -> Self {
        FormatError { rule, offset, class: ErrorClass::Incompatible, detail }
    }

    pub fn is_corruption(&self) -> bool {
        self.class == ErrorClass::Corruption
    }

    /// The same error with `base` added to its offset.
    pub fn shifted(mut self, base: u64) -> Self {
        self.offset = self.offset.saturating_add(base);
        self
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let class = match self.class {
            ErrorClass::Corruption => "corruption",
            ErrorClass::Incompatible => "incompatible",
        };
        write!(f, "{class}: {} at offset {}: {}", self.rule, self.offset, self.detail)
    }
}

impl std::error::Error for FormatError {}

pub type Result<T> = std::result::Result<T, FormatError>;

/// How a log (WAL or MANIFEST) ends after its last valid frame or record,
/// at the offset `P` the scanner reports (WAL-R4, WAL-R5, MAN-R3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// `P` is the end of the file.
    Clean,
    /// Every byte from `P` to the end is zero (padding).
    ZeroPadded,
    /// A torn tail starts at `P`: no candidate frame or record follows it.
    Torn,
}

/// Random-access source of bytes for the streaming scanners (the WAL and
/// MANIFEST replays). The engine implements it over a `Vfs` file; tests and
/// vectors use a byte slice.
pub trait ByteSource {
    type Error;

    /// Length of the source in bytes.
    fn len(&self) -> u64;

    /// Whether the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fills `buf` with the bytes at `offset`. Callers never read past
    /// [`ByteSource::len`].
    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> std::result::Result<(), Self::Error>;
}

impl ByteSource for &[u8] {
    type Error = Infallible;

    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> std::result::Result<(), Infallible> {
        let start = offset as usize;
        buf.copy_from_slice(&self[start..start + buf.len()]);
        Ok(())
    }
}

/// Error of a scanner over a [`ByteSource`]: a format violation or a read
/// error of the source.
#[derive(Debug, PartialEq, Eq)]
pub enum ReadError<E> {
    Format(FormatError),
    Source(E),
}

impl<E> From<FormatError> for ReadError<E> {
    fn from(e: FormatError) -> Self {
        ReadError::Format(e)
    }
}

impl ReadError<Infallible> {
    /// The format error of a scan over an infallible source.
    pub fn into_format(self) -> FormatError {
        match self {
            ReadError::Format(e) => e,
            ReadError::Source(never) => match never {},
        }
    }
}

/// Size of one streaming read (WAL-R2: the file is never loaded whole).
pub const READ_CHUNK: usize = 1 << 20;

/// Sequential reader with a cached window over a [`ByteSource`].
pub(crate) struct Window<'s, S: ByteSource> {
    src: &'s mut S,
    len: u64,
    start: u64,
    buf: Vec<u8>,
}

impl<'s, S: ByteSource> Window<'s, S> {
    pub(crate) fn new(src: &'s mut S) -> Self {
        let len = src.len();
        Window { src, len, start: 0, buf: Vec::new() }
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }

    /// Bytes `[off, off + n)`; the caller guarantees `off + n <= len`.
    /// Ranges larger than the window are read into a fresh buffer.
    pub(crate) fn get(&mut self, off: u64, n: usize) -> std::result::Result<std::borrow::Cow<'_, [u8]>, S::Error> {
        debug_assert!(off + n as u64 <= self.len);
        if n > READ_CHUNK {
            let mut v = vec![0; n];
            self.src.read_exact_at(off, &mut v)?;
            return Ok(std::borrow::Cow::Owned(v));
        }
        let cached_end = self.start + self.buf.len() as u64;
        if off < self.start || off + n as u64 > cached_end {
            let want = (READ_CHUNK as u64).min(self.len - off) as usize;
            self.buf.resize(want, 0);
            self.src.read_exact_at(off, &mut self.buf)?;
            self.start = off;
        }
        let at = (off - self.start) as usize;
        Ok(std::borrow::Cow::Borrowed(&self.buf[at..at + n]))
    }

    /// Whether every byte in `[from, len)` is zero.
    pub(crate) fn all_zero_from(&mut self, from: u64) -> std::result::Result<bool, S::Error> {
        let mut off = from;
        while off < self.len {
            let n = (READ_CHUNK as u64).min(self.len - off) as usize;
            if self.get(off, n)?.iter().any(|&b| b != 0) {
                return Ok(false);
            }
            off += n as u64;
        }
        Ok(true)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::ByteSource;
    use std::convert::Infallible;

    /// A source over a byte slice that records every read and checks that
    /// none goes past the end or is larger than `max_read`.
    pub(crate) struct Recording<'a> {
        pub bytes: &'a [u8],
        pub reads: Vec<(u64, u64)>,
        pub max_read: usize,
    }

    impl<'a> Recording<'a> {
        pub fn new(bytes: &'a [u8], max_read: usize) -> Self {
            Recording { bytes, reads: Vec::new(), max_read }
        }
    }

    impl ByteSource for Recording<'_> {
        type Error = Infallible;

        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), Infallible> {
            let end = offset + buf.len() as u64;
            assert!(end <= self.bytes.len() as u64, "read {offset}..{end} past the end ({})", self.bytes.len());
            assert!(buf.len() <= self.max_read, "read of {} bytes, above {}", buf.len(), self.max_read);
            self.reads.push((offset, end));
            buf.copy_from_slice(&self.bytes[offset as usize..end as usize]);
            Ok(())
        }
    }
}
