# Unit layer v6: typed schemas and secondary indexes

| Field | Value |
|---|---|
| Status | **Proposed** with [ADR-0021](../adr/0021-typed-schemas-and-secondary-indexes.md): experimental, the records may change before 1.0 |
| Layer version | `layer_version = 6`, `envelope_version = 1` |
| Related to | [units-v5](units-v5.md) (the precedent, not edited), [engine v1 format](v1.md) (frozen, no changes) |

This document specifies the layer mark that goes with the schema records. The byte layout of the records themselves
(`0x78` `SCHEMA`, `0x79` `FIELD_INDEX`) is normative in ADR-0021 (Decision) and pinned by `record_layout_is_pinned` in
`crates/kerno-units/tests/schema.rs`. As in the earlier versions they are ordinary keys and values of the engine: no
byte of the engine's v1 format changes. "Must" and "must not" are normative.

## 1. What version 6 of the layer is

The precedent of [units-v5.md](units-v5.md) §2 is followed:

- The `LAYER_META "layer"` mark carries `layer_version = 6` **if and only if** the database has or had a `0x78`
  record. A database with none keeps the mark it had (1 to 5), byte for byte.
- Version 6 is written **in the same transaction** as the first `0x78` record, and **is never lowered**. A change set
  dropped before commit leaves the mark where it was. Whoever rewrites the mark keeps its `history_policy`. Version 6
  includes everything in 5.
- **Consequence for the earlier builds.** A layer 1 to 5 reader reads the mark before any other key and rejects the
  whole database: `IncompatibleLayer` with rule UNIT-16 and `found_layer_version = 6`. It writes nothing, so it cannot
  write a unit that skips validation or leaves a stale index entry. A reader of this version rejects
  `layer_version > 6`.
- **A database written before this layer is not migrated.** Opening writes nothing (I-18).

## 2. Tags

`0x78` and `0x79` are assigned by ADR-0021. `0x77` stays unassigned and `0x7A`–`0x7F` stay reserved (UNIT-2).
