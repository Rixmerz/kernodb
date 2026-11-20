# ADR-0009 — Suppression, retention and backups

| Field | Value |
|---|---|
| Status | Accepted (2026-11-12, by the owner: option 3, under the terms of the section [Decision](#decision)) |
| Date | 2026-10-27 (opening); 2026-11-11 (criteria set before measuring); 2026-11-12 (accepted) |
| Deciders | Project owner (E9A-00b, [advance minutes of Stage 9](../architecture/stage9-early-record.md) §7, point 1) |
| Related to | SAD §8.4, §11.5, §12, NIST SP 800-88 Rev. 2 |

## Context

Deleting a current datum does not delete its revisions, indexes, embeddings, freed pages, old segments or backups.

**Question:** How to coordinate logical/physical deletion, revisions, indexes, embeddings and backups.

## Options considered

1. **Logical deletion + forced compaction/scrub** of the affected range.
2. **Cryptographic erasure** (keys per record or per collection, destroyed).
3. **Combination** according to the data class and the policy.

## Initial proposal (SAD §15)

Explicit policies per collection; suppression operations aware of derived data; documentation of backup expiry windows.

## Decision criteria

An end-to-end test shows what is removed immediately, what is subject to compaction and what deadline the backups follow.

### Criteria set on 2026-11-11, before measuring

These criteria are written before any execution of E9A-01 ([advance minutes of Stage 9, §3.2](../architecture/stage9-early-record.md)). They contain no results or predictions of results, do not choose an option and do not change the status of this ADR, which on that date was **Pending** (accepted on 2026-11-12, see [Decision](#decision)). Only the owner changes them, with a new date; they are not adjusted in view of a measurement.

**Unit of observation.** A sentinel *survives* a stage if it appears, in bytes or in its ASCII form, in some regular file of the database directory at that stage: `MANIFEST`, `MANIFEST.tmp`, `WAL.spare`, the `NNNNNN.wal` and the `NNNNNN.sst`. The key and the value are counted separately, because the MANIFEST keeps key ends of the files being retired ([DESIGN.md, `compact_range`](../../crates/kerno-storage/DESIGN.md)). The stages are those of E9A-01: before the flush, after `compact_range`, after clean shutdown and after reopening. Visibility (`get` and `scan`) is observed at the same stages.

**What "deleted" means.** In this ADR, *deleted immediately* means invisible to reads after the commit, not absent from the medium. The engine does not promise the latter ([threat model §6.5](../security/threat-model.md)), so no criterion in this section requires it.

#### Criteria for options 1 and 3

- **I-1 (immediate).** After a confirmed `delete` or `delete_range`, no new read of the same database returns the deleted value nor a key covered by the range ([v1.md §11](../file-format/v1.md), SEM-4). It is contradicted if a `get` or `scan` after the commit returns it.
- **I-2 (snapshots).** A snapshot taken before the commit keeps seeing the previous state until it is released ([v1.md §11](../file-format/v1.md), SEM-4 and SEM-5). It is contradicted if the snapshot sees the deletion before being released.
- **C-1 (live files).** After `compact_range` covering the key and a clean shutdown, every live file that still contains the sentinel must belong to one of the classes of "What can survive `compact_range`" in [DESIGN.md](../../crates/kerno-storage/DESIGN.md) that are visible in the directory: tables retained by a snapshot (counted in `pinned_files` and `pinned_bytes`) or point tombstones on the last level that a trivial background move left there. Any other live file with the sentinel contradicts option 1. Freed blocks, memory, cache and copies made outside the engine are not in the directory and are not part of this comparison.
- **C-2 (MANIFEST).** After the `compact_range` checkpoint, the MANIFEST does not contain the suppressed key, except as the end of a file that is still in the current version. It is contradicted if the key appears as the end of a file that is no longer in the current version.
- **C-3 (range tombstones).** After `compact_range`, no fragment of a range tombstone covering the compacted range survives in a live file that does not retain a snapshot. It is contradicted if one survives.
- **C-4 (deadline).** The deadline for file deletion is the return of `compact_range` for what no snapshot retains, and the release of the last snapshot that retains a file for the rest. The ADR writes it in those terms, which are observed in `pinned_files`, `pinned_bytes` and `snapshot_retained_bytes`, and not as a duration. It is contradicted if the count of retained files does not match the files that still contain the sentinel.
- **C-5 (scrub).** The "forced scrub" part of option 1 has no mechanism in the code: `compact_range` does not overwrite blocks (`crates/kerno-storage/src/db.rs:2009`). E9A-01 does not measure it. If the owner wants it, it is a separate package with its own criterion, and its physical check is outside what the engine can observe ([threat model §4](../security/threat-model.md)).

#### Criteria for option 2

Option 2 cannot be measured with E9A-01 while there is no encryption: the format rejects the `AEAD` bit with `Incompatible("encryption not supported")` ([threat model §6.3](../security/threat-model.md)). These criteria do not fix how keys are generated or stored, which ADR-0006 decides ([minutes §2.3](../architecture/stage9-early-record.md)).

- **K-1 (immediate).** After destroying the key of a record or of a collection, its data is not recoverable from the directory or from a copy, nor from a database reopened without that key. It is contradicted if a reopening without the key recovers the datum.
- **K-2 (compaction).** What is subject to compaction is encrypted bytes. Their presence does not contradict option 2 as long as K-1 holds. A plaintext sentinel in a file that should have been encrypted does contradict it.
- **K-3 (copies).** No key, live or destroyed, is in the data directory or in the copies the engine produces (T-03). It is contradicted if a copy contains a key.
- **K-4 (deadline).** The deadline of the suppression is the moment of key destruction. From then on, the persistence of the encrypted bytes stops mattering for suppression.

#### Criteria for option 3

- **A-1 (full assignment).** Option 3 declares, per collection and data class (SAD §15), which option applies. A collection without a class, or a class without an option, contradicts it.
- **A-2 (reference).** Each class assigned to option 1 meets I-1, I-2 and C-1 to C-5. Each class assigned to option 2 meets K-1 to K-4.

#### Backups (common to the three options)

- **B-1 (declared deadline).** For each data class, the ADR declares the window during which a backup copy may still contain the suppressed datum, as a finite duration, or declares that there is no window and why.
- **B-2 (not measurable with E9A-01).** E9A-01 neither confirms nor contradicts B-1. Backup copies are outside the engine ("windows are documented, not promised", [threat model §4](../security/threat-model.md)), and `crates/` has no export code ([minutes §2.3](../architecture/stage9-early-record.md)). B-1 is met with a written declaration that the owner accepts, not with a measurement. That is why the sentence of 2026-10-27 about an end-to-end test that demonstrates the backup deadline cannot be met with E9A-01, and this criterion specifies it.

#### What E9A-01 confirms or contradicts

| Criterion | Can E9A-01 confirm or contradict it? | Confirms | Contradicts |
|---|---|---|---|
| I-1 | Yes | Reads after the commit without the value | A `get` or `scan` that returns it |
| I-2 | Yes | The previous snapshot reads the previous value | The snapshot sees the deletion before being released |
| C-1 | Yes | Live files with the sentinel are only of the retained classes or of last-level tombstones | A live file with the sentinel outside those classes |
| C-2 | Yes | The MANIFEST after the checkpoint does not name the key | The key appears as the end of a file outside the current version |
| C-3 | Yes | No range tombstone of the scope in live files without a snapshot | A range tombstone of the scope in a live file without a snapshot |
| C-4 | Partially | The `pinned_*` and `snapshot_retained_bytes` counts match the files that retain the sentinel | A count that does not match those files |
| C-5 | No | — | — |
| K-1 to K-4 | No (there is no encryption) | — | — |
| A-1, A-2 | Only the part of option 1 | The option 1 classes meet I-1 to C-5 | An option 1 class that fails any of them |
| B-1 | No | — | — |
| B-2 | — | It is a finding about the scope of E9A-01 | — |

This section does not choose an option, does not fix any time or byte threshold and does not decide whether ADR-0009 is accepted: that was E9A-00b, and the owner decided it on 2026-11-12 ([Decision](#decision)). Threat T-09 of the threat model remains open.

## What M0 contributed

M0 made visible that physical deletion depends on the engine: in a CoW B+ tree, freed pages keep old bytes until reused; in an LSM, old versions persist in SST and WAL until compaction and recycling. See §5 of the [revision design](../architecture/revision-model-design.md).

## Dependencies

It depends on ADR-0001, ADR-0004 and ADR-0006.

## Decision

**Accepted on 2026-11-12 by the owner: option 3 (combination), under these terms.** The criteria of the previous section are kept as they were set on 2026-11-11, before measuring; this decision does not change them.

1. **Immediate guarantee: logical deletion.** After a confirmed `delete` or `delete_range`, later reads do not return the deleted value nor a key covered by the range. A snapshot taken before the deletion keeps seeing the previous state until it is released. Criteria I-1 and I-2.
2. **Removal of live files on compaction.** When `compact_range` covers the affected range and no open snapshot, taken before the deletion, retains a file, the live files of the directory stop containing the deleted value and key. While such a snapshot stays open, the tables it retains remain on disk and are released when it is released. Criteria C-1, C-2 and C-3, and the deadline of C-4. The count clause of C-4 is not accepted as confirmed (see the following table).
3. **No physical deletion.** It is not promised that freed blocks disappear from the medium: the engine has no mechanism to overwrite them. Criterion C-5 unmeasured; the limit is declared in the [threat model §6.5](../security/threat-model.md).
4. **Cryptographic erasure, deferred.** It is deferred until [ADR-0006](0006-encryption-and-key-management.md) (encryption and key management) is accepted, and only for data that requires it. Today the format rejects the `AEAD` bit with `Incompatible("encryption not supported")`, so there is nothing to measure. Criteria K-1 to K-4.
5. **Backups, with no declared deadline.** No backup window is declared. The deadline is set when a backup function exists in the engine; today there is none in `crates/`. Until then, this ADR asserts no deadline for backups. Criteria B-1 and B-2.

The terms do not include a suppression API, and this acceptance does not write one.

### Relation to the criteria

What is listed as "confirmed" is shown by the measurement of E9A-01 ([evidence](#evidence)). What is not confirmed is stated as such.

| Criterion | What the measurement shows | Status |
|---|---|---|
| I-1 | After `delete` and `delete_range`, at each stage, `get` and `scan` do not return what was deleted (`assert_visible_state`) | **Confirmed** |
| I-2 | The snapshot taken before the deletions keeps reading the previous state (`snapshot_unchanged`) | **Confirmed** |
| C-1 | After `compact_range` and clean shutdown, no live file contains the value or the key. Before shutdown, the only file with the sentinel is a table retained by the snapshot (`000007.sst`), counted in `pinned_files` | **Confirmed** for the stages observed. The class of point tombstones on the last level was not exercised |
| C-2 | After the `compact_range` checkpoint, the MANIFEST does not name the key, also with a snapshot open | **Confirmed** |
| C-3 | The range-tombstone fragments go from 1 to 0 after `compact_range` | **Confirmed** |
| C-4 | The deadline is observed: without a snapshot, `compact_range` retains no tables (`pinned_files=0`); with a snapshot open, on releasing it the three tables it retained disappear (`000003.sst`, `000005.sst`, `000007.sst`). The count does not match the files that contain the sentinel: `pinned_files = 3`, and only `000007.sst` contains the value and the key | **Contradicted as written** in its count clause (C-4 is contradicted if the count does not match). Point 2 takes from C-4 only the deadline. The test asserts that the files with the sentinel are among the retained ones, not that the count matches (`suppression_survival.rs`, lines 536–543) |
| C-5 | Not measured. `compact_range` does not overwrite blocks (documentation of `compact_range` in `crates/kerno-storage/src/db.rs`) | **Not measurable with E9A-01**; outside the terms (point 3) |
| K-1 to K-4 | The format has no encryption | **Not measurable** (point 4) |
| A-1 | Option 3 requires assigning each collection and data class to an option. The terms do not make that assignment; point 4 leaves it for when ADR-0006 is accepted | **Not met** |
| A-2 | Option 1 classes (points 1 to 3): I-1, I-2, C-1, C-2 and C-3 confirmed; C-4 contradicted in its count clause; C-5 unmeasured and excluded by point 3 | **Partial** |
| B-1 | A finite deadline, or the declaration that there is no window and why. The terms declare no window and point 5 defers it | **Not met.** Stays open until the backup function exists |
| B-2 | Backup copies are outside the engine (threat model §4) and E9A-01 does not measure them | **Not measurable** (finding about scope) |

### Evidence

Command, from the repository root, on 2026-11-12:

```
cargo test -p kerno-storage --all-features --release --test suppression_survival -- --nocapture
```

Verbatim extracts of the output:

```
[survival] P1 (delete: value still in the log, no flush): HOLDS (delete value in ["000004.wal"])
[survival] P1 (delete_range: value still in the log, no flush): HOLDS (range value in ["000004.wal"])
[survival]   compact_range: rewritten=4 pinned_files=3 pinned_bytes=30990 kept_tables=["000003.sst", "000005.sst", "000007.sst"] retained_bytes=31702
[survival] P2 (value in no live file after flush, compact_range, clean close): HOLDS (delete value in [], range value in [])
[survival] P3 (tombstone keeps the key in a live file until compaction discards it): HOLDS (before: ["000007.sst", "000009.sst", "MANIFEST"], after close: [])
[survival] P4 (flushed log is zeros and holds no value): HOLDS (spare is zeros: Some(true); delete value in ["000007.sst"]; range value in ["000007.sst"])
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

The test is an observation of the current behaviour, not a contract (its header says so). This ADR does not change it.

### Consequences

- What the engine promises is reduced to points 1 and 2. No product text should describe `delete` or `compact_range` as secure deletion ([threat model §6.5](../security/threat-model.md)).
- If the engine changes what E9A-01 observes, the test may change with it; this is not a regression of a guarantee, because the engine promises no more than points 1 and 2.

### Follow-up

- **C-4:** decide whether the `pinned_files` count must match the files that contain the sentinel, or whether the criterion is rewritten with a new date. It is not adjusted in view of the measurement.
- **A-1:** assign each collection and data class to an option. The terms do not fix when this is done.
- **Option 2 (K-1 to K-4):** depends on [ADR-0006](0006-encryption-and-key-management.md), pending.
- **B-1:** fix the backup deadline when a backup function exists in the engine. Until then, no deadline.
- **C-5:** a physical deletion of freed blocks would be a separate package with its own criterion ([threat model §4](../security/threat-model.md)).
