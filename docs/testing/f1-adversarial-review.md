# Adversarial review of Phase 1 (persistent storage core)

Status: read-only review, prior to merging `claude/quirky-allen-d65xsg` into `main` (HEAD `474b509`) · Scope: `crates/kerno-storage`, `crates/kerno-core`, [failure matrix](../architecture/failure-matrix.md), [F1 report](f1-report.md) · No file was modified except this document.

## 1. Summary: blockers first

| Id | Severity | Where | What |
|---|---|---|---|
| **B1** | **Blocker** | `crates/kerno-storage/src/sst.rs:169-185` (mainly `:182`) | The error of `wait_writeback` is discarded with `let _ =`. On Linux that error is **consumed**, and the SST's final `sync_data` returns `Ok`: a deferred write failure is taken as durable (it violates the ADR-0002 invariant that `db.rs:1699` cites) and confirmed commits are lost. It is active **by default** (`config.rs:512`). |
| G1 | Major (test gap) | `crates/kerno-testkit/src/vfs/sim.rs:1086-1089`, `docs/architecture/failure-matrix.md:124` | The simulator models a failure of `WaitWriteback` as "no effect", which is the opposite of Linux. That is why no sweep can detect B1, and row L6 of the matrix claims something that is only true of `start_writeback`. |
| M1 | Minor | `crates/kerno-storage/src/format/wal.rs:380-384, 449-463` | Damage to the header of a WAL with a single frame (or, in `Buffered`, without a later `sync()`) is taken as "header never synced" and discards confirmed commits without an error. It is in the specification (WAL-R6), but it is not listed among the limitations of the matrix (§4/§5). |
| M2 | Minor | `crates/kerno-core/src/vfs/std_fs.rs:135-142`, `crates/kerno-storage/src/db.rs:1766` | `AccessMode::ReadOnly` opens `LOCK` with `write(true).create(true)`: it cannot open a database on a read-only medium or directory. |
| G2 | Minor (test gap) | `crates/kerno-storage/src/config.rs:522`, `docs/architecture/failure-matrix.md:144` | `close_subcompactions: 8` is the **default** value in `Threads` mode, but no crash or failure sweep runs the close with parallel subcompactions; the matrix presents it as a limitation of a non-default option. |
| G3 | Minor (test gap) | `crates/kerno-storage/tests/process_kill.rs:76-85` | The real `kill -9` uses only one sequential writer: no concurrent group commit, no transactions, no `compact_range`, no `wal_preallocate`. |
| D1 | Minor (documentation) | `docs/testing/f1-report.md:20, 108`, `docs/architecture/failure-matrix.md:41-43` | Inexact provenance of the 1000-seed log, unarchived evidence, and a broken Markdown table. |

No blocker was found in WAL, MANIFEST, group commit, the `fsync` order of recovery, snapshot isolation, file lifecycle or v1 format decoding with corrupt input (see §4, "Surfaces reviewed without a finding"). B1 is the only finding that loses confirmed data through a realistic path and by default.

## 2. Method and limits

- Line-by-line reading of `wal.rs`, `format/wal.rs`, `recovery.rs`, `manifest.rs`, `format/manifest.rs`, `commit_queue.rs`, `db.rs` (commit, rotation, flush, compaction, close, open), `compaction.rs`, `flush.rs`, `sst.rs`, `deleter.rs`, `version.rs`, `rangedel.rs`, `txn.rs`, `snapshots.rs`, `format/{block,coding,header,filter,rangedel,mod}.rs`, `kerno-core/src/{vfs/std_fs.rs,batch.rs,error.rs,probe.rs}`, and of the tests `process_kill.rs`, `lazyfs.rs`, `damage.rs`, `common/batch_stream.rs`.
- The claims of `failure-matrix.md` and `f1-report.md` were checked against the code, against the log `spikes/storage/results/2026-10-29-f1/tests-seeds1000.log` and against `git diff 50583e2 474b509`.
- **No fault-injection experiment was run** (there is no `dm-flakey`/`dm-error` in this environment): the "inferential" classes below rest on reading the code and on the documented semantics of the Linux kernel, not on a reproduction. Each one indicates how to reproduce it.
- What could not be traced to a realistic path was discarded.

## 3. Findings

### B1 · Blocker · A deferred write failure of an SST is taken as durable

**Where.** `crates/kerno-storage/src/sst.rs:169-185` (`Sink::write_pending`):

```rust
let _ = self.file.start_writeback(offset, len);
let _ = self.file.wait_writeback(offset, len);   // sst.rs:182
```

`wait_writeback` is `sync_file_range(WAIT_BEFORE | WAIT_AFTER)` (`kerno-core/src/vfs/std_fs.rs:49-51`) and is active by default: `CompactionConfig::default()` sets `bytes_per_writeback: Some(1 << 20)` (`config.rs:512`), so every SST larger than 1 MiB (every flush of 8 MiB and every compaction) goes through here.

**Why it is a failure.** The comment at `sst.rs:177-179`, the documentation at `kerno-core/src/vfs/mod.rs:58-60` and row L6 of the matrix (`failure-matrix.md:124`) state that the error "says nothing about the data, which the final `fsync` reports". That is true of `start_writeback` (`SYNC_FILE_RANGE_WRITE` returns only submission errors and leaves the error recorded in the mapping), but not of `wait_writeback`: on Linux `sync_file_range` with `WAIT_*` ends in `file_fdatawait_range()`, which calls `file_check_and_advance_wb_err()`, that is, it returns the deferred write error and **advances the `f_wb_err` cursor of that open file description**. The `sync_data()` of `finish_with` (`sst.rs:270`) uses the same descriptor, so it no longer sees the error, and the pages that failed are marked clean. It is the same problem as "a second `fsync` returns `Ok`", which the project models and treats as fatal in other places (`FsyncFailure::LinuxDropPages`; "a failed `fsync` is not retried", `db.rs:1699-1700`).

**Concrete scenario.**

1. `write_buffer_bytes` of 8 MiB (default), `Durability::Sync`, `Background::Threads`. Commits are confirmed until the memtable is full.
2. The `kerno-flush` thread writes the SST: every 1 MiB it does `write_at`, `start_writeback` and `wait_writeback`. The device returns EIO for a range (bad sector, `dm-thin` out of space, network disk); the error is recorded; `wait_writeback` returns it and consumes it.
3. The result is discarded (`sst.rs:182`). `finish_with` does `sync_data()` → `Ok`. `Table::open_file` (`sst.rs:273`) "verifies" the SST by reading from the page cache, where the failed pages are still intact, so it passes SST-O1..O7.
4. `db.rs:1064` does `sync_dir`, `log_edit` appends and syncs the `FLUSH` record, and `retire_wal` (`db.rs:975-986`) deletes or recycles the WAL that held those commits.
5. After a power cut (or after evicting the cache and reopening), the blocks of the SST are not on disk: `Table::open` or the read returns `Corruption`, and the confirmed commits are no longer anywhere else. With LSM and compactions the same path affects the old data that a compaction's output rewrites.

The same holds for compaction (`compaction.rs:816-820` uses the same `TableBuilder`). With `Background::Inline` and the current tests it cannot be seen, because `SimVfs` does not have this semantics (G1).

**Minimal fix.**

1. At `sst.rs:182` propagate the error: `self.file.wait_writeback(offset, len)?`, unless the error indicates that the platform does not support the call (`ErrorKind::Unsupported`/`InvalidInput`, since `sync_file_range` can give `ENOSYS`/`EINVAL`); any other error poisons the handle through the existing path (`or_poison("flush failed" | "compaction failed")`). `start_writeback` may keep being ignored.
2. Riskless alternative: do not use `WAIT_*` and bound the tail with `bytes_per_sync` (which already propagates `sync_data()?`, `sst.rs:166`).
3. Correct the three texts (`sst.rs:177-179`, `vfs/mod.rs:58-73`, `failure-matrix.md:124`) so that they say that only `start_writeback` is advisory.

**Evidence class.** Inferential (code and Linux kernel semantics). **Reproduction:** an SST of several MiB on `dm-flakey`/`dm-dust` with a write error in a range, and check that, with the current code, `flush` finishes well and the database is left without data after `echo 3 > drop_caches`/restart. **Cause:** introduced (the deferred-write lever WP, commit `ce15048`, enabled by default).

### G1 · Major (test gap) · The simulator models backwards the failure that B1 exploits

**Where.** `crates/kerno-testkit/src/vfs/sim.rs:139-141` and `:1086-1089`: `WaitWriteback` is "an operation with no effect on any state", even with an injected error; the comments at `sim.rs:134-138` say the same of `StartWriteback`. With `FsyncFailure::LinuxDropPages` only `sync_data` discards pages (`sim.rs:1070-1076`).

**Consequence.** All the sweeps of row L6 and of FL (`failure_matrix_rows_are_covered`, `single_failure_at_every_point`, `extended_failure_suite!`) pass with an EIO in `WaitWriteback` because the simulator leaves the pending writes and the next `sync_data` makes them durable: exactly what Linux does not do. The matrix (`failure-matrix.md:124`) turns that assumption into a guarantee. LazyFS would not cover it either (it discards unsynced pages; it does not inject EIO in deferred writes).

**Minimal fix.** In `SimVfs`, an error injected in `WaitWriteback` (and in `SyncData`) with `LinuxDropPages` must permanently discard the writes it covers and mark the error as already seen; add a sweep that requires the engine to poison the handle (or not reference the SST). Add to `spikes/storage/run-lazyfs.sh` (or to a separate script) a scenario with `dm-flakey`.

**Evidence class.** Deterministic (reading of `sim.rs:1079-1089`). **Cause:** pre-existing in the simulator, exposed by the lever of `ce15048`.

### M1 · Minor · Damaged WAL header with a single frame discards confirmed commits

**Where.** `format/wal.rs:449-463` (branch `header_is_broken`) and `:380-384`: the search for a later frame requires `p < synced_len ≤ q`, but the first frame of a WAL always stores `synced_len = 0` (`wal.rs:119, 131`: `synced_len: 0` on creation) and, in `Buffered`, all the frames before the first `sync()`. The specification assumes it (`v1.md:212`: "a frame with `synced_len = 0` under a broken header belongs to a commit whose `fsync` never completed"), but that is only true without later damage: after the commit is confirmed, the header and the frame are durable at the same time.

**Scenario.** Database in `Sync`, writer dead with `kill -9` after the first commit of a new WAL (header + one frame, both synced); one bit is corrupted in the 96 bytes of the header (magic, `header_len` or CRC). Recovery treats the WAL as "no frames with a broken header" (REC-10c admits it in the most recent WAL) and opens with a lower `last_seq`, **without error**, losing the confirmed commit. With two or more frames in `Sync` it is detected (the second one has `synced_len > 0`).

**Why the tests do not catch it.** `damage_after_unclean_crash_never_returns_wrong_data` (`tests/damage.rs:170-249`) picks the byte at random over the whole file and tolerates only sequences smaller than those confirmed in the last frame (`in_last_frame`, `:209`, `:232`); the header falls before `from`, so the assertion would fail, but a single-frame WAL is rare in a session of 220 commits and the sweep does not look for it (in the 1000-seed log: "873 reported, 101 recovered (36 lost the damaged last frame)", line 366). The matrix (`failure-matrix.md:108` and §5) documents the ambiguity of the **last frame**, but not that of the header.

**Minimal fix.** (a) Document the limitation in `failure-matrix.md` §4 and in the report, and add a negative/positive directory vector with a single-frame WAL and a damaged header, so that it stays fixed; or (b) close it in the writer: when creating the WAL, sync the header before accepting the first frame (one `sync_data` per rotation), and require in WAL-R6 that a broken header followed by a valid-CRC frame is `Corruption` also with `synced_len = 0` when the database was in `Sync`.

**Evidence class.** Inferential. **Cause:** pre-existing (v1 design, WAL-R6).

### M2 · Minor · Read-only opening needs write permission on `LOCK`

**Where.** `std_fs.rs:135-142` (`OpenOptions::new().read(true).write(true).create(true)`), used by `KernoDb::open` also for `AccessMode::ReadOnly` (`db.rs:1766`). `SimVfs::lock` never fails for permissions (`sim.rs:1264-1282`), so `read_only.rs` does not see it (`assert_dir_unchanged(..., &["LOCK"])`).

**Scenario.** A backup directory or a read-only mount (ADR-0009) without a prior `LOCK`, or with `LOCK` in mode 0444: `open` returns `Error::Io(EROFS/EACCES)` although the database is readable; and with write permission it creates a new file in a directory that the caller asked to open read-only (I-18 admits it: "except creating `LOCK`").

**Minimal fix.** For read-only, open `LOCK` with `read(true)` and a shared/exclusive `flock` without `create`; if it does not exist, take the lock on the directory. Add a test with `StdVfs` on a 0555 directory.

**Evidence class.** Inferential. **Cause:** pre-existing.

### G2 · Minor (test gap) · The default close has no crash sweep

`config.rs:522` sets `close_subcompactions: 8` by default and `compaction.rs:121-125` applies it in `Background::Threads` (the default mode); the `Inline` mode, which all the deterministic crash sweeps use (`coverage.rs`, `crash::every_point`), uses `max_subcompactions = 1`. The matrix (`failure-matrix.md:144`) says that with `max_subcompactions > 1` "no sweep of its own is run; it is a known limitation", as if it were a non-default option. What covers the default close is the functional test `close_compaction_uses_close_subcompactions` (`tests/compaction.rs:574-615`), which injects no crashes, and `process_kill.rs` runs it only in the parent, which never dies during the close.

The risk is bounded by the code: the outputs are orphans until a single `COMPACTION` record, and `compaction::apply` (`compaction.rs:856-877`) returns an internal `Corruption` on overlapping outputs instead of installing anything. A defect here would poison the close, not corrupt data. **Minimal fix:** a crash sweep with `Background::Threads` and `close_subcompactions > 1` over a `SimVfs` (it is enough to order the operations per thread with the crash point by a global counter) or, at the least, rewrite the sentence of the matrix to say that it is the default configuration of the close and what covers it. **Class:** deterministic (configuration and matrix). **Cause:** introduced by `ce15048`.

### G3 · Minor (test gap) · The real `kill -9` covers a single writer

`process_kill.rs:76-85`: the child does sequential `commit` from one thread. There are no groups of several writers (G1..G3 of the matrix are only with `SimVfs`), no `begin_write`, no `compact_range`, no `wal_preallocate`, and the ranges are ~2.5% of the batches (`common/batch_stream.rs:36-39`). The report presents it as "real `kill -9` of a writer" (correct) and claims no more, but it is better not to read it as coverage of group commit on a real disk. **Minimal fix:** a second child with 4 writer threads and acknowledgements per thread (the check `s ≥ max(acked)` is already valid) and a variant with `wal_preallocate: true`. **Class:** deterministic. **Cause:** pre-existing within the scope of WP14.

### D1 · Minor (documentation) · Provenance and evidence of the report

1. `f1-report.md:20` says that the 1000-seed log was obtained "on commit `50583e2`", but the log itself runs `tests/process_kill.rs` and `tests/lazyfs.rs` (lines 564-615), which **do not exist** in `50583e2` and are added in `474b509` (`git ls-tree 50583e2 crates/kerno-storage/tests/` does not contain them). The log came from a working tree with those files uncommitted. `git diff --stat 50583e2 474b509 -- crates` confirms that only tests and one vector change, so the result is credible, but the sentence is not reproducible as written.
2. `f1-report.md:11` marks `fmt`/`clippy`/`cargo test --release` as "Verified at the close", and §6 only lists the commands: no output is archived. The same holds for the 200 `kill -9` (§3): pasted output, with no file under `spikes/storage/results/2026-10-29-f1/`.
3. §2 presents a single `kerno` row of bit flips (256/744), but the log has another one for the scenario with ranges (`shared_ranges`: 279/721, `kerno-inline` 265/735, lines 834 and 877): there is no discrepancy, but the table omits the only scenario with range fragments.
4. `failure-matrix.md:41-43`: there is a blank line between the row of `shared_ext_failure::*` and those of `process_kill.rs`/`lazyfs.rs`, so those two rows render as loose text and not as part of the table of §1.

**Minimal fix:** correct the provenance sentence (or regenerate the log on `474b509`), archive the outputs of §3 and §6, and remove the blank line. **Class:** deterministic.

## 4. Surfaces reviewed without a finding

What was tried to break and why it does not hold is noted, so that the next reviewer does not repeat the work.

- **`fsync` order of the WAL and of recovery.** Commit: `write_at` + `sync_data` before applying to the memtable and publishing `last_seq` (`db.rs:703-757`). Rotation in `Buffered`: syncs the old WAL before creating the new one (`db.rs:902-908`). Recovery: SSTs synced + `sync_dir` (`recovery.rs:470-474`), `sync_data` of the newest WAL with frames before reclaiming its sequence (`:486-488`, S-2b), new WAL created before `SESSION_OPEN` (REC-10c), deleted only afterwards (`:515-533`). No window of loss was found.
- **MANIFEST.** `append` is a `write_at` and a `sync_data`; `checkpoint` writes, syncs, renames and syncs the directory, and on any error the writer keeps its position and the caller poisons (`manifest.rs:155-212`); a broken tail forces a checkpoint (W-7). The `unwrap`s of `format/manifest.rs:387-410` are protected by the required-label mask (`:371-380`).
- **Group commit.** One frame per group, one CRC; the group limit (`config.rs:157-158`) and the batch limit (`:114`) fit in `MAX_FRAME_COMMITS`/`MAX_FRAME_PAYLOAD`, so the engine cannot write a frame that it itself rejects on reopen. Group failure: all receive `Io`, the queued ones `Poisoned` (`commit_queue.rs:213-235`). The lock order `txn_gate → queue → writer → manifest → state` is respected in all the paths read; `rotate` and `prepare_spare_if_absent` use `try_lock` on the `spare`, which is the only lock that another thread holds for a long time.
- **Snapshot isolation.** A snapshot is `Arc<Version>` + `last_seq` read under the same lock (`db.rs:593-602`); rotation only happens under the writer lock when a group starts, once the previous one has published, so no SST of a version contains sequences greater than the snapshot's (that is why `Table::get` and `covering_seq` may ignore `seq`). Compactions are installed only in new versions, and the old entries live as long as the snapshot's version references them (`Table::drop`, `sst.rs:476-483`). The snapshot registry only feeds metrics.
- **File lifecycle.** `mark_obsolete` is called after `log_edit` has returned `Ok` (`db.rs:1223-1226`); file numbers are not reused (`recovery.rs:318`, `next_file` of the CLEAN); a failed compaction or flush leaves orphans that the open deletes. `Busy` prevents compactions with overlapping input or output tables or ranges (`compaction.rs:272-278`), which is what makes it safe to drop a tombstone when "no lower level has the key".
- **v1 decoding with corrupt input.** `Reader`/`decode_varint` do no unchecked arithmetic (`coding.rs:70-170`); `parse_data_block`, `decode_rangedel`, `decode_filter`, `decode_header`, `decode_commit` and `check_frame` validate lengths before indexing and compare with `u64` the sums of fields bounded by format limits (`block.rs:165-224`, `format/wal.rs:147-282`, `filter.rs:61-84`, `header.rs:149-225`). The remaining `unwrap`s in non-test code are conversions of fixed-length slices that were already checked. No path to a panic or to wrong data was found.
- **Ranges and `max_tomb`.** The shortcut at `version.rs:37` (`largest_seq < max_tomb` ends the search) depends on the invariant "a shallow table whose range covers the key contains no sequences lower than those of the key in lower levels"; the flow of the oldest L0 first was followed, overlap by envelope in `overlapping`, and trivial moves, and it holds. It is an implicit dependency with no assertion (a `debug_assert` in `compaction::apply` and a model test with `dynamic_base_level: true`, which is not the default value, would be advisable).
- **Transactions.** The serializability gap is closed: the `drain` under `txn_gate` waits for the previous group to publish and complete (`db.rs:561-570`, `commit_queue.rs:238-243`).

## 5. What is claimed and what the tests demonstrate

| Claim | What supports it | Gap |
|---|---|---|
| "No wrong data and no rollback" with 1000 seeds (`f1-report.md` §2) | Log `tests-seeds1000.log`: 57 `test result:` lines without failures; `kerno` with `harmless: 0` in all the sweeps. True for damage **after a clean close** (there is no WAL) and for the damage sweep after a crash (`damage.rs`), whose only tolerated "rollback" is the last frame. | M1 (header of a single-frame WAL); damage to the medium during a deferred-write failure is not covered (B1). |
| "Power cut" (matrix §1, §6) | `SimVfs`, with the assumptions of its model. | G1: the model differs from Linux in `WaitWriteback`. LazyFS is still pending and would not cover it. |
| "Real `kill -9`, 200 + 200" (`f1-report.md` §3) | `process_kill.rs`: `s ≥ acked`, content equal to the model, clean `verify()`; a process death does not lose the OS cache, and the report says so. | G3; output not archived (D1). |
| All the rows of §6 of the matrix "have a test" | `coverage.rs` requires that each row receives at least one I/O point. | It only proves that the phase runs, not the concrete result of each row (the matrix itself says so, line 45); SC and the default close, G2. |

## 6. Proposed order of correction

1. **B1** (one line and three texts) and **G1** (simulator model and a sweep) in the same merge: without G1 the fix for B1 is not protected by any test.
2. M1: decide between documenting and closing it; add the vector.
3. M2, G2, G3 and D1 can go in a later merge without blocking the exit of Phase 1.

What this review **could not** check: B1 and M1 on a real device (see the reproductions), the execution of LazyFS (pending, already declared in the report), and any behaviour on platforms other than Linux/ext4.

## 7. Correction status

Findings that blocked the merge were corrected (2026-10-31):

- **B1** (`555565a`): `Sink::write_pending` propagates the error of `wait_writeback` (only `Unsupported`/`InvalidInput` are tolerated), so it poisons the handle through the `or_poison` path; `start_writeback` remains advisory. The texts of `sst.rs`, `kerno-core/src/vfs/mod.rs` and row L6 of the failure matrix were corrected.
- **G1** (`555565a`): `SimVfs` with `FsyncFailure::LinuxDropPages` permanently discards the writes covered by a failed `WaitWriteback`/`SyncData` and marks the error as seen. Sweep `write_amp_levers::a_failed_wait_for_writeback_never_loses_acknowledged_commits`: the engine must poison the handle or not reference the SST; it fails without the B1 fix.
- **M1** (`772f3fa`, option b): the writer syncs the WAL header before the first frame (`synced_len` starts at 96), and WAL-R6 treats every frame with valid CRC under a broken header as `Corruption`, even with `synced_len = 0`. Updated `docs/file-format/v1.md`, the failure matrix and ADR-0013; new vectors `dirty-wal-one-frame` (positive) and `wal-one-frame-broken-header` (negative); directory vectors with WAL regenerated.

Minor findings corrected after the merge of PR #2:

- **M2**: `StdVfs::lock` falls back, on `PermissionDenied` or `ReadOnlyFilesystem`, to opening `LOCK` read-only, or the directory if `LOCK` does not exist, so `AccessMode::ReadOnly` no longer needs write permission and does not create `LOCK`. Test `lock_works_in_a_directory_without_write_permission` (0555 directory; it does nothing as root, and was run as `nobody`). Limit: without `LOCK` the lock falls on the directory and does not exclude a writer that creates `LOCK` afterwards (`ponytail:` mark in the code).
- **G2**: **partial**. The crash sweep with `Background::Threads` was not added; the matrix sentence was rewritten to say that the default close has no sweep of its own and what covers it. It remains open as a test gap.
- **G3**: two new tests in `process_kill.rs` with 4 writer threads (disjoint keys per thread; each thread must keep `n` batches with `n` between the last acknowledged and one more), one with `Sync` + `wal_preallocate` and another with `Buffered`. They do not cover `begin_write` or `compact_range`.
- **D1**: the provenance of the 1000-seed log was corrected, the range row of §2 was added, the blank line of the matrix was removed, and the output of the `kill -9` runs was archived (`spikes/storage/results/2026-10-29-f1/process-kill-200.log`). The output of `fmt`/`clippy`/`test` of §6 is still not archived.

Pending (not blocking): G2 (crash sweep of the default close) and the archive of the §6 output.
