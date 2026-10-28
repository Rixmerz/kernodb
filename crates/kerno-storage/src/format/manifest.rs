//! MANIFEST (v1.md §8): record framing (§8.2), fields (§8.3), `FileMeta`
//! (§8.4), application (§8.5, MAN-A1), state validation (MAN-S1..S5) and
//! replay (MAN-R1..R4).

use std::collections::HashSet;

use super::coding::{Reader, le_u32, le_u64, put_bytes, put_u32, put_u64, put_varint};
use super::crc::{crc_ctx_parts, file_ctx};
use super::features::{SLOT_LEN, slot_is_zero};
use super::header::{FileKind, Header, HeaderExpect, decode_header};
use super::limits::{
    MAX_FILE_NUMBER, MAX_FILES_PER_LEVEL, MAX_HEADER_LEN, MAX_KEY_LEN, MAX_META_PAYLOAD, MAX_SEQ, NUM_LEVELS,
};
use super::tlv::{FIRST_IGNORABLE_TAG, Tlv, TlvIter, put_tlv, put_tlv_u8, put_tlv_varint, value_u8, value_varint};
use super::{ByteSource, End, FormatError, ReadError, Result, Window};

/// Fixed part of a record, before the payload.
pub const RECORD_HEADER_LEN: usize = 24;
/// Smallest record: header, one payload byte, slot.
pub const MIN_RECORD_LEN: u64 = (RECORD_HEADER_LEN + 1 + SLOT_LEN) as u64;

pub const KIND_SNAPSHOT: u8 = 1;
pub const KIND_SESSION_OPEN: u8 = 2;
pub const KIND_FLUSH: u8 = 3;
pub const KIND_COMPACTION: u8 = 4;
/// Reserved for the Phase 4 `MIGRATION` record (it will come with an
/// `incompat` bit); v1 rejects it as `Corruption`.
pub const KIND_MIGRATION_RESERVED: u8 = 5;

pub const TAG_NEXT_FILE: u64 = 1;
pub const TAG_LAST_SEQ: u64 = 2;
pub const TAG_LOG_NUMBER: u64 = 3;
pub const TAG_NUM_LEVELS: u64 = 4;
pub const TAG_FLAGS: u64 = 5;
pub const TAG_REASON: u64 = 6;
pub const TAG_PREV_GENERATION: u64 = 7;
pub const TAG_ADD_FILE: u64 = 10;
pub const TAG_DELETE_FILE: u64 = 11;

/// `flags` bit 0.
pub const FLAG_CLEAN: u64 = 1;

/// `reason` values.
pub const REASON_BACKGROUND: u8 = 1;
pub const REASON_DIRECTED: u8 = 2;
pub const REASON_CLOSE: u8 = 3;
pub const REASON_TRIVIAL_MOVE: u8 = 4;

/// One table as the MANIFEST records it (§8.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileMeta {
    pub level: u8,
    pub number: u64,
    pub size: u64,
    pub point_entries: u64,
    pub range_fragments: u64,
    pub smallest_seq: u64,
    pub largest_seq: u64,
    pub largest_exclusive: bool,
    pub smallest: Vec<u8>,
    pub largest: Vec<u8>,
}

impl FileMeta {
    pub fn encode_into(&self, out: &mut Vec<u8>) {
        out.push(self.level);
        put_varint(out, self.number);
        put_varint(out, self.size);
        put_varint(out, self.point_entries);
        put_varint(out, self.range_fragments);
        put_varint(out, self.smallest_seq);
        put_varint(out, self.largest_seq);
        out.push(u8::from(self.largest_exclusive));
        put_bytes(out, &self.smallest);
        put_bytes(out, &self.largest);
    }

    /// Whether the table's key range ends before `key` (MAN-S3).
    pub fn ends_before(&self, key: &[u8]) -> bool {
        if self.largest_exclusive { self.largest[..] <= *key } else { self.largest[..] < *key }
    }

    /// Whether the key ranges of two tables overlap.
    pub fn overlaps(&self, other: &FileMeta) -> bool {
        !self.ends_before(&other.smallest) && !other.ends_before(&self.smallest)
    }
}

fn check_level(level: u8, at: u64) -> Result<()> {
    if level as usize >= NUM_LEVELS {
        return Err(FormatError::corruption("MAN-S5", at, "level beyond num_levels"));
    }
    Ok(())
}

/// Decodes a `FileMeta` value (`base` is its offset). A value cut short or
/// with trailing bytes and unknown `flags` bits are MAN-R2; a level ≥ 7 is
/// MAN-S5.
pub fn decode_file_meta(value: &[u8], base: u64) -> Result<FileMeta> {
    let mut r = Reader::new(value, base, "MAN-R2");
    let level = r.u8()?;
    check_level(level, base)?;
    let number = r.varint()?;
    let size = r.varint()?;
    let point_entries = r.varint()?;
    let range_fragments = r.varint()?;
    let smallest_seq = r.varint()?;
    let largest_seq = r.varint()?;
    let flags_at = r.offset();
    let flags = r.u8()?;
    if flags & !1 != 0 {
        return Err(FormatError::corruption("MAN-R2", flags_at, "unknown FileMeta flags"));
    }
    let smallest = r.bytes(MAX_KEY_LEN)?.to_vec();
    let largest = r.bytes(MAX_KEY_LEN)?.to_vec();
    r.finish()?;
    Ok(FileMeta {
        level,
        number,
        size,
        point_entries,
        range_fragments,
        smallest_seq,
        largest_seq,
        largest_exclusive: flags == 1,
        smallest,
        largest,
    })
}

/// The contents of a record, by type (§8.3, "campos por tipo").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Replaces the whole state; only valid as record 0 of a file.
    /// `num_levels` is always 7 in v1. `files` are by increasing level, L0
    /// newest to oldest, deeper levels by `smallest`.
    Snapshot { next_file: u64, last_seq: u64, log_number: u64, clean: bool, prev_generation: u64, files: Vec<FileMeta> },
    /// Session start: L0 outputs of the recovery flush, newest to oldest.
    SessionOpen { next_file: u64, last_seq: u64, log_number: u64, files: Vec<FileMeta> },
    /// Flush: zero or more L0 files, newest to oldest.
    Flush { next_file: u64, last_seq: u64, log_number: u64, reason: Option<u8>, files: Vec<FileMeta> },
    /// Compaction: `delete` is `(level, number)`; `add` are at levels ≥ 1.
    Compaction { reason: u8, next_file: Option<u64>, add: Vec<FileMeta>, delete: Vec<(u8, u64)> },
}

impl Edit {
    pub fn kind(&self) -> u8 {
        match self {
            Edit::Snapshot { .. } => KIND_SNAPSHOT,
            Edit::SessionOpen { .. } => KIND_SESSION_OPEN,
            Edit::Flush { .. } => KIND_FLUSH,
            Edit::Compaction { .. } => KIND_COMPACTION,
        }
    }
}

/// Fields with tags ≥ 1024 as read: `(tag, value)`.
pub type IgnoredFields = Vec<(u64, Vec<u8>)>;

/// A decoded record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub edit_seq: u64,
    pub edit: Edit,
    /// Fields with tags ≥ 1024, kept as read (a writer never emits them).
    pub ignored: IgnoredFields,
}

fn put_files(out: &mut Vec<u8>, files: &[FileMeta]) {
    for f in files {
        let mut v = Vec::new();
        f.encode_into(&mut v);
        put_tlv(out, TAG_ADD_FILE, &v);
    }
}

impl Record {
    /// The TLV payload, tags in increasing order.
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match &self.edit {
            Edit::Snapshot { next_file, last_seq, log_number, clean, prev_generation, files } => {
                put_tlv_varint(&mut out, TAG_NEXT_FILE, *next_file);
                put_tlv_varint(&mut out, TAG_LAST_SEQ, *last_seq);
                put_tlv_varint(&mut out, TAG_LOG_NUMBER, *log_number);
                put_tlv_u8(&mut out, TAG_NUM_LEVELS, NUM_LEVELS as u8);
                put_tlv_varint(&mut out, TAG_FLAGS, if *clean { FLAG_CLEAN } else { 0 });
                put_tlv_varint(&mut out, TAG_PREV_GENERATION, *prev_generation);
                put_files(&mut out, files);
            }
            Edit::SessionOpen { next_file, last_seq, log_number, files } => {
                put_tlv_varint(&mut out, TAG_NEXT_FILE, *next_file);
                put_tlv_varint(&mut out, TAG_LAST_SEQ, *last_seq);
                put_tlv_varint(&mut out, TAG_LOG_NUMBER, *log_number);
                put_files(&mut out, files);
            }
            Edit::Flush { next_file, last_seq, log_number, reason, files } => {
                put_tlv_varint(&mut out, TAG_NEXT_FILE, *next_file);
                put_tlv_varint(&mut out, TAG_LAST_SEQ, *last_seq);
                put_tlv_varint(&mut out, TAG_LOG_NUMBER, *log_number);
                if let Some(r) = reason {
                    put_tlv_u8(&mut out, TAG_REASON, *r);
                }
                put_files(&mut out, files);
            }
            Edit::Compaction { reason, next_file, add, delete } => {
                if let Some(n) = next_file {
                    put_tlv_varint(&mut out, TAG_NEXT_FILE, *n);
                }
                put_tlv_u8(&mut out, TAG_REASON, *reason);
                put_files(&mut out, add);
                for (level, number) in delete {
                    let mut v = vec![*level];
                    put_varint(&mut v, *number);
                    put_tlv(&mut out, TAG_DELETE_FILE, &v);
                }
            }
        }
        for (tag, value) in &self.ignored {
            put_tlv(&mut out, *tag, value);
        }
        out
    }

    /// The whole record for a MANIFEST of this `generation`.
    pub fn encode(&self, generation: u64) -> Vec<u8> {
        seal_record(generation, self.edit_seq, self.edit.kind(), &self.encode_payload())
    }
}

/// Frames a record: header, payload, zero slot, CRC
/// `crc(generation; bytes[0..4] ‖ bytes[8..24] ‖ payload ‖ slot)`.
pub fn seal_record(generation: u64, edit_seq: u64, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(RECORD_HEADER_LEN + payload.len() + SLOT_LEN);
    put_u32(&mut out, payload.len() as u32);
    put_u32(&mut out, 0);
    put_u64(&mut out, edit_seq);
    out.push(kind);
    out.push(0);
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(payload);
    out.resize(out.len() + SLOT_LEN, 0);
    reseal_record(&mut out, generation);
    out
}

fn record_crc(rec: &[u8], generation: u64) -> u32 {
    crc_ctx_parts(&file_ctx(generation), &[&rec[0..4], &rec[8..]])
}

/// Recomputes the CRC of a whole (possibly patched) record in place.
pub fn reseal_record(rec: &mut [u8], generation: u64) {
    let crc = record_crc(rec, generation);
    rec[4..8].copy_from_slice(&crc.to_le_bytes());
}

/// Whether the stored CRC of a whole record verifies.
pub fn record_crc_ok(rec: &[u8], generation: u64) -> bool {
    rec.len() >= RECORD_HEADER_LEN && le_u32(rec, 4) == record_crc(rec, generation)
}

/// Length of a record with this payload.
pub fn record_len(payload_len: u32) -> u64 {
    (RECORD_HEADER_LEN + SLOT_LEN) as u64 + u64::from(payload_len)
}

struct Fields {
    next_file: Option<u64>,
    last_seq: Option<u64>,
    log_number: Option<u64>,
    num_levels: bool,
    clean: Option<bool>,
    reason: Option<u8>,
    prev_generation: Option<u64>,
    add: Vec<FileMeta>,
    delete: Vec<(u8, u64)>,
    tags: u32,
}

fn decode_field(f: &Tlv<'_>, d: &mut Fields) -> Result<bool> {
    let r2 = |detail| FormatError::corruption("MAN-R2", f.offset, detail);
    match f.tag {
        TAG_NEXT_FILE => d.next_file = Some(value_varint(f, "MAN-R2")?),
        TAG_LAST_SEQ => d.last_seq = Some(value_varint(f, "MAN-R2")?),
        TAG_LOG_NUMBER => d.log_number = Some(value_varint(f, "MAN-R2")?),
        TAG_PREV_GENERATION => d.prev_generation = Some(value_varint(f, "MAN-R2")?),
        TAG_NUM_LEVELS => match value_u8(f, "MAN-R2")? {
            7 => d.num_levels = true,
            2..=16 => return Err(FormatError::incompatible("MAN-S5", f.offset, "unsupported num_levels")),
            _ => return Err(FormatError::corruption("MAN-S5", f.offset, "invalid num_levels")),
        },
        TAG_FLAGS => {
            let flags = value_varint(f, "MAN-R2")?;
            if flags & !FLAG_CLEAN != 0 {
                return Err(r2("unknown record flags"));
            }
            d.clean = Some(flags == FLAG_CLEAN);
        }
        TAG_REASON => {
            let r = value_u8(f, "MAN-R2")?;
            if !(REASON_BACKGROUND..=REASON_TRIVIAL_MOVE).contains(&r) {
                return Err(r2("unknown reason"));
            }
            d.reason = Some(r);
        }
        TAG_ADD_FILE => d.add.push(decode_file_meta(f.value, f.value_offset)?),
        TAG_DELETE_FILE => {
            let mut r = Reader::new(f.value, f.value_offset, "MAN-R2");
            let level = r.u8()?;
            check_level(level, f.value_offset)?;
            let number = r.varint()?;
            r.finish()?;
            d.delete.push((level, number));
        }
        t if t >= FIRST_IGNORABLE_TAG => return Ok(false),
        _ => return Err(r2("unknown field tag below 1024")),
    }
    d.tags |= 1 << f.tag;
    Ok(true)
}

const fn mask(tags: &[u64]) -> u32 {
    let mut m = 0;
    let mut i = 0;
    while i < tags.len() {
        m |= 1 << tags[i];
        i += 1;
    }
    m
}

/// Decodes the TLV payload of a record of `kind` (`base` is its offset).
///
/// Fields are checked in order (tags non-decreasing, only 10 and 11
/// repeated, known tags below 1024, valid values); then the set of tags
/// against the record type, the level order of `SNAPSHOT` files, the level 0
/// of `SESSION_OPEN` and `FLUSH` files and the levels ≥ 1 of `COMPACTION`
/// additions. Violations are MAN-R2, except `num_levels` and levels ≥ 7
/// (MAN-S5) and the VAR-* framing rules.
pub fn decode_fields(kind: u8, payload: &[u8], base: u64) -> Result<(Edit, IgnoredFields)> {
    let mut d = Fields {
        next_file: None,
        last_seq: None,
        log_number: None,
        num_levels: false,
        clean: None,
        reason: None,
        prev_generation: None,
        add: Vec::new(),
        delete: Vec::new(),
        tags: 0,
    };
    let mut ignored = Vec::new();
    let mut prev: Option<u64> = None;
    for field in TlvIter::new(payload, base, "MAN-R2") {
        let f = field?;
        if let Some(p) = prev {
            if p > f.tag {
                return Err(FormatError::corruption("MAN-R2", f.offset, "field tags decreasing"));
            }
            if p == f.tag && f.tag != TAG_ADD_FILE && f.tag != TAG_DELETE_FILE {
                return Err(FormatError::corruption("MAN-R2", f.offset, "repeated field"));
            }
        }
        prev = Some(f.tag);
        if !decode_field(&f, &mut d)? {
            ignored.push((f.tag, f.value.to_vec()));
        }
    }
    let r2 = |detail| Err(FormatError::corruption("MAN-R2", base, detail));
    let (required, optional) = match kind {
        KIND_SNAPSHOT => (mask(&[1, 2, 3, 4, 5, 7]), mask(&[10])),
        KIND_SESSION_OPEN => (mask(&[1, 2, 3]), mask(&[10])),
        KIND_FLUSH => (mask(&[1, 2, 3]), mask(&[6, 10])),
        KIND_COMPACTION => (mask(&[6]), mask(&[1, 10, 11])),
        _ => return r2("invalid record kind"),
    };
    if d.tags & required != required || d.tags & !(required | optional) != 0 {
        return r2("fields do not match the record type");
    }
    let edit = match kind {
        KIND_SNAPSHOT => {
            if d.add.windows(2).any(|w| w[0].level > w[1].level) {
                return r2("SNAPSHOT files not by increasing level");
            }
            Edit::Snapshot {
                next_file: d.next_file.unwrap(),
                last_seq: d.last_seq.unwrap(),
                log_number: d.log_number.unwrap(),
                clean: d.clean.unwrap(),
                prev_generation: d.prev_generation.unwrap(),
                files: d.add,
            }
        }
        KIND_SESSION_OPEN | KIND_FLUSH => {
            if d.add.iter().any(|f| f.level != 0) {
                return r2("SESSION_OPEN or FLUSH file outside level 0");
            }
            let (next_file, last_seq, log_number) = (d.next_file.unwrap(), d.last_seq.unwrap(), d.log_number.unwrap());
            if kind == KIND_SESSION_OPEN {
                Edit::SessionOpen { next_file, last_seq, log_number, files: d.add }
            } else {
                Edit::Flush { next_file, last_seq, log_number, reason: d.reason, files: d.add }
            }
        }
        _ => {
            if d.add.iter().any(|f| f.level == 0) {
                return r2("COMPACTION adds a file to level 0");
            }
            Edit::Compaction { reason: d.reason.unwrap(), next_file: d.next_file, add: d.add, delete: d.delete }
        }
    };
    debug_assert!(d.num_levels || kind != KIND_SNAPSHOT);
    Ok((edit, ignored))
}

/// Checks a whole record whose CRC verified (`rec` is exactly its bytes,
/// at file offset `offset`): `edit_seq` (the expected one, within the format
/// limit 1..=2^63−1 of §1.4), `kind`, `flags`, `reserved` (MAN-R2; for
/// record 0, `edit_seq` and a type other than `SNAPSHOT` are MAN-R1), the
/// slot (AEAD-1) and the fields ([`decode_fields`]).
pub fn check_record(rec: &[u8], offset: u64, expected_edit_seq: u64, first: bool) -> Result<Record> {
    let rule = if first { "MAN-R1" } else { "MAN-R2" };
    let payload_len = le_u32(rec, 0) as usize;
    let edit_seq = le_u64(rec, 8);
    if edit_seq != expected_edit_seq {
        return Err(FormatError::corruption(rule, offset + 8, "edit_seq is not the expected one"));
    }
    if edit_seq == 0 || edit_seq > MAX_SEQ {
        return Err(FormatError::corruption(rule, offset + 8, "edit_seq outside the format limit"));
    }
    let kind = rec[16];
    if first && kind != KIND_SNAPSHOT {
        return Err(FormatError::corruption("MAN-R1", offset + 16, "record 0 is not a SNAPSHOT"));
    }
    if !(KIND_SNAPSHOT..=KIND_COMPACTION).contains(&kind) {
        return Err(FormatError::corruption("MAN-R2", offset + 16, "invalid record kind"));
    }
    if rec[17] != 0 || rec[18..24].iter().any(|&b| b != 0) {
        return Err(FormatError::corruption("MAN-R2", offset + 17, "record flags or reserved bytes are not zero"));
    }
    let payload_end = RECORD_HEADER_LEN + payload_len;
    if !slot_is_zero(&rec[payload_end..]) {
        return Err(FormatError::corruption("AEAD-1", offset + payload_end as u64, "AEAD slot is not zero"));
    }
    let (edit, ignored) = decode_fields(kind, &rec[RECORD_HEADER_LEN..payload_end], offset + RECORD_HEADER_LEN as u64)?;
    Ok(Record { edit_seq, edit, ignored })
}

/// Decodes the record at the start of `buf` (at file offset `offset`),
/// returning it and its length. An incomplete record, a `payload_len` out of
/// 1..=256 MiB or a CRC mismatch are MAN-R2 here (the replay stops there,
/// MAN-R2/MAN-R3); the rest is [`check_record`] for a record after record 0.
pub fn decode_record(buf: &[u8], generation: u64, offset: u64, expected_edit_seq: u64) -> Result<(Record, usize)> {
    let len = match record_bounds(buf, 0) {
        Some(len) => len,
        None => return Err(FormatError::corruption("MAN-R2", offset, "incomplete or out-of-range record")),
    };
    let rec = &buf[..len];
    if !record_crc_ok(rec, generation) {
        return Err(FormatError::corruption("MAN-R2", offset + 4, "record CRC mismatch"));
    }
    Ok((check_record(rec, offset, expected_edit_seq, false)?, len))
}

/// Length of the record at `at` if its header is complete, `payload_len` is
/// in range and it fits in `buf`.
fn record_bounds(buf: &[u8], at: usize) -> Option<usize> {
    let rest = buf.len().checked_sub(at)?;
    if rest < RECORD_HEADER_LEN {
        return None;
    }
    let payload_len = le_u32(buf, at);
    if payload_len == 0 || u64::from(payload_len) > MAX_META_PAYLOAD {
        return None;
    }
    let len = record_len(payload_len);
    (len <= rest as u64).then_some(len as usize)
}

/// The recovered LSM shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestState {
    pub next_file: u64,
    pub last_seq: u64,
    pub log_number: u64,
    /// Seven levels; L0 newest first, deeper levels by `smallest`.
    pub levels: Vec<Vec<FileMeta>>,
}

impl Default for ManifestState {
    fn default() -> Self {
        ManifestState { next_file: 0, last_seq: 0, log_number: 0, levels: vec![Vec::new(); NUM_LEVELS] }
    }
}

impl ManifestState {
    /// Applies a record (§8.5). `first` is whether it is record 0 of the
    /// file. A `SNAPSHOT` elsewhere, `CLEAN` with `log_number ≠ next_file`
    /// and deleting a file absent from its level are MAN-R2; a decreasing
    /// `next_file`, `last_seq` or `log_number` is MAN-A1.
    pub fn apply(&mut self, rec: &Record, first: bool) -> Result<()> {
        let r2 = |detail| Err(FormatError::corruption("MAN-R2", 0, detail));
        let a1 = |old: u64, new: u64| {
            if new < old {
                Err(FormatError::corruption("MAN-A1", 0, "next_file, last_seq or log_number decreased"))
            } else {
                Ok(())
            }
        };
        match &rec.edit {
            Edit::Snapshot { next_file, last_seq, log_number, clean, files, .. } => {
                if !first {
                    return r2("SNAPSHOT after record 0");
                }
                if *clean && log_number != next_file {
                    return r2("CLEAN SNAPSHOT with log_number != next_file");
                }
                let mut levels = vec![Vec::new(); NUM_LEVELS];
                for f in files {
                    levels[f.level as usize].push(f.clone());
                }
                *self = ManifestState { next_file: *next_file, last_seq: *last_seq, log_number: *log_number, levels };
            }
            Edit::SessionOpen { next_file, last_seq, log_number, files }
            | Edit::Flush { next_file, last_seq, log_number, files, .. } => {
                a1(self.next_file, *next_file)?;
                a1(self.last_seq, *last_seq)?;
                a1(self.log_number, *log_number)?;
                self.next_file = *next_file;
                self.last_seq = *last_seq;
                self.log_number = *log_number;
                let old = std::mem::take(&mut self.levels[0]);
                self.levels[0] = files.iter().cloned().chain(old).collect();
            }
            Edit::Compaction { next_file, add, delete, .. } => {
                if let Some(n) = next_file {
                    a1(self.next_file, *n)?;
                    self.next_file = *n;
                }
                for &(level, number) in delete {
                    let files = &mut self.levels[level as usize];
                    match files.iter().position(|f| f.number == number) {
                        Some(i) => {
                            files.remove(i);
                        }
                        None => return r2("deleted file not in its level"),
                    }
                }
                for f in add {
                    let files = &mut self.levels[f.level as usize];
                    let at = files.partition_point(|g| g.smallest <= f.smallest);
                    files.insert(at, f.clone());
                }
            }
        }
        Ok(())
    }

    /// MAN-S1..MAN-S5, in order, over the whole state. A replay checks
    /// each record after the first only on what it changes
    /// ([`StateValidator`]), with the same result.
    pub fn validate(&self) -> Result<()> {
        self.check_scalars()?;
        let mut numbers = HashSet::new();
        for f in self.levels.iter().flatten() {
            check_number(f, self.next_file, &mut numbers)?;
        }
        for f in self.levels.iter().flatten() {
            check_file(f, self.last_seq)?;
        }
        // MAN-S3.
        for level in &self.levels[1..] {
            if level.windows(2).any(|w| !w[0].ends_before(&w[1].smallest)) {
                return Err(s3());
            }
        }
        // MAN-S4.
        check_l0_order(&self.levels[0])?;
        // MAN-S5 (num_levels is checked when decoding).
        if self.levels.iter().any(|l| l.len() > MAX_FILES_PER_LEVEL) {
            return Err(s5());
        }
        Ok(())
    }

    /// MAN-S1 on the scalars: `1 ≤ next_file ≤ 2^48−1`, `1 ≤ log_number ≤
    /// next_file` and `last_seq ≤ 2^63−1` (the format limit of §1.4).
    fn check_scalars(&self) -> Result<()> {
        if self.next_file == 0
            || self.next_file > MAX_FILE_NUMBER
            || self.log_number == 0
            || self.log_number > self.next_file
            || self.last_seq > MAX_SEQ
        {
            return Err(FormatError::corruption("MAN-S1", 0, "next_file, log_number or last_seq out of range"));
        }
        Ok(())
    }

    /// Number of files in every level.
    pub fn file_count(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }
}

/// MAN-S1 for one file: number ≥ 1, below `next_file` and not in `numbers`
/// (where it is then added).
fn check_number(f: &FileMeta, next_file: u64, numbers: &mut HashSet<u64>) -> Result<()> {
    if f.number == 0 || f.number >= next_file || !numbers.insert(f.number) {
        return Err(FormatError::corruption("MAN-S1", 0, "file number duplicated or not below next_file"));
    }
    Ok(())
}

/// MAN-S2 for one file.
fn check_file(f: &FileMeta, last_seq: u64) -> Result<()> {
    let keys_ok = if f.largest_exclusive { f.smallest < f.largest } else { f.smallest <= f.largest };
    if (f.point_entries == 0 && f.range_fragments == 0)
        || !keys_ok
        || f.smallest_seq == 0
        || f.smallest_seq > f.largest_seq
        || f.largest_seq > last_seq
    {
        return Err(FormatError::corruption("MAN-S2", 0, "invalid FileMeta"));
    }
    Ok(())
}

fn s3() -> FormatError {
    FormatError::corruption("MAN-S3", 0, "level files unordered or overlapping")
}

fn s5() -> FormatError {
    FormatError::corruption("MAN-S5", 0, "too many files in a level")
}

/// MAN-S1..S5 during a replay, in time proportional to each record's
/// changes instead of to the whole state.
///
/// Record 0 (a `SNAPSHOT`) is validated in full ([`ManifestState::validate`]).
/// A later record is checked only on what it changes, which is equivalent to
/// validating the whole state again because the state before it was valid:
///
/// * MAN-S1: the scalars are rechecked; MAN-A1 keeps `next_file` from
///   decreasing, so the files already in the state are still below it, and
///   the added files are checked against a set of the numbers in the state.
/// * MAN-S2: on the added files; MAN-A1 keeps `last_seq` from decreasing.
/// * MAN-S3: on the neighbours of every added file. Deleting a file keeps a
///   level ordered: "ends before the next one starts" is transitive through
///   the deleted file, which satisfied MAN-S2.
/// * MAN-S4: over L0 when files were prepended to it. Deleting files keeps
///   it, since it is a rule on pairs.
/// * MAN-S5: on the levels that grew.
///
/// The rules are checked in that order, as [`ManifestState::validate`]
/// does, so the reported rule is the same. After an error the validator no
/// longer matches any state and must not be used again.
#[derive(Clone, Debug)]
pub struct StateValidator {
    numbers: HashSet<u64>,
}

impl StateValidator {
    /// Validates `state` in full and indexes its file numbers.
    pub fn new(state: &ManifestState) -> Result<Self> {
        state.validate()?;
        Ok(StateValidator { numbers: state.levels.iter().flatten().map(|f| f.number).collect() })
    }

    /// Validates `state` after `edit` was applied ([`ManifestState::apply`])
    /// to the last state this validator accepted. A `SNAPSHOT` (only valid
    /// as record 0) is validated in full.
    pub fn check(&mut self, state: &ManifestState, edit: &Edit) -> Result<()> {
        let (added, deleted): (&[FileMeta], &[(u8, u64)]) = match edit {
            Edit::Snapshot { .. } => {
                *self = StateValidator::new(state)?;
                return Ok(());
            }
            Edit::SessionOpen { files, .. } | Edit::Flush { files, .. } => (files, &[]),
            Edit::Compaction { add, delete, .. } => (add, delete),
        };
        state.check_scalars()?;
        for (_, number) in deleted {
            self.numbers.remove(number);
        }
        for f in added {
            check_number(f, state.next_file, &mut self.numbers)?;
        }
        for f in added {
            check_file(f, state.last_seq)?;
        }
        for f in added.iter().filter(|f| f.level > 0) {
            let files = &state.levels[f.level as usize];
            // The level is sorted by `smallest` (MAN-S3 held before, and
            // `apply` inserts by `smallest`); numbers are unique (MAN-S1).
            let from = files.partition_point(|g| g.smallest < f.smallest);
            let Some(at) = files[from..].iter().position(|g| g.number == f.number) else {
                // Unreachable: `apply` inserted the file in its level.
                return state.validate();
            };
            let i = from + at;
            let after_prev = i == 0 || files[i - 1].ends_before(&files[i].smallest);
            let before_next = i + 1 == files.len() || files[i].ends_before(&files[i + 1].smallest);
            if !after_prev || !before_next {
                return Err(s3());
            }
        }
        if added.iter().any(|f| f.level == 0) {
            check_l0_order(&state.levels[0])?;
        }
        if added.iter().any(|f| state.levels[f.level as usize].len() > MAX_FILES_PER_LEVEL) {
            return Err(s5());
        }
        Ok(())
    }
}

/// MAN-S4 in O(n log n): for files `i` before `j` in L0 whose key ranges
/// overlap, `smallest_seq(i) > largest_seq(j)`. A sweep over the key space
/// keeps the active files in a segment tree indexed by L0 position; when a
/// file starts, every active file overlaps it.
fn check_l0_order(l0: &[FileMeta]) -> Result<()> {
    let n = l0.len();
    if n < 2 {
        return Ok(());
    }
    // At equal keys: exclusive ends (0), then starts (1), then inclusive
    // ends (2).
    let mut events: Vec<(&[u8], u8, usize)> = Vec::with_capacity(2 * n);
    for (i, f) in l0.iter().enumerate() {
        events.push((&f.smallest, 1, i));
        events.push((&f.largest, if f.largest_exclusive { 0 } else { 2 }, i));
    }
    events.sort_unstable();
    let size = n.next_power_of_two();
    let mut min_ss = vec![u64::MAX; 2 * size];
    let mut max_ls = vec![0u64; 2 * size];
    let set = |min_ss: &mut [u64], max_ls: &mut [u64], i: usize, ss: u64, ls: u64| {
        let mut p = i + size;
        min_ss[p] = ss;
        max_ls[p] = ls;
        while p > 1 {
            p /= 2;
            min_ss[p] = min_ss[2 * p].min(min_ss[2 * p + 1]);
            max_ls[p] = max_ls[2 * p].max(max_ls[2 * p + 1]);
        }
    };
    // (min smallest_seq, max largest_seq) over active positions [lo, hi).
    let query = |min_ss: &[u64], max_ls: &[u64], lo: usize, hi: usize| {
        let (mut lo, mut hi) = (lo + size, hi + size);
        let (mut mn, mut mx) = (u64::MAX, 0u64);
        while lo < hi {
            if lo & 1 == 1 {
                mn = mn.min(min_ss[lo]);
                mx = mx.max(max_ls[lo]);
                lo += 1;
            }
            if hi & 1 == 1 {
                hi -= 1;
                mn = mn.min(min_ss[hi]);
                mx = mx.max(max_ls[hi]);
            }
            lo /= 2;
            hi /= 2;
        }
        (mn, mx)
    };
    for (_, kind, i) in events {
        if kind == 1 {
            let f = &l0[i];
            let (newer_min_ss, _) = query(&min_ss, &max_ls, 0, i);
            let (_, older_max_ls) = query(&min_ss, &max_ls, i + 1, n);
            if newer_min_ss <= f.largest_seq || older_max_ls >= f.smallest_seq {
                return Err(FormatError::corruption("MAN-S4", 0, "overlapping L0 files out of sequence order"));
            }
            set(&mut min_ss, &mut max_ls, i, f.smallest_seq, f.largest_seq);
        } else {
            set(&mut min_ss, &mut max_ls, i, u64::MAX, 0);
        }
    }
    Ok(())
}

/// Result of [`replay_manifest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestReplay {
    pub header: Header,
    pub state: ManifestState,
    /// MAN-R4: the last valid record is a `SNAPSHOT` with `CLEAN`.
    pub clean: bool,
    pub records: u64,
    /// `edit_seq` the next appended record must carry.
    pub next_edit_seq: u64,
    /// `P`: the end of the last valid record.
    pub valid_len: u64,
    pub end: End,
}

/// Reads the record at `pos` if it is complete, in range and its CRC
/// verifies (otherwise the replay stops at `pos`).
fn read_record<S: ByteSource>(
    w: &mut Window<'_, S>,
    pos: u64,
    generation: u64,
) -> std::result::Result<Option<Vec<u8>>, S::Error> {
    let len = w.len();
    if len - pos < RECORD_HEADER_LEN as u64 {
        return Ok(None);
    }
    let payload_len = le_u32(&w.get(pos, 4)?, 0);
    if payload_len == 0 || u64::from(payload_len) > MAX_META_PAYLOAD {
        return Ok(None);
    }
    let rlen = record_len(payload_len);
    if rlen > len - pos {
        return Ok(None);
    }
    let rec = w.get(pos, rlen as usize)?;
    Ok(record_crc_ok(&rec, generation).then(|| rec.into_owned()))
}

/// MAN-R3: whether a record with `edit_seq ∈ [expected, expected +
/// ⌊(len − p)/65⌋ + 1]`, a length in range and a valid CRC starts at some
/// offset after `p`.
fn find_candidate<S: ByteSource>(
    w: &mut Window<'_, S>,
    p: u64,
    expected: u64,
    generation: u64,
) -> std::result::Result<Option<u64>, S::Error> {
    let len = w.len();
    let max_seq = expected.saturating_add((len - p) / MIN_RECORD_LEN).saturating_add(1);
    let mut q = p + 1;
    while q + RECORD_HEADER_LEN as u64 <= len {
        let (payload_len, edit_seq) = {
            let h = w.get(q, RECORD_HEADER_LEN)?;
            (le_u32(&h, 0), le_u64(&h, 8))
        };
        let rlen = record_len(payload_len);
        let plausible = payload_len != 0
            && u64::from(payload_len) <= MAX_META_PAYLOAD
            && rlen <= len - q
            && (expected..=max_seq).contains(&edit_seq);
        if plausible && record_crc_ok(&w.get(q, rlen as usize)?, generation) {
            return Ok(Some(q));
        }
        q += 1;
    }
    Ok(None)
}

/// Replays a MANIFEST (§8.7). Calls `on_record` with the offset and contents
/// of every valid record, in order, after it has been applied and the state
/// validated.
///
/// The records passed to `on_record` are **provisional**: the tail is
/// classified after the last valid record and can still fail with MAN-R3.
/// The callback may only inspect or collect; recovery writes nothing before
/// the whole classification succeeded (v1.md §9.4, S-1).
///
/// Errors: HDR-1..9 (no HDR-10: the MANIFEST defines the `db_id`), MAN-R1
/// (generation 0, missing or invalid record 0, `prev_generation ≠
/// generation − 1`), MAN-R2, MAN-A1, MAN-S1..S5, VAR-*, AEAD-1 and MAN-R3
/// (damage followed by a later record).
pub fn replay_manifest<S: ByteSource>(
    src: &mut S,
    mut on_record: impl FnMut(u64, &Record) -> std::result::Result<(), S::Error>,
) -> std::result::Result<ManifestReplay, ReadError<S::Error>> {
    let mut w = Window::new(src);
    let len = w.len();
    let prefix = w.get(0, len.min(u64::from(MAX_HEADER_LEN)) as usize).map_err(ReadError::Source)?.into_owned();
    let header = decode_header(&prefix, len, FileKind::Manifest, &HeaderExpect::default())?;
    let generation = header.file_number;
    if generation == 0 {
        return Err(FormatError::corruption("MAN-R1", 56, "generation 0").into());
    }
    let mut pos = u64::from(header.header_len);
    let mut expected = header.per_type;
    let mut state = ManifestState::default();
    let mut validator: Option<StateValidator> = None;
    let mut clean = false;
    let mut records = 0u64;
    loop {
        let Some(bytes) = read_record(&mut w, pos, generation).map_err(ReadError::Source)? else {
            if records == 0 {
                return Err(FormatError::corruption("MAN-R1", pos, "record 0 missing or damaged").into());
            }
            break;
        };
        let first = records == 0;
        let rec = check_record(&bytes, pos, expected, first)?;
        if let Edit::Snapshot { prev_generation, clean: c, .. } = &rec.edit {
            if first && prev_generation.checked_add(1) != Some(generation) {
                return Err(FormatError::corruption("MAN-R1", pos, "prev_generation is not generation - 1").into());
            }
            clean = *c;
        } else {
            clean = false;
        }
        state.apply(&rec, first).map_err(|e| e.shifted(pos))?;
        match &mut validator {
            None => validator = Some(StateValidator::new(&state).map_err(|e| e.shifted(pos))?),
            Some(v) => v.check(&state, &rec.edit).map_err(|e| e.shifted(pos))?,
        }
        on_record(pos, &rec).map_err(ReadError::Source)?;
        records += 1;
        // check_record bounded edit_seq to 2^63−1: this cannot overflow.
        expected += 1;
        pos += bytes.len() as u64;
    }
    let end = if pos == len {
        End::Clean
    } else if w.all_zero_from(pos).map_err(ReadError::Source)? {
        End::ZeroPadded
    } else if let Some(q) = find_candidate(&mut w, pos, expected, generation).map_err(ReadError::Source)? {
        return Err(FormatError::corruption("MAN-R3", q, "damage followed by a later record").into());
    } else {
        End::Torn
    };
    Ok(ManifestReplay { header, state, clean, records, next_edit_seq: expected, valid_len: pos, end })
}

/// A whole MANIFEST: the header and the records, in order.
pub fn encode_manifest(header: &Header, records: &[Record]) -> Vec<u8> {
    let mut out = header.encode().to_vec();
    for r in records {
        out.extend(r.encode(header.file_number));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(level: u8, number: u64, s: &[u8], l: &[u8], ss: u64, ls: u64) -> FileMeta {
        FileMeta {
            level,
            number,
            size: 1000,
            point_entries: 1,
            range_fragments: 0,
            smallest_seq: ss,
            largest_seq: ls,
            largest_exclusive: false,
            smallest: s.to_vec(),
            largest: l.to_vec(),
        }
    }

    fn naive_l0(l0: &[FileMeta]) -> bool {
        (0..l0.len())
            .all(|i| (i + 1..l0.len()).all(|j| !l0[i].overlaps(&l0[j]) || l0[i].smallest_seq > l0[j].largest_seq))
    }

    #[test]
    fn l0_sweep_matches_naive_check() {
        let mut rng = kerno_testkit::rng::Rng::new(4);
        for _ in 0..20_000 {
            let n = 1 + rng.below(6) as usize;
            let l0: Vec<FileMeta> = (0..n)
                .map(|i| {
                    let a = rng.below(10) as u8;
                    let b = a + rng.below(4) as u8;
                    let ss = 1 + rng.below(20);
                    let mut f = meta(0, i as u64 + 1, &[a], &[b], ss, ss + rng.below(3));
                    f.largest_exclusive = a < b && rng.chance(1, 3);
                    f
                })
                .collect();
            assert_eq!(check_l0_order(&l0).is_ok(), naive_l0(&l0), "{l0:?}");
        }
    }

    #[test]
    fn replay_applies_records_and_detects_clean_state() {
        let db = [3; 16];
        let header = Header::manifest(db, 1, 1);
        let snap = Record {
            edit_seq: 1,
            edit: Edit::Snapshot {
                next_file: 1,
                last_seq: 0,
                log_number: 1,
                clean: true,
                prev_generation: 0,
                files: vec![],
            },
            ignored: vec![],
        };
        let flush = Record {
            edit_seq: 2,
            edit: Edit::Flush {
                next_file: 3,
                last_seq: 9,
                log_number: 2,
                reason: None,
                files: vec![meta(0, 1, b"a", b"z", 1, 9)],
            },
            ignored: vec![],
        };
        let file = encode_manifest(&header, std::slice::from_ref(&snap));
        let r = replay_manifest(&mut &file[..], |_, _| Ok(())).map_err(ReadError::into_format).unwrap();
        assert!(r.clean);
        let file = encode_manifest(&header, &[snap, flush]);
        let r = replay_manifest(&mut &file[..], |_, _| Ok(())).map_err(ReadError::into_format).unwrap();
        assert!(!r.clean);
        assert_eq!((r.records, r.next_edit_seq, r.end, r.state.last_seq), (2, 3, End::Clean, 9));
        assert_eq!(r.state.levels[0].len(), 1);
        // A torn tail is accepted; damage before a later record is not.
        let mut torn = file.clone();
        *torn.last_mut().unwrap() ^= 1;
        let r = replay_manifest(&mut &torn[..], |_, _| Ok(())).map_err(ReadError::into_format).unwrap();
        assert_eq!((r.records, r.end), (1, End::Torn));
        assert!(r.clean);
        let mut empty = file[..96].to_vec();
        assert_eq!(
            replay_manifest(&mut &empty[..], |_, _| Ok(())).map_err(ReadError::into_format).unwrap_err().rule,
            "MAN-R1"
        );
        empty.clear();
        assert_eq!(
            replay_manifest(&mut &empty[..], |_, _| Ok(())).map_err(ReadError::into_format).unwrap_err().rule,
            "HDR-1"
        );
    }

    /// A file with keys in a small space and sequences mostly in `lo..=hi`.
    fn random_meta(rng: &mut kerno_testkit::rng::Rng, level: u8, number: u64, lo: u64, hi: u64) -> FileMeta {
        let a = rng.below(40) as u8;
        let b = a + rng.below(6) as u8;
        let (ss, ls) = if lo <= hi && rng.chance(19, 20) {
            let ss = lo + rng.below(hi - lo + 1);
            (ss, ss + rng.below(hi - ss + 1))
        } else {
            let ss = 1 + rng.below(hi.saturating_add(3));
            (ss, ss + rng.below(4))
        };
        let mut f = meta(level, number, &[a], &[b], ss, ls);
        f.largest_exclusive = a < b && rng.chance(1, 4);
        if rng.chance(1, 40) {
            f.point_entries = 0;
        }
        f
    }

    /// The replay's incremental validation reports exactly what a full
    /// validation of the state after every record reports.
    #[test]
    fn incremental_validation_matches_full() {
        let mut rng = kerno_testkit::rng::Rng::new(0x5A11_DA7E);
        let mut seen = HashSet::new();
        let mut checked = 0;
        for _ in 0..3_000 {
            // A valid starting state: disjoint files in L1..L3, L0 newest first.
            let mut files = Vec::new();
            let mut number = 1;
            for i in 0..rng.below(4) {
                files.push(meta(0, number, &[i as u8], &[40 + i as u8], 40 - 3 * i, 41 - 3 * i));
                number += 1;
            }
            for level in 1..4u8 {
                let mut key = 0u8;
                while key < 40 && rng.chance(2, 3) {
                    let end = key + rng.below(3) as u8;
                    files.push(meta(level, number, &[key], &[end], 1, 5));
                    number += 1;
                    key = end + 1 + rng.below(4) as u8;
                }
            }
            let next_file = number + rng.below(3);
            let mut state = ManifestState::default();
            let snap = Record {
                edit_seq: 1,
                edit: Edit::Snapshot {
                    next_file,
                    last_seq: 50,
                    log_number: next_file,
                    clean: false,
                    prev_generation: 0,
                    files,
                },
                ignored: vec![],
            };
            state.apply(&snap, true).unwrap();
            let mut v = StateValidator::new(&state).unwrap();
            for step in 0..60 {
                let (next_file, last_seq, log) = (state.next_file, state.last_seq, state.log_number);
                // Fresh numbers from next_file on; now and then a reused or
                // too-high one (MAN-S1).
                let mut fresh = next_file;
                let mut number = |rng: &mut kerno_testkit::rng::Rng| {
                    if rng.chance(1, 30) {
                        1 + rng.below(fresh + 2)
                    } else {
                        fresh += 1;
                        fresh - 1
                    }
                };
                let edit = if rng.chance(1, 2) {
                    let last = if rng.chance(1, 50) { MAX_SEQ + 1 } else { last_seq + rng.below(10) };
                    let files: Vec<FileMeta> = (0..rng.below(3))
                        .map(|_| {
                            let n = number(&mut rng);
                            random_meta(&mut rng, 0, n, last_seq + 1, last)
                        })
                        .collect();
                    let next = if rng.chance(1, 50) { fresh.saturating_sub(1) } else { fresh + rng.below(2) };
                    let log = if rng.chance(1, 50) { next + 1 } else { (log + rng.below(3)).min(next) };
                    if rng.chance(1, 2) {
                        Edit::Flush { next_file: next, last_seq: last, log_number: log, reason: None, files }
                    } else {
                        Edit::SessionOpen { next_file: next, last_seq: last, log_number: log, files }
                    }
                } else {
                    let present: Vec<FileMeta> = state.levels.iter().flatten().cloned().collect();
                    let mut delete = Vec::new();
                    let mut add = Vec::new();
                    for f in &present {
                        if rng.chance(1, 8) {
                            delete.push((f.level, f.number));
                            // A trivial move: the same file one level deeper.
                            if f.level < 6 && rng.chance(1, 2) {
                                add.push(FileMeta { level: f.level + 1, ..f.clone() });
                            }
                        }
                    }
                    for _ in 0..rng.below(4) {
                        let level = 1 + rng.below(6) as u8;
                        let n = number(&mut rng);
                        add.push(random_meta(&mut rng, level, n, 1, last_seq));
                    }
                    let next = if rng.chance(1, 50) { fresh.saturating_sub(1) } else { fresh + rng.below(2) };
                    Edit::Compaction { reason: REASON_BACKGROUND, next_file: Some(next), add, delete }
                };
                let rec = Record { edit_seq: 2 + step, edit, ignored: vec![] };
                if state.apply(&rec, false).is_err() {
                    break;
                }
                let full = state.validate().map_err(|e| e.rule);
                let incremental = v.check(&state, &rec.edit).map_err(|e| e.rule);
                assert_eq!(incremental, full, "{rec:?} on {state:?}");
                checked += 1;
                match full {
                    Ok(()) => seen.insert("ok"),
                    Err(rule) => {
                        seen.insert(rule);
                        break;
                    }
                };
            }
        }
        for outcome in ["ok", "MAN-S1", "MAN-S2", "MAN-S3", "MAN-S4"] {
            assert!(seen.contains(outcome), "no record ended in {outcome} ({checked} checked)");
        }
    }

    /// WAL-R2's streaming applies to the MANIFEST too: reads of at most
    /// 1 MiB (a larger record whole), never past the end.
    #[test]
    fn replay_streams_manifests_larger_than_a_read_chunk() {
        use crate::format::READ_CHUNK;
        use crate::format::testing::Recording;
        let header = Header::manifest([3; 16], 2, 5);
        let mut records = vec![Record {
            edit_seq: 5,
            edit: Edit::Snapshot {
                next_file: 10,
                last_seq: 100,
                log_number: 9,
                clean: false,
                prev_generation: 1,
                files: vec![],
            },
            ignored: vec![],
        }];
        for (i, size) in [300_000, 700_000, 1_200_000, 450_000, 650_000, 20].into_iter().enumerate() {
            let i = i as u64;
            records.push(Record {
                edit_seq: 6 + i,
                edit: Edit::Flush {
                    next_file: 11 + i,
                    last_seq: 101 + i,
                    log_number: 10 + i,
                    reason: None,
                    files: vec![meta(0, 10 + i, b"a", b"z", 101 + i, 101 + i)],
                },
                ignored: vec![(1024, vec![b'x'; size])],
            });
        }
        let file = encode_manifest(&header, &records);
        assert!(file.len() > 3 << 20);
        let offsets: Vec<usize> = records
            .iter()
            .scan(96, |at, r| {
                let here = *at;
                *at += r.encode(2).len();
                Some(here)
            })
            .collect();
        let largest = records.iter().map(|r| r.encode(2).len()).max().unwrap();
        assert!(largest > READ_CHUNK);
        let replay = |bytes: &[u8]| {
            let mut src = Recording::new(bytes, READ_CHUNK.max(largest));
            let mut got = Vec::new();
            let r = replay_manifest(&mut src, |off, rec| {
                got.push((off as usize, rec.clone()));
                Ok(())
            })
            .map_err(ReadError::into_format);
            (r, got)
        };
        let (r, got) = replay(&file);
        let r = r.unwrap();
        assert_eq!((r.records, r.end, r.next_edit_seq), (records.len() as u64, End::Clean, 12));
        assert_eq!(got, offsets.iter().copied().zip(records.iter().cloned()).collect::<Vec<_>>());
        assert_eq!(r.state.levels[0].len(), 6);
        // A torn tail after several MiB.
        let torn = &file[..file.len() - 30];
        let r = replay(torn).0.unwrap();
        assert_eq!((r.records, r.end, r.valid_len), (records.len() as u64 - 1, End::Torn, offsets[6] as u64));
        // Zero padding over several chunks; a stray byte in a later chunk.
        let mut padded = file.clone();
        padded.resize(file.len() + (5 << 19), 0);
        assert_eq!(replay(&padded).0.unwrap().end, End::ZeroPadded);
        padded[file.len() + (3 << 19)] = 1;
        assert_eq!(replay(&padded).0.unwrap().end, End::Torn);
        // Damage after the first MiB followed by later records: MAN-R3.
        for (damaged_record, at) in [(2, 1000), (3, 1_100_000), (4, 50)] {
            let mut damaged = file.clone();
            damaged[offsets[damaged_record] + at] ^= 1;
            let e = replay(&damaged).0.unwrap_err();
            assert_eq!((e.rule, e.offset), ("MAN-R3", offsets[damaged_record + 1] as u64));
        }
    }

    #[test]
    fn sequences_read_from_disk_are_bounded() {
        let snap = |edit_seq, last_seq| Record {
            edit_seq,
            edit: Edit::Snapshot {
                next_file: 5,
                last_seq,
                log_number: 5,
                clean: false,
                prev_generation: 0,
                files: vec![],
            },
            ignored: vec![],
        };
        let replay = |first_edit_seq: u64, records: &[Record]| {
            let file = encode_manifest(&Header::manifest([3; 16], 1, first_edit_seq), records);
            replay_manifest(&mut &file[..], |_, _| Ok(())).map_err(ReadError::into_format)
        };
        // last_seq above 2^63−1 (§1.4) is MAN-S1.
        assert_eq!(replay(1, &[snap(1, MAX_SEQ)]).unwrap().state.last_seq, MAX_SEQ);
        assert_eq!(replay(1, &[snap(1, MAX_SEQ + 1)]).unwrap_err().rule, "MAN-S1");
        assert_eq!(replay(1, &[snap(1, u64::MAX)]).unwrap_err().rule, "MAN-S1");
        // edit_seq outside 1..=2^63−1 (§1.4): MAN-R1 for record 0, MAN-R2 after.
        assert_eq!(replay(u64::MAX, &[snap(u64::MAX, 0)]).unwrap_err().rule, "MAN-R1");
        assert_eq!(replay(0, &[snap(0, 0)]).unwrap_err().rule, "MAN-R1");
        let flush = |edit_seq| Record {
            edit_seq,
            edit: Edit::Flush { next_file: 5, last_seq: 0, log_number: 5, reason: None, files: vec![] },
            ignored: vec![],
        };
        let r = replay(MAX_SEQ, &[snap(MAX_SEQ, 0)]).unwrap();
        assert_eq!(r.next_edit_seq, MAX_SEQ + 1);
        assert_eq!(replay(MAX_SEQ, &[snap(MAX_SEQ, 0), flush(MAX_SEQ + 1)]).unwrap_err().rule, "MAN-R2");
    }
}
