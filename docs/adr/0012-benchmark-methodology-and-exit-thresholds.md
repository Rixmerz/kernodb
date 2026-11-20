# ADR-0012 — Benchmark methodology and exit thresholds

| Field | Value |
|---|---|
| Status | Accepted (2026-11-12, by delegation of the owner, revocable; [minutes of Stage 9](../architecture/stage9-record.md) §8.1, D5). It was *Proposed* and applied in M0 |
| Date | 2026-10-27 |
| Related to | SAD §17.3–17.6, §18, ADR-0001, risks R-S1, R-S3, R-S4 |

## Context

ADR-0001 must be decided with comparable evidence. A badly designed benchmark (different durability, different data, a single run, medians only) can produce the wrong decision with an appearance of rigor.

## Decision

Accepted on 2026-11-12 as it stands, option (a) of D5 in the [minutes of Stage 9](../architecture/stage9-record.md) §8.1. The text of this decision does not change with the acceptance.

Every measurement that supports an architecture decision follows these rules, implemented in [`spikes/storage/spike-bench`](../../spikes/storage/spike-bench):

### Equal conditions

| Aspect | Rule |
|---|---|
| Machine | All runs of a comparison on the same machine, sequentially (never in parallel). |
| Build | Same `rustc`, `release` profile, same dependencies (`Cargo.lock` versioned). |
| Data | Same deterministic generators and seeds for all engines. |
| Durability | Equivalent and explicit (following table). Strong durability is never compared with relaxed durability as if they were equal. |
| Cache | 64 MiB budget per engine (in SQLite, each connection has its own cache: half for the writer and an eighth for each of up to four readers). The OS page cache is shared and is declared (`warm`), or it is emptied before reopening the database for the read phases (`--cold true`, requires root). |
| Compression | Disabled in all (fjall enables it by default; it is disabled). |
| Isolation | Each combination (engine, workload, scale, repetition) runs in a **new process** and a **new directory**, so that maximum RAM and I/O counters are not contaminated. |

### Durability equivalence

| Spike mode | Prototypes | redb | fjall | SQLite |
|---|---|---|---|---|
| `sync` (survives power loss on commit) | `Durability::Sync` | `Durability::Immediate` | `PersistMode::SyncData` per commit | `journal_mode=WAL`, `synchronous=FULL` |
| `buffered` (no `fsync` per commit) | `Durability::Buffered` (survives process death) | `Durability::None` — **not equivalent**: does not survive process death | `PersistMode::Buffer` | `synchronous=NORMAL` |

The `buffered` figures for redb are published marked as not comparable.

### Repetitions and warm-up

- Each combination runs **3 times** (configurable with `REPS`), interleaving the engines in each repetition so that slow drifts of the machine do not favour one. The `large` scale runs by default **only once** (`LARGE_REPS=1`) and only in `bulk_load`, `point_reads`, `range_scans` and `small_commits`: its figures are single runs (they indicate a trend, not a median) and the range rule does not apply to them.
- The prior data load (*preload*) is not measured. In `point_reads`, `range_scans`, `small_commits`, `point_deletes` and `mixed` the measurement starts after closing and reopening the database; in `livespec`, `cauce` and `agent_memory` it starts right after the preload, on the same open instance (in `cauce`, the reopening after a dirty close happens after the measured phase and only gives `recovery_ms`); `append_seq` and `history_sim` start from an empty database, and in `bulk_load` what is measured is the load itself.
- **Warm-up** separate from the measurement (SAD §17.4): in the workloads that reopen the database before measuring (`point_reads`, `range_scans`, `small_commits`, `point_deletes` and `mixed`), with a warm cache the whole database is walked with paginated scans before measuring (its duration is recorded as `warmup_ms`, outside every metric). The others need no explicit warm-up: they measure the load itself, start from an empty database, or measure right after writing their data in the same process; in them "warm cache" means that the OS cache was not emptied. With `--cold true` there is no warm-up, on purpose; the report labels a run as "cold cache" only if the OS cache flush worked.
- The report shows the **median**; if the range between minimum and maximum exceeds 15 % of the median, the range is shown next to it.

### Mandatory metrics

Throughput; latencies **p50/p95/p99** (and p999/maximum where relevant queues exist); maximum RAM (`VmHWM`); bytes written / logical bytes (`wchar` from `/proc/self/io`, the same for all engines); disk space / logical bytes; time of clean reopening and of recovery after a dirty close; `fsync` per commit in the prototypes (counted by `CountingVfs`).

### Published environment

With each set of results `env.json` is saved: CPU, cores, memory, kernel, file system, `rustc` version, git commit and the **measured latency of `fdatasync`** (p50/p95/p99) of the disk used, which is the floor of any durable commit.

### Scales

`small` (20 000 records, fits in cache), `medium` (200 000), `large` (1 000 000, if the environment allows it; by default a single repetition and only bulk load, point reads, scans and small commits). The size is a recorded parameter, not a universal condition.

### Reproduction

A single command: [`spikes/storage/run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh). The raw data (`results.jsonl`) is versioned together with the report.

## Exit thresholds

Following SAD §17.4, **no** speed threshold was set before measuring. Using the M0 baseline, the following are proposed for Phase 1:

1. **Correctness (blocking):** zero failures in the shared suites with ≥ 1000 seeds and in the sweep of all I/O points, for both failure types.
2. **Regression (blocking in the performance CI):** no throughput metric worsens by > 10 % and no p99 by > 20 % relative to the last baseline on the same machine, unless justified in the PR.
3. **Relative target (indicative):** the chosen engine must stay within 2× of the mature engine of its same family (redb for CoW B+ tree, fjall for LSM) in the M0 workloads at `medium` scale, and document where it does not achieve it.

## Known limitations

- `wchar` counts bytes handed to write calls, not bytes that reach the device (it does not include amplification from the file system or the FTL).
- A VM with virtualized storage may have `fsync` latencies very different from a local SSD; conclusions about small commits also rest on `fsync`/commit, which is independent of the hardware.
- The reference engines do not persist the commit sequence of the spike's contract (the benchmarks do not need it).
- The prototypes have days of development; their differences from redb/fjall also measure maturity, not only architecture (R-S1).
