# Unit layer v4: adjacency counters

| Field | Value |
|---|---|
| Status | Normative since package P1 after Stage 9 (closes defect K1 of [stage9-k-e3-7.md](../architecture/stage9-k-e3-7.md)). Design decision taken by delegation; revocable |
| Layer version | `layer_version = 4`, `envelope_version = 1` |
| Related to | [units-v1](units-v1.md), [v2](units-v2.md) and [v3](units-v3.md) (not edited except a note in v3 §3), [engine v1 format](v1.md) (frozen, no changes), [fixtures](../architecture/stage9-fixtures.md) §6 |

This document specifies, byte by byte, what the unit layer adds in version 4: a count record for each `(unit, direction, relation type)` that has adjacency entries. As in the earlier versions they are **ordinary engine keys and values**: no byte of the engine's v1 format changes. "Must" and "must not" are normative.

## 1. Why

The adjacency index (`REL_OUT` `0x41`, `REL_IN` `0x42`) is read with a range scan. A lost entry leaves no trace, so, asked from the end that lost it, `relations_of` returned a short answer without error, and `query_relations` did not fall back (defect K1). Nothing ties a unit to its relations except the entries themselves. The count is the mark against which the reader compares what it walked.

## 2. What version 4 of the layer is

The precedent of [units-v3.md](units-v3.md) §2 is followed:

- The `LAYER_META "layer"` mark carries `layer_version = 4` **if and only if** the database has or had a `0x72` record. A database with none keeps the mark it had (1, 2 or 3).
- Version 4 is written **in the same transaction** as the first `0x72` record, and **is never lowered**. Whoever rewrites the mark keeps its `history_policy`. Version 4 includes everything in 3.
- **Consequence for the earlier builds.** A layer 1, 2 or 3 reader reads the mark before any other key and rejects the whole database: `IncompatibleLayer` with rule UNIT-16 and `found_layer_version = 4`. It writes nothing. A database with a single relation created by this build stops opening with a build without this version. A reader of this version rejects `layer_version > 4`.
- A mark with `layer_version < 4` and `0x72` keys is corruption (UNIT-46).
- Opening writes nothing: the first created or deleted relation that writes a count raises the mark.

## 3. Keys

| Tag | Name | Fields after the tag | Length |
|---|---|---|---|
| `0x72` | `REL_COUNT` | `unit_id[16]` ‖ `direction[1]` ‖ `relation_type` (1..=64 B) ‖ `0x00` | 20..=83 |

- `direction`: `0` for the `REL_OUT` entries of the unit, `1` for the `REL_IN` ones. Any other value is UNIT-12.
- `relation_type` with the same rules as in the adjacency key (UTF-8, no NUL, 1..=64 bytes: UNIT-12). The final NUL separates it; a missing final NUL or a length out of range is UNIT-3.
- The key order groups keys by unit and direction: **all the counts of one unit and direction form a range**.
- Tags `0x73`–`0x7F`: reserved (UNIT-2). The keys are kept out of `Key` in `codec.rs`, as are those of v3; `verify_units` reads them separately.

## 4. Values

The value is **10 bytes**: the common header and a big-endian `u64`.

| # | Field | Type | Constraint |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` (other value: UNIT-8) |
| 3 | `count` | `u64` | `>= 1`. A count that reaches zero **is deleted**: `0` is not stored (UNIT-12) |

No spare bytes (UNIT-10). Evaluation order: the value header before the key (UNIT-6, UNIT-7, UNIT-8), then the key (UNIT-3, UNIT-12), finally the value length (UNIT-9, UNIT-10) and the range of the count (UNIT-12).

## 5. Semantics

**Writer.** Creating a relation adds 1 to the count of `(from, Out, type)` and of `(to, In, type)`; deleting it subtracts 1 from the same. It is done in the same transaction as the relation and its entries, on the count **as the change set sees it** (several operations in one compose). If the record does not exist, the writer first counts the entries in the range of that `(unit, direction, type)` and starts from that number (this is the case of a database from before layer 4). Cost: one point read and one write per adjacency entry written or deleted; the start-up walk happens only once per `(unit, direction, type)`. The logical import writes the relations through the same path and leaves the same counts.

**Reader.** `relations_of` reads the counts that its entries expect and compares them with what it walked, per type:

- with a given type: **one more point read** (the record of that type);
- without a type: **one more range scan**, over the counts of that unit and direction (one per distinct type, normally a single scan call).

A type with a record whose number of walked entries is different (more or fewer, **zero included**) is `Corruption`. `query_relations` then answers by the reference path with `Fallback` `Corrupt`, and the answer is complete. A type with entries and no record **is not a finding**: it is indistinguishable from a database from before layer 4.

**Verifier** (`verify_units`):

| Id | Rule | Class |
|---|---|---|
| UNIT-45 | A `0x72` record whose count is not the number of adjacency entries of its unit, direction and type (also a record with no entries at all) | `Corruption` |
| UNIT-46 | `0x72` records in a database whose mark has `layer_version < 4` | `Corruption` |

UNIT-36 remains the rule for a relation without its two entries; UNIT-45 is the one that explains why the reading is refused. A record that does not decode is a structure finding and is not compared.

## 6. Vectors

`crates/kerno-units/tests/rel_counts.rs` (`vectors_of_the_document_match_the_codec_byte_for_byte`) reads this table, builds the `ok` rows with the production writer and compares them **byte by byte**, and decodes each negative row and checks the rule. Common inputs: unit `0102…10`. The test fixes the counts (3 positive, 13 negative, 1 positive mark and 2 negative).

| Name | Key (hex) | Value (hex) | Result |
|---|---|---|---|
| `key_out_cites_1` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `01000000000000000001` | ok |
| `key_in_cites_2` | `720102030405060708090a0b0c0d0e0f1001636974657300` | `01000000000000000002` | ok |
| `key_out_refines_300` | `720102030405060708090a0b0c0d0e0f1000726566696e657300` | `0100000000000000012c` | ok |
| `key_unit3_tag_only` | `72` | `01000000000000000001` | UNIT-3 |
| `key_unit3_no_type` | `720102030405060708090a0b0c0d0e0f100000` | `01000000000000000001` | UNIT-3 |
| `key_unit3_no_nul` | `720102030405060708090a0b0c0d0e0f10006369746573` | `01000000000000000001` | UNIT-3 |
| `key_unit3_long_type` | `720102030405060708090a0b0c0d0e0f1000616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616100` | `01000000000000000001` | UNIT-3 |
| `key_unit12_direction` | `720102030405060708090a0b0c0d0e0f1002636974657300` | `01000000000000000001` | UNIT-12 |
| `key_unit12_type_nul` | `720102030405060708090a0b0c0d0e0f1000636900657300` | `01000000000000000001` | UNIT-12 |
| `key_unit12_type_not_utf8` | `720102030405060708090a0b0c0d0e0f1000ff00` | `01000000000000000001` | UNIT-12 |
| `value_unit12_zero_count` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `01000000000000000000` | UNIT-12 |
| `value_unit6_env0` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `00000000000000000001` | UNIT-6 |
| `value_unit7_env2` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `02000000000000000001` | UNIT-7 |
| `value_unit8_flags` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `01010000000000000001` | UNIT-8 |
| `value_unit9_short` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `0100000000` | UNIT-9 |
| `value_unit10_trailing` | `720102030405060708090a0b0c0d0e0f1000636974657300` | `0100000000000000000100` | UNIT-10 |

Layer marks. The "Reader" column is the maximum `layer_version` that the reader knows:

| Name | Bytes (hex) | Reader | Result |
|---|---|---|---|
| `mark_v4` | `01000000000402` | `4` | ok |
| `mark_v4_layer3_reader` | `01000000000402` | `3` | UNIT-16 |
| `mark_v4_layer2_reader` | `01000000000402` | `2` | UNIT-16 |

## 7. Limits (normative: what the count does not see)

1. **Database written before layer 4.** Its relations have no count until a relation of that `(unit, direction, type)` is created or deleted. Until then the reading behaves as before: a lost entry **remains invisible** from its end. When the first count of an old unit is created, an entry already lost is counted as absent, so that the earlier loss remains invisible from that end afterwards. Nothing is written on opening (I-18), so there is no block migration; it is a design decision, and a migration would have to walk all the relations.
2. **Double damage.** A lost entry **together with** its count record, or a count rewritten to match the lost entry. `verify_units` sees the missing entry (UNIT-36) but not that the count disappeared with it.
3. **A lost count record** with its entries intact is not a finding (it is case 1).
4. Damage inside the value of an entry is detected by the check against its `REL`, as before.

## 8. Fixture

`crates/kerno-units/tests/vectors/rel-counts-v4/db-legacy-relations/` is a `kerno_db` database closed with three units and three relations as the **previous** writer, before this layer, left them: `REL` and its two adjacency entries, without a count record, mark 1. It was generated by writing those three records per relation with the same damage method that the crate's tests use (`raw_commit`), not with an earlier build: the bytes of the old writer are exactly those. `MANIFEST` lists the SHA-256, the size and the name of each file. The bytes are not reproduced between builds; the integrity is given by `MANIFEST`. It is regenerated with `cargo test --release -p kerno-units --all-features --test rel_counts -- --ignored regenerate_fixture`. Details in [stage9-fixtures.md](../architecture/stage9-fixtures.md) §6.

## 9. History

| Date | Change |
|---|---|
| 2026-11-13 | P1: first version (record `0x72`, mark 4, UNIT-45 and UNIT-46). |
