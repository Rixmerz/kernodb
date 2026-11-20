//! SST filter block (v1.md §7.5):
//! `filter_kind u8 = 1 ‖ k u8 ‖ reserved u16 = 0 ‖ nbits u32 ‖ bits[nbits/8]`.

use super::bloom::{self, BITS_PER_KEY, WRITER_K};
use super::coding::{le_u16, le_u32, put_u16, put_u32};
use super::limits::{MAX_BLOOM_BITS, MAX_BLOOM_K, MIN_BLOOM_BITS, MIN_BLOOM_K};
use super::{FormatError, Result};

pub const FILTER_KIND_BLOOM: u8 = 1;
/// Bytes before the bit array.
pub const FILTER_HEADER_LEN: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter<B = Vec<u8>> {
    pub k: u8,
    pub nbits: u32,
    pub bits: B,
}

/// `nbits` the writer uses for `n` keys: `max(10 n, 64)` rounded up to a
/// multiple of 8 (§7.10).
pub fn writer_nbits(n: usize) -> u32 {
    let bits = (n as u64).saturating_mul(BITS_PER_KEY).max(u64::from(MIN_BLOOM_BITS));
    bits.div_ceil(8).saturating_mul(8).min(u64::from(MAX_BLOOM_BITS)) as u32
}

impl Filter<Vec<u8>> {
    /// The writer's filter over the hashes of every point key (§7.10).
    pub fn build(hashes: &[u64]) -> Self {
        let nbits = writer_nbits(hashes.len());
        let mut bits = vec![0u8; nbits as usize / 8];
        for &h in hashes {
            bloom::insert(&mut bits, WRITER_K, h);
        }
        Filter { k: WRITER_K, nbits, bits }
    }
}

impl<B: AsRef<[u8]>> Filter<B> {
    pub fn may_contain(&self, hash: u64) -> bool {
        bloom::may_contain(self.bits.as_ref(), self.k, hash)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FILTER_HEADER_LEN + self.bits.as_ref().len());
        out.push(FILTER_KIND_BLOOM);
        out.push(self.k);
        put_u16(&mut out, 0);
        put_u32(&mut out, self.nbits);
        out.extend_from_slice(self.bits.as_ref());
        out
    }

    pub fn to_owned_filter(&self) -> Filter<Vec<u8>> {
        Filter { k: self.k, nbits: self.nbits, bits: self.bits.as_ref().to_vec() }
    }
}

/// Parses a filter block payload; every violation is SST-O7. `base` is the
/// payload's offset.
pub fn decode_filter(payload: &[u8], base: u64) -> Result<Filter<&[u8]>> {
    let o7 = |at: u64, detail| FormatError::corruption("SST-O7", base + at, detail);
    if payload.len() < FILTER_HEADER_LEN {
        return Err(o7(0, "filter shorter than its header"));
    }
    if payload[0] != FILTER_KIND_BLOOM {
        return Err(o7(0, "unknown filter_kind"));
    }
    let k = payload[1];
    if !(MIN_BLOOM_K..=MAX_BLOOM_K).contains(&k) {
        return Err(o7(1, "k out of range"));
    }
    if le_u16(payload, 2) != 0 {
        return Err(o7(2, "reserved filter bytes are not zero"));
    }
    let nbits = le_u32(payload, 4);
    if nbits < MIN_BLOOM_BITS || !nbits.is_multiple_of(8) || nbits > MAX_BLOOM_BITS {
        return Err(o7(4, "nbits out of range"));
    }
    if (payload.len() - FILTER_HEADER_LEN) as u64 != u64::from(nbits / 8) {
        return Err(o7(4, "filter length does not match nbits"));
    }
    Ok(Filter { k, nbits, bits: &payload[FILTER_HEADER_LEN..] })
}
