//! Sorted string tables (immutable, one version per user key).
//!
//! ```text
//! data block*   see block.rs (~4 KiB each, own crc32c)
//! filter block  bloom bits | k u8 | crc u32
//! index block   smallest_key bytes | count varint | (last_key bytes | offset varint | len varint)* | crc u32
//! footer        filter_off u64 | filter_len u64 | index_off u64 | index_len u64 | entries u64 | magic u64 | crc u32
//! ```
//!
//! Every byte is covered by a checksum. Opening a table verifies footer,
//! index and filter, plus the layout (blocks are contiguous, they end where
//! the filter starts, the footer ends the file) and cross-checks size,
//! entry count and key range against the MANIFEST. Data blocks are verified
//! when read.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use quick_cache::Weighter;
use spike_core::vfs::{File, OpenMode, Vfs};
use spike_core::{Error, Result};

use crate::block::{Block, BlockBuilder, EntryPos};
use crate::bloom::{self, Bloom};
use crate::coding::{Reader, put_bytes, put_u32, put_u64, put_varint, u64_at, verify_crc_trailer};
use crate::iter::InternalIterator;
use crate::manifest::FileMeta;
use crate::stats::{Stats, bump};

pub const FOOTER_LEN: usize = 52;
const MAGIC: u64 = u64::from_le_bytes(*b"KRNOSST1");
/// Output is handed to the file system in chunks of about this size.
const WRITE_CHUNK: usize = 256 << 10;

pub fn sst_name(number: u64) -> String {
    format!("{number:06}.sst")
}

#[derive(Clone)]
pub struct BlockWeighter;

impl Weighter<(u64, u64), Arc<Block>> for BlockWeighter {
    fn weight(&self, _key: &(u64, u64), block: &Arc<Block>) -> u64 {
        block.weight() as u64 + 64
    }
}

/// Block cache keyed by (file number, block offset), weighted by bytes.
pub type BlockCache = quick_cache::sync::Cache<(u64, u64), Arc<Block>, BlockWeighter>;

/// Everything tables of one engine instance share.
pub struct TableCtx {
    pub vfs: Arc<dyn Vfs>,
    pub dir: PathBuf,
    pub cache: BlockCache,
    pub stats: Arc<Stats>,
    pub block_size: usize,
}

impl TableCtx {
    pub fn new(vfs: Arc<dyn Vfs>, dir: PathBuf, cache_bytes: usize, stats: Arc<Stats>, block_size: usize) -> Self {
        let cache = BlockCache::with_weighter((cache_bytes / block_size).max(1), cache_bytes as u64, BlockWeighter);
        TableCtx { vfs, dir, cache, stats, block_size }
    }

    pub fn path(&self, number: u64) -> PathBuf {
        self.dir.join(sst_name(number))
    }
}

#[derive(Clone, Debug)]
pub struct IndexEntry {
    pub last_key: Vec<u8>,
    pub offset: u64,
    /// Including the block's crc trailer.
    pub len: u64,
}

struct Footer {
    filter_off: u64,
    filter_len: u64,
    index_off: u64,
    index_len: u64,
    entries: u64,
}

impl Footer {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(FOOTER_LEN);
        for v in [self.filter_off, self.filter_len, self.index_off, self.index_len, self.entries, MAGIC] {
            put_u64(&mut out, v);
        }
        let crc = crc32c::crc32c(&out);
        put_u32(&mut out, crc);
        out
    }

    fn decode(buf: &[u8]) -> Option<Footer> {
        let payload = verify_crc_trailer(buf)?;
        if payload.len() != FOOTER_LEN - 4 || u64_at(payload, 40)? != MAGIC {
            return None;
        }
        Some(Footer {
            filter_off: u64_at(payload, 0)?,
            filter_len: u64_at(payload, 8)?,
            index_off: u64_at(payload, 16)?,
            index_len: u64_at(payload, 24)?,
            entries: u64_at(payload, 32)?,
        })
    }
}

fn encode_index(smallest: &[u8], index: &[IndexEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    put_bytes(&mut out, smallest);
    put_varint(&mut out, index.len() as u64);
    for e in index {
        put_bytes(&mut out, &e.last_key);
        put_varint(&mut out, e.offset);
        put_varint(&mut out, e.len);
    }
    let crc = crc32c::crc32c(&out);
    put_u32(&mut out, crc);
    out
}

fn decode_index(payload: &[u8]) -> Option<(Vec<u8>, Vec<IndexEntry>)> {
    let mut r = Reader::new(payload);
    let smallest = r.bytes()?.to_vec();
    let n = r.varint_usize()?;
    let mut index = Vec::with_capacity(n.min(payload.len()));
    for _ in 0..n {
        let last_key = r.bytes()?.to_vec();
        let offset = r.varint()?;
        let len = r.varint()?;
        index.push(IndexEntry { last_key, offset, len });
    }
    r.is_empty().then_some((smallest, index))
}

/// Reads `len` bytes at `off` and verifies their crc32c trailer.
/// `Ok(None)` means checksum mismatch.
fn read_checked(file: &dyn File, off: u64, len: u64) -> Result<Option<Vec<u8>>> {
    let mut buf = vec![0u8; len as usize];
    file.read_at(&mut buf, off)?;
    Ok(verify_crc_trailer(&buf).map(<[u8]>::to_vec))
}

/// Streams a new table to disk. Keys must be added in strictly increasing
/// order. `finish` syncs the file; the caller makes its directory entry
/// durable (`sync_dir`) before referencing it from the MANIFEST.
pub struct TableBuilder {
    ctx: Arc<TableCtx>,
    number: u64,
    file: Arc<dyn File>,
    written: u64,
    pending: Vec<u8>,
    block: BlockBuilder,
    index: Vec<IndexEntry>,
    hashes: Vec<u64>,
    smallest: Option<Vec<u8>>,
    last_key: Vec<u8>,
    entries: u64,
}

impl TableBuilder {
    pub fn create(ctx: &Arc<TableCtx>, number: u64) -> Result<Self> {
        let file = ctx.vfs.open(&ctx.path(number), OpenMode::CreateNew)?;
        Ok(TableBuilder {
            ctx: ctx.clone(),
            number,
            file,
            written: 0,
            pending: Vec::new(),
            block: BlockBuilder::default(),
            index: Vec::new(),
            hashes: Vec::new(),
            smallest: None,
            last_key: Vec::new(),
            entries: 0,
        })
    }

    pub fn add(&mut self, key: &[u8], seq: u64, value: Option<&[u8]>) -> Result<()> {
        debug_assert!(self.entries == 0 || key > self.last_key.as_slice(), "keys must be strictly increasing");
        if self.smallest.is_none() {
            self.smallest = Some(key.to_vec());
        }
        self.hashes.push(bloom::hash(key));
        self.block.add(key, seq, value);
        self.last_key.clear();
        self.last_key.extend_from_slice(key);
        self.entries += 1;
        if self.block.size_estimate() >= self.ctx.block_size {
            self.finish_block()?;
        }
        Ok(())
    }

    /// Bytes the table would have if finished now (excluding filter/index).
    pub fn estimated_size(&self) -> u64 {
        self.written + self.pending.len() as u64 + self.block.size_estimate() as u64
    }

    fn finish_block(&mut self) -> Result<()> {
        let raw = self.block.finish();
        let offset = self.written + self.pending.len() as u64;
        self.index.push(IndexEntry { last_key: self.last_key.clone(), offset, len: raw.len() as u64 });
        self.pending.extend_from_slice(&raw);
        if self.pending.len() >= WRITE_CHUNK {
            self.write_pending()?;
        }
        Ok(())
    }

    fn write_pending(&mut self) -> Result<()> {
        if !self.pending.is_empty() {
            self.file.write_at(&self.pending, self.written)?;
            self.written += self.pending.len() as u64;
            self.pending.clear();
        }
        Ok(())
    }

    /// Writes filter, index and footer, syncs the file and returns the
    /// opened table. Must not be called on an empty builder.
    pub fn finish(mut self) -> Result<Table> {
        if self.entries == 0 {
            return Err(Error::Io(io::Error::other("refusing to write an empty SST")));
        }
        if !self.block.is_empty() {
            self.finish_block()?;
        }
        let filter = bloom::build(&self.hashes);
        let filter_off = self.written + self.pending.len() as u64;
        self.pending.extend_from_slice(&filter);
        let crc = crc32c::crc32c(&filter);
        put_u32(&mut self.pending, crc);
        let index_off = self.written + self.pending.len() as u64;
        let smallest = self.smallest.take().unwrap_or_default();
        self.pending.extend_from_slice(&encode_index(&smallest, &self.index));
        let footer_off = self.written + self.pending.len() as u64;
        let footer = Footer {
            filter_off,
            filter_len: index_off - filter_off,
            index_off,
            index_len: footer_off - index_off,
            entries: self.entries,
        };
        self.pending.extend_from_slice(&footer.encode());
        self.write_pending()?;
        self.file.sync_data()?;
        let largest = std::mem::take(&mut self.last_key);
        Ok(Table {
            number: self.number,
            size: self.written,
            entries: self.entries,
            smallest,
            largest,
            file: self.file.clone(),
            index: std::mem::take(&mut self.index),
            bloom: Bloom::parse(filter).expect("freshly built filter"),
            ctx: self.ctx.clone(),
            obsolete: AtomicBool::new(false),
        })
    }

    /// Gives up on the table and removes its file (best effort).
    pub fn abandon(self) {
        let _ = self.ctx.vfs.remove_file(&self.ctx.path(self.number));
    }
}

/// An open, verified SST. Shared through `Arc` by every `Version` that
/// references it; the file is deleted when the last reference goes away
/// after the table was marked obsolete (i.e. after a MANIFEST that no
/// longer lists it became durable).
pub struct Table {
    pub number: u64,
    pub size: u64,
    pub entries: u64,
    pub smallest: Vec<u8>,
    pub largest: Vec<u8>,
    file: Arc<dyn File>,
    index: Vec<IndexEntry>,
    bloom: Bloom,
    ctx: Arc<TableCtx>,
    obsolete: AtomicBool,
}

impl Table {
    pub fn open(ctx: &Arc<TableCtx>, meta: &FileMeta) -> Result<Table> {
        let name = sst_name(meta.number);
        let corrupt = |m: &str| Error::Corruption(format!("{name}: {m}"));
        let file = match ctx.vfs.open(&ctx.path(meta.number), OpenMode::Existing) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(corrupt("referenced by the MANIFEST but missing"));
            }
            Err(e) => return Err(e.into()),
        };
        let size = file.len()?;
        if size != meta.size {
            return Err(corrupt(&format!("file is {size} bytes, MANIFEST says {}", meta.size)));
        }
        let footer_off = size.checked_sub(FOOTER_LEN as u64).ok_or_else(|| corrupt("shorter than its footer"))?;
        let mut buf = [0u8; FOOTER_LEN];
        file.read_at(&mut buf, footer_off)?;
        let f = Footer::decode(&buf).ok_or_else(|| corrupt("bad footer"))?;
        if f.index_off.checked_add(f.index_len) != Some(footer_off)
            || f.filter_off.checked_add(f.filter_len) != Some(f.index_off)
        {
            return Err(corrupt("inconsistent layout"));
        }
        let filter = read_checked(&*file, f.filter_off, f.filter_len)?.ok_or_else(|| corrupt("filter checksum"))?;
        let bloom = Bloom::parse(filter).ok_or_else(|| corrupt("malformed filter"))?;
        let index = read_checked(&*file, f.index_off, f.index_len)?.ok_or_else(|| corrupt("index checksum"))?;
        let (smallest, index) = decode_index(&index).ok_or_else(|| corrupt("malformed index"))?;
        // Blocks must tile [0, filter_off) exactly, with increasing keys.
        let mut expected = 0u64;
        for (i, e) in index.iter().enumerate() {
            if e.offset != expected || e.len < 8 || (i > 0 && index[i - 1].last_key >= e.last_key) {
                return Err(corrupt("malformed index entry"));
            }
            expected = e.offset + e.len;
        }
        let largest = index.last().map(|e| e.last_key.clone()).ok_or_else(|| corrupt("empty index"))?;
        if expected != f.filter_off || smallest > index[0].last_key {
            return Err(corrupt("malformed index"));
        }
        if f.entries != meta.entries || smallest != meta.smallest || largest != meta.largest {
            return Err(corrupt("entry count or key range differs from the MANIFEST"));
        }
        Ok(Table {
            number: meta.number,
            size,
            entries: f.entries,
            smallest,
            largest,
            file,
            index,
            bloom,
            ctx: ctx.clone(),
            obsolete: AtomicBool::new(false),
        })
    }

    pub fn meta(&self) -> FileMeta {
        FileMeta {
            number: self.number,
            size: self.size,
            entries: self.entries,
            smallest: self.smallest.clone(),
            largest: self.largest.clone(),
        }
    }

    pub fn mark_obsolete(&self) {
        self.obsolete.store(true, Ordering::Release);
    }

    /// Whether the table's key range intersects `[lo, hi]`.
    pub fn overlaps(&self, lo: &[u8], hi: &[u8]) -> bool {
        self.largest.as_slice() >= lo && self.smallest.as_slice() <= hi
    }

    fn corrupt(&self, msg: String) -> Error {
        Error::Corruption(format!("{}: {msg}", sst_name(self.number)))
    }

    /// Reads (through the cache unless `fill_cache` is false and the block
    /// is not cached) and verifies data block `i`.
    pub fn read_block(&self, i: usize, fill_cache: bool) -> Result<Arc<Block>> {
        let e = &self.index[i];
        let key = (self.number, e.offset);
        let stats = &self.ctx.stats;
        if let Some(b) = self.ctx.cache.get(&key) {
            if fill_cache {
                bump(&stats.block_cache_hits, 1);
            }
            return Ok(b);
        }
        if fill_cache {
            bump(&stats.block_cache_misses, 1);
        }
        let mut buf = vec![0u8; e.len as usize];
        self.file.read_at(&mut buf, e.offset)?;
        let block = Block::parse(buf).map_err(|m| self.corrupt(format!("block at offset {}: {m}", e.offset)))?;
        if block.last_key() != Some(e.last_key.as_slice()) {
            return Err(self.corrupt(format!("block at offset {} does not match the index", e.offset)));
        }
        let block = Arc::new(block);
        if fill_cache {
            self.ctx.cache.insert(key, block.clone());
        }
        Ok(block)
    }

    /// `Some(Some(v))` value, `Some(None)` tombstone, `None` not in this table.
    pub fn get(&self, key: &[u8], hash: u64) -> Result<Option<Option<Vec<u8>>>> {
        if key < self.smallest.as_slice() || key > self.largest.as_slice() {
            return Ok(None);
        }
        let stats = &self.ctx.stats;
        bump(&stats.bloom_checks, 1);
        if !self.bloom.may_contain(hash) {
            bump(&stats.bloom_negatives, 1);
            return Ok(None);
        }
        let i = self.index.partition_point(|e| e.last_key.as_slice() < key);
        if i == self.index.len() {
            return Ok(None);
        }
        let block = self.read_block(i, true)?;
        let pos = block.lower_bound(key).ok_or_else(|| self.corrupt("unreadable block".into()))?;
        if let Some(e) = block.entry(pos)
            && block.key(&e) == key
        {
            return Ok(Some(block.value(&e).map(<[u8]>::to_vec)));
        }
        bump(&stats.bloom_false_positives, 1);
        Ok(None)
    }

    pub fn iter(&self, fill_cache: bool) -> TableIter<'_> {
        TableIter { table: self, fill_cache, block_idx: 0, pos: None }
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        if self.obsolete.load(Ordering::Acquire) && self.ctx.vfs.remove_file(&self.ctx.path(self.number)).is_ok() {
            bump(&self.ctx.stats.obsolete_files_deleted, 1);
        }
    }
}

pub struct TableIter<'a> {
    table: &'a Table,
    fill_cache: bool,
    block_idx: usize,
    /// Current block and entry (index within the block, decoded position).
    pos: Option<(Arc<Block>, usize, EntryPos)>,
}

impl TableIter<'_> {
    /// Positions at entry `entry` of block `block_idx`, moving to later
    /// blocks if needed.
    fn load(&mut self, mut block_idx: usize, mut entry: usize) -> Result<()> {
        self.pos = None;
        while block_idx < self.table.index.len() {
            let block = self.table.read_block(block_idx, self.fill_cache)?;
            if entry < block.len() {
                let e = block.entry(entry).ok_or_else(|| self.table.corrupt("unreadable block".into()))?;
                self.block_idx = block_idx;
                self.pos = Some((block, entry, e));
                return Ok(());
            }
            block_idx += 1;
            entry = 0;
        }
        self.block_idx = block_idx;
        Ok(())
    }

    fn cur(&self) -> (&Block, &EntryPos) {
        let (b, _, e) = self.pos.as_ref().expect("iterator is not valid");
        (b, e)
    }
}

impl InternalIterator for TableIter<'_> {
    fn valid(&self) -> bool {
        self.pos.is_some()
    }

    fn seek_to_first(&mut self) -> Result<()> {
        self.load(0, 0)
    }

    fn seek(&mut self, target: &[u8]) -> Result<()> {
        let idx = self.table.index.partition_point(|e| e.last_key.as_slice() < target);
        if idx == self.table.index.len() {
            self.pos = None;
            return Ok(());
        }
        let block = self.table.read_block(idx, self.fill_cache)?;
        let entry = block.lower_bound(target).ok_or_else(|| self.table.corrupt("unreadable block".into()))?;
        self.load(idx, entry)
    }

    fn next(&mut self) -> Result<()> {
        let (block, entry, _) = self.pos.as_ref().expect("iterator is not valid");
        let next = entry + 1;
        if next < block.len() {
            let e = block.entry(next).ok_or_else(|| self.table.corrupt("unreadable block".into()))?;
            let block = block.clone();
            self.pos = Some((block, next, e));
            Ok(())
        } else {
            self.load(self.block_idx + 1, 0)
        }
    }

    fn key(&self) -> &[u8] {
        let (b, e) = self.cur();
        b.key(e)
    }

    fn seq(&self) -> u64 {
        self.cur().1.seq
    }

    fn value(&self) -> Option<&[u8]> {
        let (b, e) = self.cur();
        b.value(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spike_core::vfs::SimVfs;
    use std::path::Path;

    fn ctx(block_size: usize) -> (SimVfs, Arc<TableCtx>) {
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/t")).unwrap();
        let stats = Arc::new(Stats::default());
        (sim, Arc::new(TableCtx::new(vfs, PathBuf::from("/t"), 1 << 20, stats, block_size)))
    }

    fn key(i: usize) -> Vec<u8> {
        format!("key-{i:06}").into_bytes()
    }

    fn value(i: usize) -> Option<Vec<u8>> {
        match i % 7 {
            0 => None,
            1 => Some(vec![]),
            2 => Some(vec![b'L'; 9000 + i]), // larger than a block
            _ => Some(format!("value-{i}").into_bytes()),
        }
    }

    fn build(ctx: &Arc<TableCtx>, number: u64, n: usize) -> Table {
        let mut b = TableBuilder::create(ctx, number).unwrap();
        for i in 0..n {
            b.add(&key(i * 2), i as u64, value(i).as_deref()).unwrap();
        }
        b.finish().unwrap()
    }

    #[test]
    fn build_and_read_back() {
        let (_sim, ctx) = ctx(512);
        let n = 500;
        let built = build(&ctx, 7, n);
        assert!(built.index.len() > 10, "expected many blocks");
        let t = Table::open(&ctx, &built.meta()).unwrap();
        assert_eq!(t.meta(), built.meta());
        assert_eq!(t.smallest, key(0));
        assert_eq!(t.largest, key((n - 1) * 2));
        for i in 0..n {
            let got = t.get(&key(i * 2), bloom::hash(&key(i * 2))).unwrap();
            assert_eq!(got, Some(value(i)), "key {i}");
            let absent = key(i * 2 + 1);
            assert_eq!(t.get(&absent, bloom::hash(&absent)).unwrap(), None);
        }
        // Full iteration and seeks, including block boundaries.
        let mut it = t.iter(true);
        it.seek_to_first().unwrap();
        for i in 0..n {
            assert!(it.valid());
            assert_eq!((it.key(), it.seq(), it.value()), (key(i * 2).as_slice(), i as u64, value(i).as_deref()));
            it.next().unwrap();
        }
        assert!(!it.valid());
        for i in 0..n {
            it.seek(&key(i * 2 + 1)).unwrap();
            if i + 1 < n {
                assert_eq!(it.key(), key((i + 1) * 2).as_slice());
            } else {
                assert!(!it.valid());
            }
            it.seek(&key(i * 2)).unwrap();
            assert_eq!(it.key(), key(i * 2).as_slice());
        }
        it.seek(b"").unwrap();
        assert_eq!(it.key(), key(0).as_slice());
        it.seek(b"z").unwrap();
        assert!(!it.valid());
    }

    #[test]
    fn single_entry_and_exact_block_boundaries() {
        for block_size in [64, 100, 4096] {
            let (_sim, ctx) = ctx(block_size);
            for n in [1, 2, 3, 17] {
                let built = build(&ctx, 100 + n as u64, n);
                let t = Table::open(&ctx, &built.meta()).unwrap();
                let mut it = t.iter(false);
                it.seek_to_first().unwrap();
                let mut count = 0;
                while it.valid() {
                    count += 1;
                    it.next().unwrap();
                }
                assert_eq!(count, n);
            }
        }
    }

    #[test]
    fn damage_is_detected() {
        let (sim, ctx) = ctx(256);
        let built = build(&ctx, 3, 60);
        let meta = built.meta();
        drop(built);
        let path = ctx.path(3);
        let size = meta.size;
        // Flip one bit at many positions: either open fails or every read
        // of the damaged block fails; never wrong data.
        for off in (0..size).step_by(37) {
            sim.corrupt_byte(&path, off, 0x10);
            match Table::open(&ctx, &meta) {
                Err(Error::Corruption(_)) => {}
                Err(e) => panic!("unexpected error {e}"),
                Ok(t) => {
                    let mut it = t.iter(false);
                    let mut ok = it.seek_to_first().is_ok();
                    let mut i = 0;
                    while ok && it.valid() {
                        assert_eq!(it.key(), key(i * 2).as_slice(), "flip at {off} returned wrong data");
                        assert_eq!(it.value(), value(i).as_deref(), "flip at {off} returned wrong data");
                        i += 1;
                        ok = it.next().is_ok();
                    }
                    assert!(!ok, "flip at {off} went unnoticed");
                }
            }
            sim.corrupt_byte(&path, off, 0x10);
        }
        // Wrong size in the MANIFEST.
        let mut m = meta.clone();
        m.size -= 1;
        assert!(matches!(Table::open(&ctx, &m), Err(Error::Corruption(_))));
        let mut m = meta.clone();
        m.largest = b"other".to_vec();
        assert!(matches!(Table::open(&ctx, &m), Err(Error::Corruption(_))));
    }
}
