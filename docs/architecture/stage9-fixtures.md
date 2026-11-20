# E9-01: fixtures from earlier versions

Package E9-01 of Stage 9 (`stage9-record.md`, §3; decision D11, option (a); row F-11 of E8). This document records which fixtures exist, what each one tests, and the result of the T5 check of ADR-0020 on a binary built from the base commit. It does not change the format and does not write the `MIGRATION` record, which remains reserved without a writer (`KIND_MIGRATION_RESERVED = 5`).

## 1. Fixtures of v1

| Fixture | What it contains | Who checks it |
|---|---|---|
| `crates/kerno-storage/tests/vectors/v1/dirs` | 33 database directories (clean, dirty, damaged, of a future version) with their `EXPECTED` | `crates/kerno-storage/tests/dir_vectors.rs` (opens each one and compares with `EXPECTED`) |
| `crates/kerno-storage/tests/vectors/v1/structs` | Structures of the storage format, positive and negative | `crates/kerno-storage/tests/struct_vectors.rs` |
| `crates/kerno-units/tests/vectors/v1/structs` | 33 records of the v1 units layer and 100 negatives `neg_unitNN_*` | `crates/kerno-units/tests/struct_vectors.rs` (against an independent decoder) and `layer_fixtures.rs` (against the production codec) |

`layer_fixtures.rs::v1_struct_vectors_are_read_by_the_production_decoder` reads each v1 units file with `Record::decode`: a positive one decodes, and a `neg_unitNN_*` is rejected with the rule `UNIT-NN` in its name. It also fixes the counts (33 and 100), so a vector lost or added without review breaks the test.

Result of the storage tests in this build (release mode): `dir_vectors` 8 pass, 1 ignored (regeneration); storage `struct_vectors` 6 pass, 1 ignored; units `struct_vectors` 9 pass, 1 ignored.

## 2. ADR-0020 fixture: rebuilt

ADR-0020 (T1, T5 and T6) cites `crates/kerno-units/tests/vectors/derived-v2/` and `derived-v2/db-with-vector`, which did not exist. The ADR is accepted and is not edited. The fixture has been rebuilt:

- `crates/kerno-units/tests/vectors/derived-v2/db-with-vector/`: a `kerno_db` database closed with one unit (`doc`, carrying `hello`) and its vector (method `embed`, version 1, profile `model-a`). It contains `000003.sst` and `MANIFEST`; layer mark 2.
- `crates/kerno-units/tests/vectors/derived-v2/MANIFEST`: SHA-256, size and name of each file.

The record vectors of §B of ADR-0020 (T1) are already committed in another form, in `derived_vectors.rs` (vector table of the document `units-v2.md`); this package does not duplicate them.

**The bytes are not reproduced between builds.** The engine draws the identity of the database when it is created, and two builds differ in the bytes that store it. That is why `derived_v2_directory_has_the_shape_the_build_writes` compares names and sizes, and the integrity of the committed fixture is given by `MANIFEST` (`derived_v2_manifest_lists_the_committed_files`). To regenerate it after a deliberate design change:

```text
cargo test --release -p kerno-units --test layer_fixtures -- --ignored regenerate_derived_v2
```

### Tests of `layer_fixtures.rs`

Each database test runs on `emulated_log` and on `kerno_db`. The directory can only be opened by the engine that has files; `emulated_log` builds the same content in memory and receives the same checks.

| Test | ADR-0020 criterion |
|---|---|
| `layer1_reader_refuses_the_fixture` | T6: `LayerMeta::decode` (maximum 1) gives `UNIT-16`, `found_layer_version = 2`, class `IncompatibleLayer`; this code reads the mark with `decode_upto` |
| `this_build_opens_and_serves_the_fixture` | T7: opens, serves 1 derived item, no degraded items or findings, `verify_units` clean |
| `opening_the_fixture_adds_nothing` | T7: opening the derived layer writes no more than opening the engine alone |
| `derived_v2_manifest_lists_the_committed_files` | fixture integrity |
| `derived_v2_directory_has_the_shape_the_build_writes` | the fixture is what the build writes (names and sizes) |

## 3. T5 check on a binary from the base commit

T5 asks that a build without the feature reject, on opening, a database with derived data. The reference binary is `d01f646` (last commit before E8-04).

Procedure, outside the tree: `git archive d01f646` extracted to a temporary directory; in it, a throwaway integration test opens a copy of the directory with `KernoDb::open` (real file system) and then `UnitDb::from_engine`; compiled in release mode with its own target directory. The copy and the target were deleted afterwards. The throwaway test is not committed: it is outside the surfaces of the package.

Result:

```text
RESULT IncompatibleLayer { found: 2, supported: 1 }
BEFORE {"000003.sst": "1309 bytes crc f49aa96f", "MANIFEST": "254 bytes crc 983b12f4"}
AFTER  {"000003.sst": "1309 bytes crc f49aa96f", "LOCK": "0 bytes crc 00000000", "MANIFEST": "254 bytes crc 59f502fd"}
```

- **Met:** the base binary returns `IncompatibleLayer` with `found = 2` and `supported = 1` (the variant built from `UNIT-16`).
- **Not met as T5 is written:** the "file tree identical before and after" is not met to the letter. The opening of the *engine* (which happens before the layer reads the mark) creates `LOCK` and adds a session record to `MANIFEST`; `000003.sst` stays identical. That is the engine's behaviour on any opening, including that of a layer 1 database, and the layer does not cause it. The test `opening_the_fixture_adds_nothing` fixes the part that does depend on this code: the layer adds no writes beyond those of the engine's opening.
- The check is a single run of this delivery, not a repeatable test of the repository: repeating it requires rebuilding the base commit with the procedure above.

## 4. Status

F-11 of E8 is checked with the caveat above. R-E9-13 does not occur: the fixture could be rebuilt. The erratum in the citation of ADR-0020 (`derived-v2/db-with-vector` now exists, but with the files above) passes to the E9-CL closure.

## 5. E9-02 fixture: directory with tokens

Package E9-02 (corrected on 2026-11-13 after an independent review, by express delegation of the owner). Layer 3 ([units-v3.md](../file-format/units-v3.md)) adds records `0x70`/`0x71` and raises the mark to 3; the precedent of §2 (a committed directory, with `MANIFEST`) is repeated:

- `crates/kerno-units/tests/vectors/public-ids-v3/db-with-token/`: a `kerno_db` database closed with three units: unit 1 with a token, unit 2 without one, and unit 3 with a token and deleted (its mapping no longer exists). It contains `000003.sst` and the engine's `MANIFEST`; layer mark 3.
- `crates/kerno-units/tests/vectors/public-ids-v3/MANIFEST`: SHA-256, size and name of each file.
- `crates/kerno-units/tests/vectors/public-ids-v3/TOKEN`: the token text of unit 1 (it comes from the operating system, so each regeneration gives another one).

As in §2, the bytes are not reproduced between builds; the integrity is given by `MANIFEST`. To regenerate it after a deliberate design change:

```text
cargo test --release -p kerno-units --test public_ids -- --ignored regenerate_fixture
```

| Test (`public_ids.rs`) | What it checks |
|---|---|
| `fixture_manifest_lists_the_committed_files` | integrity of the committed directory |
| `fixture_directory_opens_resolves_and_verifies` | opens; the token in `TOKEN` resolves to unit 1, unit 2 has no token, the mapping of 3 does not exist, mark 3, `verify_units` clean, and a new unit on top receives its own token. In `emulated_log` it builds the same content in memory |
| `fixture_directory_is_refused_by_a_layer2_reader` | a layer 2 reader rejects the mark with UNIT-16 and `found_layer_version = 3` |
| `legacy_directory_written_before_tokens_opens_and_verifies` | the directory of §2 (without tokens, mark 2) still opens and moves to mark 3 with the first token. `kerno_db` only: `emulated_log` does not open file directories |

The table of hexadecimal vectors of the records is in [units-v3.md](../file-format/units-v3.md) §6 and is fixed by `vectors_of_the_document_match_the_codec_byte_for_byte`.

## 6. P1 fixture: directory with relations from before layer 4

P1 package after Stage 9 (defect K1). Layer 4 ([units-v4.md](../file-format/units-v4.md)) adds the record `0x72` and raises the mark to 4 with the first count. A directory written **before** must keep opening and working; the precedent of §5 is repeated:

- `crates/kerno-units/tests/vectors/rel-counts-v4/db-legacy-relations/`: a `kerno_db` database closed with three units and three relations as the earlier writer left them (`REL` and its two adjacency entries, **without** a count record), mark 1. It contains `000003.sst` and the engine's `MANIFEST`.
- `crates/kerno-units/tests/vectors/rel-counts-v4/MANIFEST`: SHA-256, size and name of each file.

**How it was generated, with the caveat.** It was not built with a build from before the change: the three units were written with the layer and, for each relation, the three records of the old writer with `raw_commit` (the damage medium that the crate's tests already use). The bytes are those the earlier writer left for a relation, but the path that wrote them is the test path, not the old production one. As in §2 and §5, the bytes are not reproduced between builds; the integrity is given by `MANIFEST`. To regenerate it after a deliberate design change:

```text
cargo test --release -p kerno-units --all-features --test rel_counts -- --ignored regenerate_fixture
```

| Test (`rel_counts.rs`) | What it checks |
|---|---|
| `fixture_manifest_lists_the_committed_files` | integrity of the committed directory |
| `legacy_directory_opens_works_and_upgrades_when_written` | opens with mark 1, without counts, `verify_units` clean, serves its relations; opening and reading do not write; a new relation raises the mark to 4 and writes the two counts of its ends. In `emulated_log` it builds the same content in memory |
| `legacy_relations_work_and_get_a_count_when_next_changed`, `deleting_a_legacy_relation_counts_what_is_left` | the same path, in both engines, with the limit of units-v4 §7 point 1 fixed |
| `layer3_reader_refuses_a_database_with_counts` | a layer 3 reader rejects the mark with UNIT-16 and `found_layer_version = 4` |
| `vectors_of_the_document_match_the_codec_byte_for_byte` | the table of units-v4 §6 |
