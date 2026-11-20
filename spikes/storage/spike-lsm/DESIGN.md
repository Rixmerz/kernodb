# spike-lsm: leveled LSM tree (Kerno Phase 0 candidate)

A LevelDB/RocksDB-style leveled LSM tree behind the `spike_core::Engine`
contract. It is a prototype: it aims to be correct under crashes and media
damage and reasonably fast, not tuned. All file I/O goes through
`Arc<dyn Vfs>`.

## Directory layout

| File           | Purpose |
|----------------|---------|
| `LOCK`         | `Vfs::lock` (exclusive open, `Error::Locked`) |
| `MANIFEST`     | checksummed snapshot of the LSM shape, replaced atomically |
| `MANIFEST.tmp` | exists only while a new MANIFEST is being written |
| `NNNNNN.wal`   | write-ahead log, one per memtable |
| `NNNNNN.sst`   | sorted string table |

WAL and SST numbers come from one increasing counter (`next_file`) and are
never reused. On open, files that match these patterns but are not live are
deleted: WALs older than the MANIFEST's `log_number`, SSTs it does not
reference, and `MANIFEST.tmp`. Only canonical names (`NNNNNN.wal`, six or more
digits exactly as written) match; files with other names, such as a stray
`1.wal`, are left alone. A file that is already gone when recovery tries to
delete it (a snapshot of a previous handle dropped it concurrently) is fine.

## Formats

All integers are little endian. `varint` is unsigned LEB128, and `bytes` is
a varint length followed by the data. Every checksum is crc32c.

**WAL record.** `len u32 | crc u32 | seq u64 | batch`. The crc covers
`len`, `seq` and `batch`. Bit 31 of `len` (`SYNCED`) marks a record written in
`Sync` mode; the body length is the low 31 bits, so a batch that encodes to
2 GiB or more is rejected with `InvalidArgument` before anything is written. The batch is `count varint` followed by
`tag u8 (1 put / 2 delete) | key bytes | [value bytes]` per op. Each record
goes out in a single `write_at`.

**SST.** Holds one version per user key, sorted:

```text
data block*   entries | u32 offset per entry | count u32 | crc u32     (~4 KiB target)
              entry = key_len varint | seq varint | kind u8 | [value_len varint] | key | value
filter block  bloom bits (10 bits/key, 7 probes) | k u8 | crc u32
index block   smallest_key bytes | count varint | (last_key bytes | offset varint | len varint)* | crc u32
footer        filter_off u64 | filter_len u64 | index_off u64 | index_len u64 | entries u64 | magic u64 | crc u32
```

There is no prefix compression and no block compression. Binary search
within a block uses the offset array.

**MANIFEST.** `"KRNOMAN1" | body_len u32 | crc u32 | body`, where body is
`format u32 | last_seq u64 | log_number u64 | next_file u64 | levels u32`.
For each level it stores the file count, and for each file
`number, size, entries, smallest, largest`. L0 is listed newest first; L1
and below are sorted and non-overlapping. Meaning of the fields:

- `last_seq`: the newest commit contained in the SSTs.
- `log_number`: WALs numbered `>= log_number` hold the commits after
  `last_seq`, in order.

Each MANIFEST update is `write MANIFEST.tmp → sync_data → rename → sync_dir`.

## Commit path

1. `WriteBatch::validate` runs, then the writer mutex is taken (single
   writer). A poisoned handle returns `Poisoned`, and an empty batch returns
   the current seq.
2. `make_room` checks the write buffer and applies backpressure (see below).
   If the memtable is over `write_buffer_bytes`, it switches memtables:
   - In Buffered mode, `sync_data` the old WAL first.
   - Create the new WAL, then `sync_dir`.
   - Publish a new `Version` in which the old memtable is immutable.
3. Encode the record with `seq = last + 1`, `write_at` it to the WAL, and in
   Sync mode `sync_data` it.
4. Insert into the memtable. This is a `crossbeam_skiplist::SkipMap` keyed
   by (user key ascending, seq descending), with tombstones. Duplicate keys
   in a batch collapse to the last op.
5. Publish `last_seq` under the state mutex. A snapshot takes
   `(Arc<Version>, last_seq)` under the same mutex, which makes capturing
   it atomic with respect to publication. Readers at an older seq skip
   newer skip-list entries.

**fsyncs per Sync commit: exactly one `sync_data` (the WAL).** Amortised on
top of that:

- One `sync_dir` per memtable switch, i.e. every `write_buffer_bytes`.
- In the background, per flush: SST `sync_data`, `sync_dir`, then MANIFEST
  `sync_data` + `sync_dir`.

There is no group commit: concurrent committers each pay their own sync.

**Backpressure:**

- With ≥ 8 L0 files, each commit waits up to 1 ms for compaction.
- With ≥ 4 immutable memtables or ≥ 12 L0 files, the commit blocks until a
  background thread makes progress.
- Stalls are counted in `stats()`.

## Background work

There are two threads: a **flusher** and a **compactor**. They share a
condvar with the writer. Edits to the MANIFEST are serialised by a manifest
mutex. Each holder computes the new levels from the last durable ones,
writes the MANIFEST, and then installs a new `Version`.

**Flush** (oldest immutable memtable first):

1. Write the newest version of every key to a new L0 SST, then `sync_data`
   and `sync_dir`.
2. Write a MANIFEST with the table at the front of L0,
   `last_seq = max(last_seq, mem.max_seq)` and
   `log_number = mem.wal_number + 1`.
3. Install the new version and delete the WAL. Deletion is best effort;
   recovery removes leftovers.

**Compaction** (leveled, like LevelDB):

- Score L0 as `files / 4`. Score Ln (n = 1..5) as
  `bytes / (4 × write_buffer × 10^(n−1))`. The highest score ≥ 1 wins.
- L0 compaction merges all L0 files and the overlapping L1 files into L1.
- Ln compaction takes one file, chosen round-robin by key, plus the
  overlapping Ln+1 files. A file with no overlap is moved down without
  rewriting it.
- Output files are split at `target_file_size = write_buffer_bytes`.
- The merge keeps only the newest version of each key. Older snapshots keep
  reading the input files through their own `Version`.
- A tombstone is dropped when no file in a level below the output level has
  a key range containing the key.
- Outputs are synced, followed by `sync_dir`, then the MANIFEST, then the
  version install. Only after that are the inputs marked obsolete.
- On shutdown, a running compaction is abandoned and its partial outputs
  are removed.
- If the MANIFEST write fails, nothing is deleted. A rename whose
  `sync_dir` failed may still survive, so the outputs might be referenced.
  The next recovery deletes whichever files the surviving MANIFEST does not
  reference.

## Versions, snapshots and file lifetime

`Version = { active memtable, immutable memtables (newest first), levels
of Arc<Table> }`. A new version is installed by every memtable switch,
flush and compaction. A snapshot holds `Arc<Version>` and its seq.

- **Get:** active memtable, then immutable memtables, then L0 newest
  first, then one candidate file per deeper level (binary search on
  `largest`). For each file it checks the key range, the bloom filter, then
  the index.
- **Scan:** a k-way merge (linear minimum over a small number of sources)
  that yields the newest visible version per key. Tombstones are skipped,
  all `Bound` combinations are handled, and empty or inverted ranges return
  nothing.

An SST is deleted from `Table::drop`, but only for tables that were marked
obsolete. A table is marked obsolete only after a durable MANIFEST stops
referencing it, so it disappears when the last `Version` or snapshot using
it is dropped. That drop can happen on any thread.

## Recovery (every open, including a fresh database)

1. Sync the parent directory (so a newly created database directory is
   durable), take the lock and list the directory. If the MANIFEST is present, read
   and verify it; corruption is reported, never treated as "start empty".
   If it is absent but WAL or SST files exist, that is `Corruption`. If the
   directory is empty, this is a new database.
2. Open every referenced SST. Check that it exists, then check its size,
   footer, index (layout and order), filter checksum, entry count and key
   range against the MANIFEST.
3. Replay WALs `>= log_number` in number order. Records must carry
   consecutive seqs starting at `last_seq + 1`:
   - A record that verifies but has the wrong seq or a malformed batch is
     `Corruption`.
   - Replay of a log stops at the first incomplete or crc-failing record.
     That is accepted only in the newest log; in an older log it is
     `Corruption`.
   - **Torn tail or media damage.** A log is written by one handle in one
     durability mode, and in `Sync` mode a record is only written after the
     previous one is durable. If an invalid record is followed anywhere in
     the file by a valid `SYNCED` record, the invalid one was durable and has
     been damaged since: `Corruption`, and nothing is deleted. (Found by the
     M0 adversarial review: before, such damage was truncated as a torn
     tail, silently dropping acknowledged commits and deleting the log.)
     Damage to the *last* record of a log is still indistinguishable from a
     torn in-flight commit, and in `Buffered` mode out-of-order writes make a
     valid record after an invalid one a normal power-loss outcome.
4. Write the replayed commits to a new L0 SST (synced, then `sync_dir`).
5. Write a MANIFEST whose `log_number` is a fresh WAL number, so every
   existing WAL becomes obsolete. Delete obsolete WALs and orphan SSTs,
   create the new WAL, then `sync_dir`.

Consequences:

- A torn tail is never followed by appends, because the old log is never
  written again.
- The state reported by `open` is durable before `open` returns.
- A crash at any step leaves either the old MANIFEST with all its WALs, or
  the new MANIFEST. Both recover to the same state.
- Compaction is scheduled right after open if levels are over target.

## Clean close

1. Stop and join the background threads. If the handle is poisoned,
   return `Poisoned`.
2. Flush every memtable to L0.
3. Write a MANIFEST with `last_seq` equal to the newest commit and a
   `log_number` above every WAL.
4. Delete all WALs, then `sync_dir`.
5. Run compactions in the calling thread until no level is over its target.
   (Found by the M0 adversarial review: before, close abandoned the running
   compaction and step 2 added one more L0 file, so many short sessions grew
   L0 without bound and every reopen started with a compaction backlog.)

After a clean close the directory holds only `LOCK`, `MANIFEST` and SSTs,
and every byte of them is checksummed. That is why a flipped bit or a
truncation yields an error rather than an older commit.

## Integrity coverage

| Data | Protection | Checked |
|------|------------|---------|
| WAL record | crc over len, seq, batch; seq continuity | replay |
| Data block | crc; structure (contiguous entries, increasing keys); last key = index key | every read from disk |
| Filter | crc + parameter check (a bad filter would hide keys) | open |
| Index | crc; blocks tile `[0, filter_off)`, increasing keys | open |
| Footer | crc + magic; layout consistency | open |
| SST vs MANIFEST | size, entry count, smallest/largest key | open |
| MANIFEST | magic, exact length, crc, structural invariants | open |

`Error::Corruption` is returned on any mismatch. Corruption found by a
background compaction poisons the handle.

## Durability modes

- **Sync:** a commit is durable when it returns.
- **Buffered:** no per-commit sync, but records are written with `write_at`
  before `commit` returns, so a process crash loses nothing acknowledged.
  On power loss the recovered state is a **prefix of commits**, never older
  than the state the last `open` reported. This holds because:
  - the old WAL is synced before a new one is created, so only the newest
    log can have holes, and replay stops at the first hole;
  - SST and MANIFEST are durable before any WAL they replace is deleted;
  - recovery makes its result durable before returning.

## Errors and poisoning

Any I/O error in the commit path (append, sync, memtable switch) or in a
background thread (flush, compaction, MANIFEST write) poisons the handle.
After that, `commit`, `snapshot` and `close` return `Poisoned` until the
database is reopened. Snapshots taken earlier keep working on whatever is
still readable.

`Drop` sets the shutdown flag, wakes all waiters and joins both threads,
which exit on shutdown, on poison, or on their first failing I/O. A panic
in a background thread is caught and poisons the handle.

## Stats (`Engine::stats`)

- **Shape:** `l{0..6}_files` / `l{0..6}_bytes`, `memtable_bytes`,
  `immutable_memtables`, `last_seq`, `block_cache_bytes`.
- **Write path:** commits, WAL bytes and syncs, memtable switches.
- **Background work:** flushes and flush bytes, compactions, trivial moves,
  compaction bytes read and written, tombstones dropped.
- **Backpressure:** write stalls (count, µs) and slowdowns.
- **Reads:** bloom checks, negatives and false positives; block cache hits
  and misses.
- **Housekeeping:** MANIFEST writes, obsolete files deleted.

## Known limitations / what would change for production

- **MANIFEST format.** It is a full snapshot rewritten on every flush and
  compaction, which costs O(files). Production would use an append-only
  edit log with periodic checkpoints. That needs a clean-shutdown marker or
  similar so that a damaged last edit cannot pass for a torn tail.
- **Writes.** There is no group commit, so concurrent Sync writers
  serialise one fsync each. There is also no WAL recycling, so each switch
  pays a create plus `sync_dir`.
- **Compaction.** One compaction thread, no subcompactions, and no
  rate limiting. L0→L1 always takes all of L0. There are no
  grandparent-overlap limits on output files, and no dynamic level sizing.
- **SST format.** No prefix compression, compression, partitioned
  index/filter or format versioning beyond magic numbers.
- **Memory and files.** Index and filter of every table stay resident and
  every table stays open, so memory grows with the number of files. The
  block cache holds only data blocks.
- **Startup cost.** Recovery always rewrites the replayed WAL tail as an
  L0 SST and rewrites the MANIFEST. This is simple and makes open's result
  durable, but every unclean open pays a flush.
- **Reads.** Scans merge with a linear minimum search instead of a heap,
  and copy keys and values. Gets allocate a probe key.
- **Deletion side effects.** File deletion runs on whichever thread drops
  the last reference (possibly a reader dropping a snapshot), and errors
  are ignored; leftovers are cleaned at the next open.
- **Unknown files** in the directory are ignored rather than deleted.
