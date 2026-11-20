# ADR-0021 — Typed schemas per unit type and user-defined secondary indexes

| Field | Value |
|---|---|
| Status | **Proposed**, implemented in Phase C of the [roadmap](../../ROADMAP.md). It adds record families to the on-disk format (with layer 6, see Option 4), so it waits for the owner's acceptance like the other format changes did |
| Date | 2026-11-18 |
| Deciders | Project owner (pending) |
| Related to | [ADR-0007](0007-index-consistency-and-rebuilding.md) (index consistency; this ADR departs from its owner decision for one family of indexes, see [Decision](#decision)); [ADR-0014](0014-physical-representation-of-relations.md) (adjacency indexes, transactional); [ADR-0003](0003-physical-format-and-versioning.md) (versioning); [ADR-0013](0013-format-v1-and-engine-protocol.md) (engine format v1, frozen, not edited); [units-v5](../file-format/units-v5.md) (count records, the precedent for a layer bump; not edited); [units-v6](../file-format/units-v6.md) (the layer 6 mark); invariants I-21, I-23, I-24 |

## Context

A unit's payload is an unshaped byte block. Nothing says which fields a unit type has, which are required,
what type they are or whether two units may hold the same value, and the only indexes are the type and the
relation ones. Phase C of the roadmap asks for typed schemas per unit type (required fields, field types,
uniqueness, validation on write, invalid writes rejected transactionally) and secondary indexes on any field,
kept in sync with the units and covered by `verify_units`, exposed through the `kerno` facade.

Two facts shape the design:

* **Uniqueness cannot be enforced from a stale index.** ADR-0007 made `IDX_PRIMARY` and `IDX_TYPE` *derived*
  indexes with a freshness state and a rebuild. A unique constraint checked against an index that is `Stale` after
  every commit would be a constraint that holds only when someone rebuilt in time.
* **The layer already has a transactional pattern.** The adjacency indexes ([ADR-0014](0014-physical-representation-of-relations.md)),
  the live-unit counts (layer 5) and the full-text index write their records in the same engine transaction as the
  unit's head, planned by reads before the first write of the operation.

## Options considered

### 1. How an index relates to its units

1. **Transactional: entries are written in the transaction that writes the unit** (chosen). An index never
   disagrees with its units across a crash or a reopen (I-21), uniqueness is checked against what the change set
   itself sees, and there is no freshness state to report.
2. Derived with freshness and a rebuild, like `IDX_PRIMARY`. Rejected: no uniqueness (see Context), and the owner
   decided on 2026-11-05 not to maintain unit indexes inside change sets *for `IDX_PRIMARY` and `IDX_TYPE`* without a
   workload showing the reference path is not enough. This ADR does not touch those two; it adds a family whose
   contract needs the opposite.

The cost is the one K-E3-9 already named: one extra write per indexed field per write of a unit, in the same
engine batch (and counted against its limits). A type with no schema pays one point read that misses.

### 2. What the payload of a typed unit is

1. **A canonical binary document** defined by this crate (chosen): named fields, five types, one encoding per
   value. No new dependency (a JSON parser is only an optional dependency of the embedding client, and the
   lockfile is not changed by this work).
2. JSON or another self-describing format. Rejected for now: it needs a parser in the default build and has no
   canonical form, so two equal values could have different bytes and different content hashes.

### 3. Where schemas and indexes are declared

1. **One `SCHEMA` record per unit type, whose fields carry the `indexed` flag** (chosen). The one point read a
   write needs for validation also says which fields to index. An index definition is a field flag, so there is no
   second family of definitions to keep consistent.
2. A separate definition record per index. Rejected: a second read per write and a second thing to verify, for no
   capability the flag lacks.

### 4. Layer mark

1. No bump. Rejected: a build that predates schemas would open the database and write units without validating
   them (the risk described below), as the full-text index does today (its `ponytail:` says so).
2. **Layer version 6, written in the transaction of the first `SCHEMA` record, as layer 5 was for the count records**
   (chosen). A build that does not know schemas refuses the database instead of writing units that skip validation
   and leave stale entries behind. A first version of this change built it and removed it, because
   `stats_counts_units_and_relations_by_type_and_names_the_versions_and_files` in `kerno-cli` compared
   `layer_version` with `MAX_LAYER_VERSION` (`left: 5, right: 6`), and `kerno-cli` was outside that change's surface.
   The bump and the test fix now land together.

**Risk this left, resolved.** With no bump, a build that predates schemas would have opened a database with a
schema without complaint and written units without validating them or maintaining their entries. Option 2 closes it:
`LAYER_VERSION_SCHEMAS = 6` is raised by the first `SCHEMA` record and `MAX_LAYER_VERSION` is 6, so a build that knows
only layers up to 5 refuses a layer 6 database (`IncompatibleLayer`, UNIT-16) and writes nothing. The `kerno-cli` test
now compares `layer_version` with the layer its fixture was written at (5). Layer 6 is now taken by schemas: the
full-text index, which named layer 6 as its own upgrade path, would need layer 7.

## Decision

**Typed schemas and user indexes are transactional records that raise the layer mark to 6**, defined as follows. The key and
value layouts below are normative; they are pinned byte for byte by `record_layout_is_pinned` in
`crates/kerno-units/tests/schema.rs`.

### Document (the payload of a typed unit)

`document ::= 0x01 ‖ n: u16 BE ‖ n × field`; `field ::= name_len: u8 ‖ name ‖ type: u8 ‖ value`.

* Names are 1..=64 bytes of UTF-8 with no NUL, in **strictly ascending byte order**, so a document has exactly one
  encoding. At most 64 fields.
* Type tags and values: `1` Bool (one byte, `0` or `1`); `2` Int (`i64` BE); `3` Float (`f64` BE, never NaN);
  `4` Text (`u32` BE length, UTF-8); `5` Bytes (`u32` BE length).
* Any other byte, an unordered or repeated name, a NaN, trailing bytes or a short read: not a document.

### `SCHEMA` record (`0x78`)

* Key: `0x78 ‖ type_len: u8 ‖ unit_type` (`type_len` 1..=64; length-prefixed because a unit type may hold a NUL).
* Value: `envelope[2] ‖ n: u16 BE ‖ n × (name_len: u8 ‖ name ‖ type: u8 ‖ flags: u8)`, fields in strictly ascending
  name order. `envelope` is `0x01 0x00`. `flags`: bit 0 `required`, bit 1 `unique`, bit 2 `indexed`; any higher bit,
  or `unique` without `indexed`, is invalid.
* A type has at most one schema, defined once. After that only the `indexed` flag of a non-unique field changes.

### `FIELD_INDEX` entry (`0x79`)

* Key: `0x79 ‖ type_len ‖ unit_type ‖ name_len ‖ name ‖ value ‖ unit[16]`.
* Value: `0x01 0x00` (the envelope and nothing else).
* One entry per `Active` unit that has the field, for every field whose `indexed` flag is set. A unit that is
  archived, deleted or lacks the field has none.
* `value` is order-preserving and self-delimiting, so a range of keys is the units in `(value, unit id)` order and
  no value's encoding is a prefix of another's:

| Type | Encoding of `value` |
|---|---|
| Bool | one byte, `0` or `1` |
| Int | the `i64` BE with the sign bit flipped |
| Float | the `f64` bits BE, all bits flipped when the sign bit is set and the sign bit set otherwise; `-0.0` is stored as `0.0` |
| Text, Bytes | each byte as is, except `0x00` written `0x00 0xFF`; then the terminator `0x00 0x00` |

* An indexed value whose encoding exceeds 256 bytes is refused, so a key is at most 403 bytes and always fits the
  engine's 512-byte key limit.

`0x77` stays unassigned: `crates/kerno-units/tests/verify.rs` uses it as its "tag of a newer layer" (UNIT-2) sample.
The records live outside the `Key` enum of `codec.rs`, like layers 3, 4, 5 and the full-text index.

### Layer mark and compatibility

* The layer mark is raised to `LAYER_VERSION_SCHEMAS = 6` **in the transaction of the first `SCHEMA` record** (a
  change set dropped before commit leaves it where it was) and is never lowered. Whoever rewrites the mark keeps its
  `history_policy`. Layer 6 includes everything in 5. `MAX_LAYER_VERSION` is 6. See
  [units-v6](../file-format/units-v6.md).
* A database with no schema is byte-for-byte what it was, mark included; a write of any unit pays one point read of
  the schema key, which misses when the type has none.
* A build that knows only layers up to 5 refuses a layer 6 database when it opens it (`IncompatibleLayer`, UNIT-16,
  `found_layer_version = 6`), as it refuses any newer layer, and writes nothing.
* A database written before this change is not migrated; opening writes nothing.

### Behaviour

* **Validation on write.** `create`, `update` and `restore` check the new payload; `unarchive` checks it again,
  because the schema or another unit's unique value may have changed since the archive. A payload that is not a
  document, has an undeclared field, a value of the wrong type, lacks a required field, or has an indexed value too
  long is refused with `UnitError::SchemaViolation`. A restore is checked against the schema in force now, as it
  already is against the payload limit.
* **Uniqueness is among `Active` units.** Archiving or deleting a unit frees its value. The check reads the
  change set's own writes, so two creates of one value in one change set collide, and a unit never collides with
  itself.
* **A refused write stores nothing.** The check and the entry writes are planned by reads before the first write of
  the operation (the path of `plan_live` and `plan_fulltext`): the change set stays usable, and dropping it stores
  nothing of it. A change set that commits after a refusal commits the operations that were accepted.
* **Defining or indexing over existing units.** `define_schema` validates every unit of the type that is not deleted
  (archived ones included, so `unarchive` can only fail on uniqueness) and writes the entries of the `Active` ones in
  the change set that carries the definition; a unit the schema refuses, or a repeated unique value, refuses the
  whole definition and stores nothing, not even the mark. `create_index` backfills one field; `drop_index` removes
  the field's entries with a range delete. A unique field keeps its index. A definition with no unit in its change
  set has purpose `Index`.
* **Reads.** `find_by_field` and `find_range` read the entries of one field in `(value, unit id)` order and open each
  unit through the verified read of `get_unit`, which must be `Active`, of the type, and hold the entry's value in its
  payload; otherwise `Corruption`. They never write.
* **`verify_units`** (and so `kerno verify` and `kerno verify --deep`, which both run it) adds two rules:

| Rule | Class | Finding |
|---|---|---|
| UNIT-50 | Corruption | An entry is missing, extra, or does not match the payload or lifecycle of its unit; or entries exist for a field with no index flag |
| UNIT-51 | Corruption | A unit that is not deleted does not satisfy its schema, or two active units hold one value of a unique field |

  A record that does not decode is a structure finding (UNIT-3, UNIT-6 to UNIT-10, UNIT-12) and the comparison is
  not repeated on top of it. Findings name the schema record, unit ids and field positions, never a value.

## Evidence

* `crates/kerno-units/src/schema/tests.rs`: the order of every index encoding against the order of the values it
  encodes, and that none is a prefix of another; document and schema round trips; every damaged schema record names
  its rule.
* `crates/kerno-units/tests/schema.rs` (on `emulated_log` and `kerno_db`): refusals store nothing and leave the
  change set usable; uniqueness through update, archive, unarchive, delete and two creates in one change set; the index
  equals a scan of the payloads after every kind of write; ranges in value order with every bound kind; backfill and
  refusal of a definition; `create_index` and `drop_index`; no schema record without a definition and a definition
  that raises the layer mark to 6 in its own change set (dropped before commit it stays at 5), and a database
  without schema that keeps its layer;
  each way the entries can be wrong is a finding, and the read path refuses an entry that lies; the layout above.
* `crates/kerno-units/tests/schema_crash.rs`: a crash before every mutating I/O operation of a scenario that defines
  a schema, writes, adds an index to existing units, archives, unarchives, deletes and drops an index, with both crash
  kinds on both engines. After each recovery the store is byte for byte the one after a whole prefix of change sets
  (so a unit, its schema and its entries are all there or all absent), `verify_units` finds nothing, every index
  equals a scan of the payloads, a duplicate of a unique value is still refused and a new unit is accepted.
* `crates/kerno/tests/db.rs` and the crate documentation: the same through the facade, with a clean close and a drop
  without closing.
* Hand mutants, each turned red by a test: entries of the old payload not deleted; the uniqueness check skipped; the
  schema check skipped; validation restricted to a unit's first revision.

## Consequences

* **Easier.** Typed records, uniqueness and field lookups without a query language, and the guarantee that an index is
  never behind its units.
* **Harder.** Every write to a typed unit costs one read of the schema, one read of the unit's previous payload and one
  write per changed indexed field, in the same batch. A database that defines a schema must not be written by a build
  older than this one (see the risk in Option 4).
* **Committed.** The layouts above, and that a type has one schema for good.

### Limits (normative: what this does not do)

1. **A lost entry is invisible to a read.** A read checks each unit it returns against its entry; it cannot know an
   entry is missing. That is `verify_units` (UNIT-50), as it was for `IDX_TYPE` before layer 5. A per-index count like
   the one of units-v5 would close it for full scans; it is not done because it adds a write per change for no
   measured need.
2. **No schema evolution.** A schema is defined once; fields cannot be added or retyped. Changing one is a new unit
   type and a copy.
3. **Definitions are bounded by memory and the engine batch.** `define_schema` and `create_index` read every unit of
   the type into memory and write all entries in one change set; beyond the engine's batch limits they fail with
   `ChangeSetTooLarge` and store nothing.
4. **Export and import do not carry schemas.** The export format is not extended, and an import needs a new
   database, so a schema is defined after it, which validates and indexes the imported units. Retention rewrites
   heads without a payload and so never touches an entry.
5. **Equality and range only.** No multi-field index, no prefix or collation, no index on a computed value.
6. **A missing optional field is not indexed**, so `find_by_field` cannot ask for "has no value".

## Follow-up

* Owner decision on accepting this format change. The layer bump (Option 4) is done; the risk it closes is resolved.
* The full-text index names layer 6 as its upgrade path; layer 6 is taken, so that bump becomes layer 7.
* A measured workload before adding per-index counts, schema evolution or composite indexes.
