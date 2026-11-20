# Phase 1 report — exit (WP14)

Status: results of 2026-10-29 · Related to: [ADR-0001](../adr/0001-storage-engine.md), [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md), [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md), [targets T1–T10](f1-performance-goals.md), [decision on condition 1](f1-decision-cond1.md), [M0 report](m0-report.md), [test plan](test-plan.md), [failure matrix](../architecture/failure-matrix.md)

This report gathers the exit evidence of Phase 1 (SAD §16): first the **correctness**, then the **performance** against T1–T10, and at the end what is **pending or open**. The exit criteria that are not met or could not be run are stated as such; none is declared met by omission.

## 1. Summary of the exit gates

| Gate | Result | Evidence |
|---|---|---|
| `cargo fmt`, `clippy -D warnings`, `cargo test --workspace --all-features --release` | Verified at the close of this package (see §6); its output was not archived then | commands of §6 |
| `SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture` | **Met**: no failing test; `kerno` with 0 "harmless" and 0 "wrong data" | [`tests-seeds1000.log`](../../spikes/storage/results/2026-10-29-f1/tests-seeds1000.log) |
| Real `kill -9` of a writer (`StdVfs`), `KERNO_KILL_ITERS=200` | **Met**: 200 kills in `Sync` and 200 in `Buffered`, 2 tests green; plus 4 concurrent writers (group commit) with `wal_preallocate` in `Sync` and without it in `Buffered` | §3, [`process-kill-200.log`](../../spikes/storage/results/2026-10-29-f1/process-kill-200.log) |
| LazyFS (device with page loss) | **Pending**: no execution record | §4 |
| Benchmarks on the target machine, T1–T10 | **Run on the M0 VM class** (the target machine, C-09, is not named). **T1 is not met**, and there are other documented misses in §5 | [`results.jsonl`](../../spikes/storage/results/2026-10-29-f1/results.jsonl), [`report.md`](../../spikes/storage/results/2026-10-29-f1/report.md), [`env.json`](../../spikes/storage/results/2026-10-29-f1/env.json) |
| Format v1 freeze (ADR-0013 accepted, `DRAFT` bit retired) | **Freeze pending**: ADR-0013 still "Proposed" | §7 |

## 2. Correctness with 1000 seeds

Command: `SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture 2>&1 | tee spikes/storage/results/2026-10-29-f1/tests-seeds1000.log`, on a working tree based on `50583e2` with `tests/process_kill.rs` and `tests/lazyfs.rs` still uncommitted (they were added in `474b509`; between both commits only tests and one vector change, not the engine). All `test result:` lines in the log are `ok` and show 0 failures (the log contains `panicked` messages, which are those of the harness self-tests that check that an injected defect is detected; no test ends in `FAILED`).

**Corruption outcomes of the product engine (`kerno`, 1000 seeds per variant):**

| Variant | Truncations | Bit flips |
|---|---|---|
| `kerno` (default) | 1000 detected on open · 0 on read · **0 harmless** | 256 on open · 744 on read · **0 harmless** |
| `kerno-inline` | 1000 · 0 · **0** | 260 · 740 · **0** |
| `kerno-serial` | 1000 · 0 · **0** | 269 · 731 · **0** |
| `kerno-m0` (M0 profile) | 1000 · 0 · **0** | 245 · 755 · **0** |

The table omits the scenario with range fragments (`shared_ranges`): `kerno` 279 on open · 721 on read and `kerno-inline` 265 · 735, also with 0 harmless and 0 wrong data (in the log, `tests-seeds1000.log`).

No wrong data and no rollback. In the sweeps of directory vectors (`damage`), of truncations and of bit flips over v1 format files — several hundred thousand runs per scenario — every run was reported: 0 harmless and 0 with wrong data (for example, `bit flips basic-clean: 89880 runs, 89880 reported, 0 harmless, 0 wrong`). The only non-zero "harmless" in the log is that of the reference engine `log` (9 of 1000), which is not the product engine.

## 3. Real `kill -9`

`KERNO_KILL_ITERS=200 cargo test --release -p kerno-storage --test process_kill -- --nocapture`:

```
process_kill sync: 200 kills (152 after progress), recovered sequence 13142
test kill_dash_9_writer_recovers_acked_prefix_sync ... ok
process_kill buffered: 200 kills (152 after progress), recovered sequence 89534
test kill_dash_9_writer_recovers_acked_prefix_buffered ... ok
test result: ok. 2 passed; 0 failed; 0 ignored
```

After this report, two tests with **4 writer threads** were added (group commit, disjoint keys per thread, `Sync` + `wal_preallocate` and `Buffered`), and the full output of all four with 200 iterations was archived in [`process-kill-200.log`](../../spikes/storage/results/2026-10-29-f1/process-kill-200.log) (the sequence figures above are those of the first run, not those in the file). They still do not cover `begin_write` or `compact_range` on a real disk.

In both tests, after each `SIGKILL` the database is reopened, the recovered sequence is ≥ the last one confirmed by the writer, and the content matches the model. (In the 1000-seed pass of the log it was run with the 20 default iterations.)

## 4. LazyFS: pending

`crates/kerno-storage/tests/lazyfs.rs` and `spikes/storage/run-lazyfs.sh` exist, and `.github/workflows/lazyfs.yml` (manual and weekly) is added so that the owner can run it. **No run has been recorded**: in the 1000-seed log the two tests appear as `ignored, needs a LazyFS mount`, and in this environment LazyFS is neither installed nor built (`/dev/fuse` and `fusermount` exist, but not the LazyFS binary, which would have to be obtained and built from outside the repository). The exit criterion "scenario A on a device with page loss" remains **pending and is not marked as met**; until a record exists under `spikes/storage/results/<date>-f1/`, the equivalent evidence is `SimVfs` (simulated power cut) plus the real `kill -9` of §3, which does not lose pages of the operating system cache.

## 5. Performance against T1–T10

### 5.1 Environment and method

| | |
|---|---|
| Machine | The same VM class as M0: Intel Xeon 2.80 GHz, 4 cores, 15.7 GiB, ext4, kernel 6.18.44 ([`env.json`](../../spikes/storage/results/2026-10-29-f1/env.json)). **It is not the target machine**: C-09 is still unnamed |
| `fdatasync` of 4 KiB | p50 173 µs · p95 326 µs · p99 410 µs (M0: 113 / 400 / 568 µs) |
| Build | `rustc 1.97.0`, `release`, commit `50583e2` |
| Method | [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md) and [`f1-performance-goals.md`](f1-performance-goals.md): `medium` (200 000 records), `Sync`, median of 3 interleaved repetitions, `ENGINES=btree,lsm,kerno,redb,fjall,sqlite`. The ratios with `lsm` and `fjall` use engines measured **in this same session**; the `@m0` rows of the report serve only to compare with 2026-10-27 |
| Machine load | 1-minute average between 0.97 and 1.90 at the start of each group of runs (2.81 at the start of the session): a reasonable rest, **not** an empty machine. These figures replace the provisional ones of WP04 and WP05 |
| Scale `large` | **Incomplete**: `results.jsonl` only has `bulk_load` and `point_reads` with 1 000 000 records (1 repetition). `range_scans` and `small_commits` in `large` were **not run** in this session (in `report.md` only their `@m0` rows appear). It does not affect any T except the `large` part of T4, which was measured |
| `concurrent_commits` with 1 writer | Measured separately (`--threads 1`, 3 repetitions, `medium`) in [`results-threads1.jsonl`](../../spikes/storage/results/2026-10-29-f1/results-threads1.jsonl), because the main sweep only uses 8 writers |
| Data | 552 runs of the main sweep; the full report against M0 was generated with `spike-bench report --input … --baseline spikes/storage/results/2026-10-27/results.jsonl --baseline-label m0` ([`report.md`](../../spikes/storage/results/2026-10-29-f1/report.md)) |

### 5.2 T1–T10 (medians of `kerno`, scale `medium`, `Sync`)

| Id | Metric | M0 | `kerno` now | Target | Result |
|---|---|---|---|---|---|
| T1 | `agent_memory` write p99 | LSM 5 837 µs (3.71× fjall, 5.36× btree) | 3 938 µs; fjall 1 418; btree 1 748; spike-lsm 9 339 | ≤ 2.0× fjall **and** ≤ 1.5× btree | ❌ **Not met**: 2.78× fjall and 2.25× btree |
| T2 | commit p99, LiveSpec | 4 764 µs | 4 391 µs (spike-lsm 7 016) = 0.63× | ≤ 1.2× spike-lsm | ✅ |
| | Cauce | 713 µs | 1 609 µs (spike-lsm 827) = 1.95× | ≤ 1.2× spike-lsm | ❌ |
| | `append_seq` insert | 4 139 µs | 2 652 µs (5 591) = 0.47× | ≤ 1.2× spike-lsm | ✅ |
| | `append_seq` update | 6 840 µs | 4 747 µs (8 602) = 0.55× | ≤ 1.2× spike-lsm and ≤ 2× fjall | ✅ versus lsm · ❌ 2.28× fjall (2 081 µs) |
| | `history_sim` | 9 118 µs | 7 533 µs (11 928) = 0.63× | ≤ 1.2× spike-lsm | ✅ |
| T3 | read p99, LiveSpec module-scan | 657 µs | 951 µs (spike-lsm 931) = 1.02× | ≤ 1.2× spike-lsm; ≤ 1.5× spike-btree | ✅ versus lsm · ❌ 1.79× btree (532 µs) |
| | LiveSpec symbol-get | 43.7 µs | 55.5 µs (54.7) = 1.01× | ≤ 1.2× spike-lsm | ✅ |
| | Cauce step-read | 98.0 µs | 149 µs (153) = 0.97× | ≤ 1.2× spike-lsm | ✅ |
| | recall | 100 µs | 107 µs (119) = 0.90× | ≤ 1.2× spike-lsm | ✅ |
| | recent-scan | 234 µs | 159 µs (164) = 0.97× | ≤ 1.2× spike-lsm | ✅ |
| T4 | Close after `bulk_load` `medium` | 759 ms | 251 ms (range 235–296) | ≤ 250 ms | ❌ **By 1 ms, within noise: not declared met** |
| | Close after `bulk_load` `large` | 3 587 ms | 397 ms (1 repetition) | ≤ 1 000 ms | ✅ |
| T5 | Throughput per workload versus spike-lsm | — | `bulk_load` 0.85× · `point_reads` 1 thread 0.80× (4 threads 1.02×) · `range_scans` 0.71× · `cauce` 0.87× · `rmw_counters` 0.899× · `small_commits` 0.94× · `append_seq` 0.96× / 1.01× · `history_sim` 1.01× · `livespec` 1.02× · `agent_memory` 1.08× · `point_deletes` 1.00× · `mixed` 1.73× · `concurrent_commits` 3.67× | ≥ 0.9× spike-lsm | ❌ in `bulk_load`, `point_reads` (1 thread), `range_scans`, `cauce` and, by 0.001, `rmw_counters`; ✅ in the rest |
| T6 | `fsync` per commit | 1.00 | 1 writer: 1.00 (`small_commits` and `concurrent_commits`). 8 writers: **0.22** | 1 writer = 1.00; 8 writers ≤ 0.5 and ≥ 2× the throughput of 1 writer | ✅ 8 983 commits/s with 8 writers versus 3 606 with 1 = 2.49× |
| T7 | Write amplification `small_commits` | 1.09× | 4.29× (range 1.17–5.01; spike-lsm 1.09) | ≤ 1.20× | ❌ **Not met** and unstable between repetitions |
| | `bulk_load` | 3.4× | 3.33× | ≤ 3.6× | ✅ |
| T8 | Clean reopen | 3.4 ms | 3.59 ms (spike-lsm 4.29) = 0.84× | ≤ 1.0× spike-lsm | ✅ |
| | Dirty recovery | 9.73 ms | 8.52 ms (7.35) = 1.16× | ≤ 1.2× spike-lsm | ✅ (narrow margin) |
| T9 | RMW cost (`rmw_counters`) | — | 2 267 txn/s · p99 5 521 µs · 1.00 `fsync`/commit; spike-lsm 2 522 txn/s and p99 14 893 µs; fjall 3 228 txn/s | reported | reported |
| T10 | Throughput versus fjall | 0.8–1.3× | 0.70× (`rmw_counters`) to 1.12× (`point_deletes`) on the throughput workloads; 1.93× in `mixed` and 2.72× in `concurrent_commits` | reported; misses of 2× documented | reported: no throughput workload is below 0.5× fjall; the 2× tail miss is in T1 (2.78×) and T2 (2.28×) |

(Exact figures, with ranges between repetitions, are in [`report.md`](../../spikes/storage/results/2026-10-29-f1/report.md); the ratios were computed over the medians of `results.jsonl`.)

### 5.3 Documented misses

Everything is documented; none was corrected and the targets T1–T10 were not modified.

1. **T1** (`agent_memory` write p99): 2.78× fjall and 2.25× spike-btree versus ≤ 2.0× and ≤ 1.5×. An improvement over M0 (5 837 µs → 3 938 µs, 0.42× the spike-lsm of this session), but it does not reach the target. It is the case that [`f1-decision-cond1.md`](f1-decision-cond1.md) left as a candidate for NO-GO with provisional measurements; this reference measurement **does not reverse it**. According to the risk of the package, a T1 miss at exit triggers the **contingency review of ADR-0001 (R-10/R-11) by the owner**; the decision is theirs and remains open.
2. **T2**: Cauce commit p99 1.95× spike-lsm (target ≤ 1.2×); `append_seq` update p99 2.28× fjall (target ≤ 2×).
3. **T3**: LiveSpec module-scan p99 1.79× spike-btree (target ≤ 1.5×): according to the target, the condition 5 review applies.
4. **T4** `medium`: 251 ms versus ≤ 250 ms (range 235–296 ms). A 1 ms miss within the noise; it is reported as a miss.
5. **T5**: below 0.9× spike-lsm in `bulk_load` (0.85×), `point_reads` with 1 thread (0.80×), `range_scans` (0.71×), `cauce` (0.87×) and `rmw_counters` (0.899×, at the limit).
6. **T7** `small_commits`: 4.29× versus ≤ 1.20×, with a range of 1.17 to 5.01 between repetitions (the amplification depends on whether the compaction work falls within the measured window).
7. The data gap of `large` (§5.1) and the non-empty machine are limitations of the measurement, not misses.

## 6. Verification of the package tests

The acceptance commands repeated at the close are `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features --release`. The 1000-seed pass of §2 was done with full output (`--nocapture`) on that same tree (see §2); this review only adds documentation and benchmark results, with no changes to the engine code.

## 7. Format freeze: pending

[ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) is still in "Proposed" status: the owner has not accepted it. Under ADR-0003 governance the freeze **was not done**: the `DRAFT` bit remains in `WRITE_INCOMPAT` and `KNOWN_INCOMPAT` (`crates/kerno-storage/src/format/features.rs`), the vectors were not regenerated, the negative vector `draft-v1` does not exist, and [v1.md](../file-format/v1.md) still reads "Candidate for freeze". When the owner accepts ADR-0013, the process of v1.md §15 must be applied (retire `DRAFT`, regenerate the structure and directory vectors, add the negative vector `draft-v1` expecting `Incompatible('pre-freeze draft format')`, status "Frozen" and a history row).

## 8. Residual risks and open owner decisions

| Id | Risk or decision | Status |
|---|---|---|
| 1 | Contingency review of ADR-0001 for the T1 miss (R-10/R-11) | Open, owner's |
| 2 | Acceptance of ADR-0013 and freeze of v1 | Open, owner's |
| 3 | Approval of targets T1–T10 (still "pending" in [`f1-performance-goals.md`](f1-performance-goals.md)) | Open, owner's |
| 4 | Name the target machine (C-09) and repeat the measurement there (R-S3) | Open |
| 5 | LazyFS record (§4) | Pending |
| 6 | Incomplete `large` scale for `range_scans` and `small_commits` (§5.1) | Pending; repeat with `LARGE=1` on a machine at rest |
| 7 | T2, T3, T4, T5 and T7 misses (§5.3) | Not corrected in this package |
