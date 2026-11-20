# Kerno vs SQLite from Python (1.0.0a1 bindings plus the `ccr-5d0e38e9-1hmqh0` branch)

Measured 2026-11-20 in a Linux cloud container (ext4, shared CPU). The numbers compare
the two engines on the same machine in the same run, and should not be read as absolute
throughput. Script: [`bindings/python/bench/vs_sqlite.py`](../../bindings/python/bench/vs_sqlite.py)
(`N=9000 python vs_sqlite.py`; `BIN=1` makes payloads non-UTF-8).

## Setup

Both sides store the same data and sync every commit to disk:

- **Kerno:** `kerno.open(dir)` (synchronous commits), units with 200-byte payloads, a second
  revision per unit, typed relations.
- **SQLite:** WAL mode, `synchronous=FULL`, a `unit` table, a `rev` table (one row per
  revision, to match Kerno's kept history), a `rel` table with an index on `(a, type)`.

## Results (time, Kerno ÷ SQLite)

| Workload | text payload | binary payload |
|---|---|---|
| 2,000 stores, one commit each | 1.3x | 1.1x |
| 9,000 stores, one batch | 4.5x | 2.8x |
| 2,000 point reads | 1.2x | 1.5x |
| scan 11,000 units of a type | 3.1x | 2.4x |
| 9,000 updates (history kept), one batch | 4.9x | 3.0x |
| 9,000 relations, one batch | 5.9x | **10.9x** |
| `related()` × 1,000 | 5.5x | 1.6x |
| all relations of a type | 2.1x | 2.2x |
| history × 1,000 | 1.5x | 1.4x |
| disk | 16.9 MB vs 7.1 MB (2.4x) | 12.9 MB vs 7.4 MB (1.7x) |

"Text payload" is `b"x" * 200` (valid UTF-8, so it is full-text indexed); "binary" prefixes
it with `0xFF`. Opening the database takes 1–4 ms on both sides, and Kerno opened a 95 MB store in 4 ms.

With durable commits, the cost of one commit is mostly the sync to disk, and Kerno stays
close to SQLite. In batches, Kerno does 3–11 times more CPU work per write. Reads are close.

## Where the time goes

Rust only (`Db::commit` with 9,000 ops: store 215 ms, update 233 ms, relate 150 ms; the
Python layer adds about 10%). Callgrind on 3,000-op batches:

| Share | What |
|---|---|
| ~30% | memory allocation (`malloc` / `free` / `realloc`), a `Vec<u8>` per key and value |
| ~15% | `memcmp` in the memtable skiplist (`crossbeam-skiplist`) |
| ~12% | `WriteTxn::get` reads inside the change set (`load_unit`, `current`) |
| ~9% | WAL `canonicalize` |
| ~9% | full-text index maintenance, even for one-term payloads |
| ~8% | relation adjacency counts (`adjust_counts`, `count_entries`) |

## The full-text index (the biggest single factor)

Every payload that is valid UTF-8 is tokenized into the BM25 index in the same commit, with
no way to turn it off. On 9,000 realistic JSON payloads (a code index, about 250 bytes each):

| | store 9,000 | update 9,000 | disk |
|---|---|---|---|
| UTF-8 JSON (indexed) | 0.93 s | 1.12 s | 35.1 MB |
| same bytes, not UTF-8 (not indexed) | 0.13 s | 0.16 s | 10.0 MB |

Indexing made writes **7x slower** and the store **3.5x larger** for data nobody searches by
keyword. The workaround that works today is to store such payloads as bytes that are not
valid UTF-8 (for example compressed). The README says so.

**Proposal (needs an owner decision, since the format is frozen by ADR-0013):** a per-type
opt-out of full-text indexing, persisted in the database so that `verify_units` (UNIT-49)
knows which types to expect in the index. A setting that lives only in memory is not enough:
UNIT-49 expects every active text unit to be indexed, and the index must not depend on the
options of an open.

## Other improvement paths (engine work, not done here)

1. **Relation writes (10.9x).** `create_relation` keeps per-unit adjacency counts and reads
   them back inside the transaction (`plan_count`, `count_entries`). Batching the count updates
   per unit, so they are applied once per change set, would remove most of it.
2. **Allocation.** Reusing key and value buffers per change set, or using an arena for the
   memtable, targets the largest share.
3. **Reads inside a change set.** `load_unit` / `current` read the head from the write
   transaction for every op. A per-change-set cache of heads already read would avoid that.
4. **Disk per revision (about 900 bytes per small revision).** Revision metadata and key
   overhead dominate for small payloads. Compressing SST blocks would help, but it is a
   format question.
5. **A change set holds at most 10,000 operations** (`UnitConfig::max_changeset_ops`). The
   facade does not let a caller raise it, and the engine also has its own byte limit per
   write.
