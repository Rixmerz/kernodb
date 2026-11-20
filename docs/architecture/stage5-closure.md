# Closure of Stage 5

| Field | Value |
|---|---|
| Status | **Approved** on 2026-11-09 (decision delegated by the owner) |
| Date | 2026-11-06 (E5-05) |
| Act | Adversarial review and closure of Stage 5 (minutes §3, row E5-05) |
| Scope | Packages E5-00 to E5-05: entry gates, explainable footprint and plan, deterministic planner, `ExecutionObservation`, sentinels and closure, in the crate `kerno-units` |
| Documents | [Minutes of Stage 5](stage5-record.md) · [footprint and plan](stage5-footprint-and-plan.md) · [planner](stage5-planner.md) · [observation](stage5-observation.md) · [sentinels](stage5-sentinels.md) · [re-measurement of U6](stage5-u6-remeasurement.md) · [adaptive execution](adaptive-execution.md) · [ADR-0015](../adr/0015-privacy-of-observability.md) · [invariants §2.2, §2.4](invariants.md) · [roadmap](../roadmap/README.md#stage-5--planner-and-observability) · [closure of Stage 4](stage4-closure.md) |

This document describes the stage as it stands on the branch after E5-05. **The closure is not effective until the owner approves it** (§7). The promotion of I-32 (planner part) and I-37 also remains pending that approval.

## 1. Work packages delivered

| WP | Delivery | Commit | Status |
|---|---|---|---|
| E5-00 | Minutes approved (2026-11-05); decisions of §7 recorded; U6 re-measured with indexes and relations, raw output committed (closes M-9) | `6c4cf0f`, `29744e1`, `d456cb0` | done |
| E5-01 | Keyed footprint of the query shape (`fingerprint.rs`) and explainable plan (`plan.rs`) | `7bdf9b8` | done |
| E5-02 | Deterministic planner of the seven steps of spec §9 over the existing paths; `query` and `query_relations` use it; fallback recorded | `88a55ca` | done |
| E5-03 | `ExecutionObservation` with levels `off`, `aggregate`, `fingerprint`, `diagnostic` and retention bounded in memory | `878f541` | done |
| E5-04 | Sentinels in observations, explanations and errors, at the four levels | `cf7bf14` | done |
| E5-05 | Adversarial review (§3), criteria (§2), promotion of I-32 (planner) and I-37, roadmap, SAD §9 and `adaptive-execution.md` | this change | done, **pending approval** |

ADR-0015 was accepted on 2026-11-06 with the evidence of E5-04 (minutes §8, decision 1). ADR-0016 and ADR-0018 remain *Proposed*: they are not used in this stage.

## 2. Acceptance criteria (minutes §4)

| Criterion | Evidence | Status |
|---|---|---|
| The plan can be inspected | `UnitSnapshot::explain_filter` and `explain_relations` return the `Plan`: candidates with cost and usability, chosen one, reason and freshness status (`tests/plan.rs`, 10 tests; `planner.rs`, `candidates_carry_the_usability_the_state_gives`, `costs_follow_the_documented_model`). See [planner](stage5-planner.md) §1 and [footprint and plan](stage5-footprint-and-plan.md) §2 | **met** |
| The chosen plan returns the same as the reference path (I-32, planner part) | `tests/index.rs` (`check`, seeds 1–6, `differential_across_scan_pages`) compares `query` with `reference_filter` with and without a ready index, and requires that the path executed is the one `explain_filter` announces and `fallback == None` in a healthy store. `tests/relations.rs` (`assert_equivalent`) does the same with `query_relations`. Mutations of §2.4 | **met** |
| Fallback to the reference path recorded when the index is not usable (I-23, I-32) | `Stale` and `Failed`: `planner_falls_back_when_the_index_is_not_ready`, with all filters. Wrong answer or failure of an index path: `a_failing_or_wrong_index_path_falls_back_to_the_reference` and `a_failing_or_wrong_adjacency_falls_back_to_the_reference`, with one case per `Violation` and per `Corrupt`. `Building`: see H-2 (§3) | **met with one caveat** (H-2) |
| No observation, metric or explanation contains literals or content outside `diagnostic` (I-37, I-29, IU-10) | `tests/sentinels.rs`: `no_sentinel_in_any_observability_explanation_or_error_output` and `levels_below_diagnostic_carry_no_shape_text`, on `Emulated<LogEngine>` and `KernoDb`. Mutations of [sentinels](stage5-sentinels.md) §4 and of §2.4 | **met** (verified in E5-04) |
| Observing does not alter confirmed data or the commit sequence (I-34, observations only) | `tests/observation.rs`: `observing_does_not_change_the_data`, with `Off` and `Diagnostic`, on two engines; it matches answers, commit sequences and engine bytes ([observation](stage5-observation.md) §5) | **met** (verified in E5-03; the optimisations part belongs to Stage 6) |

Performance criterion of E5-02 and E5-03 (minutes §8, decision 5): **it is not an acceptance criterion**; it is recorded as a risk (§5).

## 3. Adversarial review (E5-05)

Method: the same as in Stages 3 and 4. For each invariant of the stage the code that enforces it was read, the test that fails if it breaks was located and a mutation was applied. The mutations were made on the production code of the working tree, each with an exact `sed` inverse that was checked with `git diff` (tree clean after each one). E5-05 adds no new tests: the mutations are on existing tests.

**Result: no open blockers.** There are two findings the owner must know about (H-1 and H-2) and two minor ones (H-3 and H-4).

| # | Finding | Evidence | Status |
|---|---|---|---|
| H-1 | `check_units` and `check_relations` verify that the answer of an index path is a correct subset (what the filter selects, only `Active`, strictly ascending order), but **not that it is complete**. An index path that dropped a unit the filter does select would return a wrong answer with no fallback at query time. The differential test in CI and `verify_units` (UNIT-37) detect it, not the query. It is the same limitation that [invariants §2.2](invariants.md) ("Known limitation") already recorded | Reading of `planner.rs` (`execute_units`, `check_units`, `execute_relations`, `check_relations`). Inferential | **Accepted as a known limitation.** Checking completeness at query time requires the full reference path and cancels the index. Stage 6 must not count on that check for its result equivalence (I-32 relies on the differential test) |
| H-2 | The fallback with index `Building` is not exercised through `query`. A snapshot does not see `Building` (only `UnitDb::index_status` reports it, [planner](stage5-planner.md) §4), so the acceptance of E5-02 ("fallback tested with `Stale`, `Building` and `Failed`") is met for `Building` only at the level of candidates (`candidates_carry_the_usability_the_state_gives`) | Reading of `index.rs` (`UnitSnapshot::index_status`) and of `planner.rs`. Inferential | **Open, minor.** Recorded as a caveat of §2. Does not block: the behaviour of a rebuild in progress is covered by the tests of `tests/index.rs` (`building_is_reported_while_a_build_waits`) |
| H-3 | An observation record is lost and counted (`dropped`) when the lock is busy; it is not waited for | [observation](stage5-observation.md) §3 and §9 | Accepted: it is the design |
| H-4 | Retention by age checks only the front of the queue: with a clock that goes backwards, an old record can stay behind newer ones until the size bound evicts it | `observation.rs`, `ponytail:` note of the retention; [observation](stage5-observation.md) §4 | Accepted, with its documented upgrade path (walk the queue on read) |

### 3.1 Mutations of the review

Command of each mutation: `sed -i` on the file indicated; the tests: `cargo test -p kerno-units --all-features --release --no-fail-fast` with the filter indicated. Real result of each, copied from the output:

| Mutation (row of the table of §2.4) | Tests | Real result |
|---|---|---|
| `planner.rs`: `execute_units` accepts the index path without `check_units` (`match Ok::<(), Violation>(())`) | `--lib planner` | `test planner::tests::a_failing_or_wrong_index_path_falls_back_to_the_reference ... FAILED`; `test result: FAILED. 6 passed; 1 failed` |
| `planner.rs`: the path `IndexPrimary` is never executed (`PlanPath::IndexPrimary => None`) | `--test index` | `test result: FAILED. 12 passed; 14 failed` (`differential_seeded`, `differential_across_scan_pages`, `rebuilt_equals_fresh`, `index_path_refuses_what_it_can_see`, among others) |
| `observation.rs`: the shape text comes out from `Fingerprint` (`fingerprint_text: (level >= ObservationLevel::Fingerprint)`) | `--test sentinels --test observation` and `--lib observation` | `queries_report_what_ran` (×2, `observation.rs:173`) and `levels_below_diagnostic_carry_no_shape_text` (×2, `sentinels.rs:317`) FAILED; in `--lib`, `fingerprint_records_the_hash_and_diagnostic_adds_the_text` FAILED |

All three were restored with the inverse of `sed`, and `git status` was left with only the documents of this change.

Reading of the third: the sentinel search (`no_sentinel_in_any_observability_explanation_or_error_output`) stays green with that mutation, because the shape text is markers (`units[unit_type=?text]`) and contains no literals. The structural test `levels_below_diagnostic_carry_no_shape_text` catches it. That is why E5-04 has both.

## 4. Verification of the closure gate

Commands and real result, on the branch, after E5-05:

```text
$ cargo fmt --all --check
(no output; exit code 0)

$ cargo clippy --workspace --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.48s
(exit code 0)

$ cargo test --workspace --exclude kerno-storage --all-features --release
passed=615 failed=0 ignored=8

$ cargo test -p kerno-storage --all-features --release
passed=649 failed=0 ignored=4
```

Test count (sum of the `test result` lines, aggregated):

| Suite | Before (E5-04, `cf7bf14`) | After (E5-05) |
|---|---|---|
| `cargo test --workspace --exclude kerno-storage --all-features --release` | 615 pass, 8 ignored | 615 pass, 8 ignored |
| `cargo test -p kerno-storage --all-features --release` | 649 pass, 4 ignored | 649 pass, 4 ignored |
| Full suite | **1264** | **1264** |

E5-05 adds no tests; the count does not change. `kerno-units` alone: 259 tests in E5-03; it was not counted separately again.

## 5. Risks and measurements

| ID | Risk | Status at closure |
|---|---|---|
| K-E3-8 | U2 fails on `history` | **Open**, unchanged: the planner does not touch `history` |
| K-E3-9 | U6 fails with concurrent retention | **Open**: re-measured with indexes and relations in E5-00 (`stage5-u6-remeasurement.md`): write p99 1.05–1.18×, throughput 0.89–0.94×. It still fails and **does not block** (minutes §5). Observations do not write to the engine |
| M-11 | Intermittent test `retention_every_point_paced_power_loss_sync` | Open, unchanged since E4-05 (see [closure of Stage 4](stage4-closure.md) §4). Not reproduced in the runs of E5-05 |
| M-9 | U6 without indexes | **Closed** in E5-00 |
| Planner cost (minutes §8.5, ≤ 10% on W1) | Measured in [planner](stage5-planner.md) §7: p99 +8.5% at 100,000 units; in the measurement of E5-03 (run 1), +14.1% at 20,000 and +2.8% at 100,000; run 2 does not reproduce the +14.1% (+0.3%) | **Isolated failure recorded as a risk, not blocking** (minutes §5, §8). Noise of a shared machine is more likely than a real overhead; repeating on an unloaded machine before Stage 6 is proposed |
| Cost of observing (minutes §8.5, ≤ 5% of the p99 unobserved) | `diagnostic` at 20,000 units, run 1: +5.9%; the other seven measurements meet it ([observation](stage5-observation.md) §7) | **Isolated failure recorded as a risk, not blocking** |
| Footprint with dictionary (ADR-0015) | Local key not exported, decision 4 of the minutes | **Mitigated** |

## 6. Decisions the owner takes

- **Approval of this closure**, of the promotion of I-32 (planner part) and I-37 in [invariants.md](invariants.md) §2 and §2.4, and of the acceptance of the limitations H-1 and H-2 (§3).
- **Repeat the measurement of the planner and of `diagnostic`** on an unloaded machine, if the owner wants to close the two performance risks of §5 before Stage 6. No threshold is changed.

## 7. Closure

| # | Closure condition | Status |
|---|---|---|
| 1 | Criteria of the minutes §4 met, with evidence (§2) | done (one criterion with a caveat, H-2) |
| 2 | Adversarial review with no open blockers (§3) | done |
| 3 | Promotion of I-32 (planner) and I-37 in [invariants.md](invariants.md) | done, **pending owner approval** |
| 4 | Roadmap, SAD §9 and `adaptive-execution.md` updated | done in the change that incorporates this document |
| 5 | Owner approval of this closure, of the promotion and of the limitations (§6) | **done** on 2026-11-09: the owner delegated the decision ("take the reins yourself"); the closure, the promotion of I-32 (planner) and I-37 and the limitations H-1 and H-2 are approved. The re-measurement of the planner and of `diagnostic` moves to E6-00 |

## 8. Next stage

Stage 6 (first operational plasticity) starts only when the owner approves this closure. It inherits:

- K-E3-8 and K-E3-9, and the cost risk of the planner and of `diagnostic` (§5).
- H-1: Stage 6 cannot support the equivalence of a learned optimisation on a completeness check at query time; it must use the differential test.
- I-33…I-36 and I-34 (optimisations part), which remain in Stage 6.
- The `learned` plans, `cache`, and the fallback causes `precondition_failed`, `invalidated`, `disabled` and `discrepancy` of `ExecutionObservation`, which were not implemented in this stage (minutes §2.3; [adaptive execution](adaptive-execution.md)).
