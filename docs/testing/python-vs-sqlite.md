# Kerno vs SQLite from Python (1.0.0a1 bindings plus the `ccr-5d0e38e9-1hmqh0` branch)

Measured 2026-11-20 in a Linux cloud container (ext4, shared CPU). The numbers compare
the two engines on the same machine in the same run, and should not be read as absolute
throughput. Script: [`bindings/python/bench/vs_sqlite.py`](../../bindings/python/bench/vs_sqlite.py)
(`N=9000 python vs_sqlite.py`; `BIN=1` makes payloads non-UTF-8).

## 2026-11-23: substring index and the gap to SQLite (ADR-0025)

The latest run; the sections below it are the 2026-11-20 measurement and stay as they were.
Script: [`bindings/python/bench/vs_sqlite.py`](../../bindings/python/bench/vs_sqlite.py), rewritten
from the alpha.3 one-file script: it keeps its workloads and adds two-hop traversal, substring
search (against `LIKE '%x%'` and against FTS5 `trigram`, both on SQLite), keyword search (against
FTS5 `unicode61`), close, reopen and disk, with symbol-like names and prose documents built from a
fixed seed. Methodology of ADR-0012: a fresh process and directory per repetition, both engines in
the same process alternating workload by workload, median of 3 repetitions, data on an ext4/btrfs
`/home` (not tmpfs), `fsync` per commit on both sides (`synchronous=FULL`, WAL). Python 3.12.14,
SQLite 3.53.1, 14 CPUs.

```
python bindings/python/bench/vs_sqlite.py --n 10000  --reps 3 --data <dir on disk> --out after-10k.json
python bindings/python/bench/vs_sqlite.py --n 100000 --reps 3 --data <dir on disk> --out after-100k.json
```

Raw data, logs and per-run tables: [`raw/2026-11-22-vs-sqlite/`](raw/2026-11-22-vs-sqlite/).
"Before" is commit `73517fe` (1.0.0-alpha.3 plus fixes); the script, with no `search_substring` to
call, filters `query("sym")` in Python, which is what a caller had to do. "After" is the posting
block index of this change (commit `7647047`; the 10k run's `env.json` says `2724e91` because it ran
on the working tree just before that commit, with the same code).

**Read the ratios, not the seconds across columns.** The before runs met a busy machine (1-minute
load 41-47 at 10k, 12-36 at 100k); the after runs a quiet one (2.4-3.5). Each ratio compares the
two engines inside the same repetition, so it holds; an absolute "before → after" time does not.
The relate ratio at 100k rising from 7.4 to 11.9 is SQLite gaining more from the quiet machine
(0.52 s → 0.28 s) than Kerno (3.88 s → 3.33 s), not a regression measured on the same load.

Both engines found the same matches on every needle: 18,426 at 10k and 186,413 at 100k (the
script asserts it), and the same keyword hits.

#### 10k units

| Workload | Before: Kerno / SQLite (s) | Before ratio | After: Kerno / SQLite (s) | After ratio | Kerno <= SQLite |
|---|---|---|---|---|---|
| store x2000, one commit each | 6.2 / 7.69 | 0.81 | 2.52 / 2.99 | 0.84 | yes |
| store x10000, batches of 10000 | 1.02 / 0.217 | 4.71 | 0.159 / 0.0928 | 1.72 | **no** |
| point read x2000 | 0.0295 / 0.0189 | 1.56 | 0.0137 / 0.0136 | 1.00 | no (within noise) |
| scan all 12000 units of a type | 0.136 / 0.047 | 2.90 | 0.0362 / 0.0156 | 2.31 | **no** |
| update x10000 keeping history, batches | 0.976 / 0.476 | 2.05 | 0.189 / 0.116 | 1.64 | **no** |
| relate x10000, batches | 1.15 / 0.0556 | 20.67 | 0.213 / 0.0303 | 7.04 | **no** |
| related() x1000 | 0.0521 / 0.0158 | 3.30 | 0.0124 / 0.00626 | 1.97 | **no** |
| two-hop traversal x200 | 0.0123 / 0.00194 | 6.33 | 0.0054 / 0.00154 | 3.50 | **no** |
| all relations of a type | 0.0506 / 0.0181 | 2.80 | 0.00955 / 0.00812 | 1.18 | **no** |
| history x1000 | 0.0341 / 0.0277 | 1.23 | 0.0111 / 0.00795 | 1.39 | **no** |
| store x10000 substring-indexed, batches | 0.826 / 0.55 | 1.50 | 0.626 / 0.15 | 4.17 | **no** |
| substring search x20 vs LIKE scan | 6.58 / 0.116 | 56.75 | 0.00693 / 0.0421 | 0.16 | yes |
| substring search x20 vs FTS5 trigram | 6.58 / 0.0687 | 95.87 | 0.00693 / 0.0256 | 0.27 | yes |
| store x10000 keyword-indexed, batches | 10.4 / 0.566 | 18.33 | 0.953 / 0.13 | 7.34 | **no** |
| keyword search x100, top 10 | 0.467 / 0.124 | 3.75 | 0.0139 / 0.0416 | 0.33 | yes |
| close | 1.07 / 0.00386 | 278.05 | 0.214 / 0.002 | 106.86 | **no** |
| disk MiB | 22.2 / 17 | 1.31 | 31.2 / 17 | 1.84 | **no**, size not time |
| reopen | 0.0276 / 0.0081 | 3.41 | 0.0085 / 0.000601 | 14.15 | **no** |

#### 100k units

| Workload | Before: Kerno / SQLite (s) | Before ratio | After: Kerno / SQLite (s) | After ratio | Kerno <= SQLite |
|---|---|---|---|---|---|
| store x2000, one commit each | 4.01 / 2.43 | 1.65 | 2.43 / 2.86 | 0.85 | yes |
| store x100000, batches of 10000 | 2.88 / 0.768 | 3.75 | 1.68 / 0.902 | 1.87 | **no** |
| point read x2000 | 0.0177 / 0.013 | 1.36 | 0.0226 / 0.0183 | 1.24 | **no** |
| scan all 102000 units of a type | 0.434 / 0.0988 | 4.39 | 0.486 / 0.139 | 3.51 | **no** |
| update x100000 keeping history, batches | 3.45 / 0.912 | 3.78 | 2.47 / 1.11 | 2.23 | **no** |
| relate x100000, batches | 3.88 / 0.522 | 7.42 | 3.33 / 0.28 | 11.89 | **no** |
| related() x1000 | 0.0277 / 0.00478 | 5.79 | 0.0209 / 0.00441 | 4.74 | **no** |
| two-hop traversal x200 | 0.0148 / 0.00179 | 8.26 | 0.0113 / 0.00132 | 8.61 | **no** |
| all relations of a type | 0.181 / 0.0717 | 2.52 | 0.0719 / 0.069 | 1.04 | **no** |
| history x1000 | 0.0254 / 0.00489 | 5.20 | 0.0236 / 0.00481 | 4.91 | **no** |
| store x100000 substring-indexed, batches | 2.83 / 1.87 | 1.51 | 5.32 / 1.62 | 3.29 | **no** |
| substring search x20 vs LIKE scan | 20 / 0.351 | 57.03 | 0.106 / 0.312 | 0.34 | yes |
| substring search x20 vs FTS5 trigram | 20 / 0.179 | 112.16 | 0.106 / 0.162 | 0.66 | yes |
| store x100000 keyword-indexed, batches | 19.8 / 1.05 | 18.86 | 6.99 / 0.972 | 7.20 | **no** |
| keyword search x100, top 10 | 1.78 / 0.28 | 6.36 | 0.108 / 0.253 | 0.43 | yes |
| close | 0.2 / 0.00244 | 82.15 | 0.168 / 0.00515 | 32.60 | **no** |
| disk MiB | 207 / 162 | 1.27 | 299 / 162 | 1.84 | **no**, size not time |
| reopen | 0.0303 / 0.0339 | 0.89 | 0.0386 / 0.00118 | 32.84 | **no** |

### Where kernodb now meets the target

- **Substring search**: 0.16x / 0.34x of SQLite's `LIKE` scan and 0.27x / 0.66x of FTS5
  `trigram` (10k / 100k). It was 57x and 96-112x.
- **Keyword search, top 10**: 0.33x / 0.43x (was 3.8x / 6.4x). A search restricted to a type now
  reads the head of the best-scored units only, not of every scored one.
- **One commit per store**: 0.84x / 0.85x.

### Where it still misses, and why (profiled with py-spy, ADR-0025)

| Workload (100k ratio) | Reason |
|---|---|
| store, substring-indexed (3.3x) | The index costs one 16-byte id per unit per distinct trigram, about 70 per name, written through the transaction overlay, the WAL and the memtable; FTS5 writes delta-coded row numbers. The first design, one record per trigram and unit, was 21x |
| store, keyword-indexed (7.2x) | One posting record per term and unit: the overlay `BTreeMap` (30%) and the memtable (22%) dominate, the disease the trigram index had before its blocks |
| relate (11.9x), related() (4.7x), two-hop (8.6x) | A relation checks both ends and writes three records plus counts; a traversal decodes the head of every neighbour. Point reads through the memtable skiplist, the block cache and the tables are the top frames. SQLite does one row and one index entry, and a join |
| store in batches (1.9x), update (2.2x) | Per-record cost of the write path (overlay, WAL encoding, memtable, freeing the batch); Kerno also keeps every revision as two records |
| scan by type (3.5x), history (4.9x), point read (1.0-1.2x) | Head decoding and copies of every unit across the binding; not profiled further |
| close (33x) | `close` writes the memtable out as a table (LZ4, bloom filter, a thread join) so the next open replays no WAL; SQLite only checkpoints |
| reopen (33x at the median, 1.05x in one repetition) | Opening reads the manifest and table footers; SQLite's own reopen spans 0.9 ms to 40 ms across the repetitions, so this row is mostly noise at the millisecond scale |
| disk (1.8x) | The trigram index (16 bytes per unit per trigram) took the 100k database from 207 to 299 MiB |

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
