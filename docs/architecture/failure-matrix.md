# Failure matrix

> **Historical:** sections 1 to 5 describe M0 (Phase 0 spikes) and are kept as a reference. The matrix in force for the LSM v1 format is in section 6.

Status: in force; section 6 (LSM v1) is **finished** at the closure of Phase 1 (2026-10-29) · Annex of [ADR-0002](../adr/0002-transaction-model-and-durability.md) · Criterion 3 of [SAD §17.5](SAD.md#175-acceptance-criteria)

For each interruption point of the protocol of each candidate, this document indicates **which state must be observed after reopening**. The notation is:

- `N`: last confirmed commit (`commit` returned `Ok(N)`).
- `N+1`: commit in progress when the failure occurred (not confirmed).
- "`N` or `N+1`": both are correct; never an intermediate state, never `< N`.

Failure types (modelled by [`SimVfs`](../../crates/kerno-testkit/src/vfs/sim.rs)):

| Failure | What survives |
|---|---|
| **Process death** | Everything the process wrote (the OS cache survives). |
| **Power loss** | Only what was synchronised; each pending write is lost, applied or torn (at 512 B or, in half of the seeds and points, at any byte), in any order; unsynchronised directory operations survive as a prefix. |
| **Isolated EIO** | One operation fails (no effect; a failed `fsync` leaves its writes without guarantee) and the device keeps working. |

Since Phase 1, `SimVfs` also offers optional models, disabled by default (the default behaviour, including the draws of the random generator, is fixed by the test `default_mode_crash_images_unchanged`):

| Failure (`SimVfs` option) | What survives |
|---|---|
| **Process death without loss of what is pending** (`CrashKind::ProcessKillKeepPending`) | The same as process death, but what is not synchronised remains pending: a later power loss can still lose it. |
| **Failed `fsync` with Linux semantics** (`FsyncFailure::LinuxDropPages`) | The writes covered by the failed `fsync` are discarded for ever even though readers keep seeing them, and the next `fsync` returns `Ok` without persisting them; length changes (`set_len`) are metadata and remain pending until the next successful `fsync`. By default (`FsyncFailure::KeepPending`) they remain pending, not lost. |
| **Per-directory durability** (`DirDurability::PerDir`) | `sync_dir(d)` persists only the operations on entries of `d` (a `rename` counts for the destination directory), together with the earlier ones they depend on (same inode or same name); a new directory is durable only when its parent is synchronised. |
| **Disk full** (`set_space_limit`, `fill_disk_after`) | The write or the `set_len` that would grow the used space above the limit fails with `StorageFull` (ENOSPC) and has no effect; a deleted or replaced file frees its space when its last handle is closed, as in Linux. |

## 1. How it is verified

The matrix is not only documentation: each row is exercised automatically.

| Test | What it walks | Expected result checked |
|---|---|---|
| `crash::every_point` | Crash before **each** I/O operation of a fixed scenario (creation, 25 commits, clean close, reopening, 20 commits, dirty close, recovery, 10 commits, close): process death and power loss in `Sync`, and process death in `Buffered` | state = commit `s` with `confirmed ≤ s ≤ attempted`; the recovered database accepts one more commit, closes, and that state survives a later power loss |
| `faults::single_failure_at_every_point` | An EIO at **each** I/O operation of the same scenario, followed by restart or immediate power loss | error returned, handle poisoned, no commit confirmed with a failed `fsync`, correct recovery |
| `crash::run` | Crashes at random points of long histories (5 rounds per seed, including crashes **during** recovery and during background work), in `Sync` and `Buffered`; power loss in `Buffered` only for engines that declare a consistent prefix | the same as above, chaining recoveries; in `Buffered` + power loss, any prefix from the start of the round |
| `corruption::*` | Bit flips and truncations after a clean close | correct data or an error; never other data nor rollback |
| `shared_ext_failure::*` (`extended_failure_suite!`, today over `LogEngine`) | The optional models at **each** I/O operation of the previous scenario: process death without loss of what is pending followed by an immediate power loss (`Sync` and `Buffered`), an EIO with Linux semantics (`Sync`; `Buffered` with restart), an EIO in `Buffered`, power loss with per-directory durability (`Sync`) and disk full | `confirmed ≤ s ≤ attempted`; after process death and the immediate loss, exactly the recovered `s` (recovery made its state durable); with disk full, only `Io` or `Poisoned` errors and correct recovery when space is freed; at the end, one more commit, close, power loss and comparison with the model |
| `process_kill.rs` (`kerno-storage`, WP14) | Real `kill -9` (SIGKILL) of a writer process (and, in two more tests, of one with 4 writer threads in group commit, with and without `wal_preallocate`) with `StdVfs` on a real disk, in `Sync` and in `Buffered`, at random moments (in the middle of a commit, flush or compaction, or during start-up and recovery of the child itself); each iteration continues from the state the previous one left, and a clean close alternates with an abandoned handle | `s ≥` last commit that the child reported as confirmed, content exactly equal to the model after the batches `1..=s`, `verify()` with no incidents before opening. 20 iterations per mode in CI, 200 in the weekly and release run (`KERNO_KILL_ITERS`) |
| `lazyfs.rs` + `spikes/storage/run-lazyfs.sh` (WP14) | Scenario A on a LazyFS mount with `StdVfs`; after the writer, the unsynchronised pages are discarded (`lazyfs::clear-cache`), with a control test that shows that the mount does lose data without `fsync` | `durable ≤ s ≤ last` and content equal to the model, with `durable` = everything confirmed in `Sync` and the result of `sync()` in `Buffered`. **Its record is an exit criterion and as of 2026-10-29 it is pending** (see [F1 report](../testing/f1-report.md)) |

The concrete points of the scenario vary with the engine (each one performs a different number of operations); the rows below group those points by phase of the protocol. The tests check the rule common to all rows (`N ≤ s ≤ N+1` with respect to the commit in progress), not the concrete value each row predicts: the rows explain why the rule holds at each point. The rows of `Buffered` mode are exercised with random crashes (`crash::run`) and with the process-death sweep in `Buffered`; there is no power-loss sweep in `Buffered`, because there the only guarantee is the prefix (LSM) or none (B+ tree).

## 2. Candidate A: copy-on-write B+ tree ([design](../../spikes/storage/spike-btree/DESIGN.md))

`Sync` commit protocol: write new pages (never over pages reachable from the current meta or from a live snapshot) → `fsync` → write meta into the slot that does **not** contain the current meta → `fsync`.

| # | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| B1 | Before writing anything (validation, application in memory) | `N` | `N` | Nothing touched the disk. |
| B2 | During the writing of new pages | `N` | `N` | The pages go to free ids or beyond the high-water mark; the meta of `N` does not reference them. |
| B3 | Before or during the first `fsync` | `N` | `N` | Idem; the pages may be broken, but nothing references them. |
| B4 | During the writing of the meta | `N` or `N+1` | `N` or `N+1` | The written slot may remain broken (invalid crc → the other slot is used, `N`) or complete (`N+1`, whose pages are already durable). |
| B5 | Before or during the second `fsync` | `N` or `N+1` | `N` or `N+1` | Idem. |
| B6 | After the second `fsync`, before returning | `N+1` | `N+1` | Durable commit; not yet confirmed to the caller. |
| B7 | Creation: before the `rename` of `data.kbt.tmp` | empty database | empty database | The temporary is deleted on opening. |
| B8 | Creation: after the `rename`, before the `fsync` of the directory | empty database | empty database (with or without the file) | Every opening synchronises the directory before accepting commits. |
| B9 | Opening | previous state | previous state | Recovery writes no data: `fsync` of the parent directory and of the database directory, and `fsync` of the file **before** trusting the chosen meta. |
| B9b | Crash between writing the meta of `N+1` and synchronising it; restart; power loss during the next commit | — | `N` or `N+1` | The `fsync` on opening makes the visible meta durable before reusing the pages it freed. Without it (a defect found in the M0 review) the only durable tree could be overwritten. |
| B10 | Clean close (copy of the meta to the second slot) | `N` | `N` | Both slots valid, with `N` or `N−1`/`N`; the newer is `N`. |

**`Buffered` mode** (no `fsync`): on process death, `N` or `N+1`. On power loss there is **no prefix guarantee**: a new meta may reach the disk before its pages and the freed pages are reused before being durable. The Merkle checksums guarantee that the result is the exact state of some commit **or** a detected `Error::Corruption`, never wrong data; in simulation, ~87% of the cuts (130 of 150) left the database unable to open until repaired and another 12% opened with damage detected on reading. See ADR-0002.

**Isolated EIO**: any error after validation poisons the handle; after reopening, `N` or `N+1`.

**`fsync` per `Sync` commit**: 2 (`sync_data` of the pages + `sync_data` of the meta). No `sync_dir` per commit.

## 3. Candidate B: LSM tree ([design](../../spikes/storage/spike-lsm/DESIGN.md))

`Sync` commit protocol: record `[len|crc|seq|batch]` in the active WAL (one `write_at`) → `fsync` of the WAL → insertion into the memtable → publication of the sequence.

| # | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| L1 | Before writing the record | `N` | `N` | |
| L2 | During the writing of the record | `N` or `N+1` | `N` | A broken record fails its crc and is the tail of the newest WAL → it is ignored. |
| L3 | Before or during the `fsync` of the WAL | `N` or `N+1` | `N` or `N+1` | The record may have arrived complete. |
| L4 | After the `fsync`, before publishing | `N+1` | `N+1` | Durable; replayed from the WAL. |
| L5 | Memtable switch: `fsync` of the old WAL (only `Buffered`), creation of the new WAL, `fsync` of the directory | `N` | `N` | The new WAL is still empty; if its directory entry is lost, it contained nothing. |
| L6 | Flush: writing/`fsync` of the new SST | `N` | `N` | Orphan SST (not in the MANIFEST) → deleted on opening; the data are replayed from the WAL. |
| L7 | Flush: writing of `MANIFEST.tmp`, `fsync`, `rename`, `fsync` of the directory | `N` | `N` | The old MANIFEST (+ WAL) or the new one (+ SST) survives: same logical state. |
| L8 | Flush: deletion of the already flushed WAL | `N` | `N` | WAL with number `< log_number` → deleted on opening. |
| L9 | Compaction: writing of the output SSTs | `N` | `N` | Orphan outputs → deleted on opening. |
| L10 | Compaction: MANIFEST change | `N` | `N` | Atomic (temporary + `rename`); inputs and outputs contain the same logical state. |
| L11 | Compaction: deletion of obsolete inputs | `N` | `N` | They are deleted only when a durable MANIFEST no longer references them and no snapshot uses them. |
| L12 | Recovery: `fsync` of the parent directory, replay of the WAL → SST L0 → MANIFEST with new `log_number` → deletion of old WALs → new WAL | previous state | previous state | Until the new MANIFEST is durable, the old one and its WALs remain in force; afterwards, the SST contains what was replayed. Recovery may be interrupted and repeated. |
| L13 | Clean close: flush of memtables, MANIFEST, deletion of WALs, compactions until no level exceeds its target | `N` | `N` | Combination of L6–L11. |

**`Buffered` mode**: on process death, `N` or `N+1`. On power loss, **a consistent prefix** of commits not earlier than the state reported by the last opening: the old WAL is synchronised before a new one is created (only the most recent may have holes and the replay stops at the first), and SST + MANIFEST are durable before the WAL they replace is deleted.

**Isolated EIO**: any error in the commit or in a background thread poisons the handle; if the `rename` of the MANIFEST succeeded but its directory `fsync` failed, nothing is deleted (the next clean recovery removes what the surviving MANIFEST does not reference).

**`fsync` per `Sync` commit**: 1 (`sync_data` of the WAL). Amortised: 1 `sync_dir` per memtable switch; 4 per background flush (SST, directory, MANIFEST, directory).

## 4. Media damage after a clean close

| Damage | B+ tree | LSM |
|---|---|---|
| Bit flip in a meta slot | The other slot has the same `N` → correct | — |
| Bit flip in MANIFEST | — | `Corruption` on opening |
| Bit flip in a referenced page/data block | `Corruption` on reading (crc in the parent) | `Corruption` on reading (crc of the block) |
| Bit flip in index / bloom / footer | — (they do not exist) | `Corruption` on opening (crc verified on loading) |
| Bit flip in a free page or unused bytes | harmless | — (there are no uncovered bytes) |
| Truncation of any file (even to 0 B) | `Corruption` on opening (length < `page_count × 4096`; a file without a valid meta is never reinitialised) | `Corruption` on opening (size checked against the MANIFEST; MANIFEST empty or absent with data present) |

**Damage after a dirty close.** B+ tree: each slot keeps two copies of the meta in different sectors; damaging one copy of the most recent meta changes nothing (before the M0 review, a single bit silently rolled back to the previous commit). Only if **both** copies are damaged does it roll back to the previous commit. LSM: damage to a WAL record followed by a valid record written in `Sync` mode is reported as `Corruption` instead of being truncated (before the review, confirmed commits were silently lost); damage to the **last** record remains indistinguishable from a torn write of the commit in progress.

## 5. Outside the model and limits of the evidence

- Damage to the medium **during** use or combined with crashes in `kerno-storage`: covered since WP09 by `damage_during_use_is_detected_or_harmless` and `damage_after_unclean_crash_never_returns_wrong_data` (an error or exact data; after a crash, never a sequence lower than the confirmed one unless the damage is in the last frame of the newest WAL, indistinguishable from a broken tail) and by `verify_detects_every_bit_flip`. The M0 engines do not apply it.
- Tests with real processes and devices: the lock between processes is tested by `process_lock.rs` (WP09) and the death of a real writer by `process_kill.rs` (WP14). A process death does **not** lose the OS cache, so those tests demonstrate the confirmed prefix and the WAL recovery, not durability against a power loss: that is demonstrated only by `SimVfs` (model) and, when it is recorded, by LazyFS. **The LazyFS record is pending** as of 2026-10-29 (`/dev/fuse` existed in the WP14 environment, but LazyFS could not be built there); `dm-log-writes` has not been tried.
- Failures during format migrations (they do not exist yet).

## 6. LSM v1 (`kerno-storage`)

Matrix of the Phase 1 engine ([design](phase1-design.md), §6.3). The rows are assigned to I/O points by `(phase, file type, operation)` in `crates/kerno-storage/tests/coverage.rs`, which runs scenario B in `Inline` mode and fails if a row receives no point. The rows with a prime (L5′, L7′, L10′, L12′, L13′) replace those of M0 by the protocol of the MANIFEST edit log.

| Row | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| L1–L4 | commit: before writing the record, during its writing, before or during the `fsync` of the WAL, after the `fsync` | `N` or `N+1` | `N` or `N+1` (`N` if the record was broken or did not arrive) | A broken record fails its crc and is the tail of the newest WAL → it is ignored; after the `fsync` it is durable and is replayed. |
| L5′ | rotation: creation of the new WAL (or `rename` of `WAL.spare`) and writing of its header, `sync_dir`; with `wal_preallocate`, also the preparation of `WAL.spare` from scratch (phase `wal_spare`) | `N` | `N` | The header is synchronised (`sync_data`, W-1) before accepting the first frame: a WAL without frames is an orphan and is ignored; the old WAL remains in force. As no frame precedes a durable header, a broken header with a valid frame behind it is `Corruption` even if its `synced_len` is 0 (WAL-R6; vectors `wal-one-frame-broken-header` and `dirty-wal-one-frame`). A half-made `WAL.spare` is not a canonical name: it is never replayed and is deleted on opening. |
| L6 | flush: writing and `fsync` of the SST (with `bytes_per_writeback`, also the `start_writeback` requests between chunks) | `N` | `N` | Orphan SST (no `FLUSH` record references it) → deleted on opening; the data are replayed from the WAL. A writeback request (`sync_file_range` on Linux) guarantees nothing: what it covers remains pending until the `fsync`. Only `start_writeback` is advisory (its error is recorded for the final `fsync` and ignored); the error of `wait_writeback` is **not** advisory, because on Linux it consumes the deferred write error and the final `fsync` would return `Ok`: it is propagated and poisons the handle (only `Unsupported`/`InvalidInput` are tolerated). `SimVfs` with `FsyncFailure::LinuxDropPages` discards for ever the writes that a failed `WaitWriteback` covers (`write_amp_levers::a_failed_wait_for_writeback_never_loses_acknowledged_commits`). |
| L7′ | flush: `FLUSH` record in the MANIFEST | `N` | `N` | A broken tail of the MANIFEST is accepted after a crash; if the record is complete, the SST and the new `log_number` are in force: same logical state. |
| L8 | flush: deletion of the already flushed WAL or, with `wal_preallocate`, its recycling (`rename` to `WAL.spare`, zeros, `sync_data`, `sync_dir`) | `N` | `N` | WAL with number `< log_number` or `WAL.spare` → deleted on opening; the recycling only starts when the MANIFEST no longer needs that WAL. |
| L9 | compaction: writing of the output SSTs (also `start_writeback` requests; with `dynamic_base_level` the output may go to L2 with L1 empty, and the `COMPACTION` record carries that level) | `N` | `N` | Orphan outputs → deleted on opening. |
| L10′ | compaction: `COMPACTION` record | `N` | `N` | A single atomic record; inputs and outputs contain the same logical state. |
| L11 | compaction: deferred deletion of obsolete inputs | `N` | `N` | They are deleted only when the MANIFEST no longer references them and no snapshot uses them; those not deleted are orphans and are deleted on opening. |
| L12′ | session start (S-2..S-5): replay, recovery flush, `SESSION_OPEN`, deletion of old WALs, new WAL | previous state | previous state | Until the new record is durable, the previous state and its WALs remain in force; recovery is reentrant (REC-10c). |
| L13′ | clean close (W-9): flush, mandatory and budgeted compaction, `CLEAN` checkpoint, deletion of WALs | `N` | `N` | Combination of L6–L11 and CK. |
| CK | checkpoint: `MANIFEST.tmp`, `fsync`, `rename`, `sync_dir` | `N` | `N` | The old or the new MANIFEST survives, both with the same logical state. |
| CR | creation (DIR-3) | empty database | empty database (with or without files) | Every opening synchronises the directory before accepting commits; a directory half-created is completed or rejected without reinitialising data. |
| R0 | reopening after `CLEAN`: `SESSION_OPEN` record | `N` | `N` | A broken `SESSION_OPEN` is a broken tail of the MANIFEST; the previous `CLEAN` state remains that of `N`. |
| RT | flush or compaction that writes SSTs with range fragments (also SSTs without points), and recovery with ranges in the WAL | `N` | `N` | The same rules as L6, L7′ and L9–L11: an SST with fragments is an orphan until a MANIFEST record references it, and a record is atomic. The ranges are in the WAL (canonical form, WAL-C1) until the `FLUSH`: after any crash they return to the memtable and hide what they hid. A compaction that discards covered points or fragments with nothing below does so in the outputs, which are orphans until their `COMPACTION` record. |
| DC | `compact_range`: the targeted compactions (phase `compact_range`: output SSTs, `COMPACTION` records with a targeted reason, deletion of inputs) and the checkpoint that follows (`checkpoint`, row CK) | `N` | `N` | Each step is an ordinary compaction or checkpoint: the logical state does not change between one step and the next, so a crash in the middle leaves a correct database, only less compacted. What is retained (snapshots, medium, memory) is not a guarantee of deletion: `crates/kerno-storage/DESIGN.md`. |
| RO | read-only opening, `verify()` and `inspect()` | `N` | `N` | **Zero** mutating operations (I-18); there is no failure point of its own. Tests: `read_only_open_writes_nothing_*`, `verify_is_read_only` (also on damaged databases), `inspect_reports_versions_and_bits`. |
| FL | failed `fsync` with Linux semantics | error; no confirmed commit | `N` | The handle is poisoned; the discarded writes are never confirmed. |
| NS | disk full (ENOSPC) | `Io` or `Poisoned` error | `N` | No effect on what is confirmed; the database recovers when space is freed. |
| KP | process death that leaves unsynchronised data pending, followed by power loss | `N` or `N+1` | `N` or `N+1` | Recovery makes its state durable before accepting commits. |

The rows of groups and `sync()` (G1–G3, S1–S2, FLb) are in the following subsection (WP12b). RT and DC (ranges, `compact_range`) were implemented by WP10b. `failure_matrix_rows_are_covered` covers the rows of WP08 and `range_rows_are_covered` those of ranges (DC is derived from the phase `compact_range` of the trace; RT is recorded when an SST with fragments is observed during the scenario); the crash sweeps are `range_delete_crash_at_every_point` and `compact_range_crash_at_every_point`.

**SC (subcompactions).** They are built by WP05 (`max_subcompactions`, and `close_subcompactions` for the close) and **do not add a failure point of their own**: a compaction divided into subintervals writes its outputs as orphan SSTs and installs them with **a single** `COMPACTION` record (`subcompactions_install_single_manifest_write`), so they are governed by rows L9–L11. There is no SC row in `coverage.rs`: they are covered by the sweeps of rows L9–L11, which use `Background::Inline` (one subcompaction), and by the earlier test, which fixes the single edit. **The default close** (`Background::Threads` with `close_subcompactions: 8`) **has no crash sweep of its own**: it is covered only by the functional test `close_compaction_uses_close_subcompactions` (without injecting crashes) and by the installation property above; it is a known limitation of the default configuration, not of a rare option. The risk is bounded: the outputs are orphans until a single `COMPACTION` record, and `compaction::apply` rejects overlapping outputs with `Corruption` without installing anything.

**Final state of the rows.** All the rows of this section have a test that exercises them, except those already indicated: SC (above) and the devices with page loss (LazyFS, record pending). The suite was run with 1000 seeds in the Phase 1 output without failures ([F1 report](../testing/f1-report.md)).

### `fsync` budget (LSM v1)

| Operation | `fsync` |
|---|---|
| Commit in `Sync` | 1 (< 1 with concurrency, through group commit) |
| WAL rotation | 1 `sync_dir` (+1 `sync_data` of the old WAL in `Buffered`) |
| Flush | 3 |
| Compaction | outputs + directory + 1 |
| Checkpoint | 2 + `rename` |
| Clean reopening | 2 |

### Group commit and `sync()`

WP12b provides the group rows (G1–G3), those of `sync()` (S1–S2) and of a failed `fsync` in `Buffered` (FLb). Notation: `N` = last confirmed commit before the group or the call; `N+k` = the group of `k` commits in progress, **complete**.

Commit protocol (`Sync`, one group): the leader takes `writer` and the FIFO group → `make_room` once → assigns consecutive sequences → **one frame** (`count = k`, one CRC) in **one** `write_at` → `sync_data` of the WAL → applies to the memtable in order → publishes `last_seq` and `durable_seq` **once**, after the `fsync` → completes the members and yields the leadership.

| # | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| G1 | With the group taken, before writing the frame (including `make_room`: memtable switch, WAL creation) | `N` | `N` | Nothing of the group touched the WAL. The members were not confirmed. |
| G2 | During the `write_at` of the frame of the group | `N` or `N+k` | `N` or `N+k` | A frame is a group with a single CRC: a torn write fails the CRC and is the broken tail of the newest WAL (it is ignored whole). **There is never a half-written group**; in `Sync` the `synced_len` of the frame (WAL-R5) distinguishes a broken tail from damage. |
| G3 | Before or during the `fsync` of the group, or after it and before publishing | `N` or `N+k` | `N` or `N+k` | The frame may have arrived whole. If the `fsync` fails, **all** the members receive `Io`, those queued behind the group receive `Poisoned` without anything else being written, and the handle is poisoned; after reopening, the group is whole or absent. Readers saw no sequence of the group (publication after the `fsync`, I-16). |
| S1 | `sync()` with unsynchronised frames (`Buffered`): before or during its `fsync` | prefix ≥ `N` | prefix ≥ last successful `sync()` | `sync()` queues a request without a batch; the leader does `sync_data` only if there are unsynchronised frames and returns the durable sequence. Until it returns, only what was before is valid. With the database in `Sync`, or with no pending frames, `sync()` does no I/O. |
| S2 | `sync()` whose `fsync` fails | `N` (everything confirmed) | prefix ≥ last successful `sync()` | `sync()` returns `Io` and poisons; `commit` and `sync` afterwards return `Poisoned` without I/O. |
| FLb | `Buffered` after a failed `fsync` with Linux semantics (`FsyncFailure::LinuxDropPages`) | everything confirmed | prefix ≥ last successful `sync()` | Linux discards the pages of the failed `fsync` and the next `fsync` returns `Ok` without persisting them: the `Buffered` commits after the last successful `sync()` may be lost even though they were confirmed. The engine does not retry (CONTRIBUTING): it poisons, and the recovered state remains a consistent prefix not earlier than that `sync()`. |

**`fsync` budget of the groups** (WP12b): a `Sync` commit costs **1 `sync_data` per group** of the WAL (exactly 1 with a single writer; less than 1 per commit with concurrent writers: `fsyncs_per_commit_below_half_with_8_writers`); `sync()` costs 0 in `Sync` and 0 or 1 in `Buffered` (1 only if there are unsynchronised frames). Rotation, flush and compaction do not change with respect to M0.

**How it is verified.**

| Test | Rows |
|---|---|
| `shared_concurrency::scenario_c_*` (scenario C: 4 writers × 10 commits with `set_sync_latency`; process death, power loss, death with pending data and EIO sweeps with both `fsync` models, in `Sync` and `Buffered`), on `KernoDb`, `Profiled<Inline>` and `Profiled<Serial>` | G1–G3 |
| `shared_concurrency::multi_writer_power_loss_prefix_no_holes`, `no_reader_sees_undurable_data` | G2, G3 (I-01, I-16) |
| `shared_sync::*` (`sync_makes_buffered_commits_durable`, `sync_failure_poisons`, `sync_on_poisoned_returns_poisoned`, `power_loss_at_every_io_point_buffered_with_sync`, `scenario_c_*_with_sync`) | S1, S2, FLb (I-17) |
| `group_commit.rs::failed_group_fsync_fails_all_members_and_poisons`, `queued_requests_get_poisoned_after_failure`, `leader_panic_poisons_and_wakes_followers`, `group_frame_torn_by_power_loss_recovers_prefix`, `sync_mode_readers_never_see_undurable_seq` | G2, G3 |
| `loom_leader_handoff`, `loom_poison_fanout`, `loom_txn_gate_drain`, `loom_backpressure` (`--cfg kerno_loom`) | the protocol of the queue |

The directory vectors `dirty-group-frame` and `dirty-buffered-synced-len` move to WP11 (which generates the directory vectors).

## 7. Units layer (`kerno-units`, Stage 3)

E3-08 provides the rows of the layer ([stage3-design.md §8.2–§8.3](stage3-design.md)). Notation: `N` = last `ChangeSet` confirmed (`commit`, `set_policy` or `run_retention` returned `Ok`); `N+1` = the `ChangeSet` in progress. A `ChangeSet` is **one** `Txn` of the engine and, therefore, **one** WAL frame: head (`UNIT_HEAD`), `REV_META`, `REV_PAYLOAD`, `CHANGESET`, `CS_MEMBER` and, where applicable, `LAYER_META` travel together. After each reopening it is required that the store be **byte for byte** the one a run without failures leaves after exactly `s` `ChangeSet`s (oracle `Emulated<LogEngine>`), that `verify_units` give no findings, and that the database keep working (one more `ChangeSet`, clean close, power loss, same state).

| # | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| U1 | While the `ChangeSet` is built (reads of heads and writes in the `Txn`, which do not touch the disk) | `N` | `N` | Nothing of the `ChangeSet` reached the WAL. |
| U2 | During the writing of the frame of the `ChangeSet` or before its `fsync` | `N` or `N+1` | `N` or `N+1` (`Sync`) | A frame with a single CRC: "a crash between the head and the `CHANGESET`" **is not an observable point**; no crash image contains a family of records without the others (I-21). |
| U3 | Large frame (several payloads of 8–24 KB) torn by power loss, at 512 B or at any byte | — | `N` or `N+1` | The torn frame is the broken tail of the newest WAL and is discarded whole (WAL-R); never a half-written `ChangeSet`. |
| U4 | Failed `fsync` in any `sync_data` of the scenario (WAL, SST, MANIFEST), with `KeepPending` and `LinuxDropPages` | `N` or `N+1` | `N` or `N+1` | The `ChangeSet` receives `Storage(Io)`; the following ones, `Storage(Poisoned)` without I/O. After reopening, whole or absent (I-10). |
| U5 | Disk full at any mutating operation, or with space only for the 96 B header of a new WAL and a little more | `N` or `N+1` (when space is freed and reopened) | — | `Storage(Io(StorageFull))` and then `Poisoned`; the layer does not leave a head without its revision nor members without `CHANGESET`. |
| U6 | Crash at any mutating operation of the recovery of a database whose `ChangeSet`s are only in the WAL | `N` | `N` | The second recovery gives the same store as a recovery without crashes (I-11). |
| U7 | Crash at any point of a `Retention` `ChangeSet` and of the close that dumps it to tables | `N` or `N+1` | `N` or `N+1` | Each unit is pruned whole or not pruned (IU-7, `retention_every_point`, E3-06). |
| U8 | Flush, rotation, deletion or recycling (`wal_preallocate`) of the WAL that contained the history; `compact` | identical history | identical history | The history lives in the tables like any key of the engine; retiring the WAL does not delete revisions (I-20). |
| U9 | Power loss in `Buffered` | — | prefix ≥ last clean close | Only what was synchronised survives; the recovered prefix is always of complete `ChangeSet`s. |
| U10 | Real `kill -9` of a process that confirms `ChangeSet`s in a loop (`StdVfs`, background threads, `Sync` and `Buffered`) | `N` or `N+1` | — | What was confirmed is in the OS cache; `verify()` of the engine and `verify_units` clean after each death. |

**How it is verified** (`crates/kerno-units/tests/crash.rs`, `process_kill.rs` and `retention.rs`; harness `tests/harness/units.rs`).

| Test | Rows |
|---|---|
| `{emulated_log,kerno}::changeset_every_point_{power_loss,process_kill}_{sync,buffered}` (crash before each mutating operation of a scenario with 12 `ChangeSet`s, clean close and dirty close; tear at 1 B at the odd points) | U1, U2, U9 |
| `{emulated_log,kerno}::model_crash_*` (5 rounds per seed with random crash, also during recovery, continuing on the recovered image) | U1, U2, U6, U9 |
| `{emulated_log,kerno}::recovery_reentrant_{power_loss,process_kill}` | U6 |
| `torn_write_large_change_set` | U3 |
| `fsync_failure_{keep_pending,linux_drop_pages}` | U4 |
| `enospc_fill_disk_every_point`, `enospc_space_limit_after_wal_header` | U5 |
| `retention::retention_every_point` (E3-06) | U7 |
| `wal_independence_{deleted,recycled}_wals` | U8 |
| `process_kill::kill_dash_9_changeset_writer_{sync,buffered}` | U10 |

## 8. Declared relations (`kerno-units`, Stage 4, E4-02)

E4-02 ([ADR-0014](../adr/0014-physical-representation-of-relations.md), option 2) adds three families of records: the relation record `REL` and its two adjacency indexes `REL_OUT` and `REL_IN` ([units-v1.md](../file-format/units-v1.md) §3.7–§3.8). The adjacency indexes are **not** secondary indexes with a freshness status (ADR-0007): they are written in the same `ChangeSet`, and therefore in the same WAL frame, as the record (I-21). Notation as in section 7: `N` = last confirmed `ChangeSet`; `N+1` = the `ChangeSet` in progress. After each reopening it is required that the store be **byte for byte** the one a run without failures leaves after exactly `s` `ChangeSet`s (oracle `Emulated<LogEngine>`), that `verify_units` give no findings (UNIT-36), that the walk through the index and the reference give the same, and that the live relations be those of an independent model.

| # | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| R1 | Crash at any mutating operation of a `ChangeSet` that **creates** relations (record and two entries per relation) | `N` or `N+1` | `N` or `N+1` (`Sync`) | `REL`, `REL_OUT` and `REL_IN` travel in the same frame with a single CRC: no crash image contains the record without its entries nor the entries without the record (I-21). |
| R2 | Crash at any mutating operation of a `ChangeSet` that **deletes** relations | `N` or `N+1` | `N` or `N+1` (`Sync`) | The same: the three keys are deleted in the same frame; no dangling entry nor a record without entries remains. |
| R3 | Crash during the `ChangeSet` that deletes the last relations of a unit **and** archives it | `N` or `N+1` | `N` or `N+1` (`Sync`) | Either the unit is still `Active` with its relations, or it is archived without any: never archived with a live relation (criterion 4 of ADR-0014; UNIT-36). |
| R4 | A `ChangeSet` is abandoned (`abort`) or an operation is rejected (archiving or deleting a unit with live relations) before committing | `N` | `N` | Nothing reached the WAL; the rejection does not mark the `ChangeSet` as broken and the rest of its operations remain valid. |
| R5 | Media damage that respects the engine's CRCs (equivalent to patching an SST and recomputing its CRCs): a `REL_OUT` or a `REL_IN` is missing, the `REL` is missing, or a relation points to a unit that is not `Active` | — | — | `verify_units` reports UNIT-36. The index walk does not answer with data it knows to be damaged: an index entry without `REL` is `Corruption`. |

**How it is verified** (`crates/kerno-units/tests/crash.rs` and `relations.rs`).

| Test | Rows |
|---|---|
| `{emulated_log,kerno}::relations_every_point_{power_loss,process_kill}` (crash before each mutating operation of 7 `ChangeSet`s with additions and removals of relations and a final archiving; 4 seeds; tear at 1 B at the odd points; after recovery, the store equal to the oracle's, `verify_units` clean, live relations equal to the model, index walk equal to the reference, and the database accepts a new relation and its deletion) | R1, R2, R3 |
| `{emulated_log,kerno}::abort_leaves_no_relation`, `archive_and_delete_are_rejected_with_live_relations` | R4 |
| `{emulated_log,kerno}::verifier_reports_index_damage` | R5 |

Rows R1–R3 are not repeated in `Buffered` nor with a failed `fsync` or disk full: the record and the indexes use the same commit path as any other `ChangeSet` (rows U2, U4, U5 and U9 of section 7, which cover those modes without depending on the content of the `ChangeSet`).

## 9. Primary index (`kerno-units`, Stage 4, E4-03)

E4-03 ([ADR-0007](../adr/0007-index-consistency-and-rebuilding.md)) adds the primary index: a status record `IDX_META` and an entry `IDX_PRIMARY` per `Active` unit ([units-v1.md](../file-format/units-v1.md) §3.9–§3.10). It is a **derived** index: `UnitDb::rebuild_index` builds it in its own `ChangeSet` (purpose `Index`) and it is not maintained by each `ChangeSet`. The build deletes the previous generation, writes the entries and the `IDX_META` in the same transaction, and its watermark is its own commit sequence. Notation as in section 7: `N` = last confirmed `ChangeSet`; `N+1` = the `ChangeSet` in progress. After each reopening it is required that the stored state be that of the recovered commit (`Ready` with its watermark if it was the last, `Stale` with the previous one if not), that `verify_units` give no findings (UNIT-37), that the queries give the same as the reference path, and that the database rebuild the index and leave it `Ready` in the following commit.

| # | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| I1 | Crash at any mutating operation of a build (deletion of the previous generation, entries, `IDX_META`) | `N` or `N+1` | `N` or `N+1` (`Sync`) | Everything travels in the same frame with a single CRC (I-21): the image contains the previous generation whole (with its watermark, now obsolete) or the new one whole; never a mixture nor entries without `IDX_META`. |
| I2 | The build fails because of the data (a head that does not decode, a batch above the engine's limits) | — | — | No entry is confirmed. `Failed` is recorded, with the watermark and the number of entries of the generation that remains stored, in a second `ChangeSet`. If the engine is not usable (`Io`, `Poisoned`) it is not recorded: the stored state remains the previous one and remains true. |
| I3 | Media damage that respects the engine's CRCs: an entry is missing or extra, an entry differs from its head, or `IDX_META` is missing or disagrees | — | — | `verify_units` reports UNIT-37. An index query rejects what it sees (an entry that does not match the head of the unit it returns is `Corruption`); what its filter discards is seen only by the verifier, the differential test or a rebuild. |

**How it is verified** (`crates/kerno-units/tests/index.rs`).

| Test | Rows |
|---|---|
| `{emulated_log,kerno}::index_every_point_{power_loss,process_kill}` (crash before each mutating operation of 5 `ChangeSet`s: insertion, build, schema change, rebuild, archiving; tear at 1 B at the odd points; after recovery, the state is that of the recovered commit, `verify_units` clean, the queries equal to those of a model and to the reference path, and the database rebuilds) | I1 |
| `{emulated_log,kerno}::failed_build_is_recorded_and_repaired`, `status_survives_reopen` | I2 |
| `{emulated_log,kerno}::verifier_reports_index_damage`, `index_path_refuses_what_it_can_see` | I3 |

E4-04 adds the secondary index `IDX_TYPE` ([units-v1.md](../file-format/units-v1.md) §3.11): the same build writes it, in the same transaction and under the same `IDX_META`, so rows I1 and I2 cover it without changes (the generation is one: the two classes of entries, whole or none). The crash test checks it because `check` compares each path (`IndexPath::Primary` and `IndexPath::ByType`) with the reference after each recovery.

| # | Failure point | Process death | Power loss | Why |
|---|---|---|---|---|
| I4 | Media damage that respects the engine's CRCs in `IDX_TYPE`: an entry is missing or extra, or is under another type or version | — | — | `verify_units` reports UNIT-38 (and only that rule if `IDX_PRIMARY` is fine). A query by type rejects an entry whose key does not match the head of the unit (`Corruption`); a missing one is seen only by the verifier, the differential test or a rebuild. Tested by `verifier_reports_index_damage` and `index_path_refuses_what_it_can_see` |

Rows I1–I4 are not repeated in `Buffered` nor with a failed `fsync` or disk full: the build uses the same commit path as any other `ChangeSet` (rows U2, U4, U5 and U9 of section 7).
