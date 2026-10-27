//! Page layouts (see `DESIGN.md`): node pages (leaf and branch), overflow
//! pages and free-list log pages. Meta pages live in [`crate::meta`].
//!
//! Nothing in this module verifies checksums: a page's crc32c is stored by
//! whoever points at it (the parent branch, the meta page, the previous
//! free-list page or the leaf entry owning an overflow chain) and the reader
//! checks it before decoding. Decoding still validates every length, offset
//! and page id, so a damaged page that slipped through can never cause a
//! panic, an out-of-bounds read or an unbounded walk.

use spike_core::{Error, MAX_KEY_LEN, MAX_VALUE_LEN, Result};

pub(crate) const PAGE_SIZE: usize = 4096;
pub(crate) type PageId = u64;
/// Pages 0 and 1 are the two meta slots.
pub(crate) const FIRST_DATA_PAGE: PageId = 2;

pub(crate) const PAGE_LEAF: u8 = 1;
pub(crate) const PAGE_BRANCH: u8 = 2;
pub(crate) const PAGE_OVERFLOW: u8 = 3;
pub(crate) const PAGE_FREELIST: u8 = 4;

/// type u8 | level u8 | count u16 | reserved u32 | own page id u64.
pub(crate) const NODE_HEADER: usize = 16;
/// Bytes available for entries in a node page.
pub(crate) const NODE_CAPACITY: usize = PAGE_SIZE - NODE_HEADER;
/// key_len u16 | value kind u8 | value_len u32.
pub(crate) const LEAF_ENTRY_OVERHEAD: usize = 7;
/// Overflow reference stored in a leaf: chain crc u32 | first page u64.
pub(crate) const OVERFLOW_REF_SIZE: usize = 12;
/// A value is stored inline when its whole leaf entry fits in this many
/// bytes, so every leaf holds at least four entries of any size.
pub(crate) const MAX_INLINE_ENTRY: usize = NODE_CAPACITY / 4;
/// Child reference in a branch: page id u64 | page crc u32.
pub(crate) const CHILD_REF_SIZE: usize = 12;
/// key_len u16 before each separator key.
pub(crate) const BRANCH_KEY_OVERHEAD: usize = 2;
/// A node whose encoded size drops below this is merged with a sibling
/// when the merged node fits in one page.
pub(crate) const UNDERFULL: usize = PAGE_SIZE / 3;
/// Upper bound on tree height accepted from disk (a sanity limit; 4 KiB
/// pages with >= 7 children per branch never get close).
pub(crate) const MAX_HEIGHT: u8 = 40;

/// type u8 | reserved [u8; 7] | next page id u64.
pub(crate) const OVERFLOW_HEADER: usize = 16;
pub(crate) const OVERFLOW_CAPACITY: usize = PAGE_SIZE - OVERFLOW_HEADER;

/// type u8 | reserved [u8; 3] | count u32 | own id u64 | next id u64 |
/// next crc u32 | reserved u32.
pub(crate) const FREELIST_HEADER: usize = 32;
pub(crate) const FREELIST_CAPACITY: usize = (PAGE_SIZE - FREELIST_HEADER) / 8;
/// Set in a free-list log entry when the page left the free set
/// (allocated); clear when it joined it (freed).
pub(crate) const FREELIST_ALLOC_BIT: u64 = 1 << 63;

/// A pointer to a page together with the crc32c of its full 4 KiB image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PageRef {
    pub id: PageId,
    pub crc: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Inline(Vec<u8>),
    /// `crc` covers the full images of all pages of the chain, in order.
    Overflow {
        len: u32,
        crc: u32,
        first: PageId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LeafEntry {
    pub key: Vec<u8>,
    pub value: Value,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Body {
    /// Sorted, unique keys; never empty.
    Leaf(Vec<LeafEntry>),
    /// `children.len() == keys.len() + 1`. Child `i` holds the keys in
    /// `[keys[i - 1], keys[i])`.
    Branch { keys: Vec<Vec<u8>>, children: Vec<PageRef> },
}

/// A decoded, immutable node as stored in the page cache.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Node {
    /// 0 for leaves; a branch at level `n` has children at level `n - 1`.
    pub level: u8,
    pub body: Body,
    /// Bytes the node occupies in its page (header included).
    pub encoded_size: usize,
}

pub(crate) fn inline_fits(key_len: usize, value_len: usize) -> bool {
    LEAF_ENTRY_OVERHEAD + key_len + value_len <= MAX_INLINE_ENTRY
}

pub(crate) fn leaf_entry_size(key_len: usize, value: &Value) -> usize {
    LEAF_ENTRY_OVERHEAD
        + key_len
        + match value {
            Value::Inline(v) => v.len(),
            Value::Overflow { .. } => OVERFLOW_REF_SIZE,
        }
}

pub(crate) fn branch_size(keys: &[Vec<u8>], children: usize) -> usize {
    NODE_HEADER + CHILD_REF_SIZE * children + keys.iter().map(|k| BRANCH_KEY_OVERHEAD + k.len()).sum::<usize>()
}

pub(crate) fn check_page_id(id: PageId, page_count: u64, what: &str) -> Result<()> {
    if id < FIRST_DATA_PAGE || id >= page_count {
        return Err(Error::corruption(format!("{what}: page id {id} outside 2..{page_count}")));
    }
    Ok(())
}

impl Node {
    pub(crate) fn new_leaf(entries: Vec<LeafEntry>) -> Node {
        let encoded_size = NODE_HEADER + entries.iter().map(|e| leaf_entry_size(e.key.len(), &e.value)).sum::<usize>();
        Node { level: 0, body: Body::Leaf(entries), encoded_size }
    }

    pub(crate) fn new_branch(level: u8, keys: Vec<Vec<u8>>, children: Vec<PageRef>) -> Node {
        debug_assert!(level > 0 && children.len() == keys.len() + 1);
        let encoded_size = branch_size(&keys, children.len());
        Node { level, body: Body::Branch { keys, children }, encoded_size }
    }

    /// Approximate memory footprint, used as the cache weight.
    pub(crate) fn weight(&self) -> u64 {
        let items = match &self.body {
            Body::Leaf(e) => 2 * e.len(),
            Body::Branch { keys, .. } => keys.len(),
        };
        (self.encoded_size + 32 * items + 96) as u64
    }

    /// Full page image of this node stored at page `id`.
    pub(crate) fn encode(&self, id: PageId) -> Vec<u8> {
        let mut out = Vec::with_capacity(PAGE_SIZE);
        let (kind, count) = match &self.body {
            Body::Leaf(e) => (PAGE_LEAF, e.len()),
            Body::Branch { keys, .. } => (PAGE_BRANCH, keys.len()),
        };
        out.push(kind);
        out.push(self.level);
        out.extend_from_slice(&(count as u16).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&id.to_le_bytes());
        match &self.body {
            Body::Leaf(entries) => {
                for e in entries {
                    out.extend_from_slice(&(e.key.len() as u16).to_le_bytes());
                    match &e.value {
                        Value::Inline(v) => {
                            out.push(0);
                            out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                            out.extend_from_slice(&e.key);
                            out.extend_from_slice(v);
                        }
                        Value::Overflow { len, crc, first } => {
                            out.push(1);
                            out.extend_from_slice(&len.to_le_bytes());
                            out.extend_from_slice(&e.key);
                            out.extend_from_slice(&crc.to_le_bytes());
                            out.extend_from_slice(&first.to_le_bytes());
                        }
                    }
                }
            }
            Body::Branch { keys, children } => {
                put_child(&mut out, &children[0]);
                for (k, c) in keys.iter().zip(&children[1..]) {
                    out.extend_from_slice(&(k.len() as u16).to_le_bytes());
                    out.extend_from_slice(k);
                    put_child(&mut out, c);
                }
            }
        }
        assert!(
            out.len() == self.encoded_size && out.len() <= PAGE_SIZE,
            "node encoding of {} bytes (expected {}) does not fit a page",
            out.len(),
            self.encoded_size
        );
        out.resize(PAGE_SIZE, 0);
        out
    }

    /// Decodes the page image `buf` read from page `id`. `page_count` bounds
    /// every page id the node refers to.
    pub(crate) fn decode(buf: &[u8], id: PageId, page_count: u64) -> Result<Node> {
        let bad = |what: &str| Error::corruption(format!("page {id}: {what}"));
        if buf.len() != PAGE_SIZE {
            return Err(bad("short page"));
        }
        let mut c = Cursor { buf, pos: 0, id };
        let kind = c.u8()?;
        let level = c.u8()?;
        let count = c.u16()? as usize;
        if c.u32()? != 0 {
            return Err(bad("reserved header bytes are not zero"));
        }
        if c.u64()? != id {
            return Err(bad("page holds another page's id (misdirected write?)"));
        }
        let body = match kind {
            PAGE_LEAF => {
                if level != 0 {
                    return Err(bad("leaf with non-zero level"));
                }
                if count == 0 {
                    return Err(bad("empty leaf"));
                }
                let mut entries: Vec<LeafEntry> = Vec::with_capacity(count);
                for _ in 0..count {
                    let klen = c.u16()? as usize;
                    if klen > MAX_KEY_LEN {
                        return Err(bad("key too long"));
                    }
                    let vkind = c.u8()?;
                    let vlen = c.u32()? as usize;
                    let key = c.take(klen)?.to_vec();
                    let value = match vkind {
                        0 => Value::Inline(c.take(vlen)?.to_vec()),
                        1 => {
                            let crc = c.u32()?;
                            let first = c.u64()?;
                            if vlen == 0 || vlen > MAX_VALUE_LEN {
                                return Err(bad("invalid overflow value length"));
                            }
                            check_page_id(first, page_count, &format!("page {id}: overflow pointer"))?;
                            Value::Overflow { len: vlen as u32, crc, first }
                        }
                        _ => return Err(bad("unknown value kind")),
                    };
                    if entries.last().is_some_and(|p| p.key >= key) {
                        return Err(bad("leaf keys out of order"));
                    }
                    entries.push(LeafEntry { key, value });
                }
                Body::Leaf(entries)
            }
            PAGE_BRANCH => {
                if level == 0 || level >= MAX_HEIGHT {
                    return Err(bad("invalid branch level"));
                }
                let mut keys: Vec<Vec<u8>> = Vec::with_capacity(count);
                let mut children = Vec::with_capacity(count + 1);
                children.push(c.child(page_count)?);
                for _ in 0..count {
                    let klen = c.u16()? as usize;
                    if klen > MAX_KEY_LEN {
                        return Err(bad("separator too long"));
                    }
                    let key = c.take(klen)?.to_vec();
                    if keys.last().is_some_and(|p| *p >= key) {
                        return Err(bad("separators out of order"));
                    }
                    keys.push(key);
                    children.push(c.child(page_count)?);
                }
                Body::Branch { keys, children }
            }
            _ => return Err(bad("not a tree page")),
        };
        Ok(Node { level, body, encoded_size: c.pos })
    }
}

fn put_child(out: &mut Vec<u8>, c: &PageRef) {
    out.extend_from_slice(&c.id.to_le_bytes());
    out.extend_from_slice(&c.crc.to_le_bytes());
}

/// Bounds-checked little-endian reader over one page.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
    id: PageId,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or_else(|| Error::corruption(format!("page {}: entry runs past the end of the page", self.id)))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn child(&mut self, page_count: u64) -> Result<PageRef> {
        let id = self.u64()?;
        let crc = self.u32()?;
        check_page_id(id, page_count, &format!("page {}: child pointer", self.id))?;
        Ok(PageRef { id, crc })
    }
}

/// Page image of one overflow page.
pub(crate) fn encode_overflow_page(next: PageId, chunk: &[u8]) -> Vec<u8> {
    debug_assert!(chunk.len() <= OVERFLOW_CAPACITY);
    let mut page = vec![0u8; PAGE_SIZE];
    page[0] = PAGE_OVERFLOW;
    page[8..16].copy_from_slice(&next.to_le_bytes());
    page[OVERFLOW_HEADER..OVERFLOW_HEADER + chunk.len()].copy_from_slice(chunk);
    page
}

/// Validates the header of an overflow page and returns its `next` pointer
/// (0 at the end of the chain).
pub(crate) fn overflow_page_next(buf: &[u8], id: PageId) -> Result<PageId> {
    if buf[0] != PAGE_OVERFLOW || buf[1..8] != [0; 7] {
        return Err(Error::corruption(format!("page {id}: not an overflow page")));
    }
    Ok(u64::from_le_bytes(buf[8..16].try_into().unwrap()))
}

/// Page image of one free-list log page.
pub(crate) fn encode_freelist_page(id: PageId, next: Option<PageRef>, entries: &[u64]) -> Vec<u8> {
    assert!(entries.len() <= FREELIST_CAPACITY);
    let mut page = vec![0u8; PAGE_SIZE];
    page[0] = PAGE_FREELIST;
    page[4..8].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    page[8..16].copy_from_slice(&id.to_le_bytes());
    let next = next.unwrap_or(PageRef { id: 0, crc: 0 });
    page[16..24].copy_from_slice(&next.id.to_le_bytes());
    page[24..28].copy_from_slice(&next.crc.to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        let at = FREELIST_HEADER + 8 * i;
        page[at..at + 8].copy_from_slice(&e.to_le_bytes());
    }
    page
}

/// Decodes a free-list log page: its `next` (older) page and its entries.
pub(crate) fn decode_freelist_page(buf: &[u8], id: PageId) -> Result<(Option<PageRef>, Vec<u64>)> {
    let bad = |what: &str| Error::corruption(format!("free-list page {id}: {what}"));
    if buf.len() != PAGE_SIZE || buf[0] != PAGE_FREELIST || buf[1..4] != [0; 3] || buf[28..32] != [0; 4] {
        return Err(bad("invalid header"));
    }
    let count = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    if count > FREELIST_CAPACITY {
        return Err(bad("entry count too large"));
    }
    if u64::from_le_bytes(buf[8..16].try_into().unwrap()) != id {
        return Err(bad("page holds another page's id (misdirected write?)"));
    }
    let next_id = u64::from_le_bytes(buf[16..24].try_into().unwrap());
    let next_crc = u32::from_le_bytes(buf[24..28].try_into().unwrap());
    let next = if next_id == 0 {
        if next_crc != 0 {
            return Err(bad("crc without a next page"));
        }
        None
    } else {
        Some(PageRef { id: next_id, crc: next_crc })
    };
    let entries = (0..count)
        .map(|i| {
            let at = FREELIST_HEADER + 8 * i;
            u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
        })
        .collect();
    Ok((next, entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(entries: &[(&[u8], Value)]) -> Node {
        Node::new_leaf(entries.iter().map(|(k, v)| LeafEntry { key: k.to_vec(), value: v.clone() }).collect())
    }

    #[test]
    fn leaf_roundtrip_with_inline_overflow_and_empty_items() {
        let n = leaf(&[
            (b"", Value::Inline(b"empty key".to_vec())),
            (b"a", Value::Inline(Vec::new())),
            (b"big", Value::Overflow { len: 123_456, crc: 0xDEAD_BEEF, first: 7 }),
            (&[0xFF; MAX_KEY_LEN], Value::Inline(vec![1, 2, 3])),
        ]);
        let page = n.encode(9);
        assert_eq!(page.len(), PAGE_SIZE);
        let d = Node::decode(&page, 9, 100).unwrap();
        assert_eq!(d, n);
        assert_eq!(d.encoded_size, n.encoded_size);
    }

    #[test]
    fn branch_roundtrip() {
        let children = (0..4).map(|i| PageRef { id: 10 + i, crc: i as u32 * 7 }).collect();
        let n = Node::new_branch(3, vec![b"b".to_vec(), b"c".to_vec(), vec![0xFF; MAX_KEY_LEN]], children);
        let d = Node::decode(&n.encode(5), 5, 20).unwrap();
        assert_eq!(d, n);
        // A single-child branch (no separators) is valid too.
        let n = Node::new_branch(1, Vec::new(), vec![PageRef { id: 3, crc: 1 }]);
        assert_eq!(Node::decode(&n.encode(4), 4, 5).unwrap(), n);
    }

    #[test]
    fn full_page_leaf_fits_exactly() {
        // Four maximal inline entries fill the page to the last byte.
        let v = MAX_INLINE_ENTRY - LEAF_ENTRY_OVERHEAD - 4;
        assert!(inline_fits(4, v) && !inline_fits(4, v + 1));
        let entries: Vec<(Vec<u8>, Value)> =
            (0..4).map(|i| (format!("k{i:03}").into_bytes(), Value::Inline(vec![i as u8; v]))).collect();
        let refs: Vec<(&[u8], Value)> = entries.iter().map(|(k, v)| (k.as_slice(), v.clone())).collect();
        let n = leaf(&refs);
        assert_eq!(n.encoded_size, PAGE_SIZE);
        assert_eq!(Node::decode(&n.encode(2), 2, 3).unwrap(), n);
        // The largest possible overflow entry also leaves room for four.
        const { assert!(4 * (LEAF_ENTRY_OVERHEAD + MAX_KEY_LEN + OVERFLOW_REF_SIZE) <= NODE_CAPACITY) };
        // And branches always hold several maximal separators.
        const { assert!(CHILD_REF_SIZE + 4 * (BRANCH_KEY_OVERHEAD + MAX_KEY_LEN + CHILD_REF_SIZE) <= NODE_CAPACITY) };
    }

    #[test]
    fn decode_rejects_structural_damage() {
        let n = leaf(&[(b"a", Value::Inline(b"1".to_vec())), (b"b", Value::Inline(b"2".to_vec()))]);
        let good = n.encode(3);
        let mut p = good.clone();
        p[0] = 9;
        assert!(Node::decode(&p, 3, 10).is_err(), "unknown page type");
        assert!(Node::decode(&good, 4, 10).is_err(), "wrong page id");
        let mut p = good.clone();
        p[2] = 200; // count far beyond the data: must not read past the page
        assert!(Node::decode(&p, 3, 10).is_err());
        let mut p = good.clone();
        p[16 + 3..16 + 7].copy_from_slice(&u32::MAX.to_le_bytes()); // huge inline length
        assert!(Node::decode(&p, 3, 10).is_err());
        let mut p = good.clone();
        p[16 + 7] = b'z'; // first key "z" > second key "b"
        assert!(Node::decode(&p, 3, 10).is_err(), "unsorted keys");
        let ovf = leaf(&[(b"a", Value::Overflow { len: 5000, crc: 1, first: 50 })]);
        assert!(Node::decode(&ovf.encode(3), 3, 10).is_err(), "overflow pointer beyond page count");
        let br = Node::new_branch(1, vec![b"m".to_vec()], vec![PageRef { id: 2, crc: 0 }, PageRef { id: 1, crc: 0 }]);
        assert!(Node::decode(&br.encode(3), 3, 10).is_err(), "child pointing at a meta page");
    }

    #[test]
    fn freelist_page_roundtrip() {
        let entries: Vec<u64> = (0..FREELIST_CAPACITY as u64).map(|i| i | ((i % 2) << 63)).collect();
        let next = Some(PageRef { id: 77, crc: 0xABCD });
        let p = encode_freelist_page(12, next, &entries);
        assert_eq!(decode_freelist_page(&p, 12).unwrap(), (next, entries));
        let p = encode_freelist_page(12, None, &[]);
        assert_eq!(decode_freelist_page(&p, 12).unwrap(), (None, vec![]));
        assert!(decode_freelist_page(&p, 13).is_err());
    }

    #[test]
    fn overflow_page_header() {
        let p = encode_overflow_page(42, &[7u8; OVERFLOW_CAPACITY]);
        assert_eq!(overflow_page_next(&p, 3).unwrap(), 42);
        assert_eq!(&p[OVERFLOW_HEADER..], &[7u8; OVERFLOW_CAPACITY][..]);
        let mut q = p.clone();
        q[0] = PAGE_LEAF;
        assert!(overflow_page_next(&q, 3).is_err());
    }
}
