# Test plan

Status: in force; extended at the close of Phase 1 (2026-10-29, §7) · Related to: [SAD §17.6, §18](../architecture/SAD.md#18-quality-tests-and-definition-of-done), [invariants](../architecture/invariants.md), [failure matrix](../architecture/failure-matrix.md), [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md)

## 1. Principle

Kerno's guarantees exist only if an automated test demonstrates them. Each invariant of [`invariants.md`](../architecture/invariants.md) cites the test that verifies it; an invariant without a test is listed as pending.

## 2. M0 infrastructure

The contract, the errors and `Vfs`/`StdVfs` live in [`crates/kerno-core`](../../crates/kerno-core); the simulators, the shared suites and `LogEngine`, in [`crates/kerno-testkit`](../../crates/kerno-testkit). [`spikes/storage/spike-core`](../../spikes/storage/spike-core) only re-exports them with the paths that the spikes use:

| Piece | What it does |
|---|---|
| `Engine` / `Snapshot` | Contract common to both candidates (atomic batches, commit sequence, snapshots, exclusive opening, typed errors). |
| `Vfs` | All file access goes through this abstraction; the engines never use `std::fs` directly. |
| `SimVfs` | In-memory file system that simulates a **power cut** (unsynced writes discarded, reordered or torn at 512 B or at byte level; directory metadata as a journaled prefix) and **process death** (everything written survives). It can cause the crash just before the *n*-th mutating I/O operation, or **a single I/O error** (EIO) in that operation with the device alive. Since Phase 1, with optional models disabled by default (see the [failure matrix](../architecture/failure-matrix.md)): process death that leaves unsynced data pending (`CrashKind::ProcessKillKeepPending`), Linux semantics after a failed `fsync` (`FsyncFailure::LinuxDropPages`), per-directory durability (`DirDurability::PerDir`) and a full disk (`set_space_limit`, `fill_disk_after`); also crashes and errors filtered by operation type, path and phase (`crash_when`, `fail_when`, with the phase from `kerno_core::probe`), I/O traces, byte-for-byte directory images (`dump`, `restore`, `assert_dir_unchanged`), `fsync` latency and media damage during use (`corrupt_byte_live`). |
| `HookVfs` | Wraps another `Vfs` and calls a *hook* before and after each operation, which decides whether it proceeds, fails, panics or is delayed. |
| `process` | Relaunches the test binary as a child process with a role (`spawn_child`, `child_role`), for cross-process tests. |
| `CountingVfs` | Counts bytes, operations and `fsync` to measure amplification. |
| `engine_test_suite!` | Generates the same test battery for each engine. With `config = SuiteConfig::with_range_deletes(n)`, the same battery with `n` thousandths of the random operations converted into range deletes (also the scenario of the `every_point` sweeps); without `config` it is exactly the M0 one (the default `WorkloadCfg` consumes no additional random number). |
| `range_delete_test_suite!` | Generates the `shared_range` module for engines that implement `RangeDeleteEngine`: `[start, end)` bounds and empty ranges, order within the batch (SEM-3), reads and paged scans across tombstones of several generations, snapshot isolation, random model with ranges, persistence after close, crash and power cut, and space. Validated on `LogEngine` and `Emulated<LogEngine>` (see §2.3). |
| `Emulated<E>` | Gives range deletes to any engine: under a commit mutex, each range is converted, at its position in the batch, into point deletes of the keys that exist at that point (snapshot plus the keys of the batch itself). The rewrite never introduces the empty key, which some engines reject (fjall aborts with a panic): it deletes it only when the batch's own operations or the stored state already contain it, something impossible in an engine that rejects it. It is a test oracle, not an implementation. |
| `extended_failure_suite!` | Generates the `shared_ext_failure` module: the optional `SimVfs` models at each I/O operation of scenario A (see §2.2). |
| `LogEngine` | Trivially correct engine that validates the harness itself (if a shared test fails, the test is wrong). It implements range deletes natively (operation 3 of the log). | <a id="21-validación-del-harness-mutation-testing-manual"></a>

### 2.1 Harness validation (manual mutation testing)

Before trusting the harness, known failures were injected (in `LogEngine` and, for rows 6 and 7, in a copy of the B+ tree prototype). The rows in bold forced the harness to be strengthened:

| Mutation | Detected by |
|---|---|
| Omit `fsync` on `Sync` commits | `power_loss_sync`, `power_loss_at_every_io_point` |
| Ignore the error of a failed `fsync` and confirm the commit | `single_io_failure_at_every_point` (variant with an immediate power cut; the first version, without it, did not detect it) |
| Sync the directory only if the file "has just been created" | `single_io_failure_at_every_point` — **real error** of `LogEngine`, corrected |
| `rename` of the rewritten file without a prior `fsync` | `power_loss_sync`, `power_loss_buffered` (after reducing the key space of the workload to force rewrites of the log; the first version of the harness did not detect it) |
| Treat corruption in the middle of the log as a broken tail | `bit_flips_are_detected_or_harmless` |
| B+ tree: `close()` does not write the second meta slot | `bit_flips_are_detected_or_harmless` (seed 7) — **only after aiming half of the damage at headers/metas**; with uniform damage it survived more than 40 seeds |
| B+ tree: the commit overwrites the **current** meta slot | `power_loss_sync`, `power_loss_at_every_io_point` — **only after adding byte-level tearing**; with 512 B tearing the meta (92 B) was written atomically and the failure was invisible |
| (Log truncated after clean close) | `truncations_are_detected_or_harmless` — which is why `LogEngine` declares `detects_truncation = false` |

### 2.2 Extended failure suite (Phase 1)

`extended_failure_suite!` ([`harness/ext.rs`](../../crates/kerno-testkit/src/harness/ext.rs)) runs on `LogEngine` in `kerno-testkit/tests/reference_engine.rs`. It found three **real errors** of `LogEngine`, each corrected with a regression test (in `kerno-testkit/src/reference.rs`; each test fails without its correction):

| Real error of `LogEngine` | Detected by (`shared_ext_failure::`) | Regression |
|---|---|---|
| Recovery accepted whatever it could read: the unsynced records (`Buffered` session, crash before `fsync`) and the pages dropped by a failed `fsync` were never made durable, and a later power cut lost or corrupted confirmed state. It now rewrites the recovered prefix and syncs it before accepting commits. | `power_loss_after_kill_keep_pending_{sync,buffered}`, `single_io_failure_at_every_point_buffered`, `linux_fsync_drop_faults_{sync,buffered_restart}` | `recovery_makes_an_unsynced_tail_durable`, `recovery_rewrites_pages_dropped_by_a_failed_fsync` |
| A new database did not sync the parent of its directory: with per-directory durability, the whole database could disappear after a confirmed commit. It now syncs the directory that contains each directory it created, including `.` in a relative path (a first version synced only the immediate parent and still failed with nested paths such as `/data/db`; the next one synced all the ancestors up to `/`, skipped `.` and could not create the database under an ancestor with only traverse permission). | `per_dir_durability_power_loss_sync` | `new_database_directory_survives_power_loss_with_per_dir_durability` |
| In `Buffered`, `close` synced the clean-shutdown marker together with the records it vouches for: a power cut could keep the marker and lose an earlier record, and recovery reported corruption. It now syncs the records first. | `single_io_failure_at_every_point_buffered` | `buffered_close_makes_records_durable_before_the_clean_marker` |

The review of the suite also corrected two defects of the model itself, each with its own self-test of `SimVfs`: a failed `fsync` with Linux semantics also discarded the pending `set_len` calls, so a truncation whose `fsync` failed never reached the disk (`linux_drop_pages_keeps_length_changes`; `recovery_truncation_survives_a_failed_sync` shows the effect on `LogEngine`: a stale record reappeared after a confirmed commit), and with per-directory durability a `rename` between directories could resurrect the source name (`per_dir_cross_dir_rename_does_not_resurrect_source`). It also hardened the full-disk model: a deleted or replaced file keeps its space while it stays open, as in Linux (`space_limit_freed_by_remove`).

### 2.3 Range deletes (Phase 1, WP10a)

`range_delete_test_suite!` ([`harness/range.rs`](../../crates/kerno-testkit/src/harness/range.rs)) is first validated on `LogEngine` (native) and `Emulated<LogEngine>` in `kerno-testkit/tests/reference_engine.rs`; `engine_test_suite!` with `config = SuiteConfig::with_range_deletes(20)`, on the same two engines in `kerno-testkit/tests/reference_ranges.rs` (a separate target so that the FP fingerprint of `reference_engine` only prints the default suite). The spikes and the benchmark adapters reject ranges with `InvalidArgument` before writing anything (`engines_without_range_deletes_reject_them`). `kerno-storage` implements them from WP10b and runs the same on itself: `tests/shared_ranges.rs` (`engine_test_suite!` with `SuiteConfig::with_range_deletes(20)` and `range_delete_test_suite!`, on `KernoDb` and the `Inline` and `M0` profiles), `tests/range_delete.rs` (reading from each source, SSTs of fragments only, fragments cut at the outputs, exclusive bounds, discard at the last level, rotation at 4 096 tombstones, bloom filter and crash sweep at each point of scenario B with ranges), `tests/compact_range.rs` (the mark stays in no live file, snapshots retained, MANIFEST checkpoint, cache and crash sweep), the MVCC model `prop_mvcc_model_*` of `tests/model.rs` (the main oracle: puts, deletes, ranges, snapshots, `flush`, compactions and `compact_range`, reopens and power cuts; `PROPTEST_CASES=5000` in the gate) and `differential_range_deletes`, which adds `kerno` to the native oracle `LogEngine`.

To check that the suite detects errors, eleven mutations were injected, one at a time, and `shared_range` was run on the two reference engines; the twelfth, specific to an LSM, is modelled with a test engine (last row). All were detected:

| Mutation | Detected by (`shared_range::`) |
|---|---|
| `LogEngine`: the final end of the range included | all except `space_is_reclaimed_after_range_deletes` |
| `LogEngine`: ranges applied after the points of their batch | `order_within_batch_range_then_put`, `scan_across_tombstones_in_all_sources`, `range_survives_reopen`, random model, isolation |
| `LogEngine`: ranges applied before the points of their batch | `order_within_batch_put_then_range`, random model, isolation |
| `LogEngine`: recovery ignores the ranges of the log | `range_survives_reopen` and the other tests that reopen |
| `LogEngine`: a batch of empty ranges does not consume a sequence | `delete_range_bounds_and_degenerate`, isolation |
| `LogEngine`: never rewrites its log (does not reclaim space) | `space_is_reclaimed_after_range_deletes` (native, in cycle 6, and `Emulated`) |
| `Emulated`: the range ignores the keys written earlier in its batch | `order_within_batch_put_then_range`, random model, isolation |
| `Emulated`: the rewritten batch can end up empty (without a sequence) | `delete_range_bounds_and_degenerate`, `scan_across_tombstones_in_all_sources`, isolation |
| `Emulated`: only the first page of a range is deleted | `space_is_reclaimed_after_range_deletes` |
| `Emulated`: the final end of the range included | all except `space_is_reclaimed_after_range_deletes` |
| `WriteBatch::validate` accepts `start > end` | `delete_range_bounds_and_degenerate` (both engines) |
| Never discards the points covered only by a range (overwrites are recovered) | `space_is_reclaimed_after_range_deletes`, in cycle 6 (self-test `space_check_fails_for_an_engine_that_keeps_range_deleted_points`) |

`space_is_reclaimed_after_range_deletes` writes, in each cycle, keys that no earlier cycle wrote (all inside `["c", "d")`): no value is overwritten or deleted with a point operation, so only range deletes turn old values into garbage. A first version always refilled the same keys; each refill hid the values deleted by range behind a newer point for the same key, and an engine that recovers overwritten versions but keeps the points covered only by a range (the compaction error that WP10b must exclude) passed the test and the whole suite. `space_check_fails_for_an_engine_that_keeps_range_deleted_points` checks it with such an engine: `LogEngine` plus a file that keeps the value bytes of each point covered by a range until that key is written again; it fails in cycle 6, and the previous version passed. The test has no fixed number of cycles: a range delete takes few bytes, so an engine that reclaims nothing keeps at least all the value bytes written (random, incompressible), and the test goes on (minimum six cycles, maximum 64) until those bytes alone exceed the bound `4 × first fill + 1 MiB` in two cycles. With six fixed cycles, the native `LogEngine` that never rewrites its log passed (it grows one fill per cycle and stays under the bound); now it fails in cycle 6. `space_check_fails_for_an_engine_that_reclaims_nothing` checks it with an engine that keeps, in its directory, the keys and values of each batch forever.

The review of the package added three regressions. `Emulated<E>` converted a batch made only of empty ranges into the deletion of the empty key, and fjall aborts with a panic on an empty key; seed 2027 has no such batch, seed 1 does (commit 162). Now the rewrite never introduces the empty key (`the_rewrite_never_introduces_the_empty_key`, with an engine that aborts on it; `an_existing_empty_key_is_deleted_like_any_other` fixes that it deletes it only when it is already stored or the batch itself puts it; `emulated_engines_commit_empty_ranges_and_ranges_from_the_empty_key`, over all the engines of the differential), and `differential_range_deletes` runs the histories of seeds 2027 and 1. The rejection test of `kerno-storage` first fills the write buffer, so that a batch that passed the check at the entry of `commit` would switch memtable before reaching the WAL rejection: without that check the test fails (the disk records operations), and before it passed anyway.

On `kerno-storage` (WP03b), the extended suite runs with background threads and with the `Inline` profile (`tests/ext_failure.rs`, modules `threads` and `inline`), together with the shared suite in that profile (`tests/shared.rs`, module `inline`), the determinism of the inline mode (`tests/inline.rs`), a full disk targeted at the commit, at the flush and at each operation of the open (`tests/enospc.rs`), and the `proptest` model test with snapshots, reopens and power cuts (`tests/model.rs`; cases: `PROPTEST_CASES`, otherwise `SPIKE_SEEDS`, otherwise 256). In scenario A it found no errors. The per-directory durability model, with a nested path, did find a **real** one, the same that `LogEngine` had: a new database synced only the immediate parent of its directory, so with `/data/apps/db` on an empty disk a power cut could erase the whole database after confirmed `Sync` commits. Now a new database syncs the directory that contains each directory it created (regression `nested_new_directory::new_nested_database_directory_survives_power_loss_with_per_dir_durability` in `tests/regressions.rs`; it fails without the correction). The review of WP03b found two defects in the first correction, which synced all the ancestors up to `/`: an ancestor that can only be traversed (mode 0711) prevented creating the database forever, and the current directory of a relative path (`db`, `data/apps/db`) was never synced. Each case has its regression in the same module (the relative paths, in the previous one): `traverse_only_ancestors_do_not_prevent_creating_a_database`, `unsyncable_parent_fails_the_open`, `retried_open_syncs_the_ancestors_an_earlier_open_created`, `retried_open_after_a_partial_create_dir_all_syncs_the_ancestors` (an earlier open left `/data/apps` without `db`: the ancestors above those created are also synced, without a guarantee, in every new database), `retried_open_does_not_swallow_an_io_error_syncing_an_ancestor` (that best-effort sync stops silently only on `PermissionDenied`, `ReadOnlyFilesystem` or `Unsupported`; an `EIO` makes the open fail) and `execute_only_ancestor_on_the_real_file_system` (real file system; it is skipped if the process is not subject to permission bits, such as root, and there it runs with `setpriv --bounding-set=-dac_override,-dac_read_search`). `SimVfs` treats `.` and the empty path as the same directory (`current_dir_aliases_name_the_same_directory`). With threads, `TestHooks::compact_once` returns whether any compaction finished while it waited (`threads_compact_once_reports_a_running_compaction` in `tests/inline.rs`).

## 3. Shared tests (SAD §17.6)

| SAD §17.6 requirement | Test |
|---|---|
| The same sequence of operations produces the same logical state in each candidate | `spike-bench/tests/differential.rs` (prototypes, `LogEngine`, redb, fjall and SQLite); with range deletes, `differential_range_deletes` (histories of seeds 2027 and 1; native `LogEngine` as oracle; prototypes, redb, fjall and SQLite through `Emulated<E>`; also snapshots taken in the middle of each stretch) |
| Insert/update/delete/reopen recovers the expected state | `functional::*` (clean and dirty reopen, random model of 1200 commits, isolation, concurrency, space) |
| Detects or rejects truncated files | `corruption::truncations`, `corruption::bit_flips` |
| Failures at predefined points compared with the expected matrix | `crash::every_point` (crash before **each** I/O operation of a fixed scenario, for power cut and process death), `faults::single_failure_at_every_point` (one isolated EIO at each point) and `crash::run` (random points in long histories, even during recovery) |
| Benchmarks repeatable with a single command | [`spikes/storage/run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh) |
| At least one measurement of range, recovery and memory/disk | `range_scans`, `bulk_load`/`cauce` (`recovery_ms`), `peak_rss_mb`, `space_amp*` |

### 3.0 Adversarial review and regressions

Besides the shared suites, each candidate went through an adversarial review (five reviewers per failure class —crash protocol, concurrency, integrity against damaged files, contract semantics, performance— and two independent verifiers per finding, who had to refute it or reproduce it with a test). The confirmed defects became tests in `tests/regressions.rs` of the corresponding engine, except three low-severity ones of the LSM that were only verified with the reviewer's reproduction: batches of more than 4 GiB (the test would have to allocate more than 4 GiB), unchecked arithmetic with crafted SSTs and the double block read in `seek` (performance). The full list is in the [M0 report §2.2](m0-report.md#22-defects-found-and-how-they-were-found).

### 3.1 How to run them

```bash
cargo test -p kerno-testkit                 # harness + LogEngine
cargo test -p spike-btree                   # candidate CoW B+ tree
cargo test -p spike-lsm                     # candidate LSM
cargo test -p spike-bench --all-features --release           # differential test (with kerno)
cargo test --workspace --all-features --release              # everything (CI splits it into two parallel jobs:)
cargo test --workspace --exclude kerno-storage --all-features --release   #   test-spikes
cargo test -p kerno-storage --all-features --release                      #   test-kerno (all profiles)

# More seeds (the random suites use between 4 and 150 by default):
SPIKE_SEEDS=1000 cargo test -p spike-btree --release

# Reproduce a specific failure:
SPIKE_SEED=123 cargo test -p spike-lsm shared_crash::power_loss_sync
SPIKE_POINT=57 cargo test -p spike-lsm shared_crash::power_loss_at_every_io_point
```

## 4. Test pyramid by phase (SAD §18.1)

| Level | M0 (spike) | Phase 1 onwards |
|---|---|---|
| Unit | node/block encoding, bloom filters, manifest, WAL | + document serialization, keys, policies |
| Properties / model | `BTreeMap` model with random histories | `proptest` with case reduction; models of revisions and indexes |
| Integration | `Engine` contract | public API, CLI, documents, revisions |
| Crash testing | `SimVfs` (power and process, all I/O points; an isolated EIO at each point) | + optional `SimVfs` models at each point (`extended_failure_suite!`: Linux semantics of pages dropped after a failed `fsync`, per-directory durability, full disk, process death without losing what is pending); real `kill -9` of a writer (`process_kill.rs`, done in Phase 1); device that loses writes (LazyFS: prepared, **record pending**; `dm-log-writes`: not attempted) |
| Corruption | bit flips and truncations after clean close | + damage during use, crafted files with valid CRC, exhaustive damage of the directory vectors (done in Phase 1) |
| Fuzzing | — | `cargo fuzz` over format readers, WAL/manifest and the document parser (Phase 6) |
| Compatibility | — | test vectors per format version (done in Phase 1: 194 of structure and 29 of directory, with a regeneration guard), migrations with simulated interruption |
| Performance | `spike-bench` (ADR-0012) | Performance CI with the regression thresholds of ADR-0012 |

## 5. Continuous integration

[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) runs on every PR and on `main`: `cargo fmt --check`, `clippy -D warnings`, all the suites in `release` (with a summary of the corruption results on the job page) and a benchmark pass at `tiny` scale as a smoke test. A scheduled weekly run raises the random suites to 1000 seeds and the `kill -9` runs to 200 iterations per mode. The details of the Phase 1 jobs are in §7.5.

## 6. M0 gate

M0 closes only if all the shared tests pass for the chosen candidate with `SPIKE_SEEDS ≥ 400` in `release`, the sweep of all the I/O points passes for both failure types, and the benchmark results are published with their environment. The results of each run are recorded in the [M0 report](m0-report.md).

## 7. Phase 1: `kerno-storage` tests

What was added to the M0 plan with the product engine. The figures of the exit gates and their result are in the [Phase 1 report](f1-report.md); the design of each suite, in [phase1-design.md §6](../architecture/phase1-design.md).

### 7.1 Harness macros and where they run

Each macro generates a test module for any engine that implements the corresponding contract. They are validated **first** on `LogEngine` or `Emulated<E>` (if they fail there, the test is wrong) and then on `kerno-storage`.

| Macro (generated module) | Contract | Content | Runs on `kerno-storage` |
|---|---|---|---|
| `engine_test_suite!` (`shared_functional`, `shared_crash`, `shared_faults`, `shared_corruption`) | `Engine` | functional, crashes at random points and at each I/O point, isolated EIO, bit flips and truncations | `shared.rs` (profiles `KernoDb`, `inline`, `m0_profile`, `serial`) |
| `extended_failure_suite!` (`shared_ext_failure`) | `Engine` | the optional `SimVfs` models at each point of scenario A | `ext_failure.rs` (threads and `Inline`) |
| `range_delete_test_suite!` (`shared_range`) | `RangeDeleteEngine` | `[start, end)`, order within the batch, snapshots, space | `shared_ranges.rs` |
| `sync_test_suite!` (`shared_sync`) | `SyncEngine` | `sync()`, poisoning, prefix ≥ last `sync()` | `sync.rs` |
| `group_commit_test_suite!` (`shared_concurrency`) | `Engine` | scenario C (several writers), visibility after durability | `sync.rs` |
| `txn_test_suite!` (`shared_txn`) | `TxnEngine` | read-modify-write transactions, own writes, abort | `txn.rs` (native in `KernoDb` and `Inline`; `Emulated<LogEngine>` validates the suite) |

The suites that fix a detail of the engine (format, recovery, metrics) are not macros: `format.rs`, `recovery_rules.rs`, `clean_shutdown.rs`, `read_only.rs`, `manifest_log.rs`, `file_lifecycle.rs`, `coverage.rs`, `verify.rs`, `crafted.rs`, `damage.rs`, `process_lock.rs`, `compact_range.rs`, `range_delete.rs`, `group_commit.rs`, `snapshot_metrics.rs`, `compaction.rs`, `close_budget.rs`, `metrics.rs`, `wal_prealloc.rs`, `write_amp_levers.rs`, `enospc.rs`, `inline.rs`, `model.rs`, `engine.rs`, `regressions.rs` and the vectors (`struct_vectors.rs`, `dir_vectors.rs`, `spec_decoder/`).

### 7.2 `SimVfs` modes

All are disabled by default (the default behaviour, including the draws of the random generator, is fixed by `default_mode_crash_images_unchanged`); the [failure matrix](../architecture/failure-matrix.md) says what survives in each.

| Mode | What for |
|---|---|
| `CrashKind::{PowerLoss, ProcessKill}` | those of M0 |
| `CrashKind::ProcessKillKeepPending` | process death leaves unsynced data pending; a later power cut can still lose it (row KP) |
| `FsyncFailure::{KeepPending, LinuxDropPages}` | failed `fsync`: pending (default) or discarded forever, as in Linux (rows FL, FLb) |
| `DirDurability::{Global, PerDir}` | global `sync_dir(d)` or only under `d` |
| `set_space_limit`, `fill_disk_after` | full disk (row NS) |
| `crash_when`, `fail_when`, `start_trace` | crashes and errors filtered by type, path and phase; I/O traces (`coverage.rs`, MANIFEST sweeps) |
| `dump`, `restore`, `assert_dir_unchanged`, `corrupt_byte_live` | byte-for-byte images, check that nothing was mutated (I-18), damage during use |
| `set_sync_latency` | `sync_data` sleeps outside the global mutex, so that groups form |

### 7.3 Scenarios and sweeps

- **A** (that of M0): creation, 25 commits, close, open, 20 commits, `Drop`, open, 10 commits, close.
- **B "format":** creation → 30 commits (10% ranges) → `compact_range` → close → open → 10 transactions → `sync` → `Drop` → open → forced checkpoint → close. `coverage.rs` walks it in `Inline` mode with a trace.
- **C "group":** 4 writers × 10 commits with `set_sync_latency`.
- Each scenario, under process death, power cut, death with pending data + cut, EIO with both `fsync` models, and ENOSPC, in `Sync` and `Buffered` where it makes sense; plus filtered sweeps over the operations of `MANIFEST*` and the targeted test `chained_crash_during_session_start_after_torn_wal` (REC-10c).
- `failure_matrix_rows_are_covered` fails if a row of the matrix receives no point; `threads_mode_rows_superset_of_inline` requires the same with background threads.

### 7.4 Real processes and real device

| Test | What it does | Status |
|---|---|---|
| `process_lock.rs` | a child keeps the database open; the parent checks `Locked` on open, read-only, `verify()` and `inspect()`; the lock is released when the child dies | in CI |
| `process_kill.rs` | a child (the same test binary, `kerno_testkit::process`) writes with `StdVfs` a sequence of batches and publishes each confirmed sequence on stdout; the parent kills it with SIGKILL at a random moment, runs `verify()`, reopens and requires `s ≥` the last confirmed and content equal to the model. Each iteration continues from the state of the previous one. `KERNO_KILL_ITERS` sets the iterations (20 by default, 200 in the weekly and exit runs) | in CI |
| `lazyfs.rs` + `spikes/storage/run-lazyfs.sh` + `.github/workflows/lazyfs.yml` | scenario A on a LazyFS mount (pinned commit), with a control test that the mount loses unsynced data and `lazyfs::clear-cache` between the writer and the checker; the script returns 2 (**pending**, never green) if FUSE or the tools are missing | manual and weekly; **record pending** (see the Phase 1 report) |

The two tests of `process_kill.rs` do not run at the same time (a `fork` in one thread copies the descriptors that the other has open, among them the `LOCK` of an ongoing `verify()`, and the `flock` lock is not released until the child's `exec`: the other test's reopen saw `Locked`).

### 7.5 Exit gates and CI budget

| Gate | Command | Where |
|---|---|---|
| G0 | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features --release` | every PR (`checks`, `test-spikes`, `test-kerno`) |
| FP (harness fingerprint) | the three commands of [phase1-design.md §1](../architecture/phase1-design.md) with `SPIKE_SEEDS=400`; they must give exactly the figures of the `Outcomes` of that section | each package and the output |
| Correctness, 1000 seeds | `SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture` | weekly (`SPIKE_SEEDS` and `KERNO_KILL_ITERS` are set by the `schedule` event) and output |
| `kill -9` | `KERNO_KILL_ITERS=200 cargo test --release -p kerno-storage --test process_kill` | weekly and output |
| `loom` | `RUSTFLAGS="--cfg kerno_loom" cargo test -p kerno-storage --release --lib commit_queue` | weekly and on demand (`loom` job) |
| LazyFS | `./spikes/storage/run-lazyfs.sh` | manual and weekly (`lazyfs.yml`) |
| Benchmarks | `ENGINES=btree,lsm,kerno,redb,fjall,sqlite ./spikes/storage/run-benchmarks.sh` and `spike-bench report --baseline … --baseline-label m0` | phase output and each PR that changes performance (CONTRIBUTING) |
| Vectors | `struct_vectors_encode_exact`, `dir_vectors_are_reproducible` (normal, in CI) | each PR; a vector that fails is not regenerated to pass ([CONTRIBUTING](../../CONTRIBUTING.md)) |

Time budget (targets of [phase1-design.md §6.7](../architecture/phase1-design.md)): `test-spikes` ≤ 15 min and `test-kerno` ≤ 25 min per PR. Measured on the exit VM (4 cores), the normal `kerno-storage` suite with `--all-features --release` takes TIEMPO_SUITE; the times on GitHub runners have not been measured. If a package exceeds the budget, reduce the default seeds of its new tests, not those of the shared suite.
