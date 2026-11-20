# Stage 5 · E5-02 — Deterministic planner

Contract of E5-02 ([Stage 5 minutes](stage5-record.md) §3). The code is in `crates/kerno-units/src/planner.rs` (steps, costs, validation) and `plan.rs` (the validated choice, `Decision`). The plan and footprint types are those of [E5-01](stage5-footprint-and-plan.md).

## 1. The seven steps of spec §9

`UnitSnapshot::query` and the new `query_relations` **are** those steps over the paths that already existed (minutes §2.2); no index, path or query was added.

| Step | What it does here |
|---|---|
| 1. Analyse and validate | Every `UnitFilter` and `RelationQuery` is valid: a condition that no stored value meets (empty or too long type) is a legitimate query with an empty answer, as both paths already gave |
| 2. Valid candidates | `PlanPath::candidates_of`, each one marked usable or not (I-23) |
| 3. Estimate cost | With what the layer already has: the status and number of entries of `IDX_META`, and a bounded probe (`PROBE` = 64 reads) of the range the path would read. No new statistic |
| 4. Choose | The cheapest usable one; on a tie, the first in the table (`Decision::new`) |
| 5. Execute | The chosen path, through the same code as the explicit calls |
| 6. Record | `QueryOutcome` and `RelationsOutcome` carry the path and the fallback; `query` hands them to the observer of [E5-03](stage5-observation.md) |
| 7. Validate and fall back | The answer is checked against the query (only what the filter selects, only `Active`, strictly ascending order of the reference). If the index path fails with damage (`Corruption`) or the answer does not pass, **the reference path answers** and the result says so (`Fallback { abandoned, cause }`). An error that is not damage is not hidden with a second attempt |

`explain_filter` and `explain_relations` return the `Plan` that `query` / `query_relations` follow in that snapshot: candidates with cost and usability, chosen one, reason and index status. The plan carries the keyed footprint, never the query.

## 2. Determinism

The decision is a pure function of the query and of what the snapshot contains: it reads no clock, no randomness and no handle. The same input and snapshot give the same candidates, costs and choice, on any engine and after reopening.

## 3. Cost

Number of expected key-value reads; comparable only within one plan. `N` = entries of the `IDX_META` generation; opening one verified unit = `OPEN_READS` (3) reads; `m` = units that the filter keeps.

| Path | Cost |
|---|---|
| `UnitsReference` | `N × (1 + OPEN_READS)` |
| `IndexPrimary` | `N + OPEN_READS × m` |
| `IndexType` | `(1 + OPEN_READS) × m`; `m` is probed (exact if the range is smaller than `PROBE`) |
| `RelationsReference` | `r` (relations probed) |
| `Adjacency` | `2 × a` (adjacency entries probed) |

Without a full probe, `m` is `N / 2` if the filter fixes any condition and `N` if not. Consequences: a filter with no conditions costs the same via the reference as via `IDX_PRIMARY`, and the tie goes to the reference; an empty or unbuilt index has `N = 0`, so the reference wins, which is also the only usable one.

**Observable behaviour change.** Before, `query` with the index `Ready` always used the index. Now, with index `Ready`, it uses the index **only if it is cheaper**: a filter with no conditions answers via the reference (`QuerySource::Reference`). The result does not change (I-32); `QueryOutcome::source`/`path` does. `QuerySource::Reference` now documents the three causes.

ponytail: the `N / 2` prior and the probe limit are fixed numbers, not learned; W1 (type filter of 1%) is decided by the probe. Upgrade: count by type in `IDX_META`, written by the rebuild, if a workload shows the prior choosing the slower path.

## 4. Fallback by freshness status

The index status the planner sees is that of the **snapshot**: `Building` is only reported by `UnitDb::index_status` (the handle); a snapshot sees what is stored, and a rebuild in progress has stored nothing. Therefore:

| Status | Path | How it is tested |
|---|---|---|
| Not built / `Stale` | reference, `Reason::Fallback { Stale }` | `planner_falls_back_when_the_index_is_not_ready`, with all filters |
| `Failed` | reference, `Reason::Fallback { Failed }` (after repairing the data, which does not clear the status) | same |
| `Building` | the snapshot still sees the stored generation `Ready` and answers the same as the reference; the plan for `Building` (both index paths unusable) is tested over the candidates | same test (during the rebuild waiting) and `candidates_carry_the_usability_the_state_gives` |
| Index path that fails or answers wrongly | reference, fallback recorded | `a_failing_or_wrong_index_path_falls_back_to_the_reference`, `a_failing_or_wrong_adjacency_falls_back_to_the_reference` (fault injection; one case per `Violation` and per `Corrupt`) |

## 5. Differential test in CI (I-32)

The checks of `check` (`tests/index.rs`: seeds 1–6 plus sweeps by pages and rebuild rounds) and of `assert_equivalent` (`tests/relations.rs`) now also compare `query` and `query_relations` with the reference path, with and without a ready index, over the same filters and seeds; they require `fallback == None` in a healthy store, that the path executed is the one `explain_*` announces, and that two explanations of the same snapshot are equal.

## 6. Test count and mutation

Suite `cargo test --workspace --exclude kerno-storage --all-features --release`: **581 → 590** (7 unit tests of `planner.rs` and 2 of `tests/index.rs`, one per engine; the differential assertions were added to existing tests). Each new test was seen to fail with a mutation of the test itself (inverting the expected status, changing `Violation`, changing a cost, comparing with a fixed path) and was restored.

## 7. Overhead on W1 (threshold ≤ 10%, minutes §8)

`index_bench` compares `query` (planner) with the previous path (`index_status` + `indexed_scan`, without estimating cost), in five rounds alternating both, over W1 (type filter, ~1% selectivity), in full mode and on the real file system. The overhead is that of the best round of each path.

| Scale | p50 | p99 | Threshold |
|---|---|---|---|
| 20,000 units | +2.2% | +0.6% | meets |
| 100,000 units | +1.8% | **+8.5%** | meets, with little margin on p99 |

This should be read with care: the p99 at 100,000 units varies between rounds (10.96–14.39 ms on the previous path) and the machine is shared, so +8.5% does not reliably distinguish an overhead of between 0% and 12%. The added cost is the probe of the type range (up to 64 index entry reads). It is recorded as data; a failure would be a risk and not a blocker (minutes §5).

### Raw output

```text
     Running `target/release/examples/index_bench`
scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms
20000,type,reference,76.799,95.460,110.150,199,79
20000,type,primary,6.392,10.013,11.670,199,79
20000,type,ideal,0.881,1.268,2.618,199,79
20000,type,secondary,0.945,1.095,1.215,199,79
# 20000 type: primary/ideal p50 = 7.26x, primary p95 = 10.013 ms, secondary/ideal p50 = 1.07x -> threshold exceeded
# round 0: planner p50/p99 = 0.967/1.170 ms, legacy = 0.942/1.450 ms
# round 1: planner p50/p99 = 0.970/1.334 ms, legacy = 0.953/1.163 ms
# round 2: planner p50/p99 = 0.952/1.703 ms, legacy = 0.947/1.171 ms
# round 3: planner p50/p99 = 0.949/1.720 ms, legacy = 0.928/1.355 ms
# round 4: planner p50/p99 = 0.953/1.314 ms, legacy = 0.941/2.178 ms
# 20000 type: planner overhead over the current path: p50 +2.2 %, p99 +0.6 % (threshold 10 %) -> within
20000,type+schema,reference,74.569,100.957,111.394,67,79
20000,type+schema,primary,5.504,6.676,7.608,67,79
20000,type+schema,ideal,0.304,0.404,0.475,67,79
20000,type+schema,secondary,0.346,0.455,0.547,67,79
# 20000 type+schema: primary/ideal p50 = 18.10x, primary p95 = 6.676 ms, secondary/ideal p50 = 1.14x -> threshold not exceeded
100000,type,reference,472.017,592.730,650.109,999,675
100000,type,primary,46.422,88.199,96.111,999,675
100000,type,ideal,9.318,12.250,12.810,999,675
100000,type,secondary,10.086,13.417,16.349,999,675
# 100000 type: primary/ideal p50 = 4.98x, primary p95 = 88.199 ms, secondary/ideal p50 = 1.08x -> threshold exceeded
# round 0: planner p50/p99 = 9.288/15.047 ms, legacy = 8.138/12.567 ms
# round 1: planner p50/p99 = 8.099/11.891 ms, legacy = 7.953/10.961 ms
# round 2: planner p50/p99 = 9.589/12.570 ms, legacy = 8.457/12.809 ms
# round 3: planner p50/p99 = 9.721/14.680 ms, legacy = 9.255/12.491 ms
# round 4: planner p50/p99 = 8.729/16.205 ms, legacy = 8.916/14.388 ms
# 100000 type: planner overhead over the current path: p50 +1.8 %, p99 +8.5 % (threshold 10 %) -> within
100000,type+schema,reference,471.772,564.813,609.720,333,675
100000,type+schema,primary,47.891,72.129,78.186,333,675
100000,type+schema,ideal,2.369,2.672,3.171,333,675
100000,type+schema,secondary,2.931,3.505,3.637,333,675
# 100000 type+schema: primary/ideal p50 = 20.22x, primary p95 = 72.129 ms, secondary/ideal p50 = 1.24x -> threshold exceeded
# secondary index justified by W1: true
```
