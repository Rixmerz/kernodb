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

Two builds: **1.0.0a1** (first measurement) and the **alpha.2 branch** (keyword index opt-in,
LZ4 data blocks, change-set caches, counts written once per change set; ADR-0023).

| Workload | 1.0.0a1, text payload | 1.0.0a1, binary payload | alpha.2 branch |
|---|---|---|---|
| 2,000 stores, one commit each | 1.3x | 1.1x | 1.0x |
| 9,000 stores, one batch | 4.5x | 2.8x | 2.8–3.1x |
| 2,000 point reads | 1.2x | 1.5x | 1.3–1.4x |
| scan 11,000 units of a type | 3.1x | 2.4x | 2.3–3.0x |
| 9,000 updates (history kept), one batch | 4.9x | 3.0x | 3.5x |
| 9,000 relations, one batch | 5.9x | 10.9x | 5.9–11x (Kerno ~0.16 s every run; SQLite varies 0.014–0.026 s) |
| `related()` × 1,000 | 5.5x | 1.6x | 2.3–2.7x |
| all relations of a type | 2.1x | 2.2x | 1.8–2.9x |
| history × 1,000 | 1.5x | 1.4x | 0.8–1.4x |
| **disk** | 16.9 MB vs 7.1 MB (2.4x larger) | 12.9 MB vs 7.4 MB (1.7x) | **3.4 MB vs 7.1 MB (2x smaller)** |

"Text payload" is `b"x" * 200` (valid UTF-8, keyword-indexed in 1.0.0a1); "binary" prefixes it
with `0xFF`. On the alpha.2 branch no type is keyword-indexed by default, so both payloads
behave the same. The synthetic payload compresses unusually well: on real data (LiveSpec's
history of the kernodb repository) LZ4 took the store from 66 MB to 35 MB (1.9x).

With durable commits, one commit per write costs about what it costs SQLite. In batches Kerno
still does 3–11 times more CPU work per write: it keeps every revision, and each relation is
three records (the relation and both adjacency entries), which SQLite's single index does not
need. Reads are within 1.5x except scans by type.

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

**Done (proposed): [ADR-0022](../adr/0022-per-type-fulltext-opt-out.md), units layer v7.**
`db.set_fulltext(type, False)` before the first unit of a type. With it, the same 9,000 JSON
payloads store in 0.24 s, update in 0.19 s and take 10.0 MB, with no change to the payload.
It does not touch the frozen engine format, only the units layer, as ADR-0021 did.

## Other improvement paths (engine work, not done here)

1. **Relation writes (10.9x).** Done in part: a change set now caches the adjacency counts and
   the active units it already read (82 → 72 ms on 9,000 relations among 300 units). The
   profile after that is dominated by allocation and the memtable (items 2 and 3).
2. **Allocation.** Reusing key and value buffers per change set, or using an arena for the
   memtable, targets the largest share.
3. **Reads inside a change set.** `load_unit` / `current` read the head from the write
   transaction for every op. A per-change-set cache of heads already read would avoid that.
4. **Disk per revision.** Done: with the keyword index opt-in and LZ4 data blocks
   (ADR-0023), a small revision takes 133 B on disk instead of about 900 B, and LiveSpec's real
   store went from 66 MB to 35 MB.
5. **A change set holds at most 10,000 operations** by default. Done: `Db::open_with_limits`
   and `kerno.open(path, max_batch_ops=...)` change it, but the engine's byte limit per write still applies.
