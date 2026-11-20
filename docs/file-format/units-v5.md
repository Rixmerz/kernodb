# Unit layer v5: live-unit counts by type

| Field | Value |
|---|---|
| Status | Normative since package P5 after Stage 9 (closes defect K2 of [stage9-k-e3-7.md](../architecture/stage9-k-e3-7.md)). Format change approved by the owner on 2026-11-14 (option A of [post9-pending.md](../architecture/post9-pending.md), point 2) |
| Layer version | `layer_version = 5`, `envelope_version = 1` |
| Related to | [units-v1](units-v1.md), [v2](units-v2.md), [v3](units-v3.md) and [v4](units-v4.md) (not edited; the reservation of v4 §3 for the tags `0x73`–`0x7F` is lifted here only for `0x73`), [engine v1 format](v1.md) (frozen, no changes), [fixtures](../architecture/stage9-fixtures.md) §6 |

This document specifies, byte by byte, what the unit layer adds in version 5: a count record for each unit type (`unit_type`) with the number of `Active` units of that type. As in the earlier versions it is an **ordinary key and value** of the engine: no byte of the engine's v1 format changes. "Must" and "must not" are normative.

## 1. Why

The `IDX_TYPE` entries (`0x52`) are read with a range scan, and the total of `IDX_META` (`0x50`) only stores the whole primary index. A lost entry leaves no trace: a per-type query through the `IDX_TYPE` path returned one unit fewer, without error (defect K2), and a lost `IDX_PRIMARY` entry went unnoticed if the `IDX_META` total was also lowered until it matched what was left. Nothing said how many units a type has except the entries themselves. The count is the mark against which the reader compares what it walked, as [units-v4.md](units-v4.md) does for adjacency.

**Why a record of its own and not a field of `IDX_META`.** `IDX_META` is written only by an index rebuild, and its watermark defines freshness (I-23): if the writers rewrote it on every commit, the index could never be `Ready`. The count, by contrast, is maintained by the writers whether or not an index exists, so it is an independent witness of what the index must contain. Moreover, a value with a type map would grow without limit and force reading it whole to touch one type; a record per type is read with a point read.

## 2. What version 5 of the layer is

The precedent of [units-v4.md](units-v4.md) §2 is followed:

- The `LAYER_META "layer"` mark carries `layer_version = 5` **if and only if** the database has or had a `0x73` record. A database with none keeps the mark it had (1, 2, 3 or 4).
- Version 5 is written **in the same transaction** as the first `0x73` record, and **is never lowered**. Whoever rewrites the mark keeps its `history_policy`. Version 5 includes everything in 4.
- **Consequence for the earlier builds.** A layer 1 to 4 reader reads the mark before any other key and rejects the whole database: `IncompatibleLayer` with rule UNIT-16 and `found_layer_version = 5`. It writes nothing. As almost every database creates some unit, **every database with a unit created by this build stops opening with a build without this version**. A reader of this version rejects `layer_version > 5`.
- A mark with `layer_version < 5` and `0x73` keys is corruption (UNIT-48).
- **A database written before this layer is not migrated.** Opening writes nothing (I-18). The count of a type appears when a write moves a unit of that type into or out of `Active` (§5). Until then the database is read exactly as before.

## 3. Keys

| Tag | Name | Fields after the tag | Length |
|---|---|---|---|
| `0x73` | `TYPE_COUNT` | `type_len[1]` ‖ `unit_type` (`type_len` = 1..=64 B) | 3..=66 |

- `unit_type`: UTF-8 and non-empty, as in `UNIT_HEAD`. **It may contain NUL**: unlike a relation type, a unit type does not forbid it, and so the key carries the length prefix (as `IDX_TYPE`, [units-v1.md](units-v1.md) §3.11) and not a terminator.
- A total length other than `2 + type_len`, or outside 3..=66, is UNIT-3. An empty `unit_type` or one that is not UTF-8 is UNIT-12.
- The keys are kept out of `Key` in `codec.rs`, as are those of v3 and v4; `verify_units` reads them separately. The tags `0x74`–`0x7F` remain reserved (UNIT-2).

## 4. Values

The value is **10 bytes**: the common header and a big-endian `u64`, the same shape as [units-v4.md](units-v4.md) §4.

| # | Field | Type | Constraint |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` (other value: UNIT-8) |
| 3 | `count` | `u64` | `>= 0`: the number of `Active` units of that type |

**Zero is stored.** Unlike the adjacency count, a type that once had units keeps its record with `count = 0`. Thus an absent record always means "type never counted" (a database from before this layer), and the writer does not walk the headers again for a type it already knows. The cost is a few bytes per type that has existed.

No spare bytes (UNIT-10). Evaluation order: the value header before the key (UNIT-6, UNIT-7, UNIT-8), then the key (UNIT-3, UNIT-12), finally the value length (UNIT-9, UNIT-10).

## 5. Semantics

**What it counts.** The number of units with `lifecycle = Active` and that `unit_type`: exactly those that have an entry in `IDX_PRIMARY` and `IDX_TYPE` when the index is `Ready`. Archived and deleted units do not count. The count is per type, not per `(type, schema_version)`: changing the schema version of a unit (`update`, `restore`) does not touch it.

**Writer.** The count changes in the same transaction as the header of the unit, on the count **as the change set sees it** (several operations in one compose):

| Operation | Effect |
|---|---|
| `create` | +1 to the type |
| `archive` | −1 |
| `unarchive` | +1 |
| `delete` of an `Active` unit | −1 |
| `delete` of an `Archived` unit | none (it already left the count) |
| `update`, `restore` | none (the type and `Active` do not change) |
| retention, `reclaim`, `compact` | none (they do not touch the set of active units) |
| logical import | +1 for each unit whose final header is `Active`, through the same write path |

A change set that is discarded, that is closed by its limit or whose operation is rejected leaves no count: the count is planned with reads **after** all the checks that can reject the operation and **before** the first write (in P1 a rejected creation left orphan counters; here it cannot happen). If the record does not exist, the writer first counts the `Active` headers of that type (a walk of the whole `UNIT_HEAD` range, once per type and database) and starts from that number; this is the case of a database from before this layer and of a type that never had units.

Cost: one point read and **one more write** for each `create`, `archive`, `unarchive` and `delete` (in the same engine batch, which counts that write against its operation limit); the start-up walk happens once per type.

**Reader.**

- *`IDX_TYPE` path* (filter with `unit_type`): **one more point read**, that of the record of that type. If it exists, the walk covers **the whole range of the type**, even if the filter fixes `schema_version` (entries of other versions are skipped by their key, without opening the unit), and the number of walked entries must be the count. Without a record, the walk is the requested one, as before.
- *`IDX_PRIMARY` path*: **one more range scan**, over all the `0x73` records (one per type). The path already visits all the entries whatever the filter; it counts them per type and compares each type that has a record. This sees case K2: the lost entry, with the `IDX_META` total lowered to match.

A type with a record whose number of walked entries is different (more or fewer, **zero included**) is `Corruption`. `query` then answers by the reference path with `Fallback` `Corrupt`, and the answer is complete. A type with entries and no record **is not a finding**: it is indistinguishable from a database from before this layer. The check of `IDX_META` (total of entries) remains as it was.

**Verifier** (`verify_units`):

| Id | Rule | Class |
|---|---|---|
| UNIT-47 | A `0x73` record whose count is not the number of `Active` headers of its type (also a record greater than zero of a type with no unit at all) | `Corruption` |
| UNIT-48 | `0x73` records in a database whose mark has `layer_version < 5` | `Corruption` |

UNIT-47 compares with the **headers**, not with the index entries: the index may legitimately be `Stale` or not yet built. UNIT-37 and UNIT-38 remain the rules for missing entries; UNIT-47 explains why the reading is refused. A record that does not decode is a structure finding and is not compared; with a header that does not decode, the comparison is skipped (it is not known which type it is).

## 6. Vectors

`crates/kerno-units/tests/type_counts.rs` (`vectors_of_the_document_match_the_codec_byte_for_byte`) reads this table, builds the `ok` rows with the production writer and compares them **byte by byte**, and decodes each negative row and checks the rule. The test fixes the counts (4 positive, 11 negative, 1 positive mark and 2 negative).

| Name | Key (hex) | Value (hex) | Result |
|---|---|---|---|
| `key_doc_3` | `7303646f63` | `01000000000000000003` | ok |
| `key_a_0` | `730161` | `01000000000000000000` | ok |
| `key_nul_7` | `7303610062` | `01000000000000000007` | ok |
| `key_utf8_300` | `7302c3a9` | `0100000000000000012c` | ok |
| `key_unit3_tag_only` | `73` | `01000000000000000001` | UNIT-3 |
| `key_unit3_no_type` | `7300` | `01000000000000000001` | UNIT-3 |
| `key_unit3_length_over` | `7303646f` | `01000000000000000001` | UNIT-3 |
| `key_unit3_length_under` | `7302646f63` | `01000000000000000001` | UNIT-3 |
| `key_unit3_long_type` | `73416161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161616161` | `01000000000000000001` | UNIT-3 |
| `key_unit12_not_utf8` | `7301ff` | `01000000000000000001` | UNIT-12 |
| `value_unit6_env0` | `7303646f63` | `00000000000000000003` | UNIT-6 |
| `value_unit7_env2` | `7303646f63` | `02000000000000000003` | UNIT-7 |
| `value_unit8_flags` | `7303646f63` | `01010000000000000003` | UNIT-8 |
| `value_unit9_short` | `7303646f63` | `0100000000` | UNIT-9 |
| `value_unit10_trailing` | `7303646f63` | `0100000000000000000300` | UNIT-10 |

Layer marks. The "Reader" column is the maximum `layer_version` that the reader knows:

| Name | Bytes (hex) | Reader | Result |
|---|---|---|---|
| `mark_v5` | `01000000000502` | `5` | ok |
| `mark_v5_layer4_reader` | `01000000000502` | `4` | UNIT-16 |
| `mark_v5_layer3_reader` | `01000000000502` | `3` | UNIT-16 |

## 7. Limits (normative: what the count does not see)

1. **Type from before layer 5.** It has no count until a write moves a unit of that type into or out of `Active`. Until then the reading behaves as before: a lost `IDX_TYPE` entry **remains invisible** from the `IDX_TYPE` path. Unlike [units-v4.md](units-v4.md) §7.1, when the first count of an old type is created it starts from the **headers** and not from the entries: an `IDX_TYPE` entry already lost is not subtracted, and from then on the per-type reading sees it (the count says one unit more than the entries). Nothing is written on opening (I-18), so there is no block migration; it is a design decision, and a migration would have to walk all the headers.
2. **Double damage.** A lost `IDX_TYPE` entry **together with** the count record of its type, or a count rewritten to match the lost entry, leave a reading by `IDX_TYPE` that agrees with itself and answers short. `verify_units` sees the missing entry (UNIT-38) and, if the count was rewritten, that it is not the headers' (UNIT-47); the `IDX_PRIMARY` path also sees the lowered count.
3. **A lost count record** with its entries intact is not a finding (it is case 1).
4. **A `Stale` index** is not compared: only a read over a `Ready` index (watermark equal to the snapshot's sequence) uses the count, because only then are the entries exactly the active units.
5. **A count and a header damaged at once** (a type changed in the header of a unit) are seen by the check of the path against the header, as before.

## 8. Fixture

`crates/kerno-units/tests/vectors/type-counts-v5/db-legacy-types/` is a `kerno_db` database closed with four units (`a/1`, `b/1`, `a/2` active and one archived `a/1`), the primary index and the per-type index built and `Ready`, as the **previous** writer, before this layer, left them: without a `0x73` record, mark 1. It was generated with the build of this layer and then, with the same damage method that the crate's tests use (`raw_commit`), the `0x73` records were removed and the mark returned to 1 and the `IDX_META` watermark to the last commit (`to_legacy` in `tests/harness/units.rs`); the bytes of the old writer are exactly those. `MANIFEST` lists the SHA-256, the size and the name of each file. The bytes are not reproduced between builds; the integrity is given by `MANIFEST`. It is regenerated with `cargo test --release -p kerno-units --all-features --test type_counts -- --ignored regenerate_fixture`.

With this layer, no build can write at once a layer 2, 3 or 4 database that has units: the tests of the earlier layers that build a database in memory leave it at its layer with `to_legacy`, and the directories of [derived-v2](units-v2.md), public-ids-v3 and rel-counts-v4 remain those written by the builds of their time.

## 9. History

| Date | Change |
|---|---|
| 2026-11-14 | P5: first version (record `0x73`, mark 5, UNIT-47 and UNIT-48). |
