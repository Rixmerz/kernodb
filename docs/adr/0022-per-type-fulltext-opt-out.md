# ADR-0022 — Per-type opt-out of the keyword index

| Field | Value |
|---|---|
| Status | **Proposed**. Implemented on branch `ccr-5d0e38e9-1hmqh0`. It adds a record family to the on-disk format (with layer 7), so it waits for the owner's acceptance, like ADR-0021 |
| Date | 2026-11-20 |
| Deciders | Project owner (pending) |
| Related to | [ADR-0021](0021-typed-schemas-and-secondary-indexes.md) (layer 6, the precedent for a record family per unit type); [ADR-0013](0013-format-v1-and-engine-protocol.md) (engine format v1, frozen, not edited); [units-v6](../file-format/units-v6.md) (not edited); [units-v7](../file-format/units-v7.md) (the layer 7 mark and the record); [Kerno vs SQLite from Python](../testing/python-vs-sqlite.md) (the measurement) |

## Context

The keyword index (`fulltext`, BM25) indexes the head payload of **every** `Active` unit whose
bytes are valid UTF-8, in the commit that writes it. There is no way to turn it off. An
application that stores structured text it never searches by keyword (JSON records, code
metadata) pays for an index nobody reads. Measured on 9,000 JSON payloads of about 250 bytes
each (`docs/testing/python-vs-sqlite.md`):

| | store 9,000 | update 9,000 | disk |
|---|---|---|---|
| indexed (today) | 0.93–0.99 s | 1.12–1.19 s | 35.1 MB |
| not indexed | 0.13–0.24 s | 0.16–0.19 s | 10.0 MB |

The first integration (LiveSpec's history store) worked around it by compressing its payloads,
so they stop being UTF-8. That works, but it costs the reader a decompression and makes the
payload opaque to every other tool.

Two constraints rule out a simple open-time option:

* **`verify_units` (UNIT-49) must know what the index should hold.** It expects every `Active`
  text unit to be indexed. An option that lives only in the memory of one open would make a
  correct database fail verification. It could also leave entries that a later open, with the
  option off, never removes.
* **The index must never disagree with the units.** It is written in the same transaction as
  the head, and a reader trusts it.

## Decision

1. **One record per unit type with options**, tag `0x77` (unassigned until now; units-v6 §2),
   key `0x77 ‖ type_len: u8 ‖ unit_type` (the shape of a `SCHEMA` key), value
   `envelope_version (1) ‖ reserved_flags (0) ‖ options: u8`. Bit 0 is `NO_FULLTEXT`. The other
   bits are reserved and must be 0. An all-zero byte is never written: turning the option off
   again deletes the record.
2. **An option changes only while the type has no `Active` unit**, as the change set sees it (the
   live count of layer 5, or a count of the heads for a legacy type). Only `Active` units are in
   the keyword index. At that moment the type has no index entry, and from then on every write
   follows the option in force. The index therefore holds exactly the `Active` units of the
   types without `NO_FULLTEXT`.
3. **The writer** (`plan_fulltext`) keeps no document for a unit whose type has `NO_FULLTEXT`, and
   still removes an old one (there is none, by point 2, but the path is the general one).
4. **`verify_units`** reads the `0x77` records and drops the units of those types from what
   UNIT-49 expects. An index entry left for one of them is then reported like any stray entry.
   A `0x77` record in a database whose mark is below 7 is a UNIT-49 finding.
5. **Layer 7.** The first `0x77` record raises the `LAYER_META "layer"` mark to 7 in its own
   transaction, and the mark is never lowered (the precedent of units-v5 and units-v6). A build
   of layer 6 or older refuses such a database (UNIT-16) before it can write: it cannot index a
   type that opted out, nor skip one that did not.
6. **API:** `ChangeSet::set_fulltext(type, enabled)`, `UnitDb::set_fulltext`,
   `Db::set_fulltext`, and Python `Database.set_fulltext(type, enabled)`. Setting the state the
   type already has writes nothing.

## Options considered

1. **An open-time option** (`UnitConfig`). Rejected: it breaks UNIT-49, and it lets two opens
   disagree about one database (see Context).
2. **A field of the `SCHEMA` record** (ADR-0021). Rejected: it would force a schema on every
   type that only wants to skip the index, and change a record layout that is already pinned.
3. **An exclusion by payload shape** (for example, "not indexed when it starts with `{`").
   Rejected: implicit, surprising, and still not visible to `verify_units`.
4. **This decision**: a record per type, persisted, changed only while the type is empty.

## Consequences

* Writes of an opted-out type cost what a non-UTF-8 payload costs today (4–6x faster than
  indexed in the measurement above), and its store is about 3.5x smaller.
* A database that uses the option can no longer be opened by builds of layer 6 or older (by
  design, point 5). One that never uses it keeps its mark, byte for byte.
* To change the option on a type that has units, the caller archives or deletes them first, or
  writes to a new type. This is deliberate. A rebuild of the index for one type is not provided.
* `fulltext/mod.rs` had reserved "layer version 7" for a future mark of its own records.
  That plan now takes layer 8.

## Tests

`crates/kerno/tests/batch_ext.rs`: an opted-out type is not found by `search_text`, the
setting survives a reopen, `verify` stays clean (also after archive / unarchive across a
change of the option), a change is refused while the type has active units, a layer 6 reader
refuses a layer 7 database (UNIT-16), and the record layout is pinned. `crates/kerno-units`
passes with the new tag (the "newer layer" probe in `tests/verify.rs` moved from `0x77` to the
reserved `0x7A`).
