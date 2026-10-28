//! Free-page management.
//!
//! In memory the writer keeps two sets:
//!
//! * `reusable` — free pages the next commit may overwrite;
//! * `pending[n]` — pages freed by commit `n`. They are still reachable from
//!   the meta of commit `n - 1` (the other meta slot, until commit `n + 1`
//!   overwrites it) and from snapshots older than `n`, so they become
//!   reusable only for commits after `n`, once no live snapshot has a
//!   sequence below `n` ([`FreeState::release`]).
//!
//! On disk the *union* of both sets (everything below the high-water mark
//! that the committed tree does not use) is persisted as a log: a chain of
//! free-list pages, newest first, each holding entries "page joined the free
//! set" / "page left the free set" (bit 63). The meta points at the newest
//! page with its crc and every page stores the id and crc of the next (older)
//! one, so the whole chain is covered by checksums from the meta. Replaying
//! the chain from oldest to newest rebuilds the set. A commit appends its
//! changes by rewriting the head page (copy-on-write) while it has room, or
//! by pushing new pages; when the log grows well past the size of the set
//! it is compacted into a fresh chain that lists the set directly.

use std::collections::{BTreeMap, BTreeSet};

use spike_core::{Error, File, Result};

use crate::engine::read_page;
use crate::meta::Meta;
use crate::page::{
    FREELIST_ALLOC_BIT, FREELIST_CAPACITY, PageId, PageRef, check_page_id, decode_freelist_page, encode_freelist_page,
};

/// The log is compacted once it has this many pages more than twice the
/// minimum needed to list the free set.
const COMPACT_SLACK: u64 = 8;

/// Pages produced by one commit (page id, full page image).
pub(crate) type PageWrites = Vec<(PageId, Vec<u8>)>;

#[derive(Debug, Default)]
pub(crate) struct Log {
    /// Page ids of the chain, oldest first; the last one is the head.
    pub pages: Vec<PageId>,
    pub head: Option<PageRef>,
    /// The head page's pointer to the next (older) page.
    pub head_next: Option<PageRef>,
    /// The head page's entries; the next commit appends to a copy of them.
    pub head_entries: Vec<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct LogStats {
    pub pages_written: u64,
    pub compacted: bool,
}

#[derive(Debug)]
pub(crate) struct FreeState {
    pub reusable: BTreeSet<PageId>,
    pub pending: BTreeMap<u64, Vec<PageId>>,
    pub pending_count: u64,
    /// High-water mark: the next page id taken when nothing is reusable.
    pub page_count: u64,
    pub log: Log,
}

impl FreeState {
    /// Rebuilds the free set of `meta` by replaying its free-list log.
    /// After open there are no snapshots, so every free page is reusable.
    pub(crate) fn load(file: &dyn File, meta: &Meta) -> Result<FreeState> {
        let corrupt = |what: String| Error::corruption(format!("free list: {what}"));
        // Walk newest → oldest; every hop is verified against the crc stored
        // by the previous hop, and the length is bounded by the meta.
        let mut chain: Vec<(PageId, Option<PageRef>, Vec<u64>)> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cur = meta.free_head;
        while let Some(r) = cur {
            if chain.len() as u64 >= meta.free_log_pages {
                return Err(corrupt("chain is longer than recorded in the meta page".into()));
            }
            check_page_id(r.id, meta.page_count, "free-list pointer")?;
            if !seen.insert(r.id) {
                return Err(corrupt(format!("page {} appears twice in the chain", r.id)));
            }
            let page = read_page(file, r.id)?;
            if crc32c::crc32c(&page) != r.crc {
                return Err(corrupt(format!("page {} checksum mismatch", r.id)));
            }
            let (next, entries) = decode_freelist_page(&page, r.id)?;
            chain.push((r.id, next, entries));
            cur = next;
        }
        if chain.len() as u64 != meta.free_log_pages {
            return Err(corrupt("chain is shorter than recorded in the meta page".into()));
        }
        let mut set = BTreeSet::new();
        for (_, _, entries) in chain.iter().rev() {
            for &e in entries {
                let id = e & !FREELIST_ALLOC_BIT;
                check_page_id(id, meta.page_count, "free-list entry")?;
                let ok = if e & FREELIST_ALLOC_BIT != 0 { set.remove(&id) } else { set.insert(id) };
                if !ok {
                    return Err(corrupt(format!("inconsistent log entry for page {id}")));
                }
            }
        }
        if let Some(id) = seen.iter().find(|id| set.contains(id)) {
            return Err(corrupt(format!("log page {id} is listed as free")));
        }
        if set.len() as u64 != meta.free_count {
            return Err(corrupt(format!("{} free pages, meta records {}", set.len(), meta.free_count)));
        }
        let log = match chain.first() {
            None => Log::default(),
            Some((_, next, entries)) => Log {
                pages: chain.iter().rev().map(|(id, _, _)| *id).collect(),
                head: meta.free_head,
                head_next: *next,
                head_entries: entries.clone(),
            },
        };
        Ok(FreeState { reusable: set, pending: BTreeMap::new(), pending_count: 0, page_count: meta.page_count, log })
    }

    /// Size of the persisted free set (reusable and pending pages).
    pub(crate) fn total_free(&self) -> u64 {
        self.reusable.len() as u64 + self.pending_count
    }

    /// Every free page (reusable or pending).
    pub(crate) fn all_free(&self) -> BTreeSet<PageId> {
        self.reusable.iter().chain(self.pending.values().flatten()).copied().collect()
    }

    /// Makes the pages freed by commits `<= upto` reusable. Called at the
    /// start of a commit with the oldest live snapshot's sequence.
    pub(crate) fn release(&mut self, upto: u64) {
        while let Some(entry) = self.pending.first_entry() {
            if *entry.key() > upto {
                break;
            }
            let ids = entry.remove();
            self.pending_count -= ids.len() as u64;
            self.reusable.extend(ids);
        }
    }

    /// Takes the lowest reusable page (recording it in `allocated`) or
    /// extends the file.
    pub(crate) fn alloc(&mut self, allocated: &mut Vec<PageId>) -> PageId {
        if let Some(id) = self.reusable.pop_first() {
            allocated.push(id);
            id
        } else {
            let id = self.page_count;
            self.page_count += 1;
            id
        }
    }

    fn add_pending(&mut self, seq: u64, freed: Vec<PageId>) {
        if !freed.is_empty() {
            self.pending_count += freed.len() as u64;
            self.pending.entry(seq).or_default().extend(freed);
        }
    }

    /// Records commit `seq` in the free-list log: `allocated` are the pages it
    /// took from the reusable set, `freed` the pages its tree no longer uses.
    /// Appends the log pages to write to `writes` and moves `freed` (plus any
    /// log pages it replaces) to `pending[seq]`.
    pub(crate) fn commit(
        &mut self,
        seq: u64,
        mut allocated: Vec<PageId>,
        mut freed: Vec<PageId>,
        writes: &mut PageWrites,
    ) -> LogStats {
        let cap = FREELIST_CAPACITY;
        let min_pages = (self.total_free() + freed.len() as u64).div_ceil(cap as u64);
        if self.log.pages.len() as u64 > COMPACT_SLACK + 2 * min_pages {
            return self.compact(seq, freed, writes);
        }
        let changes = allocated.len() + freed.len();
        if changes == 0 {
            return LogStats::default();
        }
        // Copy-on-write of the head page with the new entries appended (two
        // extra entries: allocating the new head, freeing the old one).
        if let Some(old) = self.log.head
            && self.log.head_entries.len() + changes + 2 <= cap
        {
            let id = self.alloc(&mut allocated);
            freed.push(old.id);
            let mut entries = std::mem::take(&mut self.log.head_entries);
            push_entries(&mut entries, &allocated, &freed);
            let page = encode_freelist_page(id, self.log.head_next, &entries);
            self.log.head = Some(PageRef { id, crc: crc32c::crc32c(&page) });
            *self.log.pages.last_mut().expect("log has a head page") = id;
            self.log.head_entries = entries;
            writes.push((id, page));
            self.add_pending(seq, freed);
            return LogStats { pages_written: 1, compacted: false };
        }
        // Otherwise push new pages; their own allocations need entries too.
        let mut n = changes.div_ceil(cap);
        while changes + n > n * cap {
            n += 1;
        }
        let ids: Vec<PageId> = (0..n).map(|_| self.alloc(&mut allocated)).collect();
        let mut entries = Vec::with_capacity(changes + n);
        push_entries(&mut entries, &allocated, &freed);
        self.push_pages(&ids, &entries, writes);
        self.add_pending(seq, freed);
        LogStats { pages_written: n as u64, compacted: false }
    }

    /// Replaces the log with a fresh chain listing the free set directly.
    fn compact(&mut self, seq: u64, mut freed: Vec<PageId>, writes: &mut PageWrites) -> LogStats {
        // The old chain is reachable from the previous meta: pending, not
        // reusable.
        freed.append(&mut self.log.pages);
        self.log = Log::default();
        let n = (self.total_free() + freed.len() as u64).div_ceil(FREELIST_CAPACITY as u64) as usize;
        let mut ignored = Vec::new();
        let ids: Vec<PageId> = (0..n).map(|_| self.alloc(&mut ignored)).collect();
        self.add_pending(seq, freed);
        let mut all: Vec<u64> = self.all_free().into_iter().collect();
        all.sort_unstable();
        self.push_pages(&ids, &all, writes);
        LogStats { pages_written: n as u64, compacted: true }
    }

    /// Writes `entries` into the pages `ids` (in order, `FREELIST_CAPACITY`
    /// per page, possibly leaving trailing pages empty) on top of the head.
    fn push_pages(&mut self, ids: &[PageId], entries: &[u64], writes: &mut PageWrites) {
        let cap = FREELIST_CAPACITY;
        assert!(entries.len() <= ids.len() * cap, "free-list pages too small for their entries");
        for (i, &id) in ids.iter().enumerate() {
            let chunk = &entries[(i * cap).min(entries.len())..((i + 1) * cap).min(entries.len())];
            let page = encode_freelist_page(id, self.log.head, chunk);
            self.log.head_next = self.log.head;
            self.log.head = Some(PageRef { id, crc: crc32c::crc32c(&page) });
            self.log.head_entries = chunk.to_vec();
            self.log.pages.push(id);
            writes.push((id, page));
        }
    }
}

fn push_entries(entries: &mut Vec<u64>, allocated: &[PageId], freed: &[PageId]) {
    entries.extend(allocated.iter().map(|id| id | FREELIST_ALLOC_BIT));
    entries.extend_from_slice(freed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::PAGE_SIZE;
    use spike_core::vfs::{OpenMode, SimVfs, Vfs};
    use std::path::Path;

    /// Applies `writes` to a file and returns the meta describing `fs`.
    fn persist(file: &dyn File, fs: &FreeState, writes: &PageWrites) -> Meta {
        for (id, page) in writes {
            file.write_at(page, id * PAGE_SIZE as u64).unwrap();
        }
        Meta {
            page_count: fs.page_count,
            free_head: fs.log.head,
            free_log_pages: fs.log.pages.len() as u64,
            free_count: fs.total_free(),
            ..Meta::empty()
        }
    }

    #[test]
    fn log_replay_matches_memory_through_appends_pushes_and_compactions() {
        let vfs = SimVfs::new(1);
        vfs.create_dir_all(Path::new("/d")).unwrap();
        let file = vfs.open(Path::new("/d/f"), OpenMode::CreateNew).unwrap();
        file.write_at(&[0u8; 2 * PAGE_SIZE], 0).unwrap();
        let mut fs = FreeState::load(&*file, &Meta::empty()).unwrap();
        let mut rng = spike_core::rng::Rng::new(5);
        // Pages "in use" by an imaginary tree.
        let mut used: Vec<PageId> = Vec::new();
        let mut compactions = 0;
        for seq in 1..=3000u64 {
            fs.release(seq - 1);
            let mut allocated = Vec::new();
            let grow = rng.below(if seq % 500 < 250 { 40 } else { 3 }) as usize;
            for _ in 0..grow {
                used.push(fs.alloc(&mut allocated));
            }
            let mut freed = Vec::new();
            for _ in 0..rng.below(20) {
                if used.is_empty() {
                    break;
                }
                let i = rng.below(used.len() as u64) as usize;
                freed.push(used.swap_remove(i));
            }
            let mut writes = Vec::new();
            let st = fs.commit(seq, allocated, freed, &mut writes);
            compactions += st.compacted as u32;
            let meta = persist(&*file, &fs, &writes);
            if seq % 97 == 0 || st.compacted {
                let loaded = FreeState::load(&*file, &meta).unwrap();
                assert_eq!(loaded.reusable, fs.all_free(), "seq {seq}");
                assert_eq!(loaded.log.pages, fs.log.pages);
                assert_eq!(loaded.log.head_entries, fs.log.head_entries);
                // Every page below the high-water mark is exactly one of:
                // meta, used, log page or free.
                let mut owners = vec![0u8; fs.page_count as usize];
                owners[0] = 1;
                owners[1] = 1;
                for &p in used.iter().chain(&fs.log.pages).chain(&loaded.reusable) {
                    owners[p as usize] += 1;
                }
                assert!(owners.iter().all(|&o| o == 1), "seq {seq}: page accounting broken");
            }
            assert!(fs.log.pages.len() as u64 <= COMPACT_SLACK + 2 * fs.total_free().div_ceil(508) + 4);
        }
        assert!(compactions > 0, "the test must exercise compaction");
    }

    #[test]
    fn load_rejects_inconsistent_logs() {
        let vfs = SimVfs::new(1);
        vfs.create_dir_all(Path::new("/d")).unwrap();
        let file = vfs.open(Path::new("/d/f"), OpenMode::CreateNew).unwrap();
        file.write_at(&[0u8; 8 * PAGE_SIZE], 0).unwrap();
        let write = |id: PageId, entries: &[u64], next: Option<PageRef>| {
            let p = encode_freelist_page(id, next, entries);
            file.write_at(&p, id * PAGE_SIZE as u64).unwrap();
            PageRef { id, crc: crc32c::crc32c(&p) }
        };
        let meta = |head, pages, count| Meta {
            page_count: 8,
            free_head: Some(head),
            free_log_pages: pages,
            free_count: count,
            ..Meta::empty()
        };
        let h = write(2, &[3, 4], None);
        assert_eq!(FreeState::load(&*file, &meta(h, 1, 2)).unwrap().reusable.len(), 2);
        assert!(FreeState::load(&*file, &meta(h, 1, 3)).is_err(), "count mismatch");
        assert!(FreeState::load(&*file, &meta(h, 2, 2)).is_err(), "chain shorter than recorded");
        let bad_crc = PageRef { id: 2, crc: h.crc ^ 1 };
        assert!(FreeState::load(&*file, &meta(bad_crc, 1, 2)).is_err(), "checksum");
        let h = write(2, &[3, 3], None);
        assert!(FreeState::load(&*file, &meta(h, 1, 1)).is_err(), "double free");
        let h = write(2, &[3 | FREELIST_ALLOC_BIT], None);
        assert!(FreeState::load(&*file, &meta(h, 1, 0)).is_err(), "allocating a page that is not free");
        let h = write(2, &[2], None);
        assert!(FreeState::load(&*file, &meta(h, 1, 1)).is_err(), "log page listed as free");
        let h = write(2, &[9], None);
        assert!(FreeState::load(&*file, &meta(h, 1, 1)).is_err(), "entry beyond the high-water mark");
        // A two-page chain replays oldest first: freed in the older page,
        // allocated in the newer one.
        let older = write(3, &[5, 6], None);
        let newer = write(2, &[5 | FREELIST_ALLOC_BIT, 7], Some(older));
        let fs = FreeState::load(&*file, &meta(newer, 2, 2)).unwrap();
        assert_eq!(fs.reusable.into_iter().collect::<Vec<_>>(), vec![6, 7]);
        assert_eq!(fs.log.pages, vec![3, 2]);
    }
}
