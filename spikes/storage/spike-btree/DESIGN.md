# spike-btree: copy-on-write B+ tree candidate

Phase 0 spike for Kerno. This is an LMDB/redb-style shadow-paging B+ tree
implementing the `spike_core::Engine` contract. It is a prototype: the format
is not stable and favours clarity over speed. All file I/O goes through
`spike_core::Vfs`.

## Files

| file | purpose |
|------|---------|
| `data.kbt` | the database: an array of 4 KiB pages; page `n` is at byte `n * 4096` |
| `data.kbt.tmp` | exists only while a new database is being created |
| `LOCK` | `Vfs::lock` target; `WouldBlock` maps to `Error::Locked` |

All integers are little-endian. "crc" always means crc32c.

## Page types

### Meta pages (ids 0 and 1)

There are two double-buffered slots. Each slot holds **two copies** of the
92-byte meta record, at offsets `0` and `2048` (different sectors); a slot
decodes to its newest valid copy. Damage to one copy of the newest meta (bit
rot after an unclean shutdown, when the other slot holds the previous commit)
therefore no longer rolls the database back. A torn meta write can leave one
new and one old copy; using the new one is safe because a meta is only written
after the pages it references are durable.

| bytes | field |
|-------|-------|
| 0..8 | magic `KRNOBTRE` |
| 8..12 | format version (1) |
| 12..16 | page size (4096) |
| 16..24 | commit sequence |
| 24..32 / 32..36 | root page id / crc of the root page (0/0 = empty tree) |
| 36..40 | tree height (0 empty, 1 = root is a leaf) |
| 40..48 | page count (high-water mark; every id below it has been written) |
| 48..56 / 56..60 | newest free-list log page id / its crc (0/0 = none) |
| 60..64 | reserved, must be 0 |
| 64..72 | number of free-list log pages (bounds the chain walk) |
| 72..80 | number of free pages (checked after replay) |
| 80..88 | number of keys (statistics) |
| 88..92 | crc of bytes 0..88 |

A slot is valid when its magic, crc, version and page size match and its
fields are mutually consistent (for example, a root is present exactly when
the height is non-zero, and ids are below the page count).

### Node pages (leaf type 1, branch type 2)

Header (16 bytes): `type u8 | level u8 | count u16 | reserved u32 (0) | own page id u64`.
Storing the page's own id catches misdirected writes.

* **Leaf** (level 0, `count` ≥ 1 entries, keys strictly increasing). Each entry
  is `key_len u16 | kind u8 | value_len u32 | key`, followed by:
  * kind 0 (inline): the value bytes;
  * kind 1 (overflow): `chain crc u32 | first overflow page u64`.

  A value is inline when its whole entry (7 + key + value) is at most 1020
  bytes, which is a quarter of the payload area. Any leaf therefore holds at
  least 4 entries. The largest overflow entry is 7 + 512 + 12 = 531 bytes.
* **Branch** (level ≥ 1): `child0 (id u64, crc u32)`, then `count` repetitions
  of `key_len u16 | separator | child (id u64, crc u32)`. Child `i` holds the
  keys in `[sep[i-1], sep[i])`. Separators are full keys: the first key of the
  right-hand node at split time. At least 7 maximal (512-byte) separators fit
  in a branch.

### Overflow pages (type 3)

Header: `type u8 | 7 reserved bytes | next page id u64` (0 = end of chain),
followed by 4080 payload bytes. The leaf entry stores the value length and a
crc computed over the full 4 KiB images of all pages in the chain, in order.
That crc covers payload, next pointers and padding. A read walks exactly
`ceil(len / 4080)` pages. Every pointer is range-checked, the last page must
have `next == 0` and the others must not, and the chain crc is checked at the
end. A corrupt pointer can therefore neither loop nor be accepted. Chains are
immutable: CoW of a leaf shares them, and overwriting or deleting the value
frees them. That requires walking and verifying the chain to learn its page ids.

### Free-list log pages (type 4)

Header (32 bytes):
`type u8 | 3 reserved | count u32 | own id u64 | next id u64 | next crc u32 | reserved u32`,
followed by up to 508 `u64` entries. An entry with bit 63 clear means "page
joined the free set" and with bit 63 set "page left the free set". The meta
points at the newest page (id + crc) and each page points at the next older
one (id + crc), so the whole chain is crc-linked from the meta. Replaying from
oldest to newest rebuilds the persisted free set. The replay rejects
inconsistent entries such as a double free, freeing a log page, or an id at or
above the page count. It also checks the set size against the meta.

## Commit protocol

1. Validate the batch, take the writer mutex, then refuse with `Poisoned` if
   the handle is poisoned. An empty batch returns the current sequence.
2. Reclaim: move pages freed by commits `<= oldest live snapshot` (or all of
   them if there are no snapshots) from *pending* to *reusable*.
3. Apply. Deduplicate and sort the batch (last op per key wins), then apply it
   in one top-down pass. Each touched node is copied out of its page once into
   an owned in-memory node, and its old page id is recorded as freed. The
   sorted operations are partitioned among children by separator. Each level
   then fixes its children:
   * drops children that became empty;
   * merges an underfull child (< 1/3 page) with its right or left sibling if
     the merged node fits a page, repeating while it stays underfull;
   * splits oversized nodes into as many balanced pieces as needed, so a large
     batch can split one node many ways.

   The root grows while there is more than one piece, and collapses while it
   is a branch with a single child. Values over the inline threshold become
   new overflow chains. Overflow chains of overwritten or deleted values are
   freed.
4. Write-out, bottom-up. Children get page ids before their parents, so each
   parent embeds its children's crcs. Ids come from the lowest reusable page,
   or else from the high-water mark. The free-list log is updated in one of
   three ways:
   * the head page is rewritten (CoW) with this commit's entries appended,
     when they fit;
   * new pages are pushed on top of the chain;
   * the whole log is compacted into a fresh chain listing the free set,
     when it has grown past `8 + 2 × minimum pages`.
5. I/O (`Durability::Sync`): write all pages, sorted and coalesced into one
   `write_at` per run of consecutive ids, then `sync_data`. Then write the
   meta into the slot that does **not** hold the current meta, then
   `sync_data`.
6. Publish: evict the replaced node versions from the cache, insert the new
   nodes (decoded from their own page images), and swap the published root
   under a short mutex.

**fsyncs:** a Sync commit costs 2 `sync_data` and no `sync_dir`. A Buffered
commit costs 0. `open` costs 2 `sync_dir` (the parent directory, so a newly
created database directory is durable, and the database directory) and 1
`sync_data` (see Recovery), plus 1 more `sync_data` when it creates the file. `close` costs 1 in Sync mode and 2 in Buffered mode (first
sync the outstanding writes, then write and sync the second meta copy). A
single-key update in a 3-level tree writes 3 node pages, 1 free-list page and
the meta: 5 × 4 KiB.

Any error after step 1 poisons the handle. That commit returns the error, and
every later `commit` or `close` returns `Error::Poisoned` until the database
is reopened. The handle is marked poisoned *before* the commit runs and
cleared on success, so a panic inside a commit also poisons it (parking_lot
mutexes do not poison themselves). A commit sequence at `u64::MAX` is an
error, never a wrap-around. Dropping an engine does no I/O. Drop behaves like a process exit,
and recovery handles it.

**Creation** is atomic: both metas (sequence 0) are written to
`data.kbt.tmp`, then `sync_data`, then `rename` to `data.kbt`. Every open then
runs `sync_dir`, not only the one that created the file: an earlier open may
have renamed the file and then failed before its directory sync, and nothing
may be acknowledged on top of a directory entry that is not durable. An
existing `data.kbt` therefore always held valid metas at some point. When a
data file is empty, truncated or has no valid meta, open reports
`Error::Corruption` and never re-initializes it.

**Clean close** syncs outstanding buffered writes, then writes the current meta
into the other slot as well and syncs. After a clean close both slots are
identical, so damage to either slot cannot roll the database back.

## Recovery (open)

1. Take the lock. If `data.kbt` is missing, create it as described above,
   removing a leftover `.tmp` first. Then `sync_dir`.
2. If the file is shorter than two pages, report corruption.
3. Read both metas and use the valid one with the highest sequence. If neither
   is valid, report corruption. The next commit writes the other slot, which
   also heals a damaged slot.
4. `sync_data` the file. The selected meta may be visible but not durable (the
   previous process died between writing it and syncing it, or that sync
   failed). Everything after this point treats it as durable — the pages it
   freed, i.e. the previous tree, become reusable — so it must be. (Found by
   the M0 adversarial review: without this, a restart followed by a power loss
   during the next commit could destroy the only durable tree.)
5. If the file is shorter than `page_count × 4096`, report corruption (for
   example, after truncation).
6. Replay the free-list log, verifying every page's crc from the meta down,
   and check its consistency. Every free page is reusable, because no
   snapshots exist yet.
7. Read and verify the root page so that a damaged root fails fast.

No log needs replaying and nothing is written during recovery (only the
directory syncs and the data-file sync). Pages written
by a commit that never got its meta are simply free (below the high-water
mark) or beyond it, and get overwritten later.

## Page reclamation rules

* A commit never overwrites a page reachable from the current meta. In Sync
  mode that meta is durable, so it is the last durable meta. A commit also
  never overwrites a page reachable from a live snapshot.
* A page freed by commit `n` was reachable only from trees before `n`. It
  becomes reusable for commits after `n`, and only once the oldest live
  snapshot has sequence ≥ `n`. Until then it is *pending*.
* The persisted free set is "reusable ∪ pending", so reopening never leaks
  pages (`verify()` checks that every page below the high-water mark is
  exactly one of: meta, tree node, overflow page, log page, or free).
* The snapshot registry is a `BTreeMap<seq, count>` guarded by the same short
  mutex as the published root. A snapshot reads the root and registers
  atomically, so the writer can never release pages a just-taken snapshot
  needs. A snapshot also holds the directory lock (through `Arc<Inner>`), so a
  snapshot that outlives `close()` keeps other handles out instead of letting
  them overwrite its pages.

## Integrity coverage

| bytes | covered by |
|-------|------------|
| meta fields (each copy) | meta crc (bytes 0..88 of the copy) |
| root page (all 4096 bytes) | crc in the meta |
| every other tree page | crc in its parent's child entry |
| overflow chains (all bytes of all pages) | chain crc + length in the leaf entry |
| free-list log pages | crc in the meta, then in the previous log page |
| rest of a meta page (outside the two 92-byte copies), the second copy while the first is valid, free pages | never read; damage is harmless |

Crcs are verified on every read from the file, which in practice means every
cache miss (the cache key includes the crc). Decoding also validates lengths, ordering,
levels and page-id ranges, so a crc collision still cannot cause a panic or an
out-of-bounds read. The shared bit-flip suite (400 seeds, half of the flips aimed at
headers and metas) gave 28 flips detected at open, 191 detected on read and
181 harmless, with no wrong data and no rollback (1000 seeds: 79 / 451 / 470).
The truncation suite gave 400/400 detected at open.

## Cache

The cache is one `quick_cache::sync::Cache` with capacity `Options::cache_bytes`
holding two kinds of entries: decoded nodes, keyed by `(page id, crc)` and
weighted at about 4 KiB plus the entry index; and assembled overflow values,
keyed by `(first page id, chain crc)` and weighted by their length. (Before the
M0 review, overflow values were never cached and every read of a value over
~1 KiB issued one `pread` per 4 KiB page.) Decoded nodes share their page image: keys and inline
values are slices of it, so copying a node for CoW only bumps reference counts.
Freshly written nodes are inserted at commit, and the versions they replace are
evicted. Without eviction the stale versions fill the hot section and the new
ones get evicted from the small cold queue before their next use; measured on
random updates, that took the hit rate from 100% to 8%.

## Durability modes

* **Sync**: a commit is durable when `commit` returns. Power loss at any point
  recovers to the last acknowledged commit or the one in flight. A failed
  write or `sync_data` is never acknowledged: it poisons the handle, and
  nothing freed by the failed commit is ever reused by that handle.
* **Buffered**: no fsyncs. Process crashes recover to the last acknowledged
  commit or the one in flight. **Power loss does not guarantee a consistent
  prefix.** Without a barrier, a newer meta can reach the disk before the
  pages it references, and pages freed by earlier commits are reused before
  anything is durable. Thanks to the Merkle crcs the outcome is still the
  exact state of some commit, or a detected `Corruption`, never wrong data
  (`tests::buffered_power_loss_gives_a_prefix_or_a_detected_error`). In that
  test's 150 simulated crashes, though, only 2 recovered a consistent state;
  130 were detected at open (mostly a newer meta whose free-list page or file
  extension never landed) and 18 on read. In Buffered mode the only
  durability point is a clean `close`. The shared suite is registered with
  `buffered_power_loss_prefix = false`. A real guarantee would need periodic
  sync points: defer page reuse until the next sync point, keep the last
  synced meta in its slot until then, and at open verify the newer meta's
  tree, skipping subtrees written before the sync point (that needs a "written
  at" sequence in page headers), before falling back.

## Known limitations

* Two fsyncs per Sync commit (page barrier and meta). There is no group
  commit: concurrent committers queue on the writer mutex and each pays its
  own fsyncs.
* Write amplification is that of a CoW tree. Every commit rewrites the whole
  root-to-leaf path of every touched leaf, plus one free-list page and the
  meta. A batch of N random keys touches about N leaves.
* A commit that changes nothing (for example, deleting absent keys) still
  copies the path and gets a new sequence number.
* Overflow chains are read page by page on a cache miss (one `read_at` per
  4 KiB). Freeing a chain reads all of it. Chains are not allocated
  contiguously.
* Separators are full keys (no suffix truncation or prefix compression), and
  nodes are decoded into an index of slices rather than searched in place.
* Underfull nodes are only merged, never rebalanced. After heavy deletes,
  leaves can stay at about 1/3 fill.
* The file never shrinks. Free pages are reused lowest-id first, but trailing
  free pages are not truncated. Long-lived snapshots pin every page freed
  since they were taken.
* After an *unclean* shutdown the two slots hold the last two commits. If
  *both* copies in the newest slot are then damaged by media corruption, open
  falls back to the previous commit, whose pages may already have been reused
  (reads then report corruption). One damaged copy is harmless.
* Reads of a deliberately crafted file whose crcs are all valid (keys outside
  their separator bounds) can miss keys; crc32c is not an authentication code
  (threat model T-04). Writes on such a file fail with `Corruption`.
* Cache keys are `(page id, crc32c)`. A reused page id whose new content has
  the same crc as a still-cached old version would alias, a 2^-32 event per
  reuse. Evicting replaced versions at commit makes it rarer still.
* The format has no page-level checksums of its own (crcs live in the parent),
  so a stray page cannot be validated without the path that reaches it. That
  is fine for this engine, but it rules out offline scrubbing of free pages.

## What would change for a production version

* A single fsync per commit: either "1PC + checksum", where recovery verifies
  the pages written since the previous meta (needs a per-page "written at"
  sequence), or a small WAL for small transactions, plus group commit.
* The buffered-mode sync-point scheme above, giving a consistent prefix on
  power loss.
* An overflow format with an extent list (or a contiguous allocation) in the
  first page, so values can be read in one I/O and freed without reading them.
* Searching nodes in place in the page buffer (slotted pages) instead of
  decoding them, plus prefix compression and suffix-truncated separators.
* A free-page B+ tree or bitmap with extent allocation instead of the log, and
  tail truncation and online compaction to shrink the file.
* Per-page checksums in page headers (in addition to the Merkle crcs) for
  scrubbing, and possibly xxh3/BLAKE3 instead of crc32c for stronger
  collision resistance.
* Multiple named trees (tables and indexes) under one meta, which Kerno's
  document and index layers need.
