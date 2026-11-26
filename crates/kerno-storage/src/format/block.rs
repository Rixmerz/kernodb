//! SST block encapsulation (v1.md §7.2) and data blocks (§7.3, SST-R1).
//!
//! Every SST block, data or meta, is `payload ‖ slot[40] ‖ crc u32` with
//! `crc = crc(file_number ‖ block_offset; payload ‖ slot)`; the index and the
//! footer store its length, `payload_len + 44`.

use super::coding::{Reader, le_u32, put_u32, put_varint};
use super::crc::{block_ctx, crc_ctx};
use super::features::{SLOT_LEN, slot_is_zero};
use std::borrow::Cow;

use super::limits::{MAX_DATA_BLOCK_PAYLOAD, MAX_KEY_LEN, MAX_SEQ, MAX_VALUE_LEN};
use super::{FormatError, Result};

/// Bytes the encapsulation adds to a payload: slot and CRC.
pub const BLOCK_TRAILER_LEN: usize = SLOT_LEN + 4;
/// Smallest data block payload: one three-byte entry, one offset, count.
pub const MIN_DATA_BLOCK_PAYLOAD: usize = 11;

pub const KIND_PUT: u8 = 1;
pub const KIND_DELETE: u8 = 2;

/// Appends `payload ‖ zero slot ‖ crc` for the block at `block_offset`.
pub fn append_block(out: &mut Vec<u8>, payload: &[u8], file_number: u64, block_offset: u64) {
    let start = out.len();
    out.extend_from_slice(payload);
    out.resize(out.len() + SLOT_LEN, 0);
    let crc = crc_ctx(&block_ctx(file_number, block_offset), &out[start..]);
    put_u32(out, crc);
}

/// The encapsulated block for `payload` at `block_offset`.
pub fn seal_block(payload: &[u8], file_number: u64, block_offset: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + BLOCK_TRAILER_LEN);
    append_block(&mut out, payload, file_number, block_offset);
    out
}

/// Recomputes the CRC of a whole (possibly patched) block in place.
pub fn reseal_block(block: &mut [u8], file_number: u64, block_offset: u64) {
    let at = block.len() - 4;
    let crc = crc_ctx(&block_ctx(file_number, block_offset), &block[..at]);
    block[at..].copy_from_slice(&crc.to_le_bytes());
}

/// Checks the encapsulation of a whole block and returns its payload: length
/// at least 44 and payload within `max_payload`, CRC (all reported as
/// `rule`), then a zero slot (AEAD-1).
pub fn open_block<'a>(
    block: &'a [u8],
    file_number: u64,
    block_offset: u64,
    max_payload: u64,
    rule: &'static str,
) -> Result<&'a [u8]> {
    if block.len() < BLOCK_TRAILER_LEN {
        return Err(FormatError::corruption(rule, block_offset, "block shorter than its trailer"));
    }
    let payload_len = block.len() - BLOCK_TRAILER_LEN;
    if payload_len as u64 > max_payload {
        return Err(FormatError::corruption(rule, block_offset, "block payload above the format limit"));
    }
    let crc_at = block.len() - 4;
    if crc_ctx(&block_ctx(file_number, block_offset), &block[..crc_at]) != le_u32(block, crc_at) {
        return Err(FormatError::corruption(rule, block_offset + crc_at as u64, "block CRC mismatch"));
    }
    if !slot_is_zero(&block[payload_len..crc_at]) {
        return Err(FormatError::corruption("AEAD-1", block_offset + payload_len as u64, "AEAD slot is not zero"));
    }
    Ok(&block[..payload_len])
}

/// One point entry of a data block; `value: None` is a point tombstone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<B = Vec<u8>> {
    pub key: B,
    pub seq: u64,
    pub value: Option<B>,
}

impl<B: AsRef<[u8]>> Entry<B> {
    pub fn to_owned_entry(&self) -> Entry<Vec<u8>> {
        Entry {
            key: self.key.as_ref().to_vec(),
            seq: self.seq,
            value: self.value.as_ref().map(|v| v.as_ref().to_vec()),
        }
    }
}

/// Builds one data block payload:
/// `entry* ‖ offsets u32 × count ‖ count u32`.
#[derive(Debug, Default)]
pub struct DataBlockBuilder {
    buf: Vec<u8>,
    offsets: Vec<u32>,
}

impl DataBlockBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends `key_len ‖ seq ‖ kind ‖ [value_len] ‖ key ‖ value`. Keys must
    /// be strictly increasing (the caller's contract, SST-I1).
    pub fn add(&mut self, key: &[u8], seq: u64, value: Option<&[u8]>) {
        self.offsets.push(self.buf.len() as u32);
        put_varint(&mut self.buf, key.len() as u64);
        put_varint(&mut self.buf, seq);
        match value {
            Some(v) => {
                self.buf.push(KIND_PUT);
                put_varint(&mut self.buf, v.len() as u64);
                self.buf.extend_from_slice(key);
                self.buf.extend_from_slice(v);
            }
            None => {
                self.buf.push(KIND_DELETE);
                self.buf.extend_from_slice(key);
            }
        }
    }

    /// Entries in the block.
    pub fn count(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Exact payload length if the block were finished now: entry bytes,
    /// `4 × count` and 4. The writer closes a block when this reaches
    /// `block_size` (§7.10).
    pub fn payload_len(&self) -> usize {
        self.buf.len() + 4 * self.offsets.len() + 4
    }

    /// The payload; the builder is empty afterwards.
    pub fn finish(&mut self) -> Vec<u8> {
        // The next block grows to about this one's size: one allocation
        // instead of a doubling chain per block.
        let cap = self.buf.capacity();
        let mut out = std::mem::replace(&mut self.buf, Vec::with_capacity(cap));
        out.reserve(4 * self.offsets.len() + 4);
        for &o in &self.offsets {
            put_u32(&mut out, o);
        }
        put_u32(&mut out, self.offsets.len() as u32);
        self.offsets.clear();
        out
    }
}

/// The payload of a data block holding `entries`.
pub fn encode_data_block<B: AsRef<[u8]>>(entries: &[Entry<B>]) -> Vec<u8> {
    let mut b = DataBlockBuilder::new();
    for e in entries {
        b.add(e.key.as_ref(), e.seq, e.value.as_ref().map(AsRef::as_ref));
    }
    b.finish()
}

/// Parses a data block payload (SST-R1): `count ≥ 1`; `offsets[i]` is the
/// start of entry `i` (so `offsets[0] = 0` and they increase strictly); the
/// entries cover exactly the bytes before the offsets; `seq ∈ [1, 2^63−1]`;
/// `kind` is PUT or DELETE; keys increase strictly. Lengths above the format
/// limits or beyond the entry area are VAR-3. `base` is the payload's offset.
pub fn parse_data_block(payload: &[u8], base: u64) -> Result<Vec<Entry<&[u8]>>> {
    let mut entries: Vec<Entry<&[u8]>> = Vec::new();
    walk_data_block(payload, base, |count, e| {
        if entries.is_empty() {
            entries.reserve_exact(count);
        }
        entries.push(e);
    })?;
    Ok(entries)
}

/// [`parse_data_block`] that hands each entry to `visit` (with the block's entry count, which
/// the entry area was already found large enough for) instead of collecting them: the same
/// checks, in the same order, with the same errors, and no allocation.
pub fn walk_data_block<'a>(payload: &'a [u8], base: u64, mut visit: impl FnMut(usize, Entry<&'a [u8]>)) -> Result<()> {
    let r1 = |at: u64, detail| FormatError::corruption("SST-R1", base + at, detail);
    let n = payload.len();
    if n < 8 {
        return Err(r1(0, "data block shorter than count and one offset"));
    }
    let count = le_u32(payload, n - 4) as usize;
    if count == 0 {
        return Err(r1((n - 4) as u64, "data block with count 0"));
    }
    let offsets_at = match n.checked_sub(4 + 4 * count) {
        Some(at) => at,
        None => return Err(r1((n - 4) as u64, "offsets beyond the block")),
    };
    let mut r = Reader::new(&payload[..offsets_at], base, "SST-R1");
    // What `visit` may reserve: never more entries than the entry area can hold.
    let room = count.min(offsets_at / 3 + 1);
    let mut prev: Option<&[u8]> = None;
    for i in 0..count {
        if le_u32(payload, offsets_at + 4 * i) as usize != r.pos() {
            return Err(r1((offsets_at + 4 * i) as u64, "offset is not the start of its entry"));
        }
        let at = r.offset();
        let key_len = r.varint()?;
        if key_len > MAX_KEY_LEN as u64 {
            return Err(FormatError::corruption("VAR-3", at, "key length above the format limit"));
        }
        let seq_at = r.offset();
        let seq = r.varint()?;
        if seq == 0 || seq > MAX_SEQ {
            return Err(r1(seq_at - base, "sequence out of range"));
        }
        let kind_at = r.offset();
        let kind = r.u8()?;
        let value_len = match kind {
            KIND_PUT => {
                let vat = r.offset();
                let v = r.varint()?;
                if v > MAX_VALUE_LEN as u64 {
                    return Err(FormatError::corruption("VAR-3", vat, "value length above the format limit"));
                }
                Some(v)
            }
            KIND_DELETE => None,
            _ => return Err(r1(kind_at - base, "unknown entry kind")),
        };
        if key_len + value_len.unwrap_or(0) > r.remaining() as u64 {
            return Err(FormatError::corruption("VAR-3", at, "entry beyond the entry area"));
        }
        let key = r.take(key_len as usize)?;
        let value = match value_len {
            Some(v) => Some(r.take(v as usize)?),
            None => None,
        };
        if prev.is_some_and(|prev| prev >= key) {
            return Err(r1(at - base, "keys not strictly increasing"));
        }
        prev = Some(key);
        visit(room, Entry { key, seq, value });
    }
    r.finish()?;
    Ok(())
}

/// Opens (SST-R1: length, limit, CRC; AEAD-1) and parses a whole data block.
/// Data-block codec byte of an SST with `BLOCK_LZ4`: the payload as is.
pub const CODEC_RAW: u8 = 0;
/// Data-block codec byte: `raw_len: u32 LE ‖ LZ4 block`.
pub const CODEC_LZ4: u8 = 1;
/// The most bytes a stored data-block payload may take: the payload plus the
/// codec byte and the length of a compressed one.
pub const MAX_STORED_DATA_PAYLOAD: u64 = MAX_DATA_BLOCK_PAYLOAD + 5;

/// What an SST with `BLOCK_LZ4` stores for a data-block `payload`: the LZ4
/// form when it is smaller, else the payload as is.
pub fn store_data_payload(payload: &[u8]) -> Vec<u8> {
    let compressed = lz4_flex::block::compress(payload);
    if compressed.len() + 5 < payload.len() + 1 && compressed.len() + 5 >= MIN_DATA_BLOCK_PAYLOAD {
        let mut out = Vec::with_capacity(5 + compressed.len());
        out.push(CODEC_LZ4);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&compressed);
        out
    } else {
        let mut out = Vec::with_capacity(1 + payload.len());
        out.push(CODEC_RAW);
        out.extend_from_slice(payload);
        out
    }
}

/// What an SST with `BLOCK_LZ4` stores for `payload` when the LZ4 pass is
/// skipped: codec 0 and the payload as is.
pub fn store_raw_data_payload(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + payload.len());
    out.push(CODEC_RAW);
    out.extend_from_slice(payload);
    out
}

/// Checks the encapsulation of data block `block` (SST-R1) and returns its
/// data-block payload, decompressed when the SST has `BLOCK_LZ4` (`lz4`). A
/// codec this reader does not know, a length above the format limit or a
/// stream that does not decompress to exactly that length is SST-R1.
pub fn data_block_payload(block: &[u8], file_number: u64, block_offset: u64, lz4: bool) -> Result<Cow<'_, [u8]>> {
    if !lz4 {
        return Ok(Cow::Borrowed(open_block(block, file_number, block_offset, MAX_DATA_BLOCK_PAYLOAD, "SST-R1")?));
    }
    let stored = open_block(block, file_number, block_offset, MAX_STORED_DATA_PAYLOAD, "SST-R1")?;
    let bad = |detail| FormatError::corruption("SST-R1", block_offset, detail);
    match stored.split_first() {
        Some((&CODEC_RAW, body)) => Ok(Cow::Borrowed(body)),
        Some((&CODEC_LZ4, body)) if body.len() >= 4 => {
            let raw_len = le_u32(body, 0) as usize;
            if raw_len as u64 > MAX_DATA_BLOCK_PAYLOAD {
                return Err(bad("compressed data block longer than the format limit"));
            }
            let out =
                lz4_flex::block::decompress(&body[4..], raw_len).map_err(|_| bad("data block does not decompress"))?;
            if out.len() != raw_len {
                return Err(bad("data block decompresses to another length"));
            }
            Ok(Cow::Owned(out))
        }
        _ => Err(bad("unknown data block codec")),
    }
}

pub fn decode_data_block(block: &[u8], file_number: u64, block_offset: u64) -> Result<Vec<Entry<&[u8]>>> {
    let payload = open_block(block, file_number, block_offset, MAX_DATA_BLOCK_PAYLOAD, "SST-R1")?;
    parse_data_block(payload, block_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_block_roundtrip_and_minimum() {
        let entries = vec![
            Entry { key: b"a".to_vec(), seq: 3, value: Some(b"x".to_vec()) },
            Entry { key: b"b".to_vec(), seq: 1, value: None },
        ];
        let payload = encode_data_block(&entries);
        let block = seal_block(&payload, 5, 96);
        let got: Vec<_> = decode_data_block(&block, 5, 96).unwrap().iter().map(Entry::to_owned_entry).collect();
        assert_eq!(got, entries);
        // A block copied to another offset or file does not verify.
        assert_eq!(decode_data_block(&block, 5, 97).unwrap_err().rule, "SST-R1");
        assert_eq!(decode_data_block(&block, 6, 96).unwrap_err().rule, "SST-R1");
        // The smallest payload is 11 bytes: an empty-key DELETE.
        let min = encode_data_block(&[Entry { key: Vec::new(), seq: 1, value: None }]);
        assert_eq!(min.len(), MIN_DATA_BLOCK_PAYLOAD);
        assert!(parse_data_block(&min, 0).is_ok());
    }
}
