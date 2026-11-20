# Entry minutes of Stage 5: planner and observability

| Field | Value |
|---|---|
| Date | 2026-11-05 |
| Status | Approved by the owner on 2026-11-05 |
| Act | Approval of the scope and of packages E5-00 to E5-05 by the project owner |
| Documents | [Roadmap, Stage 5](../roadmap/README.md#stage-5--planner-and-observability) · [closure of Stage 4](stage4-closure.md) · [adaptive execution](adaptive-execution.md) · [ADR-0015](../adr/0015-privacy-of-observability.md) · [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) · [ADR-0007](../adr/0007-index-consistency-and-rebuilding.md) · [invariants](invariants.md) · spec v0.2.0 §9, §16 |

## 0. Status

These minutes are **approved** (§8). The WPs are sequential, as in Stages 3 and 4.

## 1. Start condition

| Condition | Status |
|---|---|
| Stage 4 closed | Met: [closure of Stage 4](stage4-closure.md), approved on 2026-11-05. Inherited with K-E3-8, K-E3-9 and the recommendation of M-9 |
| Open decision of ADR-0007 (§6 of the closure) | Resolved on 2026-11-05: unit indexes stay derived and only `rebuild_index` writes them; after a write, `query` uses the reference path until the next rebuild |
| ADR-0015 (observability privacy) | *Accepted* on 2026-11-06 with the evidence of E5-04 (§8, decision 1; [report](stage5-sentinels.md)) |

## 2. Scope

### 2.1 Objectives

1. Deterministic planner with the seven steps of spec §9: validate the query, generate valid candidates, estimate cost, choose, execute, record and validate invariants with fallback.
2. Plan explanation, inspectable by the caller.
3. `ExecutionObservation` with detail levels, with no literals or content outside `diagnostic` (ADR-0015, I-37).

### 2.2 Starting point

`kerno-units` already chooses between two paths, with no explicit planner: `UnitSnapshot::query` uses `IndexPath::Auto` (`IDX_TYPE` if the filter fixes `unit_type`, otherwise `IDX_PRIMARY`) when the index is `Ready` and, if not, `reference_filter`; `QueryOutcome::source` says which one. Relations are queried by adjacency (`relations_of`) with their reference path (`reference_relations_of`). There is no estimated cost, no explanation and no observations; the layer emits no metrics (IU-10).

The candidates of this stage are **only the existing paths**: reference path, `IDX_PRIMARY`, `IDX_TYPE` and adjacency. The stage adds no new indexes, paths or queries (spec §16: no function without need).

### 2.3 Out of scope

- Learned optimisations, reusable plans and caches: Stage 6 (ADR-0016, I-33…I-36). The `plan_kind` `learned` is not implemented.
- New indexes, index maintenance in each `ChangeSet` or background rebuild (decision of ADR-0007 of 2026-11-05).
- Query language or text analyser: the query remains `UnitFilter` and the existing relations query.
- Persistence of observations in the engine and export: observations live in memory with bounded retention unless the owner decides otherwise (§7, point 3).
- Comparison with SQLite: Stage 9.

## 3. Work packages

All are sequential. "Done" includes `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the suite of the WP and the full suite green, the test count before and after, and each new test seen to fail by mutation.

| WP | Content | Depends on | Acceptance |
|---|---|---|---|
| **E5-00** Entry gates | Approval of these minutes. Decision recorded on ADR-0015 and on the points of §7. Re-measurement of U6 with active indexes and relations, with the raw output committed (M-9) | Stage 4 closed | Minutes approved; status of ADR-0015 updated in `docs/adr/`; output of `unit_bench` committed; no later WP starts before |
| **E5-01** Footprint and explainable plan | Normalized footprint of `UnitFilter` and of the relations query, with literals replaced by typed markers and hash with algorithm and version recorded (ADR-0015). Plan type and its explanation: candidates considered, estimated cost, chosen one, reason and freshness status of the index (I-23) | E5-00 | The explanation lists all the candidates and the reason for the choice; the footprint is stable for the same shape and contains no literals; fixed footprint vectors |
| **E5-02** Deterministic planner | The seven steps of spec §9 over the existing candidates. Cost with the statistics already available (index status and watermark, type range); no new statistic without a consumer. Validation of the result and fallback to the reference path, recorded. `query` moves to use the planner without changing its result | E5-01 | Same input and same snapshot give the same plan (determinism); differential test planner against reference path in CI (I-32), with the seeds and filters of `tests/index.rs` and `tests/relations.rs`; fallback tested with index `Stale`, `Building` and `Failed` |
| **E5-03** `ExecutionObservation` | Optional record with the fields of [adaptive execution §2](adaptive-execution.md) that apply (without `cache` or `learned`) and the levels `off`, `aggregate`, `fingerprint` and `diagnostic`. Retention bounded by size and age, in memory. Recording never fails or blocks the query | E5-02 | Default level according to §7; `diagnostic` only by explicit activation; a recording failure is counted and the query returns its result; recording cost measured against `off` (ADR-0015, criterion 4) |
| **E5-04** Sentinels | Sentinel test on types, payloads, metadata, provenance and filter values, searched in every observability, explanation and error output, at each level. Threat model review for observations and explanations | E5-03 | No sentinel outside `diagnostic` (I-37); the test dies with the mutation "copy the literal into the footprint or into the explanation" |
| **E5-05** Closure | Adversarial review. Promotion of I-32 (planner) and I-37. Update of the roadmap, of SAD §9 and of `adaptive-execution.md` | all | Criteria of §4 met; review report with no open blockers |

Critical path: E5-00 → E5-01 → E5-02 → E5-03 → E5-04 → E5-05.

## 4. Acceptance criteria

From the [roadmap, Stage 5](../roadmap/README.md#stage-5--planner-and-observability):

| Criterion | Invariant | Planned evidence | Status |
|---|---|---|---|
| The plan can be inspected | — (spec §9, diagnostic mode) | Explanation with candidates, costs, choice and reason (E5-01, E5-02) | pending |
| The chosen plan returns the same as the reference path | I-32 (planner part) | Differential test planner against reference in CI, units and relations, with and without a ready index (E5-02) | pending |
| Fallback to the reference path recorded when the index is not usable | I-23, I-32 | Tests with each freshness status (E5-02) | pending |
| No observation, metric or explanation contains literals or content outside `diagnostic` | I-37, I-29, IU-10 | Sentinel test at all levels (E5-04) | verified in E5-04 ([report](stage5-sentinels.md)) |
| Observing does not alter confirmed data or the commit sequence | I-34 (observations only; the optimisations part is Stage 6) | Test: same reads and same sequence with observation `off` and `diagnostic` (E5-03) | verified in E5-03 ([report](stage5-observation.md)) |

## 5. Risks inherited

| ID | Risk in Stage 5 | Proposed treatment |
|---|---|---|
| **K-E3-8** | U2 fails on `history`. The planner does not touch `history` | No action unless the owner decides; no threshold is lowered |
| **K-E3-9** | U6 fails with concurrent retention; not re-measured with indexes (M-9) | Re-measure in E5-00 and commit the output. Observations do not write to the engine, so they should not affect U6 |
| **M-11** | Intermittent test `retention_every_point_paced_power_loss_sync` | Can break the full suite at random; a failure of that test is not attributed to a WP without reproducing it |
| **New** | The planner adds cost to each `query` (estimating and explaining) | Measure W1 with the planner against the current path in E5-02; threshold only if the owner sets it before (§6) |
| **New** | A footprint of few values admits dictionary attacks (ADR-0015) | Hash with a local key not exported, if the owner decides it (§7, point 4) |

Merge policy: the [Phase 1 correctness gate](phase1-design.md#71-correctness-gate-adr-0012-threshold-1-c-25-blocking) is blocking. Performance failures are recorded as risks and do not block.

## 6. Performance thresholds

These minutes **propose no thresholds**. T1–T10 and U1–U6 do not change and are not lowered. ADR-0015 (criterion 4) asks that Stage 5 fix the threshold for the cost of recording an observation against the p99 of the observed query: if the owner wants it, it is fixed before measuring (§7, point 5).

## 7. Points the owner must decide

1. **ADR-0015.** It is *Proposed*. Is it accepted (option 2, levels with redaction) in E5-00, before E5-03, or accepted with the evidence of E5-04? ADR-0016 and ADR-0018 may stay *Proposed*: they are not used in this stage.
2. **Default level of the observations.** ADR-0015 proposes `off`. Alternative: `aggregate` (counters without text), useful for Stage 6 without exposing literals.
3. **Where the observations live.** Proposal: in memory, with retention by size and age and default values fixed in E5-03. Persisting them requires a format and ADR-0009 (suppression), out of scope.
4. **Hash of the footprint.** Versioned hash without a key, or hash with a local key not exported (ADR-0015, dictionary attacks).
5. **Thresholds.** Whether a threshold is fixed for the cost of observation (ADR-0015, criterion 4) and for the planner overhead on W1, before measuring.
6. **Assignment of I-34.** [invariants.md](invariants.md) places it in Stage 6; these minutes propose verifying in E5-03 only the observations part.

## 8. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-05 | **Approved** | Scope, packages and criteria unchanged. Decisions of §7: (1) ADR-0015 is accepted with the evidence of E5-04; (2) default level `off`; (3) observations in memory only, with retention by size and age; (4) footprint hash with a local key not exported; (5) thresholds fixed before measuring: cost of recording an observation ≤ 5% of the p99 of the observed query, planner overhead on W1 ≤ 10%; a failure is recorded as a risk and does not block; (6) E5-03 verifies the observations part of I-34 |
