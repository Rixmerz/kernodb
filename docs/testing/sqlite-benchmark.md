# Kerno against SQLite: a reproducible benchmark

Roadmap Phase C asks for "a published, reproducible benchmark against SQLite". This is it: one
script, equivalent durability settings, every workload reported, and the cases where Kerno loses
listed first-class, not buried.

| | |
|---|---|
| Run | 2026-11-18, commit `fd1c035` (the run's `env.json` records it) |
| Raw data and tables | [`spikes/storage/results/2026-11-18-sqlite/`](../../spikes/storage/results/2026-11-18-sqlite/): `results-sync.jsonl`, `results-buffered.jsonl`, `progress-*.log`, `env.json`, `tables.md` (every table of §4) |
| Reproduce | `SCALES=tiny REPS=5 spikes/storage/run-sqlite-comparison.sh` ([script](../../spikes/storage/run-sqlite-comparison.sh), [aggregator](../../spikes/storage/aggregate-sqlite.py)) |
| Earlier, descriptive runs | [`sqlite-report.md`](sqlite-report.md) (E8-S0) and [`sqlite-report-e9.md`](sqlite-report-e9.md) (E9-17), in Spanish, with their own scripts and data; this one replaces their hand-assembled commands with a script |

**Read §3 before the numbers.** The machine was heavily shared during the run, so only differences
larger than the spread between repetitions mean anything.

## 1. What is compared

- **The storage engines, not the databases.** Both engines run behind the same key-value
  contract (`Engine`/`Snapshot` of `kerno-core`) in the `spike-bench` harness: `kerno` is
  `kerno-storage` (`KernoDb`), `sqlite` is the bundled SQLite through `rusqlite`, one table
  `kv(k BLOB PRIMARY KEY, v BLOB NOT NULL) WITHOUT ROWID`. The units layer, the query language and
  everything SQLite offers above a key-value table (SQL, joins, a query planner) are **not**
  measured. A result here says how the two engines store and fetch bytes, nothing about queries.
- **Same workloads, same data.** The 12 workloads of the harness that both engines run as they
  are: `bulk_load`, `point_reads`, `range_scans`, `small_commits`, `point_deletes`, `append_seq`,
  `mixed`, `history_sim`, `livespec`, `cauce`, `agent_memory`, `concurrent_commits` (8 writer
  threads). `rmw_counters` is left out, as in the earlier reports: SQLite runs it through an
  emulation layer, so it is not the same workload in both.
- **Same cache budget.** Both get the harness default of 64 MiB. SQLite splits it between its
  connections (half for the writer, an eighth for each reader connection) because every SQLite
  connection has a private page cache.
- **Fresh process and directory per run**, and the two engines alternate repetition by
  repetition, so a slow drift of the machine does not favour one of them.

### Durability: the pairs

| Pair | Kerno | SQLite | Same guarantee? |
|---|---|---|---|
| `sync` | `Durability::Sync`: the commit's WAL frame is `fsync`ed before `commit` returns (concurrent writers share one `fsync`, a group commit) | `journal_mode=WAL`, `synchronous=FULL`: the WAL is `fsync`ed at every commit | **Yes**: a commit that returned survives a power cut |
| `buffered` | `Durability::Buffered`: no `fsync` per commit | `journal_mode=WAL`, `synchronous=NORMAL`: the WAL is `fsync`ed at checkpoints | **No.** Both survive a process crash. How much of the latest commits each loses in a power cut, and how each recovers, are different, and this report does not measure it. The pair compares the cost of running without a per-commit `fsync`. |

Quote `sync` when comparing the engines. Treat `buffered` as a second, looser data point.

## 2. How to reproduce

```bash
# On a quiet machine, with the scratch directory on the disk you want to measure:
SCALES=tiny REPS=5 OUT=spikes/storage/results/<date>-sqlite DATA=bench-data \
  spikes/storage/run-sqlite-comparison.sh
```

The script builds `spike-bench` in release mode, writes `env.json` (CPU, kernel, `rustc`, commit,
load average, file system, and the latency of a 4 KiB `fdatasync` on the scratch disk), runs both
durability pairs, and aggregates with `aggregate-sqlite.py` into `tables.md`. Options:
`SCALES` (`tiny` 5 000 records, `small` 20 000, `medium` 200 000, or a number), `REPS`,
`WORKLOADS`, `OUT`, `DATA`. **It refuses a scratch directory on tmpfs or ramfs**: `fsync` is free
there, so the `sync` pair would compare nothing (`ALLOW_TMPFS=1` overrides it, for a measurement you
will label as such).

For each metric the aggregator prints the median and the `[min-max]` over the repetitions and the
ratio of the medians, and labels Kerno **better**, **worse**, or **tie**. It is a tie when the
ranges of the two engines overlap; only disjoint ranges count as a difference. Throughput metrics
(`*_kops`, `*_per_s`) are better when higher; latency, time, amplification, memory and `fsync`s per
commit are better when lower.

## 3. Environment, and why the numbers are noisy

| | |
|---|---|
| Machine | AMD Ryzen 7 4800H, 14 hardware threads, 15 GiB, Linux 7.3.3, `rustc` 1.99.1 |
| Scratch disk | NVMe, **btrfs** (`compress=zstd:1`), not tmpfs |
| `fdatasync` of 4 KiB | p50 1 474 µs, p99 5 526 µs, max 9 142 µs (`env.json`). The earlier E9 run measured p50 93 µs on a different file system, so the `sync` numbers of the two reports are not comparable. |
| Load | **High and not mine.** Other jobs ran on the machine during the whole run. The 1-minute load average the harness records at the start of each run: `sync` pair min 10.7, median 27.6, max 37.2; `buffered` pair min 17.5, median 29.0, max 32.3 (on 14 hardware threads). |
| Scale | `tiny`: 5 000 records. The whole data set fits in memory, so this is mostly a CPU and `fsync` benchmark, not a disk-capacity one. |
| Repetitions | 5 per engine and workload: 120 runs per pair, 240 in total, no run failed. |

Consequences: absolute numbers are not reproducible on another machine, and any ratio inside the
spread of the repetitions is noise (the tables call it a tie). The direction of the large
differences (below) was the same in the two earlier, quieter runs on a different disk. A quiet
machine, a larger scale and more repetitions would tighten the ranges; the script takes them as
options.

## 4. Results

All tables are in [`tables.md`](../../spikes/storage/results/2026-11-18-sqlite/tables.md). Kerno /
SQLite is the ratio of the medians.

### 4.1 Count, per pair

| Pair | Kerno better | Tie (ranges overlap) | Kerno worse |
|---|---|---|---|
| `sync` | 35 | 27 | 5 |
| `buffered` | 44 | 18 | 5 |

(67 metrics per pair; the count leaves out metrics only one engine reports, such as `fsync`s per
commit.)

### 4.2 `sync` pair (equal guarantee): headline per workload

| Workload | Metric | Kerno | SQLite | Kerno / SQLite | |
|---|---|---|---|---|---|
| `bulk_load` | `load_kops` | 69.6 | 18.2 | 3.81 | better |
| `point_reads` | `get_1t_kops` | 363 | 171 | 2.12 | better |
| `point_reads` | `get_4t_p99_us` | 3.33 | 13.2 | 0.25 | better |
| `range_scans` | `scan100_kops` | 13.1 | 6.75 | 1.94 | better |
| `small_commits` | `commits_per_s` | 335 | 290 | 1.16 | tie |
| `small_commits` | `commit_p99_us` | 10 797 | 11 674 | 0.92 | tie |
| `point_deletes` | `delete_commits_per_s` | 332 | 282 | 1.17 | tie |
| `append_seq` | `append_kops` | 36.2 | 30.1 | 1.20 | better |
| `mixed` | `mixed_kops` | 7.30 | 2.77 | 2.64 | better |
| `history_sim` | `revisions_per_s_k` | 30.9 | 10.8 | 2.85 | better |
| `livespec` | `changesets_per_s` | 400 | 293 | 1.37 | tie |
| `cauce` | `steps_per_s` | 506 | 376 | 1.35 | tie |
| `agent_memory` | `steps_per_s` | 369 | 255 | 1.44 | tie |
| `concurrent_commits` (8 threads) | `cc_commits_per_s` | 1 785 | 430 | 4.15 | better |
| `concurrent_commits` | `cc_commit_p99_us` | 14 639 | 54 166 | 0.27 | better |

What the `sync` pair shows:

- **One writer, one `fsync` per commit: a tie.** `small_commits`, `point_deletes`, `cauce`,
  `agent_memory` and `livespec` are bound by the 1.5 ms `fdatasync`, and the two engines pay it the
  same way. Kerno does not commit faster than SQLite when the disk is the bottleneck, only with
  less write amplification (1.17 against 30.0 in `small_commits`).
- **Many writers: Kerno wins clearly** (4.2× the commits, 0.27× the p99), because group commit lets
  eight threads share one `fsync` (0.21 `fsync`s per commit).
- **Reads and scans: Kerno is 2× faster** in throughput and has the smaller tail, in memory.
- **Write amplification and space:** lower in every workload that reports them (`bulk_load` 1.01
  against 6.25; `history_sim` space 4.59 against 7.85).

### 4.3 `buffered` pair (not equal guarantees): headline

| Workload | Metric | Kerno | SQLite | Kerno / SQLite | |
|---|---|---|---|---|---|
| `small_commits` | `commits_per_s` | 36 630 | 6 634 | 5.52 | better |
| `small_commits` | `commit_p99_us` | 67.5 | 1 950 | 0.03 | better |
| `point_deletes` | `delete_commits_per_s` | 133 932 | 6 460 | 20.7 | better |
| `bulk_load` | `load_kops` | 350 | 34.8 | 10.0 | better |
| `cauce` | `steps_per_s` | 35 670 | 2 303 | 15.5 | better |
| `concurrent_commits` | `cc_commits_per_s` | 69 806 | 8 579 | 8.14 | better |
| `point_reads` | `get_1t_kops` | 1 103 [262-1 161] | 519 [161-584] | 2.13 | tie |
| `mixed` | `mixed_kops` | 124 [61.6-804] | 21.8 [12.9-81.1] | 5.70 | tie |

These ratios are large because Kerno `Buffered` does no `fsync` per commit and SQLite
`synchronous=NORMAL` still syncs at its checkpoints. Do not read them as "Kerno is 5 to 20 times
faster than SQLite"; read them as what skipping the per-commit `fsync` is worth in each engine.

### 4.4 Where Kerno is worse (disjoint ranges), in both pairs

| Pair | Workload | Metric | Kerno | SQLite | Kerno / SQLite |
|---|---|---|---|---|---|
| sync | `bulk_load` | `close_ms` | 52.0 [47.8-78.1] | 8.24 [7.23-13.6] | 6.32 |
| sync | `bulk_load` | `reopen_ms` | 5.96 [5.47-8.86] | 2.87 [2.70-4.59] | 2.07 |
| sync | `bulk_load` | `recovery_ms` | 22.0 [16.6-37.4] | 2.82 [1.32-3.75] | 7.81 |
| sync | `cauce` | `recovery_ms` | 94.0 [61.8-520] | 0.56 [0.42-2.49] | 166.64 |
| sync | `append_seq` | `peak_rss_mb` | 11.0 [10.9-11.1] | 8.37 [8.22-8.53] | 1.31 |
| buffered | `bulk_load` | `close_ms` | 27.8 [25.6-52.0] | 3.03 [2.87-4.07] | 9.18 |
| buffered | `bulk_load` | `reopen_ms` | 3.38 [3.26-4.33] | 0.60 [0.53-0.69] | 5.65 |
| buffered | `bulk_load` | `recovery_ms` | 14.7 [10.3-25.2] | 0.74 [0.57-0.96] | 19.93 |
| buffered | `cauce` | `recovery_ms` | 173 [150-380] | 1.07 [0.98-16.4] | 161.67 |
| buffered | `append_seq` | `peak_rss_mb` | 11.0 [10.9-11.1] | 8.39 [8.13-8.53] | 1.31 |

**Kerno loses where work happens at the start and the end of a process**, not in steady state:

- **Close.** `close` flushes the memtable and may run compactions before it returns, up to
  `Config::close_budget` (by default 256 MiB or 200 ms), so closing right after a bulk load costs
  28 to 52 ms against 3 to 8 ms for SQLite.
- **Opening a database that was not closed cleanly (`recovery_ms`).** Kerno replays its WAL at
  open; 22 to 173 ms in these runs, against 0.6 to 2.8 ms for SQLite. The `cauce` row is the largest
  gap of the whole report (two orders of magnitude) and has a wide spread (62 to 520 ms in the
  `sync` pair). For a long-lived service it is paid once; for a short-lived CLI that opens a
  database often, it is the cost a user feels first.
- **Memory:** about 2.6 MiB more peak RSS in `append_seq` (11.0 against 8.4 MiB).

The report does not diagnose these three: it measures them. They are the candidates for the next
performance work, in that order.

## 5. What this does not show

- **Not a claim about SQL databases.** Only the key-value layer of each engine ran. Queries,
  joins, schemas and indexes, where Kerno's layers add work and SQLite's planner is mature, are not
  compared.
- **Not a claim about large data.** At 5 000 records everything is in memory. A `small` (20 000)
  or `medium` (200 000) run, on a quiet machine, is the next measurement.
- **Not a power-loss comparison.** The `sync` pair is equivalent by the documented behaviour of
  each engine's setting (`fsync` of the WAL per commit); this report did not cut power or use a
  device that loses pages; the crash tests of `crates/kerno-storage/tests` check Kerno alone, not
  SQLite. The `buffered` pair is not equivalent at all.
- **One machine, one file system, one load.** btrfs with compression, under heavy and uncontrolled
  load. The `fdatasync` latency of the disk moves the `sync` numbers more than any engine
  difference.
- **SQLite is not tuned** beyond WAL, `synchronous` and the cache size: default page size, default
  WAL auto-checkpoint, no `mmap`. Kerno runs with its defaults too.
