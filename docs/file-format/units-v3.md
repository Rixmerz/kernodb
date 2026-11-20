# Unit layer v3: public identifiers

| Field | Value |
|---|---|
| Status | Normative since E9-02, **merged on 2026-11-13** (squash `2697980`). It copies the decisions of [ADR-0005](../adr/0005-public-identifiers.md) (option 1), which prevails if there is a discrepancy. Decisions D-a to D-f of 2026-11-13, taken by express delegation from the owner |
| Layer version | `layer_version = 3`, `envelope_version = 1` |
| Related to | [unit layer v1](units-v1.md) (not edited) and [v2](units-v2.md) (edited only in three places, for the new maximum mark: the "Layer version" row of the header, the third point of §2 and a row of §8; this document only adds), [ADR-0005](../adr/0005-public-identifiers.md), [logical export](export-v0-draft.md), [engine v1 format](v1.md) (frozen, no changes) |

This document specifies, byte by byte, what the unit layer adds in version 3: the mapping between a public identifier (an opaque 128-bit token) and the unit it names. As in versions 1 and 2, they are **ordinary engine keys and values**: no byte of the engine's v1 format nor its list of feature bits changes. The compatibility boundary is the layer mark. The implementation (`crates/kerno-units/src/ids.rs`) and the test `vectors_of_the_document_match_the_codec_byte_for_byte` (`crates/kerno-units/tests/public_ids.rs`) are written only from this text and from those of the earlier versions. "Must" and "must not" are normative.

## 1. Conventions

Those of [units-v1.md](units-v1.md) §1: **big-endian** integers, the common two-byte header of the values (`envelope_version = 1`, `reserved_flags = 0`; UNIT-6, UNIT-7, UNIT-8) and a 16-byte `unit_id`. A `token` is 16 bytes (128 bits) from an operating-system random generator, unrelated to the `unit_id`, the clock or the creation order.

**Token text** (not stored; shown and validated by the API): `k1_` followed by 26 characters, the 128 bits in base32 RFC 4648, lowercase, no padding. The digit is the text version (`PUBLIC_ID_TEXT_VERSION = 1`), a constant distinct from the record version (`PUBLIC_ID_RECORD_VERSION`, the `envelope_version` of §4). The 26 characters carry 130 bits: the last two must be zero, and a text with them set is rejected (one text per id). Validation order, from the cheapest to the most expensive and before any lookup: prefix `k`, digit and `_`; version; body length; alphabet; final bits. Each failure has its own error (`PublicIdError`).

## 2. What version 3 of the layer is

The precedent of [units-v2.md](units-v2.md) §2 is followed (a layer that adds records raises the mark):

- The `LAYER_META "layer"` mark (units-v1 §3.1) carries `layer_version = 3` **if and only if** the database has or had a `0x70`/`0x71` record. A database with none keeps the mark it had (1 or 2): one with mark 1 is opened by any build of layer 1, 2 or 3; one with mark 2 by any build of layer 2 or 3 (one of layer 1 rejects it by its mark, units-v2 §2).
- Version 3 is written **in the same transaction** as the first `0x70`/`0x71` record. **It is never lowered**: not by `set_policy`, not by deleting the unit with a token, not by any later `ChangeSet`. Whoever rewrites the mark keeps its `history_policy`. Version 3 includes everything in 2: a layer 3 reader reads persisted derived data.
- **Consequence for the earlier builds.** A layer 1 or 2 reader reads the mark before any other key and rejects the whole database: `IncompatibleLayer` with rule UNIT-16 and `found_layer_version = 3`. It writes nothing. There is no partial reading: a database with a single token stops opening with a build without this version. A reader of this version rejects `layer_version > 3` with the same rule.
- A mark with `layer_version < 3` and `0x70`/`0x71` keys is corruption (UNIT-44).
- Opening writes nothing: the first token of a layer 1 or 2 database is what raises the mark.

## 3. Keys

| Tag | Name | Fields after the tag | Length |
|---|---|---|---|
| `0x70` | `PUBLIC_ID` | `token[16]` | 17 |
| `0x71` | `PUBLIC_ID_OF` | `unit_id[16]` | 17 |

- `0x70` is the index `token → unit`; `0x71` the inverse `unit → token`, which makes it possible to show again the token of a unit without scanning anything.
- Tags `0x72`–`0x7F`: reserved. A layer 3 that finds them rejects them with UNIT-2 (`IncompatibleLayer`). (Later note: [units-v4.md](units-v4.md) uses `0x72` for the adjacency counters, with mark 4; `0x73`–`0x7F` remain reserved.) In a layer 2 or earlier, the whole `0x7` nibble falls under UNIT-2.
- The keys are kept out of `Key` in `codec.rs`, as are the persisted derived data (`0x60`–`0x62`): `verify_units` reads them separately.

## 4. Values

Both values are **18 bytes**: the common header and 16 bytes.

| # | Field | Type | Constraint |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` (`PUBLIC_ID_RECORD_VERSION`) |
| 2 | `reserved_flags` | `u8` | `0`. Reserved for a future state of a revoked or reissued token; today any other value is UNIT-8 |
| 3 | `other` | `[u8; 16]` | in `0x70`, the `unit_id` that the token names; in `0x71`, the `token` of the unit |

No spare bytes (UNIT-10).

**Evaluation order (declared exception to units-v2 §6).** For these records the **value header is read before the key length**: `envelope_version` (UNIT-6, UNIT-7), `reserved_flags` (UNIT-8), key length (UNIT-3) and finally value length (UNIT-9, UNIT-10). So a future record with another token length is read as a newer layer (UNIT-7), not as damage. A value of less than two bytes is UNIT-9.

## 5. Validation rules

The per-record rules reuse those of units-v1: UNIT-3 (key length other than 17), UNIT-6, UNIT-7, UNIT-8, UNIT-9, UNIT-10 and UNIT-2 (extended: tags of nibble `0x4`–`0xF` outside `0x40`–`0x42`, `0x50`–`0x52`, `0x60`–`0x62` and `0x70`–`0x71`). UNIT-16 (extended): `layer_version` greater than the one the reader knows (3 in this version, 2 in a layer 2 build, 1 in a layer 1 build).

Rules between records, checked by `verify_units` (each one is its own finding; a record that does not decode is a structure finding and its counterpart is not reported again as missing):

| Id | Rule | Class |
|---|---|---|
| UNIT-39 | `0x70` record without the `0x71` of its unit | `Corruption` |
| UNIT-40 | `0x71` record without the `0x70` of the token it names | `Corruption` |
| UNIT-41 | A pair that names a unit without `UNIT_HEAD` (a logically deleted unit keeps its head; the deletion removes the pair, §7) | `Corruption` |
| UNIT-42 | Two tokens for one unit: another `0x70` names the same unit, or the `0x71` of the unit names another token | `Corruption` |
| UNIT-43 | One token for two units: the `0x70` of the token names another unit, or two `0x71` name the same token | `Corruption` |
| UNIT-44 | `0x70`/`0x71` records in a database whose mark has `layer_version < 3` | `Corruption` |

The texts of the findings name the record, never a token together with the `unit_id` of its unit (D-d). Resolution applies the same rule inline: `resolve_public_id` reads `0x70` and then `0x71`, and `public_id` reads `0x71` and then `0x70`; a pair that does not match is `Corruption`, and neither call returns a token that does not resolve (D-c).

## 6. Vectors

`crates/kerno-units/tests/public_ids.rs` (`vectors_of_the_document_match_the_codec_byte_for_byte`) reads this table, builds the `ok` rows with the production codec and compares them **byte by byte**, and decodes each negative row and checks the rule. Common inputs: unit `0102…10`, token `000102…0f` (text `k1_aaaqeayeaudaocajbifqydiob4`). The test fixes the counts (2 positive, 15 negative, 1 positive mark and 3 negative).

| Name | Key (hex) | Value (hex) | Result |
|---|---|---|---|
| `key_token` | `70000102030405060708090a0b0c0d0e0f` | `01000102030405060708090a0b0c0d0e0f10` | ok |
| `key_reverse` | `710102030405060708090a0b0c0d0e0f10` | `0100000102030405060708090a0b0c0d0e0f` | ok |
| `key_unit3_token_short` | `70000102030405060708090a0b0c0d0e` | `01000102030405060708090a0b0c0d0e0f10` | UNIT-3 |
| `key_unit3_token_long` | `70000102030405060708090a0b0c0d0e0f00` | `01000102030405060708090a0b0c0d0e0f10` | UNIT-3 |
| `key_unit3_token_empty` | `70` | `01000102030405060708090a0b0c0d0e0f10` | UNIT-3 |
| `key_unit3_reverse_short` | `710102030405060708090a0b0c0d0e0f` | `0100000102030405060708090a0b0c0d0e0f` | UNIT-3 |
| `key_unit3_reverse_long` | `710102030405060708090a0b0c0d0e0f1000` | `0100000102030405060708090a0b0c0d0e0f` | UNIT-3 |
| `value_unit6_env0` | `70000102030405060708090a0b0c0d0e0f` | `00000102030405060708090a0b0c0d0e0f10` | UNIT-6 |
| `value_unit7_env2` | `70000102030405060708090a0b0c0d0e0f` | `02000102030405060708090a0b0c0d0e0f10` | UNIT-7 |
| `value_unit7_env2_other_length` | `70000102030405060708090a0b0c0d0e0f` | `02000102030405060708090a0b0c0d0e0f1000000000` | UNIT-7 |
| `value_unit7_env2_bad_key` | `70000102030405060708090a0b0c0d0e` | `02000102030405060708090a0b0c0d0e0f10` | UNIT-7 |
| `value_unit8_flags` | `70000102030405060708090a0b0c0d0e0f` | `01010102030405060708090a0b0c0d0e0f10` | UNIT-8 |
| `value_unit9_empty` | `70000102030405060708090a0b0c0d0e0f` | `` | UNIT-9 |
| `value_unit9_one_byte` | `70000102030405060708090a0b0c0d0e0f` | `01` | UNIT-9 |
| `value_unit9_envelope_only` | `70000102030405060708090a0b0c0d0e0f` | `0100` | UNIT-9 |
| `value_unit9_short` | `70000102030405060708090a0b0c0d0e0f` | `01000102030405060708090a0b0c0d0e0f` | UNIT-9 |
| `value_unit10_trailing` | `70000102030405060708090a0b0c0d0e0f` | `01000102030405060708090a0b0c0d0e0f1000` | UNIT-10 |

Layer marks (units-v1 §3.1: `envelope_version`, `reserved_flags`, `layer_version: u32`, policy). The "Reader" column is the maximum `layer_version` that the reader knows:

| Name | Bytes (hex) | Reader | Result |
|---|---|---|---|
| `mark_v3` | `01000000000302` | `3` | ok |
| `mark_v3_layer2_reader` | `01000000000302` | `2` | UNIT-16 |
| `mark_v3_layer1_reader` | `01000000000302` | `1` | UNIT-16 |
| `mark_v4_layer3_reader` | `01000000000402` | `3` | UNIT-16 |

## 7. Lifecycle, atomicity and export (normative)

1. **Creation.** The token and its two records are written in the transaction of the unit (`ChangeSet::create_public`); they are confirmed or vanish with it. The mark rises to 3 in that same transaction if it was below. If the random generator fails, nothing is created.
2. **Deletion (D-a).** Deleting a unit (`delete`, also through `DerivedDb`) removes **both** records in the same transaction, so that its token afterwards resolves exactly like one never issued (`None`). A retention that prunes old revisions does not touch the mapping of a live unit. The layer has no other operation that forgets a unit.
3. **Export and import (D-b).** The [logical export](export-v0-draft.md) carries the token of each unit that has one (its `format_version` becomes 1), and `import` restores it in the transaction of the unit's first retained revision: a token is stable after exporting and importing. A file that would give one token to two units is rejected before the first write with `UnitError::PublicIdTaken`.
4. **No token.** A unit created with `ChangeSet::create`, or written before this module, has no token; `public_id` returns `None`. A token is only issued when the unit is created.
5. **Permissions.** A valid token authorises nothing (I-27): it only says which unit it names. The layer that exposes Kerno checks permissions in each operation.

## 8. Source of randomness (D-e)

The 16 bytes come from `/dev/urandom`, read with `std` (no new dependencies). That file exists on Linux, macOS, the BSDs and other Unixes. On any other platform (Windows, WASI) opening it fails and `create_public` returns that error (`UnitError::Storage(Io)`) without creating anything; reading and resolving tokens already issued keeps working. A read error or a short read is an error. A 16-byte token with all bytes equal (a constant source) is also rejected, as an error and without retrying, so that a broken source cannot leave creation in a loop. Only a collision with an already-issued token is retried, with a maximum of 4 draws. Improvement path: `getrandom` if a non-Unix target is ever supported.

## 9. Fixture

`crates/kerno-units/tests/vectors/public-ids-v3/db-with-token/` is a `kerno_db` database closed with three units: unit 1 with a token, unit 2 without one, and unit 3 with a token and deleted (no mapping). Layer mark 3. `MANIFEST` lists the SHA-256, the size and the name of each file, and `TOKEN` the token text of unit 1. The bytes are not reproduced between builds (the engine draws the identity of the database at random and the token comes from the operating system): the integrity is given by `MANIFEST`. It is regenerated with `cargo test --release -p kerno-units --test public_ids -- --ignored regenerate_fixture`. It is checked by `fixture_directory_opens_resolves_and_verifies`, `fixture_directory_is_refused_by_a_layer2_reader` and `fixture_manifest_lists_the_committed_files`. The details are in [stage9-fixtures.md](../architecture/stage9-fixtures.md) §5.

## 10. History

| Date | Change |
|---|---|
| 2026-11-13 | E9-02: first version (value `version ‖ 16 bytes`, mark not raised). |
| 2026-11-13 | Correction after independent review, by express delegation from the owner (the branch is not merged and nobody has data with these records, so the on-disk format is corrected now): value with the common header (B1), layer 3 mark (B2), cross-record checks UNIT-39 to UNIT-44 (B3), deletion and export (D-a, D-b). |
