//! The 96-byte common header of every MANIFEST, WAL and SST (v1.md §3) and
//! its validation, HDR-1..HDR-10 in order.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::coding::{le_u32, le_u64};
use super::crc::crc32c;
use super::features::{KNOWN_INCOMPAT, KNOWN_RO_COMPAT, WRITE_INCOMPAT};
use super::limits::{MAX_BLOCK_SIZE, MAX_HEADER_LEN, MIN_BLOCK_SIZE, MIN_HEADER_LEN};
use super::{FormatError, Result, features};

/// Length of the v1 header.
pub const HEADER_LEN: usize = 96;
/// The only format version v1 writes and reads.
pub const FORMAT_VERSION: u32 = 1;

pub const MAGIC_MANIFEST: [u8; 8] = *b"KRNO_MAN";
pub const MAGIC_WAL: [u8; 8] = *b"KRNO_WAL";
pub const MAGIC_SST: [u8; 8] = *b"KRNO_SST";
/// Reserved for the Phase 4 logical export.
pub const MAGIC_EXPORT: [u8; 8] = *b"KRNO_EXP";
/// Magics of the M0 prototypes (`spike-lsm` MANIFEST and SST, reference
/// engine log): `Incompatible("M0 prototype format")`.
pub const M0_MAGICS: [[u8; 8]; 3] = [*b"KRNOMAN1", *b"KRNOSST1", *b"KRNOLOGC"];

/// Whether `magic` is a v1 magic (of any type, including the reserved
/// export one) or an M0 prototype magic.
pub fn is_known_magic(magic: &[u8]) -> bool {
    [MAGIC_MANIFEST, MAGIC_WAL, MAGIC_SST, MAGIC_EXPORT].iter().chain(&M0_MAGICS).any(|m| m[..] == *magic)
}

/// Block size v1 writes in the SST header (informative).
pub const DEFAULT_BLOCK_SIZE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Manifest,
    Wal,
    Sst,
}

impl FileKind {
    pub fn magic(self) -> [u8; 8] {
        match self {
            FileKind::Manifest => MAGIC_MANIFEST,
            FileKind::Wal => MAGIC_WAL,
            FileKind::Sst => MAGIC_SST,
        }
    }

    /// End of the per-type field area that v1 uses (bytes 64..end).
    fn per_type_end(self) -> usize {
        match self {
            FileKind::Manifest | FileKind::Wal => 72,
            FileKind::Sst => 68,
        }
    }
}

/// A decoded common header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub kind: FileKind,
    pub format_version: u32,
    pub header_len: u32,
    pub compat: u64,
    pub ro_compat: u64,
    pub incompat: u64,
    pub db_id: [u8; 16],
    /// WAL and SST: the number in the file name. MANIFEST: the generation.
    pub file_number: u64,
    /// The per-type field at 64: MANIFEST `first_edit_seq`, WAL `base_seq`,
    /// SST `block_size` (a `u32`).
    pub per_type: u64,
}

impl Header {
    /// A v1 header as this writer produces it (`incompat` = `WRITE_INCOMPAT`).
    pub fn new(kind: FileKind, db_id: [u8; 16], file_number: u64, per_type: u64) -> Header {
        Header {
            kind,
            format_version: FORMAT_VERSION,
            header_len: MIN_HEADER_LEN,
            compat: 0,
            ro_compat: 0,
            incompat: WRITE_INCOMPAT,
            db_id,
            file_number,
            per_type,
        }
    }

    pub fn manifest(db_id: [u8; 16], generation: u64, first_edit_seq: u64) -> Header {
        Header::new(FileKind::Manifest, db_id, generation, first_edit_seq)
    }

    pub fn wal(db_id: [u8; 16], file_number: u64, base_seq: u64) -> Header {
        Header::new(FileKind::Wal, db_id, file_number, base_seq)
    }

    pub fn sst(db_id: [u8; 16], file_number: u64, block_size: u32) -> Header {
        Header::new(FileKind::Sst, db_id, file_number, u64::from(block_size))
    }

    /// `ro_compat` bits this reader does not know: non-zero means the whole
    /// database opens read-only (HDR-7).
    pub fn unknown_ro_compat(&self) -> u64 {
        self.ro_compat & !KNOWN_RO_COMPAT
    }

    /// Encodes the 96-byte header, with its CRC. `header_len` is written as
    /// stored (tests patch other values; see [`reseal_header`]).
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..8].copy_from_slice(&self.kind.magic());
        b[8..12].copy_from_slice(&self.format_version.to_le_bytes());
        b[12..16].copy_from_slice(&self.header_len.to_le_bytes());
        b[16..24].copy_from_slice(&self.compat.to_le_bytes());
        b[24..32].copy_from_slice(&self.ro_compat.to_le_bytes());
        b[32..40].copy_from_slice(&self.incompat.to_le_bytes());
        b[40..56].copy_from_slice(&self.db_id);
        b[56..64].copy_from_slice(&self.file_number.to_le_bytes());
        match self.kind {
            FileKind::Sst => b[64..68].copy_from_slice(&(self.per_type as u32).to_le_bytes()),
            FileKind::Manifest | FileKind::Wal => b[64..72].copy_from_slice(&self.per_type.to_le_bytes()),
        }
        let crc = crc32c(&b[..HEADER_LEN - 4]);
        b[HEADER_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
        b
    }
}

/// What HDR-10 compares the header with: the MANIFEST's `db_id` and the
/// number in the file name (WAL and SST only; `None` skips the check).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeaderExpect {
    pub db_id: Option<[u8; 16]>,
    pub file_number: Option<u64>,
}

/// Validates a header, HDR-1..HDR-10 in order.
///
/// `prefix` holds the first bytes of the file (at least
/// `min(file_len, 4096)` of them) and `file_len` its full length. HDR-7 and
/// HDR-8 never fail: see [`Header::unknown_ro_compat`].
pub fn decode_header(prefix: &[u8], file_len: u64, kind: FileKind, expect: &HeaderExpect) -> Result<Header> {
    let avail = (prefix.len() as u64).min(file_len) as usize;
    // HDR-1.
    if avail < HEADER_LEN {
        return Err(FormatError::corruption("HDR-1", 0, "file shorter than the header"));
    }
    let b = &prefix[..avail];
    // HDR-2.
    let magic: [u8; 8] = b[0..8].try_into().unwrap();
    if magic != kind.magic() {
        if M0_MAGICS.contains(&magic) {
            return Err(FormatError::incompatible("HDR-2", 0, "M0 prototype format"));
        }
        return Err(FormatError::corruption("HDR-2", 0, "unknown magic"));
    }
    // HDR-3: length, then CRC, before the version.
    let header_len = le_u32(b, 12);
    if !(MIN_HEADER_LEN..=MAX_HEADER_LEN).contains(&header_len)
        || !header_len.is_multiple_of(8)
        || u64::from(header_len) > file_len
        || header_len as usize > b.len()
    {
        return Err(FormatError::corruption("HDR-3", 12, "invalid header_len"));
    }
    let crc_at = header_len as usize - 4;
    if crc32c(&b[..crc_at]) != le_u32(b, crc_at) {
        return Err(FormatError::corruption("HDR-3", crc_at as u64, "header CRC mismatch"));
    }
    // HDR-4.
    let format_version = le_u32(b, 8);
    if format_version == 0 {
        return Err(FormatError::corruption("HDR-4", 8, "format_version 0"));
    }
    if format_version > FORMAT_VERSION {
        return Err(FormatError::incompatible("HDR-4", 8, "unsupported format_version"));
    }
    // HDR-5.
    if header_len != MIN_HEADER_LEN {
        return Err(FormatError::corruption("HDR-5", 12, "header_len is not 96 in version 1"));
    }
    // HDR-6.
    let incompat = le_u64(b, 32);
    if incompat & features::DRAFT != 0 {
        return Err(FormatError::incompatible("HDR-6", 32, "pre-freeze draft format"));
    }
    if incompat & !KNOWN_INCOMPAT != 0 {
        return Err(FormatError::incompatible("HDR-6", 32, "unknown incompat feature"));
    }
    if incompat & features::AEAD != 0 {
        return Err(FormatError::incompatible("HDR-6", 32, "encryption not supported"));
    }
    // HDR-7 and HDR-8 do not fail.
    let compat = le_u64(b, 16);
    let ro_compat = le_u64(b, 24);
    // HDR-9.
    if le_u32(b, 88) != 0 {
        return Err(FormatError::corruption("HDR-9", 88, "reserved header bytes are not zero"));
    }
    let end = kind.per_type_end();
    if b[end..88].iter().any(|&x| x != 0) {
        return Err(FormatError::corruption("HDR-9", end as u64, "unused per-type bytes are not zero"));
    }
    let per_type = match kind {
        FileKind::Sst => {
            let block_size = le_u32(b, 64);
            if !(MIN_BLOCK_SIZE..=MAX_BLOCK_SIZE).contains(&block_size) {
                return Err(FormatError::corruption("HDR-9", 64, "block_size out of range"));
            }
            u64::from(block_size)
        }
        FileKind::Manifest | FileKind::Wal => le_u64(b, 64),
    };
    // HDR-10.
    let db_id: [u8; 16] = b[40..56].try_into().unwrap();
    let file_number = le_u64(b, 56);
    if expect.db_id.is_some_and(|id| id != db_id) || expect.file_number.is_some_and(|n| n != file_number) {
        return Err(FormatError::corruption("HDR-10", 40, "file belongs to another database or was renamed"));
    }
    Ok(Header { kind, format_version, header_len, compat, ro_compat, incompat, db_id, file_number, per_type })
}

/// Recomputes the CRC of a (possibly patched) header in place, using the
/// `header_len` stored at offset 12. Returns `false` if that length does not
/// fit in `buf` or is below 8. Used by tests and crafted-file generators.
pub fn reseal_header(buf: &mut [u8]) -> bool {
    if buf.len() < 16 {
        return false;
    }
    let len = le_u32(buf, 12) as usize;
    if len < 8 || len > buf.len() {
        return false;
    }
    let crc = crc32c(&buf[..len - 4]);
    buf[len - 4..len].copy_from_slice(&crc.to_le_bytes());
    true
}

/// A fresh random `db_id` for a new database: 16 bytes derived from std's
/// randomly keyed `RandomState` hashes of the time, the process id and a
/// counter. Tests inject a fixed value through the engine configuration.
pub fn new_db_id() -> [u8; 16] {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut id = [0u8; 16];
    for (i, half) in id.chunks_mut(8).enumerate() {
        let h = RandomState::new().hash_one((now, n, i, std::process::id()));
        half.copy_from_slice(&h.to_le_bytes());
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::features::{AEAD, DRAFT};

    const DB: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

    fn patched(h: &Header, f: impl FnOnce(&mut [u8; HEADER_LEN])) -> [u8; HEADER_LEN] {
        let mut b = h.encode();
        f(&mut b);
        reseal_header(&mut b);
        b
    }

    fn rule(b: &[u8], kind: FileKind, expect: &HeaderExpect) -> (&'static str, bool) {
        let e = decode_header(b, b.len() as u64, kind, expect).unwrap_err();
        (e.rule, e.is_corruption())
    }

    #[test]
    fn header_roundtrip_matches_spec_example() {
        // v1.md §13.5: WAL header, db_id 00..0f, file 1, base_seq 1.
        let h = Header::wal(DB, 1, 1);
        let b = h.encode();
        assert_eq!(&b[92..], &[0x38, 0x8c, 0x41, 0xea]);
        let expect = HeaderExpect { db_id: Some(DB), file_number: Some(1) };
        assert_eq!(decode_header(&b, 96, FileKind::Wal, &expect), Ok(h));
    }

    #[test]
    fn header_validation_order() {
        let h = Header::wal(DB, 3, 10);
        let none = HeaderExpect::default();
        let wal = FileKind::Wal;
        // HDR-1 before anything: a short file with a wrong magic.
        assert_eq!(rule(&b"XXXX"[..], wal, &none), ("HDR-1", true));
        // HDR-2 (magic) before HDR-3 (CRC): wrong magic and stale CRC.
        let mut b = h.encode();
        b[0] = b'Z';
        assert_eq!(rule(&b, wal, &none), ("HDR-2", true));
        b[0..8].copy_from_slice(b"KRNOMAN1");
        assert_eq!(rule(&b, wal, &none), ("HDR-2", false));
        // A valid header of another type is "any other magic".
        assert_eq!(rule(&h.encode(), FileKind::Sst, &none), ("HDR-2", true));
        // HDR-3 (CRC) before HDR-4 (version): random damage never looks
        // Incompatible.
        let mut b = h.encode();
        b[8] = 2;
        assert_eq!(rule(&b, wal, &none), ("HDR-3", true));
        // HDR-3 (length) before HDR-3 (CRC) and before the version.
        let b = patched(&h, |b| b[12] = 95);
        assert_eq!(rule(&b, wal, &none), ("HDR-3", true));
        let b = patched(&h, |b| b[12] = 100);
        assert_eq!(rule(&b, wal, &none), ("HDR-3", true));
        // HDR-4 before HDR-5 and HDR-6: version 2 with a longer header and
        // an unknown incompat bit is Incompatible by version.
        let mut b = patched(&h, |b| {
            b[8] = 2;
            b[33] = 0x10;
        })
        .to_vec();
        b.resize(104, 0);
        b[12] = 104;
        reseal_header(&mut b);
        assert_eq!(rule(&b, wal, &none), ("HDR-4", false));
        b[8] = 0;
        reseal_header(&mut b);
        assert_eq!(rule(&b, wal, &none), ("HDR-4", true));
        // HDR-5 before HDR-6.
        b[8] = 1;
        reseal_header(&mut b);
        assert_eq!(rule(&b, wal, &none), ("HDR-5", true));
        // HDR-6 before HDR-9: AEAD with a reserved byte set.
        let b = patched(&h, |b| {
            b[32] |= AEAD as u8;
            b[88] = 1;
        });
        assert_eq!(rule(&b, wal, &none), ("HDR-6", false));
        // HDR-9 before HDR-10.
        let b = patched(&h, |b| b[80] = 1);
        let foreign = HeaderExpect { db_id: Some([9; 16]), file_number: Some(4) };
        assert_eq!(rule(&b, wal, &foreign), ("HDR-9", true));
        // HDR-10 last.
        assert_eq!(rule(&h.encode(), wal, &foreign), ("HDR-10", true));
        let renamed = HeaderExpect { db_id: Some(DB), file_number: Some(4) };
        assert_eq!(rule(&h.encode(), wal, &renamed), ("HDR-10", true));
        // HDR-7 and HDR-8 never fail.
        let b = patched(&h, |b| {
            b[16] = 0xff;
            b[24] = 0x01;
        });
        let got = decode_header(&b, 96, wal, &none).unwrap();
        assert_eq!((got.compat, got.unknown_ro_compat(), got.incompat), (0xff, 1, 0));
        // The pre-freeze draft bit is refused by name, before unknown bits.
        let b = patched(&h, |b| b[39] = 0x80);
        let e = decode_header(&b, 96, wal, &none).unwrap_err();
        assert_eq!((e.rule, e.is_corruption()), ("HDR-6", false));
        assert_eq!(DRAFT, 1 << 63);
    }

    #[test]
    fn db_ids_differ() {
        assert_ne!(new_db_id(), new_db_id());
    }
}
