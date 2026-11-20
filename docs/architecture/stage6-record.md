# Entry minutes of Stage 6: first operational plasticity

| Field | Value |
|---|---|
| Date | 2026-11-09 |
| Status | **Approved** on 2026-11-09 (decision delegated by the owner) |
| Act | Proposal of scope and of packages E6-00 to E6-05 |
| Documents | [Roadmap, Stage 6](../roadmap/README.md#stage-6--first-operational-plasticity) · [closure of Stage 5](stage5-closure.md) · [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) (accepted, G1–G7) · [ADR-0015](../adr/0015-privacy-of-observability.md) · [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) · [adaptive execution](adaptive-execution.md) · [invariants](invariants.md) · spec v0.2.0 §9, §15.5, §16 |

## 0. Status

These minutes are **approved** (§8). The WPs are sequential, as in Stages 3 to 5.

## 1. Start condition

| Condition | Status |
|---|---|
| Stage 5 closed | Met: [closure](stage5-closure.md) approved on 2026-11-09, with the promotion of I-32 (planner) and I-37 and the limitations H-1 and H-2 |
| ADR-0016 (adaptation and invalidation) | *Accepted* on 2026-11-09, subject to G1–G7 |
| ADR-0018 (versioning of derived data) | *Proposed*. Only needed if the learned state is persisted (§7, point 3) |
| Narrative threads experiment | Closed with NO-GO ([report](../../experiments/narrative-threads/results/REPORT.md)); it does not affect this stage or the persistent format |

## 2. Scope

### 2.1 Objectives

1. **A single** reversible optimisation (roadmap): a reusable plan **or** a result cache invalidated by commit sequence. Which of the two is a decision of the owner (§7, point 1).
2. The metadata of spec §9 for that optimisation: declared dependencies with version, creation evidence, version of the optimisation and of the planner.
3. Global and per-query deactivation (I-36), invalidation by dependency before each use (I-35, G4) and recorded fallback (I-33).
4. Net measured improvement against the disabled mode, with thresholds fixed before measuring.

### 2.2 Starting point

After Stage 5, `kerno-units` has a deterministic planner (`planner.rs`) over four paths (reference, `IDX_PRIMARY`, `IDX_TYPE`, adjacency), an explainable `Plan`, a footprint with a local key (`fingerprint.rs`) and `ExecutionObservation` in memory with levels `off` to `diagnostic`. `check_units` and `check_relations` validate partial correctness, not completeness (H-1). The fallback causes `precondition_failed`, `invalidated`, `disabled` and `discrepancy` and the `plan_kind` `learned`/`cache` exist only in the design ([adaptive execution](adaptive-execution.md)).

### 2.3 Out of scope

- A second optimisation, or any optimisation that changes the result or approximates it.
- The exception "unless proof that the data read did not change" of the cache (removed by G4 until a later ADR).
- New indexes or paths, statistics without a consumer (spec §16).
- Persisting the learned state, unless the owner decides (§7, point 3).
- Narrative or associative retrieval: Stage 7; the threads experiment is not integrated.

## 3. Work packages

All are sequential. "Done" includes `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, the suite of the WP and the full suite green, the test count before and after, and each new test seen to fail by mutation.

| WP | Content | Depends on | Acceptance |
|---|---|---|---|
| **E6-00** Entry gates | Approval of the closure of Stage 5 and of these minutes. Decisions of §7 recorded. Re-measurement of the planner and of `diagnostic` on an unloaded machine (inherited risk of the closure §5), with raw output committed. Baseline measurement of the disabled mode for the load that E6-04 will use | Stage 5 closed | Minutes approved; thresholds of §6 fixed; outputs committed; no later WP starts before |
| **E6-01** Dependency model and learned state | Type of optimisation with its versioned dependencies (G4): schema version, index definition and its watermark, statistics, preconditions and, for a cache, the `commit_seq` of its snapshot. Store of the learned state separate from user data, outside every transaction (ADR-0016, decision). Global and per-query switches. Transition log (G7) with the rules of ADR-0015 | E6-00 | Each declared dependency has a test that changes it and invalidates the optimisation (I-35); fixed dependency vectors; no dependency without a version |
| **E6-02** Use in the planner | The optimisation as a candidate of the planner: used only if the current preconditions are checked at that use (G3); its answer goes through `check_units`/`check_relations` (G5); any failure, discrepancy or doubt executes the reference path with the cause recorded (`precondition_failed`, `invalidated`, `disabled`, `discrepancy`) (G2, I-33). The explanation says whether it used learned state and which (G7) | E6-01 | Differential test with the optimisation enabled, disabled and invalidated midway through execution (G1, I-32); injection of false preconditions and of corrupt state (G2, G3); mutations that remove a check on the learned path die (G5) |
| **E6-03** Reversibility and deactivation | Activating, deactivating, invalidating and deleting the state does not touch confirmed data, revisions, relations, indexes or `commit_seq` | E6-02 | I-34 (optimisations part) and I-36: full suite with adaptation disabled; logical state, engine bytes and `commit_seq` equal before and after; with adaptation disabled no optimisation takes part (counter at zero) |
| **E6-04** Measurement and sentinels | Net improvement against the disabled mode with the load and thresholds of §6. Sentinels (I-37) in new records, explanations and errors, at each level. Test that walks each transition of G7 and finds its record | E6-03 | Measurements with raw output committed; sentinels green and killed by the mutation "copy a literal into the optimisation's record" |
| **E6-05** Closure | Adversarial review. Promotion of I-33…I-36 (and of I-34 in full). Update of the roadmap, SAD §9 and `adaptive-execution.md` | all | Criteria of §4 met; review report with no open blockers |

Critical path: E6-00 → E6-01 → E6-02 → E6-03 → E6-04 → E6-05.

## 4. Acceptance criteria

From the [roadmap, Stage 6](../roadmap/README.md#stage-6--first-operational-plasticity) and ADR-0016:

| Criterion | Invariant / guarantee | Planned evidence | Status |
|---|---|---|---|
| Net improvement measured against the disabled mode, with thresholds fixed beforehand | — | E6-04, thresholds of §6 | **not claimed** (E6-05): p50 of W1 meets it; p99 of W1 is not certified; the cache stays disabled by default ([closure](stage6-closure.md) §4) |
| Equivalence of results | I-32, G1 | Differential test enabled / disabled / invalidated midway (E6-02); does not rely on completeness checks at query time (H-1) | **met** (E6-02; [closure](stage6-closure.md) §2) |
| Fallback to a valid plan, recorded, on precondition, invalidation, deactivation or discrepancy | I-33, G2, G3 | Injection of failures and falsified observations (E6-02) | **met** (E6-02; recorded in the cache, not in the observation: H-6) |
| Invalidation proven for each dependency | I-35, G4 | One case per declared dependency (E6-01) | **met** (E6-01) |
| Invalidating, deactivating or deleting does not alter data | I-34, G6 | E6-03 | **met** (E6-03) |
| Deactivation verified | I-36, G6 | E6-03 | **met** (E6-03); the cost part of the mutation "cache switched off" survives (H-7) |
| The learned path passes the same validations | G5 | Mutations (E6-02) | **met** (E6-05, mutation of `check_units` killed) |
| Traceability without literals | G7, I-37 | Walk of transitions and sentinels (E6-04) | **met** (E6-04, E6-04b) |

Status of the stage (E6-05): **closed, pending owner approval** ([closure](stage6-closure.md)). The net improvement is not claimed, so the cache stays disabled by default (§8, decision 2).

## 5. Risks

| ID | Risk | Proposed treatment |
|---|---|---|
| K-E3-8, K-E3-9 | U2 on `history`; U6 with concurrent retention | No change; this stage does not write to the engine. No threshold is lowered |
| Inherited (closure of E5 §5) | Cost of the planner and of `diagnostic` with isolated failures | Re-measure in E6-00 on an unloaded machine |
| H-1 (inherited) | In-query checks do not guarantee completeness | The equivalence of the optimisation is shown only with the differential test (G1) |
| **New** | The gain is small: the planner costs little (+0.3% to +14% of p99), so a reusable plan saves at most that margin; a cache tied to `commit_seq` only hits without intermediate writes | Choose the optimisation with data from E6-00 (§7, point 1); if the improvement does not reach the threshold, the stage closes with the negative result documented and the optimisation disabled by default |
| **New** | The learned state grows without bound | Bound by size and age, as the observations (E5-03) |
| **New** | A result cache keeps user content in memory | Outside the scope of observability (it is not a record), but within ADR-0009 (suppression) if persisted: hence the proposal is memory only |

Merge policy: the [Phase 1 correctness gate](phase1-design.md#71-correctness-gate-adr-0012-threshold-1-c-25-blocking) is blocking. Performance failures are recorded as risks and do not block, except the net improvement criterion, which decides whether the optimisation is enabled by default.

## 6. Performance thresholds

T1–T10 and U1–U6 do not change. Proposal, to be fixed in E6-00 before measuring:

- Net improvement: p50 of the repeated-read load of W1 with the optimisation ≥ 20% better than disabled; p99 no worse than disabled by more than 5%.
- With a load without repetition (worst case), overhead ≤ 5% of p99 against disabled.
- Writes: no measurable change in U1 (the optimisation does not write to the engine).

## 7. Points the owner must decide

1. **Which optimisation.** (a) Reusable plan by footprint: invalidates with schema, index and statistics versions; saves only the cost of planning. (b) Result cache by footprint and `commit_seq`: invalidates exactly by sequence (G4); hits only on repeated reads without writes in between. **Recommendation: (b)**, because its invalidation is the simplest to demonstrate (one exact dependency) and its potential gain is greater than the cost of the planner; the final decision rests on the baseline measurement of E6-00.
2. **Default state.** Proposal: disabled by default until E6-04 shows the net improvement; enabled only if it meets §6.
3. **Where the learned state lives.** Proposal: in memory, with a bound by size and age. Persisting it requires ADR-0018 accepted and ADR-0009 (suppression).
4. **Thresholds of §6**, fixed before measuring.
5. **Measurement load.** Proposal: W1 with a fixed proportion of repeated queries (for example 80% repeated and 20% interleaved writes), plus the load without repetition as the worst case.

## 8. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-09 | **Approved** | The owner delegated the decisions ("the best and most correct, take the reins yourself"). Decisions of §7: (1) result cache by footprint and `commit_seq` (option b), to be confirmed with the baseline measurement of E6-00: confirmed by the [re-measurement of E6-00](stage6-e600-remeasurement.md): the planner costs ≈ 0% at p50, so a reusable plan cannot reach §6; (2) disabled by default until §6 is met; (3) state in memory only, with a bound by size and age; (4) thresholds of §6 as they are, fixed before measuring; (5) load: W1 with 80% repeated reads and 20% interleaved writes, plus the load without repetition as the worst case |
