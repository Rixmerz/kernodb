# Stage 4 — Workloads of the secondary indexes (E4-04)

This document fixes, **before measuring**, the workloads that may justify a secondary index ([Stage 4 minutes](stage4-record.md) §3, E4-04): "only those that a measured workload justifies; each one has its workload written before it is built". The methodology is that of [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md) (same process, same options, `KernoDb` on the real file system, separate warm-up, median and percentiles). The results are added in §4 without changing §1–§3.

## 1. Which queries the stage supports

The only filtered query of Stage 4 is `UnitFilter` (E4-03): **active** units with `unit_type` equal to a value, `schema_version` equal to a value, or both, in id order. The primary path (`IDX_PRIMARY`) walks **all** the active entries, applies the filter to each entry and opens (verified read) only those that match. A secondary index on `(unit_type, schema_version)` would avoid walking the entries that do not match; it cannot avoid opening the ones that do. Relations already have their adjacency indexes (`REL_OUT`/`REL_IN`, ADR-0014) and are not covered here.

## 2. Workload W1 — selective filter by type

| Parameter | Value |
|---|---|
| Active units | `N` = 20,000 and 100,000 (`small` scale and mid-way to `medium` of ADR-0012) |
| Archived units | 10% additional (they do not appear in the index or in the answer) |
| Types | 100 uniform types → **selectivity 1%** per type (200 and 1,000 results) |
| Schema versions | 1–3 uniform; the combined query `(type, version)` has selectivity ≈0.33% |
| Payload | 256 bytes; empty metadata |
| Queries | `query(type)` and `query(type, version)` with the index `Ready`; 10% unmeasured warm-up; 200 samples (`N` = 20,000) and 100 (`N` = 100,000) per path |
| Paths measured | reference (`reference_filter`), primary (`query`, `QuerySource::Index`) and **ideal bound**: `get_unit` of exactly the ids that match, with no walk. It is the minimum that any secondary index could cost, which would also have to walk its own range |

## 3. Threshold (fixed before measuring)

A secondary index on `(unit_type, schema_version)` is justified **only if**, at `N` = 100,000 and for either of the two queries, **both** conditions are met:

1. **Margin**: the median of the primary path exceeds **2×** the median of the ideal bound (an index could, at most, divide the latency by two or more).
2. **Matters**: the p95 of the primary path exceeds **10 ms**.

If they are not met, no secondary index is built and the conclusion is documented with the figures. No threshold is lowered after measuring.

## 4. Results (2026-11-05)

Command: `cargo run --release -p kerno-units --example index_bench` (3 passes, seed 42). Environment: 4 vCPU Intel Xeon @ 2.80 GHz, 15 GiB, Linux 6.18, `rustc` 1.97.0, OS cache warm. Raw data in [`spikes/storage/results/2026-11-05-e4-04/`](../../spikes/storage/results/2026-11-05-e4-04/) (`index_bench-w1-before.txt`: before the index is built; `-after.txt`: with it). Figures in ms, median of the 3 passes for each percentile.

### 4.1 Before the secondary index (primary path of E4-03)

| `N` | Query | Results | Reference p50 | Primary p50 / p95 | Ideal bound p50 | Primary / ideal (p50) |
|---|---|---|---|---|---|---|
| 20,000 | type | 199 | 93.3 | 9.9 / 12.6 | 1.51 | 7.4× |
| 20,000 | type + version | 67 | 100.9 | 6.9 / 12.0 | 0.54 | 14.4× |
| 100,000 | type | 999 | 508.7 | 59.4 / 68.7 | 12.15 | 4.9× |
| 100,000 | type + version | 333 | 520.9 | 41.2 / 59.1 | 4.50 | 10.6× |

At `N` = 100,000 both queries exceed the threshold in all 3 passes: the primary path is 4.8–12.3× the ideal bound (> 2×) and its p95 is 57–71 ms (> 10 ms). Walking the 100,000 entries of `IDX_PRIMARY` costs more than opening the 333–999 units that match. **The workload justifies a secondary index on `(unit_type, schema_version)`**, which is built: `IDX_TYPE` ([units-v1.md](../file-format/units-v1.md) §3.11).

### 4.2 With `IDX_TYPE`

| `N` | Query | Primary p50 | `IDX_TYPE` p50 / p95 | Ideal bound p50 | `IDX_TYPE` / ideal (p50) |
|---|---|---|---|---|---|
| 20,000 | type | 10.2 | 1.49 / 2.16 | 1.25 | 1.07–1.19× |
| 20,000 | type + version | 8.2 | 0.49 / 0.71 | 0.44 | 1.07–1.14× |
| 100,000 | type | 62.3 | 13.1 / 17.1 | 10.18 | 1.25–1.51× |
| 100,000 | type + version | 55.6 | 5.2 / 6.3 | 3.93 | 1.26–1.31× |

With the index, the query by type is at 1.07–1.51× of the ideal bound (4–11× faster than the primary path); the one by type and version, at ≤ 1.31×. At `N` = 100,000 the p95 by type stays above 10 ms (17 ms) because merely opening the 999 results already costs 12 ms (ideal bound): no index can go below that, and the remaining cost is the verification of each unit returned.

**Cost**: building the index writes twice as many entries. Its duration goes from 54–75 ms to 134–179 ms with 20,000 units, and from 317–360 ms to 793–858 ms with 100,000 (≈2.4×). No application `ChangeSet` pays anything: the index is derived (ADR-0007) and only `rebuild_index` writes it.

### 4.3 Conclusion

W1 justifies `IDX_TYPE` and is the only secondary index of the stage. A filter by schema version alone has no written workload and stays on `IDX_PRIMARY`.
