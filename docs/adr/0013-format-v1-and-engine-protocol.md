# ADR-0013 — Format v1 and engine protocol

| Field | Value |
|---|---|
| Status | Accepted (2026-10-31), conditional on the adversarial review of Phase 1 ([f1-adversarial-review](../testing/f1-adversarial-review.md)), whose blockers B1, G1 and M1 were corrected |
| Date | 2026-10-28 |
| Deciders | Project owner |
| Related to | ADR-0001, ADR-0002, ADR-0003 (accepted, which this ADR interprets without replacing), ADR-0006, ADR-0012; [format v1](../file-format/v1.md); [Phase 1 design](../architecture/phase1-design.md) §4, §5, §7, §12 and §13; invariants I-03, I-08, I-09, I-11, I-15, I-16, I-18 |

## Context

ADR-0001, ADR-0002 and ADR-0003 fix **what** the core must meet (levelled LSM, commit and durability semantics, versioning and integrity), but leave open the details of **how** they are made concrete in bytes and in the engine protocol. The Phase 1 design (C-01…C-33) and the draft [`v1.md`](../file-format/v1.md) take those decisions; some interpret an accepted rule in a way that the owner must be able to review before the format is frozen, because changing them afterwards requires raising `format_version` or a feature bit (v1.md §15).

This ADR gathers those interpretations, with the alternatives that were discarded and the reason. It does not change any accepted ADR: where an accepted ADR needs a follow-up note, it is added there (C-21).

Since WP06 the format exists as pure code (`crates/kerno-storage/src/format/`), with 194 structure vectors (`crates/kerno-storage/tests/vectors/v1/structs/`), an independent decoder written from the specification (`tests/spec_decoder/`) and hexadecimal blocks in v1.md §13.5 that CI compares with the vectors. The engine does not use it yet (WP07).

## Options considered

Each point indicates the chosen option (**in bold**) and the discarded ones.

1. **Common header with `db_id`** (ADR-0003 §1).
   - **96-byte header at offset 0 of the three file types, with a random 16-byte `db_id` and the file number, verified against the MANIFEST and the name (HDR-10).** It detects files from another database or renamed ones with a valid CRC, which no checksum detects.
   - Header at the end in append-only files (ADR-0003 §1 allows it): a log cannot end in a header while it is being written, and the header at the beginning allows rejecting by version before reading anything else.
   - No `db_id` (only *magic* and version): an SST copied from another database with the same number would be accepted.
2. **No `CURRENT` file.**
   - **`MANIFEST` with a fixed name, replaced by atomic rename at each *checkpoint* (W-10).** A single file defines the state; creation (DIR-3) and replacement are the same protocol temporary → `fsync` → `rename` → `fsync` of the directory from ADR-0003 §3.
   - `CURRENT` pointing to `MANIFEST-NNNNNN` (LevelDB/RocksDB style): one more file and one more sync per *checkpoint*, and one more intermediate state to recover.
3. **`CLEAN` inside the *checkpoint* and `SESSION_OPEN`** (ADR-0003 §3, ADR-0001 condition 3).
   - **Clean shutdown writes a *checkpoint* whose only record is a `SNAPSHOT` with `CLEAN`; reopening appends `SESSION_OPEN`, which cancels it (MAN-R4).** After a clean shutdown the directory contains only `LOCK`, `MANIFEST` (one record) and SST, so no later damage is ambiguous; a clean reopening costs 2 syncs (M0: 3).
   - A `CLEAN` record appended to the log: it leaves a long log after shutdown and forces deciding whether a broken tail after `CLEAN` is damage.
   - A separate marker file: another file whose state must be made atomic with the MANIFEST.
4. **`synced_len` instead of the `SYNCED` bit** (ADR-0002, C-04).
   - **Each WAL frame carries `synced_len`, the bytes of the file that the writer knew to be durable when writing it (W-11); WAL-R5 distinguishes a broken tail from damage by looking for a later frame with `synced_len` beyond the broken point.** It works the same with a writer in `Sync`, with *group commit* (one frame = one group, one CRC) and in `Buffered`, and admits per-commit durability without changing the format. For it to work with groups, the bound on `first_seq` of a candidate frame at `q` counts one commit per byte from the broken point (`q − P`), not one commit per frame as in M0: a frame may carry up to 65 536 single-byte commits (WAL-C3), and with M0's bound damage in a durable group was taken for a broken tail (corrected in the WP06 review; fixed by the vectors `neg_walr5_group_*` and `neg_walr6_group_broken_header`).
   - The M0 bit 31 `SYNCED` per record: it does not express groups or the synchronized prefix in `Buffered`.
   - Accepting any broken tail of the most recent WAL: silently loses confirmed commits if the damage is in the middle of the file (the medium-high defect of M0).
5. **AEAD slot** (ADR-0003 §2, ADR-0006).
   - **40-byte slot (`nonce[24] ‖ tag[16]`) always present in every SST block and every MANIFEST record; in WAL frames, only if that WAL's header has the `AEAD` bit (per file). With `AEAD` disabled the slot must be zero and the CRC covers it (AEAD-1).** A WAL never survives a clean shutdown, so enabling encryption does not require migrating it, and small commits do not pay 40 bytes per frame (T7).
   - Slot always present in the WAL too: a single constant, `FRAME_SLOT_ALWAYS` (`format::features`), and the specification (WAL-R6, v1.md §5) already cover this variant; the vectors include a frame with a slot (`frame_with_slot`).
   - Slot only with the bit enabled in all types: enabling encryption would force rewriting all SSTs and the MANIFEST.
6. **Range tombstones in the v1 base** (ADR-0001 condition 2, C-11).
   - **`DELETE_RANGE` in the canonical commit, a block of fragmented ranges in the SST and `largest_exclusive` in `FileMeta`, without an `incompat` bit.** Every v1 reader understands them from day one; SSTs of ranges only are valid.
   - Adding them later with an `incompat` bit: every base created earlier would be left unable to suppress ranges without a migration.
7. **One version per user key in each SST** (SST-I1).
   - **Each SST contains at most one version per key; snapshots retain whole files (W-6, SEM-5).** The reader does not need to merge versions inside a table, and visibility is decided by sequences alone.
   - Multi-version SSTs: they would arrive with an `incompat` bit if retention of long snapshots required it.
8. **`Incompatible` and `ReadOnly` errors** (ADR-0002, C-16).
   - **Two new variants of `kerno_core::Error`, which is marked `#[non_exhaustive]`.** An unknown version or feature is not damage and must not be presented as `Corruption`; the header CRC is checked before the version (HDR-3 before HDR-4) so that random damage never looks like `Incompatible`.
   - Reusing `Corruption` or `InvalidArgument`: it would confuse whoever decides whether to restore a copy or update the binary.
9. **Automatic read-only on an unknown `ro_compat` bit** (ADR-0003 §1, HDR-7).
   - **The whole database opens read-only and `is_read_only()` reports the file and the bits.** It is ADR-0003's semantics taken to the database level: a file with an unknown `ro_compat` bit means no v1 writer may touch the set.
   - Rejecting with `Incompatible`: prevents reading data that a v1 reader understands.
   - Opening read-write per file: a writer would rewrite that file in a compaction without keeping what it does not understand.
10. **`DRAFT` bit and freeze process** (ADR-0003, v1.md §4.3 and §15).
    - **Until the freeze every file carries `incompat` bit 63; when freezing (WP14) the bit leaves the known set and a database of the draft is rejected with `Incompatible("pre-freeze draft format")`.** No database created with rules that could still change is read with the final rules. While it applies, changing a byte requires a history entry in v1.md, vectors regenerated with a note in this ADR, and `spec_decoder_agrees`.
    - No marking: a draft database could be opened after the freeze and read wrongly.
11. **Format limits versus API limits** (v1.md §1.4, C-26).
    - **Readers apply broad format limits (`Corruption`); the writer applies stricter API limits (`InvalidArgument`) before writing anything.** Raising an API limit within the format limit is not a format change.
    - A single limit: any future widening would be a format change.
12. **Deferred per-commit durability** (ADR-0002, C-05).
    - **Phase 1 offers durability per database (`Sync`/`Buffered`) and `sync()`; `commit_with` is left for later.** The format already admits it: `synced_len` describes any mix of synchronized and unsynchronized frames.
    - Implementing it now: more states to test in the crash sweeps without a use case requiring it in Phase 1.
13. **Verified platform: Linux only** (ADR-0002, C-28).
    - **The guarantees of `Sync` are verified on Linux (ext4/xfs); on another platform, opening with `Sync` requires `allow_unverified_platform`.** macOS (`F_FULLFSYNC`) and Windows remain unverified.
    - Promising the same guarantees everywhere: there is no evidence to support them (C-28).
14. **Performance targets T1–T10** (ADR-0001 condition 1, ADR-0012).
    - **The targets are those of [phase1-design.md](../architecture/phase1-design.md) §7.2, recorded before measuring (WP04) and approved by the owner (C-06).** This ADR cites them by reference: it does not duplicate them so that they do not diverge.

15. **WAL preallocation and recycling without an `incompat` bit** (follow-up of WP05, ADR-0001 condition 1).
    - **The writer leaves the WAL as `header ‖ zeros` and reuses an already-flushed WAL after overwriting it entirely with zeros; the reader does not change.** It is the zero padding that §6.1 and WAL-R4 already admit, so no valid file changes its bytes and the vectors remain identical (v1.md §6.5, note "Preallocation and recycling"). The frames of a recycled file's previous life would not be reproduced (their CRC covers another file number), but an intermediate WAL of the chain that ends in them would be a broken tail (REC-10a): that is why they are overwritten with zeros before the file is renamed to its canonical name.
    - Recycling without zeroing, with the `incompat` bit reserved: it saves the zero write, but requires the reader to accept as a clean end a file that ends in foreign frames (a change to WAL-R4 and REC-10a that invalidates the crash sweep and the vectors). It is left for when measurement asks for that saving.

## Criteria for the decision

Set before implementing the codec:

- No accepted rule from ADR-0001/0002/0003 is relaxed: every byte that influences a result is covered by a CRC (I-08) and by a validation rule with an identifier.
- Random damage is never presented as `Incompatible`; an incompatibility is never presented as damage.
- What a crash leaves half-done is distinguished from later damage without time-dependent heuristics (I-03, I-11).
- Cost on the hot path: no byte per commit that is not necessary (T7) and no extra sync on clean reopening (T8).
- Each decision can be reverted before the freeze by changing the draft and regenerating vectors (v1.md §15).

## Evidence

- The codec (`crates/kerno-storage/src/format/`) implements v1.md §1–§8 and §12 as pure functions, with 194 vectors (45 positive, 149 negative covering each rule that a decoder can report, with its class).
- `spec_decoder_agrees`: a decoder written only from v1.md agrees with the codec in rule, class and content on all vectors and on 5 000 decoder-edited inputs on each run (1.4 million in a local run with `KERNO_AGREE_INPUTS=100000`), including edits within a block, a frame or a record with its CRC recalculated.
- `decoders_never_panic_on_random_bytes`: 10^5 inputs for each of the 14 decoders without panics.
- The bloom function is that of M0 (`bloom_matches_m0_function`), and the canonical form of the commit is equivalent to applying the batch operation by operation (`canonicalize_matches_sequential_application`, randomized).
- The behavioural evidence of the engine (crash sweeps, corruption at 1000 seeds, `kill -9`, LazyFS) arrives with WP07–WP14 and will be linked here before requesting acceptance.

## Decision

The options marked in points 1–14 are adopted for format v1 and the Phase 1 engine protocol, as specified by [`v1.md`](../file-format/v1.md):

| # | Decision | Interprets | Specification |
|---|---|---|---|
| 1 | Common 96-byte header with `db_id` and file number | ADR-0003 §1 | v1.md §3, HDR-10 |
| 2 | No `CURRENT`; `MANIFEST` replaced by rename | ADR-0003 §3 | v1.md §2, §8.9 |
| 3 | `CLEAN` in the *checkpoint*; `SESSION_OPEN` on reopening | ADR-0003 §3 | v1.md §8.5, §9.4, MAN-R4 |
| 4 | `synced_len` per frame | ADR-0002 | v1.md §6.2, W-11, WAL-R5 |
| 5 | AEAD slot per block and record; in the WAL, per file (`FRAME_SLOT_ALWAYS = false`) | ADR-0003 §2 | v1.md §5 |
| 6 | Range tombstones in the base | ADR-0001 cond. 2 | v1.md §6.3, §7.4 |
| 7 | One version per key in each SST | ADR-0001 | v1.md SST-I1 |
| 8 | `Incompatible` and `ReadOnly` | ADR-0002 | phase1-design §4 |
| 9 | Read-only on unknown `ro_compat` | ADR-0003 §1 | v1.md HDR-7 |
| 10 | `DRAFT` bit and freeze | ADR-0003 | v1.md §4.3, §15 |
| 11 | Format and API limits separated | ADR-0003 §2 | v1.md §1.4 |
| 12 | Deferred per-commit durability | ADR-0002 | phase1-design §9 |
| 13 | Only Linux verified | ADR-0002 | phase1-design §5.10 |
| 14 | T1–T10 by reference | ADR-0001, ADR-0012 | phase1-design §7.2 |

## Consequences

- **Format:** v1 is defined byte by byte with rule identifiers; any later change after the freeze requires `format_version` or a new bit, vectors and an ADR.
- **Recovery:** the decision table (v1.md §9) is part of the format; WP08 implements it as a pure function shared by opening, read-only mode and `verify()`.
- **Encryption:** enabling AEAD (ADR-0006) will not require migrating SST or MANIFEST; WALs are already created with the bit.
- **Cost:** 32 bytes of frame header per group of commits and 44 per SST block; a clean reopening, 2 syncs.
- **`synced_len` and broken header (point 4):** the first frame of each WAL carries `synced_len = 0`, because W-1 does not sync the header. If after a crash the header of the most recent WAL is damaged (not by the crash, but by a medium fault) and that WAL only has frames with `synced_len = 0`, WAL-R6 treats it as a WAL without frames, and a commit confirmed in `Sync` can be lost without an error. It is a case of damage after writing, not of a crash; the M0 corruption suites act after a clean shutdown, when no WAL remains. The alternative is to sync the header in W-1 (one more `sync_data` per rotation, once every `write_buffer_bytes`), so that every frame would carry `synced_len ≥ 96` and WAL-R6 could reject any valid frame under a broken header. It is left as a decision for the owner (see Follow-up). **Closed (2026-10-31, M1 of the adversarial review):** the writer syncs the header in W-1 and WAL-R6 rejects any valid frame under a broken header.
- **WP06 clarifications** (history of v1.md, without changing any valid file): the broken-header rule of WAL-R6 now includes HDR-2 with an unknown *magic* (neither v1 nor M0's), because a header never synced can be left as zeros while the first frame does reach the disk (the `SimVfs` model produces this); the table in §13.1 fixes which rule each check reports and in what order, which is what allows two independent decoders to agree. The WP06 review corrected the bound of WAL-R5 and WAL-R6 for group frames (point 4) and added to the reader two limits of §1.4 that it did not apply (`last_seq` and `edit_seq` ≤ 2^63−1, so that no derived sequence overflows); no existing vector changed.

## What is immutable and what is versioned

With the acceptance, the owner fixes the boundary:

- **Immutable while v1 exists** (changing it requires a new `format_version`, not a bit): the *magic* and the 96-byte common header (§3), integer encoding, varints and checksums (§1.1–§1.2), the TLV encapsulation (§1.3), the WAL frame and SST block encapsulation (§6.2, §7.2), the SST footer (§7.8), the MANIFEST record (§8.2) and the semantics of the recovery table (§9), including WAL-R6 hardened by M1.
- **Versioned by feature bits** (§4): new capabilities that an old reader can ignore (`compat`), open only read-only (`ro_compat`) or must reject (`incompat`), among them AEAD (point 5) and per-commit durability (point 12).
- **Adjustable without touching the format:** API limits (point 11), tuning parameters (block and memtable sizes, `bytes_per_sync`, compaction) and the performance targets T1–T10, which remain by reference (point 14).
- The `DRAFT` bit (point 10) was withdrawn at the freeze of 2026-10-31; files written with it are rejected with `Incompatible("pre-freeze draft format")`.

## Follow-up

- ~~Before the freeze (WP14): acceptance by the owner, in particular of points 2, 3, 4, 5, 8, 9 and 10 (phase1-design §13, decision 2), and review of the threat model (R-07).~~ Done: accepted on 2026-10-31 and R-07 reviewed in WP14.
- ~~Execute the freeze (v1.md §15)~~: executed on 2026-10-31. `WRITE_INCOMPAT` = 0, `DRAFT` outside `KNOWN_INCOMPAT` and rejected by its name in HDR-6, all vectors regenerated with the guard, negative vector `draft-v1` added.
- ~~Decide whether W-1 syncs the WAL header~~: adopted (M1, 2026-10-31).
- Link the engine evidence here (WP07–WP14) and the measurement of T7 and T8.
- Change log of the draft's bytes (v1.md §15, table "Draft history"): the last is the freeze itself, which changes the `incompat` of the header (and its CRC) of every file written. From it on, a byte change requires `format_version` or a new bit.

## Amendment (2026-11-20): `incompat` bit 1, `BLOCK_LZ4`

[ADR-0023](0023-format-v2-cut-for-alpha2.md) step 3.3 defines `incompat` bit 1 for SSTs, through
the feature-bit path that the "What is immutable and what is versioned" section of this ADR
keeps open. A table with the bit stores each data block's payload as `codec ‖ body` (raw or LZ4,
v1.md §4.4). The block encapsulation, the header, the footer and every other structure are
unchanged, so `format_version` stays 1 and no frozen vector changes. On a WAL or MANIFEST the bit
is unknown (HDR-6). A reader of 1.0.0-alpha.1 refuses a table that has it (HDR-6), which is
consistent with the cut of ADR-0023 (alpha.1 databases are not read by alpha.2 and the other
way round).
