# Diagnosis of the write-latency tails

| Field | Value |
|---|---|
| Status | WP04 diagnosis ([Phase 1 design](../architecture/phase1-design.md) §5.4). **Provisional** measurements: the machine was shared with other packages that were compiling and running tests (1-minute load between 1.4 and 14 during the runs). The reference measurement is the WP14 one, with the machine at rest |
| Date | 2026-10-28 |
| Measured code | `b742393` (`kerno-storage` = `spike-lsm` + instrumentation; the default behaviour is that of M0) |
| Targets | [f1-performance-goals.md](f1-performance-goals.md), recorded before measuring (commit `fed9656`) |
| Data | [`spikes/storage/results/2026-10-28-f1-diag/`](../../spikes/storage/results/2026-10-28-f1-diag): [`report.md`](../../spikes/storage/results/2026-10-28-f1-diag/report.md) (tables), `results.jsonl`, `ab.jsonl`, `trace.jsonl` and their repetitions `*-5reps.jsonl`, `overhead.jsonl` and `overhead-final*.jsonl`, `env.json`, `runs.log` (with `uptime` before and after each run) |
| Related to | [ADR-0001](../adr/0001-storage-engine.md) (condition 1), [M0 report](m0-report.md), [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md) |

## 1. Summary

- **The `agent_memory` tail is almost entirely the WAL `fdatasync`.** In the 1 % slowest memory-write commits, **99.4 %** of the time (median of 3 runs; 98.6 % in 5 other runs with a more loaded machine) is in the WAL sync; **100 %** of the time is attributed to named phases (requirement: ≥ 80 %). No commit of the tail was blocked, none changed memtable and, with the machine lightly loaded, none was slowed down.
- **That `fdatasync` is slow when something else writes to the disk.** With a flush or a compaction of the engine itself in progress, the p99 of `wal_sync` is 12 ms (16 ms with the more loaded machine); without them, 4 ms; the isolated 4 KiB `fdatasync` of `env.json`, 0.42 ms. With the machine lightly loaded, 41 of the 50 commits of the tail synced with background writing of the engine, versus 15 % of all commits.
- **Verdicts:** H3 (write interference) **confirmed**, it is the mechanism; H2 (debt after the load) **confirmed** as the origin of that background writing (the measured phase starts with ~78 MB of pending compaction and half of the tail falls in its first fifth), but without the "L0 without priority" effect; H1 (multi-prefix flushes) **confirmed structurally** (each L0 compaction rewrites on average 63 % of the L1 files; the compaction writes 2.8× the bytes of the flush, versus 1.2–1.4× in the other three workloads) and **rejected as a direct cause of the tail**: the stalls and slowdowns it predicted do not appear in the measured phase; H4 (foreground rotation) **rejected**.
- **A/B:** `bytes_per_sync = 1 MiB` halves the p99 of `wal_sync` with background writing in every session, but the p99 of memory writes only improves in the least loaded session (−39 %), and compaction becomes slower (twice the debt at the start of the measured phase, L0 up to 12 files, some stall). Removing the throttling changes nothing; raising the L0 limits makes things worse.
- **T1, provisional:** `kerno` 5.0 ms versus fjall 0.45 ms (11×) and spike-btree 0.81 ms (6.2×), with targets of ≤ 2× and ≤ 1.5×. What must be corrected in WP05 is the competition of the WAL with background writing, not back-pressure.

## 2. Method

### 2.1 Instrumentation (WP04)

`KernoDb::metrics()` returns typed metrics and `stats()` flattens them with stable names ([DESIGN.md](../../crates/kerno-storage/DESIGN.md), "Statistics"). The ones this diagnosis uses:

| Question | Metric |
|---|---|
| Where does a commit spend its time? | log2 histograms (each power of two split into four) of `make_room`, `wal_write`, `wal_sync`, `apply` and the total: `commit_<fase>_{count,micros,p50_micros,p99_micros,max_micros}` |
| Does the WAL `fdatasync` compete with background writing? (H3) | `wal_sync` split into `wal_sync_busy` (there was a flush or a compaction writing when the sync started or ended) and `wal_sync_quiet` |
| Are writers blocked or slowed down? (H1, H2) | stall episodes, counted once on entering the wait, by reason (`stall_episodes_{memtable,l0}`, `stall_micros_*`, `stall_micros_max`); `slowdown_commits`, `slowdown_micros` |
| How much does each L0 compaction rewrite? (H1) | `l0_compactions`, `l0_compaction_input_bytes{,_max}`, L1 files rewritten (`l0_compaction_l1_files{,_max}`) and present (`l0_compaction_l1_files_present`) |
| Is there compaction debt? Does L0 lose against the deeper levels? (H2) | `pending_compaction_bytes`, `l0_files_max`, `compactions_while_l0_slowdown` |
| Does a foreground rotation cost anything? (H4) | `memtable_rotation_micros{,_max}` |

`spike-bench` adds at the end of each workload all the engine statistics as `stat_<nombre>`. To attribute the tail, `--trace true` reads the statistics before and after each measured `agent_memory` commit (outside the timed region): since the commits are serialised, the difference of the accumulated counters is exactly the breakdown of that commit. Of the 5 000 measured commits the slowest 1 % (50) is taken and its latency, measured by the bench, is shared out among stalls on memtables, stalls on L0, slowdown, rotation, the rest of `make_room`, `wal_write`, `wal_sync` and `apply` (named phases), engine time outside them, and time outside the engine (`attr_tail_*`; `attr_all_*` for the 5 000). The trace loads the machine somewhat (two `stats()` calls per step) and is never used to compare engines.

**Cost of the instrumentation** (acceptance criterion: `kerno` throughput ≥ 0.97× spike-lsm in the same session): **not verifiable on this shared machine; the verdict is left for WP14.** The first run (`small`, 3 repetitions, `overhead.jsonl`, load 4.7–5.2) gives medians of commits/s of 1.03×, reads with 1 thread 1.00× and with 4 threads 1.07×, but it does not serve as a test: in `small_commits` repetition 0 of both engines is atypical (1 144 commits/s `lsm` and 1 254 `kerno`, against about 6 500–6 900 in repetitions 1 and 2), so the dispersion of commits/s is 89 % (`lsm`) and 82 % (`kerno`), far above the 15 % of the risk rule, which already required repeating with 5. The repetitions on the final commit of the package (`e1575d3`, WP04 integrated with WP03a, with the same engine code; `overhead-final.jsonl`, `overhead-final-5reps.jsonl`) were made with load 9.8–12.2: with 3 repetitions, 1.30×, 0.92× and 0.86×; with 5, 1.18×, 1.17× and 1.09×, with dispersions of up to 496 %. An independent run of the review (5 repetitions, load 2.6–4.2, not recorded in the repository) gave 0.67×, 0.98× and 1.35×, with `kerno` commits/s between 916 and 4 918. With this dispersion the ratio neither proves nor refutes the criterion. After those measurements, the phase histograms of the commit began to be written without atomic read-modify-write operations (there is a single writer, under the write lock): relaxed loads and stores, plus six `Instant::now()` per commit. The per-thread read counters keep the 4 readers from writing to the same cache line (in M0 all of them added into the same atomic counter).

### 2.2 Runs

Scale `medium` (200 000 records), `Sync`, cache of 64 MiB, a new process and a new directory per run, engines interleaved (ADR-0012), ext4 over `/dev/vda` (4-core VM at 2.10 GHz; M0 was measured on another VM at 2.80 GHz).

| File | Command | 1-min load |
|---|---|---|
| `results.jsonl` | `suite --engines lsm,kerno,btree,fjall --workloads agent_memory,livespec,cauce,append_seq --scales medium --reps 3 --durability sync` | 1.4–3.1 |
| `results-5reps.jsonl` | the same with `--reps 5` (risk rule: dispersion > 15 %) | 2.0–8.2 |
| `ab.jsonl` | per variant, `KERNO_TUNING=<variante> suite --engines kerno,kerno-tuned --workloads agent_memory … --reps 3` | 1.9–5.6 |
| `ab-5reps.jsonl` | the two variants with `bytes_per_sync`, `--reps 5` | 3.2–14 |
| `trace.jsonl`, `trace-5reps.jsonl` | `KERNO_TUNING=bytes_per_sync=1048576 suite --engines kerno,kerno-tuned --workloads agent_memory … --trace true`, 3 and 5 repetitions | 4.9–5.6; 10–13 |
| `overhead.jsonl` | `suite --engines lsm,kerno --workloads small_commits,point_reads --scales small --reps 3` | 4.7–5.2 |
| `overhead-final.jsonl`, `overhead-final-5reps.jsonl` | the same on the final commit of the package, with `--reps 3` and `--reps 5` | 9.8–12.2 |

**Noise.** Absolute latencies depend heavily on the load: with the machine at 8–14, the p99 of `kerno`'s memory writes goes from 5 to 11 ms, and those of spike-btree and fjall shoot up in some repetitions (ranges of 0.9–9.9 ms and 0.6–7.8 ms in `results-5reps.jsonl`). That is why the diagnosis rests on structural metrics (stall episodes and reasons, L0 files, size of the compactions, L1 files per compaction, split by phase), which do not depend on the CPU available, and takes the ratios between engines of the least loaded session (`results.jsonl`) as provisional.

## 3. Results

### 3.1 Comparison in the same session (`results.jsonl`, load 1.4–3.1)

p99 in µs, median of 3 (range if it exceeds 15 %). `kerno` and spike-lsm run the same algorithm: their structural statistics match (37 flushes, 24–25 compactions in `agent_memory`; identical in the other three workloads).

| Workload | Metric | spike-btree | spike-lsm | `kerno` | fjall | `kerno` / fjall | `kerno` / spike-btree |
|---|---|---:|---:|---:|---:|---:|---:|
| `agent_memory` | memory write p99 | 807 (782–3 314) | 5 276 (5 194–7 770) | **4 997** (4 502–7 471) | 454 | 11.0× | 6.2× |
| | memory write p50 | 370 | 255 | 245 | 205 | | |
| LiveSpec | commit p99 | 7 795 (2 161–12 394) | 4 168 | 4 452 | 780 | 5.7× | |
| Cauce | commit p99 | 1 148 | 409 | 412 | 360 | 1.1× | |
| `append_seq` | commit of 100, insert · update | 1 114 · 1 556 | 3 328 · 5 231 | 3 170 · 5 018 | 699 · 728 | 4.5× · 6.9× | |

### 3.2 Attribution of the `agent_memory` tail (`--trace true`, `kerno`)

Share of the total time of the 50 slowest memory-write commits (out of 5 000), median per run:

| | 3 repetitions (load ~5) | 5 repetitions (load 10–13) |
|---|---:|---:|
| Mean latency of the tail · minimum (≈ p99) | 9.2 ms · 5.0 ms | 15.3 ms · 10.4 ms |
| `wal_sync` | **99.4 %** (99.2–99.5) | **98.6 %** (96.5–99.8) |
| `wal_write` · `apply` | 0.2 % · 0.3 % | 0.2 % · 0.5 % |
| stalls (memtables, L0) | 0 % | 0 % |
| slowdown (`slowdown`) | 0 % | 0 % (up to 2.3 % in 2 of 5) |
| memtable rotation | 0 % | 0 % (0.1 % in 1 of 5) |
| rest of `make_room`, rest of the engine, outside the engine | 0 % | 0 % |
| **Attributed to named phases** | **100 %** (99.8–100) | **100 %** |
| Tail commits with background writing of the engine during the `fdatasync` | 41 of 50 (11–46) | 24 of 50 (6–46) |
| … of all measured commits | 741 of 5 000 (15 %) | 376 of 5 000 (8 %) |
| Tail commits in the first fifth of the measured phase | 25 of 50 (14–30) | 26 of 50 (18–33) |
| Pending compaction at the start of the measured phase | 78 MB (72–80); L1 with 61 MB and a target of 32 MiB | 63 MB (46–69) |
| Slowdowns during the measured phase | 0 in the 3 | 0 in 2 of 5; 11–37 in 3 of 5 |

With the machine very loaded, the part of the tail without background writing of the engine itself grows: the `fdatasync` also competes with the writing of other processes (compilations of other packages on the same disk), which is the same mechanism with another origin.

### 3.3 Structural metrics (`kerno`, `results.jsonl`; those of `results-5reps.jsonl` match)

| | `agent_memory` | LiveSpec | Cauce | `append_seq` |
|---|---:|---:|---:|---:|
| Stall episodes (memtables · L0) | 0 · 0 | 0 · 0 | 0 · 0 | 0 · 0 |
| Slowed-down commits (whole run) | 33 (1–72), all in the initial load in the lightly loaded traced runs | 0 | 0 | 0 |
| Maximum files in L0 | 9 (8–10) | 5 | 5 | 5 |
| L0 compactions · L1 files rewritten / present | 8 · 19 / 30 (63 %) | 4 · 4 / 12 (33 %) | 2 · 4 / 4 | 5 · 12 / 16 (75 %) |
| Maximum input of an L0 compaction | 74 MB (64–105) | 62 MB | 59 MB | 60 MB |
| Bytes written per compaction / per flush | 881 / 316 MB (**2.8×**) | 178 / 133 MB (1.3×) | 88 / 73 MB (1.2×) | 230 / 164 MB (1.4×) |
| Deep-level compactions with L0 in slowdown | 0 (0–3) | 0 | 0 | 0 |
| Longest flush · longest compaction | 53 ms · 236 ms | 30 ms · 162 ms | 31 ms · 170 ms | 40 ms · 172 ms |
| `wal_sync` p99 with background writing · without it | **12.0 ms · 4.0 ms** | 11.9 · 2.4 ms | 8.4 · 0.68 ms | 7.4 · 0.83 ms |
| Commits with background writing during the `fdatasync` | 29 % | 14 % | 4 % | 32 % |
| Rotations · total time · maximum | 37 · 59 ms · 11 ms | 17 · 13 ms · 7 ms | 10 · 4 ms · 0.6 ms | 23 · 6 ms · 0.4 ms |

The `wal_sync` and rotation figures cover all the commits of the run, including the initial load.

### 3.4 A/B experiments with `kerno-tuned`

Each variant was run interleaved with `kerno` in the same session; "variant / `kerno`" compares the medians of that session.

| `KERNO_TUNING` | Load | p99 memory write | p99.9 | steps/s | `wal_sync` p99 with background writing | Slowdowns · max. L0 · stalls |
|---|---|---:|---:|---:|---:|---|
| `slowdown=off` | 1.9–2.1 | 1.15× | 0.93× | 0.96× | 1.09× | 0 · 8 · 0 (`kerno`: 36 · 9 · 0) |
| `bytes_per_sync=1048576` | 2.0–2.4 | **0.61×** (4.9 → 3.0 ms) | 0.72× | 0.85× | **0.46×** (13.2 → 6.1 ms) | 153 · 12 · 0–2 (`kerno`: 55 · 10 · 0) |
| same, 5 repetitions | 3.2–13 | 1.04× | 1.00× | 0.75× | 0.58× | 119 · 12 · 1 |
| same, traced session | 4.9–5.6 | 1.60× | — | — | — | pending compaction at the start of the measured phase: 150 MB versus 78 MB |
| same, traced, 5 repetitions | 10–12 | 0.79× | — | — | — | 150 MB versus 63 MB |
| `l0_slowdown=20,l0_stop=36` | 2.4–4.7 | 1.29× | 1.73× | 0.55× | 1.01× | 0 · 8 · 0 |
| `slowdown=off,bytes_per_sync=1048576,l0_slowdown=12` | 4.7–5.6 | 0.93× | 1.05× | 0.64× | 0.54× | 0 · 12 · 1–2 (140–270 ms stalled per run) |
| same, 5 repetitions | 8–14 | 0.85× | 0.59× | 1.52× | 0.63× | 0 · 12 · 1–2 (130–910 ms stalled per run) |

Reading:

- **`bytes_per_sync`** halves, in every session, the p99 of the WAL `fdatasync` while there is background writing: syncing the SST every MiB bounds the dirty data that the WAL `fdatasync` finds ahead of it (on ext4 with `data=ordered`, the journal commit that a `fdatasync` of a growing file requires must first write the already-allocated data blocks of the other files). But syncing more often makes the compaction slower: with the initial load of `agent_memory` the debt at the start of the measured phase doubles, L0 reaches 12 files, slowdowns appear (those of the initial load) and some stall. The effect on the write p99 is positive in the least loaded session and is lost in the noise in the others.
- **Removing the throttling** does not change the tail (there was no throttling in the measured phase) and, with `bytes_per_sync`, lets L0 reach the stop: 1–2 stalls per run, which add up to 130–910 ms (the longest, 0.9 s).
- **Raising the L0 limits** makes things worse: more L0 files to read on each query (0.55× steps/s) and larger L0 compactions, without avoiding any stall because there were none.

## 4. Verdict per hypothesis

### H1 — multi-prefix flushes: **confirmed structurally; rejected as a direct cause of the tail**

Evidence for the mechanism: in `agent_memory` each L0 compaction rewrites on average 63 % of the L1 files (19 of 30 in 8 compactions; 72 % in `results-5reps.jsonl`) and up to 74–105 MB of input per compaction, although the new data (increasing ids) can only coincide with the last SST of `emb/`: the range of each L0 file, from `emb/<id viejo>` to `mem/<id nuevo>`, contains all the SSTs of the old `mem/`. Result: the compaction writes 2.8× the bytes of the flush, twice what the other three workloads write (1.2–1.4×).

Evidence against the consequence it predicted ("L0 grows, each commit sleeps 1 ms with L0 ≥ 8 and blocks at 12"): 0 stall episodes in the four workloads and in all repetitions; L0 reaches 8–10 files only during the initial load; no slowdown in the measured phase with the machine lightly loaded, and at most 2.3 % of the tail's time with the machine very loaded. H1 acts **through H3**: it multiplies the background writing with which the WAL competes.

### H2 — debt after the load: **confirmed (the debt); rejected (L0 without priority)**

The measured phase starts with 63–80 MB of pending compaction (L1 at twice its target of 32 MiB) and 6–7 files in L0; during it about 9 more compactions run (out of 24). The tail is concentrated at the beginning: half of the 50 slowest commits fall in the first fifth of the measured phase (25 of 50; 10 if they were spread evenly). When `bytes_per_sync` slows the compaction, the debt at the start doubles (150 MB). By contrast, "L0 without priority" is not observed: no compaction of a deep level was chosen with L0 in slowdown (0; at most 3 in one run), and the debt is in L1, not in L0.

### H3 — write interference: **confirmed; it is the mechanism of the tail**

- 98.6–99.4 % of the time of the tail is the WAL `fdatasync` (§3.2).
- With a flush or a compaction of the engine in progress, the p99 of `wal_sync` is 3–12× that without them in the four workloads (§3.3), and both are far above the isolated 4 KiB `fdatasync` (p99 0.42 ms in `env.json`).
- With the machine lightly loaded, 41 of the 50 commits of the tail synced with background writing of the engine, versus 15 % of all the measured commits.
- Bounding the dirty data of the SSTs (`bytes_per_sync`) halves the p99 of those syncs in every session (§3.4).
- With the machine very loaded, the writing of other processes to the same disk produces the same effect (more slow commits without background writing of the engine): the WAL `fdatasync` waits behind any pending write.

### H4 — foreground rotation: **rejected**

No commit of the tail of the lightly loaded traced runs changed memtable (at most 1 of 50 with the machine very loaded, 0.1 % of the time). Rotations are 0.7 % of the commits (37 of 5 550) and cost 59 ms in total per run; the slowest, 8–24 ms (`sync_dir` with the disk busy), can appear in the p99.9 or the maximum, not in the p99. According to §10.2 of the design, the pre-created WAL is a cut that is only justified if H4 is confirmed: it is not justified.

## 5. Consequences for WP05

- The **back-pressure** levers (rate throttling, L0 priority) do not attack the observed tail: in the measured phase there are no stalls or throttling. They remain useful as a safety net when the compaction falls behind (as with `bytes_per_sync`, which takes L0 to the stop).
- What reduces the tail is **less background writing and spread more evenly**: *flush split* (H1: avoids rewriting the old `mem/` files in each L0 compaction), partial L0 compaction and cutting by overlap with the grandparent (smaller compactions), and `bytes_per_sync` (H3), whose cost in compaction speed must be offset by the above. None of the levers in the table of §5.4 changes the part of the tail that is due to the writing of other processes.
- A lever that the table of §5.4 does not include and that attacks the mechanism directly: that the WAL `fdatasync` does not have to confirm the filesystem journal (a WAL with its size already allocated and written, like the log recycling of RocksDB). The design leaves it out of Phase 1 (§9, "WAL recycling and preallocation"); if WP05 does not reach T1, it is a candidate for the decision memo, subject to measurement. **Follow-up: implemented and measured in [§8](#8-wal-preallocation-and-recycling).**
- Always measure with `--trace true` in addition to the clean runs: the split by phase and `wal_sync_busy`/`wal_sync_quiet` distinguish a real improvement from a change in the machine's noise.
- The T1 figures of this document are provisional; the WP05 decision point must repeat the measurement in the same session with spike-btree and fjall, and the reference one is that of WP14.

## 6. Limitations

- **Shared machine.** All the absolute latencies and ratios are provisional (§2.2). The conclusions rest on structural metrics and on the split by phase, which hold with loads from 1.4 to 14.
- **A single machine and a single filesystem** (ext4 over a virtual disk). The weight of H3 depends on how the filesystem orders the journal commit and on the device latency; on a local NVMe or on another filesystem it may be different (R-S3).
- **`wal_sync_busy`** marks a sync as concurrent with background writing if there was a flush or a compaction in progress when it started or when it ended; a background write that starts and ends inside the sync is not seen. The writing that the kernel does on its own (`writeback`) is not seen either.
- **Approximate histograms.** The percentiles of the engine have an error < 25 %; the attribution uses exact sums (to the microsecond) per commit.
- **The trace** adds two calls to `stats()` per step of `agent_memory` (outside the measurement), and for that reason it is not used to compare engines. <a id="7-resultados-de-wp05"></a>

## 7. WP05 results

| Field | Value |
|---|---|
| Status | **Provisional.** The machine was shared with other packages (1-minute load between 0.7 and 7.4 during the runs; each row of `results.jsonl` carries `load1`/`load5`/`load15` and `env.json` the initial load). The reference measurement is the WP14 one, with the machine at rest |
| Date | 2026-10-28 |
| Measured code | `kerno-storage` with the condition-1 levers (§5.4 of the design) on `8f720db`; `results.jsonl`, `report.md`, `large.jsonl`, `env.json` and the `round2-*` files are from the final version, with the close levers (§7.4) |
| Data | [`spikes/storage/results/2026-10-28-f1-wp05/`](../../spikes/storage/results/2026-10-28-f1-wp05): `results.jsonl` and [`report.md`](../../spikes/storage/results/2026-10-28-f1-wp05/report.md) (the six workloads, scale `medium`, 3 repetitions, `--durability sync`, `lsm`, `kerno`, `btree` and `fjall` interleaved in the same session), `large.jsonl` (`bulk_load`, scale `large`, 1 repetition), `rerun-agent_memory.jsonl` (repetition of `agent_memory` with 5 repetitions, once more with the machine less loaded), `ab.jsonl` (lever by lever), `cond.jsonl` and `cond-final.jsonl` (conditional step), `cauce-ab.jsonl`, `round2-close.jsonl` (close of `bulk_load`, `medium` with 10 repetitions and `large` with 3), `round2-cauce-attribution.jsonl` and `round2-compaction-syncs.jsonl` (attribution of T2 of Cauce), `round2-agent_memory.jsonl` (`bytes_per_sync` versus T1), `env.json` |

### 7.1 Final default values

The diagnosis (§4) did not reject any lever outright, so all are on by default except two:

- `l0_priority = false`: H2 did not show the "L0 without priority" effect (§4). Turning it on did not change the p99 (2.73 ms with the lever versus 2.55 ms without it, median of 5 and 35 runs respectively, within the noise).
- `max_subcompactions = 1` and `max_background_compactions = 1`: the conditional step (§7.3) is implemented and tested, but it did not demonstrate an improvement beyond the noise.

The rest: `flush_split`, `l0_metric = Depth`, `l0_partial`, `l0_trivial_move`, `slowdown = Rate { 16 MiB/s }`, `bytes_per_sync = 1 MiB`, `heap_merge_threshold = 8`, `close_subcompactions = 8` (only for the compactions of the close, which in addition do not sync by stretches; §7.4), `max_compaction_bytes` 25 × and `grandparent_overlap_bytes` 10 × the target size; close `Bounded { 256 MiB, 200 ms }`. The `M0` profile turns them all off.

### 7.2 Effect of each lever on T1 (`ab.jsonl`, `agent_memory`, `medium`)

`memory_write_p99_us`, median of the runs (each variant with 5; the default values with 35 spread over the session; 1-minute load 0.7–2.0):

| Variant | p99 (µs) | Versus the default value |
|---|---:|---:|
| default values | 2 554 | — |
| `m0` profile (all levers off) | 5 087 | 2.0× worse |
| `bytes_per_sync = none` | 3 822 | 1.5× worse |
| `slowdown = sleep:1000` | 2 806 | +10 % (noise) |
| `flush_split = false` | 2 718 | +6 % (noise) |
| `l0_priority = true` | 2 728 | +7 % (noise) |
| `l0_partial = false` | 2 570 | 0 % |
| `l0_metric = files` | 2 257 | −12 % (noise) |

The levers together halve the p99 of M0. Of them, only `bytes_per_sync` stands out from the noise by itself (consistent with H3, §4); the rest acts jointly or below the resolution of these measurements, and the dispersion between runs of the default value (2.0–3.8 ms) is larger than the difference between them.

### 7.3 T1 and the decision rule

T1 target: `memory_write_p99_us` ≤ 2.0× fjall **and** ≤ 1.5× spike-btree, in the same session.

| Measurement | `kerno` | fjall | btree | `kerno` / fjall | `kerno` / btree | Load (1 min) |
|---|---:|---:|---:|---:|---:|---|
| `results.jsonl`, 3 repetitions (final version) | 2 480 | 795 | 1 342 | 3.12× | 1.85× | 1.1–1.2 |
| `rerun-agent_memory.jsonl`, 5 repetitions (repetition with less load) | 2 748 | 1 059 | 1 376 | 2.60× | 2.00× | 1.3–1.7 |
| `cond-final.jsonl`, 15 repetitions | 2 585 | 974 | 1 340 | 2.65× | 1.93× | 0.9–1.7 |
| `cond-final.jsonl`, `max_subcompactions=4,max_background_compactions=2` | 2 372 | 974 | 1 340 | 2.43× | 1.77× | 1.2–1.7 |

(The M0 p99 in the same session: `lsm` 5 472 µs, that is 6.0× fjall and 4.3× btree.)

1. With the default values, T1 **is not met** and the deviation is > 25 % (2.65× versus 2.0× is +33 %; 1.93× versus 1.5× is +29 %). This load was repeated once with the machine less loaded (`rerun-agent_memory.jsonl`): same result (+30 % and +33 %), so it is not due to the noise of that session.
2. Following the rule, the conditional step was implemented within WP05: subcompactions (the L0 compaction is split at the boundaries of the L1 files, `std::thread::scope`, outputs installed in **one** MANIFEST edit) and `max_background_compactions = 2` (`being_compacted` set under the MANIFEST lock, `pick` excludes those entries, exclusion of output ranges in the same level, `compaction::apply` checks it again), with the tests `subcompactions_install_single_manifest_write` and `concurrent_compactions_preserve_model`.
3. It was measured again (`cond.jsonl` with each half separately and both together, `cond-final.jsonl` with both together and 15 repetitions interleaved with the other engines): the p99 drops to 2 372 µs (−8 %, 2.43× fjall and 1.77× btree). That improvement falls within the dispersion (in `cond.jsonl` the medians of the three variants are 2 211–3 035 µs versus 2 560 without them, with no stable order), so **T1 is still not met** and the conditional step shows no effect.
4. **Decision: GO not granted; candidate for NO-GO, provisional until WP14.** [f1-decision-cond1.md](f1-decision-cond1.md) is written and the owner is pointed to it. The sequential execution continues and the v1 format does not depend on this decision.

### 7.4 T2–T5 (`report.md`, `large.jsonl`; median of 3 repetitions)

| Target | Measure | `kerno` | Reference | Ratio | Target | |
|---|---|---:|---|---:|---|---|
| T2 | LiveSpec `changeset_commit_p99_us` | 3 021 | `lsm` 4 362 | 0.69× | ≤ 1.2× | met |
| T2 | `append_seq` insert `append_commit100_p99_us` | 1 888 | `lsm` 4 313 | 0.44× | ≤ 1.2× | met |
| T2 | `append_seq` update `update_commit100_p99_us` | 3 774 | `lsm` 6 070 / fjall 1 425 | 0.62× / 2.65× | ≤ 1.2× `lsm`, ≤ 2× fjall | `lsm` yes, fjall **no** |
| T2 | `history_sim` `rev_commit100_p99_us` | 5 239 | `lsm` 8 196 | 0.64× | ≤ 1.2× | met |
| T2 | Cauce `step_commit_p99_us` | 1 117 | `lsm` 484 | 2.31× | ≤ 1.2× | **not met** (attributed to `bytes_per_sync`; conflict with T1) |
| T3 | LiveSpec `symbol_get_p99_us` | 31.6 | `lsm` 29.6 | 1.07× | ≤ 1.2× | met |
| T3 | LiveSpec `module_scan_p99_us` | 617 | `lsm` 666 / btree 306 | 0.93× / 2.02× | ≤ 1.2× `lsm`, ≤ 1.5× btree | `lsm` yes, btree **no** (M0 neither: `lsm` 2.18× btree) |
| T3 | Cauce `step_read_p99_us` | 77.6 | `lsm` 73.5 | 1.06× | ≤ 1.2× | met |
| T3 | `recall_p99_us` / `recent_scan_p99_us` | 55.4 / 65.1 | `lsm` 56.8 / 68.5 | 0.98× / 0.95× | ≤ 1.2× | met |
| T4 | `bulk_load` `close_ms`, `medium` | 204 (192–222) | `lsm` 726, fjall 70 | — | ≤ 250 ms | met |
| T4 | `bulk_load` `close_ms`, `large` (1 repetition) | 376 | `lsm` 3 401, fjall 970 | — | ≤ 1 000 ms | met |
| T5 | throughput: LiveSpec / Cauce / `append_seq` / `agent_memory` / `history_sim` / `bulk_load` | 791 / 4 202 / 183 / 1 368 / 78.7 / 209 | `lsm` 774 / 4 485 / 174 / 1 276 / 81.9 / 187 | 1.02× / 0.94× / 1.05× / 1.07× / 0.96× / 1.12× | ≥ 0.9× | met |

1-minute load of `results.jsonl`: 1.1–1.5; of `large.jsonl`: 1.2–1.6.

Reading:

- **T4: met with the close levers.** The first measurement (587 ms `medium`, 1 270 ms `large`) stayed in the **mandatory** stretch of the close: `bulk_load` loads in random order, so each L0 file spans all the keys and each one that goes down rewrites all of L1. Breakdown of a `medium` close before the changes below (ms, load ~1): ~25 waiting for the flush in progress of the background thread, ~10 dumping the memtable, ~135 compacting ~108 MB of input (6 L0 files and 8 L1 files) and ~50 deleting those inputs (the filesystem takes ~0.5 ms per MB to free recently written files, also in parallel: 56 versus 58 ms for 112 MB in a separate test). Four changes, all only in the close, without relaxing the mandatory floor (the L0 metric still ends below `l0_trigger` after each clean close):
  1. The mandatory stretch takes, with `l0_partial`, only the oldest files needed to bring the metric below `l0_trigger` (3 of 6 in `bulk_load`; ~90 MB of input instead of ~108). Test: `close_floor_takes_only_the_oldest_files_it_needs`.
  2. The compactions of the close are split into subintervals (`close_subcompactions = 8`); the background ones keep `max_subcompactions = 1`, because in the A/B of `agent_memory` that value at 4 raised the p99 from 2 609 to 3 076 µs (`round2-agent_memory.jsonl`). Test: `close_compaction_uses_close_subcompactions`.
  3. The SSTs of those compactions do not do the intermediate `sync_data` of `bytes_per_sync`, which exist so that background writing does not compete with the WAL `fdatasync`; on close no WAL is writing.
  4. The time budget no longer lets a compaction start that, at the pace of the previous ones of the same close (deletion included), would finish after `max_duration`: before, a compaction started at 198 ms took a close to 266 ms.

  Result: `medium` 204 ms median (192–222, `results.jsonl`) and 210 ms (200–230, 10 repetitions, `round2-close.jsonl`), against a target of 250 ms; `large` 376 ms (`large.jsonl`) and 358 ms (353–382, 3 repetitions). With the load of 1.1–1.6 of these sessions there is ~40 ms of margin; the reference measurement is still the WP14 one.
- **T2 of Cauce: attributed, and in conflict with T1.** With the machine less loaded (load 0.9–1.8; `round2-cauce-attribution.jsonl`, 7 repetitions per variant, medians of `step_commit_p99_us`): `lsm` 537 µs, `kerno` 1 124 µs (2.09×), `kerno-tuned[m0]` 540 µs, `bytes_per_sync=none` 529 µs, `slowdown=off` 1 125 µs, `flush_split=false` 1 057 µs. The regression is due to `bytes_per_sync`; the rate throttling and `flush_split` have no influence. Within `bytes_per_sync`, it is the intermediate `sync_data` calls of the **compactions**: removing them only from those (`round2-compaction-syncs.jsonl`, 6–7 interleaved repetitions, load 1.3–1.5) leaves Cauce at 620 µs versus 525 of `lsm` (1.18×, within the target), but raises the p99 of `agent_memory` from 2 304 to 3 382 µs (ranges 1 990–2 712 and 2 662–4 223, not overlapping), that is, it worsens T1, which is already the main miss. Higher values of the parameter give the same trade-off (`round2-agent_memory.jsonl`). That is why `bytes_per_sync = 1 MiB` is left in the background compactions and the choice between T1 and T2 of Cauce is left for the owner ([f1-decision-cond1.md](f1-decision-cond1.md)); the switch used for that test was temporary and is not in the code.
- **T3 of `module_scan`.** Against `lsm` it is met (0.93× in `results.jsonl`; 1.10× in `round2-cauce-attribution.jsonl`): the 1.24× of the first measurement was noise from a session with load 5+. Against btree it stays at 2.02×, but M0 is the same (`lsm` 2.18× btree in the same session; `kerno-tuned[m0]` 626 µs versus 684 of `kerno` in `round2-cauce-attribution.jsonl`): it is the cost of a scan that merges several levels of an LSM versus a B-tree, not of a WP05 lever.
- **T5.** `kerno` stays at 0.94–1.12× of `lsm`.

### 7.5 Package acceptance

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features --release` pass.
- Bounded close: the tests of `tests/close_budget.rs` (`l0_bounded_across_100_short_sessions`: depth of L0 < `l0_trigger` and ≤ 2 × `l0_trigger` files after each close; `space_bounded_across_100_short_sessions`; `budgeted_close_crash_at_every_point`).
- **T1 not met; T4 met** (`medium` 204 ms, `large` 376 ms) after the close levers. T2 of Cauce (attributed to `bytes_per_sync`, in conflict with T1), `module_scan` against btree (as in M0) and `append_seq` update against fjall remain unmet. The condition-1 decision is in [f1-decision-cond1.md](f1-decision-cond1.md) for the owner. <a id="8-preasignación-y-reciclado-del-wal"></a>

## 8. WAL preallocation and recycling

Follow-up of WP05: lever 1 of [f1-decision-cond1.md §3](f1-decision-cond1.md#3-next-levers), which attacks the mechanism of H3 instead of reducing the background writing. **It does not change the status of the decision**: T1 is still not met (§8.3).

| Field | Value |
|---|---|
| Status | **Provisional**, like the rest of the document: shared machine, 1-minute load 1.1–2.0 in the engine runs and 1.6–5.6 in those of the micro-benchmark (each series carries its `uptime-*.txt`). The reference measurement is still the WP14 one, with the machine at rest |
| Date | 2026-10-28 |
| Measured code | `kerno-storage` with `Config::wal_preallocate` ([DESIGN.md](../../crates/kerno-storage/DESIGN.md), "Preallocated and recycled WAL"), on `b745205`. The first implementation (`*-v1-fullzero.*`) zeroed the whole recycled WAL; the final one only the written part |
| Data | [`spikes/storage/results/2026-10-28-f1-wal/`](../../spikes/storage/results/2026-10-28-f1-wal): `walsync.jsonl` and `walsync-load5.jsonl` (micro-benchmark), `agent_memory.jsonl` (10 repetitions), `results.jsonl` and `report.md` (six workloads, `medium`, 5 repetitions), `large.jsonl`, `control-*.jsonl` (controls of the switch), `*-v1-fullzero.*` (first implementation), `env.json`, `uptime-*.txt` |

### 8.1 What it does

The WAL is written as `cabecera ‖ ceros` of the size of the *write buffer* before its first frame, and a WAL already flushed is overwritten with zeros and kept as a spare (`WAL.spare`) for the next rotation. The `fdatasync` of a frame overwrites blocks already allocated and written, so it does not have to confirm the size of the file. **No format change**: the zeros are the padding of v1.md §6.1 and WAL-R4; recycling without zeroing would need the reserved `incompat` bit and would change WAL-R4/REC-10a ([ADR-0013, point 15](../adr/0013-format-v1-and-engine-protocol.md)). Switch: `Config::wal_preallocate` (default `false`, because the micro-benchmark at rest shows no gain; it is turned on with `KERNO_TUNING=wal_preallocate=true`; the `M0` profile turns it off). Tests: `wal.rs` (17 WAL tests, 8 new), `tests/wal_prealloc.rs` (11) and the crash, corruption and shared failure sweeps, which now go through preallocated and recycled WALs (the scene of the `every_point` sweep recycles WALs: `the_crash_sweep_scenario_recycles_logs`).

### 8.2 `fdatasync` micro-benchmark (`spike-bench walsync`)

Each variant adds 512 B per iteration and measures the following `sync_data` (5 000 per run, variants interleaved): `grow` (the file grows), `setlen` (a prior `set_len`: final size, blocks not allocated) and `prewritten` (zeros written and synced up to the final size). "With background writing" adds a thread that writes bursts of 16 MiB with `sync_data` every MiB and a 30 ms pause in the same directory. Median of the runs (in parentheses, minimum–maximum), in µs:

| Session | Background | Variant | p50 | p99 |
|---|---|---|---:|---:|
| load 1.6–1.8, 9 repetitions (`walsync.jsonl`) | no | `grow` | 148 | 435 (326–532) |
| | no | `setlen` | 106 | 379 (299–705) |
| | no | `prewritten` | 149 | 360 (290–607) |
| | yes | `grow` | 164 | 1 629 (1 533–1 732) |
| | yes | `setlen` | 109 | 1 487 (1 318–1 669) |
| | yes | `prewritten` | 164 | 1 649 (1 341–1 798) |
| load 4.9–5.9, 7 repetitions (`walsync-load5.jsonl`) | no | `grow` | 156 | 3 987 (475–4 131) |
| | no | `setlen` | 110 | 723 (309–5 276) |
| | no | `prewritten` | 159 | 765 (403–5 304) |
| | yes | `grow` | 181 | 3 957 (1 871–7 963) |
| | yes | `setlen` | 127 | 2 128 (1 507–3 879) |
| | yes | `prewritten` | 186 | 2 458 (1 396–3 148) |

Reading: **with the machine at rest the micro-benchmark shows no gain** (p99 of `grow` and `prewritten` indistinguishable, also with the background writing of its own); with the load of other processes (load ≥ 5) the p99 of `grow` is 3–5× that of `prewritten`, but with ranges that come to overlap (one run of `prewritten` reached 5.3 ms). The gain of the isolated `fdatasync` therefore depends on the contention with foreign writing, exactly the H3 scenario, and it is not reproduced with the background writing of the benchmark itself. `setlen` has a lower p50 (~110 versus ~150 µs) and a p99 that is not distinguishable from `prewritten`; it was not tried in the engine.

### 8.3 `agent_memory` and T1

Sessions of the A/B (`kerno` with the lever, `kerno-tuned` with `wal_preallocate=false`; `agent_memory` `medium`, `Sync`, interleaved repetitions, median and range of `memory_write_p99_us`, µs):

| Session | Code | Reps | `kerno` p99 | without lever p99 | `kerno` p50 | without lever p50 | fjall p99 | btree p99 | Load |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| `agent_memory-v1-fullzero.jsonl` | first impl. | 7 | 1 754 (1 449–3 017) | 2 370 (2 212–2 621) | 217 | 361 | 814 | 1 247 | 1.7–4.0 |
| `results-v1-fullzero.jsonl` | first impl. | 5 | 2 783 (2 312–3 133) | 2 548 (1 679–3 027) | 231 | 401 | 1 132 | 1 182 | 1.2–1.7 |
| `agent_memory.jsonl` | final | 10 | 1 958 (1 367–2 738) | 2 563 (1 884–2 771) | 197 | 331 | 1 032 | 1 068 | 1.1–1.5 |
| `results.jsonl` | final | 5 | 2 437 (1 657–2 781) | 2 548 (2 347–2 841) | 215 | 364 | 1 116 | 1 127 | 1.4–1.6 |
| `control-tuned-*.jsonl` | final | 6 | 2 416 (2 048–2 877) | 2 030 (1 688–2 421) | 214 | 326 | — | — | 1.1 |

(In the control, `kerno-tuned[wal_preallocate=true]` gives 2 430 µs and 203 µs of p50: the `Tuned` profile adds nothing by itself.)

With the final code, the 15 repetitions of the two sessions that include fjall and btree (`agent_memory.jsonl` and `results.jsonl`):

| | p99 (µs), median (quartiles) | p50 (µs) | versus fjall (target ≤ 2.0×) | versus btree (target ≤ 1.5×) |
|---|---:|---:|---:|---:|
| `kerno` with the lever | 2 173 (1 708–2 466) | 206 | **2.08×** | **1.93×** |
| `kerno` without the lever | 2 548 (2 347–2 763) | 348 | 2.44× | 2.27× |
| fjall | 1 044 (657–1 147) | 286 | — | — |
| btree | 1 123 (1 022–1 225) | 445 | — | — |

1. **The write p50 drops 40 % in a stable way** (206 versus 348 µs; 197–231 versus 326–401 µs in all the series and in the control), and the throughput of `agent_memory` rises between 7 and 28 % (`steps_per_s`, depending on the series): it is the expected effect of not confirming the size on each commit. In `cauce` the p50 does not change (169 versus 162 µs in the control).
2. **The p99 improves on average by 15 % (8 % with the 27 repetitions of the three series with the lever), but not in a stable way**: in the five series the ranges with and without the lever overlap, and in the control it is worse (2 416 versus 2 030 µs). The first implementation, which zeroed the whole WAL, worsened the p99 in one session (2 783 versus 2 548): writing zeros competes with the `fdatasync` like any other background write, and that is why the recycling only rewrites the written part.
3. **T1 is not met**: 2.08× fjall (target ≤ 2.0×) and 1.93× btree (≤ 1.5×); without the lever, 2.44× and 2.27×. The lever brings T1 closer by 15 %, but the ratio with btree is still 29 % above the target; what remains is the tail of the background writing that no WAL lever touches (§4).

### 8.4 T2–T5 (`report.md`; median of 5 repetitions, `Sync`, 1-minute load 1.1–2.0)

The ratios are against `lsm` (M0) of the same session; "without lever" is `kerno-tuned[wal_preallocate=false]`.

| Target | Measure | `kerno` | without lever | `lsm` | fjall | btree | `kerno`/`lsm` | Target | |
|---|---|---:|---:|---:|---:|---:|---:|---|---|
| T2 | LiveSpec `changeset_commit_p99_us` | 1 605 | 2 658 | 4 342 | 1 658 | 2 793 | 0.37× | ≤ 1.2× | met |
| T2 | `append_seq` `append_commit100_p99_us` | 1 508 | 2 043 | 3 803 | 973 | 1 692 | 0.40× | ≤ 1.2× | met |
| T2 | `append_seq` `update_commit100_p99_us` | 2 339 | 3 654 | 6 054 | 1 378 | 1 635 | 0.39× (1.70× fjall) | ≤ 1.2× `lsm`, ≤ 2× fjall | **both met** (WP05: 2.65× fjall) |
| T2 | `history_sim` `rev_commit100_p99_us` | 4 506 | 5 698 | 8 331 | 2 261 | 13 959 | 0.54× | ≤ 1.2× | met |
| T2 | Cauce `step_commit_p99_us` | 998 | 1 094 | 504 | 514 | 1 320 | 1.98× | ≤ 1.2× | **not met** (WP05: 2.31×; the p50 does not change: 173 versus 171 µs) |
| T3 | LiveSpec `symbol_get_p99_us` | 31.3 | 30.4 | 28.1 | 21.7 | 29.6 | 1.11× | ≤ 1.2× | met |
| T3 | LiveSpec `module_scan_p99_us` | 711 | 627 | 540 | 398 | 295 | 1.32× (without lever 1.16×) | ≤ 1.2× `lsm`, ≤ 1.5× btree | **not met with the lever** (see below) |
| T3 | Cauce `step_read_p99_us` | 76.7 | 83.6 | 74.7 | 75.2 | 54.0 | 1.03× | ≤ 1.2× | met |
| T3 | `recall_p99_us` / `recent_scan_p99_us` | 53.2 / 59.3 | 54.4 / 65.6 | 55.7 / 61.1 | 41.5 / 64.4 | 34.6 / 51.5 | 0.95× / 0.97× | ≤ 1.2× | met |
| T4 | `bulk_load` `close_ms`, `medium` | 219 | 223 | 731 | 77 | 11 | — | ≤ 250 ms | met |
| T4 | `bulk_load` `close_ms`, `large` (3 repetitions, `large.jsonl`) | 409 | 449 | 3 446 | 982 | — | — | ≤ 1 000 ms | met |
| T5 | throughput: LiveSpec / Cauce / `append_seq` / `agent_memory` / `history_sim` / `bulk_load` | 925 / 4 293 / 205 / 1 553 / 84.2 / 209 | 810 / 4 326 / 176 / 1 268 / 72.9 / 203 | 873 / 4 373 / 179 / 1 318 / 80.3 / 189 | | | 1.06× / 0.98× / 1.15× / 1.18× / 1.05× / 1.11× | ≥ 0.9× | met |

- **`module_scan_p99_us` (T3).** With the lever it is 667–711 µs in three sessions versus 604–627 without it and 540–577 of `lsm` (`control-livespec-*.jsonl`: 667 and 691 versus 604 µs, overlapping ranges 591–724 and 509–728; the p50 is the same, 161–167 µs). It is a difference of ~10–14 % that falls within the dispersion and that has not been explained (the reads do not use the WAL); it is not ruled out that it is real. In WP05 it was 0.93× `lsm`.
- **Cauce (T2)** goes from 2.31× to 1.98× `lsm`: an improvement, but the target (≤ 1.2×) is still not met and its attribution to `bytes_per_sync` ([§7.4](#74-t2t5-reportmd-largejsonl-median-of-3-repetitions)) does not change.
- **LiveSpec, `append_seq` and `history_sim` improve 30–40 % in p99** (and the LiveSpec p50 by 16 %: 381 versus 457 µs) with the lever.

### 8.5 Costs

| Measure (`bulk_load` `medium`, median of 5) | with the lever | without it |
|---|---:|---:|
| `write_amp` | 3.97 | 2.97 |
| `reopen_ms` | 17.0 (16.3–22.4) | 3.71 |
| `recovery_ms` | 28.9 | 6.36 |
| `cauce` `recovery_ms` | 33.2 | 8.89 |
| `history_sim` `space_amp` | 2.06 (2.06–2.85) | 1.68 |
| `append_seq` `space_amp` | 2.24 | 2.04 |

- **Writing:** the WAL is written twice (the frame and the zeros of the recycling, only the written part), hence +1.0 in `write_amp` of `bulk_load` (+1.0 in `large`: 6.49 versus 5.52).
- **Opening:** `reopen_ms` rises 13 ms and `recovery_ms` 22 ms (creating the recovery WAL with the size of the *write buffer*, eight `sync_data` of 1 MiB, and reading its zeros on replay; it was not measured how much corresponds to each thing). A handle that is opened for few writes pays that time.
- **Space:** up to one more WAL (the spare) while the handle is open; the files are deleted on close (`space_amp_closed` the same: 1.04).
- **Full disk:** the space of the WAL is taken when it is created, not when a commit is made; a commit that fits in the preallocated space needs no new space. A failure while preparing the spare does not poison the handle (`a_failure_preparing_the_spare_is_not_an_engine_failure`).

### 8.6 Limits

- Same machine and same filesystem as the rest (ext4 over a virtual disk); the `fdatasync` gain depends on how the filesystem orders the journal commit and on the contention with other processes (R-S3).
- The micro-benchmark at rest does not reproduce the gain of the micro-benchmark under load; the lever is left off by default because the micro-benchmark shows no gain at rest; the evidence in favour of turning it on is that of the engine (p50 −40 %, T2 −30–40 %) and it is pending confirmation by the WP14 reference measurement before the default value is changed. **Decided in [§9.1](#91-wal_preallocate-does-it-help-it-stays-off)** with 15 more interleaved pairs, plus Cauce and LiveSpec: it stays off.
- The p99 of T1 improves below the dispersion between runs (quartiles 1 708–2 466 µs with the lever): the 15 % improvement is a median, not a difference that separates the distributions.

## 9. Follow-up on condition 1: `wal_preallocate`, incremental flush and dynamic levels

Follow-up of [§8](#8-wal-preallocation-and-recycling) and of levers 4 and 5 of [f1-decision-cond1.md §3](f1-decision-cond1.md#3-next-levers). **It does not change the status of the decision**: T1 is still not met (§9.5).

| Field | Value |
|---|---|
| Status | **Provisional**, like the rest of the document: shared machine with other packages that were compiling and testing (each series carries the 1-minute load of each row and [`README.txt`](../../spikes/storage/results/2026-10-28-f1-cond1b/README.txt) summarises it). The reference measurement is still the WP14 one, with the machine at rest |
| Date | 2026-10-28 |
| Measured code | `kerno-storage` on `1764311` with `File::start_writeback` / `wait_writeback`, `CompactionConfig::{bytes_per_writeback, dynamic_base_level, file_picker}` ([DESIGN.md](../../crates/kerno-storage/DESIGN.md)) |
| Data | [`spikes/storage/results/2026-10-28-f1-cond1b/`](../../spikes/storage/results/2026-10-28-f1-cond1b) |

### 9.1 `wal_preallocate`: does it help? It stays off

`agent_memory` `medium`, `Sync`, `kerno` (off, the default value) versus `kerno-tuned[wal_preallocate=true]`, interleaved with fjall and btree. Two series of the same order, the second with the order of the engines inverted to remove the effect of the order within the repetition; 15 repetitions in total, median of `memory_write_p99_us` (µs):

| Series | Load | `kerno` off | with the lever | paired difference (median) | lever better in | fjall | btree |
|---|---:|---:|---:|---:|---:|---:|---:|
| A (7 rep., `kerno` first) | 2.6–6.1 | 4 927 (3 928–9 593) | 5 628 (3 363–8 126) | −12 % | 4 of 7 | 2 026 | 1 916 |
| B (8 rep., `kerno-tuned` first) | 1.4–1.8 | 3 873 (3 256–4 952) | 3 448 (3 006–4 067) | −15 % | 6 of 8 | 949 | 1 518 |
| A + B | 1.4–6.1 | 4 016 (quartiles 3 828–4 936) | 3 582 (3 363–5 628) | **−12 %** | **10 of 15** | 1 050 | 1 625 |

The write p50 drops 44 % (570 → 322 µs) in the 15 pairs, as in [§8.3](#83-agent_memory-and-t1). With the lever T1 stands at 3.41× fjall and 2.20× btree (3.83× and 2.47× without it): **it is not met in either form**. The other workloads that depend on the WAL (`livespec` and `cauce`, 7 repetitions interleaved with `lsm`, load 1.3–2.0, `walprealloc-t3.jsonl`):

| Measure | `kerno` off | with the lever | `lsm` |
|---|---:|---:|---:|
| LiveSpec `changeset_commit_p99_us` | 3 119 (2 468–5 329) | 2 513 (2 095–3 023) | 6 410 |
| LiveSpec `module_scan_p99_us` | 911 (779–945) | 977 (818–1 008) | 856 |
| Cauce `step_commit_p99_us` | 1 321 (1 213–1 535) | 1 499 (1 342–1 716) | 686 |
| Cauce `step_commit_p50_us` | 263 | 267 | 264 |

**Decision: `wal_preallocate` stays at `false`.** In favour: the p50 (−44 %), a median improvement of the p99 of T1 of between 12 and 15 % in the three independent series (§8.3 and these two) and the commit p99 of LiveSpec (−19 %). Against: the improvement of the p99 of T1 does not separate the distributions (10 of 15 pairs; overlapping ranges and quartiles) and T1 is not met; the p99 of Cauce (T2) worsens 13 % (also with overlapping ranges) and `module_scan` 7 %; and the cost of [§8.5](#85-costs) includes +1.0 of `write_amp` in `bulk_load` (≈ 3.2 → 4.2 against the limit of 3.6 of T7) and +13 ms of opening. A lever that does not meet the target that motivated it, that takes T7 out of its target and whose effect is of the order of the noise, is not a reasonable default value. The evidence in favour of turning it on (p50, T2 of LiveSpec) is not a target of T1. <a id="92-palanca-5-volcado-incremental-de-las-sst-con-sync_file_range"></a>

### 9.2 Lever 5: incremental flush of the SSTs with `sync_file_range`

**Implementation.** `File::start_writeback(offset, len)` and `File::wait_writeback(offset, len)` (by default they do nothing; `StdVfs` on Linux: `sync_file_range` with `SYNC_FILE_RANGE_WRITE` and with `WAIT_BEFORE | WAIT_AFTER`; on other platforms, nothing). `SimVfs` models them as mutating operations with no effect (`OpKind::StartWriteback` and `WaitWriteback`): they count for the crash and injected-failure points and appear in the traces, but **they make nothing durable** (what is unsynced is lost on a power cut exactly as without them: `an_sst_writeback_request_never_makes_data_durable`). Option: `CompactionConfig::bytes_per_writeback`.

**Micro-benchmark first** (`spike-bench walsync --writeback true`; p99 of the `fdatasync` of 512 B that grows, or of an already written file, while another thread writes bursts of 16 MiB in the same directory; median of 6–7 interleaved repetitions, load 1.7–3.0, µs):

| How the burst bounds its dirty data | file that grows (default WAL) | preallocated file |
|---|---:|---:|
| without background writing | 460 | 458 |
| nothing until the end of the burst (M0) | 9 880–10 519 | 9 814–10 559 |
| `sync_data` every MiB (`bytes_per_sync`) | 2 494–2 724 | 2 234–2 482 |
| `sync_file_range(WRITE)` every MiB, without waiting | 10 117–10 166 | 10 260–10 707 |
| same, and waiting for the previous range (two ranges in flight) | 5 222 | 4 413 |
| same, and waiting for the same range (one in flight) | 3 608 | 2 867 |

Requesting the flush without waiting **does not change the tail**: it only advances the burst and the WAL `fdatasync` still waits behind the whole queue of the device. What bounds the tail is not leaving more than one range in flight; with that wait, `sync_file_range` stays at ~1.3× the `fdatasync` per MiB. In the micro-benchmark **no variant improves on `bytes_per_sync`**.

**In the engine** the SST writer requests the flush of each MiB and waits for that same range (the "one range in flight" variant), instead of syncing (`bytes_per_sync = None`). A/B against the default value of that time (`bytes_per_sync = 1 MiB`), interleaved, 6 repetitions per series, median (range) and paired difference:

| Series | Load | Measure | default | `bytes_per_writeback` | paired difference | better in |
|---|---|---|---:|---:|---:|---:|
| 1 MiB | 1.7–1.9 | `agent_memory` p99 | 3 763 (3 168–4 264) | 2 696 (2 134–3 318) | **−24 %** | 6 of 6 |
| 1 MiB (another session) | 1.2–2.0 | `agent_memory` p99 | 3 358 (2 910–4 575) | 2 982 (2 550–3 719) | −10 % | 4 of 6 |
| 1 MiB (another session) | 1.6–1.8 | Cauce `step_commit_p99_us` | 1 423 (1 200–1 584) | 1 148 (953–1 251) | **−21 %** | 6 of 6 |
| 256 KiB | 1.3–1.8 | `agent_memory` p99 | 3 167 (2 243–3 836) | 2 563 (2 195–3 736) | −4 % | 4 of 6 |
| 256 KiB | 1.5–1.8 | Cauce `step_commit_p99_us` | 1 327 (997–1 457) | 853 (839–976) | **−34 %** | 6 of 6 |

The value of 1 MiB improves the p99 of T1 between 10 and 24 % per series (median of the 12 pairs: −18 %, better in 10 of 12) and that of Cauce between 21 % (1 MiB) and 34 % (256 KiB), better in 6 of 6 repetitions in both series: it **resolves half of the T1 / T2 conflict of Cauce** described in [§7.4](#74-t2t5-reportmd-largejsonl-median-of-3-repetitions), because it removes the journal barrier of the intermediate `sync_data` of the compactions without leaving the writing unbounded. It does not meet T1 by itself (3 358–3 763 → 2 696–2 982 µs). <a id="93-palanca-4-niveles-dinámicos-y-prioridad-por-bytes"></a>

### 9.3 Lever 4: dynamic levels and byte priority

**`dynamic_base_level`.** With L1 empty, no compaction in flight towards it, and L2 small enough for an L0 compaction with `l0_trigger` files to fit in `max_compaction_bytes`, L0 is compacted directly into L2: the data is rewritten once instead of once per level while the database is small. The condition that L1 be empty is not a detail: a file in L1 would be older than what bypasses it, and `compaction::apply` rejects it as an internal `Corruption` ([DESIGN.md](../../crates/kerno-storage/DESIGN.md)). A clean close does **not** skip L1 (merging L0 with all of L2 costs much more than with L1; without that exception the close of `bulk_load` `medium` rose 6–17 % in the A/Bs).

**`file_picker = MinOverlap`** (byte priority: from the level above its target, the file that rewrites the fewest bytes of the level below per byte that goes down is compacted).

A/B against the default value of that time, interleaved, 6 repetitions, `agent_memory` `medium`:

| Variant | Load | p99 default | p99 with the lever | paired difference | better in | compactions · MB written per compaction (default → lever) |
|---|---|---:|---:|---:|---:|---|
| `dynamic_base_level` | 1.3–1.8 | 3 591 (2 734–4 284) | 2 527 (2 380–2 900) | **−30 %** | 6 of 6 | 22 → 17 · 561 → 523 (−7 %) |
| `file_picker = MinOverlap` | 1.5–1.8 | 3 718 (2 718–4 452) | 3 039 (2 464–3 514) | −17 % | 6 of 6 | 23 → 18 · 611 → 467 (−23 %) |
| `bytes_per_writeback` + `dynamic_base_level` | 1.4–1.8 | 3 474 (2 804–4 059) | **2 240 (1 894–2 562)** | **−31 %** | 6 of 6 | 23 → 14 · 611 → 530 (−13 %) |

The write amplification of `bulk_load` does not change (3.0–3.3 in all the variants): the load in random order rewrites all of L1 for each L0 file, and these levers do not touch that. In `agent_memory` the compaction writing drops between 7 and 23 %, and the flush bytes (318 MB) are the same.

**Costs.**

- `dynamic_base_level`: the commit p99 of LiveSpec rises 54 % (5 118 versus 3 332 µs with the lever off; 16 and 8 repetitions; `attr-livespec.jsonl`): longer L0 → L2 compactions. It stays at 0.74–0.87× `lsm` (target ≤ 1.2×). In the isolated A/B the close of `bulk_load` `medium` rose 6 % (299 → 325 ms) and 17 % together with `bytes_per_writeback` (297 → 318 ms); with the close exception above, the final session measures it at 179 ms versus 252 with the previous values (§9.4; the isolated exception was not measured).
- `MinOverlap` **is not the default value** (it was not measured with the following correction, so its figures in the table have not been repeated). As measured, it delayed precisely the files that contain the most garbage and failed the shared test `space_is_reclaimed_after_range_deletes` with the `Inline` profile (3 375 523 bytes on disk after the reload of cycle 6, limit 3 172 072). Now the picker first passes the files with range-delete fragments, the test passes and `Inline` includes `MinOverlap`. The space-amplification limit foreseen by [f1-decision-cond1.md §3](f1-decision-cond1.md#3-next-levers) still applies.

### 9.4 New default values and T1–T5

The default values of `CompactionConfig` that change: `bytes_per_sync = None`, `bytes_per_writeback = Some(1 MiB)`. `dynamic_base_level = false` (off by default until the owner decides: T1 improves, the commit p99 of LiveSpec worsens 1.43×), `file_picker = RoundRobin` and `wal_preallocate = false` do not change. The `M0` profile turns the three off. On a platform without `sync_file_range` the default value leaves the SSTs unbounded (it only affects the tail of the WAL `sync`, and the guarantees of `Sync` are only verified on Linux).

Final session (`results.jsonl`): `kerno` with the new values as they were then (`dynamic_base_level` on; it is no longer the default value), `kerno-tuned[bytes_per_sync=1048576,bytes_per_writeback=none,dynamic_base_level=false]` (the previous default values), `lsm`, btree and fjall, `medium`, `Sync`, 5 interleaved repetitions, median. **1-minute load: 1.14–2.38 (median 1.41).**

| T | load | metric | kerno | range | previous default | lsm | fjall | btree | k/lsm | k/fjall | k/btree | k/previous |
| T1 | agent_memory | `memory_write_p99_us` | 2339 | 2093–2425 | 3709 | 6947 | 783 | 1399 | 0.34× | 2.99× | 1.67× | 0.63× |
| T2 | livespec | `changeset_commit_p99_us` | 4387 | 4094–5394 | 3062 | 5911 | 1428 | 3406 | 0.74× | 3.07× | 1.29× | 1.43× |
| T2 | append_seq | `append_commit100_p99_us` | 2486 | 2048–2623 | 2486 | 5059 | 1483 | 2632 | 0.49× | 1.68× | 0.94× | 1.00× |
| T2 | append_seq | `update_commit100_p99_us` | 4936 | 4172–5358 | 4772 | 7778 | 1637 | 2468 | 0.63× | 3.01× | 2.00× | 1.03× |
| T2 | history_sim | `rev_commit100_p99_us` | 7078 | 6275–7995 | 7016 | 11026 | 2851 | 19055 | 0.64× | 2.48× | 0.37× | 1.01× |
| T2 | cauce | `step_commit_p99_us` | 1197 | 901–1346 | 1402 | 671 | 574 | 2030 | 1.78× | 2.09× | 0.59× | 0.85× |
| T3 | livespec | `symbol_get_p99_us` | 47.1 | 44.8–52.7 | 48.2 | 47.4 | 37.2 | 44.6 | 0.99× | 1.27× | 1.06× | 0.98× |
| T3 | livespec | `module_scan_p99_us` | 842 | 700–898 | 871 | 848 | 565 | 478 | 0.99× | 1.49× | 1.76× | 0.97× |
| T3 | cauce | `step_read_p99_us` | 127 | 108–142 | 130 | 138 | 109 | 90.3 | 0.92× | 1.17× | 1.41× | 0.98× |
| T3 | agent_memory | `recall_p99_us` | 71.9 | 67.6–80.3 | 85.6 | 80.8 | 61.1 | 52.2 | 0.89× | 1.18× | 1.38× | 0.84× |
| T3 | agent_memory | `recent_scan_p99_us` | 89.3 | 84.0–111 | 109 | 99.9 | 98.0 | 81.7 | 0.89× | 0.91× | 1.09× | 0.82× |
| T4 | bulk_load | `close_ms` | 179 | 175–186 | 252 | 1058 | 187 | 21.6 | 0.17× | 0.96× | 8.28× | 0.71× |
| T5 | livespec | `changesets_per_s` | 623 | 571–671 | 610 | 631 | 639 | 379 | 0.99× | 0.98× | 1.65× | 1.02× |
| T5 | cauce | `steps_per_s` | 2882 | 2809–3009 | 2693 | 2983 | 3002 | 1000 | 0.97× | 0.96× | 2.88× | 1.07× |
| T5 | append_seq | `append_kops` | 113 | 110–136 | 124 | 126 | 128 | 82.9 | 0.90× | 0.88× | 1.36× | 0.91× |
| T5 | agent_memory | `steps_per_s` | 1100 | 1059–1202 | 915 | 949 | 1338 | 1016 | 1.16× | 0.82× | 1.08× | 1.20× |
| T5 | history_sim | `revisions_per_s_k` | 57.3 | 54.2–60.7 | 54.6 | 58.1 | 53.1 | 7.24 | 0.99× | 1.08× | 7.91× | 1.05× |
| T5 | bulk_load | `load_kops` | 118 | 105–130 | 129 | 116 | 134 | 15.6 | 1.02× | 0.88× | 7.56× | 0.91× |
| T7 | bulk_load | `write_amp` | 3.27 | 3.25–3.31 | 3.18 | 3.37 | 1.58 | 10.1 | 0.97× | 2.07× | 0.32× | 1.03× |
| T8 | bulk_load | `reopen_ms` | 4.08 | 3.81–13.3 | 4.30 | 6.59 | 517 | 1.42 | 0.62× | 0.01× | 2.88× | 0.95× |
| T8 | bulk_load | `recovery_ms` | 8.37 | 7.78–13.4 | 9.44 | 8.34 | 512 | 1.04 | 1.00× | 0.02× | 8.04× | 0.89× |
| - | agent_memory | `memory_write_p50_us` | 496 | 440–505 | 485 | 445 | 417 | 630 | 1.11× | 1.19× | 0.79× | 1.02× |
| - | agent_memory | `space_amp` | 3.02 | 3.02–3.02 | 3.02 | 2.46 | 3.15 | 3.93 | 1.23× | 0.96× | 0.77× | 1.00× |
| - | append_seq | `write_amp` | 2.74 | 2.74–2.75 | 2.87 | 3.45 | 1.74 | 2.27 | 0.80× | 1.57× | 1.21× | 0.96× |

(`k/previous` < 1 is an improvement in the latency and space metrics, > 1 in the throughput ones. `bulk_load` `large`, 3 repetitions, load 1.2–2.6: `close_ms` 531 (453–632) versus 598 with the previous values, `lsm` 4 658 and fjall 1 405; target ≤ 1 000 ms.)

Reading:

- **T1 is not met**: 2 339 µs versus 783 of fjall in the same session (**2.99×**, target ≤ 2.0×) and 1 399 of btree (**1.67×**, ≤ 1.5×). The new values bring the p99 down 37 % from the previous ones of the same session (3 709 µs: 4.74× fjall and 2.65× btree) and bring the ratio with btree to within 11 % of the target, but the ratio with fjall worsens compared with those of §7–§8 (2.1–2.7×) because fjall measured 783 µs here, less than in the other sessions (1 044–1 059): the ratio with fjall depends as much on fjall as on `kerno`.
- **T2**: LiveSpec 0.74× `lsm` and `append_seq` insert and update 0.49× and 0.63× (met); `history_sim` 0.64× (met); `append_seq` update 3.01× fjall (target ≤ 2×: not met, as in M0 and in WP05); **Cauce 1.78× `lsm`** (before 2.09×; the target is ≤ 1.2×: still not met, better).
- **T3**: `symbol_get` 0.99×, Cauce `step_read` 0.92×, `recall` 0.89× and `recent_scan` 0.89× (met); `module_scan` 0.99× `lsm` (met) and 1.76× btree (not met, as M0: 2.18×).
- **T4**: `medium` 179 ms (target ≤ 250 ms) and `large` 531 ms (≤ 1 000 ms): met. With the load of these sessions (1.3–2.0) the previous default value measured 252 ms.
- **T5**: between 0.90× and 1.16× `lsm` in the six workloads (≥ 0.9×: met, `append_seq` at the limit).
- **T7**: `write_amp` of `bulk_load` 3.27 (≤ 3.6: met; 3.18 with the previous values).
- **T8**: clean opening 4.1 ms and recovery 8.4 ms, 0.62× and 1.00× `lsm`.

### 9.5 Status of the decision and limits

- **T1 is still not met** (2.99× fjall and 1.67× btree versus ≤ 2.0× and ≤ 1.5×): the status of [f1-decision-cond1.md](f1-decision-cond1.md) remains *candidate for NO-GO, provisional*. No target is modified.
- All the figures come from a shared machine (1-minute load 0.7–6.1; the series with load > 3 are marked) and from a single filesystem (ext4 over a virtual disk). An improvement of 10–15 % in the p99 of T1 is within the dispersion between repetitions; those above 25 % (`dynamic_base_level`, the combination) are repeated in 6 of 6 repetitions but with 6 repetitions per series.
- The new default values are a decision taken with these series and remain pending WP14's confirmation with the machine at rest. `dynamic_base_level` (off by default) changes the trade-off between targets (T1 better, commit p99 of LiveSpec worse, still within T2); if the owner prefers the opposite, it is turned on with `KERNO_TUNING=dynamic_base_level=true` or `CompactionConfig::dynamic_base_level`.
- The shared test `space_is_reclaimed_after_range_deletes` and `space_is_reclaimed` of the threaded profiles (`threads`, `m0_profile`) had isolated failures when the whole suite ran with other processes compiling (also in the `M0` profile, which carries no new lever); run in isolation they always pass (6 of 6). They have not been attributed.
