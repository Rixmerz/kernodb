//! The two double-buffered meta pages (page ids 0 and 1).
//!
//! A commit writes its meta into the slot that does *not* hold the current
//! meta, so the last durable commit is never touched while a new one is being
//! written. Open picks the valid slot with the highest sequence (and later
//! commits alternate from there, which also works when open had to fall back
//! to the only intact slot). A clean close writes the final meta into both
//! slots.

use spike_core::{Error, Result};

use crate::page::{FIRST_DATA_PAGE, MAX_HEIGHT, PAGE_SIZE, PageRef};

pub(crate) const MAGIC: &[u8; 8] = b"KRNOBTRE";
pub(crate) const FORMAT_VERSION: u32 = 1;
/// The crc32c of bytes `0..META_CRC_AT` is stored at `META_CRC_AT`.
const META_CRC_AT: usize = 88;
pub(crate) const META_LEN: usize = META_CRC_AT + 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Meta {
    pub seq: u64,
    /// Root page and its crc; `None` for an empty tree.
    pub root: Option<PageRef>,
    /// Number of levels (0 for an empty tree, 1 when the root is a leaf).
    pub height: u8,
    /// High-water mark: every page id below it has been written.
    pub page_count: u64,
    /// Newest page of the free-list log.
    pub free_head: Option<PageRef>,
    pub free_log_pages: u64,
    /// Size of the persisted free set (checked after replay).
    pub free_count: u64,
    /// Number of keys (statistics only).
    pub entries: u64,
}

impl Meta {
    pub(crate) fn empty() -> Meta {
        Meta {
            seq: 0,
            root: None,
            height: 0,
            page_count: FIRST_DATA_PAGE,
            free_head: None,
            free_log_pages: 0,
            free_count: 0,
            entries: 0,
        }
    }

    /// Byte offset of meta slot `slot` (0 or 1).
    pub(crate) fn slot_offset(slot: usize) -> u64 {
        debug_assert!(slot < 2);
        (slot * PAGE_SIZE) as u64
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut p = vec![0u8; PAGE_SIZE];
        let root = self.root.unwrap_or(PageRef { id: 0, crc: 0 });
        let free = self.free_head.unwrap_or(PageRef { id: 0, crc: 0 });
        p[0..8].copy_from_slice(MAGIC);
        p[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
        p[12..16].copy_from_slice(&(PAGE_SIZE as u32).to_le_bytes());
        p[16..24].copy_from_slice(&self.seq.to_le_bytes());
        p[24..32].copy_from_slice(&root.id.to_le_bytes());
        p[32..36].copy_from_slice(&root.crc.to_le_bytes());
        p[36..40].copy_from_slice(&u32::from(self.height).to_le_bytes());
        p[40..48].copy_from_slice(&self.page_count.to_le_bytes());
        p[48..56].copy_from_slice(&free.id.to_le_bytes());
        p[56..60].copy_from_slice(&free.crc.to_le_bytes());
        // 60..64 reserved (zero)
        p[64..72].copy_from_slice(&self.free_log_pages.to_le_bytes());
        p[72..80].copy_from_slice(&self.free_count.to_le_bytes());
        p[80..88].copy_from_slice(&self.entries.to_le_bytes());
        let crc = crc32c::crc32c(&p[..META_CRC_AT]);
        p[META_CRC_AT..META_LEN].copy_from_slice(&crc.to_le_bytes());
        p
    }

    /// Decodes and validates one meta slot; `Err` describes why it is
    /// unusable (torn, never written, damaged, foreign file).
    pub(crate) fn decode(p: &[u8]) -> std::result::Result<Meta, String> {
        if p.len() < META_LEN {
            return Err("short meta page".into());
        }
        let u32_at = |at: usize| u32::from_le_bytes(p[at..at + 4].try_into().unwrap());
        let u64_at = |at: usize| u64::from_le_bytes(p[at..at + 8].try_into().unwrap());
        if &p[0..8] != MAGIC {
            return Err("bad magic".into());
        }
        if crc32c::crc32c(&p[..META_CRC_AT]) != u32_at(META_CRC_AT) {
            return Err("checksum mismatch".into());
        }
        if u32_at(8) != FORMAT_VERSION {
            return Err(format!("unsupported format version {}", u32_at(8)));
        }
        if u32_at(12) as usize != PAGE_SIZE {
            return Err(format!("unsupported page size {}", u32_at(12)));
        }
        if u32_at(60) != 0 {
            return Err("reserved bytes are not zero".into());
        }
        let page_count = u64_at(40);
        let opt_ref = |id: u64, crc: u32, what: &str| -> std::result::Result<Option<PageRef>, String> {
            match id {
                0 if crc == 0 => Ok(None),
                0 => Err(format!("{what} crc without a page")),
                id if id < FIRST_DATA_PAGE || id >= page_count => Err(format!("{what} page {id} out of range")),
                id => Ok(Some(PageRef { id, crc })),
            }
        };
        let root = opt_ref(u64_at(24), u32_at(32), "root")?;
        let height = u32_at(36);
        let free_head = opt_ref(u64_at(48), u32_at(56), "free-list")?;
        let free_log_pages = u64_at(64);
        let free_count = u64_at(72);
        if page_count < FIRST_DATA_PAGE {
            return Err("page count too small".into());
        }
        if root.is_some() != (height > 0) || height > u32::from(MAX_HEIGHT) {
            return Err(format!("invalid tree height {height}"));
        }
        if free_head.is_some() != (free_log_pages > 0) || free_log_pages >= page_count || free_count >= page_count {
            return Err("invalid free-list summary".into());
        }
        Ok(Meta {
            seq: u64_at(16),
            root,
            height: height as u8,
            page_count,
            free_head,
            free_log_pages,
            free_count,
            entries: u64_at(80),
        })
    }
}

/// The meta chosen at open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Selected {
    pub meta: Meta,
    /// Slot it was read from; the next commit writes the other one.
    pub slot: usize,
    /// Both slots hold exactly this meta (a clean close happened).
    pub both: bool,
}

/// Picks the valid meta with the highest commit sequence from the two slots.
pub(crate) fn select(slot0: &[u8], slot1: &[u8]) -> Result<Selected> {
    let one = |meta, slot| Selected { meta, slot, both: false };
    match (Meta::decode(slot0), Meta::decode(slot1)) {
        (Ok(a), Ok(b)) if b.seq > a.seq => Ok(one(b, 1)),
        (Ok(a), Ok(b)) => Ok(Selected { meta: a, slot: 0, both: a == b }),
        (Ok(a), Err(_)) => Ok(one(a, 0)),
        (Err(_), Ok(b)) => Ok(one(b, 1)),
        (Err(e0), Err(e1)) => Err(Error::corruption(format!("no valid meta page (slot 0: {e0}; slot 1: {e1})"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: u64) -> Meta {
        Meta {
            seq,
            root: Some(PageRef { id: 17, crc: 0x1234_5678 }),
            height: 3,
            page_count: 40,
            free_head: Some(PageRef { id: 21, crc: 99 }),
            free_log_pages: 2,
            free_count: 5,
            entries: 1000,
        }
    }

    #[test]
    fn roundtrip() {
        for m in [Meta::empty(), sample(7)] {
            assert_eq!(Meta::decode(&m.encode()).unwrap(), m);
        }
    }

    #[test]
    fn every_single_bit_flip_in_the_covered_bytes_is_rejected() {
        let good = sample(7).encode();
        for byte in 0..META_LEN {
            for bit in 0..8 {
                let mut p = good.clone();
                p[byte] ^= 1 << bit;
                assert!(Meta::decode(&p).is_err(), "flip of bit {bit} in byte {byte} accepted");
            }
        }
    }

    #[test]
    fn select_prefers_highest_valid_sequence() {
        let (a, b) = (sample(7).encode(), sample(8).encode());
        assert_eq!(select(&a, &b).unwrap(), Selected { meta: sample(8), slot: 1, both: false });
        assert_eq!(select(&b, &a).unwrap(), Selected { meta: sample(8), slot: 0, both: false });
        assert_eq!(select(&a, &a).unwrap(), Selected { meta: sample(7), slot: 0, both: true });
        // A torn newer slot (only the first sector landed, or garbage)
        // falls back to the older one.
        let mut torn = sample(9).encode();
        torn[META_CRC_AT] ^= 0xFF;
        assert_eq!(select(&torn, &a).unwrap(), Selected { meta: sample(7), slot: 1, both: false });
        let zero = vec![0u8; PAGE_SIZE];
        assert_eq!(select(&a, &zero).unwrap(), Selected { meta: sample(7), slot: 0, both: false });
        assert!(matches!(select(&zero, &torn), Err(Error::Corruption(_))));
    }

    #[test]
    fn inconsistent_fields_are_rejected_even_with_a_valid_crc() {
        let mut m = sample(3);
        m.height = 0; // root without height
        assert!(Meta::decode(&m.encode()).is_err());
        let mut m = sample(3);
        m.root = Some(PageRef { id: 40, crc: 0 }); // beyond page count
        assert!(Meta::decode(&m.encode()).is_err());
        let mut m = sample(3);
        m.free_log_pages = 0; // head without pages
        assert!(Meta::decode(&m.encode()).is_err());
    }
}
