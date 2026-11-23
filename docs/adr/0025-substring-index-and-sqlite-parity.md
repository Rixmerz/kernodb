# ADR-0025 — A trigram index of posting blocks for substring search, and the profiled gaps to SQLite

| Field | Value |
|---|---|
| Status | **Proposed** (2026-11-23), implemented; waits for the owner |
| Date | 2026-11-23 |
| Deciders | Project owner (pending) |
| Related to | ADR-0012 (benchmark methodology), ADR-0022/0023 (per-type options, opt-in keyword index), [units-v8](../file-format/units-v8.md), [`docs/testing/python-vs-sqlite.md`](../testing/python-vs-sqlite.md) §2026-11-23 |

## Context

The goal is that kernodb is at least as fast as SQLite on every workload the repository
benchmarks. At 1.0.0-alpha.3 substring search was the weakest point: 40 to 51 times slower than
SQLite in the store benchmark of LiveSpec, and 57 times slower than a `LIKE '%x%'` scan (112
times slower than FTS5 `trigram`) at 100k units in `bindings/python/bench/vs_sqlite.py`. Kerno had
no substring index: a caller listed the type with `query(type)` and filtered in Python.

The baseline of every workload, measured before any change, is in
[`docs/testing/raw/2026-11-22-vs-sqlite/`](../testing/raw/2026-11-22-vs-sqlite/)
(`before-10k.*`, `before-100k.*`, commit `73517fe`).

## Profiling

No `perf` or `valgrind` on the machine (both need root to install). The release build of the
Python extension was profiled with `py-spy record --native -r 250 -f raw` over a driver that runs
the benchmark's workloads at 100k units, each in a named function, and the samples were
aggregated per workload (inclusive and self time per frame). Findings:

| Workload | Where the time goes | Root cause |
|---|---|---|
| substring search (alpha.3) | Python: `query(type)` decodes every unit, then `in` on each payload | **No index**: every search reads every payload of the type through the binding |
| store, substring-indexed (first index: one engine record per trigram and unit) | `BTreeMap::insert` of the transaction overlay 42%, freeing the commit request 17%, memtable skiplist and `apply_commit` 15% | **Records per unit**: about 70 trigrams per 80-byte name, so a batch of 10,000 names was 700,000 engine records, each paying the overlay, the WAL encoding, the memtable and its free. 70% of all samples of the run |
| store, keyword-indexed | overlay `BTreeMap::insert` 30%, memtable 22%, freeing the request 8% | The same disease: one posting record per term and unit (ADR-0023's design) |
| relate, related(), two-hop | memtable skiplist lookups 14–21%, SST `Table::get` and the block cache 11–15%, `BTreeSet` inserts of the change set 8% | Point reads: a relation checks both ends and writes three records (the relation, two adjacency entries) plus counts; each traversal step decodes the unit head of every neighbour. SQLite answers from one B-tree index and one join |
| close | LZ4 compression and bloom filter of the flushed table, joining the background thread | `close` writes the memtable out as a table so that the next open replays no WAL; SQLite's close only checkpoints |
| scan by type, history | per-unit head decode and copies across the binding | Not profiled further: no change attempted |

## Options considered

1. **One engine record per trigram and unit** (the first implementation). Exact and simple:
   a write deletes and puts the trigrams that changed. Measured: search 0.60× SQLite's `LIKE`
   at 10k, but the indexed store **21×** SQLite (2.55 s against 0.12 s), see the profile above.
2. **Posting blocks**: one record per type, trigram and range of units, at most 128 unit ids
   each, written once per change set. Fewer records by two orders of magnitude.
3. **Dense per-type row numbers with delta-coded posting lists** (what FTS5 does). The
   smallest index, but a new mapping record per unit and a merge process for the segments.
4. **No index, a faster scan in Rust**. Still linear in the payloads of the type: cannot beat an
   index on selective needles.

## Decision criteria

Set before measuring: exact `LIKE '%x%'` semantics (ASCII case-insensitive, bytes otherwise),
the same hit count as SQLite on every needle of the benchmark, `verify_units` able to prove the
index exact, opt-in per type like the keyword index, and the search at or below SQLite's
`LIKE` and FTS5 `trigram` at 10k and 100k units.

## Evidence

Same script, same methodology (ADR-0012: a fresh process and directory per repetition, both
engines alternating in it, median of 3 with the range, data on `/home`, an `fsync` per commit),
Python 3.12.14, SQLite 3.53.1. Kerno / SQLite ratio, lower is better:

| Workload | 10k before | 10k after | 100k before | 100k after |
|---|---|---|---|---|
| substring search ×20 vs `LIKE` | 56.75 | **0.16** | 57.03 | **0.34** |
| substring search ×20 vs FTS5 `trigram` | 95.87 | **0.27** | 112.16 | **0.66** |
| store, substring-indexed, batches | 1.50 (no index) | 4.17 | 1.51 (no index) | 3.29 |

Option 1 measured 21.11 at 10k for the indexed store; option 2 brought it to 4.17. The full
tables are in [`python-vs-sqlite.md`](../testing/python-vs-sqlite.md).

## Decision

Option 2: a trigram index of posting blocks, opt-in per type (`set_substring`), layer version 8
([units-v8](../file-format/units-v8.md)). `search_substring(needle, limit, types)` in Rust,
Python and Node.

- A block key is `0x7A ‖ type ‖ gram ‖ 0x00 ‖ bound` or `… ‖ gram ‖ 0x01` (the tail); the block of
  a unit is the first key at or after `gram ‖ 0x00 ‖ unit`, one forward seek, which the engine
  has (it has no reverse seek). Unit ids are UUIDv7, so new units append to the tail block.
- A change set keeps its gram changes and writes them at commit, each changed block once.
- A search intersects the lists of the needle's trigrams, shortest first, probing a long list by
  seeks when few candidates are left, and checks a needle over three bytes against the payload.
- `verify_units` (UNIT-52) proves the blocks hold exactly the `Active` units of the indexed
  types, in their ranges.

## Consequences

- Substring search is 3 to 6 times faster than SQLite's `LIKE` scan and faster than FTS5
  `trigram` at both scales, with the same hits.
- **Still slower than SQLite** (2026-11-23, 100k after): indexed store 3.3×, keyword-indexed
  store 7.2×, relate 11.9×, related() 4.7×, two-hop 8.6×, history 4.9×, scan by type 3.5×,
  batch store 1.9×, update 2.2×, point read 1.2×, close 33×, reopen 33× (but 1.05 in one of the
  three repetitions: SQLite's own reopen ranges 0.9 ms to 40 ms), disk 1.8× larger. The causes
  are the profile rows above: the engine's cost per record, the extra records of relations and
  postings, and the table written at close. None is fixed by this ADR.
- Faster than SQLite: substring search, keyword search (0.33× at 10k, 0.43× at 100k, from reading
  the head of only the best-scored units in a type-restricted search), and one commit per store
  (0.84–0.85×).
- The index is 16 bytes per unit per distinct trigram, before LZ4: the 100k run's database grew
  from 207 MiB to 299 MiB with it.
- A small commit rewrites the tail block of each of its grams (up to 2 KiB each).
- Format: layer 8; a build that reads up to 7 refuses a database that ever turned the option on.

## Follow-up

- Posting blocks for the keyword index (the 7× indexed store has the same cause).
- Lower the engine's per-record cost on the write path (the transaction overlay and the
  memtable are the top frames of every batch write).
- Relations: fewer records per relation, or a read path that avoids decoding every neighbour's
  head for a traversal.
- `close` without a table write, if reopen can replay the WAL as fast.
