# ADR-0002 — Transaction model and durability

| Field | Value |
|---|---|
| Status | **Accepted** (2026-10-28, by the project owner, together with ADR-0001) |
| Date | 2026-10-27 (proposal) · 2026-10-28 (acceptance) |
| Related to | SAD §4.2, §6.2, §7.2, §7.3; [failure matrix](../architecture/failure-matrix.md); invariants I-01…I-11; ADR-0001 |

## Context

SAD §7.2 requires explicitly defining:

1. what it means for a transaction to be committed;
2. when it is synced and what the API returns if the sync fails;
3. how partial writes are detected;
4. how crashes before, during and after the commit are resolved;
5. how an interrupted format update is recovered;
6. which tool verifies or exports a damaged database without overwriting it.

The approval criterion (SAD §15) is a **failure matrix** with the expected state after reopening at each interruption point, and the documentation of the guarantees of each durability mode.

## Options considered

1. **WAL + commit protocol**, with a main structure that can be rebuilt (LSM; also a B-tree with a SQLite/InnoDB-style WAL).
2. **Copy-on-write with a commit root/meta** (shadow paging in the style of LMDB/redb): the commit is the atomic write of a meta that points to the new root.
3. **Hybrid variant** depending on the chosen engine (e.g. CoW with "1PC + checksum", or CoW with a small WAL for small commits).

Both main options were implemented in M0 and tested with the same failure matrix.

## Decision

### Commit semantics (independent of the engine)

| Concept | Definition |
|---|---|
| **Transaction** | A set of writes (put/delete) on a database, applied atomically. Kerno will have a single writer per database (D-03): the write transaction sees its own writes and the last committed state (serializable for writes); readers work on snapshots (snapshot isolation). |
| **Commit sequence** | A 64-bit, monotonic integer assigned by the database: each non-empty commit receives `previous + 1`. It is persisted, survives crashes, and is the transactional order (never the UUIDv7 timestamp, I-26). It serves as the *watermark* for derived data (ADR-0007). |
| **Committed** | `commit` returned `Ok(seq)`. In `Sync` mode this implies **durable against power loss**. In `Buffered` mode, it implies visible and durable against process death. |
| **Visibility** | A commit is visible to new snapshots when `commit` returns, and never before meeting the guarantee of its durability mode: in `Sync`, no reader observes data that a crash could lose. |
| **Snapshot** | View of the exact state after commit `seq`, stable while it lives, without blocking the writer. Long-lived snapshots retain old versions (pages or files) and will be exposed as a metric (age of the oldest snapshot, bytes retained). |
| **Empty commit** | Does not create a commit; returns the current sequence. |

### Errors and the result of a commit

| Error | Meaning for the caller |
|---|---|
| `InvalidArgument` | Rejected before writing. **Not applied.** |
| `Locked` | The database is open in another handle or process. |
| `Poisoned` | An earlier error left the handle in an unknown state; **not applied**. The database must be reopened. |
| `Io` during commit | **Indeterminate result** until reopening: after recovery the commit is either complete or absent, never half-done. The handle becomes poisoned. |
| `Corruption` | Damage was detected; no damaged data is returned. |

After a failed `fsync`, **it is not retried on the assumption that the data reached the disk** (I-10): the handle becomes poisoned and recovery decides based on what is actually on disk.

### Durability modes

| Mode | Guarantee | Cost observed in M0 |
|---|---|---|
| `Sync` (default) | Committed ⇒ survives power loss. | 1 `fsync` per commit in LSM, 2 in the prototype's CoW B+ tree. |
| `Buffered` | Committed ⇒ survives process death. On power loss, **the chosen engine must guarantee a consistent prefix** of commits (never damaged data). It is completed with an explicit `sync()` call and on `close()`. | 0 `fsync` per commit. |

M0 showed that this last guarantee **is not free in a CoW B+ tree**: without barriers, a new meta may persist before its pages, and freed pages are reused before they are durable; the prototype detects the problem (it never returns wrong data) but ~87 % of the simulated cuts left the database unable to open (and another 12 %, with damage detected on read). Offering it in a CoW requires periodic synchronization points and deferred reuse of pages (see the [prototype design](../../spikes/storage/spike-btree/DESIGN.md#durability-modes)). In an LSM it comes from the WAL's own design.

**Group commit** remains an optimization for Phase 1 for concurrent writers; it does not change the semantics.

### Detection of partial writes and recovery

- Every journal record, page, block, manifest or meta carries an explicit length and a checksum verified before use (ADR-0003).
- The last valid commit is identified unambiguously: the meta with the highest sequence and valid crc (CoW), or MANIFEST + replay of the WAL with consecutive sequences (LSM).
- A broken tail is only accepted at the end of the most recent journal after a crash; after a clean shutdown, or when followed by valid records, it is corruption.
- The files that define the state are created and replaced atomically (temporary → `fsync` → `rename` → `fsync` of the directory). An existing file that is empty or has no valid state is corruption, never a "new database".
- Recovery is automatic on open, **re-entrant** (it may crash and be repeated, I-11) and non-destructive: it only discards what the protocol defines as not committed (broken tails, orphan files not referenced by the durable state). On corruption, `open` fails with `Corruption` and **does not repair**.
- The complete matrix, per engine and per failure point, is in [failure-matrix.md](../architecture/failure-matrix.md).

### Interrupted format update

It will be designed in ADR-0003: the migration writes to new files (or to a new version of the state), and the version change is the same atomic step as a commit (meta or MANIFEST); the migration state is persisted so it can be resumed or aborted. M0 does not implement migrations.

### Verification tool

The CLI (Phase 2) will have `kerno verify` (reads everything, validates checksums and structure, does not write) and `kerno export --salvage` (exports what is readable to a logical format in another directory). No destructive repair runs automatically (SAD §12).

## Evidence (M0)

- Both prototypes pass, with 400 seeds in `release`: crashes at random points (including during recovery), a crash before **each** I/O operation of the fixed scenario (process death and power loss), an isolated EIO on each operation (restart and immediate cut), bit flips and truncations. See the [M0 report](../testing/m0-report.md).
- The suites were validated by injecting known faults into the reference engine (`fsync` omitted, `rename` without `fsync`, `fsync` error ignored, conditional directory sync); all were detected ([test plan](../testing/test-plan.md#21-harness-validation-manual-mutation-testing)).

## Consequences

- The Phase 1 public API (ADR-0010) will expose: `begin_write()` (write transaction with read-your-own-writes), `commit()`/`abort()`, `snapshot()`, `sync()`, and the durability mode per database and, optionally, per commit.
- The commit sequence is part of the persistent format and of the public contract.
- The engine chosen in ADR-0001 inherits the corresponding row of the failure matrix as a **specification**; any protocol change must update it and keep the suites green.

## Follow-up

- Phase 1: read-modify-write transactions on the chosen engine; explicit `sync()`; snapshot metrics; group commit if the benchmarks justify it.
- Phase 1: Linux semantics of a failed `fsync` and tests with real devices (R-S2).
- **Phase 1 follow-up (WP14, 2026-10-29; no change of decision).** The two points above are implemented in `kerno-storage`: pessimistic read-modify-write transactions (`begin_write`, [Phase 1 design §5.7](../architecture/phase1-design.md)), `sync()`, group commit and snapshot metrics; the Linux semantics after a failed `fsync` are tested with `FsyncFailure::LinuxDropPages` in `SimVfs`. Per-commit durability is still deferred: the format allows it (`synced_len` per frame). The protocol decisions that interpret this ADR are in [ADR-0013](0013-format-v1-and-engine-protocol.md) and in [the v1 format](../file-format/v1.md); the failure matrix rows are in [failure-matrix.md §6](../architecture/failure-matrix.md#6-lsm-v1-kerno-storage). Output evidence: a real `kill -9` of a writer with `StdVfs` in `Sync` and `Buffered` (`crates/kerno-storage/tests/process_kill.rs`) and the [Phase 1 report](../testing/f1-report.md). The test with a device that loses pages (LazyFS) is still **pending its record** (see that report).
