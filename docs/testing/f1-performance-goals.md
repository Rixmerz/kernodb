# Phase 1 performance targets

| Field | Value |
|---|---|
| Status | **Proposal pending owner approval** (C-06; [Phase 1 design](../architecture/phase1-design.md) §13, decision 1) |
| Recorded on | 2026-10-28, in WP04, **before** any WP05 measurement |
| Source | [Phase 1 design](../architecture/phase1-design.md) §7.2, copied unchanged |
| Related | [ADR-0001](../adr/0001-storage-engine.md) (condition 1), [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md), [M0 report](m0-report.md), [diagnosis of the queues](f1-queue-diagnosis.md) |

These targets are recorded before measuring so that no WP05 or WP14 figure can choose its own threshold after the fact. The values in the table are those of the design (§7.2) and are not modified in this package: any change requires the owner's approval and is recorded in this document with its date and its reason.

## Method

- Scale `medium` (200 000 records), `Sync`, median of 3 interleaved repetitions, same session and same machine with `ENGINES=btree,lsm,kerno,redb,fjall,sqlite` ([ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md)).
- M0 values of 2026-10-27 (VM, 4 cores, ext4, `fdatasync` 4 KiB p50 113 / p99 568 µs).
- The ratios ("≤ 1.2× spike-lsm", "≤ 2.0× fjall") are computed with engines measured **in the same session**; never against figures from another machine or another day. To compare with M0, `spike-bench report --baseline spikes/storage/results/2026-10-27/results.jsonl --baseline-label m0` labels those rows as `lsm@m0`, etc., and never mixes them with the new ones.
- The WP04 and WP05 measurements are taken with other packages compiling and testing on the same machine: they are **provisional**. The reference measurement is the WP14 one, with the machine at rest (§11 of the design).

## Targets

| Id | Metric | M0 | Target |
|---|---|---|---|
| T1 | `agent_memory` memory-write p99 | LSM 5 837 µs; B+ tree 1 573; fjall 1 090 (3.71× / 5.36×) | ≤ 2.0× fjall **and** ≤ 1.5× spike-btree |
| T2 | commit p99: LiveSpec / Cauce / `append_seq` insert / `append_seq` update / `history_sim` | 4 764 / 713 / 4 139 / 6 840 / 9 118 µs | ≤ 1.2× spike-lsm; also `append_seq` update ≤ 2× fjall (M0 2.60×); misses documented |
| T3 | read p99: LiveSpec module-scan / symbol-get, Cauce, recall, recent-scan | 657 / 43.7 / 98.0 / 100 / 234 µs | ≤ 1.2× spike-lsm; LiveSpec module-scan ≤ 1.5× spike-btree (M0 1.49×), otherwise review of condition 5 |
| T4 | Clean close after `bulk_load` | 759 ms (`medium`), 3 587 ms (`large`); fjall 96 / 919 | ≤ 250 ms and ≤ 1 000 ms with the default budget |
| T5 | Throughput of each workload | e.g. 4 044 commits/s, load 162 kops/s, reads 230 kops/s | ≥ 0.9× spike-lsm |
| T6 | `fsync` per commit | 1.00 | 1 writer = 1.00; `concurrent_commits` with 8 writers ≤ 0.5 and ≥ 2× the throughput of 1 writer |
| T7 | Write amplification `small_commits` / `bulk_load` | 1.09× / 3.4× | ≤ 1.20× (the frame header adds 16 B per commit) / ≤ 3.6× |
| T8 | Clean reopen / dirty recovery | 3.4 ms / 9.73 ms | ≤ 1.0× / ≤ 1.2× spike-lsm |
| T9 | RMW cost (`rmw_counters`) | — | reported (R-S6) |
| T10 | Throughput versus fjall | 0.8–1.3× | reported; misses of 2× documented (C-07) |

## How each target is measured

| Id | Workload (`spike-bench`) | Columns |
|---|---|---|
| T1 | `agent_memory` | `memory_write_p99_us` |
| T2 | `livespec`, `cauce`, `append_seq`, `history_sim` | `changeset_commit_p99_us`, `step_commit_p99_us`, `append_commit100_p99_us`, `update_commit100_p99_us`, `rev_commit100_p99_us` |
| T3 | `livespec`, `cauce`, `agent_memory` | `module_scan_p99_us`, `symbol_get_p99_us`, `step_read_p99_us`, `recall_p99_us`, `recent_scan_p99_us` |
| T4 | `bulk_load` (`medium` and `large`) | `close_ms` |
| T5 | all | the throughput column of each workload (`commits_per_s`, `load_kops`, `get_1t_kops`, …) |
| T6 | `small_commits`; `concurrent_commits --threads 1` and `--threads 8` | `commit_fsyncs_per_commit`; `cc_fsyncs_per_commit`, `cc_commits_per_s` |
| T7 | `small_commits`, `bulk_load` | `write_amp` |
| T8 | `bulk_load` | `reopen_ms`, `recovery_ms` |
| T9 | `rmw_counters` (WP13a; native transactions in `kerno` since WP13b) | `rmw_txns_per_s`, `rmw_txn_p99_us`, `rmw_fsyncs_per_commit`; in `kerno`, `stat_txn_gate_wait_micros` |
| T10 | all | ratio `kerno` / `fjall` of the throughput column |

`concurrent_commits` (WP04) runs 20 000 small `Sync` commits (one `put` of 200 bytes with disjoint keys) spread across `--threads` threads; with 8 writers, T6 can only be met once group commit exists (WP12).

## Approval

| Date | Decision | Notes |
|---|---|---|
| — | pending | The owner approves or adjusts T1–T10 before WP05 measures (design §13, decision 1). |
