# ADR-0026 — Posting blocks for graph adjacency and keyword postings

| Field | Value |
|---|---|
| Status | **Proposed** (2026-11-23), implemented; waits for the owner |
| Date | 2026-11-23 |
| Deciders | Project owner (pending) |
| Related to | [ADR-0025](0025-substring-index-and-sqlite-parity.md) (the trigram blocks this reuses), [ADR-0012](0012-benchmark-methodology-and-exit-thresholds.md) (benchmark method), [ADR-0023](0023-format-v2-cut-for-alpha2.md) (the earlier format cut), [units-v9](../file-format/units-v9.md), [`perf-gaps-2026-11.md`](../testing/perf-gaps-2026-11.md) Ranks 6 and 7 |

## Context

The profiles of 2026-11-23 ([report](../testing/perf-gaps-2026-11.md) §2) found the same cost in
three places: one engine record per entry. The trigram index paid it first, and ADR-0025 cut it
from 21x to 3.5x of SQLite's time by storing each posting list as blocks of at most 128 unit ids,
written once per change set. Two stores still wrote one record per entry:

| Store | Records | Measured (100k units, kerno / SQLite) |
|---|---|---|
| Keyword postings (`POSTING`, `0x74`) | one per term and unit | keyword-indexed batch store 7.33x; the overlay `BTreeMap` insert was 26.6% of its samples |
| Adjacency (`REL_OUT`, `REL_IN`, `0x41`, `0x42`) | one per relation and end | relate 7.81x, `related()` 5.64x, two-hop 11.21x |

## Options considered

1. **Keep one record per entry** and lower the engine's per-record cost (the transaction overlay,
   the memtable, freeing the commit request). Helps every write, but is the engine's change, not
   this layer's, and the postings stay the largest record count of a keyword batch.
2. **Posting blocks** for both stores, with the mechanics of ADR-0025 shared: a list is a run of
   entries cut in blocks of at most `max` entries under one key prefix, one record per block,
   written once per change set. A format change.
3. **Dense per-type numbers and delta-coded lists** (FTS5's design). The smallest index, but a new
   mapping record per unit and a merge process.
4. **Embed the relation's data in the adjacency entry** (type, provenance, `created_seq`), so a
   traversal builds a `Relation` without reading the `REL` record. It would remove the
   per-neighbour read, but it duplicates the `REL` record in two more places that the writer must
   keep equal, and it weakens the check that every adjacency entry agrees with its `REL`
   (UNIT-36, I-32). Not taken.

## Decision criteria

Set before measuring: the answers of every read stay equal to the reference path's (I-32); a lost
block is still found by the count records (K1); `verify_units` proves both indexes exact;
`UnitHasRelations` is still raised for a unit with a relation the change set itself created; the
writer reads no more than it did (no read of a list known to be empty); a count record lost with
its entries left behind never makes the writer overwrite them (added after a review found the
first build did); and the benchmark rows (`relate`, `related()`, two-hop, keyword store, keyword
search) are no worse than before and reported next to SQLite whatever they show.

## Decision

Option 2, in one format change: **layer version 9**, a cut like ADR-0023: a database starts at 9, a
mark below 9 is refused, and nothing migrates (nobody runs the earlier format; there is no
compatibility code). [units-v9](../file-format/units-v9.md) is the specification.

- **One shared mechanism** (`crates/kerno-units/src/blocks.rs`): the block keys, the seek, the merge
  of a change set's operations into the blocks it touches, the cut at the limit and the deletion of
  an emptied block. The trigram index, the keyword postings and the adjacency lists are three
  block codecs over it; the trigram bytes did not change.
- **Keyword postings**: key `0x74 ‖ term ‖ 0x00 ‖ (0x00 ‖ bound | 0x01)`, entries `unit ‖ tf ‖ dl`
  (24 bytes), 128 per block. The `DOC` record per unit stays (a change reads it to know which
  postings to remove). The `STATS` record is no longer read and written once per unit: a change set
  sums the deltas of its documents and writes it once at commit.
- **Adjacency**: key `tag ‖ unit ‖ type ‖ 0x00 ‖ (0x00 ‖ bound | 0x01)`, entries `edge ‖ other`
  (32 bytes), 64 per block. The `REL` record per relation and the count records per
  `(unit, direction, type)` are unchanged. A relation writes one `REL` record and keeps its two
  entries in the change set; both are written at commit.
- **A list known to be empty is not read.** The writer reads the count record of each list it
  changes anyway; a count of zero before the change set's first entry means the list has no block,
  so its blocks are written directly. With random endpoints most lists hold one or two entries, and
  a read per list would have made `relate` slower than the blind puts it replaces. A missing count
  record does not mean zero: the record may have been lost with the entries left behind, and the
  tail block of that list would be written over unread. So a missing record is still rebuilt by
  counting the entries of the blocks (`adj_count::count_entries`, one seek for a new list), as the
  writer did before this change, and only a count the blocks confirm is taken for an empty list.
- **A list that has entries is read with a point read of its tail, not a seek.** The tail holds
  every id above the last bound, so when its first entry is at or below the lowest id the change
  set changes in the list, the tail is the block of all of them (`blocks::flush`). Ids are minted
  in ascending order, so the entries a change set adds almost always fall in the tail; a removal
  below it, or a list whose tail is gone, takes the seek as before. A tail that does not decode is
  left to the seek, which reports it only if the change needs that block.
- **The counts and the adjacency changes of a change set are one map by list** (`AdjList`: the
  count as the change set leaves it, whether its record is written, the entries to put or remove,
  sorted by relation id). A relation plans the counts of its two lists by reads alone
  (`plan_relation`), then applies them with the entries once its `REL` record is written, so a
  read that fails still leaves the change set usable. Commit writes the count record and the
  blocks of each list together (`flush_adjacency`). The counts used to live in a hash map and a set
  of their own, and each relation paid a count key built, cloned twice and inserted in both, on top
  of the entry keys.
- **`UnitHasRelations` sees the change set's own relations** by merging the blocks of the
  transaction with the entries the change set keeps (`ChangeSet::relation_of_unit`), without
  flushing, so a refused operation leaves the change set usable.
- **The planner's probes count entries, not blocks** (`probe_adjacency`), so the cost and
  absent-index decisions the planner tests pin do not move.
- **`Db::related` reads the head of the unit** (`UnitSnapshot::unit_exists`) instead of loading and
  hashing its payload to learn that it exists. It is not part of the block design, and it is in the
  same rows: every `related()` and every hop of a two-hop traversal paid it.
- The read of an adjacency list still checks every entry against its `REL` record, its two ends and
  its mirror entry, and the counts (K5, K1): one `REL` read, one head read per distinct end, one
  seek to the mirror block per entry. Those reads are what option 4 would remove and what this
  decision keeps.

## Evidence

Same script, same method as ADR-0025 and ADR-0012: `bindings/python/bench/vs_sqlite.py`, a fresh
process and directory per repetition, both engines alternating in it, median of 3 with the range,
data on `/home`, an `fsync` per commit, Python 3.14.9 and SQLite 3.54.0. "Before" is the wheel built
from `ea8b610` (the parent of this change), "after" the wheel of this change, run back to back in the
same sitting. The load average at each start (3.59 to 4.74), the raw output and the tables are in
[`docs/testing/raw/2026-11-24-blocks/`](../testing/raw/2026-11-24-blocks/); the rows next to
SQLite are in the [report](../testing/perf-gaps-2026-11.md) §5. Kerno / SQLite ratio, lower is
better.

| Row (Python, units level) | 10k before | 10k after | 100k before | 100k after |
|---|---|---|---|---|
| store x N keyword-indexed, batches | 7.27 | **3.74** | 6.68 | **3.52** |
| keyword search x100, top 10 | 0.37 | **0.15** | 0.41 | **0.11** |
| relate x N, batches | 7.08 | **4.98** | 10.07 | 9.64 |
| `related()` x1000 | 2.85 | **2.17** | 5.83 | **5.00** |
| two-hop traversal x200 | 4.80 | **3.49** | 11.41 | **8.69** |
| all relations of a type | 1.11 | 1.10 | 0.96 | 0.95 |
| close | 103 | 57 | 42 | 67 |
| disk (size) | 1.83 | 1.72 | 1.84 | 1.76 |

The `close` ratio at 100k rose because SQLite's close took 0.0041 s before and 0.0019 s after;
Kerno's went from 0.172 s to 0.124 s. Rows that are not part of this change moved within the range
of their repetitions.

**Relate.** A first build of this change (the staging of entries in a map per list, a seek to read
each list that has entries, a count cache and a staging map of their own) was slower than the build
before it at 100k, 2.43 s against 2.91 s for 100k random-pair relates at 1000 per commit, and the
benchmark relates random pairs, so most lists hold one or two entries and a block saves no record.
Temporary timers on the commit path (not kept) located the cost: the seek that counts a list with no
count record, which the old writer paid too (126,607 seeks, 0.67 s of about 2.9 s), then the
staging (0.14 s) and the flush of the blocks (0.29 s) against 0.08 s for the old writer's three blind
puts. The decisions above (the tail point read, the sorted `Vec` of operations, one map for the
counts and the entries) bring the writer to the old build's time without a format change. Five
alternated runs of the old build and this one (100k random-pair relates through `UnitDb`, seconds,
[`relate-alternated.txt`](../testing/raw/2026-11-24-blocks/relate-alternated.txt)): at 1000 per
commit 2.436 (2.429 to 2.513) against 2.387 (2.329 to 2.476), and at 10,000 per commit 2.597 (2.484
to 2.733) against 2.295 (2.224 to 2.343). In the Python run relate at 100k went 2.98 s to 2.85 s
(ranges 2.95 to 3.04 and 2.71 to 2.95), which is within the range of the repetitions. A build that
skipped the seek for a list with no count record measured cheaper, but could overwrite a list whose
count record was lost, so it was not kept; a cheaper proof of emptiness (a point read of the tail)
needs every non-empty list to have a tail, which costs a promotion of the last bounded block
whenever a tail is emptied, and was not taken either.

All the criteria are met, the last one with a stated reading. "The writer reads no more than it did"
holds as the criterion defines it, for a list known to be empty: it costs the one seek it always
cost and no block read. A list that already has entries is not covered by it: the writer now makes
a point read of its tail and rewrites the block where the old writer made a blind put per entry.
That extra read is paid back inside the same change (the relate rows above, 2% and 12% less time
than the old writer), not removed.

## Consequences

- **Format:** layer 9. A build of 1.0.0-alpha.3 refuses a database of this build
  (`IncompatibleLayer`), and this build refuses one of alpha.3 (`LayerTooOld`): export with the
  build that wrote it, import with the other. The mark of an alpha.3 database that never used a
  newer feature (layer 7) is refused too, because its adjacency and postings are per-entry records.
- **Writes:** a keyword batch writes one record per posting block instead of one per term and unit;
  a relation batch one block per list instead of one entry per relation and end. A small commit
  rewrites the block of each list it changes (up to 4 KiB for a posting block, 2 KiB for an
  adjacency block).
- **Reads:** a search reads a few blocks per term instead of one record per posting. An adjacency
  scan reads blocks, and a hub with many relations of one type reads them in order. The per-neighbour
  `REL`, head and mirror reads of a traversal are unchanged (option 4 above), so the two-hop gap is
  smaller than the trigram precedent suggests.
- **Verifier:** UNIT-36 and UNIT-49 also check that each block holds only ids in its range; a
  block that does not decode makes the comparison of its index skip, as an entry that did not
  decode did.
- **Follow-up:** carry `created_seq` and the provenance kind in the adjacency entry if the
  per-neighbour `REL` read is still the top of the traversal profile; per-hop reads batched into one
  `relations_of` for several units (the two-hop row issues 1 + N calls from Python); `close`
  without a table write.
