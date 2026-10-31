//! SST properties block (v1.md §7.6): TLV with strictly increasing tags;
//! tags 1–12 mandatory; unknown tags below 1024 are `Corruption`, from 1024
//! up they are ignored.

use super::limits::{MAX_KEY_LEN, MAX_SEQ};
use super::manifest::FileMeta;
use super::tlv::{FIRST_IGNORABLE_TAG, TlvIter, put_tlv, put_tlv_u8, put_tlv_varint, value_u8, value_varint};
use super::{FormatError, Result};

pub const TAG_POINT_ENTRIES: u64 = 1;
pub const TAG_POINT_DELETES: u64 = 2;
pub const TAG_RANGE_FRAGMENTS: u64 = 3;
pub const TAG_SMALLEST_SEQ: u64 = 4;
pub const TAG_LARGEST_SEQ: u64 = 5;
pub const TAG_DATA_BLOCKS: u64 = 6;
pub const TAG_RAW_KEY_BYTES: u64 = 7;
pub const TAG_RAW_VALUE_BYTES: u64 = 8;
pub const TAG_SMALLEST_KEY: u64 = 9;
pub const TAG_LARGEST_KEY: u64 = 10;
pub const TAG_LARGEST_EXCLUSIVE: u64 = 11;
pub const TAG_CREATION_REASON: u64 = 12;

/// `creation_reason` values.
pub const REASON_FLUSH: u8 = 1;
pub const REASON_COMPACTION: u8 = 2;
pub const REASON_RECOVERY: u8 = 3;
pub const REASON_DIRECTED: u8 = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Props {
    pub point_entries: u64,
    pub point_deletes: u64,
    pub range_fragments: u64,
    pub smallest_seq: u64,
    pub largest_seq: u64,
    pub data_blocks: u64,
    pub raw_key_bytes: u64,
    pub raw_value_bytes: u64,
    pub smallest_key: Vec<u8>,
    pub largest_key: Vec<u8>,
    pub largest_exclusive: bool,
    pub creation_reason: u8,
    /// Fields with tags ≥ 1024, kept as read (a writer never emits them).
    pub ignored: Vec<(u64, Vec<u8>)>,
}

impl Props {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_tlv_varint(&mut out, TAG_POINT_ENTRIES, self.point_entries);
        put_tlv_varint(&mut out, TAG_POINT_DELETES, self.point_deletes);
        put_tlv_varint(&mut out, TAG_RANGE_FRAGMENTS, self.range_fragments);
        put_tlv_varint(&mut out, TAG_SMALLEST_SEQ, self.smallest_seq);
        put_tlv_varint(&mut out, TAG_LARGEST_SEQ, self.largest_seq);
        put_tlv_varint(&mut out, TAG_DATA_BLOCKS, self.data_blocks);
        put_tlv_varint(&mut out, TAG_RAW_KEY_BYTES, self.raw_key_bytes);
        put_tlv_varint(&mut out, TAG_RAW_VALUE_BYTES, self.raw_value_bytes);
        put_tlv(&mut out, TAG_SMALLEST_KEY, &self.smallest_key);
        put_tlv(&mut out, TAG_LARGEST_KEY, &self.largest_key);
        put_tlv_u8(&mut out, TAG_LARGEST_EXCLUSIVE, u8::from(self.largest_exclusive));
        put_tlv_u8(&mut out, TAG_CREATION_REASON, self.creation_reason);
        for (tag, value) in &self.ignored {
            put_tlv(&mut out, *tag, value);
        }
        out
    }
}

/// Parses a properties payload (`base` is its offset). Framing errors are
/// VAR-1..3 (a key above 16 KiB is VAR-3); tag order, unknown tags below
/// 1024, malformed values, `largest_exclusive ∉ {0, 1}`,
/// `creation_reason ∉ 1..=4` and missing mandatory tags are SST-O6.
pub fn decode_props(payload: &[u8], base: u64) -> Result<Props> {
    let mut p = Props::default();
    let mut seen = 0u16;
    let mut prev: Option<u64> = None;
    for field in TlvIter::new(payload, base, "SST-O4") {
        let f = field?;
        let o6 = |detail| FormatError::corruption("SST-O6", f.offset, detail);
        if prev.is_some_and(|t| t >= f.tag) {
            return Err(o6("property tags not strictly increasing"));
        }
        prev = Some(f.tag);
        match f.tag {
            TAG_POINT_ENTRIES..=TAG_RAW_VALUE_BYTES => {
                let v = value_varint(&f, "SST-O6")?;
                *match f.tag {
                    TAG_POINT_ENTRIES => &mut p.point_entries,
                    TAG_POINT_DELETES => &mut p.point_deletes,
                    TAG_RANGE_FRAGMENTS => &mut p.range_fragments,
                    TAG_SMALLEST_SEQ => &mut p.smallest_seq,
                    TAG_LARGEST_SEQ => &mut p.largest_seq,
                    TAG_DATA_BLOCKS => &mut p.data_blocks,
                    TAG_RAW_KEY_BYTES => &mut p.raw_key_bytes,
                    _ => &mut p.raw_value_bytes,
                } = v;
            }
            TAG_SMALLEST_KEY | TAG_LARGEST_KEY => {
                if f.value.len() > MAX_KEY_LEN {
                    return Err(FormatError::corruption("VAR-3", f.offset, "key length above the format limit"));
                }
                let key = if f.tag == TAG_SMALLEST_KEY { &mut p.smallest_key } else { &mut p.largest_key };
                *key = f.value.to_vec();
            }
            TAG_LARGEST_EXCLUSIVE => match value_u8(&f, "SST-O6")? {
                0 => p.largest_exclusive = false,
                1 => p.largest_exclusive = true,
                _ => return Err(o6("largest_exclusive is not 0 or 1")),
            },
            TAG_CREATION_REASON => {
                let r = value_u8(&f, "SST-O6")?;
                if !(REASON_FLUSH..=REASON_DIRECTED).contains(&r) {
                    return Err(o6("unknown creation_reason"));
                }
                p.creation_reason = r;
            }
            t if t >= FIRST_IGNORABLE_TAG => p.ignored.push((t, f.value.to_vec())),
            _ => return Err(o6("unknown property tag below 1024")),
        }
        if (TAG_POINT_ENTRIES..=TAG_CREATION_REASON).contains(&f.tag) {
            seen |= 1 << f.tag;
        }
    }
    if seen != 0b1_1111_1111_1110 {
        return Err(FormatError::corruption("SST-O6", base, "mandatory property missing"));
    }
    Ok(p)
}

/// SST-O6 beyond the encoding: `point_entries + range_fragments > 0`,
/// `point_deletes ≤ point_entries`, `1 ≤ smallest_seq ≤ largest_seq ≤
/// last_seq`, `smallest_key ≤ largest_key` (`<` when exclusive) and, given
/// the `FileMeta`, every shared field equal. Errors carry `at`, the offset of
/// the properties block.
pub fn check_props(p: &Props, meta: Option<&FileMeta>, last_seq: u64, at: u64) -> Result<()> {
    let o6 = |detail| Err(FormatError::corruption("SST-O6", at, detail));
    if p.point_entries == 0 && p.range_fragments == 0 {
        return o6("table without entries");
    }
    if p.point_deletes > p.point_entries {
        return o6("point_deletes above point_entries");
    }
    if p.smallest_seq == 0 || p.smallest_seq > p.largest_seq || p.largest_seq > last_seq.min(MAX_SEQ) {
        return o6("sequence range invalid or above the MANIFEST's last_seq");
    }
    let keys_ok = if p.largest_exclusive { p.smallest_key < p.largest_key } else { p.smallest_key <= p.largest_key };
    if !keys_ok {
        return o6("smallest_key above largest_key");
    }
    if let Some(m) = meta {
        let same = m.point_entries == p.point_entries
            && m.range_fragments == p.range_fragments
            && m.smallest_seq == p.smallest_seq
            && m.largest_seq == p.largest_seq
            && m.largest_exclusive == p.largest_exclusive
            && m.smallest == p.smallest_key
            && m.largest == p.largest_key;
        if !same {
            return o6("properties differ from the MANIFEST's FileMeta");
        }
    }
    Ok(())
}
