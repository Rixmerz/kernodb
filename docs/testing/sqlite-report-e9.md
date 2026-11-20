# kerno vs SQLite comparison report — new pass, no thresholds (E9-17)

| Field | Value |
|---|---|
| Status | Results of 2026-11-12. **This report sets no thresholds** (D6 of [`stage9-record.md`](../architecture/stage9-record.md) §8.1): it is a descriptive comparison |
| Package | E9-17 of [`stage9-record.md`](../architecture/stage9-record.md) §3 |
| Replaces | The E8-S0 pass ([`sqlite-report.md`](sqlite-report.md)) only as a measurement at rest; that report is kept |
| Raw data | [`spikes/storage/results/2026-11-12-e9/`](../../spikes/storage/results/2026-11-12-e9/) |

## 0. What it is and what it is not

- **Comparison without thresholds**: no figure is a target or a bar. The kerno / SQLite ratios are ratios of observed medians.
- **All** workloads are reported, including those in which kerno is slower (§4).
- Unlike E8-S0, this pass was run **at rest** (§2). That does not turn any figure into a general value: it is a single machine, a single scale and 5 repetitions.

## 1. Method

- **Existing binary, unchanged**: `spike-bench` (`spikes/storage/spike-bench`) as it is in the repository, built in `release` with `--features candidates,kerno`. No file of the binary or of any crate was modified. The durability pairs are those of [`sqlite-report.md`](sqlite-report.md) §1: kerno `Sync` versus SQLite WAL + `synchronous=FULL`, and kerno `Buffered` versus SQLite WAL + `synchronous=NORMAL`. **The guarantee against a power cut is not identical in the second pair** and this report does not measure it.
- **Workloads.** The 12 of [`sqlite-report.md`](sqlite-report.md) §1. `rmw_counters` is left out, for the same reason (in SQLite it runs on an emulation layer and is not the same workload).
- **Scale** `tiny` (5 000 records; everything fits in memory).
- **Repetitions: 5 per engine and workload** (seeds `42 + repetition`, the same for both), interleaved kerno, SQLite, kerno… Each cell of the tables comes from **5 runs**; a durability pair is 120 runs (12 workloads × 5 × 2 engines), 240 in total. No run failed and none was discarded.
- **Dispersion.** Median of the 5 repetitions and, in square brackets, minimum and maximum (observed range, not a confidence interval). The last column is the ratio of the medians, kerno / SQLite. When the ranges of the two engines overlap, the difference of medians does not distinguish one engine from the other.
- **Aggregation.** [`aggregate.py`](../../spikes/storage/results/2026-11-12-e9/aggregate.py) (the E8-S0 one with the list of files of this pass) produces [`tables.md`](../../spikes/storage/results/2026-11-12-e9/tables.md), which contains all the tables (§3).

### 1.1 Exact commands

From the root of the repository (`DATA` is a temporary working directory, deleted at the end):

```
cargo build --release -p spike-bench --features candidates,kerno
target/release/spike-bench env --dir $DATA > spikes/storage/results/2026-11-12-e9/env.json
W=bulk_load,point_reads,range_scans,small_commits,point_deletes,append_seq,mixed,history_sim,livespec,cauce,agent_memory,concurrent_commits
target/release/spike-bench suite --engines kerno,sqlite --workloads $W --scales tiny --reps 5 \
  --durability sync --dir $DATA --out spikes/storage/results/2026-11-12-e9/results-sync.jsonl
target/release/spike-bench suite --engines kerno,sqlite --workloads $W --scales tiny --reps 5 \
  --durability buffered --dir $DATA --out spikes/storage/results/2026-11-12-e9/results-buffered.jsonl
python3 -I spikes/storage/results/2026-11-12-e9/aggregate.py > spikes/storage/results/2026-11-12-e9/tables.md
```

### 1.2 Raw files

| File | Content |
|---|---|
| `env.json` | Output of `spike-bench env`, **reduced** to cores, memory, commit, load and `fdatasync` latency; the fields that describe the environment without being needed to reproduce it were removed by hand (T-NM) |
| `results-sync.jsonl`, `progress-sync.log` | Sync pair, 120 runs |
| `results-buffered.jsonl`, `progress-buffered.log` | Buffered pair, 120 runs |
| `aggregate.py`, `tables.md` | Aggregation and its output |

## 2. Environment and machine load (T-SQ1)

- 4 cores, 15.7 GiB. Code: commit `0be147b`.
- Free space before measuring: 7.1 GiB.
- **Nothing else was compiling or running** during the two batches (the build of `spike-bench` finished before). 1-minute load when each batch was launched: 1.83 (Sync) and 1.69 (Buffered), below 4 (T-SQ1). The load that the binary itself records in each run includes the benchmark itself:

  | File | Runs | Load (1 min), minimum – median – maximum |
  |---|---|---|
  | `results-sync.jsonl` | 120 | 1.33 – 1.54 – 1.90 |
  | `results-buffered.jsonl` | 120 | 1.58 – 1.69 – 1.89 |

  No run exceeded 4, so no block had to be repeated.
- `fdatasync` of 4 KiB (`env.json`): p50 93.0 µs · p95 160 µs · p99 198 µs · max 1 014 µs. In E8-S0 the same command measured p50 3 994 µs. This changes mainly the Sync pair: **the Sync figures of this pass and those of E8-S0 are not comparable with each other** (another device at another time); within each pass the kerno / SQLite comparison is valid.

## 3. Results

All the tables (each figure, median [min–max] of 5 runs) are in [`tables.md`](../../spikes/storage/results/2026-11-12-e9/tables.md); §7 reproduces them in full, without changes.

## 4. Observations (descriptive; no compliance conclusion)

Each point cites medians from the tables.

**Where kerno is slower or larger than SQLite:**

- **Opening and recovery.** `recovery_ms` of `bulk_load`: 3.54 versus 0.38 ms (Sync) and 3.96 versus 0.32 (Buffered); of `cauce`: 49.6 versus 0.36 (Sync) and 63.5 versus 0.35 (Buffered). `reopen_ms` of `bulk_load`: 0.62 versus 0.30 (Sync) and 0.77 versus 0.33 (Buffered).
- **Close after `bulk_load`.** `close_ms`: 11.1 versus 4.48 ms (Sync) and 14.0 versus 3.73 (Buffered).
- **`livespec`, module read.** `module_scan_p99_us`: 258 versus 165 µs (Sync) and 503 versus 162 (Buffered); `module_scan_p50_us` is equal in Sync (82.8 in both).
- **`small_commits` in Sync.** `commits_per_s` 7 226 versus 8 047 ([6 749–7 384] and [7 327–8 735]; overlapping ranges, so the difference of medians does not distinguish one engine from the other); `commit_p50_us` 127 versus 101 µs.
- **`append_seq`.** `peak_rss_mb` 11.0 versus 8.42 MB (Sync); `update_kops` in Sync 227 versus 252, with overlapping ranges.
- **`mixed` in Buffered**, `write_p50_us`: 25.3 versus 23.2 µs, with overlapping ranges.
- **`concurrent_commits`**, `cc_commit_p50_us`: 377 versus 131 µs (Sync) and 87.2 versus 18.2 (Buffered), although kerno's commit rate is higher.

**Where kerno is faster or smaller:**

- Point reads: `get_1t_kops` 1 628 versus 895 (Sync); `get_4t_kops` 4 928 versus 1 995.
- Range scans: `scan100_kops` 60.4 versus 36.5 (Sync).
- Writes: `bulk_load` `load_kops` 366 versus 89.3 (Sync); `concurrent_commits` 20 842 versus 6 769 commits/s (Sync), with 0.22 `fsync` per commit in kerno. In Buffered, `small_commits` 332 291 versus 57 444 commits/s and `cauce` 123 628 versus 13 121 steps/s; **it is not an advantage of equivalent guarantees** (§1).
- Amplification: `write_amp` of `bulk_load` 1.01 versus 6.25; of `small_commits` 1.17 versus 30.0 (Sync).
- Commit tails in workloads with `fsync`: `changeset_commit_p99_us` of `livespec` 940 versus 5 730 µs (Sync); `cc_commit_p99_us` 715 versus 11 665 µs (Sync).

**Workloads with a ratio close to 1 in Sync:** `commit_p95_us` of `small_commits` (0.99), `delete_commit_p50_us` (1.05), `step_commit_p50_us` of `cauce` (1.06) and `step_commit_p99_us` (0.94).

## 5. What **cannot** be concluded

- No compliance conclusion: there is no threshold.
- Nothing about a power cut or about data loss.
- Nothing about data sets larger than memory or about the `small`, `medium` or `large` scales.
- Nothing precise about tail latencies: with 5 repetitions, p99, p99.9 and maxima have wide ranges in several workloads (for example `get_4t_p99_us` of SQLite, [4.96–14.4] µs).
- Nothing about other machines or another disk device: the `fsync` of this pass is much faster than that of E8-S0 (§2) and would mainly change the Sync pair.
- Not a comparison between two tuned engines: the SQLite adapter is that of the binary (one writer connection and up to four readers, with no further tuning), and kerno's uses the default configuration of the binary.
- Each ratio is of medians of 5 repetitions, with no confidence interval nor significance test.

## 6. Limits of the run

1. A single pass per pair; the two batches were measured one right after the other, and one batch is not compared with the other.
2. `rmw_counters` was not run.
3. `env.json` is a reduced version of the binary's output (§1.2); the rest of the raw files are as the binary wrote them.

## 7. Tables

### Durability `sync`: kerno `Sync` versus SQLite WAL + `synchronous=FULL`

#### `bulk_load` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `load_kops` | 366 [314–385] | 89.3 [78.5–101] | 4.10 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 11.1 [10.2–11.5] | 4.48 [4.25–5.60] | 2.48 |
| `reopen_ms` | 0.62 [0.60–0.71] | 0.30 [0.30–0.45] | 2.07 |
| `recovery_ms` | 3.54 [3.52–5.15] | 0.38 [0.37–0.51] | 9.32 |
| `peak_rss_mb` | 9.50 [9.39–9.63] | 11.0 [10.9–11.1] | 0.86 |

#### `point_reads` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `get_1t_kops` | 1628 [1484–1700] | 895 [888–969] | 1.82 |
| `get_1t_p50_us` | 0.52 [0.50–0.52] | 0.86 [0.82–0.87] | 0.60 |
| `get_1t_p95_us` | 0.64 [0.61–0.78] | 1.60 [1.45–1.65] | 0.40 |
| `get_1t_p99_us` | 0.74 [0.70–0.89] | 2.11 [1.90–2.32] | 0.35 |
| `get_4t_kops` | 4928 [4535–5307] | 1995 [1827–2876] | 2.47 |
| `get_4t_p99_us` | 1.01 [0.96–1.11] | 12.7 [4.96–14.4] | 0.08 |

#### `range_scans` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `scan100_kops` | 60.4 [53.0–62.4] | 36.5 [34.3–39.7] | 1.66 |
| `scan100_p50_us` | 13.8 [13.6–14.4] | 23.4 [22.0–24.0] | 0.59 |
| `scan100_p95_us` | 19.8 [16.9–22.6] | 37.5 [29.4–39.5] | 0.53 |
| `scan100_p99_us` | 34.0 [32.4–43.0] | 59.2 [44.9–62.4] | 0.57 |

#### `small_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `commits_per_s` | 7226 [6749–7384] | 8047 [7327–8735] | 0.90 |
| `commit_p50_us` | 127 [122–129] | 101 [91.1–107] | 1.25 |
| `commit_p95_us` | 219 [200–225] | 221 [203–269] | 0.99 |
| `commit_p99_us` | 355 [317–366] | 327 [276–376] | 1.08 |
| `commit_p999_us` | 677 [654–1000] | 2548 [1912–5841] | 0.27 |
| `write_amp` | 1.17 [1.17–1.17] | 30.0 [29.9–30.1] | 0.04 |
| `commit_fsyncs_per_commit` | 1.00 [1.00–1.00] | — | — |

#### `point_deletes` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `delete_commits_per_s` | 7160 [6961–8672] | 6040 [5638–6653] | 1.19 |
| `delete_commit_p50_us` | 127 [108–136] | 121 [113–126] | 1.05 |
| `delete_commit_p99_us` | 323 [285–360] | 432 [339–493] | 0.75 |
| `get_deleted_p50_us` | 0.36 [0.34–0.37] | 0.79 [0.77–0.84] | 0.45 |
| `get_deleted_p99_us` | 0.59 [0.57–0.77] | 2.44 [2.27–2.75] | 0.24 |
| `get_live_p99_us` | 1.84 [1.78–2.12] | 2.58 [2.56–3.24] | 0.71 |

#### `append_seq` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `append_kops` | 238 [131–281] | 159 [130–182] | 1.50 |
| `update_kops` | 227 [201–280] | 252 [194–268] | 0.90 |
| `update_commit100_p99_us` | 628 [456–2652] | 720 [580–1032] | 0.87 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.0 [10.9–11.1] | 8.42 [8.03–8.49] | 1.30 |

#### `mixed` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `mixed_kops` | 128 [113–139] | 52.8 [48.9–53.9] | 2.43 |
| `read_p50_us` | 1.65 [1.57–1.71] | 10.4 [10.1–10.9] | 0.16 |
| `read_p99_us` | 12.8 [10.3–19.2] | 68.5 [67.3–73.1] | 0.19 |
| `write_p50_us` | 270 [262–299] | 525 [499–562] | 0.51 |
| `write_p99_us` | 557 [489–865] | 2165 [2087–2415] | 0.26 |

#### `history_sim` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `revisions_per_s_k` | 122 [89.1–140] | 52.2 [47.9–56.9] | 2.34 |
| `rev_commit100_p99_us` | 995 [871–4207] | 4108 [3607–4481] | 0.24 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.24 [2.14–3.62] | 3.66 [3.38–3.90] | 0.61 |
| `history_scan_p99_us` | 4.27 [3.52–6.32] | 11.1 [10.7–14.6] | 0.38 |
| `reconstruct_p99_us` | 2.58 [1.92–3.73] | 4.13 [4.04–6.12] | 0.62 |

#### `livespec` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `changesets_per_s` | 1588 [1527–1728] | 1058 [1029–1071] | 1.50 |
| `changeset_commit_p50_us` | 346 [294–367] | 452 [447–467] | 0.77 |
| `changeset_commit_p99_us` | 940 [875–979] | 5730 [5612–6025] | 0.16 |
| `symbol_get_p99_us` | 5.43 [5.21–6.43] | 9.46 [8.94–10.2] | 0.57 |
| `module_scan_p50_us` | 82.8 [80.7–90.3] | 82.8 [82.3–83.2] | 1.00 |
| `module_scan_p99_us` | 258 [218–301] | 165 [160–179] | 1.57 |

#### `cauce` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `steps_per_s` | 6739 [6495–7195] | 5638 [5394–6142] | 1.20 |
| `step_commit_p50_us` | 127 [121–131] | 121 [111–125] | 1.06 |
| `step_commit_p99_us` | 325 [289–334] | 344 [267–372] | 0.94 |
| `step_read_p99_us` | 20.5 [18.2–24.4] | 63.1 [54.9–74.0] | 0.33 |
| `recovery_ms` | 49.6 [48.0–54.4] | 0.36 [0.34–0.38] | 136.29 |

#### `agent_memory` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `steps_per_s` | 3068 [2757–3460] | 1463 [1287–1562] | 2.10 |
| `memory_write_p50_us` | 223 [211–255] | 293 [272–313] | 0.76 |
| `memory_write_p99_us` | 813 [599–1137] | 1377 [1163–2238] | 0.59 |
| `recall_p99_us` | 14.6 [13.1–18.6] | 57.6 [51.5–77.5] | 0.25 |
| `recent_scan_p99_us` | 33.3 [30.9–39.0] | 112 [91.6–155] | 0.30 |
| `expire_commit_p99_us` | 902 [423–1764] | 4188 [3809–6607] | 0.22 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

#### `concurrent_commits` (sync; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 20842 [20295–23449] | 6769 [6582–6967] | 3.08 |
| `cc_commit_p50_us` | 377 [343–393] | 131 [126–135] | 2.88 |
| `cc_commit_p99_us` | 715 [584–791] | 11665 [11297–11813] | 0.06 |
| `cc_fsyncs_per_commit` | 0.22 [0.22–0.22] | — | — |

### Durability `buffered`: kerno `Buffered` versus SQLite WAL + `synchronous=NORMAL`

#### `bulk_load` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `load_kops` | 531 [370–563] | 91.1 [79.7–97.1] | 5.82 |
| `write_amp` | 1.01 [1.01–1.01] | 6.25 [6.22–6.33] | 0.16 |
| `space_amp_closed` | 1.03 [1.03–1.03] | 1.48 [1.46–1.50] | 0.70 |
| `close_ms` | 14.0 [12.7–16.8] | 3.73 [3.51–4.28] | 3.75 |
| `reopen_ms` | 0.77 [0.60–0.83] | 0.33 [0.29–0.33] | 2.30 |
| `recovery_ms` | 3.96 [3.03–5.13] | 0.32 [0.30–0.36] | 12.41 |
| `peak_rss_mb` | 9.45 [9.30–9.54] | 11.1 [11.0–11.1] | 0.86 |

#### `point_reads` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `get_1t_kops` | 1603 [1552–1714] | 872 [725–951] | 1.84 |
| `get_1t_p50_us` | 0.52 [0.49–0.53] | 0.88 [0.83–1.01] | 0.59 |
| `get_1t_p95_us` | 0.64 [0.62–0.68] | 1.67 [1.47–2.00] | 0.38 |
| `get_1t_p99_us` | 0.77 [0.74–0.85] | 2.26 [1.90–2.83] | 0.34 |
| `get_4t_kops` | 4800 [4456–5117] | 2427 [2176–2478] | 1.98 |
| `get_4t_p99_us` | 0.96 [0.95–1.08] | 12.5 [10.1–14.7] | 0.08 |

#### `range_scans` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `scan100_kops` | 61.1 [56.8–64.7] | 38.9 [37.5–40.0] | 1.57 |
| `scan100_p50_us` | 13.8 [13.2–14.3] | 22.9 [22.0–23.2] | 0.60 |
| `scan100_p95_us` | 17.4 [15.9–18.4] | 32.8 [29.6–34.8] | 0.53 |
| `scan100_p99_us` | 32.9 [31.4–33.4] | 50.0 [48.0–51.1] | 0.66 |

#### `small_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `commits_per_s` | 332291 [291183–345897] | 57444 [53937–60207] | 5.78 |
| `commit_p50_us` | 1.94 [1.92–2.09] | 6.09 [5.94–6.14] | 0.32 |
| `commit_p95_us` | 3.81 [3.74–4.06] | 9.56 [8.47–9.84] | 0.40 |
| `commit_p99_us` | 10.1 [8.38–11.0] | 21.9 [20.8–23.5] | 0.46 |
| `commit_p999_us` | 24.4 [21.0–28.9] | 1496 [613–3271] | 0.02 |
| `write_amp` | 1.27 [1.27–1.27] | 29.7 [29.6–29.8] | 0.04 |
| `commit_fsyncs_per_commit` | 0.00 [0.00–0.00] | — | — |

#### `point_deletes` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `delete_commits_per_s` | 504718 [448818–522184] | 31415 [30829–36515] | 16.07 |
| `delete_commit_p50_us` | 1.65 [1.62–1.69] | 8.37 [7.63–10.1] | 0.20 |
| `delete_commit_p99_us` | 3.59 [3.19–3.72] | 45.4 [40.1–57.5] | 0.08 |
| `get_deleted_p50_us` | 0.37 [0.35–0.39] | 0.80 [0.77–0.81] | 0.46 |
| `get_deleted_p99_us` | 0.62 [0.55–0.85] | 2.38 [2.16–2.54] | 0.26 |
| `get_live_p99_us` | 1.42 [1.29–1.90] | 2.72 [2.38–2.92] | 0.52 |

#### `append_seq` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `append_kops` | 887 [871–918] | 511 [421–543] | 1.73 |
| `update_kops` | 791 [779–817] | 389 [324–406] | 2.03 |
| `update_commit100_p99_us` | 122 [110–134] | 435 [324–659] | 0.28 |
| `write_amp` | 1.02 [1.02–1.02] | 2.35 [2.35–2.38] | 0.43 |
| `space_amp` | 3.05 [3.04–3.06] | 4.27 [4.26–4.28] | 0.71 |
| `peak_rss_mb` | 11.0 [10.9–11.1] | 8.50 [8.34–8.59] | 1.30 |

#### `mixed` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `mixed_kops` | 679 [632–705] | 136 [130–139] | 5.01 |
| `read_p50_us` | 1.92 [1.85–2.08] | 15.1 [13.8–15.4] | 0.13 |
| `read_p99_us` | 20.8 [15.7–25.1] | 94.5 [86.5–160] | 0.22 |
| `write_p50_us` | 25.3 [23.8–27.6] | 23.2 [20.3–24.4] | 1.09 |
| `write_p99_us` | 99.7 [92.9–151] | 356 [247–491] | 0.28 |

#### `history_sim` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `revisions_per_s_k` | 331 [245–343] | 60.4 [55.2–63.7] | 5.49 |
| `rev_commit100_p99_us` | 344 [302–382] | 9855 [9675–10461] | 0.03 |
| `space_amp` | 4.59 [4.59–4.59] | 7.85 [7.85–7.85] | 0.58 |
| `history_scan_p50_us` | 2.25 [2.16–3.17] | 3.80 [3.76–3.92] | 0.59 |
| `history_scan_p99_us` | 4.16 [3.71–15.3] | 12.7 [10.6–15.9] | 0.33 |
| `reconstruct_p99_us` | 2.71 [2.07–5.00] | 5.39 [4.20–5.95] | 0.50 |

#### `livespec` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `changesets_per_s` | 3026 [2898–3082] | 1383 [1366–1426] | 2.19 |
| `changeset_commit_p50_us` | 41.0 [40.7–42.1] | 105 [100–106] | 0.39 |
| `changeset_commit_p99_us` | 160 [147–171] | 11411 [11125–11526] | 0.01 |
| `symbol_get_p99_us` | 5.23 [4.95–5.42] | 8.96 [8.00–9.52] | 0.58 |
| `module_scan_p50_us` | 84.3 [81.9–85.4] | 80.3 [79.4–82.1] | 1.05 |
| `module_scan_p99_us` | 503 [470–532] | 162 [156–191] | 3.10 |

#### `cauce` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `steps_per_s` | 123628 [110694–125224] | 13121 [12407–14130] | 9.42 |
| `step_commit_p50_us` | 2.96 [2.94–3.29] | 11.8 [11.1–12.6] | 0.25 |
| `step_commit_p99_us` | 7.54 [7.27–9.92] | 63.8 [59.2–78.7] | 0.12 |
| `step_read_p99_us` | 14.8 [12.5–16.1] | 62.5 [57.9–66.6] | 0.24 |
| `recovery_ms` | 63.5 [59.1–73.0] | 0.35 [0.35–0.47] | 179.97 |

#### `agent_memory` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `steps_per_s` | 15497 [14144–15728] | 2216 [2036–2433] | 6.99 |
| `memory_write_p50_us` | 7.00 [6.99–7.45] | 40.3 [36.4–43.2] | 0.17 |
| `memory_write_p99_us` | 34.2 [32.0–36.9] | 574 [530–672] | 0.06 |
| `recall_p99_us` | 16.4 [14.8–17.8] | 54.7 [48.1–66.5] | 0.30 |
| `recent_scan_p99_us` | 38.8 [34.3–39.9] | 93.3 [78.8–135] | 0.42 |
| `expire_commit_p99_us` | 143 [117–858] | 9372 [8643–15843] | 0.02 |
| `space_amp` | 1.89 [1.88–1.89] | 2.03 [2.02–2.03] | 0.93 |

#### `concurrent_commits` (buffered; repetitions: kerno 5, sqlite 5)

| Metric | kerno median [min–max] | sqlite median [min–max] | kerno / sqlite (medians) |
|---|---|---|---|
| `cc_threads` | 8.00 [8.00–8.00] | 8.00 [8.00–8.00] | 1.00 |
| `cc_commits_per_s` | 85971 [82277–89258] | 26204 [25032–28192] | 3.28 |
| `cc_commit_p50_us` | 87.2 [84.7–90.8] | 18.2 [16.8–21.8] | 4.79 |
| `cc_commit_p99_us` | 262 [253–290] | 8569 [8110–8774] | 0.03 |

