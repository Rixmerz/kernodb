# Stage 5 · E5-03 — `ExecutionObservation`

Contract of E5-03 ([Stage 5 minutes](stage5-record.md) §3 and decisions of §8). The code is in `crates/kerno-units/src/observation.rs` (observer, levels, retention) and in the two points of `planner.rs` where `query` and `query_relations` report what happened. Starting fields: [adaptive execution §2](adaptive-execution.md); privacy: [ADR-0015](../adr/0015-privacy-of-observability.md).

## 1. Levels

Each `UnitDb` has an `Observer` (`UnitDb::observer()`) that all its snapshots share. It starts at `Off`; no setting, environment reading or other level raises it: `Diagnostic` applies only after `Observer::set_level(Diagnostic)`.

| Level | Keeps |
|---|---|
| `Off` (default, minutes §8.2) | Nothing. A query pays one atomic load |
| `Aggregate` | Counters (`ObservationStats`): executions, errors, fallbacks, rows, total and maximum duration, per path. No records, no footprint, no text |
| `Fingerprint` | The counters and one `ExecutionObservation` per execution, with the keyed footprint of the query shape |
| `Diagnostic` | The above and the shape text (`fingerprint_text`, e.g. `units[unit_type=?text]`) |

Lowering the level acts on what is already stored: below `Diagnostic` the retained texts are deleted; below `Fingerprint`, the records. Only `clear` deletes the counters.

The footprint key (minutes §8.4) is generated from a system source the first time the level is set to `Fingerprint` or higher, lives in memory and is not exported; if the system cannot provide it, `set_level` returns the error and the level does not change. `set_key` exists for reproducible tests.

## 2. What a record carries

`ExecutionObservation`: `observation_id` (local, increasing), `fingerprint`, `fingerprint_text` (only `Diagnostic`), `path` (the path that answered; `None` if it failed before), `plan_version` (`PLANNER_VERSION`), `commit_seq` of the snapshot, `duration_ns`, `rows_returned`, `outcome`, `fallback` and `recorded_at`.

It has no field for a filter, a relations query, a unit, a payload, a key or an error message: the query is reduced to its shape and discarded, and the error is stored as a class (`ErrorKind`), never its message, which may quote a key. The sentinel test over all levels is E5-04 ([report](stage5-sentinels.md)).

`rows_examined` and `bytes_read` are not recorded: no path counts them, and counting them forces instrumenting the hot loops. `cache` and `learned` are Stage 6 (minutes §3).

## 3. Recording never fails or blocks the query

- The query result is computed first and returned intact; recording comes afterwards and cannot change it.
- Recording takes the observer's lock with `try_lock`. If another thread holds it, the observation is **lost and counted** (`ObservationStats::dropped`) instead of waiting. A missing key is counted the same way.
- A poisoned lock is recovered (its state is counters and a queue; a panic does not leave them inconsistent).
- If the level was lowered while the query was running, only what the current level allows is recorded.

## 4. Retention

Memory only (minutes §8.3), bounded by size and by age (`ObservationConfig`): at most `max_records` (default 1024) and none older than `max_age` (default 1 h) relative to the newest. Both bounds are applied on insertion and on reading; evicted records are counted in `evicted`. A configuration that retains nothing (0 records or age 0) is refused.

ponytail: the age is checked only at the front of the queue, so a clock that goes backwards can leave an old record behind newer ones until the size bound evicts it. Upgrade: walk the queue on read.

## 5. I-34, observations part

The observer does not write to the engine, does not open transactions and does not read the layer's `Clock` (a test-step clock would change the timestamps of later commits): it has its own, `SystemClock` by default. `observing_does_not_change_the_data` (`tests/observation.rs`) runs the same commits and reads on two databases, one with `Off` and another with `Diagnostic`, on `Emulated<LogEngine>` and on `KernoDb`, and matches the answers (units, paths, relations), the commit sequences and the engine bytes. It also checks that the observed run did record (4 + 4 × 6 observations), so that it is not a comparison between two silent runs.

## 6. Test count and mutation

`cargo test --workspace --exclude kerno-storage --all-features --release`: **590 → 611** (+21: 13 unit tests of `observation.rs` and 4 of `tests/observation.rs` for two engines). `kerno-units` alone: 238 → 259.

Each new test was seen to fail with a mutation and was restored. The mutations were applied to the production code with a prior copy and restored from it:

| Mutation | Tests that kill it |
|---|---|
| initial level `Aggregate` | `default_is_off_and_off_keeps_nothing`, `the_default_is_off_and_keeps_nothing` (×2), `observing_does_not_change_the_data` (×2) |
| `begin` always returns `Some` | `default_is_off_and_off_keeps_nothing` |
| `Aggregate` keeps records | `aggregate_counts_and_keeps_no_record`, `a_query_that_outlives_a_lowering_…` |
| shape text from `Fingerprint` | `fingerprint_records_the_hash_and_diagnostic_adds_the_text`, `queries_report_what_ran` (×2) |
| lowering the level does not delete texts / records | `lowering_the_level_drops_what_it_no_longer_allows` |
| the level lowered during the query is ignored | `a_query_that_outlives_a_lowering_…` |
| no size bound | `retention_is_bounded_by_size`, `concurrent_recording_…` (×2) |
| no age bound | `retention_is_bounded_by_age` |
| evicted records not counted | `retention_is_bounded_by_size`, `retention_is_bounded_by_age`, `concurrent_recording_…` (×2) |
| unvalidated configuration | `a_config_that_keeps_nothing_is_refused` |
| loss by waiting not counted | `recording_that_would_wait_is_lost_and_counted`, `concurrent_recording_…` (×2) |
| poisoned lock propagates | `a_poisoned_lock_is_recovered_not_propagated` |
| missing key not counted | `a_missing_key_is_a_counted_loss` |
| `ErrorKind::Corruption` → `Storage` | `error_kinds_carry_no_message` |
| an error keeps the path | `a_failed_query_is_recorded_as_a_class_without_a_path` |
| the path is lost from the record | `fingerprint_records_the_hash_…`, `queries_report_what_ran` (×2) |
| the observed query returns another answer | `observing_does_not_change_the_data` (×2), `queries_report_what_ran` (×2), `concurrent_recording_…` (×2) |
| `commit_seq` fixed at 0 | `queries_report_what_ran` (×2) |
| relations observed with the units shape | `queries_report_what_ran` (×2), only after adding the assertion on `QueryKind` |

The last row was a finding: the mutation stayed green, that is, the test did not distinguish the kind of query observed. The assertion `fingerprint.kind()` was added and the mutation turned red.

## 7. Cost of observing (threshold ≤ 5% of the p99 of the unobserved query, minutes §8.5)

`index_bench` measures `secondary` (W1, type filter, ~1% selectivity) with the observer at `off` and at each level, in five rounds alternating the levels, full mode and on the real file system. The cost is the added p50 of the best round of each level against `off`, divided by the p99 of the best round of `off`.

| Scale | `aggregate` | `fingerprint` | `diagnostic` |
|---|---|---|---|
| 20,000 units, run 1 | −1.5% | +3.2% | **+5.9%** |
| 20,000 units, run 2 (`--seed 7`) | −1.5% | +0.6% | +3.6% |
| 100,000 units, run 1 | +3.0% | −0.2% | −2.6% |
| 100,000 units, run 2 (`--seed 7`) | −1.3% | −0.5% | −3.4% |

Reading: the machine is shared and differences in p50 of tens of microseconds are confused with noise (there are negative values). In 11 of 12 measurements (three levels in four runs) the cost meets the threshold. The exception is `diagnostic` at 20,000 units, run 1 (+5.9%); in run 2 it meets it (+3.6%). The real cost is bounded by the work added per execution: the footprint (keyed BLAKE3 over the shape text) and, in `diagnostic`, copying the text. `diagnostic` is only by explicit activation. **It is recorded as a risk, not as a blocker** (minutes §5 and §8): with the W1 query at around 1.3 ms, 5% is ~65 µs, of the order of the cost of `diagnostic` itself; for slower queries the fraction falls.

The planner on W1 (E5-02, threshold ≤ 10%) was measured again in these same runs: p50/p99 +6.1% / **+14.1%** at 20,000 units (run 1) and +2.6% / +0.3% in run 2; +2.4% / +2.8% and +2.2% / −6.0% at 100,000. One value out of four is outside the threshold, and the second run does not reproduce it. It is noise from the shared machine more than a change (E5-03 does not touch the planner except for the report to the observer, which at `off` is one atomic load); it is noted for E5-05.

### Raw output

Run 1 (`cargo run --release -p kerno-units --example index_bench -- --dir <dir>`):

```text
scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms
20000,type,reference,71.440,86.992,108.472,199,109
20000,type,primary,7.953,8.289,8.480,199,109
20000,type,ideal,1.083,1.538,1.653,199,109
20000,type,secondary,1.284,1.895,2.169,199,109
# 20000 type: primary/ideal p50 = 7.35x, primary p95 = 8.289 ms, secondary/ideal p50 = 1.19x -> threshold not exceeded
# round 0: planner p50/p99 = 1.255/1.864 ms, legacy = 1.277/1.937 ms
# round 1: planner p50/p99 = 1.225/1.956 ms, legacy = 1.668/2.099 ms
# round 2: planner p50/p99 = 1.199/1.911 ms, legacy = 1.163/1.808 ms
# round 3: planner p50/p99 = 1.205/1.681 ms, legacy = 1.194/1.473 ms
# round 4: planner p50/p99 = 1.239/3.031 ms, legacy = 1.131/2.125 ms
# 20000 type: planner overhead over the current path: p50 +6.1 %, p99 +14.1 % (threshold 10 %) -> exceeded
# 20000 type: observation at Aggregate adds p50 -0.0204 ms = -1.51 % of the unobserved p99 1.350 ms (threshold 5 %) -> within
# 20000 type: observation at Fingerprint adds p50 +0.0424 ms = +3.15 % of the unobserved p99 1.350 ms (threshold 5 %) -> within
# 20000 type: observation at Diagnostic adds p50 +0.0798 ms = +5.91 % of the unobserved p99 1.350 ms (threshold 5 %) -> exceeded
20000,type+schema,reference,69.322,82.248,112.954,67,109
20000,type+schema,primary,7.254,11.639,13.561,67,109
20000,type+schema,ideal,0.472,0.743,1.050,67,109
20000,type+schema,secondary,0.486,0.693,0.886,67,109
# 20000 type+schema: primary/ideal p50 = 15.36x, primary p95 = 11.639 ms, secondary/ideal p50 = 1.03x -> threshold exceeded
100000,type,reference,455.479,574.590,610.601,999,698
100000,type,primary,53.957,64.433,82.387,999,698
100000,type,ideal,9.602,10.948,12.097,999,698
100000,type,secondary,10.528,14.335,16.519,999,698
# 100000 type: primary/ideal p50 = 5.62x, primary p95 = 64.433 ms, secondary/ideal p50 = 1.10x -> threshold exceeded
# round 0: planner p50/p99 = 11.721/18.101 ms, legacy = 11.213/17.741 ms
# round 1: planner p50/p99 = 11.994/15.253 ms, legacy = 11.792/36.830 ms
# round 2: planner p50/p99 = 11.482/14.402 ms, legacy = 11.238/19.409 ms
# round 3: planner p50/p99 = 11.518/16.607 ms, legacy = 11.906/14.006 ms
# round 4: planner p50/p99 = 12.890/17.782 ms, legacy = 12.467/17.352 ms
# 100000 type: planner overhead over the current path: p50 +2.4 %, p99 +2.8 % (threshold 10 %) -> within
# 100000 type: observation at Aggregate adds p50 +0.3456 ms = +3.02 % of the unobserved p99 11.463 ms (threshold 5 %) -> within
# 100000 type: observation at Fingerprint adds p50 -0.0260 ms = -0.23 % of the unobserved p99 11.463 ms (threshold 5 %) -> within
# 100000 type: observation at Diagnostic adds p50 -0.2980 ms = -2.60 % of the unobserved p99 11.463 ms (threshold 5 %) -> within
100000,type+schema,reference,434.437,519.880,552.383,333,698
100000,type+schema,primary,43.255,51.928,62.686,333,698
100000,type+schema,ideal,3.397,3.782,4.019,333,698
100000,type+schema,secondary,3.658,4.124,4.432,333,698
# 100000 type+schema: primary/ideal p50 = 12.73x, primary p95 = 51.928 ms, secondary/ideal p50 = 1.08x -> threshold exceeded
# secondary index justified by W1: true
```

Run 2 (`-- --dir <dir> --seed 7`, only the `secondary`, planner and observation lines):

```text
scale,query,path,p50_ms,p95_ms,p99_ms,mean_hits,build_ms
20000,type,secondary,1.236,1.458,1.534,201,106
# 20000 type: planner overhead over the current path: p50 +2.6 %, p99 +0.3 % (threshold 10 %) -> within
# 20000 type: observation at Aggregate adds p50 -0.0269 ms = -1.54 % of the unobserved p99 1.739 ms (threshold 5 %) -> within
# 20000 type: observation at Fingerprint adds p50 +0.0099 ms = +0.57 % of the unobserved p99 1.739 ms (threshold 5 %) -> within
# 20000 type: observation at Diagnostic adds p50 +0.0633 ms = +3.64 % of the unobserved p99 1.739 ms (threshold 5 %) -> within
20000,type+schema,secondary,0.433,0.568,0.611,67,106
100000,type,secondary,11.821,13.399,13.971,999,634
# 100000 type: planner overhead over the current path: p50 +2.2 %, p99 -6.0 % (threshold 10 %) -> within
# 100000 type: observation at Aggregate adds p50 -0.1778 ms = -1.32 % of the unobserved p99 13.433 ms (threshold 5 %) -> within
# 100000 type: observation at Fingerprint adds p50 -0.0607 ms = -0.45 % of the unobserved p99 13.433 ms (threshold 5 %) -> within
# 100000 type: observation at Diagnostic adds p50 -0.4565 ms = -3.40 % of the unobserved p99 13.433 ms (threshold 5 %) -> within
100000,type+schema,secondary,3.748,4.353,5.304,333,634
```

## 8. Status against the criteria of the minutes (§4)

| Criterion | Status after E5-03 |
|---|---|
| Observing does not alter confirmed data or the commit sequence (I-34, observations only) | Verified by `observing_does_not_change_the_data` on two engines |
| Nothing of literals or content outside `diagnostic` (I-37) | The record has no field for them; the sentinel test per level is E5-04 |

## 9. Risks

- `diagnostic` exceeded 5% in one of four measurements (§7). Risk recorded, not blocking.
- Under contention, observations are lost (counted in `dropped`) instead of waiting: a low counter is information, not a failure. If Stage 6 needs complete records under load, a per-thread collector will be needed.
