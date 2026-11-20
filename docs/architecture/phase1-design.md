# Phase 1 design: persistent core

| Field | Value |
|---|---|
| Status | Design proposal for executing Phase 1 (SAD §16). The decisions that interpret accepted ADRs are recorded in ADR-0013 (WP06) |
| Date | 2026-10-28 |
| Base | M0 closed at `0cba925`; ADR-0001, ADR-0002 and ADR-0003 accepted on 2026-10-28 |
| Format | [`docs/file-format/v1.md`](../file-format/v1.md) (draft; frozen when the phase closes) |
| Related | [roadmap](../roadmap/README.md) "Phase 1: work plan", [failure matrix](failure-matrix.md), [invariants](invariants.md), [test plan](../testing/test-plan.md), [M0 report](../testing/m0-report.md), ADR-0012 |

## 0. Summary

Phase 1 turns the `spike-lsm` prototype into the first engine of the product, `kerno-storage`, with a physical format v1 that is specified and tested, and it meets conditions 1–3 of ADR-0001 and the Phase 1 follow-ups of ADR-0002. Its output is that of SAD §16: an experimental engine that demonstrates the persistence invariants and passes the updated recovery matrix.

This design synthesises three proposals assessed by two judges (format first, risk first, incremental). From each it takes the best-rated parts:

- **From the "format first" approach:** the complete v1 format (common header with `db_id`, WAL frames per group with `synced_len`, MANIFEST as an edit log with `SESSION_OPEN` and a `CLEAN` checkpoint, recovery decision table, writer invariants W-1..W-13, structure vectors before the engine, `spec_hex_matches_vectors`, rejection of foreign directories).
- **From the "risk first" approach:** the **execution order**: the latency queues of condition 1 are diagnosed and corrected *before* investing in the format, with targets recorded before measuring and a quantitative decision point; the new suites are validated first against the reference engine; an independent decoder written from the specification; CRC with file context; *flush split* with an L0 depth metric; the list of what is never cut.
- **From the incremental approach:** the "copy first" port verified with a normalised `diff`; a switch for each large change whose "off" value reproduces M0, with the shared suite running in that profile; a single compaction thread by default; the exact footprint of `Outcomes` in each package; `base_seq` in the WAL header; extensible scenario steps (`Step::Ext`).

It also covers what no proposal addressed: disk full (ENOSPC), determinism of the closing budget, protection against databases created with the draft (`DRAFT` bit), deferred deletion of obsolete files, locking between two real processes, testing with a real device as an exit criterion, bounded memory in recovery, the supported platform, the race between orphaned snapshots and `verify()`, the **re-entrancy** of recovery when the new WAL is created before its record in the MANIFEST, a concurrency checker (`loom`) for the commit pipeline, back-pressure of the commit queue, the CI time budget, the process for changing draft bytes and the coverage comparison between thread mode and the deterministic mode.

## 1. Execution principles

1. **Packages in a dependency graph and always green.** The work packages (WP, §11) run in parallel when their dependencies are already integrated; each integration leaves the branch with `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features --release` green (gate **G0**).
2. **Harness non-regression footprint (gate FP).** Every package runs:

   ```text
   SPIKE_SEEDS=400 cargo test --release -p spike-btree --test shared -- shared_corruption --nocapture 2>&1 | grep Outcomes
   SPIKE_SEEDS=400 cargo test --release -p kerno-testkit --test reference_engine -- shared_corruption --nocapture 2>&1 | grep Outcomes
   SPIKE_SEEDS=400 cargo test --release -p spike-lsm --test shared -- shared_corruption --nocapture 2>&1 | grep Outcomes
   ```

   and must obtain exactly `btree BitFlip: Outcomes { detected_at_open: 28, detected_on_read: 191, harmless: 181 }`, `btree Truncate: Outcomes { detected_at_open: 400, detected_on_read: 0, harmless: 0 }` and `log BitFlip: Outcomes { detected_at_open: 397, detected_on_read: 0, harmless: 3 }` (without threads: deterministic), and for `lsm` `Truncate` 400/0/0 and `BitFlip` with `harmless: 0` (it has threads; the 36/364 split may vary). Before WP01 the second command uses `-p spike-core`.
3. **Copy first, change one subsystem per package.** `kerno-storage` is born as a copy of `spike-lsm` with only renames. `spike-lsm` is **frozen** as the performance baseline in the same session (ADR-0012); the only edit allowed is the `match` arms that `Op::DeleteRange` requires (WP10).
4. **Switches with M0 profile.** Each large behaviour change (compaction levers, closing budget, group commit) has a switch in `Config` whose "off" value reproduces M0. The shared suite also runs in the `M0` profile so that regressions can be bisected.
5. **Tests before functions.** Each new suite is first validated against `LogEngine` (native) or `Emulated<E>` (emulation in the testkit); only then is it run against `kerno-storage`. If it fails against the reference, the test is wrong.
6. **Determinism.** `Background::Inline` runs flush and compaction inline; with the byte-only closing budget (`max_duration: None`), the I/O sequence is deterministic: the `every_point` sweeps number the same in each run and the vectors are regenerated byte for byte.
7. **The risk that can bring the phase down is measured first.** Condition 1 (latency queues) is diagnosed in WP04 and attacked in WP05, before the format; if it is not met, the owner knows at the start, not at the end.
8. **Never cut** (§10.2): the model and property tests, the crash sweeps filtered over the MANIFEST, the multi-writer crash suite, the targeted re-entrancy test, the vectors and their guard, and the FP footprint.

## 2. Crates and dependencies

```text
crates/kerno-core       (no external dependencies)
  error     Error, Result                       (+ Incompatible, ReadOnly in WP07)
  batch     Op, WriteBatch, MAX_KEY_LEN, MAX_VALUE_LEN  (+ Op::DeleteRange in WP10)
  Durability
  vfs       File, OpenMode, Vfs, dir_size, StdVfs
  probe     per-thread phase label for the test traces (WP03)
  contract  #[doc(hidden)] Engine, Snapshot, Options (+ small_for_tests);
            extensions: TestHooks (WP03), SyncEngine (WP12), TxnEngine/Txn (WP13), RangeDeleteEngine (WP10)

crates/kerno-testkit    (publish = false; kerno-core, crc32c, parking_lot)
  rng (Rng, unchanged byte for byte), reference (LogEngine), vfs::{SimVfs, CrashKind, CountingVfs,
  IoCounters, IoSnapshot, HookVfs}, harness::*, emulated (Emulated<E>), process (re-execution of the test
  binary), macros: engine_test_suite!, extended_failure_suite!, range_delete_test_suite!,
  sync_test_suite!, group_commit_test_suite!, txn_test_suite!

crates/kerno-storage    (kerno-core, crc32c, parking_lot, quick_cache, crossbeam-skiplist)
  dev: kerno-testkit, proptest; [target.'cfg(kerno_loom)'.dev-dependencies] loom
  dev (only to generate the M0 vectors): spike-lsm, spike-btree

spikes/storage/spike-core    → shim: re-exports kerno-core and kerno-testkit under their usual paths
spike-lsm, spike-btree       → spike-core (no code changes except WP10)
spike-bench                  → spike-core; [candidates] spike-lsm, spike-btree; [kerno] kerno-storage
```

Rules and CI guards:

- `kerno-testkit` never depends on `kerno-storage` (it avoids the cycle and the double compilation in unit tests).
- The product does not depend at run time on test crates or spikes:
  `tree=$(cargo tree -p kerno-storage -e normal --prefix none) && grep -q '^kerno-storage ' <<<"$tree" && ! grep -E '^(spike-|kerno-testkit|proptest|loom)' <<<"$tree"` and `test "$(cargo tree -p kerno-core -e normal --prefix none | wc -l)" -eq 1`.
- All file access goes through `Vfs` (CONTRIBUTING): `if grep -rn 'std::fs' crates/kerno-storage/src; then false; else test $? -eq 1; fi`.
- The guards fail closed: an error of `cargo tree` or of `grep` (renamed crate, missing directory) makes the guard fail. The form `! orden | grep -q …` passed in that case, because `!` also inverts the failure of the command.
- The harness contract lives in `kerno_core::contract` (`#[doc(hidden)]`): `kerno-storage` implements it directly, so the harness tests the product's own type without adapters, and `spike-bench` and the differential test add `kerno` without wrappers. The future public facade `kerno` (ADR-0010, pending) does not re-export it.
- `Op` is not marked `#[non_exhaustive]` (it would force wildcard arms in the spikes); `Error` is (outside its crate it is only used with `matches!`). `Options` is not (two places use functional update syntax).

## 3. Public API of `kerno-storage` (Phase 1, explicitly unstable)

No semver promise (C-17). The names are fixed here so that the packages match.

```rust
pub enum AccessMode { ReadWrite, ReadOnly }

pub struct Config {
    pub access: AccessMode,                    // ReadWrite
    pub create_if_missing: bool,               // true
    pub durability: Durability,                // Sync (por base; por commit: diferido, C-05)
    pub cache_bytes: usize,                    // 64 MiB
    pub write_buffer_bytes: usize,             // 8 MiB
    pub max_immutable_memtables: usize,        // 4
    pub limits: Limits,                        // key 512 B, value 1 MiB, batch 64 MiB, 1<<20 ops (≤ límites de formato)
    pub compaction: CompactionConfig,          // §5.4; CompactionConfig::m0(write_buffer) = M0
    pub close_budget: CloseBudget,             // Bounded{max_bytes: 256 MiB, max_duration: Some(200 ms)} | Unbounded (= M0)
    pub group_commit: GroupCommitConfig,       // {enabled: true, max_group_bytes: 1 MiB, max_group_commits: 1024,
                                               //  max_queued_bytes: 64 MiB}
    pub allow_unverified_platform: bool,       // false (§5.10)
    #[doc(hidden)] pub test: TestConfig,       // {background: Threads | Inline, db_id: Option<[u8; 16]>}
}
impl From<&kerno_core::contract::Options> for Config { /* durability, cache, write buffer; resto por defecto */ }

pub struct KernoDb;   // Send + Sync
impl KernoDb {
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &Config) -> Result<KernoDb>;
    pub fn is_read_only(&self) -> Option<ReadOnlyReason>;   // Requested | UnknownRoCompat { file, bits }
    pub fn commit(&self, batch: &WriteBatch) -> Result<u64>;            // lote ciego; se agrupa (WP12)
    pub fn begin_write(&self) -> Result<WriteTxn<'_>>;                  // RMW exclusivo (WP13)
    pub fn snapshot(&self) -> Result<KernoSnapshot>;
    pub fn sync(&self) -> Result<u64>;                                  // secuencia duradera (WP12)
    pub fn compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<CompactRangeReport>; // WP10
    pub fn metrics(&self) -> Metrics;                                   // tipado; stats() lo aplana
    pub fn close(self) -> Result<CloseReport>;  // {duration, flushed_bytes, compacted_bytes, budget_exhausted, l0_files, l0_depth}
    #[doc(hidden)] pub fn hooks(&self) -> &dyn TestHooks;               // wait_idle, flush, compact_once
}
impl WriteTxn<'_> {
    pub fn read_seq(&self) -> u64;
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;          // lee sus propias escrituras
    pub fn scan(&self, start: Bound<&[u8]>, end: Bound<&[u8]>, limit: usize) -> Result<Vec<(Vec<u8>, Vec<u8>)>>;
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()>;     // límites comprobados aquí
    pub fn delete(&mut self, key: &[u8]) -> Result<()>;
    pub fn delete_range(&mut self, start: &[u8], end: &[u8]) -> Result<()>;   // [start, end)
    pub fn commit(self) -> Result<u64>;    // vacía: secuencia actual, no consume ninguna
    pub fn abort(self);                    // Drop == abort
}
impl KernoSnapshot { seq(), get(), scan() }   // paginado y reanudable con Excluded(last) (C-27)

pub fn verify(vfs: Arc<dyn Vfs>, dir: &Path, opts: &VerifyOptions) -> Result<VerifyReport>; // nunca escribe
pub fn inspect(vfs: Arc<dyn Vfs>, dir: &Path) -> Result<FormatInfo>;                       // solo cabeceras
#[doc(hidden)] pub mod testing;   // perfiles Profiled<P>: Inline, M0, Serial (cada uno implementa Engine)
#[doc(hidden)] pub mod format;    // códecs v1, parches de cabecera, generador de vectores
```

**`Metrics`** (all are exported by `stats()` with stable names; the M0 names are kept):

| Group | Keys |
|---|---|
| Shape | `l{0..6}_files`, `l{0..6}_bytes`, `l0_depth`, `range_fragments`, `memtable_bytes`, `immutable_memtables`, `last_seq`, `durable_seq` |
| Writes | `commits`, `commit_groups`, `group_commits_max`, `wal_bytes`, `wal_syncs`, `memtable_rotations`, `queued_bytes_max` |
| Commit phases | `commit_{queue,make_room,wal_write,wal_sync,apply}_p99_micros` (log2 histogram) |
| Stalls | `stall_episodes_{memtable,l0}`, `stall_micros_{memtable,l0}`, `stall_micros_max`, `slowdown_commits`, `slowdown_micros`; the old `write_stalls` (now episodes), `write_stall_micros`, `write_slowdowns` |
| Background | `flushes` (including closing and recovery), `flush_micros_max`, `compactions`, `compaction_micros_max`, `compaction_bytes_{read,written}`, `l0_compaction_input_bytes_max`, `l0_compaction_l1_files_max`, `pending_compaction_bytes`, `trivial_moves`, `tombstones_dropped`, `range_tombstones_dropped` |
| Life cycle | `obsolete_bytes_pending`, `obsolete_files_deleted`, `delete_failures` |
| Closing and opening | `close_compaction_micros`, `close_budget_exhausted`, `last_open_clean`, `last_open_replayed_bytes` |
| MANIFEST | `manifest_records`, `manifest_checkpoints`, `manifest_bytes` |
| Snapshots | `snapshots_live`, `snapshot_oldest_seq`, `snapshot_oldest_age_ms`, `snapshot_retained_bytes` |
| Transactions | `txns_committed`, `txns_aborted`, `txn_gate_wait_micros` |
| Reads | bloom and cache counters (spread over 16 slots with cache-line padding to remove the M0 contention, `stats.rs:44`) |

**`VerifyReport`** (WP09): `files` (version and bits per file read), `clean`, `manifest_torn_tail`, `last_seq`, `levels` (per level files/bytes/entries/fragments), `wals` (number, frames, commits, valid header, broken tail), `orphans` (name and length), `unknown`, `manifest_tmp`, "orphans that vanished" (`vanished_orphans`, RO-4) and `issues: Vec<Issue { file, offset: Option<u64>, rule: &'static str, detail: String }>`; `is_ok()`. `VerifyOptions { deep, check_wal }` (both `true` by default). Damage is reported in `issues`; `Err` is kept for `Io`, `Locked`, `Incompatible` and `InvalidArgument` (a directory that is not a database). `inspect()` returns `FormatInfo { files: Vec<FileInfo> }`.

## 4. Error model

`kerno_core::Error` keeps the five variants of M0 (ADR-0002, ADR-0010) and adds two (C-16); it is marked `#[non_exhaustive]`. The harness helper `assert_damage_error` remains strict (`Corruption | Io | Poisoned`): a bit flip cannot produce a header with a valid CRC and an unknown version.

| Situation | Error | State afterwards |
|---|---|---|
| Batch or transaction outside limits; `start > end`; sequence exhausted (2^63−1) | `InvalidArgument` | nothing written, no sequence consumed, handle healthy |
| Directory opened by another handle or process | `Locked` | no effects (only the creation of `LOCK`) |
| WAL write or `fsync` failed, group `fsync` failed, ENOSPC, error or panic in the background or in the commit leader | `Io`, and then `Poisoned` | indeterminate result until reopening; after reopening, the commit is whole or absent; all members of the failed group receive `Io`; the queued unwritten ones, `Poisoned` |
| Damage (CRC, structure, file made with a valid CRC) | `Corruption` | never repaired automatically; on reads it does not poison; in the background it does |
| `format_version` > 1, unknown `incompat` bit, `AEAD`, M0 format, draft after freezing | `Incompatible` | checked before any write; directory identical byte for byte |
| Foreign or missing directory without `create_if_missing` | `InvalidArgument` | nothing touched |
| Write to a read-only handle | `ReadOnly` | — |
| Platform without verified guarantees and `Sync` without `allow_unverified_platform` | `InvalidArgument` | — |

The messages cite file, offset and rule ID, never key or value bytes (T-08, R-88). A panic in user code inside a transaction only releases the write slot: the engine did not touch shared state.

## 5. Engine changes

### 5.1 Port and deterministic mode (WP02, WP03)

- `kerno-storage` = copy of `spike-lsm/src` and `tests` with `spike_core` → `kerno_core`/`kerno_testkit`, `LsmEngine` → `KernoDb`, `LsmSnapshot` → `KernoSnapshot`, `name()` = `"kerno"`, threads `kerno-flush` and `kerno-compact-0`. The crate documentation says "development format; never for user data" until v1 exists.
- `Background::Inline` (WP03): no threads; `make_room` rotates and runs inline the flush of the oldest immutable memtable and the compactions that `pick` chooses until none is left; `close` and `compact_range` also inline. It respects the lock order `writer → manifest → state`.
- Profiles `testing::Profiled<P>` (`Inline`, `M0`, `Serial`): new types that implement `Engine` with another `Config`, to run the harness macros in several configurations without touching `Options`. `Engine::name()` returns `kerno-inline`, `kerno-m0`, `kerno-serial`.
- **Phase probe** (`kerno_core::probe`, WP03): `probe::phase("flush")` returns a guard that sets a per-thread label; `SimVfs::trace()` records it with each operation. Cost: one write to a `thread_local`. Labels: `create`, `session`, `commit`, `rotate`, `flush`, `compact`, `compact_range`, `close`, `checkpoint`, `delete`, `sync`.

### 5.2 Format v1 in the engine (WP06, WP07)

The whole format is in [v1.md](../file-format/v1.md). WP06 implements it as **pure code** (`crates/kerno-storage/src/format/`, without I/O, with structure vectors); WP07 makes the engine write and read only v1, still with the MANIFEST as a single snapshot rewritten by rename on each change (M0 protocol over v1 records). Points that change the engine:

- Common header in the three file types, `db_id` and file number verified (HDR-10).
- WAL by frames (one frame per commit until WP12, one per group afterwards), `synced_len` (W-11) and `base_seq`; replay in a 1 MiB stream.
- Canonical batch (WAL-C1) applied to the memtable.
- SST v1 (44-byte wrapper with AEAD slot, CRC with context, TLV properties, 80-byte footer); `TableIter` and `Table::get` expose the sequence of each entry.
- API limits in `Config::limits`, checked in `kerno-storage` (not in `WriteBatch::validate`, which the spikes continue to use). Checked arithmetic on every value read from disk; file numbers ≤ 2^48−1 (DIR-1).
- Creation protocol DIR-3 and rejection of foreign directories DIR-2. `create_dir_all` and `sync_dir(padre)` only on creation.
- `DRAFT` bit in every file written.

### 5.3 Condition 3 of ADR-0001, first half: MANIFEST as an edit log (WP08)

- `ManifestState` keeps the append handle and the current `edit_seq`. `write_manifest` becomes `append_record`: one `write_at` + `sync_data` (before: two syncs plus creation and rename). The flush costs 3 syncs (SST, directory, record) instead of 4.
- `log_number` becomes the **exact number of the oldest live WAL**: the flush of memtable `m` sets it to the WAL of the next oldest memtable (or that of the active one). M0 used `wal_number + 1`, which might not be a WAL.
- Session start S-1..S-6 (v1.md §9.4): the new WAL is created **before** its `SESSION_OPEN`, and REC-10c makes that window re-entrant. A clean reopening costs 2 syncs.
- `close()` still follows W-9: it rotates if the active memtable received commits, flushes all, runs mandatory and budgeted compaction, writes the `CLEAN` checkpoint and deletes the WAL. After a clean close the directory contains only `LOCK`, `MANIFEST` (a single record) and SST, as in M0, so that no later damage is ambiguous.
- Recovery decision table REC-1..REC-13 implemented as a pure function `recovery::classify(listado, manifest, cabeceras) -> Plan` shared by read-write opening, read-only opening and `verify()`; writes are only executed by `recovery::apply(plan)` (I-18 by construction).
- Bounded recovery memory: S-2 writes an L0 SST each time the replayed memtable reaches `write_buffer_bytes`.
- **Deferred deletion (W-13):** `Table::drop` of an obsolete table enqueues its path in a shared `Deleter`; if the engine is alive, a `kerno-delete` thread (or inline execution in `Inline`) deletes it and retries failures; if the handle has already closed, it is deleted at once (M0 behaviour). Metrics `obsolete_bytes_pending`, `delete_failures`. Reads no longer pay for deletions.

### 5.4 Condition 1 of ADR-0001: bounded latency queues (WP04, WP05)

**Diagnosis first (WP04).** Hypotheses to confirm or discard with metrics before correcting anything:

- **H1, multi-prefix flushes.** `agent_memory` writes `mem/<id>` and `emb/<id>` in each batch with ids increasing over time (`workloads.rs:598-658`); LiveSpec does the same with `cs/` and `sym/`. Each L0 SST spans from `emb/<id viejo>` to `mem/<id nuevo>`, which contains **all** the L1 SSTs with old `mem/`, and `compaction::pick` takes all of L0 plus all overlapping L1: enormous compactions, L0 grows, each commit sleeps 1 ms with L0 ≥ 8 and blocks at 12.
- **H2, debt after loading.** The initial load leaves pending work that drains during the measured phase; a single thread and L0 without priority.
- **H3, write interference.** The SSTs are written in 256 KiB chunks and a single final `sync_data`; the WAL's `fdatasync` waits behind it.
- **H4, foreground rotation.** Creating the WAL and `sync_dir` happen inside the commit.

**Levers (WP05), each with a switch** in `CompactionConfig` (the profile `CompactionConfig::m0()` turns them all off):

| Lever | Switch | Hypothesis |
|---|---|---|
| *Flush split*: when writing a memtable the output is cut between two consecutive keys whenever some L1 file lies whole between them; range fragments are trimmed at the cut | `flush_split` | H1 |
| L0 metric by **depth** (maximum number of L0 files that overlap at one key) for scoring, slowing and stopping | `l0_metric = Depth \| Files` | H1 |
| L0 priority: with metric ≥ `l0_slowdown`, L0 always wins | `l0_priority` | H2 |
| Partial compaction of L0: the **oldest suffix** of L0 up to `max_compaction_bytes` (25 × target size) including overlapping L1, minimum one file (always safe: the files left are newer) | `l0_partial` | H1, H2 |
| Cut by overlap with the grandparent level (10 × target size) | `grandparent_overlap_bytes` | H2 |
| Trivial move from L0 when the oldest file does not overlap L1 | `l0_trivial_move` | H2 |
| Rate-based slowdown: delay = batch bytes / rate (16 MiB/s initially, divided by 2 while the metric grows) instead of a fixed 1 ms | `slowdown = Rate \| Sleep(1 ms)` | H2 |
| `sync_data` every 1 MiB written of SST | `bytes_per_sync` | H3 |
| Heap merge with more than 8 children | `heap_merge_threshold` | reads |
| Incremental flush of the SSTs: `File::start_writeback` (`sync_file_range`) after each chunk and waiting for that range, instead of the intermediate `sync_data`; no effect on durability (`SimVfs`: operation without effect) | `bytes_per_writeback` (default 1 MiB; `bytes_per_sync` becomes `None`) | H3 (WP05 follow-up, [§9.2 of the diagnosis](../testing/f1-queue-diagnosis.md#92-lever-5-incremental-flush-of-the-ssts-with-sync_file_range)) |
| L0 is compacted into L2 while L1 is empty and L2 is small (the clean close does not skip L1) | `dynamic_base_level` (optional, off by default) | background writing (WP05 follow-up, [§9.3](../testing/f1-queue-diagnosis.md#93-lever-4-dynamic-levels-and-byte-priority)) |
| Choice of the file to compact by overlap with the level below | `file_picker = MinOverlap` (off by default; files with range-deletion fragments go first) | background writing (same) |
| Conditional: subcompactions (split at L1 boundaries, `std::thread::scope`, a single edit) and 2 threads with `being_compacted` exclusion | `max_subcompactions`, `max_background_compactions` (1 by default) | if WP05 requires it |

**Closing budget (C-10).** `compact_for_close`: first, mandatory, compact until the L0 metric falls below `l0_trigger` (it keeps the intent of the M0 regression `short_sessions`); then, while byte and time budget remains, one more bounded compaction (the excess is bounded by `max_compaction_bytes`). `max_duration: None` makes the close deterministic (tests, sweeps and vectors). What is pending is picked up by scoring after the next opening; the format does not record "debt".

**Lock metrics.** An episode is counted once when entering the wait (M0 counted wake-ups), by reason, with its duration and the maximum; `spike-bench` records `stat_*` from `Engine::stats()` at the end of each load.

**Decision point (WP05).** With the targets of §7 recorded in WP04 before measuring:

- T1 met ⇒ **GO**.
- T1 not met by ≤ 25 % ⇒ GO, with the deviation documented.
- T1 not met by > 25 % ⇒ subcompactions and the second thread are implemented inside WP05, and it is measured again; if it is still not met, `docs/testing/f1-decision-cond1.md` is written (candidate for NO-GO, with the next levers and the contingency R-10/R-11) and the owner is told. Sequential execution continues unless the owner stops it: the v1 format does not depend on this decision.

### 5.5 Condition 2 of ADR-0001: range tombstones and targeted compaction (WP10)

- **Semantics** (v1.md §11): `delete_range(start, end)` deletes `[start, end)`; `start > end` ⇒ `InvalidArgument`; `start == end` has no effect. Canonical batch (WAL-C1). **Visibility by sequences only** (SEM-4): a point `p@s` is hidden in snapshot `S` if some fragment covering it has `s < u ≤ S`; with the same sequence the point wins.
- **Memtable:** list of ranges `(start, end, seq)` append-only behind an `RwLock`; forced rotation when a memtable reaches 4 096 range tombstones (bounds memory and read cost).
- **SST:** block of ranges fragmented (one sequence per fragment, the largest), resident like the index and the filter. The file bounds include the extent of the fragments (`largest_exclusive`).
- **`get(k)`:** walks the sources from newest to oldest keeping `max_tomb` (maximum sequence `≤ S` of the fragments covering `k`, checked in each source **before** the bloom filter); the first point found with `s ≥ max_tomb` is returned (`None` if it is a point tombstone); if `s < max_tomb`, `None`; early exit as soon as `largest_seq` of the next source < `max_tomb`. (The incremental proposal returned the point before looking at the ranges of the same memtable: incorrect.)
- **`scan`:** `MergeIter` produces the newest version of each key with its sequence; a fragment aggregator per source, with cursors that only advance, hides the key if the maximum coverage sequence exceeds that of the point. With no fragments in any source no cursor is created (the M0 cost).
- **Flush and compaction:** they discard points covered by an input fragment with a higher sequence; they merge and trim fragments at each output cut (size, grandparent, *flush split*); they discard a fragment when no file deeper than the output overlaps it (`DeeperLevels::range_may_overlap`; always in L6). SSTs with ranges only are allowed.
- **`compact_range(start, end)`:** (1) if the active memtable received commits, it rotates and waits for the immutables to empty (the data leaves the WAL, which is deleted after its `FLUSH`); (2) sends a request to the compactor (in `Inline`, inline) that compacts, level by level, every file overlapping the range until no deeper level overlaps it, so that the tombstones disappear; (3) writes a checkpoint so that the keys of the range leave the edit history; (4) purges from the cache the blocks of the deleted tables; (5) returns `CompactRangeReport { files_rewritten, bytes_read, bytes_written, pinned_files, pinned_bytes }`, where what is retained are input tables still referenced by old versions (live snapshots).
- **Mechanism only** (C-11, R-52, ADR-0009 pending): no retention policy and no guarantee of physical deletion. What can survive is documented in `DESIGN.md`: files retained by snapshots, blocks freed from deleted files, cache memory.

### 5.6 Condition 3 of ADR-0001, second half: group commit; ADR-0002: `sync()` (WP12)

- **Write slot** `txn_gate: Mutex<()>` (D-03). A blind `commit` takes it only to enqueue; a `WriteTxn` holds it for its whole life.
- **Queue** `Mutex<CommitQueue>` + `Condvar` per request. The head request is the **leader**: (1) takes `writer`; (2) forms a FIFO group up to `max_group_bytes`/`max_group_commits`; (3) runs `make_room` once (a group never crosses two WALs); (4) assigns consecutive sequences and encodes **one frame** with the current `synced_len`; (5) one `write_at`; (6) `sync_data` if the base is `Sync` or there is a `sync()` request in the group; (7) applies the batches to the memtable in order; (8) publishes `last_seq` and `durable_seq` **once**, after the `fsync` in `Sync` (W-12, I-16); (9) completes all of them and yields leadership. A single group in flight.
- **Failures:** failed write or `fsync` poison; the group members receive `Io` (an `io::Error` is built per member: it is not `Clone`), the queued ones `Poisoned`. The leader's body runs inside `catch_unwind`: a panic poisons and wakes everyone with an error (they never hang).
- **Back-pressure:** if the queued bytes exceed `max_queued_bytes`, `commit` waits before enqueuing.
- **Fast path:** with the queue empty, the leader waits for nobody (no regression for a single writer).
- **`sync()`:** enqueues a request without a batch; the leader synchronises only if `synced_len < bytes escritos`; it returns `durable_seq`. In `Buffered`, with Linux semantics after a failed `fsync`, `Buffered` commits since the last successful `sync()` may be lost: the recovered state is still a prefix ≥ the last successful `sync()` (row FL).
- **`GroupCommitConfig { enabled: false }`** (profile `Serial`) reproduces the one-frame-per-commit path.
- **Concurrency check:** the queue protocol is a state machine without I/O (`commit_queue.rs`) written over a `crate::sync` facade that uses `parking_lot`/`std` normally and `loom` with `--cfg kerno_loom`. `loom` models with ≤ 3 threads: leader handover, propagation of poisoning, `begin_write` drain wait, back-pressure. In addition, a stress test with `HookVfs` that injects random delays and `yield`s (10^4 iterations) and `parking_lot` deadlock detection in a debug test.

### 5.7 Follow-ups of ADR-0002: RMW transactions and snapshot metrics (WP13)

- **Pessimistic (C-03, option b).** `begin_write`: takes `txn_gate`, waits until `published_seq == assigned_seq` (pipeline drain; in `Sync` it may cost an `fsync`), takes a registered snapshot. Overlay: `BTreeMap<Vec<u8>, Option<Vec<u8>>>` + list of ranges + ordered log of operations. `get`: point in the overlay, then if no transaction ranges, then the snapshot. `scan`: merges pages of the snapshot (filtered by the transaction's own ranges, re-paging until `limit` is filled) with the overlay. `commit`: canonicalises, validates bounds, enqueues with the slot taken, releases the slot and waits. An empty transaction returns the current sequence; `abort`/`Drop` do not consume a sequence. The overlay is bounded by `Limits` (`put` returns `InvalidArgument` when the maximum batch is exceeded).
- Serializable by construction: nobody else writes between the transaction's reads and its commit (D-03). Blind commits from other threads wait; transactions are not chained to each other (documented; `txn_gate_wait_micros` exposes it).
- **Snapshot registry:** `Arc<SnapshotRegistry>` shared by `Inner` and each snapshot (it outlives the handle); `BTreeMap<(seq, id), Instant>`; `Drop` deregisters. `snapshot_retained_bytes` = bytes of obsolete tables still alive (rises in `mark_obsolete`, falls in `Table::drop`) + bytes of retired memtables still retained (rises on uninstall, falls in `Memtable::drop`) (C-30).

### 5.8 Read-only, `verify()` and `inspect()` (WP08, WP09)

- **Read-only** (WP08): by `AccessMode::ReadOnly` or by an unknown `ro_compat` bit. It takes the exclusive `LOCK`, runs `classify` and replays the WALs in memory; no threads, no `create_dir_all`, `sync_dir`, deletions or checkpoints. `commit`, `begin_write`, `sync`, `compact_range` ⇒ `ReadOnly`; `close` writes nothing.
- **`verify()`** (WP09, library; the CLI arrives in Phase 2, C-01): takes the `LOCK`, reuses `classify` with a null sink, opens each table with `fill_cache = false`, walks **all** the blocks (SST-R*, SST-V*), checks MAN-S4, dry-replays each live WAL, checks slots and bits, and reports orphans and unknowns without deleting them (RO-4 for those that disappear during verification). Streamed, without whole-file buffers.
- **`inspect()`**: headers of all files (version, bits, `db_id`, number, size); takes the `LOCK`.

### 5.9 Locking and lock order

- `txn_gate → queue → writer → manifest → state`; the `Deleter` queue and the snapshot registry are leaves. It is documented in `DESIGN.md` and checked with an order `debug_assert` in test builds.
- Exclusive `LOCK` for every opening mode, `verify()` and `inspect()` (`Vfs::lock` only offers exclusive; consistent with SAD §4.2).

### 5.10 Platform (C-28)

The guarantees of `Sync` are verified only on **Linux** (ext4/xfs). On another platform, `KernoDb::open` with `Durability::Sync` returns `InvalidArgument("durability not verified on this platform")` unless `allow_unverified_platform = true`; macOS (`F_FULLFSYNC`) and Windows remain unverified. CI runs only Linux; a test `platform_gate_matches_target` fixes the behaviour by `cfg!(target_os = "linux")`.

## 6. Verification plan

### 6.1 Testkit extensions (WP03, optional: the default behaviour does not change nor consume more random numbers)

| Extension | What it adds |
|---|---|
| `CrashKind::ProcessKillKeepPending` | the image keeps `durable` and `pending`; a later power cut over it loses what was not synchronised (M0 marked everything as durable, `sim.rs:204-205, 241`) |
| `FsyncFailure::{KeepPending, LinuxDropPages}` | with `LinuxDropPages`, a failed `sync_data` discards for ever the pending writes of that inode (the visible data does not change) and the next `sync_data` returns `Ok` (R-66) |
| `DirDurability::{Global, PerDir}` | with `PerDir`, `sync_dir(d)` only makes durable the operations under `d`, and a new directory is durable only after syncing its parent |
| `set_space_limit(Some(bytes))` | writes and `set_len` that would grow the total fail with `ErrorKind::StorageFull`, without effect; `remove_file` frees space (ENOSPC) |
| `crash_when(pred, nth, kind)`, `fail_when(pred, nth, then)` | triggers filtered by `OpInfo { kind, path, offset, len, phase }` |
| `start_trace()`/`trace()` | `(n, kind, path, offset, len, phase, thread)` per mutating operation |
| `dump()`/`restore()`/`assert_dir_unchanged(&a, &b, ignore)` | full image in bytes (including unlinked inodes still reachable) and byte-for-byte comparison |
| `set_sync_latency(d)` | `sync_data` sleeps `d` **outside** the global mutex (so that groups form) |
| `corrupt_byte_live(path, off, mask)` | damage with pending writes, during use |
| `HookVfs` | generic wrapper with hooks before/after each operation (errors, panics, delays); replaces the four hand-written wrappers |
| `crash::{Step<E>, scenario, run_scenario}` public, `Step::Ext(nombre, Arc<dyn Fn(&E) -> Result<()>>)` | per-package scenarios without new type constraints in the shared macro |
| `extended_failure_suite!(E)` (module `shared_ext_failure`) | death sweep with pending writes + later cut (`Sync`, `Buffered`); EIO sweep with `LinuxDropPages` (`Sync`; `Buffered` with restart); EIO sweep in `Buffered` (missing in M0); crash sweep with `PerDir`; ENOSPC sweep |
| `process` | re-runs the test binary as a child process (`std::env::current_exe()`, `--exact <prueba>`, variable `KERNO_CHILD`) for locking between processes and `kill -9` |

### 6.2 Suites against `kerno-storage`

| Target (`crates/kerno-storage/tests/…`) | Content | WP |
|---|---|---|
| `shared.rs` | `engine_test_suite!(KernoDb, buffered_power_loss_prefix = true)`; modules `inline` (profile `Inline`) and `m0_profile` (profile `M0`, from WP05); `serial` (from WP12) | 02, 03, 05, 12 |
| `ext_failure.rs` | `extended_failure_suite!` with `KernoDb` and with the `Inline` profile | 03 |
| `enospc.rs`, `model.rs` | full disk; `proptest` model (put, delete, snapshot, reopening, cut; extended with ranges, `compact_range` and transactions) | 03, 05, 10, 13 |
| `metrics.rs`, `compaction.rs`, `close_budget.rs` | metrics and the levers of condition 1 | 04, 05 |
| `format.rs` | headers, bits, versions, M0, foreign directories, limits, platform | 07 |
| `struct_vectors.rs`, `spec_decoder/` | structure vectors, independent decoder, hex of the specification | 06 |
| `recovery_rules.rs`, `clean_shutdown.rs`, `read_only.rs`, `manifest_log.rs`, `file_lifecycle.rs`, `coverage.rs` | edit log, decision table, read-only, deferred deletion, coverage of the matrix | 08 |
| `verify.rs`, `crafted.rs`, `damage.rs`, `process_lock.rs` | integrity, crafted files, damage during use and after crashes, locking between processes | 09 |
| `shared_ranges.rs`, `range_delete.rs`, `compact_range.rs` | range tombstones and targeted compaction | 10 |
| `dir_vectors.rs` | directory vectors, guard, exhaustive damage | 11 |
| `group_commit.rs`, `sync.rs` (+ `loom` in `src/commit_queue.rs`) | groups, `sync()`, concurrency | 12 |
| `txn.rs`, `snapshot_metrics.rs` | RMW and snapshot metrics | 13 |
| `engine.rs`, `regressions.rs` | M0 tests carried over; those coupled to the format are rewritten (`wal_damage` with `format` helpers, `format_version` byte by byte expecting `Incompatible`, `short_sessions` with the L0 metric) | 02, 05, 07 |
| `process_kill.rs` | real `kill -9` of a writer with `StdVfs` | 14 |

`LogEngine` runs all the new macros (`kerno-testkit/tests/reference_engine.rs`) with native `sync()` and `DeleteRange`; the transactions run with `Emulated<LogEngine>`. The spikes only run `engine_test_suite!` and their regressions.

### 6.3 Coverage of the failure matrix

`coverage.rs` (WP08, extended in each package) runs scenario B in `Inline` mode with a trace and assigns each point to a row by `(phase, file type, operation)`:

| Row | Protocol step | Phase / file / operation |
|---|---|---|
| L1–L4 | commit | `commit` / WAL / `write_at`, `sync_data` |
| L5′ | rotation (WAL header not yet durable) | `rotate` / WAL / create, `write_at`; dir / `sync_dir` |
| L6, L7′, L8 | flush: SST, `FLUSH` record, WAL deletion | `flush` / SST; `flush` / MANIFEST; `delete` / WAL |
| L9, L10′, L11 | compaction: outputs, `COMPACTION` record, deferred deletion | `compact` / SST; `compact` / MANIFEST; `delete` / SST |
| L12′ | session start S-2..S-5 | `session` / * |
| L13′ | W-9 close | `close` / * |
| CK | checkpoint | `checkpoint` / `MANIFEST.tmp`, rename |
| CR | DIR-3 creation | `create` / * |
| R0 | reopening after `CLEAN` (`SESSION_OPEN`) | `session` / MANIFEST |
| G1–G3, S1–S2 | group, `sync()` | `commit`, `sync` (scenario C, several writers) |
| RT, DC, SC | ranges, `compact_range`, subcompactions | `compact_range` / *; `compact` (if built) |
| RO | read-only and `verify()`: **zero** mutating operations | — |
| FL, NS, KP | Linux failed `fsync`, ENOSPC, death + cut | `SimVfs` modes |

`failure_matrix_rows_are_covered` fails if a row receives no point. `threads_mode_rows_superset_of_inline` runs the same scenario with threads and checks that it covers the same rows (the errors that only exist with threads are covered with `crash::run` and the shared suite in `Threads` mode, which remains the default profile).

### 6.4 New `every_point` scenarios

- **A** (the M0 one): creation, 25 commits, close, opening, 20 commits, `Drop`, opening, 10 commits, close.
- **B "format":** creation → 30 commits (10 % ranges) → `compact_range` → close → opening → 10 transactions → `sync` (`Buffered`) → `Drop` → opening → forced checkpoint → close. It is extended in each package with `Step::Ext`.
- **C "group":** 4 writers × 10 commits with `set_sync_latency`.
- Each scenario under `PowerLoss`, `ProcessKill`, `ProcessKillKeepPending` (`Sync` and `Buffered`) and the EIO sweeps in `Sync` and `Buffered` with both `fsync` models, plus ENOSPC. In addition, sweeps filtered over the operations of `MANIFEST*` (`crash_when`) and the targeted test `chained_crash_during_session_start_after_torn_wal` (REC-10c).

### 6.5 Differential test (`spike-bench/tests/differential.rs`)

It adds `kerno` (checked sequence), 8 seeds in CI (64 in the weekly run), observations also in old snapshots, a `Buffered` path, and histories with `DeleteRange` (WP10) and RMW (WP13): native `LogEngine` as oracle, spike-btree, spike-lsm, redb, fjall and SQLite through `Emulated<E>` (range = scan + point deletes under the commit mutex; RMW = snapshot reads + batch).

### 6.6 Real processes and real device

- `process_lock.rs` (WP09): a child opens the database with `StdVfs` and waits; the parent checks `Locked` on opening, read-only, `verify()` and `inspect()`; after killing the child, the opening works.
- `process_kill.rs` (WP14): the child writes with `StdVfs` and publishes through a pipe each confirmed sequence; the parent kills it with `Child::kill` (SIGKILL) at random moments, reopens and requires `s ≥ confirmed` and the model; 200 iterations in `Sync` and in `Buffered`.
- **LazyFS** (WP14): `spikes/storage/run-lazyfs.sh` and `.github/workflows/lazyfs.yml` (manual and weekly). It runs scenario A on a LazyFS mount with loss of unsynchronised pages. **Its record is an exit criterion** (R-84); if the environment does not allow FUSE, the output stays pending that record, and it is not treated as met.

### 6.7 CI and time budget

- PR jobs in parallel: `checks` (fmt, clippy, guards), `test-spikes` (spikes, testkit and differential; target ≤ 15 min), `test-kerno` (`kerno-storage` with all profiles; target ≤ 25 min), `bench-smoke` (`tiny`, includes `kerno`).
- The additional profiles (`inline`, `m0_profile`, `serial`, ranges) use the default seeds in PR.
- Weekly: `SPIKE_SEEDS=1000` across the whole workspace, split into a matrix per test target so as not to exceed the 6 h limit per job; `loom` (`RUSTFLAGS="--cfg kerno_loom" cargo test -p kerno-storage --release --lib commit_queue`); LazyFS.
- If a package exceeds the budget, it reduces the default seeds of its new tests, not those of the shared suite.

## 7. Acceptance criteria

### 7.1 Correctness gate (ADR-0012 threshold 1, C-25), blocking
<a id="71-correctness-gate-adr-0012-threshold-1-c-25-blocking"></a>

1. Zero failures with `SPIKE_SEEDS=1000` in all the suites of `kerno-storage` and in all its profiles.
2. Zero failures in all the `every_point` sweeps (scenarios A, B, C) for process death, power cut and death + cut, in `Sync` and `Buffered` where it makes sense; in the EIO sweeps (`Sync`, `Buffered` × both `fsync` models) and ENOSPC; and in the sweeps filtered over the MANIFEST.
3. Corruption of `kerno` at 1000 seeds: **0 wrong data, 0 rollbacks, 0 harmless** in `BitFlip`; 100 % of truncations detected on opening. Clean vectors: exhaustive damage with 0 harmless.
4. `kill -9` (200 iterations × 2 modes) and the LazyFS record.
5. All the rows of the matrix covered (`failure_matrix_rows_are_covered`).

### 7.2 Performance targets (recorded in `docs/testing/f1-performance-goals.md` in WP04, **before** measuring; approved by the owner, C-06)

Scale `medium` (200 000 records), `Sync`, median of 3 interleaved repetitions, same session and same machine with `ENGINES=btree,lsm,kerno,redb,fjall,sqlite` (ADR-0012). M0 values from 2026-10-27 (VM, 4 cores, ext4, `fdatasync` 4 KiB p50 113 / p99 568 µs).

| Id | Metric | M0 | Target |
|---|---|---|---|
| T1 | `agent_memory` p99 of memory writes | LSM 5 837 µs; B+ tree 1 573; fjall 1 090 (3.71× / 5.36×) | ≤ 2.0× fjall **and** ≤ 1.5× spike-btree |
| T2 | p99 of commit: LiveSpec / Cauce / `append_seq` insert / `append_seq` update / `history_sim` | 4 764 / 713 / 4 139 / 6 840 / 9 118 µs | ≤ 1.2× spike-lsm; plus `append_seq` update ≤ 2× fjall (M0 2.60×); failures documented |
| T3 | p99 of reads: LiveSpec module-scan / symbol-get, Cauce, recall, recent-scan | 657 / 43.7 / 98.0 / 100 / 234 µs | ≤ 1.2× spike-lsm; LiveSpec module-scan ≤ 1.5× spike-btree (M0 1.49×), otherwise a review of condition 5 |
| T4 | Clean close after `bulk_load` | 759 ms (`medium`), 3 587 ms (`large`); fjall 96 / 919 | ≤ 250 ms and ≤ 1 000 ms with the default budget |
| T5 | Throughput of each load | e.g. 4 044 commits/s, load 162 kops/s, reads 230 kops/s | ≥ 0.9× spike-lsm |
| T6 | `fsync` per commit | 1.00 | 1 writer = 1.00; `concurrent_commits` with 8 writers ≤ 0.5 and ≥ 2× the throughput of 1 writer |
| T7 | Write amplification `small_commits` / `bulk_load` | 1.09× / 3.4× | ≤ 1.20× (the frame header adds 16 B per commit) / ≤ 3.6× |
| T8 | Clean reopening / dirty recovery | 3.4 ms / 9.73 ms | ≤ 1.0× / ≤ 1.2× spike-lsm |
| T9 | RMW cost (`rmw_counters`) | — | reported (R-S6) |
| T10 | Performance against fjall | 0.8–1.3× | reported; 2× misses documented (C-07) |

Every PR that changes performance attaches the comparison with the baseline on the same machine (CONTRIBUTING) until a dedicated performance CI exists (C-32). To compare with M0, `spike-bench report --baseline spikes/storage/results/2026-10-27/results.jsonl --baseline-label m0` labels those rows as `lsm@m0`, etc., and never mixes them with the new ones.

## 8. New invariants and failure matrix

Proposed for `invariants.md` (WP08–WP13, promoted in WP14):

| Id | Invariant | Tests |
|---|---|---|
| I-16 | Visibility after durability: in `Sync`, no reader observes data that a crash could lose | `no_reader_sees_undurable_data`, `sync_mode_readers_never_see_undurable_seq` |
| I-17 | After `sync()` → `Ok(s)`, every commit ≤ `s` survives a cut; a failure poisons | `shared_sync::*` |
| I-18 | Read-only opening, `verify()`, `inspect()` and every rejected opening do not mutate the directory (except creating `LOCK`) | `read_only_open_writes_nothing`, `verify_is_read_only`, `*_rejected_untouched` |
| I-19 | Range semantics: order within the batch, visibility by sequences (SEM-3, SEM-4) | `shared_range::*`, `prop_mvcc_model_*` |

Changes to the existing ones: I-01 (atomicity of each group member), I-03 (the prefix is mandatory for `kerno`, C-24), I-04 (overflow: error, never wrap to 0, C-31), I-09 (`CLEAN` checkpoint), I-11 (REC-10c), I-12 (closing budget across short sessions and deferred deletion), I-15 (from "partial" to verified).

The failure matrix gains a section "LSM v1 (`kerno-storage`)" with the rows of §6.3 (L1–L4, L5′, L6, L7′, L8, L9, L10′, L11, L12′, L13′, CK, CR, R0, G1–G3, S1–S2, RT, DC, SC, RO, FL, NS, KP) and the new `fsync` budget: `Sync` commit = 1 (< 1 with concurrency), rotation = 1 `sync_dir` (+1 `sync_data` of the old WAL in `Buffered`), flush = 3, compaction = outputs + directory + 1, checkpoint = 2 + rename, clean reopening = 2. The M0 section stays as history.

## 9. Out of scope for Phase 1

| Element | Phase |
|---|---|
| CLI (`kerno verify`, `kerno export --salvage`) | P2 |
| Stable typed API, semver, `kerno` facade | P2 / ADR-0010 |
| Documents, primary index, filters | P2 |
| Revisions, ChangeSets, history, policies | P3 (ADR-0004) |
| Public identifiers | P4 (ADR-0005) |
| Encryption and key management (v1 only reserves the slot) | P4 (ADR-0006) |
| Logical export, suppression with guarantees, copies, migrations (v1 reserves record type 5 and bits) | P4 (ADR-0003 §5, ADR-0009) |
| Secondary indexes, text, vectors | P5 |
| Fuzzing with `cargo fuzz`, `cargo audit`/`cargo deny`, beta benchmarks | P6 |
| Durability per commit (`commit_with`) | deferred; the format already admits it (C-05) |
| Compression, prefix compression, partitioned index or filter, dynamic base level | not required; the format admits them via `incompat` bits |
| WAL recycling and preallocation | added after WP05 as a lever of condition 1 (`Config::wal_preallocate`, [results](../testing/f1-queue-diagnosis.md#8-wal-preallocation-and-recycling)); no format change: they use the zero fill of v1.md §6.1 |
| Multi-process writing, server, replication, SQL | not goals (SAD §1.4) |
| Guarantees on macOS and Windows | unverified (§5.10) |

## 10. Risks

### 10.1 Register

| # | Risk | Early signal | Mitigation |
|---|---|---|---|
| K1 | Condition 1 is not met | WP04 does not attribute the p99 to correctable mechanisms; T1 fails in WP05 | Diagnosis with metrics per phase; levers with switches; quantitative decision rule; memo to the owner |
| K2 | Range tombstones return wrong data | counterexample from the MVCC model | Visibility by sequences only; `proptest` model as main oracle; suite validated first in `LogEngine`; differential; sweeps |
| K3 | Group commit breaks the broken-tail rule or poisoning | sweep with several writers | Frame = group with `synced_len`; one group in flight; `catch_unwind`; `loom` |
| K4 | Edit-log recovery goes back or loses commits | harmless or rollback results in the corruption suite | Required-WAL rule, `CLEAN` in the checkpoint, W-8, filtered sweeps, REC-10c with a targeted test |
| K5 | Drift between specification and code; unstable vectors | regeneration guard | Structure vectors before the engine; `spec_decoder`; `spec_hex_matches_vectors`; process of §15 of v1.md |
| K6 | RMW: lost updates, deadlocks | concurrent counters test | Pessimistic with drain; lock order checked; `loom` |
| K7 | Extracting the harness loses coverage | the FP footprints change | WP01 pure move; FP in all packages |
| K8 | Sweeps unstable because of threads | non-reproducible seeds | `Inline`, byte-only budget, `threads_mode_rows_superset_of_inline` |
| K9 | Benchmarks not comparable | dispersion > 15 % | Same session always; targets in ratios; target machine named before WP14 |
| K10 | Scope larger than the calendar (C-33) | speed of the packages | Cut list (§10.2) |
| K11 | New dependencies not available without network (`proptest` is in the local cache; `loom` is not, and Cargo resolves it in `Cargo.lock` even though it only compiles with `--cfg kerno_loom`) | `cargo fetch` failure when adding it | If `loom` cannot be resolved, WP12 replaces it with our own exhaustive interleaving explorer for the queue state machine (≤ 3 threads, explicit permutations of steps) and documents it; licences are justified in the PR (R-21) |

### 10.2 Cuts, and what is never cut

Cut, in this order: subcompactions and second thread (already conditional), pre-created WAL (only if H4 is confirmed), LazyFS automation (the manual run with output is kept), `inspect()` as a separate function, polish of `report --baseline`. **Never:** model tests, the MANIFEST-filtered sweeps, the multi-writer crash suite, the REC-10c re-entrancy test, the vectors and their guard, the FP footprint.

## 11. Work packages

The packages form a **dependency graph**, not a chain. Each one is implemented in its own *worktree*, starting from the branch with all its dependencies already integrated; it goes through a review from two approaches (durability and format; contract and tests) and its corrections, and is integrated into the branch with a single commit (*squash*), one at a time, after passing G0 in full and FP (§1). When two parallel packages touch the same file, their changes remain additive and local, and the new methods of the contract's traits carry a default body so as not to break those who implement them in parallel.

To parallelise, three packages are split into a **contract and oracle** half (`kerno-core`, `kerno-testkit`, `LogEngine`, `Emulated<E>`, spikes, bench), which is validated first against the reference (principle 5), and an **engine** half (`kerno-storage`): WP10a/WP10b, WP12a/WP12b and WP13a/WP13b. WP03 is split the same way (WP03a, testkit; WP03b, engine). `Config` and the `testing` module (`Profile`, `Profiled<P>`) are created in WP02 so that WP03b, WP04 and WP05 extend them without clashing. The directory vectors of group commit move from WP12 to WP11.

| WP | Title | Depends on | Level | Result |
|---|---|---|---|---|
| WP01 | Extract `kerno-core` and `kerno-testkit`; `spike-core` as shim | — | 1 | pure move; identical footprints |
| WP02 | Port `spike-lsm` to `kerno-storage` (copy), `Config` and `testing`; connect bench, differential and CI | WP01 | 2 | 24 + 7 + 7 tests green with `kerno` |
| WP03a | Testkit: failure model, phase probe, `HookVfs`, processes, `extended_failure_suite!` over `LogEngine` | WP01 | 2 | extended suites green in the reference |
| WP03b | `Inline` mode, `Inline` profile, phases in the engine, extended suites, ENOSPC and `proptest` model over `kerno`; CI split | WP02, WP03a | 3 | extended suites green in `kerno` |
| WP04 | Observability, integration in the bench, queue diagnosis and recorded targets | WP02 | 3 | `f1-performance-goals.md`, `f1-queue-diagnosis.md` |
| WP06 | v1 codec, structure vectors, independent decoder, ADR-0013 | WP02 | 3 | format as pure code |
| WP10a | `DeleteRange` in the contract, spikes and oracle; `range_delete_test_suite!` over the reference | WP02, WP03a | 3 | range suite validated in `LogEngine` and `Emulated<LogEngine>` |
| WP12a | `SyncEngine`, `sync_test_suite!`, `group_commit_test_suite!`, scenario C over `LogEngine` | WP03a | 3 | `sync()` and concurrency suites validated |
| WP05 | Bounded queues, closing budget, M0 profile and decision point | WP03b, WP04 | 4 | T1–T5 measured (provisional); GO or memo |
| WP07 | The engine writes and reads only v1 (MANIFEST as snapshot) | WP06, WP03b | 4 | suites green over v1, 0 harmless |
| WP13a | `TxnEngine`, transactions in `Emulated<E>`, `txn_test_suite!`, `rmw_counters`, `differential_rmw` (references) | WP10a, WP12a | 4 | transaction suite validated |
| WP08 | MANIFEST as log, session, `CLEAN`, recovery table, read-only, deferred deletion | WP07, WP05 | 5 | condition 3a, matrix covered |
| WP12b | Group commit, `sync()`, `loom`, back-pressure in `kerno` | WP07, WP12a, WP04, WP03b | 5 | condition 3b, T6 |
| WP09 | `verify()`, `inspect()`, crafted files, damage during use, locking between processes | WP08 | 6 | integrity (R-69, R-85, R-67) |
| WP10b | Range tombstones in `kerno` and `compact_range` | WP08, WP05, WP10a | 6 | condition 2 |
| WP11 | Directory vectors (also those of group commit), regeneration guard, freezing candidate | WP09, WP10b, WP12b | 7 | full format on disk and protected |
| WP13b | RMW transactions and snapshot metrics in `kerno` | WP12b, WP10b, WP13a | 7 | ADR-0002 complete |
| WP14 | `kill -9`, LazyFS, 1000-seed gate, benchmarks, freezing and documentation | WP11, WP13b | 8 | Phase 1 output |

The critical path has 8 levels (WP01 → WP02 → WP03b → WP07 → WP08 → WP10b → WP11/WP13b → WP14) against the 14 of the original chain. The measurements of WP04 and WP05 are made while other packages compile and test on the same machine: they rely on structural metrics of the engine (lock episodes, L0 depth, compaction sizes), record the system load and are **provisional**; the reference measurement is that of WP14, with the machine at rest.

## 12. Ambiguities resolved

| Id | Resolution |
|---|---|
| C-01 | `verify()` as a library in Phase 1; CLI in Phase 2 |
| C-02 | Group commit mandatory (ADR-0001 requires it); demonstrated with `fsync` per commit < 1 |
| C-03 | Write slot: the transaction holds it until it enqueues; blind commits only to enqueue; the leader groups blind batches |
| C-04, C-05 | `synced_len` per frame replaces `SYNCED`; durability per commit is deferred (the format already admits it) |
| C-06, C-07, C-08 | Targets T1–T10 recorded before measuring; "2× fjall" applies to throughput and to the named p99s; accepting ADR-0012 with them is proposed |
| C-09 | Comparisons in the same session; the owner names the target machine before WP14 (proposal: the M0 VM class and a consumer NVMe ext4 Linux machine) |
| C-10 | Mandatory part of L0 at close; the rest with budget |
| C-11 | Mechanism only; ranges in the base of v1 (no bit) |
| C-12 | 40 B slot per SST block and MANIFEST record; in the WAL, per file (pending signature in ADR-0013) |
| C-13 | Header at offset 0 of all files; the SST keeps the footer; `LOCK` exempt; `.tmp` follows its final type |
| C-14 | Only a WAL with no frames after a dirty close (or larger than `next_file` after a clean one) may have a broken header |
| C-15 | Read-only paths take the exclusive `LOCK` and may only create `LOCK` |
| C-16 | `Incompatible` and `ReadOnly` |
| C-17 | Unstable API; hidden contract |
| C-18 | Contract in `kerno-core`; harness in `kerno-testkit`; `spike-core` as shim |
| C-19 | "Freeze" = stable bytes + vectors + tested opening and recovery; migrations with v2 (record type 5 reserved) |
| C-21 | New ADR-0013; accepted ADRs only receive follow-up notes |
| C-22 | SAD 0.2.0 in WP14 |
| C-23 | I-16 |
| C-24 | `buffered_power_loss_prefix = true` for `kerno` |
| C-25 | The gate covers all the sweeps and the random cut in `Buffered` |
| C-26 | Format and API limits kept separate (v1.md §1.4) |
| C-27 | `scan` paginated and resumable; streamed iteration in Phase 2 |
| C-28 | Linux verified; the rest unverified and gated in `open` |
| C-29 | The output is judged against the updated matrix (§8) |
| C-30 | §5.7 |
| C-31 | Error, never wrap to 0 |
| C-32 | Manual rule per PR until there is a dedicated machine |
| C-33 | Order of §11; the contingency (R-10) at the decision point of WP05 |

## 13. Owner's decisions

1. Approve T1–T10 before the measurement of WP05 (WP04 records them as a proposal).
2. Accept ADR-0013 before the freezing (WP14), in particular: AEAD slot of the WAL per file, no `CURRENT` file, `CLEAN` inside the checkpoint, `synced_len`, `Incompatible`/`ReadOnly`, automatic read-only on an unknown `ro_compat`, `DRAFT` bit.
3. Accept ADR-0012 or adopt its thresholds as the exit criterion.
4. Name the target machine or machines before WP14.
5. Decide on the WP05 memo if T1 remains unmet.
