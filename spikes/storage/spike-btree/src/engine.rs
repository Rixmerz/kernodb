//! The [`Engine`] implementation: open/recovery, the commit protocol,
//! snapshots and the read path.

use std::any::Any;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use parking_lot::Mutex;
use quick_cache::Weighter;
use quick_cache::sync::Cache;
use spike_core::vfs::{File, OpenMode, Vfs};
use spike_core::{Durability, Engine, Error, Options, Result, Snapshot, WriteBatch};

use crate::freelist::{FreeState, PageWrites};
use crate::meta::{self, Meta};
use crate::page::{
    Body, Bytes, FIRST_DATA_PAGE, Node, OVERFLOW_CAPACITY, OVERFLOW_HEADER, PAGE_SIZE, PageId, PageRef, Value,
    check_page_id, overflow_page_next,
};
use crate::tree::{NewRoot, Txn, Writeout, dedupe};

const DATA_FILE: &str = "data.kbt";
const DATA_TMP: &str = "data.kbt.tmp";
const LOCK_FILE: &str = "LOCK";

/// Reads the full image of page `id`.
pub(crate) fn read_page(file: &dyn File, id: PageId) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; PAGE_SIZE];
    file.read_at(&mut buf, id * PAGE_SIZE as u64)?;
    Ok(buf)
}

/// Cache key: (page id, crc, kind). Tree nodes are keyed by their page id
/// and page crc; overflow values by their first page id and chain crc.
type CacheKey = (PageId, u32, u8);
const NODE: u8 = 0;
const VALUE: u8 = 1;

/// One cache shared by tree nodes and assembled overflow values, so large
/// values compete for the same budget instead of always going to disk.
#[derive(Clone)]
enum Cached {
    Node(Arc<Node>),
    Value(Arc<[u8]>),
}

#[derive(Clone)]
struct CacheWeighter;

impl Weighter<CacheKey, Cached> for CacheWeighter {
    fn weight(&self, _key: &CacheKey, entry: &Cached) -> u64 {
        match entry {
            Cached::Node(node) => node.weight(),
            Cached::Value(bytes) => bytes.len() as u64 + 64,
        }
    }
}

#[derive(Default)]
struct Counters {
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    overflow_pages_read: AtomicU64,
    commits: AtomicU64,
    tree_pages_written: AtomicU64,
    overflow_pages_written: AtomicU64,
    freelist_pages_written: AtomicU64,
    freelist_compactions: AtomicU64,
    fsyncs: AtomicU64,
}

/// What a snapshot reads: one committed tree.
#[derive(Clone, Copy, Debug)]
struct View {
    seq: u64,
    root: Option<PageRef>,
    height: u8,
    page_count: u64,
}

impl View {
    fn of(m: &Meta) -> View {
        View { seq: m.seq, root: m.root, height: m.height, page_count: m.page_count }
    }
}

/// The latest committed view plus the registry of live snapshots
/// (sequence → count). Guarded by one short-lived mutex so that taking a
/// snapshot and registering it is atomic with respect to the writer.
struct Published {
    view: View,
    readers: BTreeMap<u64, usize>,
}

/// State shared by the engine handle and its snapshots.
pub(crate) struct Inner {
    file: Arc<dyn File>,
    cache: Cache<CacheKey, Cached, CacheWeighter>,
    published: Mutex<Published>,
    counters: Counters,
    /// Held until the engine and every snapshot are gone, so no other handle
    /// can modify pages a lingering snapshot still reads.
    _lock: Box<dyn Any + Send + Sync>,
}

impl Inner {
    /// Reads a tree node through the cache, verifying the page's crc against
    /// the one stored by its parent (or the meta page) on a miss.
    pub(crate) fn read_node(&self, r: PageRef, level: u8, page_count: u64) -> Result<Arc<Node>> {
        let node = match self.cache.get(&(r.id, r.crc, NODE)) {
            Some(Cached::Node(n)) => {
                self.counters.cache_hits.fetch_add(1, Relaxed);
                n
            }
            _ => {
                self.counters.cache_misses.fetch_add(1, Relaxed);
                let n = Arc::new(self.load_node(r, page_count)?);
                self.cache.insert((r.id, r.crc, NODE), Cached::Node(n.clone()));
                n
            }
        };
        if node.level != level {
            return Err(Error::corruption(format!("page {}: level {} where {level} was expected", r.id, node.level)));
        }
        Ok(node)
    }

    /// Reads and verifies a node from the file, bypassing the cache.
    fn load_node(&self, r: PageRef, page_count: u64) -> Result<Node> {
        check_page_id(r.id, page_count, "tree pointer")?;
        let page = read_page(&*self.file, r.id)?;
        let crc = crc32c::crc32c(&page);
        if crc != r.crc {
            return Err(Error::corruption(format!(
                "page {}: checksum mismatch (stored {:#010x}, computed {crc:#010x})",
                r.id, r.crc
            )));
        }
        Node::decode(&page, r.id, page_count)
    }

    /// Walks an overflow chain of `len` bytes. The walk is bounded by the
    /// length (never more than `ceil(len / capacity)` pages), every pointer is
    /// range-checked, and the crc over all page images must match `crc`.
    fn walk_overflow(
        &self,
        first: PageId,
        len: u32,
        crc: u32,
        page_count: u64,
        mut visit: impl FnMut(PageId, &[u8]),
    ) -> Result<()> {
        let pages = (len as usize).div_ceil(OVERFLOW_CAPACITY);
        let mut id = first;
        let mut actual = 0u32;
        let mut left = len as usize;
        for i in 0..pages {
            check_page_id(id, page_count, "overflow pointer")?;
            let page = read_page(&*self.file, id)?;
            self.counters.overflow_pages_read.fetch_add(1, Relaxed);
            actual = crc32c::crc32c_append(actual, &page);
            let next = overflow_page_next(&page, id)?;
            if (next == 0) != (i + 1 == pages) {
                return Err(Error::corruption(format!("overflow page {id}: chain length does not match value length")));
            }
            let take = left.min(OVERFLOW_CAPACITY);
            visit(id, &page[OVERFLOW_HEADER..OVERFLOW_HEADER + take]);
            left -= take;
            id = next;
        }
        if actual != crc {
            return Err(Error::corruption(format!("overflow chain at page {first}: checksum mismatch")));
        }
        Ok(())
    }

    fn read_value(&self, v: &Value, page_count: u64) -> Result<Vec<u8>> {
        match v {
            Value::Inline(bytes) => Ok(bytes.to_vec()),
            &Value::Overflow { len, crc, first } => {
                if let Some(Cached::Value(bytes)) = self.cache.get(&(first, crc, VALUE)) {
                    self.counters.cache_hits.fetch_add(1, Relaxed);
                    return Ok(bytes.to_vec());
                }
                self.counters.cache_misses.fetch_add(1, Relaxed);
                let mut out = Vec::with_capacity(len as usize);
                self.walk_overflow(first, len, crc, page_count, |_, chunk| out.extend_from_slice(chunk))?;
                self.cache.insert((first, crc, VALUE), Cached::Value(Arc::from(out.as_slice())));
                Ok(out)
            }
        }
    }

    /// Page ids of a (verified) overflow chain.
    pub(crate) fn overflow_pages(&self, first: PageId, len: u32, crc: u32, page_count: u64) -> Result<Vec<PageId>> {
        let mut ids = Vec::new();
        self.walk_overflow(first, len, crc, page_count, |id, _| ids.push(id))?;
        Ok(ids)
    }

    fn snapshot_view(&self) -> View {
        let mut p = self.published.lock();
        let view = p.view;
        *p.readers.entry(view.seq).or_default() += 1;
        view
    }

    fn release_view(&self, seq: u64) {
        let mut p = self.published.lock();
        if let Some(n) = p.readers.get_mut(&seq) {
            *n -= 1;
            if *n == 0 {
                p.readers.remove(&seq);
            }
        }
    }

    fn get(&self, view: &View, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let Some(mut r) = view.root else { return Ok(None) };
        let mut level = view.height - 1;
        loop {
            let node = self.read_node(r, level, view.page_count)?;
            match &node.body {
                Body::Branch { keys, children } => {
                    r = children[keys.partition_point(|s| **s <= *key)];
                    level -= 1;
                }
                Body::Leaf(entries) => {
                    return match entries.binary_search_by(|e| (*e.key).cmp(key)) {
                        Ok(i) => Ok(Some(self.read_value(&entries[i].value, view.page_count)?)),
                        Err(_) => Ok(None),
                    };
                }
            }
        }
    }

    /// Range scan with an explicit stack of (branch, next child index); no
    /// sibling pointers, which copy-on-write could not maintain.
    fn scan(
        &self,
        view: &View,
        start: Bound<&[u8]>,
        end: Bound<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut out = Vec::new();
        let Some(root) = view.root else { return Ok(out) };
        if limit == 0 || range_is_empty(start, end) {
            return Ok(out);
        }
        let mut stack: Vec<(Arc<Node>, usize)> = Vec::new();
        // Descend to the leaf that would hold the start key.
        let mut node = self.read_node(root, view.height - 1, view.page_count)?;
        while let Body::Branch { keys, children } = &node.body {
            let i = match start {
                Bound::Unbounded => 0,
                Bound::Included(k) | Bound::Excluded(k) => keys.partition_point(|s| **s <= *k),
            };
            let child = self.read_node(children[i], node.level - 1, view.page_count)?;
            stack.push((node, i + 1));
            node = child;
        }
        let Body::Leaf(entries) = &node.body else { unreachable!() };
        let mut pos = match start {
            Bound::Unbounded => 0,
            Bound::Included(k) => entries.partition_point(|e| *e.key < *k),
            Bound::Excluded(k) => entries.partition_point(|e| *e.key <= *k),
        };
        loop {
            let Body::Leaf(entries) = &node.body else { unreachable!() };
            for e in &entries[pos..] {
                let in_range = match end {
                    Bound::Unbounded => true,
                    Bound::Included(k) => *e.key <= *k,
                    Bound::Excluded(k) => *e.key < *k,
                };
                if !in_range {
                    return Ok(out);
                }
                out.push((e.key.to_vec(), self.read_value(&e.value, view.page_count)?));
                if out.len() == limit {
                    return Ok(out);
                }
            }
            // Next leaf: climb to the nearest branch with an unvisited child,
            // then descend along leftmost children.
            loop {
                let Some((branch, next)) = stack.last_mut() else { return Ok(out) };
                let Body::Branch { children, .. } = &branch.body else { unreachable!() };
                if *next < children.len() {
                    let (r, level) = (children[*next], branch.level - 1);
                    *next += 1;
                    node = self.read_node(r, level, view.page_count)?;
                    break;
                }
                stack.pop();
            }
            while let Body::Branch { children, .. } = &node.body {
                let child = self.read_node(children[0], node.level - 1, view.page_count)?;
                stack.push((node, 1));
                node = child;
            }
            pos = 0;
        }
    }
}

fn range_is_empty(start: Bound<&[u8]>, end: Bound<&[u8]>) -> bool {
    match (start, end) {
        (Bound::Included(a), Bound::Included(b)) => a > b,
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => a >= b,
        _ => false,
    }
}

/// Writer-side state, guarded by the writer mutex.
struct Writer {
    /// Meta of the last commit (durable unless `unsynced`).
    meta: Meta,
    free: FreeState,
    durability: Durability,
    poisoned: Option<String>,
    /// Buffered commits were written without `sync_data`.
    unsynced: bool,
    /// Slot holding `meta`; the next commit writes the other one.
    meta_slot: usize,
    /// Both meta slots hold `meta` (after a clean close / on reopen).
    both_slots_current: bool,
}

/// Copy-on-write B+ tree engine (see the crate documentation).
pub struct BTreeEngine {
    inner: Arc<Inner>,
    writer: Mutex<Writer>,
}

/// A read view of one commit; keeps its pages from being reused.
pub struct BTreeSnapshot {
    inner: Arc<Inner>,
    view: View,
}

impl Drop for BTreeSnapshot {
    fn drop(&mut self) {
        self.inner.release_view(self.view.seq);
    }
}

impl Snapshot for BTreeSnapshot {
    fn seq(&self) -> u64 {
        self.view.seq
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.inner.get(&self.view, key)
    }

    fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.inner.scan(&self.view, start, end, limit)
    }
}

/// Creates the data file atomically: the initial metas are written to a
/// temporary file, synced and renamed into place (`open` then syncs the
/// directory). An existing data file therefore always had valid metas at some
/// point, and a damaged one is reported instead of being mistaken for "no
/// database".
fn create_data_file(vfs: &dyn Vfs, dir: &Path) -> Result<Arc<dyn File>> {
    let tmp = dir.join(DATA_TMP);
    if vfs.exists(&tmp)? {
        vfs.remove_file(&tmp)?;
    }
    let file = vfs.open(&tmp, OpenMode::CreateNew)?;
    let m = Meta::empty().encode();
    let mut both = Vec::with_capacity(2 * PAGE_SIZE);
    both.extend_from_slice(&m);
    both.extend_from_slice(&m);
    file.write_at(&both, 0)?;
    file.sync_data()?;
    vfs.rename(&tmp, &dir.join(DATA_FILE))?;
    Ok(file)
}

impl BTreeEngine {
    fn commit_locked(&self, w: &mut Writer, batch: &WriteBatch) -> Result<u64> {
        let inner = &*self.inner;
        let seq = w.meta.seq.checked_add(1).ok_or_else(|| Error::corruption("commit sequence exhausted"))?;
        // Pages freed by commit n were reachable only from trees before n.
        // Meta n (durable in Sync mode) superseded meta n - 1, so they become
        // reusable once no live snapshot is older than n.
        let oldest = inner.published.lock().readers.keys().next().copied();
        w.free.release(oldest.unwrap_or(u64::MAX));

        // 1. Apply the batch to in-memory copies of the touched paths.
        let ops = dedupe(batch);
        let mut txn = Txn::new(inner, w.meta.page_count);
        let new_root = txn.apply_root(w.meta.root, w.meta.height, &ops)?;
        let entries = w
            .meta
            .entries
            .checked_add_signed(txn.entries_delta)
            .ok_or_else(|| Error::corruption("key count underflow"))?;

        // 2. Assign page ids bottom-up and encode the pages.
        let mut out = Writeout::new(&mut w.free);
        let (root, height) = match new_root {
            NewRoot::Empty => (None, 0),
            NewRoot::Tree { root, level } => (Some(out.write_child(root)?), level + 1),
        };
        let Writeout { allocated, mut writes, nodes, overflow_pages, .. } = out;
        let tree_pages = writes.len() as u64 - overflow_pages;
        let replaced = txn.replaced;
        let released_values = txn.released_values;
        let log = w.free.commit(seq, allocated, txn.freed, &mut writes);

        // 3. Pages, barrier, meta into the other slot, barrier.
        let sync = w.durability == Durability::Sync;
        self.write_pages(&mut writes)?;
        if sync {
            self.sync()?;
        }
        let meta = Meta {
            seq,
            root,
            height,
            page_count: w.free.page_count,
            free_head: w.free.log.head,
            free_log_pages: w.free.log.pages.len() as u64,
            free_count: w.free.total_free(),
            entries,
        };
        let slot = 1 - w.meta_slot;
        inner.file.write_at(&meta.encode(), Meta::slot_offset(slot))?;
        if sync {
            self.sync()?;
        } else {
            w.unsynced = true;
        }

        // 4. Publish.
        w.meta = meta;
        w.meta_slot = slot;
        w.both_slots_current = false;
        // Superseded versions stay readable from disk for older snapshots,
        // but keeping them cached would crowd out the live tree.
        for r in replaced {
            inner.cache.remove(&(r.id, r.crc, NODE));
        }
        for (first, crc) in released_values {
            inner.cache.remove(&(first, crc, VALUE));
        }
        for (r, node) in nodes {
            inner.cache.insert((r.id, r.crc, NODE), Cached::Node(node));
        }
        inner.published.lock().view = View::of(&meta);
        let c = &inner.counters;
        c.commits.fetch_add(1, Relaxed);
        c.tree_pages_written.fetch_add(tree_pages, Relaxed);
        c.overflow_pages_written.fetch_add(overflow_pages, Relaxed);
        c.freelist_pages_written.fetch_add(log.pages_written, Relaxed);
        c.freelist_compactions.fetch_add(log.compacted as u64, Relaxed);
        Ok(seq)
    }

    fn sync(&self) -> Result<()> {
        self.inner.counters.fsyncs.fetch_add(1, Relaxed);
        self.inner.file.sync_data()?;
        Ok(())
    }

    /// Writes pages sorted by id, one `write_at` per run of consecutive ids.
    fn write_pages(&self, writes: &mut PageWrites) -> Result<()> {
        writes.sort_unstable_by_key(|(id, _)| *id);
        debug_assert!(writes.windows(2).all(|w| w[0].0 < w[1].0), "page written twice in one commit");
        let mut i = 0;
        while i < writes.len() {
            let start = writes[i].0;
            let mut j = i + 1;
            while j < writes.len() && writes[j].0 == start + (j - i) as u64 {
                j += 1;
            }
            let offset = start * PAGE_SIZE as u64;
            if j == i + 1 {
                self.inner.file.write_at(&writes[i].1, offset)?;
            } else {
                let mut run = Vec::with_capacity((j - i) * PAGE_SIZE);
                for (_, page) in &writes[i..j] {
                    run.extend_from_slice(page);
                }
                self.inner.file.write_at(&run, offset)?;
            }
            i = j;
        }
        Ok(())
    }

    fn close_locked(&self, w: &mut Writer) -> Result<()> {
        if w.unsynced {
            // Make the latest meta and everything it references durable
            // before the second copy of the meta is written.
            self.sync()?;
            w.unsynced = false;
        }
        if !w.both_slots_current {
            self.inner.file.write_at(&w.meta.encode(), Meta::slot_offset(1 - w.meta_slot))?;
            self.sync()?;
            w.both_slots_current = true;
        }
        Ok(())
    }

    /// Full consistency check of the current state (tests and tooling): walks
    /// the whole tree from disk, verifying every checksum, key order and
    /// separator bound, and checks that every page below the high-water mark
    /// is used exactly once (meta, tree node, overflow page, free-list log
    /// page, or free) and that the persisted free list equals the in-memory
    /// one.
    pub fn verify(&self) -> Result<VerifyReport> {
        let w = self.writer.lock();
        if let Some(m) = &w.poisoned {
            return Err(Error::Poisoned(m.clone()));
        }
        let inner = &*self.inner;
        let meta = w.meta;
        let on_disk = read_page(&*inner.file, w.meta_slot as PageId)?;
        if Meta::decode(&on_disk).ok() != Some(meta) {
            return Err(Error::corruption("verify: the current meta slot on disk differs from the committed meta"));
        }
        const FREE: u8 = 1;
        const TREE: u8 = 2;
        const OVERFLOW: u8 = 3;
        const LOG: u8 = 4;
        let mut owner = vec![0u8; meta.page_count as usize];
        owner[0] = u8::MAX;
        owner[1] = u8::MAX;
        let claim = |owner: &mut Vec<u8>, id: PageId, what: u8| -> Result<()> {
            match owner.get_mut(id as usize) {
                Some(o) if *o == 0 => {
                    *o = what;
                    Ok(())
                }
                Some(o) => Err(Error::corruption(format!("verify: page {id} used twice (kinds {o} and {what})"))),
                None => Err(Error::corruption(format!("verify: page {id} beyond the high-water mark"))),
            }
        };
        let mut report = VerifyReport { page_count: meta.page_count, height: meta.height, ..Default::default() };
        if let Some(root) = meta.root {
            // Iterative walk: (page, level, lower bound, upper bound).
            type Bounds = (Option<Bytes>, Option<Bytes>);
            let mut todo: Vec<(PageRef, u8, Bounds)> = vec![(root, meta.height - 1, (None, None))];
            while let Some((r, level, (lo, hi))) = todo.pop() {
                claim(&mut owner, r.id, TREE)?;
                report.tree_pages += 1;
                let node = inner.load_node(r, meta.page_count)?;
                if node.level != level {
                    return Err(Error::corruption(format!("verify: page {} has the wrong level", r.id)));
                }
                let in_bounds =
                    |k: &[u8]| lo.as_deref().is_none_or(|lo| k >= lo) && hi.as_deref().is_none_or(|hi| k < hi);
                match node.body {
                    Body::Leaf(entries) => {
                        for e in &entries {
                            if !in_bounds(&e.key) {
                                return Err(Error::corruption(format!("verify: key outside its subtree in {}", r.id)));
                            }
                            if let Value::Overflow { len, crc, first } = e.value {
                                for id in inner.overflow_pages(first, len, crc, meta.page_count)? {
                                    claim(&mut owner, id, OVERFLOW)?;
                                    report.overflow_pages += 1;
                                }
                            }
                        }
                        report.entries += entries.len() as u64;
                    }
                    Body::Branch { keys, children } => {
                        if keys.iter().any(|k| !in_bounds(k)) {
                            return Err(Error::corruption(format!(
                                "verify: separator outside its subtree in {}",
                                r.id
                            )));
                        }
                        for (i, c) in children.into_iter().enumerate() {
                            let clo = if i == 0 { lo.clone() } else { Some(keys[i - 1].clone()) };
                            let chi = if i < keys.len() { Some(keys[i].clone()) } else { hi.clone() };
                            todo.push((c, level - 1, (clo, chi)));
                        }
                    }
                }
            }
        }
        if report.entries != meta.entries {
            return Err(Error::corruption(format!("verify: {} keys, meta records {}", report.entries, meta.entries)));
        }
        for &id in &w.free.log.pages {
            claim(&mut owner, id, LOG)?;
            report.freelist_pages += 1;
        }
        let free = w.free.all_free();
        for &id in &free {
            claim(&mut owner, id, FREE)?;
        }
        report.free_pages = free.len() as u64;
        if let Some(id) = owner.iter().position(|&o| o == 0) {
            return Err(Error::corruption(format!("verify: page {id} leaked (neither used nor free)")));
        }
        if w.free.page_count != meta.page_count {
            return Err(Error::corruption("verify: allocator high-water mark differs from the meta"));
        }
        let persisted = FreeState::load(&*inner.file, &meta)?;
        if persisted.reusable != free {
            return Err(Error::corruption("verify: persisted free list differs from the in-memory one"));
        }
        Ok(report)
    }
}

/// Summary returned by [`BTreeEngine::verify`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub entries: u64,
    pub height: u8,
    pub page_count: u64,
    pub tree_pages: u64,
    pub overflow_pages: u64,
    pub freelist_pages: u64,
    pub free_pages: u64,
}

impl Engine for BTreeEngine {
    type Snapshot = BTreeSnapshot;

    fn name() -> &'static str {
        "btree"
    }

    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> Result<Self> {
        vfs.create_dir_all(dir)?;
        // The directory itself must be durable in its parent, or a power
        // loss can remove a newly created database with everything in it.
        if let Some(parent) = dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            vfs.sync_dir(parent)?;
        }
        let lock = vfs.lock(&dir.join(LOCK_FILE)).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock => Error::Locked,
            _ => Error::Io(e),
        })?;
        let path = dir.join(DATA_FILE);
        let file =
            if vfs.exists(&path)? { vfs.open(&path, OpenMode::Existing)? } else { create_data_file(&*vfs, dir)? };
        // On every open, not only right after creating the file: an earlier
        // open may have renamed it into place and then failed before syncing
        // the directory, and nothing may be acknowledged on top of a file
        // whose directory entry is not durable.
        vfs.sync_dir(dir)?;
        let len = file.len()?;
        if len < 2 * PAGE_SIZE as u64 {
            return Err(Error::corruption(format!("data file is {len} bytes, shorter than its two meta pages")));
        }
        let mut metas = vec![0u8; 2 * PAGE_SIZE];
        file.read_at(&mut metas, 0)?;
        let meta::Selected { meta, slot: meta_slot, both: both_slots_current } =
            meta::select(&metas[..PAGE_SIZE], &metas[PAGE_SIZE..])?;
        // The selected meta may be visible but not durable (its process died
        // between writing it and syncing it, or that sync failed). Everything
        // below treats it as durable: the pages it freed — the previous tree,
        // the only durable one in that case — become reusable. Make it so.
        file.sync_data()?;
        if len < meta.page_count.saturating_mul(PAGE_SIZE as u64) {
            return Err(Error::corruption(format!(
                "data file is {len} bytes but commit {} uses {} pages (truncated?)",
                meta.seq, meta.page_count
            )));
        }
        let free = FreeState::load(&*file, &meta)?;
        let cache_bytes = opts.cache_bytes.max(16 * PAGE_SIZE) as u64;
        let inner = Arc::new(Inner {
            file,
            cache: Cache::with_weighter((cache_bytes as usize / PAGE_SIZE).max(16), cache_bytes, CacheWeighter),
            published: Mutex::new(Published { view: View::of(&meta), readers: BTreeMap::new() }),
            counters: Counters::default(),
            _lock: lock,
        });
        if let Some(root) = meta.root {
            // Fail fast on a damaged root.
            inner.read_node(root, meta.height - 1, meta.page_count)?;
        }
        debug_assert!(meta.page_count >= FIRST_DATA_PAGE);
        Ok(BTreeEngine {
            inner,
            writer: Mutex::new(Writer {
                meta,
                free,
                durability: opts.durability,
                poisoned: None,
                unsynced: false,
                meta_slot,
                both_slots_current,
            }),
        })
    }

    fn commit(&self, batch: &WriteBatch) -> Result<u64> {
        if batch.has_range_deletes() {
            return Err(Error::InvalidArgument("delete_range not supported by this prototype".into()));
        }
        batch.validate()?;
        let mut w = self.writer.lock();
        if let Some(m) = &w.poisoned {
            return Err(Error::Poisoned(m.clone()));
        }
        if batch.is_empty() {
            return Ok(w.meta.seq);
        }
        // Poisoned until proven otherwise: if `commit_locked` panics, the
        // allocator state may be ahead of the meta and the handle must not
        // be used again (parking_lot mutexes do not poison themselves).
        let seq = w.meta.seq;
        w.poisoned = Some(format!("commit {} panicked", seq.wrapping_add(1)));
        let result = self.commit_locked(&mut w, batch);
        w.poisoned = match &result {
            Ok(_) => None,
            // The in-memory allocator state and possibly the file are now
            // ahead of the last meta; only reopening (recovery) is safe.
            Err(e) => Some(format!("commit {} failed: {e}", seq.wrapping_add(1))),
        };
        result
    }

    fn snapshot(&self) -> Result<BTreeSnapshot> {
        let view = self.inner.snapshot_view();
        Ok(BTreeSnapshot { inner: self.inner.clone(), view })
    }

    fn close(self) -> Result<()> {
        let mut w = self.writer.lock();
        if let Some(m) = &w.poisoned {
            return Err(Error::Poisoned(m.clone()));
        }
        w.poisoned = Some("close panicked".into());
        let result = self.close_locked(&mut w);
        w.poisoned = result.as_ref().err().map(|e| format!("close failed: {e}"));
        result
    }

    fn stats(&self) -> Vec<(String, u64)> {
        let w = self.writer.lock();
        let c = &self.inner.counters;
        let snapshots = self.inner.published.lock().readers.values().sum::<usize>() as u64;
        let get = |a: &AtomicU64| a.load(Relaxed);
        [
            ("seq", w.meta.seq),
            ("entries", w.meta.entries),
            ("height", u64::from(w.meta.height)),
            ("pages_total", w.meta.page_count),
            ("file_bytes", w.meta.page_count * PAGE_SIZE as u64),
            ("free_pages", w.free.total_free()),
            ("free_pages_reusable", w.free.reusable.len() as u64),
            ("free_pages_pending", w.free.pending_count),
            ("freelist_log_pages", w.free.log.pages.len() as u64),
            ("live_snapshots", snapshots),
            ("commits", get(&c.commits)),
            ("tree_pages_written", get(&c.tree_pages_written)),
            ("overflow_pages_written", get(&c.overflow_pages_written)),
            ("freelist_pages_written", get(&c.freelist_pages_written)),
            ("freelist_compactions", get(&c.freelist_compactions)),
            ("fsyncs", get(&c.fsyncs)),
            ("overflow_pages_read", get(&c.overflow_pages_read)),
            ("cache_hits", get(&c.cache_hits)),
            ("cache_misses", get(&c.cache_misses)),
            ("cache_bytes", self.inner.cache.weight()),
            ("cache_entries", self.inner.cache.len() as u64),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }
}
