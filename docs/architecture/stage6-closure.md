# Closure of Stage 6

| Field | Value |
|---|---|
| Status | **Approved by delegation of the owner on 2026-11-11** (§7); the owner may revoke it. |
| Date | 2026-11-11 (E6-05) |
| Act | Adversarial review, promotion of I-33…I-36 and closure of Stage 6 (minutes §3, row E6-05) |
| Scope | Packages E6-00 to E6-05: result cache as the only optimisation, in `kerno-units` (`cache.rs`, `planner.rs`, `tests/cache*.rs`) |
| Documents | [Minutes of Stage 6](stage6-record.md) · [measurement and sentinels](stage6-measurement.md) (§8, final verdict) · [E6-00](stage6-e600-remeasurement.md) · [adaptive execution](adaptive-execution.md) · [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) · [ADR-0015](../adr/0015-privacy-of-observability.md) · [invariants §2.5](invariants.md) · [roadmap](../roadmap/README.md#stage-6--first-operational-plasticity) · [closure of Stage 5](stage5-closure.md) |

This document describes the stage as it stands on the branch after E6-05. **The result cache stays disabled by default** (minutes §8, decision 2). The net improvement of §6 of the minutes is **not claimed**: the p99 of W1 cannot be certified. No threshold has been changed.

## 0. Result in brief

| Threshold (minutes §6) | Result in the final measurement (§4) | Passes |
|---|---|---|
| W1, p50 with the cache on ≥ 20% better | −97.8% to −97.9% in the four runs | **Yes** |
| W1, p99 ≤ +5% | +0.5%, +7.8%, +4.7%, +7.5% in four runs; paired real cost of the read that is not served of +3.4% to +4.9% | **Not certified** (two of four runs above +5%; the real measured cost is below, but within the noise) |
| Worst case without repetitions, p99 ≤ +5% (20,000 and 100,000 units) | Paired real cost from −0.6% to +0.5% (mean); in the eight median-p99 runs, two readings stay at +5.3% (one at 20,000, another at 100,000) | **Yes in the paired measurement; not in every run** |
| Joint net-improvement criterion (§6) | Not met stably: the p99 of W1 is not certified | **Not claimed** |

**Consequence (minutes §8.2):** the result cache stays **disabled by default** and available as an option (`ResultCache::set_enabled`). Enabling it by default would require an owner decision on a criterion that the measurement does not allow to be claimed.

## 1. Work packages delivered

| WP | Delivery | Commits (main) | Status |
|---|---|---|---|
| E6-00 | Minutes approved (2026-11-09) with the decisions of §7 and §8; re-measurement of the planner on an unloaded machine | `558880f`, `ac1a000`, `38e2a5d`, `fccd7d2` | done |
| E6-01 | Versioned dependency model (`Dependency`, `Dependencies`), in-memory store with a size and age bound, transition log (G7) | `fbe1e95`, `3759479` | done |
| E6-02 | Cache as the first candidate of the planner in `query` and `query_relations`; fallback with recorded cause; check of rows on each hit | `44f076a`, `f17cb20` | done |
| E6-03 | Reversibility and deactivation: enabling, disabling, invalidating and deleting do not touch data or `commit_seq` | `b08f1cb`, `e93208c`, `3985822` | done |
| E6-04 | Measurement with the load of §6 and I-37 sentinels | `f0d5cca`, `ca28c7f` | done; the verdict of its first attempt was revised in E6-04b |
| E6-04b | Admission (do not store a result that no one will repeat), footprint memoised per shape, bench of 20 rounds and paired measurement; final verdict of §8 of the measurement | `b7dd18f`, `2a1c433`, `c5ab3cb`, `9ea9363`, `15b2814` | done; verdict of §8 |
| E6-05 | This closure: adversarial review (§3), promotion of I-33…I-36 in `invariants.md` §1 and §2.5, update of the roadmap, of SAD §9, of `adaptive-execution.md` and of the table of §4 of the minutes, corrections of comments in `cache.rs` | this change | done, **pending approval** |

## 2. Acceptance criteria (minutes §4)

| Criterion | Invariant / guarantee | Evidence | Status |
|---|---|---|---|
| Net improvement measured against the disabled mode, with thresholds fixed beforehand | — | §4: p50 of W1 meets it; p99 of W1 is not certified; worst case meets it in the paired measurement | **not claimed**: the cache stays disabled by default |
| Equivalence of results | I-32, G1 | `tests/cache_planner.rs`, `cache_on_off_and_invalidated_give_identical_results` (`emulated_log` and `kerno_db`): cache on and off, with writes between rounds and a retained database that reads an old snapshot. Limitation H-1 (§3) | **met** |
| Fallback to a valid plan, recorded, on precondition, invalidation, deactivation or discrepancy | I-33, G2, G3 | `tests/cache_planner.rs`: `a_failed_check_discards_the_entry_and_the_reference_path_answers`, `observations_never_act_as_authority`, `every_use_of_the_cache_is_a_transition`. The cause is in the cache's log, not in the observation (H-6) | **met** (with H-6) |
| Invalidation proven for each dependency | I-35, G4 | `cache.rs`, `each_declared_dependency_invalidates_the_entry_and_is_named` (one case per `Dependency`, with exhaustive `match`); `tests/cache_reversibility.rs`, `invalidating_by_each_dependency_leaves_the_data_as_it_was` | **met** |
| Invalidating, deactivating or deleting does not alter data | I-34, G6 | `tests/cache_reversibility.rs`, `every_operation_on_the_cache_leaves_the_data_as_it_was`, `a_cache_in_use_never_changes_what_a_commit_writes` (logical state, engine bytes and `commit_seq` equal) | **met** |
| Deactivation verified | I-36, G6 | `tests/cache_reversibility.rs`, `with_the_cache_off_no_optimization_takes_part`, `a_cache_switched_off_after_use_stops_taking_part`. The mutation of §3 (H-7) survives | **met with H-7** |
| The learned path passes the same validations | G5 | Mutation of `check_units` on a hit: dies (§3.1) | **met** (with H-1) |
| Traceability without literals | G7, I-37 | `cache.rs`, `every_transition_leaves_a_record`; `tests/cache_sentinels.rs`, `every_g7_transition_leaves_its_record` and `no_literal_in_anything_the_cache_path_reports` (four levels, two engines); mutations S-M1 and S-M2 of [E6-04b](stage6-measurement.md) §4 | **met** |

## 3. Adversarial review (E6-05)

### 3.1 What was reviewed

- `crates/kerno-units/src/cache.rs` (1,675 lines): dependency model, `CacheEntry`, store `ResultCache`, admission (`worth_storing`), transition log and footprint memoised per shape.
- `crates/kerno-units/src/planner.rs`, functions `answer_units`, `answer_relations`, `store`, `execute_units` and `execute_relations`: the only entry of the cache into the query path.
- `crates/kerno-units/tests/cache.rs`, `cache_planner.rs`, `cache_reversibility.rs`, `cache_admission.rs` and `cache_sentinels.rs`.
- Against ADR-0016 (G1 to G7) and I-37, according to the reading of the code, of the tests and of the mutations of §3.3.

Each guarantee was read in the code that enforces it and the test that fails if it breaks was looked for. Result per guarantee:

| Guarantee | What enforces it in the code | Test that defends it | Status |
|---|---|---|---|
| G1 equivalence | Each hit goes through `check_units`/`check_relations`, and each entry stores the answer of the planned path or of the reference path at the same `commit_seq` | `cache_on_off_and_invalidated_give_identical_results`; mutation of G4 (§3.3) | meets; completeness not checked at query time (H-1) |
| G2 reference always available | Each failure, invalidation, deactivation or discrepancy in `answer_units`/`answer_relations` executes the reference path or the planned path without learned state | `a_failed_check_discards_the_entry_and_the_reference_path_answers` | meets |
| G3 observations are not authority | `answer_units` and `answer_relations` do not read the observer; the cache uses only `index_stats` and `fingerprint_of` | `observations_never_act_as_authority` | meets (reading of the code confirmed by search in `planner.rs`) |
| G4 explicit invalidation | `Dependencies::first_change` compares the five dependencies before each use; `commit_seq` is an exact match; there is no "unless proven" exception | mutation of `commit_seq` (§3.3): dies | meets |
| G5 no skipping of validations | Hit → `check_units`; a failure deletes the entry and the reference answers | mutation of `check_units` (§3.3): dies | meets (H-1) |
| G6 reversibility | All cache operations act on `ResultCache`, which has no engine handle nor transaction | `cache_reversibility.rs` (logical state, bytes, `commit_seq`) | meets (not mutated) |
| G7 traceability | Each transition calls `State::record`, with `OPTIMIZATION_VERSION` and `PLANNER_VERSION`; `Transition` has no field for literals | `every_transition_leaves_a_record` and `every_g7_transition_leaves_its_record` | meets (H-6 on the observation) |
| I-37 | `TransitionRecord`, `CacheError`, `CacheStats` and `Transition` have no field for filter, unit or row; `UnitError::InvalidArgument` carries only the name of the limit | `no_literal_in_anything_the_cache_path_reports` (four levels, two engines, with control); mutations S-M1 and S-M2 | meets |

### 3.2 Findings

Each finding states the evidence and whether it was corrected in this change. **Blockers: none.** No finding contradicts G1 to G7 or I-37; the ones that remain are documentary, of coverage or of design, and are listed as decisions in §6.

| # | Finding | Evidence | Class | Cause | Status |
|---|---|---|---|---|---|
| H-1 | `check_units` and `check_relations` verify that the answer of the cache is a correct subset (filter, `Active`, order), not that it is complete. It is the same limit as H-1 of [closure of Stage 5](stage5-closure.md) §3 | Reading of `planner.rs` (`answer_units`, `check_units`). Inferential | inferential | inherited | **accepted as a limitation**; covered by the differential test, not by the query |
| H-2 | The fallback with index `Building` is not exercised through `query` (H-2 of Stage 5) | Inherited | inferential | inherited | **open**, unchanged in this stage |
| H-3 | The creation evidence of the entry is constant: `Evidence { observations: 1, window: Duration::ZERO }` in `planner.rs` (`store`). It does not come from `ExecutionObservation`. G3 allows it (the observation is not authority), but spec §9 asks for "observed evidence (number of observations and window)", and here the field describes no observation | `planner.rs`, `store()`. Deterministic | deterministic | introduced in E6-02 | **open**: the owner decides whether the evidence must come from the observer (with G3) or whether it is enough to document that it is the execution that produced the answer |
| H-4 | Header comment of `cache.rs` out of date: it said the cache "is not connected to `query` nor to the planner (E6-02)" | `grep` of `cache.rs` and `planner.rs:354` | deterministic | pre-existing (E6-01) | **corrected** in E6-05 (comment only) |
| H-5 | An entry older than `max_age` is never served (the check is in `lookup`), but it stays in memory while the cache does not create an entry, change its bounds or look up that same footprint. `prune_entries` is called only by `create` and `set_config`. The header comment said "none older than `max_age`", which is not true at rest. Minutes §5 asks for "bound by size and age" | `grep -n prune_entries crates/kerno-units/src/cache.rs` → only the calls inside `set_config` and `create`. Deterministic | deterministic | pre-existing (E6-01) | **comment corrected** in E6-05. **The code does not change**: memory stays bounded by `max_entries` and `max_bytes`. A possible one-line correction (call `prune_entries` in `stats()` and `records()`), which the owner decides |
| H-6 | The causes of the cache (`precondition_failed`, `invalidated`, `disabled`, `discrepancy`) do not reach `ExecutionObservation`. Its `fallback` field carries `FallbackCause` (`Corrupt` or `Invalid`); a `discrepancy` appears as `Invalid` with the abandoned `ResultCache` path, and an `invalidated` or `disabled` leaves no trace in the observation (the query is recorded with its planned path and without `fallback`). They are in the transition log (G7), and `adaptive-execution.md` §2 said otherwise | `grep` of `observation.rs` and `planner.rs` (`CacheFallbackCause` is used only in `cache.fallback`). Deterministic | deterministic | introduced in E6-02 against the design of [adaptive execution](adaptive-execution.md) | **documented** in `adaptive-execution.md` §2 and in I-33. Carrying it to the observation is a change of public type (`FallbackCause`), which is not done in this stage |
| H-7 | The mutation "with the cache off, the cache path is followed" (`if !self.cache.enabled()` → `if false`, `planner.rs:354`) **survives** the whole cache suite. With the mutation, with the cache off one more index read is made per query; neither answers nor counters change. No test of I-36 observes that the cache path is not executed | §3.3, row I-36. Deterministic | deterministic | pre-existing (E6-03) | **open**: I-36 is verified in its results and counters part, not in its cost part. A reliable test requires observing the index read, which has no injection point today |

### 3.3 Mutations of the review

Each mutation was applied with `sed` to one production line, the suite was run with `cargo test -p kerno-units --all-features --release --no-fail-fast --lib --test cache --test cache_planner --test cache_reversibility --test cache_admission --test cache_sentinels` (without positional filter, which filters all binaries at once) and it was restored with `git checkout`. The tree was clean after each one.

| Mutation | Guarantee / invariant | Real result |
|---|---|---|
| `cache.rs`, `Dependency::CommitSeq => Some(0)` | G4, I-35 | **dies**: `an_entry_never_serves_another_commit_seq`, `each_declared_dependency_invalidates_the_entry_and_is_named`, `first_change_vectors`, `dependency_vectors`, `every_transition_leaves_a_record` (lib); `a_commit_invalidates_the_cached_result_of_an_earlier_snapshot` and `cache_on_off_and_invalidated_give_identical_results` in `emulated_log` and `kerno_db` |
| `planner.rs`, `CachedResult::Units(units) => Ok::<_, Violation>(units)` | G5, I-33 | **dies**: `a_failed_check_discards_the_entry_and_the_reference_path_answers` in `emulated_log` and `kerno_db`; `no_literal_in_anything_the_cache_path_reports` in both engines (`test result: FAILED. 8 passed; 2 failed` and `2 passed; 2 failed`) |
| `planner.rs:354`, `if !self.cache.enabled()` → `if false` | I-36, G6 | **survives** (H-7): none of the five test binaries fails |

What was not mutated: the transition log (G7) and the I-37 sentinels were reviewed by reading and by the mutations S-M1 and S-M2 of [E6-04b](stage6-measurement.md) §4, already recorded.

### 3.4 What remains unreviewed

- Concurrency between readers that share the cache: the measurement and the tests are single-threaded.
- Other scales of W1 (only 20,000 units) and other proportions of repetition.

## 4. Measurement (E6-04 and E6-04b, final verdict of the measurement §8)

Percentages of the cache on against off (negative = faster), **median of rounds**, p99 unless stated. The data are the table of [measurement §8](stage6-measurement.md#8-e6-04b-final-verdict-four-runs-2026-11-11), unchanged.

| Load | 5 rounds, run 1 | 5 rounds, run 2 | 20 rounds, run A | 20 rounds, run B | Threshold |
|---|---|---|---|---|---|
| No repetitions, 20,000 units (p99) | +4.0% | −3.8% | +2.9% | **+5.3%** | ≤ +5% |
| No repetitions, 100,000 units (p99) | **+5.3%** | +2.6% | −4.5% | +1.4% | ≤ +5% |
| W1 `w1-index-ready`, 20,000 units (p99) | +0.5% | **+7.8%** | +4.7% | **+7.5%** | ≤ +5% |
| W1 `w1-index-ready`, 20,000 units (p50) | −97.9% | −97.8% | −97.8% | −97.9% | ≤ −20% |

Paired measurement (temporary probe, now removed, of [measurement §8](stage6-measurement.md#8-e6-04b-final-verdict-four-runs-2026-11-11)): the cache is switched on and off every 10 reads over the same snapshot, and means are compared.

| What | Off | On | Difference |
|---|---|---|---|
| Read without repetition, 20,000 (mean) | 819 – 1,213 µs | 823 – 1,216 µs | −0.6% to +0.5% |
| First read after a write in W1 (mean, the one that fixes the p99) | 3,588 – 3,667 µs | 3,720 – 3,834 µs | **+3.4% to +4.9%** (≈ 125 – 180 µs) |

### 4.1 Honest reading

- **p50 of W1: meets** and is not fragile. A read served from the cache costs ≈ 0.03 ms against ≈ 1.3 ms.
- **p99 of W1: cannot be certified.** The read that fixes the p99 is the first after each write, the one the cache cannot serve (G4). Its paired real cost is +3.4% to +4.9%, below 5%, but the dispersion between rounds takes it above in two of four runs. Reducing it requires not copying the result when storing it, which changes the public type `CachedResult`/`QueryOutcome` and the existing tests; it stays outside this stage ([measurement §8](stage6-measurement.md#8-e6-04b-final-verdict-four-runs-2026-11-11)).
- **Worst case without repetitions: the real cost meets it** in the paired measurement (−0.6% to +0.5%). But the p99 of one round run does exceed +5% in two of eight readings (+5.3% at each scale). **It is recorded as an isolated failure of one run, not as a real overhead.**
- **Joint criterion of §6 (net improvement): not claimed.** It requires that p50 and p99 of W1 and the worst case meet at once and stably, and the p99 of W1 does not.

### 4.2 Limits of the measurement

- One machine (4 CPU, background load ≈ 1). W1 only at 20,000 units. No concurrency. No variation of the cache size (`max_entries`, `max_bytes`).
- The command of the final run is `index_bench -- --cache`; the long bench was not repeated in this stage, and the numbers are taken from the committed output (`stage6-measurement-output.txt`).
- No threshold or parameter of the measurement was changed in E6-05: only documents and comments were touched.

## 5. Verification of E6-05

| Check | Command | Result |
|---|---|---|
| Suite of `kerno-units` on the final tree (after the comment changes) | `cargo test -p kerno-units --all-features --release` | `exit=0`; **320 passed, 1 ignored**; each binary `test result: ok` |
| Format | `cargo fmt --all --check` | `fmt_exit=0` |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `Finished dev profile` in 20.43 s; `clippy_exit=0` |

The tests of `kerno-units` were green before E6-05 (320 passed), and remain so after the documentation and comment changes of `cache.rs`.

## 6. Open decisions for the owner

1. **Cache disabled by default** (minutes §8.2). It stays: the net improvement of §6 is not claimed.
2. **H-3**: does the creation evidence come from the observer, or is it enough to document that it is the execution that produced the answer?
3. **H-5**: is `prune_entries` also called in `stats()` and `records()`, to meet the age bound at rest? It is one line per function and needs a test.
4. **H-6**: is the cache cause carried to `ExecutionObservation` (change of `FallbackCause`), or does it stay in the transition log?
5. **H-7**: is the I-36 test accepted in its results and counters part, or is an injection point requested to observe the index read?
6. **What to do about the p99 of W1**: accept the closure with the cache off and the measurement documented, or repeat with more rounds or with the copy avoided (change of public type).

## 7. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-11 | **Approved by delegation** | The owner delegated the decisions ("the best and most correct, take the reins yourself"; "I am not going to review it, do it yourself"). Decisions of §6: the closure is accepted with the cache **disabled by default** and the measurement documented (the p99 of W1 is not certified; avoiding the copy remains possible work for a later stage); H-5, H-6 and H-7 are accepted as recorded risks, with no code change in this stage; the rest of the open points of §6 pass as an inherited risk to the next stage. The owner may revoke this approval |
