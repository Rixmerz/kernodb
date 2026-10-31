//! Data blocks in memory: the unit of caching and binary search inside an SST.
//!
//! The on-disk block (`docs/file-format/v1.md` §7.2–§7.3) is built and
//! validated by [`crate::format`]: `entry* ‖ offsets u32 × count ‖ count u32`,
//! inside the 44-byte encapsulation with the context CRC. A [`Block`] keeps
//! the validated payload and decodes entries on demand, so its accessors can
//! only fail on a bug.

use std::ops::Range;

use crate::format::block::{BLOCK_TRAILER_LEN, KIND_DELETE, KIND_PUT};
use crate::format::coding::Reader;

/// Location of one entry inside a block's buffer.
#[derive(Clone, Debug)]
pub struct EntryPos {
    pub key: Range<usize>,
    pub seq: u64,
    /// `None` for a tombstone.
    pub value: Option<Range<usize>>,
}

fn decode_at(entries: &[u8], off: usize) -> Option<EntryPos> {
    let mut r = Reader::new(entries.get(off..)?, 0, "SST-R1");
    let klen = r.varint().ok()? as usize;
    let seq = r.varint().ok()?;
    let vlen = match r.u8().ok()? {
        KIND_PUT => Some(r.varint().ok()? as usize),
        KIND_DELETE => None,
        _ => return None,
    };
    let key_start = off.checked_add(r.pos())?;
    let key_end = key_start.checked_add(klen)?;
    let value = match vlen {
        Some(n) => Some(key_end..key_end.checked_add(n)?),
        None => None,
    };
    let end = value.as_ref().map_or(key_end, |v| v.end);
    (end <= entries.len()).then_some(EntryPos { key: key_start..key_end, seq, value })
}

/// A verified, immutable data block.
pub struct Block {
    /// Entries, offsets and count (the encapsulation stripped).
    data: Vec<u8>,
    count: usize,
    /// Start of the offsets array == end of the entries.
    offsets_at: usize,
}

impl Block {
    /// Wraps a whole encapsulated data block that
    /// [`read_data_block`](crate::format::sst::read_data_block) accepted
    /// (CRC, structure, key order and sequences already checked).
    pub fn from_validated(mut raw: Vec<u8>) -> Block {
        raw.truncate(raw.len() - BLOCK_TRAILER_LEN);
        let count_at = raw.len() - 4;
        let count = u32::from_le_bytes(raw[count_at..].try_into().expect("four bytes")) as usize;
        Block { offsets_at: count_at - 4 * count, count, data: raw }
    }

    fn entries(&self) -> &[u8] {
        &self.data[..self.offsets_at]
    }

    fn offset(&self, i: usize) -> usize {
        let at = self.offsets_at + 4 * i;
        u32::from_le_bytes(self.data[at..at + 4].try_into().expect("four bytes")) as usize
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::block::{DataBlockBuilder, seal_block};

    #[test]
    fn roundtrip_and_search() {
        let mut b = DataBlockBuilder::new();
        b.add(b"a", 1, Some(b"x"));
        b.add(b"b", 7, None);
        b.add(b"d", 3, Some(b""));
        let block = Block::from_validated(seal_block(&b.finish(), 5, 96));
        assert_eq!(block.len(), 3);
        let e = block.entry(1).unwrap();
        assert_eq!(block.key(&e), b"b");
        assert_eq!(e.seq, 7);
        assert_eq!(block.value(&e), None);
        let e = block.entry(2).unwrap();
        assert_eq!(block.value(&e), Some(&b""[..]));
        let e = block.entry(0).unwrap();
        assert_eq!((block.key(&e), block.value(&e), e.seq), (&b"a"[..], Some(&b"x"[..]), 1));
        assert!(block.entry(3).is_none());
        assert_eq!(block.lower_bound(b"").unwrap(), 0);
        assert_eq!(block.lower_bound(b"b").unwrap(), 1);
        assert_eq!(block.lower_bound(b"c").unwrap(), 2);
        assert_eq!(block.lower_bound(b"e").unwrap(), 3);
        assert_eq!(block.weight(), block.data.len());
    }
}
