# ADR-0023 — The format v2 cut for 1.0.0-alpha.2

| Field | Value |
|---|---|
| Status | **Accepted in direction by the owner (2026-11-20, in the session that planned it); the final freeze is pending the measurements of every step.** It supersedes the compatibility parts of [ADR-0022](0022-per-type-fulltext-opt-out.md) |
| Date | 2026-11-20 |
| Deciders | Project owner |
| Related to | [ADR-0013](0013-format-v1-and-engine-protocol.md) (engine format v1); [ADR-0021](0021-typed-schemas-and-secondary-indexes.md) and [ADR-0022](0022-per-type-fulltext-opt-out.md) (layers 6 and 7); [Kerno vs SQLite from Python](../testing/python-vs-sqlite.md) (the measurements) |

## Context

Kerno 1.0.0-alpha.1 is published, but nobody runs it in production. Each format change so far
added a layer version (1 to 7) and kept every older reader path: legacy count rebuilds, units
written before an index existed, marks that are raised but never lowered. That keeps alpha.1
databases readable, at the price of code paths and verifier exceptions that a database created
today never needs.

The measurements of 2026-11-20 found the costs that the format itself decides:

| Cost | Measured | Where |
|---|---|---|
| Keyword index of every UTF-8 payload | writes about 7x slower, store about 3.5x larger | `fulltext`, written in every commit |
| Relation writes | 6–11x slower than SQLite in a batch | per-edge work in `create_relation` (counts, adjacency, record) |
| Space per small revision | about 900 bytes for a 210-byte payload | revision metadata and keys, uncompressed SST blocks |
| Change-set limit | 10,000 operations, fixed | `UnitConfig::max_changeset_ops`, not reachable from the facade |

## Decision

**Cut the format once for 1.0.0-alpha.2, with no compatibility path, and freeze the result.**

1. **No backward readers.** A build of alpha.2 refuses a database written by alpha.1 (its units
   layer mark is below the cut version). It does not migrate it. A user who has one exports it
   with the alpha.1 CLI (`kerno <dir> export`) and imports it with alpha.2 (`kerno import`).
2. **The cut version of the units layer is 7.** A database created by alpha.2 writes the mark at
   7 from its first commit. A mark below 7 is refused at open (`LayerTooOld`). The tests and the
   committed fixture directories that existed only for older marks were removed with it
   (`derived-v2`, `public-ids-v3`, `rel-counts-v4`, `type-counts-v5`); the record vectors of
   `v1/structs` stay, since those records are still the current ones.
3. **What enters the cut** (each step measured against SQLite with
   `bindings/python/bench/vs_sqlite.py` before the next one starts):
   1. **Keyword index off by default, per type** (this step). The `0x77` type-options record
      of ADR-0022 changes meaning: bit 0 is `FULLTEXT`, and a type is indexed **only** when
      its record has it. `Db::set_fulltext(type, true)` turns it on at any time: the type's
      active units are indexed (or cleared) in the same change set, so it also works after an
      import (export-v1 carries no type option, like schemas). `verify_units` expects index
      entries only for the types that have it. Applications declare their types: the server
      takes `--fulltext <types>`, and the examples turn on the types they search.
   2. **Relation writes**: decide whether the adjacency counts are written once per change set
      instead of once per edge, and measure what remains of the per-edge cost.
   3. **Space per revision**: compact the revision metadata, or compress SST blocks. Block
      compression touches the **engine** format (ADR-0013), so it would make that format v2.
      This ADR decides it only after a measurement shows the gain.
   4. **The change-set limit**: configurable from the facade (done:
      `Db::open_with_limits`, `kerno.open(max_batch_ops=)`). Whether the default goes up is
      measured with step 2.
4. **What does not enter** (no format change, separate work): opening one directory from
   several processes read-only, and exposing BM25 and hybrid search in Python. The keyword
   index is opt-in after step 1, so the Python search API comes after it.
5. **The freeze.** Steps 1 to 4 are done on the branch. The engine format did not need a new
   version: step 3 used a feature bit (ADR-0013, amendment of 2026-11-20). What freezes with
   alpha.2 is the units layer at version 7 (units-v7.md) and the `BLOCK_LZ4` bit (v1.md §4.4).

## Consequences

* An alpha.1 database cannot be opened by alpha.2. That is by design. Export and import are
  the way across.
* For applications that do not search by keyword (the common case for structured data), the
  default write path loses the largest single cost.
* An application that wants keyword search must say so per type, before storing the type's
  first unit. The README and the Python docs say this first.
* Tests that build databases at layer marks 1 to 6 to exercise legacy paths no longer describe
  a database alpha.2 can open. They are rewritten to the cut version, or removed together with
  the legacy path they test, never silently.

## Status of the steps

| Step | State |
|---|---|
| 3.1 Keyword index opt-in | **done** on the branch: workspace green (1,757 tests without `kernodb-storage`), Python 29 |
| 3.2 Relation writes | **done**: adjacency counts are kept in the change set and written once per change set at commit (no byte of the records changes). 9,000 relations among 300 units: 82 → 63 ms with the earlier caches; vs SQLite in a batch 10.9x → 5.9x. The rest is the three records per edge (REL, out, in), which the two-way adjacency needs |
| 3.3 Space per revision | **done**: SST data blocks are LZ4-compressed when that makes them smaller (`incompat` bit `BLOCK_LZ4`, v1.md §4.4, `Config::block_compression`, on by default). The block encapsulation is unchanged, so **the engine stays format v1** with a new feature bit, as ADR-0013 allows. No `format_version` bump and no regenerated vectors. Measured: LiveSpec's history of the kernodb repository 66 → 35 MB, with the same index time; a small-payload revision 481 → 133 B. Revision metadata (`REV_META`, ~115 B) was left as is: compression already takes most of it |
| 3.4 Change-set limit | **decided**: configurable (`Db::open_with_limits`, `kerno.open(max_batch_ops=)`, tested with 50,000 operations in one commit); the default stays 10,000, because the engine's own byte limit per write still applies and a larger default would turn a too-big batch into a late failure instead of an early one |
| 2 Refusing marks below 7 | **done**, legacy tests and fixtures removed |
