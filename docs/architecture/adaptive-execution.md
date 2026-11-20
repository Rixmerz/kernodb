# Adaptive execution: `ExecutionObservation` and equivalence with the reference path

Status: **partially implemented**. Stage 5 (planner and observability, `kerno-units`) is closed in [stage5-closure.md](stage5-closure.md) (owner approval pending): the planner of §2 of the specification with the existing paths, the plan explanation and `ExecutionObservation` with levels. Stage 6 (first operational plasticity) is **closed, pending owner approval** ([closure](stage6-closure.md), 2026-11-11): the result cache (`cache`) is implemented and **disabled by default**; its net improvement over the disabled mode is not claimed (the p99 of W1 is not certified). **Not implemented:** learned plans (`learned`), which were not chosen as an optimisation of Stage 6 ([minutes](stage6-record.md) §7, point 1). Roadmap: [roadmap](../roadmap/README.md). Related to: [architecture specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §6.5, §9, §10 and §15; [ADR-0015](../adr/0015-privacy-of-observability.md); [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md); invariants I-32…I-37 in [invariants](invariants.md).

This document fixes **what** is observed and **what** must be testable before a single learned optimisation exists. It does not choose data structures or cost formulas: those were decided by Stage 5 with evidence, in [stage5-planner.md](stage5-planner.md) (costs and paths) and [stage5-observation.md](stage5-observation.md) (levels and retention).

## 1. Terms

- **Reference path.** Simple, correct execution of a query, without learned state: walk through the primary index or full scan, with explicit filtering and ordering. It is the correctness oracle and is always available.
- **Candidate plan.** Valid strategy for a query: a concrete index, a walk order or a reused plan.
- **Learned optimisation.** State derived from observations (reusable plan, result cache, priority of a path) that changes **how** a query is executed, never **what** it returns.
- **Query footprint.** Normalized shape of a query with the literals replaced by typed markers (`unit_type = ?str`). It identifies the shape, not the values.

## 2. `ExecutionObservation`

Optional record, disabled by default, of an execution. Conceptual fields:

| Field | Content | Sensitive data |
|---|---|---|
| `observation_id` | local, increasing identifier of the observer | no |
| `fingerprint` | keyed footprint of the query shape, `units/blake3-keyed/v1/<hex>` or `relations/…` (E5-01) | contains no literals |
| `fingerprint_text` | the shape with typed markers, e.g. `units[unit_type=?text]` | only with level `diagnostic` (ADR-0015) |
| `path` | path that answered (`UnitsReference`, `IndexPrimary`, `IndexType`, `RelationsReference` or `Adjacency`); empty if the query failed before choosing | no |
| `plan_version` | `PLANNER_VERSION` (E5-03). No learned-optimisation version: that belongs to Stage 6 | no |
| `commit_seq` | sequence of the snapshot read (I-04) | no |
| `duration_ns` | duration in nanoseconds, measured with the observer's clock | no |
| `rows_returned` | rows returned | no |
| `outcome` | `Ok` or `Error(class)`, with the class `Corruption`, `Storage`, `Invalid` or `Other`; never the message | only the class |
| `fallback` | `None`, or the abandoned path and the cause: `Corrupt`, or `Invalid` with `NotMatching`, `NotActive` or `OutOfOrder` | no |
| `recorded_at` | timestamp of the record (does not order commits, I-26) | no |

Implemented in E5-03 and E5-04 (`crates/kerno-units/src/observation.rs`). **Not implemented**, with the reason: `plan_id` and `plan_kind` as a field (the path is enough: `ExecutionObservation::plan_kind()` derives `reference` or `planned`; `learned` belongs to Stage 6); `rows_examined` and `bytes_read` (no path counts them, and counting them forces instrumenting the loops, [stage5-observation.md](stage5-observation.md) §2); `cache` (Stage 6); `Outcome` has no `cancelled` variant. The causes `precondition_failed`, `invalidated`, `disabled` and `discrepancy` of the cache (Stage 6, I-33) are recorded in the **cache transition log** (G7), not in the `fallback` field of the observation, which only carries `Corrupt` or `Invalid`; a `discrepancy` appears in the observation as `Invalid` with the abandoned `ResultCache` path. Finding H-6 of the [closure of Stage 6](stage6-closure.md).

Rules:

1. **Never** are parameter values, unit content, complete keys or tokens stored (I-29, ADR-0015).
2. Observations are derived diagnostic data: they do not take part in transactions, they are not replicated in logical exports by default and they can be deleted with no effect on the data.
3. Recording an observation cannot make the observed query fail or block; if recording fails, the query returns its result and the loss is counted (`ObservationStats::dropped`, `evicted`).
4. Retention bounded by size and by age, configurable; by default 1024 records and 1 hour (`ObservationConfig`, E5-03).

## 3. Mandatory metadata of a learned optimisation

Each optimisation records (spec §9): the footprint it applies to, preconditions, schema dependencies, indexes and statistics, cost estimate, observed evidence (number of observations and window), date and version, invalidators and fallback path. An optimisation without any of these fields cannot be activated. **H-3 of Stage 6, closed in P3:** the result cache carries dependencies on schema, planner, index and commit sequence; its creation evidence is what the observer had for that query shape (observations and window, `Observer::evidence_of`), and the entry carries the measured cost, in nanoseconds, of the execution it replaces. Without one or the other the entry is not created, so it cannot be activated; in particular, with the observer below `Fingerprint` there is no evidence and the cache stores nothing. Both data are visible in the transition log (`Created`) and in the plan explanation (`Plan::cache()`, which also says whether the cache was consulted and whether it answers with learned state). Statistics remain covered only by the exact match of the commit sequence ([ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md), "Review of 2026-11-13"). Limit: "cost estimate" is a measurement of a single execution, not a model.

## 4. Equivalence invariants

| ID | Invariant | How it will be tested |
|---|---|---|
| I-32 | **Equivalence of results.** For every query and every snapshot, the chosen plan (planned or learned) returns the same multiset of results as the reference path; if the query fixes an order, the same sequence. | Differential test: each generated query is executed by the reference path and by each candidate plan over the same snapshot (`commit_seq`); also with property tests over random data and queries. **Planner implemented (E5-02):** `tests/index.rs` (`check`, seeds 1–6 and sweeps by pages) and `tests/relations.rs` (`assert_equivalent`) compare `query` and `query_relations` with the reference path, with and without a ready index, and require that the path executed is the one `explain_*` announces. The `learned` plans belong to Stage 6. |
| I-33 | **Correct fallback.** If a precondition is not met, the optimisation is invalidated or disabled, the reference path (or another valid plan) is executed and the result meets I-32; the fallback is recorded: its cause in the cache transition log (`Fallback { cause }`) and, when the cache answers and fails its check, also in the `fallback` field of the observation as `Invalid` with the abandoned `ResultCache` path. The causes `precondition_failed`, `invalidated` and `disabled` are only in the cache log. | Injection of false preconditions and invalidations during the differential test. |
| I-34 | **Invalidating does not alter data.** Activating, deactivating, invalidating or deleting optimisations and observations does not change any confirmed data, revision, relation or transactional index, nor the commit sequence. | Compare the logical state and `commit_seq` before and after each administration operation. |
| I-35 | **Invalidation by dependencies.** A change of schema, of index definition or of relevant statistics invalidates or revalidates the optimisations that depend on it before their next use. | Tests that change each declared dependency and check that the next execution does not use the obsolete plan. |
| I-36 | **Global and per-query deactivation.** With adaptation disabled (globally or for one query), no learned optimisation takes part and the results meet I-32. | Run the full suite with adaptation disabled and compare with the reference path. |
| I-37 | **Observations without sensitive data.** No observation, metric or plan explanation contains query literals or unit content outside the `diagnostic` level. | Test with sentinel data: search for the sentinels in everything exported by observability. **Implemented (E5-04):** `tests/sentinels.rs` searches for the sentinels in observations, counters, explanations and errors, at the four levels, and checks that `fingerprint_text` exists only in `diagnostic`. |

A discrepancy (today: the rows of an entry fail `check_units` / `check_relations` when served; there is no sampled verification against the reference path) records the cause `Discrepancy` in the cache transition log, **deletes the entry** (`DeleteReason::Discrepancy`) and returns the result of the reference path, which may be stored as a new entry. It **does not disable** the optimisation for that query nor globally. See [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md), "Review of 2026-11-13".

## 5. Value criterion (spec §9 and §10)

An optimisation is only kept if it brings measured **net utility**: an improvement in latency or in examined work against its cost of maintenance, memory and storage, with the adaptive mode disabled as the comparison. Frequency alone is not enough. Thresholds are fixed before measuring (ADR-0012).

## 6. Out of scope

Knowledge learning (inferred relations, summaries, consolidation): Stages 7–8 and ADR-0018. A learned plan never modifies content.
