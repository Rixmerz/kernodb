//! Data blocks: the unit of I/O, checksumming and caching inside an SST.
//!
//! ```text
//! entry*   key_len varint | seq varint | kind u8 | [value_len varint, PUT only] | key | value
//! offsets  u32 per entry (start offset of each entry, for binary search)
//! count    u32
//! crc      u32  crc32c of everything above
//! ```
//!
//! Keys inside a block are strictly increasing user keys (an SST holds at
//! most one version per key). `Block::parse` verifies the checksum *and* the
//! structure of every entry once, so the accessors only fail on bugs.

use std::ops::Range;

use crate::coding::{Reader, put_u32, put_varint, u32_at, verify_crc_trailer};

pub const KIND_PUT: u8 = 1;
pub const KIND_DELETE: u8 = 2;
/// count u32 + crc u32.
const TRAILER: usize = 8;

#[derive(Default)]
pub struct BlockBuilder {
    buf: Vec<u8>,
    offsets: Vec<u32>,
}

impl BlockBuilder {
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// `value == None` is a tombstone.
    pub fn add(&mut self, key: &[u8], seq: u64, value: Option<&[u8]>) {
        self.offsets.push(self.buf.len() as u32);
        put_varint(&mut self.buf, key.len() as u64);
        put_varint(&mut self.buf, seq);
        match value {
            Some(v) => {
                self.buf.push(KIND_PUT);
                put_varint(&mut self.buf, v.len() as u64);
            }
            None => self.buf.push(KIND_DELETE),
        }
        self.buf.extend_from_slice(key);
        if let Some(v) = value {
            self.buf.extend_from_slice(v);
        }
    }

    pub fn size_estimate(&self) -> usize {
        self.buf.len() + 4 * self.offsets.len() + TRAILER
    }

    /// Returns the serialized block (with trailer) and resets the builder.
    pub fn finish(&mut self) -> Vec<u8> {
        let mut out = std::mem::take(&mut self.buf);
        out.reserve(4 * self.offsets.len() + TRAILER);
        for &o in &self.offsets {
            put_u32(&mut out, o);
        }
        put_u32(&mut out, self.offsets.len() as u32);
        let crc = crc32c::crc32c(&out);
        put_u32(&mut out, crc);
        self.offsets.clear();
        out
    }
}

/// Location of one entry inside a block's buffer.
#[derive(Clone, Debug)]
pub struct EntryPos {
    pub key: Range<usize>,
    pub seq: u64,
    /// `None` for a tombstone.
    pub value: Option<Range<usize>>,
    end: usize,
}

fn decode_at(entries: &[u8], off: usize) -> Option<EntryPos> {
    let mut r = Reader::new(entries.get(off..)?);
    let klen = r.varint_usize()?;
    let seq = r.varint()?;
    let vlen = match r.u8()? {
        KIND_PUT => Some(r.varint_usize()?),
        KIND_DELETE => None,
        _ => return None,
    };
    let key_start = off + r.pos();
    let key_end = key_start.checked_add(klen)?;
    let (value, end) = match vlen {
        Some(n) => {
            let value_end = key_end.checked_add(n)?;
            (Some(key_end..value_end), value_end)
        }
        None => (None, key_end),
    };
    (end <= entries.len()).then_some(EntryPos { key: key_start..key_end, seq, value, end })
}

/// A verified, immutable data block.
pub struct Block {
    /// Entries, offsets and count (checksum stripped).
    data: Vec<u8>,
    count: usize,
    /// Start of the offsets array == end of the entries.
    offsets_at: usize,
}

impl Block {
    /// Verifies checksum and structure. The error is a short description.
    pub fn parse(mut raw: Vec<u8>) -> Result<Block, &'static str> {
        let payload_len = verify_crc_trailer(&raw).ok_or("checksum mismatch")?.len();
        raw.truncate(payload_len);
        let count_at = raw.len().checked_sub(4).ok_or("block too short")?;
        let count = u32_at(&raw, count_at).ok_or("block too short")? as usize;
        let offsets_at =
            count.checked_mul(4).and_then(|n| count_at.checked_sub(n)).ok_or("entry count exceeds block size")?;
        if count == 0 {
            return Err("empty block");
        }
        let block = Block { data: raw, count, offsets_at };
        // Every entry must decode, start where the previous one ended, and
        // keys must be strictly increasing.
        let mut expected = 0;
        let mut prev: Option<Range<usize>> = None;
        for i in 0..count {
            if block.offset(i) != expected {
                return Err("entry offsets are not contiguous");
            }
            let e = decode_at(block.entries(), expected).ok_or("malformed entry")?;
            if let Some(p) = prev
                && block.data[p] >= block.data[e.key.clone()]
            {
                return Err("keys out of order");
            }
            expected = e.end;
            prev = Some(e.key);
        }
        if expected != offsets_at {
            return Err("unexpected bytes after the last entry");
        }
        Ok(block)
    }

    fn entries(&self) -> &[u8] {
        &self.data[..self.offsets_at]
    }

    fn offset(&self, i: usize) -> usize {
        let at = self.offsets_at + 4 * i;
        u32::from_le_bytes(self.data[at..at + 4].try_into().unwrap()) as usize
    }

    pub fn len(&self) -> usize {
        self.count
    }

    /// Bytes held in memory (cache weight).
    pub fn weight(&self) -> usize {
        self.data.len()
    }

    /// Entry `i` (`i < len()`); `None` only if the block was not validated.
    pub fn entry(&self, i: usize) -> Option<EntryPos> {
        if i >= self.count {
            return None;
        }
        decode_at(self.entries(), self.offset(i))
    }

    pub fn key(&self, e: &EntryPos) -> &[u8] {
        &self.data[e.key.clone()]
    }

    pub fn value(&self, e: &EntryPos) -> Option<&[u8]> {
        e.value.clone().map(|r| &self.data[r])
    }

    /// Index of the first entry whose key is `>= key` (`len()` if none).
    pub fn lower_bound(&self, key: &[u8]) -> Option<usize> {
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let e = self.entry(mid)?;
            if self.key(&e) < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Some(lo)
    }

    pub fn last_key(&self) -> Option<&[u8]> {
        let e = self.entry(self.count - 1)?;
        Some(&self.data[e.key])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Entry<'a> = (&'a [u8], u64, Option<&'a [u8]>);

    fn build(entries: &[Entry<'_>]) -> Vec<u8> {
        let mut b = BlockBuilder::default();
        for (k, s, v) in entries {
            b.add(k, *s, *v);
        }
        b.finish()
    }

    #[test]
    fn roundtrip_and_search() {
        let raw = build(&[(b"a", 1, Some(b"x")), (b"b", 7, None), (b"d", 3, Some(b""))]);
        let block = Block::parse(raw).unwrap();
        assert_eq!(block.len(), 3);
        let e = block.entry(1).unwrap();
        assert_eq!(block.key(&e), b"b");
        assert_eq!(e.seq, 7);
        assert_eq!(block.value(&e), None);
        let e = block.entry(2).unwrap();
        assert_eq!(block.value(&e), Some(&b""[..]));
        assert_eq!(block.lower_bound(b"").unwrap(), 0);
        assert_eq!(block.lower_bound(b"b").unwrap(), 1);
        assert_eq!(block.lower_bound(b"c").unwrap(), 2);
        assert_eq!(block.lower_bound(b"e").unwrap(), 3);
        assert_eq!(block.last_key(), Some(&b"d"[..]));
    }

    #[test]
    fn every_bit_flip_is_detected() {
        let raw = build(&[(b"key-1", 1, Some(b"value-1")), (b"key-2", 2, None)]);
        for byte in 0..raw.len() {
            for bit in 0..8 {
                let mut bad = raw.clone();
                bad[byte] ^= 1 << bit;
                assert!(Block::parse(bad).is_err(), "flip at {byte}/{bit} not detected");
            }
        }
        for len in 0..raw.len() {
            assert!(Block::parse(raw[..len].to_vec()).is_err());
        }
    }

    #[test]
    fn structural_checks_behind_a_valid_checksum() {
        fn with_crc(mut payload: Vec<u8>) -> Vec<u8> {
            let c = crc32c::crc32c(&payload);
            put_u32(&mut payload, c);
            payload
        }
        // Zero entries.
        assert!(Block::parse(with_crc(vec![0, 0, 0, 0])).is_err());
        // Keys out of order.
        assert_eq!(Block::parse(build(&[(b"b", 1, None), (b"a", 2, None)])).err(), Some("keys out of order"));
        // Count larger than the block.
        assert!(Block::parse(with_crc(vec![1, 0, 0, 0, 9, 0, 0, 0])).is_err());
    }
}
