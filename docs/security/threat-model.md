# Initial threat model (embedded library)

Status: reviewed for Phase 1 (R-07, 2026-10-29; §6) · Related to: [SAD §11](../architecture/SAD.md#11-security-privacy-and-life-cycle), ADR-0005, ADR-0006, ADR-0009, invariants I-08, I-27…I-29

This document delimits **what Kerno protects and what it does not protect** in its initial form (a library in the same process + local CLI). It must be reviewed before the physical format is frozen (ADR-0003; the review of the v1 format is in §6) and again before any network server.

## 1. Scope and assumptions

- Kerno runs **inside** the process of the host application, with its same privileges. All code in that process can read Kerno's memory, including decrypted data and loaded keys.
- The application is responsible for authenticating its users and for authorising each business operation (SAD §11.1). Kerno offers mechanisms (limits, validation, opaque identifiers, audit), it does not decide who can do what.
- The operating system and the file system are trusted for isolation between OS users; they are not trusted for the integrity of data at rest (it can be corrupted, truncated or lose unsynced writes).

## 2. Assets

| Asset | Why it matters |
|---|---|
| Committed data (current state) | Silent loss or alteration is the most serious failure of the engine. |
| Revision history and ChangeSets | Contains old versions, possibly personal data already "deleted" from the current state. |
| Journal/WAL, freed pages, obsolete segments, temporary files | Contain copies of data the user believes deleted. |
| Backups and exports | Live outside the engine's control and with another life cycle. |
| Encryption and token keys | Their loss implies data loss; their theft, exposure of everything encrypted. |
| Internal identifiers (UUIDv7) | Reveal creation instant and make enumeration easier. |
| Logs, metrics and audit | Can leak content if not minimised. |

## 3. Adversaries and failures considered

| ID | Source | Examples | Planned response |
|---|---|---|---|
| T-01 | **Non-malicious media failures** | bit rot, damaged sectors, broken writes, truncations, `fsync` that fails | Checksums on everything persisted, atomic file creation, detection and explicit error (I-08, I-09); poisoning after a failed `fsync` (I-10). **Verified in M0** with `SimVfs`. |
| T-02 | **Process or system crash** | kill -9, OOM, power cut during commit, compaction or migration | Commit and recovery protocol with failure matrix (ADR-0002); re-entrant recovery (I-11). **Verified in M0.** |
| T-03 | **Read access to disk or backups** | stolen laptop, cloud backup, discarded disk | Encryption at rest with AEAD and keys outside the data directory (ADR-0006, I-28). *Not in 1.0 (D3, taken on 2026-11-13): 1.0 does not promise confidentiality at rest nor key management. If built, the chosen design is per-block AEAD (ADR-0006, option 1; D2); nothing is implemented.* |
| T-04 | **Deliberate tampering with files** | an attacker edits pages and recomputes CRC32C | CRC32C does **not** protect against this (R-S5), nor does `db_id` or the CRC context (§6.1). Requires per-page/block AEAD (ADR-0006); format v1 reserves the slot (§6.3). *Pending.* |
| T-05 | **Hostile input via API** | huge keys/values, pathological ranges, malicious documents | Explicit limits and prior validation (I-13), query limits and pagination (SAD §9.3), fuzzing (Phase 6). |
| T-06 | **Hostile base files** | opening a crafted database to cause panics, loops or out-of-range reads | Validation of all lengths, pointers and strings with bounds; in `kerno-storage`, the suite of crafted files with valid CRC (§6.2); property tests that mutate WAL, SST and MANIFEST and open the result read-write and read-only, with 2 000 cases per file type and mode (E9A-03, `tests/open_properties.rs`); coverage-guided fuzzing of the 14 decoders and of opening, 30 minutes per target with no failures (E9A-04, §6.2). *Mitigated, not closed.* |
| T-07 | **End users of the application** | guessing or enumerating IDs, tampering with tokens | Opaque, validated tokens (ADR-0005); a valid token does not authorise (I-27); authorisation belongs to the application. |
| T-08 | **Leaks through observability** | content in logs, errors or audit | Minimised audit with no payloads (I-29); error messages without document content. Measured in `kerno-storage` (E9A-02, `tests/redaction.rs`): in five engine configurations, neither the errors (the seven variants of `Error`), nor the `Issue` and reports of `verify()`, nor the output of `inspect()` nor the metrics contain a key or value sentinel in any of their text forms. `kerno-storage` has no logging layer: that part is left to the layer that emits text. For observations and plan explanations, see T-11. **Stage 9 (E9-03):** the same sentinel search, with fragments of 8 bytes or more, covers errors, `verify_units` reports and `kerno-units` observations (`tests/redaction_units.rs`); no mutation evidence for the paths of the `verify` report and of the observations. |
| T-09 | **Involuntary retention** | revisions, freed pages, segments or backups that keep suppressed data | ADR-0009 accepted on 2026-11-12 (option 3): immediate logical deletion and removal of live files on compaction, if no open snapshot retains them; no guaranteed physical deletion; deferred cryptographic erasure to ADR-0006; no declared backup deadline (§6.5). **Measured** (E9A-01, `tests/suppression_survival.rs`, observation of current behaviour and not a contract): after `delete` or `delete_range` without `FLUSH`, the value and the key remain in the WAL; after `FLUSH`, `compact_range` and clean shutdown, no live file in the directory contains the value or the key, with the default configuration and with `wal_preallocate`; with a snapshot open before the deletion, `compact_range` leaves three live tables (`pinned_files = 3`), only one of them contains the value and the key, and all three disappear when the snapshot is dropped; with `wal_preallocate`, the flushed WAL is left with zeros. **Not measured:** freed blocks of the medium (C-5), backups, encryption, point tombstones that a trivial move carries to the last level, and background-thread deletion. T-09 remains **open**. |
| T-10 | **Supply chain** | compromised dependency | Few dependencies in the core, versioned `Cargo.lock`, dependency audit (`cargo audit`/`cargo deny`) in Phase 6. **Status at the close of Stage 9:** no manifest has changed since `52f026f` (I-31); `ci.yml` does not run fuzzing or dependency audit. *Open.* |
| T-11 | **Leaks through observations and plan explanations** (Stage 5) | literals of a filter or of a relation query, types, payloads, metadata or provenance in an execution record, a counter, a plan explanation or an error; dictionary attack on the fingerprint; enabling `diagnostic` by accident; reading observations after a suppression | The query is reduced to its form with typed markers and discarded; the record has no field for content and the error is stored as a class, not as a message. The fingerprint is a BLAKE3 with a local key that is not exported. Default level `off`; `diagnostic` only with an explicit `set_level`. Observations kept in memory only, with bounded retention, excluded from export. Verified by the E5-04 sentinel test (ADR-0015, I-37). **Residual:** whoever runs code in the process reads the memory and the key (out of scope, §4); the key is not erased when freed; counters and durations reveal volume and timing (side channel, §4). |

## 4. Out of scope (explicitly)

- Attackers with code execution in the host process or with administrator privileges on the machine while the database is open.
- Side channels (timing, CPU cache) in the engine's operations.
- Guaranteeing physical erasure on media with wear-levelling or on copies that Kerno does not control (external backups, OS or cloud snapshots); windows are documented, not promised.
- Automatic regulatory compliance (SAD §11.5): the profiles map controls, they do not certify.

## 5. Consequences for M0 and later phases

- The storage choice (ADR-0001) affects T-03, T-04 and T-09: where old copies of the data remain and which unit is encrypted and authenticated. This is recorded in ADR-0001 and ADR-0009.
- The format (ADR-0003) must reserve from its first version space and a version for authenticated encryption, even though Phase 1 does not activate it.
- No core component requires network or an external provider (I-31).

## 6. Phase 1 review (R-07)

Scope: format v1 ([`v1.md`](../file-format/v1.md)) and the `kerno-storage` engine as they stand at the close of Phase 1. It changes no decision: it fixes what each mechanism defends and what it does not, so that the owner can freeze the format knowing what it promises (ADR-0013, ADR-0003).

### 6.1 `db_id` and CRC with context: they detect errors, not attacks

- Every header carries a random 16-byte `db_id` and the file number (HDR-10), and every CRC is computed with a context that the reader supplies and that is not stored (v1.md §1.2: file number, block offset or MANIFEST generation). Thus an SST copied from another database, a renamed file, a block moved to another offset or a MANIFEST record from another generation **do not verify** even though each one is intact on its own.
- What this defends is the **non-malicious error**: mixed restorations, manual copies, tools that rename. T-01 is covered.
- What it does **not** defend: an attacker with write access to the directory can build an entire coherent database (same `db_id`, numbers and recomputed CRCs). The `db_id` is not a secret nor a key, and CRC-32C is linear. T-04 has no answer until AEAD is activated.

### 6.2 Position on crafted files

- Every value read from disk (lengths, counters, pointers, varints, sequences, file numbers) is validated with checked arithmetic before use; file numbers are bounded to 2^48−1 (DIR-1) and the sequence, when exhausted, returns `InvalidArgument`, it does not wrap to 0 (I-04).
- `crates/kerno-storage/tests/crafted.rs` fabricates files with **valid CRC** (huge lengths, overlapping indexes, disorder between blocks, false `smallest`/`largest`, sequences above `last_seq`, overlapping levels, overlong varints…) and requires that `verify()` reports the broken rule and that the engine returns `Corruption` without panic or unbounded allocations. Recovery memory is bounded (S-2 writes an L0 SST every time the replayed memtable reaches `write_buffer_bytes`) and WAL replay is streamed.
- A crafted file can, at most, occupy CPU and disk bounded by its size. There is no formal guarantee. **Coverage-guided fuzzing (E9A-04, 2026-11-12):** 15 targets in `crates/kerno-storage/fuzz` (one for each of the 14 decoders that `struct_vectors.rs` exercises, and one for opening a database), 30 minutes per target on a local machine with 4 CPUs: zero failures and zero crashes. The committed corpus is not run in CI. It covers only the readers of `kerno-storage` and only the inputs that coverage reached. **Properties (E9A-03):** the tests of `tests/open_properties.rs` pass with 2 000 cases per file type and mode. T-06 is **mitigated, not closed**.

### 6.3 AEAD reservation

- Each SST block and each MANIFEST record carry a 40-byte slot (`nonce[24] ‖ tag[16]`) just before the CRC; the one in WAL frames exists only if that WAL's header has the `AEAD` bit (v1.md §5; decision pending signature in ADR-0013).
- Phase 1 **does not encrypt or authenticate**: with the `AEAD` bit off, the slot must be zero (AEAD-1, covered by the CRC) and a file with the bit on is rejected with `Incompatible("encryption not supported")` before any write. The reservation avoids a format migration when encryption is activated; it protects nothing by itself.
- The associated data (file number, offset, block type) and key management are decided in ADR-0006 (Phase 4). Until then T-03 and T-04 remain **open**.

### 6.4 Read-only guarantees

- `AccessMode::ReadOnly`, `verify()` and `inspect()` do not mutate the directory except for creating `LOCK` (I-18): no threads, no `create_dir_all`, no `sync_dir`, no deletions, no checkpoints, and `close` writes nothing. Every rejected opening (`Locked`, `Incompatible`, foreign directory) leaves the directory identical byte by byte. Checked by `read_only_open_writes_nothing_*`, `verify_is_read_only` and `*_rejected_untouched`.
- An unknown `ro_compat` bit opens the whole database read-only (HDR-7), and an unknown `incompat` bit rejects it (HDR-6): an old version never writes over structures it does not understand.
- The three paths take the exclusive `LOCK` (`flock`), also between processes (`process_lock.rs`): an external reader cannot observe a database while a writer modifies it, and vice versa.
- Error messages cite file, offset and rule, never key or value bytes (T-08).

### 6.5 No claim of physical deletion

- Kerno deletes obsolete files (WAL, compacted SSTs) with `remove_file`, deferred and with retries. **That does not erase the data from the medium**: the file system may keep the blocks, a wear-levelled SSD or a system snapshot may keep copies, and cache blocks or process memory may hold values already suppressed.
- A live snapshot retains obsolete files while it exists (`snapshot_retained_bytes` exposes it, `compact_range` reports `pinned_files`). `delete_range` and `compact_range` are **mechanism**: they make tombstones and covered points disappear from the live files. [ADR-0009](../adr/0009-suppression-retention-and-backups.md), accepted on 2026-11-12 with option 3, fixes what the engine promises: immediate logical deletion and removal of live files on compaction, when no open snapshot older than the deletion retains them. It does not promise physical deletion, nor a declared backup deadline, and cryptographic erasure is deferred to ADR-0006. `crates/kerno-storage/DESIGN.md` lists what can survive. T-09 remains **open**.
- No product documentation or message must describe `delete` or `compact_range` as secure deletion.

### 6.6 The bloom filter of an SST: finding F-E9A-03-1

**Finding (E9A-03).** A filter block whose bits were rewritten and resealed with a valid CRC passed every opening check, and `get` answered "does not exist" for keys the table contains, without error, while a `scan` returned them. It is T-04 with a concrete effect: the filter is a derivative of the table's keys that the engine consults **before** reading the data block, and a CRC does not distinguish an honest filter from one rewritten by whoever can recompute it. Reproduced with a database of 357 visible keys: with each table's filter zeroed, `get` hid 307, read-write and read-only.

**What `verify()` detected before.** With `deep` (the mode that reads the data blocks) it already reported SST-V3, "every point key is in the filter", without key bytes (I-29): one `Issue` per affected table. Without `deep` it reported nothing, because it only ran the opening checks, and opening did not look at the filter beyond its shape (SST-O7).

**What is done now.**

- *Engine opening (`Table::open`) and `verify()` without `deep`.* The table index, which opening already has in memory, gives the last key of each data block, and that key is a point key of the table (SST-R2 requires it when reading the block). If the filter does not contain it, the table is rejected with SST-V3 (`Corruption`; in `verify()`, an `Issue`). It costs one hash per data block and no block read, so `get` pays nothing for distrusting the filter. A filter zeroed or overwritten block by block no longer opens. The check lives in the engine and in `verify()`, not in the pure codec `open_sst`: the latter keeps the frozen rule order of v1.md §13.1 and does not change any valid file nor the format.
- *`verify()` with `deep`.* No change of rule: SST-V3 still probes the filter with each key it reads. It is the check that reaches a filter that only lacks interior keys of a block.
- No new check compares `nbits` or `k` with the number of entries: the format fixes the limits the reader accepts (`k` 1..=30, `nbits` ≥ 64, multiple of 8) and the writer's values (`k` = 7, 10 bits per key) only describe this writer, they are not a reading rule. Requiring them would reject files that the format declares valid.

**Residual (not closed without authentication).** The opening check is partial on purpose: a filter that only lacks interior keys of a block is opened and `get` answers "does not exist" for them without error. Only `verify()` with `deep` reports it (SST-V3). Closing it on the read path would require reading the data block to doubt every "no", which removes the purpose of the filter; and closing it entirely against a deliberate rewrite requires the engine to distinguish a block it wrote from a rewritten one, that is, per-block AEAD (ADR-0006, pending; the slot is reserved, §6.3). A filter with extra bits (saturated) is not detected either: the format allows it and it only costs performance, because every `get` then reads a data block. Therefore the engine does **not** claim to protect against a deliberate rewrite: the opening check and SST-V3 detect corruption and carelessness, not an attacker who knows which keys to hide. Whoever needs that guarantee before ADR-0006 must run `verify()` with `deep` on the directory before trusting it. T-04 remains **open**.

**Limit at the close of Stage 9 (H-2).** Authenticated integrity against a rewrite with valid CRC is decision D4: retained on 2026-11-12 and **taken on 2026-11-13 by express delegation of the owner, revocable**. No authenticated integrity in 1.0; it was not implemented. H-2 is closed as an accepted and documented limit: the limit must not be read as a protection of the engine ([closure](../architecture/stage9-closure.md) §6).

Tests: `tests/open_properties.rs` (`resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify`, `known_limit_get_trusts_a_resealed_filter_missing_interior_keys`, and the filter again among the rewrite targets), `tests/verify.rs::verify_detects_bloom_missing_key`, `tests/redaction.rs`.

### 6.7 Status summary

| Threat | Status at the close of Phase 1 |
|---|---|
| T-01, T-02 | Covered and tested (failure matrix, real `kill -9`, `SimVfs` sweeps); the record with a device that loses pages (LazyFS) is pending ([F1 report](../testing/f1-report.md)) |
| T-03, T-04 | Open at the close of Stage 9 (D2, D3 and D4 taken on 2026-11-13, revocable): no encryption or authentication in 1.0; slot reserved. A rewritten SST filter with valid CRC is rejected at opening if it does not contain the last key of a block, and `verify()` with `deep` reports the rest (§6.6); `get` over a filter that only hides interior keys cannot be detected without AEAD |
| T-05 | Key, value and batch limits checked in `kerno-storage` (`Config::limits`) |
| T-06 | Mitigated with validation, crafted file suite, opening properties (E9A-03) and fuzzing of 15 targets with no failures (E9A-04). Finding F-E9A-03-1 is corrected (§6.6). *Mitigated, not closed* |
| T-08 | Measured in `kerno-storage`: no error, `Issue`, `verify()` report, `inspect()` output or metric contains a sentinel (E9A-02). No logging layer in `kerno-storage` |
| T-09 | Open: no guarantee of physical erasure. Stage 9 adds the public reclaim (`reclaim`, `compact`), which is mechanism and not secure deletion (D14, I-44). ADR-0009 accepted on 2026-11-12 with its limits (§6.5). Not measured: C-5, memory, caches, tombstones, deletion thread and handle (H-5). What survives in the scenario measured by E9A-01 is in the T-09 row (§3) |
