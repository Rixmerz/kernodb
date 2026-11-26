//! The index parts of an SST (v1.md §7.6.1, property tag 1024, rules SST-O8, SST-P1, SST-P2).
//!
//! The index block of a table (§7.7) is one block under one CRC: reading one of its entries
//! means reading and checking all of it, and for a large table that is most of what a first
//! lookup costs. The `index_parts` property cuts the index payload into consecutive parts of
//! `per_part` entries and records, for each part, its last key, the offset of its first data
//! block, its length in bytes and a CRC of those bytes. The property lives in the properties
//! block, whose own CRC covers it, so a reader that holds the properties can read one part of
//! the index block, check it against its CRC and trust it as it trusts the index.
//!
//! The index block itself is unchanged, and the tag is in the range a reader that does not
//! know it ignores (§7.6): a table with the property is a valid table for every v1 reader, and
//! a table without it is read through its whole index, as before.

use super::block::BLOCK_TRAILER_LEN;
use super::coding::{Reader, put_bytes, put_u32, put_varint, varint_len};
use super::crc::{block_ctx, crc_ctx};
use super::header::HEADER_LEN;
use super::limits::MAX_KEY_LEN;
use super::props::Props;
use super::sst::{BlockHandle, BlockIndex, Footer, IndexEntry, MIN_DATA_BLOCK_LEN};
use super::{FormatError, Result};

/// The property tag: the first tag of the ignorable range.
pub const TAG_INDEX_PARTS: u64 = 1024;
/// Entries per part this writer uses. A table whose index fits in one part gets no property.
pub const PART_ENTRIES: u64 = 64;
/// The most entries a part may declare.
pub const MAX_PART_ENTRIES: u64 = 1 << 16;

/// One part of the index: entries `[c × per_part, (c + 1) × per_part)` of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexPart {
    /// The last key of the last data block of the part.
    pub last_key: Vec<u8>,
    /// The offset of the first data block of the part.
    pub first_block: u64,
    /// The bytes its entries take in the index payload.
    pub len: u64,
    /// `crc(file_number ‖ offset of the part in the file; its bytes)`.
    pub crc: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexParts {
    pub per_part: u64,
    pub parts: Vec<IndexPart>,
}

impl IndexParts {
    /// `per_part varint ‖ count varint ‖ (last_key bytes ‖ first_block varint ‖ len varint ‖
    /// crc u32) × count`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, self.per_part);
        put_varint(&mut out, self.parts.len() as u64);
        for p in &self.parts {
            put_bytes(&mut out, &p.last_key);
            put_varint(&mut out, p.first_block);
            put_varint(&mut out, p.len);
            put_u32(&mut out, p.crc);
        }
        out
    }

    /// Parses the property value at `base`: framing only (VAR-1..3, and SST-O6 for a value cut
    /// short or with trailing bytes). What it must say is [`check_parts_shape`].
    pub fn decode(value: &[u8], base: u64) -> Result<IndexParts> {
        let mut r = Reader::new(value, base, "SST-O6");
        let per_part = r.varint()?;
        let count = r.varint()?;
        // Never more parts than the value can hold (a part takes at least seven bytes).
        let mut parts = Vec::with_capacity((count as usize).min(value.len() / 7));
        for _ in 0..count {
            let last_key = r.bytes(MAX_KEY_LEN)?.to_vec();
            let first_block = r.varint()?;
            let len = r.varint()?;
            let crc = r.u32()?;
            parts.push(IndexPart { last_key, first_block, len, crc });
        }
        r.finish()?;
        Ok(IndexParts { per_part, parts })
    }

    /// How many entries part `c` holds in a table of `data_blocks` blocks.
    pub fn entries_in(&self, c: usize, data_blocks: u64) -> u64 {
        if c + 1 < self.parts.len() { self.per_part } else { data_blocks - self.per_part * c as u64 }
    }

    /// Where each part starts within the index payload of a table of `data_blocks` blocks (the
    /// payload opens with the entry count), and where the last one ends.
    pub fn starts(&self, data_blocks: u64) -> Vec<u64> {
        let mut at = varint_len(data_blocks) as u64;
        let mut out = Vec::with_capacity(self.parts.len() + 1);
        for p in &self.parts {
            out.push(at);
            at = at.saturating_add(p.len);
        }
        out.push(at);
        out
    }
}

/// The CRC of the bytes of an index part that starts at `offset` of file `file_number`.
pub fn part_crc(file_number: u64, offset: u64, bytes: &[u8]) -> u32 {
    crc_ctx(&block_ctx(file_number, offset), bytes)
}

/// The index payload of `entries` (as [`encode_index`](super::sst::encode_index)) and the byte
/// at which each run of `per_part` entries starts, with the end of the payload last.
pub fn encode_index_with_bounds<B: AsRef<[u8]>>(entries: &[IndexEntry<B>], per_part: u64) -> (Vec<u8>, Vec<usize>) {
    let mut out = Vec::new();
    let mut bounds = Vec::with_capacity(entries.len() / per_part.max(1) as usize + 2);
    put_varint(&mut out, entries.len() as u64);
    for (i, e) in entries.iter().enumerate() {
        if (i as u64).is_multiple_of(per_part) {
            bounds.push(out.len());
        }
        put_bytes(&mut out, e.last_key.as_ref());
        put_varint(&mut out, e.handle.offset);
        put_varint(&mut out, e.handle.len);
    }
    bounds.push(out.len());
    (out, bounds)
}

/// The parts of an index whose payload (`payload`, cut at `bounds` by
/// [`encode_index_with_bounds`]) is stored in a block at `index_offset`.
pub fn build_index_parts<B: AsRef<[u8]>>(
    entries: &[IndexEntry<B>],
    payload: &[u8],
    bounds: &[usize],
    per_part: u64,
    file_number: u64,
    index_offset: u64,
) -> IndexParts {
    let parts = bounds
        .windows(2)
        .enumerate()
        .map(|(c, w)| {
            let first = c * per_part as usize;
            let last = (first + per_part as usize).min(entries.len()) - 1;
            IndexPart {
                last_key: entries[last].last_key.as_ref().to_vec(),
                first_block: entries[first].handle.offset,
                len: (w[1] - w[0]) as u64,
                crc: part_crc(file_number, index_offset + w[0] as u64, &payload[w[0]..w[1]]),
            }
        })
        .collect();
    IndexParts { per_part, parts }
}

/// SST-O8, the part that needs no index: `per_part` in `1..=65536`; as many parts as
/// `data_blocks` asks for, and at least one; last keys strictly increasing; first blocks
/// strictly increasing from 96 and below `D`; every part long enough for its entries; and the
/// parts cover the index payload exactly. Errors carry `at`, the offset of the properties block.
pub fn check_parts_shape(parts: &IndexParts, props: &Props, footer: &Footer, data_end: u64, at: u64) -> Result<()> {
    let o8 = |detail| Err(FormatError::corruption("SST-O8", at, detail));
    if parts.per_part == 0 || parts.per_part > MAX_PART_ENTRIES {
        return o8("index_parts: entries per part out of range");
    }
    let n = parts.parts.len() as u64;
    if n == 0 || props.data_blocks.div_ceil(parts.per_part) != n {
        return o8("index_parts: part count differs from data_blocks");
    }
    let mut total = varint_len(props.data_blocks) as u64;
    for (c, p) in parts.parts.iter().enumerate() {
        let entries = parts.entries_in(c, props.data_blocks);
        // An entry is at least a key length, an offset and a length.
        if p.len < 3 * entries {
            return o8("index_parts: part shorter than its entries");
        }
        total = match total.checked_add(p.len) {
            Some(t) => t,
            None => return o8("index_parts: lengths overflow"),
        };
        let first_ok = match c.checked_sub(1) {
            None => p.first_block == HEADER_LEN as u64,
            Some(prev) => {
                let q = &parts.parts[prev];
                p.first_block > q.first_block && p.last_key > q.last_key
            }
        };
        if !first_ok || p.first_block >= data_end {
            return o8("index_parts: parts out of order");
        }
    }
    if footer.index.len < BLOCK_TRAILER_LEN as u64 || total != footer.index.len - BLOCK_TRAILER_LEN as u64 {
        return o8("index_parts: parts do not cover the index payload");
    }
    Ok(())
}

/// SST-O8, the part that needs the bytes of the index payload: every part's CRC. `index_offset`
/// is the offset of the index block. The shape must have passed [`check_parts_shape`].
pub fn check_parts_crcs(parts: &IndexParts, props: &Props, payload: &[u8], file_number: u64, at: u64) -> Result<()> {
    let starts = parts.starts(props.data_blocks);
    for (c, p) in parts.parts.iter().enumerate() {
        let (from, to) = (starts[c] as usize, starts[c + 1] as usize);
        let ok = payload.get(from..to).is_some_and(|bytes| part_crc(file_number, at + from as u64, bytes) == p.crc);
        if !ok {
            return Err(FormatError::corruption("SST-O8", at + from as u64, "index_parts: CRC of a part differs"));
        }
    }
    Ok(())
}

/// SST-O8, the part that needs the decoded index: each part ends at the entry it names (its
/// last key), starts at the block it names, and its length is that of its entries. Errors carry
/// `at`, the offset of the index block.
pub fn check_parts_entries<I: BlockIndex + ?Sized>(parts: &IndexParts, index: &I, at: u64) -> Result<()> {
    let o8 = |detail| Err(FormatError::corruption("SST-O8", at, detail));
    let blocks = index.blocks() as u64;
    for (c, p) in parts.parts.iter().enumerate() {
        let first = c as u64 * parts.per_part;
        let last = first + parts.entries_in(c, blocks) - 1;
        if last >= blocks {
            return o8("index_parts: part beyond the index");
        }
        if index.last_key(last as usize) != p.last_key.as_slice()
            || index.handle(first as usize).offset != p.first_block
        {
            return o8("index_parts: part differs from the index");
        }
        let len: u64 = (first..=last)
            .map(|i| {
                let (k, h) = (index.last_key(i as usize), index.handle(i as usize));
                (varint_len(k.len() as u64) + k.len() + varint_len(h.offset) + varint_len(h.len)) as u64
            })
            .sum();
        if len != p.len {
            return o8("index_parts: part length differs from its entries");
        }
    }
    Ok(())
}

/// What a reader checks of part `c` once it has read its bytes at `offset` (the offset of the
/// part in the file): SST-P1, the CRC; SST-P2, the entries are exactly the part's, parse to the
/// last byte, and say what the property says and what SST-O5 asks of an index within the part
/// (contiguous blocks from `first_block` to the next part's first block, or to `D`; lengths in
/// range; last keys strictly increasing, above the previous part's last key and ending at this
/// part's).
pub fn check_part<I: BlockIndex + ?Sized>(
    parts: &IndexParts,
    c: usize,
    entries: &I,
    data_blocks: u64,
    data_end: u64,
    offset: u64,
) -> Result<()> {
    let p2 = |detail| Err(FormatError::corruption("SST-P2", offset, detail));
    let p = &parts.parts[c];
    if entries.blocks() as u64 != parts.entries_in(c, data_blocks) {
        return p2("index part with another number of entries");
    }
    let mut next = p.first_block;
    for i in 0..entries.blocks() {
        let h: BlockHandle = entries.handle(i);
        if h.offset != next {
            return p2("index part entries are not contiguous");
        }
        if h.len < MIN_DATA_BLOCK_LEN || h.len > super::block::MAX_STORED_DATA_PAYLOAD + BLOCK_TRAILER_LEN as u64 {
            return p2("data block length out of range");
        }
        let above = match i.checked_sub(1) {
            Some(prev) => entries.last_key(prev) < entries.last_key(i),
            None => c == 0 || parts.parts[c - 1].last_key.as_slice() < entries.last_key(i),
        };
        if !above {
            return p2("index part keys not strictly increasing");
        }
        next = match h.offset.checked_add(h.len) {
            Some(n) => n,
            None => return p2("index part offsets overflow"),
        };
    }
    let end = parts.parts.get(c + 1).map_or(data_end, |q| q.first_block);
    if next != end {
        return p2("index part does not end where the next begins");
    }
    if entries.last_key(entries.blocks() - 1) != p.last_key.as_slice() {
        return p2("index part does not end at its last key");
    }
    Ok(())
}

/// The last keys of the parts as an index of their own: what a check that samples index keys
/// (SST-V3 at the load of a filter) walks when the index itself is not resident.
pub struct PartKeys<'a>(pub &'a IndexParts);

impl BlockIndex for PartKeys<'_> {
    fn blocks(&self) -> usize {
        self.0.parts.len()
    }

    fn last_key(&self, i: usize) -> &[u8] {
        &self.0.parts[i].last_key
    }

    fn handle(&self, _i: usize) -> BlockHandle {
        BlockHandle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<IndexEntry> {
        let mut offset = HEADER_LEN as u64;
        (0..n)
            .map(|i| {
                let len = 60 + (i as u64 * 37) % 4000;
                let e =
                    IndexEntry { last_key: format!("key-{i:07}").into_bytes(), handle: BlockHandle { offset, len } };
                offset += len;
                e
            })
            .collect()
    }

    fn built(n: usize, per: u64) -> (Vec<IndexEntry>, Vec<u8>, IndexParts, Props, Footer, u64) {
        let es = entries(n);
        let (payload, bounds) = encode_index_with_bounds(&es, per);
        assert_eq!(payload, super::super::sst::encode_index(&es), "the same payload as the plain encoder");
        let index_offset = 1 << 20;
        let parts = build_index_parts(&es, &payload, &bounds, per, 9, index_offset);
        let props = Props { data_blocks: n as u64, ..Props::default() };
        let data_end = es.last().map_or(96, |e| e.handle.offset + e.handle.len);
        let footer = Footer {
            index: BlockHandle { offset: index_offset, len: (payload.len() + BLOCK_TRAILER_LEN) as u64 },
            ..Footer::default()
        };
        (es, payload, parts, props, footer, data_end)
    }

    #[test]
    fn parts_roundtrip_and_hold_for_what_was_built() {
        for (n, per) in [(1, 64), (64, 64), (65, 64), (1000, 64), (1000, 7), (129, 1)] {
            let (es, payload, parts, props, footer, data_end) = built(n, per);
            assert_eq!(IndexParts::decode(&parts.encode(), 0).unwrap(), parts);
            assert_eq!(parts.parts.len() as u64, (n as u64).div_ceil(per));
            check_parts_shape(&parts, &props, &footer, data_end, 0).unwrap();
            check_parts_crcs(&parts, &props, &payload, 9, footer.index.offset).unwrap();
            check_parts_entries(&parts, &es, footer.index.offset).unwrap();
            let starts = parts.starts(props.data_blocks);
            for (c, &start) in starts.iter().enumerate().take(parts.parts.len()) {
                let first = c * per as usize;
                let last = (first + per as usize).min(n);
                check_part(&parts, c, &es[first..last], props.data_blocks, data_end, start).unwrap();
            }
        }
    }

    #[test]
    fn a_property_that_lies_is_refused() {
        let (es, payload, parts, props, footer, data_end) = built(300, 64);
        let shape = |p: &IndexParts| check_parts_shape(p, &props, &footer, data_end, 0).map_err(|e| e.rule);
        let mut p = parts.clone();
        p.per_part = 0;
        assert_eq!(shape(&p), Err("SST-O8"));
        let mut p = parts.clone();
        p.parts.pop();
        assert_eq!(shape(&p), Err("SST-O8"));
        let mut p = parts.clone();
        p.parts[1].len += 1;
        assert_eq!(shape(&p), Err("SST-O8"));
        let mut p = parts.clone();
        p.parts[0].first_block = 97;
        assert_eq!(shape(&p), Err("SST-O8"));
        let mut p = parts.clone();
        p.parts[2].last_key = p.parts[1].last_key.clone();
        assert_eq!(shape(&p), Err("SST-O8"));
        // A CRC that is not the part's, and bytes that are not the CRC's.
        let mut p = parts.clone();
        p.parts[3].crc ^= 1;
        assert_eq!(check_parts_crcs(&p, &props, &payload, 9, footer.index.offset).unwrap_err().rule, "SST-O8");
        let mut damaged = payload.clone();
        damaged[40] ^= 0x10;
        assert_eq!(check_parts_crcs(&parts, &props, &damaged, 9, footer.index.offset).unwrap_err().rule, "SST-O8");
        assert_eq!(check_parts_crcs(&parts, &props, &payload, 8, footer.index.offset).unwrap_err().rule, "SST-O8");
        // A part that names another key or block than the index has there.
        let mut p = parts.clone();
        p.parts[1].last_key.push(0);
        assert_eq!(check_parts_entries(&p, &es, 0).unwrap_err().rule, "SST-O8");
        let mut p = parts.clone();
        p.parts[1].first_block += 1;
        assert_eq!(check_parts_entries(&p, &es, 0).unwrap_err().rule, "SST-O8");
        // A part read as another one, or with an entry that is not its own.
        let part = |c: usize| &es[c * 64..((c + 1) * 64).min(300)];
        assert_eq!(check_part(&parts, 1, part(2), 300, data_end, 0).unwrap_err().rule, "SST-P2");
        assert_eq!(check_part(&parts, 4, part(3), 300, data_end, 0).unwrap_err().rule, "SST-P2");
        let mut wrong = part(1).to_vec();
        wrong[5].handle.len += 1;
        assert_eq!(check_part(&parts, 1, &wrong, 300, data_end, 0).unwrap_err().rule, "SST-P2");
        let mut wrong = part(1).to_vec();
        wrong[63].last_key.push(1);
        assert_eq!(check_part(&parts, 1, &wrong, 300, data_end, 0).unwrap_err().rule, "SST-P2");
    }

    #[test]
    fn a_value_cut_short_or_with_more_bytes_does_not_parse() {
        let (_, _, parts, ..) = built(200, 64);
        let v = parts.encode();
        for cut in 0..v.len() {
            assert!(IndexParts::decode(&v[..cut], 0).is_err(), "cut at {cut}");
        }
        let mut more = v.clone();
        more.push(0);
        assert_eq!(IndexParts::decode(&more, 0).unwrap_err().rule, "SST-O6");
    }
}
