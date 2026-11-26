# Unit layer v12: keyword postings by document ordinal

| Field | Value |
|---|---|
| Status | Introduced on the branch `perf/layer12`; not frozen |
| Layer version | `layer_version = 12`, `envelope_version = 1` |
| Related to | [units-v11](units-v11.md) (the cut this one supersedes; its payload rules are unchanged), [units-v9](units-v9.md) (the posting blocks of `0x74`, whose keys and values change here), [engine v1 format](v1.md) (frozen, no changes) |

"Must" and "must not" are normative. Only the keyword index changes: `POSTING` (`0x74`), `DOC`
(`0x75`), `STATS` (`0x76`) and the new `ORD` (`0x7b`). The trigram index (`0x7a`) and the
adjacency blocks keep the units-v9 design.

## 1. What version 12 of the layer is

- A cut, like versions 7, 9, 10 and 11: a database is created at version 12, a build opens
  version 12 only, and a mark below 12 is refused at open (`LayerTooOld`). There is no
  migration: export the database with the build that wrote it and import it into a build of
  version 12 (import rebuilds the index through ordinary writes).
- A build that reads up to 11 refuses a mark of 12 with `IncompatibleLayer` (UNIT-16).

Why: a units-v9 posting was 24 bytes, `unit[16] ‖ tf: u32 BE ‖ dl: u32 BE`, and a unit id has
about 9 random tail bytes that do not prefix-compress. On a 41,882-document database the
postings were 53.9 MB of 248 MB. SQLite FTS5 names a document by a dense integer rowid; this
layer does the same.

## 2. Ordinals

- Every document in the keyword index has an ordinal `ord: u32`. A writer gives ordinals in
  ascending order from `STATS.next_ord` and never gives one twice.
- A unit keeps its ordinal while it stays in the index: an update that re-indexes it keeps it.
  A unit that leaves the index (delete, archive, a payload with no term, its type's keyword
  index turned off) drops it; when it comes back it gets a new one.
- A change set that is refused or aborted may leave unused ordinals: gaps are valid.
- A write that would need an ordinal above `u32::MAX` is refused (`InvalidArgument`); the
  counter never wraps. Export and import number the documents again from 0.

## 3. Records

- `ORD` `0x7b ‖ ord: u32 BE` → `envelope[2] ‖ unit[16]`: the unit of an ordinal. Written with
  the unit's first `DOC` and deleted with its last, in the same transaction.
- `DOC` `0x75 ‖ unit[16]` → `envelope[2] ‖ ord: u32 BE ‖ dl: u32 BE ‖ n: u32 BE ‖ n × (len: u8 ‖
  term)`: the ordinal is new; the rest is as in units-v9.
- `STATS` `0x76` → `envelope[2] ‖ since: u64 BE ‖ docs: u64 BE ‖ total_len: u64 BE ‖
  next_ord: u64 BE`: `next_ord` is new, above every ordinal ever given (at most 2^32).
- `POSTING` blocks: key `0x74 ‖ term ‖ 0x00 ‖ 0x00 ‖ bound_ord: u32 BE` (a bounded block) or
  `0x74 ‖ term ‖ 0x00 ‖ 0x01` (the tail, unchanged). Value `envelope[2] ‖ n × (varint(delta) ‖
  varint(tf) ‖ varint(dl))`, `1 ≤ n ≤ 128`, strictly ascending by ordinal: the first delta is the
  ordinal itself, each other one is the difference to the previous ordinal (at least 1). A varint
  is unsigned LEB128 of a `u32`, in its shortest form. `tf ≥ 1` and `dl ≥ tf`. A bounded block
  holds the ordinals above the previous block's bound and at most its own; the tail holds those
  above the last bound. Blocks are cut and merged as in units-v9 §2.

## 4. Queries

BM25 is computed per ordinal, as before per unit. Hits are in descending score, ties in
**ascending ordinal** (the order the documents entered the index; units-v9 broke ties by unit
id). Only the ordinals a search returns, and in a search restricted to types those whose head it
reads, are resolved to units, with one `multi_get` of their `ORD` records per slice of the
ranking.

## 5. What `verify_units` checks (UNIT-49)

In addition to units-v9: every posting's ordinal has an `ORD` record; every `ORD` names a unit
whose `DOC` has that ordinal; every `DOC`'s ordinal has the `ORD` of its unit; every ordinal is
below `STATS.next_ord`. A record of these families that does not decode is a structure finding
(UNIT-3, UNIT-6 to UNIT-13) and the comparison is skipped.
