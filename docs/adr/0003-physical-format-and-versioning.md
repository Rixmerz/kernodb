# ADR-0003 — Physical format and versioning

| Field | Value |
|---|---|
| Status | **Accepted** (2026-10-28, by the project owner): the versioning and integrity rules are mandatory; the concrete format is frozen when Phase 1 closes |
| Date | 2026-10-27 (proposal) · 2026-10-28 (acceptance) |
| Related to | SAD §13, D-10; ADR-0001, ADR-0002, ADR-0006; [format principles](../file-format/README.md); invariants I-08, I-09, I-15, I-30 |

## Context

Once real databases exist, changing the format will be a migration, not a refactoring. SAD §13 asks for three separate version axes (physical format, logical schema, derived data), safe rejection of incompatible versions, detection of truncation and corruption, and an independent logical export.

M0 does not freeze any format: the two prototypes have their own formats, versioned only by a *magic* and a version number. What M0 does provide is evidence of **which format properties are needed** to pass the integrity and failure tests, and that is what this ADR fixes.

## Decision

### 1. Identification and version of each file

Every persistent file begins (or, if it is append-only, ends) with a fixed header:

| Field | Content |
|---|---|
| `magic` | 8 bytes per file type (`KRNO…`), distinct for data, journal, manifest, export |
| `format_version` | `u32` of the physical format of that file type |
| `compat` / `ro_compat` / `incompat` | Three sets of feature bits, in the style of ext4: a reader that does not know an `incompat` bit **rejects** opening; one that does not know a `ro_compat` bit opens **read-only**; `compat` bits are ignored safely |
| `page_size` / `block_size` | if applicable |
| `crc32c` of the header | |

Thus a new feature (e.g. encryption, compression, a new index) is added as a feature bit without forcing a full migration, and an old version of Kerno never writes over a format it does not understand.

### 2. Integrity

- **Every byte that influences a result is covered by a checksum that is verified before use** (I-08). M0's tests showed that any exception — a bloom filter without a crc, a free list without a crc — produces silent wrong results.
- The M0 model is kept according to the chosen engine: checksums in the parent (Merkle tree) for a CoW B+ tree; checksum per record/block/file for an LSM.
- **Own header checksum in each page/block** in addition to the parent's, so that individual pages can be verified and debugged (`kerno verify`) without walking the tree.
- CRC32C to detect accidental damage. Integrity against tampering is AEAD (ADR-0006): the format reserves, from version 1, space per page/block for a nonce and tag, and an `incompat` bit indicates whether they are in use.
- Every length, offset, counter or pointer read from disk is validated against the file size and the format's limits **before** indexing, reserving memory or iterating; chains (overflow, free lists) are traversed with a limit derived from verified data.

### 3. Atomicity of the files that define the state

- Atomic creation and replacement (temporary → `fsync` → `rename` → `fsync` of the directory).
- An existing file that is empty, short or without a valid state is `Corruption`, never a "new database" (I-09). In M0 both prototypes had to adopt this to pass the truncation suite.
- After a **clean shutdown** there must be no ambiguity between "broken tail" and "corruption": either the clean shutdown is marked, or the state is left in structures without a tail (full checkpoint), or there is redundancy (two copies of the meta with the same commit).

### 4. Independent version axes (SAD §13)

| Axis | Where it lives | Who changes it |
|---|---|---|
| Physical format | header of each file + feature bits | engine releases |
| Logical schema | collection metadata (reserved keys) | the application (schema migrations) |
| Derived data | definition of each index/embedding: algorithm, version, parameters, watermark | Kerno when rebuilding (ADR-0007) |

Changing the version of a derived index does **not** require migrating the physical format: the index is marked `stale` and rebuilt from the source.

### 5. Physical format migrations

- Preconditions checked (supported source version, free space, recent backup or explicit confirmation).
- The migration writes new structures without modifying the source ones; the version change is one more atomic commit (meta or manifest), so the failure matrix of ADR-0002 covers it.
- The migration state is persisted: after a crash, opening resumes or reverts.
- Later validation (full `verify`) before deleting the source structures.
- Each release documents which versions it opens directly, which it migrates and which it rejects. No indefinite backward compatibility or automatic *downgrade* is promised.
- Migration tests run with a simulated interruption at each I/O point (same technique as `crash::every_point`).

### 6. Logical export

Format independent of internal storage, versioned and documented: a header (version, origin, commit sequence of the exported snapshot), followed by records of collections, document, revision, `ChangeSet` and public identifier mappings, with a checksum per record and a final record with counts. Proposed encoding: JSON Lines for readability and deterministic CBOR (RFC 8949 §4.2) for binary fidelity; the final choice depends on ADR-0008. The export is always taken from a snapshot (consistent without blocking writers).

### 7. Test vectors

Each format version publishes example files and their expected contents. Compatibility tests open all vectors of supported versions and check the safe rejection of unsupported ones and of unknown `incompat` bits.

## M0 evidence

- Bit flips (400 seeds, half aimed at headers, footers and metas) and truncations (including to 0 bytes) after a clean shutdown: both prototypes return correct data or an error, never wrong data nor an earlier commit ([M0 report](../testing/m0-report.md)).
- Real faults that the tests forced to be corrected and that motivate the rules above: corruption in the middle of the log treated as a broken tail after a clean shutdown (rule 3, reference engine); directory sync only when the file is created (rule 3 and ADR-0002; reference engine and B+ tree); choice of the meta slot by sequence parity, which after losing one slot would have overwritten the only valid meta (rule 3; B+ tree).
- Design risk flagged before implementing and covered by rule 2: a damaged bloom filter without a checksum would make present keys appear absent, with no error at all.

## Consequences

- The concrete format (page or block layout) is written in `docs/file-format/` during Phase 1 from the prototype chosen in ADR-0001, with these rules as requirements.
- No user database is created before that document and its test vectors exist (SAD §20.6).
- Reserving space for AEAD from version 1 costs a few bytes per page/block, but avoids a full migration when encryption is enabled.

## Follow-up

- Phase 1: specification of format v1, test vectors, `kerno verify`.
- Phase 4: logical export and import, round-trip tests.
- Phase 6: fuzzing of format readers; compatibility tests between releases.
- **Phase 1 follow-up (WP14, 2026-10-29; no change of decision).** Format v1 is specified in [`docs/file-format/v1.md`](../file-format/v1.md), with structure and directory vectors, an independent decoder and a byte-by-byte regeneration guard; the interpretations that the owner must review are in [ADR-0013](0013-format-v1-and-engine-protocol.md). Integrity verification exists as a library (`verify()`, `inspect()`); the CLI `kerno verify` moves to Phase 2. **Freeze status: pending.** It requires the owner to accept ADR-0013 and the [threat model](../security/threat-model.md) to be reviewed (review R-07 is done); in the meantime every file written carries the `DRAFT` bit (v1.md §4.3) and no user database should be created with this format.
