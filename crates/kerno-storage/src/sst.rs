//! Sorted string tables, format v1 (`docs/file-format/v1.md` §7): immutable,
//! one version per user key.
//!
//! ```text
//! header        common header (KRNO_SST, file number, block size)
//! data block*   ~4 KiB each, context CRC over (file number, offset)
//! range block   range tombstone fragments (only if the table has any)
//! filter block  bloom filter
//! props block   TLV properties (entry counts, sequence and key ranges, ...)
//! index block   last key, offset and length of every data block
//! footer        80 bytes: handles of the meta blocks, magic, CRC
//! ```
//!
//! The bytes are produced and checked by [`crate::format::sst`]; this module
//! streams them to a file, keeps the resident part of an open table (index,
//! filter, properties) and reads data blocks through the block cache. Every
//! byte is covered by a checksum, and the CRC of a block depends on the file
//! number and the offset it is read at. Opening a table runs SST-O1..O7
//! against the MANIFEST's `FileMeta`; data blocks are verified (SST-R1..R3)
//! when read.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use kerno_core::vfs::{File, OpenMode, Vfs};
use kerno_core::{Error, Result};
use quick_cache::Weighter;

use crate::block::{Block, EntryPos};
use crate::deleter::Deleter;
use crate::files::{FileSource, format_error, read_error};
use crate::format::ByteSource;
use crate::format::filter::Filter;
use crate::format::limits::{MAX_BLOCK_SIZE, MAX_SEQ, MIN_BLOCK_SIZE};
use crate::format::manifest::FileMeta;
use crate::format::props::Props;
use crate::format::rangedel::Fragment;
use crate::format::sst::{IndexEntry, SstBuilder, SstExpect, file_meta, open_sst_from, read_data_block};
use crate::iter::InternalIterator;
use crate::stats::{Stats, bump};

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
    /// Target size of a data block (written to the SST header; clamped to the
    /// 512 B..1 MiB the format accepts).
    pub block_size: usize,
    /// New tables call `sync_data` every this many bytes written, besides
    /// the final one (`CompactionConfig::bytes_per_sync`).
    pub bytes_per_sync: Option<u64>,
    /// New tables ask the file system to start writing their data back every
    /// this many bytes written (`File::start_writeback`), waiting for that range before going on, and
    /// without making anything durable
    /// (`CompactionConfig::bytes_per_writeback`).
    pub bytes_per_writeback: Option<u64>,
    /// The database id every table of this engine carries (HDR-10).
    pub db_id: [u8; 16],
    /// Where an obsolete table's file is queued when its last reference goes
    /// away (W-13).
    pub deleter: Arc<Deleter>,
}

impl TableCtx {
    pub fn new(vfs: Arc<dyn Vfs>, dir: PathBuf, cache_bytes: usize, stats: Arc<Stats>, block_size: usize) -> Self {
        let cache = BlockCache::with_weighter((cache_bytes / block_size).max(1), cache_bytes as u64, BlockWeighter);
        let deleter = Deleter::inline(vfs.clone(), stats.clone());
        TableCtx {
            vfs,
            dir,
            cache,
            stats,
            block_size,
            bytes_per_sync: None,
            bytes_per_writeback: None,
            db_id: [0; 16],
            deleter,
        }
    }

    pub fn with_deleter(mut self, deleter: Arc<Deleter>) -> Self {
        self.deleter = deleter;
        self
    }

    pub fn with_bytes_per_sync(mut self, bytes_per_sync: Option<u64>) -> Self {
        self.bytes_per_sync = bytes_per_sync;
        self
    }

    pub fn with_bytes_per_writeback(mut self, bytes_per_writeback: Option<u64>) -> Self {
        self.bytes_per_writeback = bytes_per_writeback;
        self
    }

    pub fn with_db_id(mut self, db_id: [u8; 16]) -> Self {
        self.db_id = db_id;
        self
    }

    pub fn path(&self, number: u64) -> PathBuf {
        self.dir.join(sst_name(number))
    }
}

/// Buffers the bytes of a table and writes them in chunks.
struct Sink {
    file: Arc<dyn File>,
    written: u64,
    /// `written` at the last intermediate `sync_data`.
    synced: u64,
    /// `written` at the last intermediate `sync_data` or writeback request.
    hinted: u64,
    pending: Vec<u8>,
    bytes_per_sync: Option<u64>,
    bytes_per_writeback: Option<u64>,
    stats: Arc<Stats>,
}

impl Sink {
    fn push(&mut self, bytes: Vec<u8>) -> Result<()> {
        if self.pending.is_empty() {
            self.pending = bytes;
        } else {
            self.pending.extend_from_slice(&bytes);
        }
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
            if let Some(every) = self.bytes_per_sync
                && self.written - self.synced >= every
            {
                // Bounds the dirty data the final sync has to write back at
                // once (phase1-design.md §5.4, H3).
                self.file.sync_data()?;
                self.synced = self.written;
                self.hinted = self.written;
            } else if let Some(every) = self.bytes_per_writeback
                && self.written - self.hinted >= every
            {
                // The same bound without a journal barrier: the kernel starts
                // writing these bytes back and the call waits for that, so
                // the device queue never holds more than this range of the
                // table (starting writeback without waiting only moves the
                // burst earlier: `spike-bench walsync --writeback true`).
                // Only `start_writeback` is advisory: its error stays
                // recorded for the final `sync_data`. `wait_writeback`
                // consumes the writeback error on Linux, so the final sync
                // would return `Ok` for data that never reached the disk:
                // its error must fail the table, unless the platform lacks
                // the call.
                let (offset, len) = (self.hinted, self.written - self.hinted);
                let _ = self.file.start_writeback(offset, len);
                match self.file.wait_writeback(offset, len) {
                    Ok(()) => {}
                    // ENOSYS / EINVAL: no `sync_file_range` here.
                    Err(e) if matches!(e.kind(), io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput) => {}
                    Err(e) => return Err(e.into()),
                }
                bump(&self.stats.sst_writeback_hints, 1);
                self.hinted = self.written;
            }
        }
        Ok(())
    }
}

/// Streams a new table to disk. Keys must be added in strictly increasing
/// order. `finish` syncs the file; the caller makes its directory entry
/// durable (`sync_dir`) before referencing it from the MANIFEST.
pub struct TableBuilder {
    ctx: Arc<TableCtx>,
    number: u64,
    /// Why the table is written (`creation_reason` property, §7.6).
    reason: u8,
    sink: Sink,
    inner: SstBuilder,
    entries: u64,
}

impl TableBuilder {
    pub fn create(ctx: &Arc<TableCtx>, number: u64, reason: u8) -> Result<Self> {
        let file = ctx.vfs.open(&ctx.path(number), OpenMode::CreateNew)?;
        let block_size = u32::try_from(ctx.block_size).unwrap_or(MAX_BLOCK_SIZE).clamp(MIN_BLOCK_SIZE, MAX_BLOCK_SIZE);
        Ok(TableBuilder {
            ctx: ctx.clone(),
            number,
            reason,
            sink: Sink {
                file,
                written: 0,
                synced: 0,
                hinted: 0,
                pending: Vec::new(),
                bytes_per_sync: ctx.bytes_per_sync,
                bytes_per_writeback: ctx.bytes_per_writeback,
                stats: ctx.stats.clone(),
            },
            inner: SstBuilder::new(ctx.db_id, number, block_size),
            entries: 0,
        })
    }

    /// Syncs the file once, when finished, whatever `bytes_per_sync` and
    /// `bytes_per_writeback` say.
    pub fn without_intermediate_syncs(mut self) -> Self {
        self.sink.bytes_per_sync = None;
        self.sink.bytes_per_writeback = None;
        self
    }

    pub fn add(&mut self, key: &[u8], seq: u64, value: Option<&[u8]>) -> Result<()> {
        self.inner.add(key, seq, value);
        self.entries += 1;
        let ready = self.inner.take_ready();
        if !ready.is_empty() {
            self.sink.push(ready)?;
        }
        Ok(())
    }

    /// Bytes the table would have if finished now (excluding filter, index
    /// and footer).
    pub fn estimated_size(&self) -> u64 {
        self.inner.len() + self.inner.open_block_len() as u64
    }

    /// Writes filter, properties, index and footer, syncs the file and
    /// returns the opened table. Must not be called on an empty builder.
    #[cfg(test)]
    pub fn finish(self) -> Result<Table> {
        self.finish_with(&[])
    }

    /// [`finish`](Self::finish) for a table that also holds the range
    /// tombstone `fragments` (sorted, disjoint, within the keys the table
    /// stands for). A table of fragments only is fine; a table of nothing is
    /// not.
    pub fn finish_with(self, fragments: &[Fragment]) -> Result<Table> {
        let TableBuilder { ctx, number, reason, mut sink, inner, entries } = self;
        if entries == 0 && fragments.is_empty() {
            return Err(Error::Io(io::Error::other("refusing to write an empty SST")));
        }
        let (rest, props, len) = inner.finish(fragments, reason);
        sink.push(rest)?;
        sink.write_pending()?;
        sink.file.sync_data()?;
        debug_assert_eq!(sink.written, len);
        // Opening what was just written also checks it (SST-O1..O7).
        Table::open_file(&ctx, sink.file, &file_meta(0, number, len, &props), MAX_SEQ)
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
    /// The first key the table stands for: its first point or fragment
    /// start. Always inclusive.
    pub smallest: Vec<u8>,
    /// The last one: its last point, or the end of its last fragment if that
    /// is beyond the last point, in which case `largest_exclusive` is set.
    pub largest: Vec<u8>,
    /// `largest` is the end of a range tombstone fragment, not a key the
    /// table may hold.
    pub largest_exclusive: bool,
    file: Arc<dyn File>,
    index: Vec<IndexEntry>,
    props: Props,
    filter: Filter,
    /// The range tombstones of the table, sorted and disjoint, resident like
    /// the index (SST-F1).
    fragments: Vec<Fragment>,
    ctx: Arc<TableCtx>,
    obsolete: AtomicBool,
    /// `ro_compat` bits of the header this engine does not know (HDR-7):
    /// a database with such a file opens read-only.
    pub unknown_ro_compat: u64,
}

impl Table {
    /// Opens the table `meta` describes, against the MANIFEST that has it at
    /// `last_seq` (SST-O1..O7).
    /// Point entries in the table (the `point_entries` property).
    pub fn entries(&self) -> u64 {
        self.props.point_entries
    }

    pub fn open(ctx: &Arc<TableCtx>, meta: &FileMeta, last_seq: u64) -> Result<Table> {
        let file = match ctx.vfs.open(&ctx.path(meta.number), OpenMode::Existing) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::Corruption(format!(
                    "{}: referenced by the MANIFEST but missing",
                    sst_name(meta.number)
                )));
            }
            Err(e) => return Err(e.into()),
        };
        Table::open_file(ctx, file, meta, last_seq)
    }

    fn open_file(ctx: &Arc<TableCtx>, file: Arc<dyn File>, meta: &FileMeta, last_seq: u64) -> Result<Table> {
        let name = sst_name(meta.number);
        let (opened, size) = {
            let mut src = FileSource::new(&*file)?;
            let ex = SstExpect { file_number: meta.number, db_id: ctx.db_id, meta: Some(meta), last_seq };
            let size = src.len();
            (open_sst_from(&mut src, &ex).map_err(|e| read_error(&name, e))?, size)
        };
        Ok(Table {
            number: meta.number,
            size,
            smallest: opened.props.smallest_key.clone(),
            largest: opened.props.largest_key.clone(),
            largest_exclusive: opened.props.largest_exclusive,
            file,
            index: opened.index,
            props: opened.props,
            filter: opened.filter,
            fragments: opened.fragments,
            ctx: ctx.clone(),
            obsolete: AtomicBool::new(false),
            unknown_ro_compat: opened.header.unknown_ro_compat(),
        })
    }

    /// What the MANIFEST records about this table when it is in `level`.
    pub fn meta(&self, level: u8) -> FileMeta {
        file_meta(level, self.number, self.size, &self.props)
    }

    /// Marks the table as replaced: its file goes when the last reference
    /// does. Until then its bytes count as retained by snapshots
    /// (`snapshot_retained_bytes`).
    pub fn mark_obsolete(&self) {
        if !self.obsolete.swap(true, Ordering::AcqRel) {
            self.ctx.stats.retained_table_bytes.fetch_add(self.size, Ordering::Relaxed);
        }
    }

    /// Whether no key `>= key` can be in the table (the same rule as
    /// `FileMeta::ends_before`, MAN-S3): its last key is below `key`, or its
    /// extent ends exactly at `key` without including it.
    pub fn ends_before(&self, key: &[u8]) -> bool {
        if self.largest_exclusive { self.largest.as_slice() <= key } else { self.largest.as_slice() < key }
    }

    /// The highest sequence in the table, points and fragments.
    pub fn largest_seq(&self) -> u64 {
        self.props.largest_seq
    }

    pub fn point_deletes(&self) -> u64 {
        self.props.point_deletes
    }

    pub fn has_fragments(&self) -> bool {
        !self.fragments.is_empty()
    }

    pub fn fragments(&self) -> &[Fragment] {
        &self.fragments
    }

    /// The sequence of the fragment covering `key`, if one does. Every
    /// fragment of a table is visible to whoever sees the table (SEM-5).
    pub fn covering_seq(&self, key: &[u8]) -> Option<u64> {
        crate::format::rangedel::covering_seq(&self.fragments, key)
    }

    /// Removes the blocks of this table from the block cache.
    pub fn purge_cache(&self) {
        for e in &self.index {
            self.ctx.cache.remove(&(self.number, e.handle.offset));
        }
    }

    fn corrupt(&self, msg: String) -> Error {
        Error::Corruption(format!("{}: {msg}", sst_name(self.number)))
    }

    /// Reads (through the cache unless `fill_cache` is false and the block
    /// is not cached) and verifies data block `i` (SST-R1..R3).
    pub fn read_block(&self, i: usize, fill_cache: bool) -> Result<Arc<Block>> {
        let handle = self.index[i].handle;
        let key = (self.number, handle.offset);
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
        let mut buf = vec![0u8; handle.len as usize];
        self.file.read_at(&mut buf, handle.offset)?;
        read_data_block(&buf, self.number, &self.index, i, &self.props)
            .map_err(|e| format_error(&sst_name(self.number), e))?;
        let block = Arc::new(Block::from_validated(buf));
        if fill_cache {
            self.ctx.cache.insert(key, block.clone());
        }
        Ok(block)
    }

    /// `Some((seq, Some(v)))` value, `Some((seq, None))` tombstone, `None`
    /// not in this table.
    pub fn get(&self, key: &[u8], hash: u64) -> Result<Option<(u64, Option<Vec<u8>>)>> {
        // A key at the exclusive end of the table is not one it holds.
        if key < self.smallest.as_slice()
            || key > self.largest.as_slice()
            || (self.largest_exclusive && key == self.largest.as_slice())
        {
            return Ok(None);
        }
        let stats = &self.ctx.stats;
        bump(&stats.bloom_checks, 1);
        if !self.filter.may_contain(hash) {
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
            return Ok(Some((e.seq, block.value(&e).map(<[u8]>::to_vec))));
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
        if self.obsolete.load(Ordering::Acquire) {
            self.ctx.stats.retained_table_bytes.fetch_sub(self.size, Ordering::Relaxed);
            self.ctx.deleter.submit(self.ctx.path(self.number), self.size);
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
        if entry < block.len() {
            // Position in the block already read (a second `read_block`
            // would hit the disk again when the cache is bypassed).
            let e = block.entry(entry).ok_or_else(|| self.table.corrupt("unreadable block".into()))?;
            self.block_idx = idx;
            self.pos = Some((block, entry, e));
            return Ok(());
        }
        self.load(idx + 1, 0)
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
    use crate::format::bloom;
    use crate::format::props::REASON_FLUSH;
    use kerno_testkit::vfs::SimVfs;
    use std::path::Path;

    const DB: [u8; 16] = [0x5a; 16];

    fn ctx(block_size: usize) -> (SimVfs, Arc<TableCtx>) {
        let sim = SimVfs::new(1);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/t")).unwrap();
        let stats = Arc::new(Stats::default());
        let ctx = TableCtx::new(vfs, PathBuf::from("/t"), 1 << 20, stats, block_size).with_db_id(DB);
        (sim, Arc::new(ctx))
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
        let mut b = TableBuilder::create(ctx, number, REASON_FLUSH).unwrap();
        for i in 0..n {
            b.add(&key(i * 2), i as u64 + 1, value(i).as_deref()).unwrap();
        }
        b.finish().unwrap()
    }

    fn reopen(ctx: &Arc<TableCtx>, t: &Table) -> Result<Table> {
        Table::open(ctx, &t.meta(0), MAX_SEQ)
    }

    #[test]
    fn build_and_read_back() {
        let (_sim, ctx) = ctx(512);
        let n = 500;
        let built = build(&ctx, 7, n);
        assert!(built.index.len() > 10, "expected many blocks");
        let t = reopen(&ctx, &built).unwrap();
        assert_eq!(t.meta(0), built.meta(0));
        assert_eq!(
            (t.meta(3).level, t.meta(0).point_entries, t.meta(0).smallest_seq, t.meta(0).largest_seq),
            (3, 500, 1, 500)
        );
        assert_eq!(t.smallest, key(0));
        assert_eq!(t.largest, key((n - 1) * 2));
        for i in 0..n {
            let got = t.get(&key(i * 2), bloom::hash(&key(i * 2))).unwrap();
            assert_eq!(got, Some((i as u64 + 1, value(i))), "key {i}");
            let absent = key(i * 2 + 1);
            assert_eq!(t.get(&absent, bloom::hash(&absent)).unwrap(), None);
        }
        // Full iteration and seeks, including block boundaries.
        let mut it = t.iter(true);
        it.seek_to_first().unwrap();
        for i in 0..n {
            assert!(it.valid());
            assert_eq!((it.key(), it.seq(), it.value()), (key(i * 2).as_slice(), i as u64 + 1, value(i).as_deref()));
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
        for block_size in [64, 512, 4096] {
            let (_sim, ctx) = ctx(block_size);
            for n in [1, 2, 3, 17] {
                let built = build(&ctx, 100 + n as u64, n);
                let t = reopen(&ctx, &built).unwrap();
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
    fn tables_carry_the_creation_reason_and_database_id() {
        let (_sim, ctx) = ctx(4096);
        let mut b = TableBuilder::create(&ctx, 3, crate::format::props::REASON_RECOVERY).unwrap();
        b.add(b"k", 9, Some(b"v")).unwrap();
        let t = b.finish().unwrap();
        assert_eq!(t.props.creation_reason, crate::format::props::REASON_RECOVERY);
        // Another database's table is not this one's (HDR-10).
        let other = Arc::new(TableCtx::new(ctx.vfs.clone(), ctx.dir.clone(), 1 << 20, ctx.stats.clone(), 4096));
        let e = Table::open(&other, &t.meta(0), MAX_SEQ).err().unwrap();
        assert!(matches!(&e, Error::Corruption(m) if m.contains("HDR-10")), "{e:?}");
    }

    #[test]
    fn estimated_size_tracks_the_open_block() {
        let (_sim, ctx) = ctx(4096);
        let mut b = TableBuilder::create(&ctx, 1, REASON_FLUSH).unwrap();
        let mut last = b.estimated_size();
        for i in 0..200 {
            b.add(&key(i), 1, Some(&[7u8; 100])).unwrap();
            let now = b.estimated_size();
            assert!(now > last, "the estimate grows with every entry");
            last = now;
        }
        let t = b.finish().unwrap();
        assert!(t.size > last, "the finished table adds filter, properties, index and footer");
    }

    #[test]
    fn bytes_per_sync_syncs_while_writing() {
        use kerno_testkit::vfs::CountingVfs;
        for (every, suppressed) in [(None, false), (Some(1u64), false), (Some(1u64), true)] {
            let (vfs, io) = CountingVfs::wrap(SimVfs::new(1).as_vfs());
            vfs.create_dir_all(Path::new("/t")).unwrap();
            let stats = Arc::new(Stats::default());
            let ctx = Arc::new(TableCtx::new(vfs, PathBuf::from("/t"), 1 << 20, stats, 64).with_bytes_per_sync(every));
            let mut b = TableBuilder::create(&ctx, 1, REASON_FLUSH).unwrap();
            if suppressed {
                b = b.without_intermediate_syncs();
            }
            // About 1 MiB of entries: several 256 KiB chunks.
            for i in 0..1000 {
                b.add(&key(i), 1, Some(&[7u8; 1000])).unwrap();
            }
            b.finish().unwrap();
            let io = io.snapshot();
            assert!(io.writes >= 4, "{} writes", io.writes);
            let expected = match every {
                // Only the final sync.
                None => 1,
                Some(_) if suppressed => 1,
                // After every write, plus the final one.
                Some(_) => io.writes + 1,
            };
            assert_eq!(
                io.file_syncs, expected,
                "bytes_per_sync {every:?} (suppressed: {suppressed}), {} writes",
                io.writes
            );
        }
    }

    #[test]
    fn bytes_per_writeback_asks_for_writeback_while_writing() {
        use kerno_testkit::vfs::OpKind;
        // (bytes_per_sync, bytes_per_writeback, suppressed) and whether hints are expected.
        let cases = [
            (None, None, false, false),
            (None, Some(1u64), false, true),
            // A sync already bounds the dirty data: no hint on top of it.
            (Some(1u64), Some(1u64), false, false),
            (None, Some(1u64), true, false),
        ];
        for (sync, writeback, suppressed, hints) in cases {
            let sim = SimVfs::new(1);
            sim.start_trace();
            let vfs = sim.as_vfs();
            vfs.create_dir_all(Path::new("/t")).unwrap();
            let stats = Arc::new(Stats::default());
            let ctx = Arc::new(
                TableCtx::new(vfs, PathBuf::from("/t"), 1 << 20, stats.clone(), 64)
                    .with_bytes_per_sync(sync)
                    .with_bytes_per_writeback(writeback),
            );
            let mut b = TableBuilder::create(&ctx, 1, REASON_FLUSH).unwrap();
            if suppressed {
                b = b.without_intermediate_syncs();
            }
            for i in 0..1000 {
                b.add(&key(i), 1, Some(&[7u8; 1000])).unwrap();
            }
            let table = b.finish().unwrap();
            let trace = sim.trace();
            let writes: Vec<_> = trace.iter().filter(|t| t.op.kind == OpKind::WriteAt).collect();
            let hinted: Vec<_> = trace.iter().filter(|t| t.op.kind == OpKind::StartWriteback).collect();
            let waited: Vec<_> = trace.iter().filter(|t| t.op.kind == OpKind::WaitWriteback).collect();
            assert!(writes.len() >= 4, "{} writes", writes.len());
            let ctx_desc = format!("sync {sync:?}, writeback {writeback:?}, suppressed {suppressed}");
            assert_eq!(stats.sst_writeback_hints.load(std::sync::atomic::Ordering::Relaxed), hinted.len() as u64);
            if !hints {
                assert!(hinted.is_empty() && waited.is_empty(), "{ctx_desc}: {} hints", hinted.len());
                continue;
            }
            // With 1 byte as the interval, every chunk written is hinted, and
            // the hints tile the file with no gap or overlap.
            assert_eq!(hinted.len(), writes.len(), "{ctx_desc}");
            for (w, h) in writes.iter().zip(&hinted) {
                assert_eq!(
                    (h.op.offset, h.op.len),
                    (w.op.offset, w.op.len),
                    "{ctx_desc}: hint does not cover its chunk"
                );
                assert!(w.n < h.n, "{ctx_desc}: hint before the data it covers");
            }
            // Each request is followed by the wait for the same range.
            assert_eq!(waited.len(), hinted.len(), "{ctx_desc}");
            for (w, h) in waited.iter().zip(&hinted) {
                assert_eq!((w.op.offset, w.op.len), (h.op.offset, h.op.len), "{ctx_desc}: waits for another range");
                assert_eq!(w.n, h.n + 1, "{ctx_desc}: the wait does not follow the request");
            }
            assert_eq!(table.size, writes.iter().map(|w| w.op.len).sum::<u64>());
        }
    }

    #[test]
    fn a_failed_writeback_request_does_not_fail_the_table() {
        use kerno_testkit::vfs::OpKind;
        let sim = SimVfs::new(2);
        let vfs = sim.as_vfs();
        vfs.create_dir_all(Path::new("/t")).unwrap();
        let ctx = Arc::new(
            TableCtx::new(vfs, PathBuf::from("/t"), 1 << 20, Arc::new(Stats::default()), 64)
                .with_bytes_per_writeback(Some(1)),
        );
        // The first request fails (one-shot); the data and the final sync are
        // what make the table valid.
        sim.fail_when(|op| op.kind == OpKind::StartWriteback, 1, None);
        let mut b = TableBuilder::create(&ctx, 1, REASON_FLUSH).unwrap();
        for i in 0..1000 {
            b.add(&key(i), 1, Some(&[7u8; 1000])).unwrap();
        }
        let table = b.finish().unwrap();
        assert!(sim.fault_injected(), "the request that was to fail never happened");
        let mut it = table.iter(false);
        it.seek_to_first().unwrap();
        let mut n = 0;
        while it.valid() {
            n += 1;
            it.next().unwrap();
        }
        assert_eq!(n, 1000);
    }

    #[test]
    fn damage_is_detected() {
        let (sim, ctx) = ctx(512);
        let built = build(&ctx, 3, 60);
        let meta = built.meta(0);
        drop(built);
        let path = ctx.path(3);
        let size = meta.size;
        // Flip one bit at many positions: either open fails or every read
        // of the damaged block fails; never wrong data.
        for off in (0..size).step_by(37) {
            sim.corrupt_byte(&path, off, 0x10);
            match Table::open(&ctx, &meta, MAX_SEQ) {
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
        // Wrong size, key range or sequences in the MANIFEST.
        let mut m = meta.clone();
        m.size -= 1;
        assert!(matches!(Table::open(&ctx, &m, MAX_SEQ), Err(Error::Corruption(_))));
        let mut m = meta.clone();
        m.largest = b"other".to_vec();
        assert!(matches!(Table::open(&ctx, &m, MAX_SEQ), Err(Error::Corruption(_))));
        let mut m = meta.clone();
        m.largest_seq += 1;
        assert!(matches!(Table::open(&ctx, &m, MAX_SEQ), Err(Error::Corruption(_))));
        // A table with sequences above the MANIFEST's last_seq.
        assert!(matches!(Table::open(&ctx, &meta, meta.largest_seq - 1), Err(Error::Corruption(_))));
    }

    #[test]
    fn a_block_moved_within_the_file_does_not_verify() {
        let (_sim, ctx) = ctx(512);
        // Equal entries, so the blocks have the same length.
        let mut b = TableBuilder::create(&ctx, 4, REASON_FLUSH).unwrap();
        for i in 0..100 {
            b.add(&key(i), 1, Some(&[b'v'; 120])).unwrap();
        }
        let t = b.finish().unwrap();
        let (h0, h1) = (t.index[0].handle, t.index[1].handle);
        assert_eq!(h0.len, h1.len, "test setup: blocks of one length");
        // Block 0's bytes at block 1's offset.
        let f = ctx.vfs.open(&ctx.path(4), OpenMode::Existing).unwrap();
        let mut bytes = vec![0; h0.len as usize];
        f.read_at(&mut bytes, h0.offset).unwrap();
        drop(t);
        f.write_at(&bytes, h1.offset).unwrap();
        let t = Table::open(&ctx, &written_meta(&ctx, 4), MAX_SEQ).unwrap();
        let e = t.read_block(1, false).err().expect("a moved block must not verify");
        assert!(matches!(&e, Error::Corruption(m) if m.contains("SST-R1")), "{e:?}");
        assert!(t.read_block(0, false).is_ok());
    }

    /// The `FileMeta` of table `number` as written (read back from its own
    /// properties, not from the damaged data blocks).
    fn written_meta(ctx: &Arc<TableCtx>, number: u64) -> FileMeta {
        let f = ctx.vfs.open(&ctx.path(number), OpenMode::Existing).unwrap();
        let size = f.len().unwrap();
        let mut src = FileSource::new(&*f).unwrap();
        let ex = SstExpect { file_number: number, db_id: DB, meta: None, last_seq: MAX_SEQ };
        let m = open_sst_from(&mut src, &ex).map_err(|e| read_error("t", e)).unwrap();
        file_meta(0, number, size, &m.props)
    }
}
