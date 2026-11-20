# Logical export format v1 (frozen)

| Field | Value |
|---|---|
| Status | **Frozen.** Normative for `format_version = 1` |
| Implementation | `crates/kerno-units/src/export.rs` (`export`, `export_snapshot`), `crates/kerno-units/src/import.rs` (`import`) |
| Golden files | `crates/kerno-units/tests/golden/export-v1/` and `crates/kerno-units/tests/export_golden.rs` |
| Related | [ADR-0003](../adr/0003-physical-format-and-versioning.md) §6, [ADR-0005](../adr/0005-public-identifiers.md), [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md), [ADR-0021](../adr/0021-typed-schemas-and-secondary-indexes.md), [compatibility promise](../compatibility.md) |

This document was `export-v0-draft.md`, a draft whose bytes were no promise.
It is frozen as the file that the draft already was at `format_version = 1`,
with one change to the header: the draft flag is retired (section 2). "Must"
and "must not" are normative.

The export is the one format that does not depend on the physical format
(invariant I-30): the same logical state gives the same bytes on any engine,
and any later version of Kerno reads it. It is the path for moving data
across major versions and engines (see [compatibility](../compatibility.md)).

## 1. What it holds and what it does not

**It holds**, as the public reads of the layer answer them at one snapshot:
units, their retained revisions, relations, history policies, and the
**public id token** of each unit that has one
([ADR-0005](../adr/0005-public-identifiers.md); a token is the same after
export and import). A deleted unit has no token: deleting removes the mapping.

**It does not hold:**

- **Derived data** (vectors, summaries, descriptors, indexes): rebuilt after an
  import (ADR-0018).
- **Typed schemas and secondary indexes** ([ADR-0021](../adr/0021-typed-schemas-and-secondary-indexes.md) §4):
  an import needs a new database, and a schema defined after it validates and
  indexes the imported units.
- **Change sets as such** (`CHANGESET`, `CS_MEMBER`): each revision carries its
  `commit_seq`, actor and source; the reason and the correlation of the change
  set are not kept, and a relation-only change set is rebuilt as an `Index` one.
- **Anything physical:** no snapshot sequence, file, level, offset, compaction
  or reclaim state. The same logical state gives the same bytes (I-30).
- **Revisions the history policy does not keep content for:** they are left out,
  as for any reader (`get_revision` answers `HistoryUnavailable`). See the
  limit in section 5.

Anything in the list above that a later version wants to carry needs a new
`format_version` (section 6).

## 2. Header

The magic is `KUNITEXD` (8 bytes). The last letter dates from the draft
("Draft") and is kept so that no file changes meaning; read it as a name, not
as a status. **`KRNO_EXP` stays reserved and unused**: this format neither
writes it nor reads it.

| Field | Type | Value |
|---|---|---|
| `magic` | 8 bytes | `KUNITEXD` |
| `format_version` | u32 | `1` |
| `flags` | u8 | `0`. No flag is defined in version 1 |
| `reserved` | 3 bytes | zeros |

A reader of version 1 answers:

| Input | Result |
|---|---|
| `flags = 0`, reserved zeros | read |
| `flags = 1` (bit 0, the retired "draft, not frozen" flag of a pre-freeze file) | `Incompatible("pre-freeze draft export")`. Re-export from the source database |
| any other `flags` value, or a non-zero reserved byte | `Corruption` |
| `format_version` other than 1 (0 had no tokens; 2 and above are newer) | `Incompatible`, before anything is written |
| a wrong magic, a short file, a digest that does not match | `Corruption` |

The retired flag follows the engine's own precedent for its `DRAFT` bit
([v1.md](v1.md) §4.3): a file written before the freeze is refused by name,
not guessed at.

## 3. Encoding

Integers are big-endian. `bytes` is a `u32` length and the content. `opt_u64`
is a `u8` (0 absent, 1 present) and, if present, a `u64`. `opt_bytes` is the
same with `bytes`. Any `u8` flag of an option other than 0 or 1 is
`Corruption`. The sections come in this order, with no padding:

1. **Policies.** `default_policy` u8 (`0xFF` when the database has no layer
   mark yet; otherwise the `HistoryPolicy` byte), `n` u32 and `n` records
   `{unit_type: bytes, policy: u8}` in `unit_type` order.
2. **Tokens.** `n` u32 and `n` records in strict unit id order: `unit` (16
   bytes) and `token` (16 bytes). Each `unit` is a unit of the file that is not
   deleted (its last revision is not `Delete`). A token must not repeat and
   must not have its 16 bytes all equal. A unit without a token does not appear.
3. **Units.** `n` u32 and `n` records in id order: `id` (16 bytes), `unit_type`
   bytes, `created_seq` u64, `created_at` i64 (microseconds), `oldest` u64,
   `head` u64, `r` u32 and `r` revisions in revision order: `rev` u64, `kind`
   u8, `parent` opt_u64, `commit_seq` u64, `created_at` i64, `schema_version`
   u32, `content_hash` (`alg` u8 and 32 bytes), `payload_len` u32,
   `restored_from` opt_u64, `metadata` bytes, `actor` opt_bytes, `source`
   opt_bytes, `payload` opt_bytes (absent on a tombstone).
4. **Relations.** `n` u32 and `n` records in id order: `id`, `from`, `to` (16
   bytes each), `relation_type` bytes, `created_seq` u64, `provenance` u8,
   `origin` bytes (empty when `Declared`), `confidence` u16 in thousandths (0
   unless `Inferred`).
5. **Digest.** BLAKE3, 32 bytes, of every byte before it. Nothing may follow.

The revision `kind` bytes are `Create` 1, `Update` 2, `Archive` 3, `Unarchive`
4, `Delete` 5, `Restore` 6. The `provenance` bytes are `Declared` 0,
`Imported` 1, `Algorithmic` 2, `Inferred` 3.

**Checks before the first write.** A reader decodes and checks the whole file
before it writes anything, so a refused file leaves the target as it was.
Besides the encoding above, it checks: strict ordering in every section; the
revisions of a unit cover `[oldest, head]` with no gap and their `commit_seq`
grow; each payload matches its `content_hash` and `payload_len`; every
relation end is a unit of the file; the default policy byte, and "no layer
mark" only for a file with no content. A repeated token (one token for two
units) is `UnitError::PublicIdTaken`, whose text names neither the token nor a
unit; every other violation is `Corruption`.

**Import.** `import` needs a new database with no commit and the exported
default policy. It restores each token in the change set of the first retained
revision of its unit, replays the commit sequences the file names (an
unnamed sequence is an empty `Index` change set), and checks the result with
`verify`. The export of the result is, byte for byte, the file imported.

## 4. Determinism

The bytes are a function of the logical state only. Two databases with the same
state, on any engine and after any flush, compaction, reclaim or reopen,
export the same bytes (tests `export_physical.rs`, `export_import.rs`
`same_bytes_*`, `export_golden.rs`).

## 5. Limits

Two limits are declared, not hidden. Neither changes a byte of the format.

- **A history that a policy trimmed does not import.** Under `Audit` (and any
  policy that keeps the content of some revisions only) the exporter leaves out
  the revisions with no content, so a unit's records do not cover
  `[oldest, head]` and the importer refuses the file as `Corruption` ("cannot be
  imported"). The file is a faithful export and a valid backup of what the
  public reads give, but it is not an import source. A unit with one revision,
  or under `Versioned`, or under `None` (which leaves a suffix) imports.
  Making such a file importable is a change of what `r` may mean, and so of the
  format version.
- **A reader may refuse a very high commit sequence.** The format allows any
  `u64` in `created_seq`, `commit_seq` and `oldest`/`head`. The reference reader
  replays every sequence from 1 as one change set, so it refuses a file whose
  highest sequence is above `MAX_REPLAY_SEQ` (1,000,000) with `InvalidArgument`
  and writes nothing. This is a limit of that reader, not of the format: a
  database with more commits exports, and another reader may import it.

## 6. Versioning

A reader refuses another `format_version` as `Incompatible`. Version 1 is
frozen: no byte it writes or accepts changes. A change of any kind, adding or
removing a field, changing an order, giving a flag a meaning, carrying more
state, needs a new `format_version`, new golden files in
`crates/kerno-units/tests/golden/` and an ADR. A later Kerno must still read
every file of version 1 (the [compatibility promise](../compatibility.md)).

## History

| Date | Change |
|---|---|
| 2026-11-13 | Draft, `format_version = 0` (E9-04) |
| 2026-11-13 | Token section added, `format_version = 1` (E9-02), still a draft |
| 2026-11-18 | **Frozen at `format_version = 1`** (ROADMAP phase D). The draft flag is retired: writers emit `flags = 0`; `flags = 1` is refused as `Incompatible`. Golden files added. The magic and every other byte are those of the draft |
