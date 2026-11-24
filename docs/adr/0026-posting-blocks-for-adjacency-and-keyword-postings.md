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
from `f7a5004`, "after" the wheel of this change, run in the same sitting. The load average
at each start, the raw output and the tables are in
[`docs/testing/raw/2026-11-23-blocks/`](../testing/raw/2026-11-23-blocks/); the rows next to
SQLite are in the [report](../testing/perf-gaps-2026-11.md) §5. Kerno / SQLite ratio, lower is
better.

| Row (Python, units level) | 10k before | 10k after | 100k before | 100k after |
|---|---|---|---|---|
| store x N keyword-indexed, batches | 6.71 | **3.62** | 6.50 | **3.54** |
| keyword search x100, top 10 | 0.37 | **0.14** | 0.40 | **0.11** |
| relate x N, batches | 6.73 | **5.71** | 10.24 | 10.74 |
| `related()` x1000 | 2.63 | **2.15** | 5.63 | **4.84** |
| two-hop traversal x200 | 5.05 | **3.68** | 11.10 | **8.14** |
| all relations of a type | 1.21 | 1.16 | 0.98 | 0.98 |
| close | 123 | 64 | 50 | 62 |
| disk (size) | 1.83 | 1.72 | 1.84 | 1.76 |

The load average at the start of the four runs was 2.99 to 6.54. Relate at 100k is the one row
that got worse (2.90 s to 3.18 s, ranges 2.85 to 2.90 against 2.98 to 3.71), so the criterion "no
worse than before" is not met there: the benchmark relates random pairs, lists hold one or two
entries, a block saves no record on them, and the seek that counts a list with no count record is
paid for each new list (see the decision above). A first build that skipped that seek measured
relate at 4.94x (10k) but could overwrite a list whose count record was lost, so it was not kept.
Rows that are not part of this change moved within the range of their repetitions.

Two criteria are met only in part. "No worse than before" fails for relate at 100k (above). "Reads
no more than it did" holds for a list with no entries, which costs the one seek it always cost and
no block read; for a list that already has entries the writer now reads and rewrites a block per
commit where the old one made a blind put per entry. That second gap is read from the code, not
measured on its own: the relate row is where it would show.

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
