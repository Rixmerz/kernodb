# Entry minutes of Stage 4: relations and indexes

| Field | Value |
|---|---|
| Date | 2026-11-04 (draft) |
| Status | Approved (2026-11-04); closure of the stage approved on 2026-11-05 ([stage4-closure.md](stage4-closure.md)) |
| Act | Approval of the scope and of packages E4-00 to E4-05 by the project owner |
| Documents | [Roadmap, Stage 4](../roadmap/README.md#stage-4--relations-and-indexes) · [closure of Stage 3](stage3-closure.md) · [ADR-0014](../adr/0014-physical-representation-of-relations.md) · [ADR-0007](../adr/0007-index-consistency-and-rebuilding.md) · [invariants](invariants.md) · [objectives T1–T10](../testing/f1-performance-goals.md) |

## 0. Status

These minutes were **approved by the owner on 2026-11-04** (see §8). No later WP starts before the previous one, as in Stage 3. E4-00…E4-04 are merged (PR #16–#20). E4-05 (closure) is done (PR #21) and **approved by the owner on 2026-11-05**: [stage4-closure.md](stage4-closure.md).

## 1. Start condition

The roadmap requires for Stage 4 a stable units API (ADR-0010 accepted). Status checked on 2026-11-04:

| Condition | Status |
|---|---|
| Stable units API (ADR-0010 accepted) | Met: ADR-0010 **Accepted** on 2026-10-31 ([minutes E3-00](stage3-record-e3-00.md)). The public API of `kerno-units` was closed with Stage 3 |
| Stage 3 closed | Met: [closure of Stage 3](stage3-closure.md), approved by the owner on 2026-11-04. Inherited with risks K-E3-8 and K-E3-9 |

## 2. Scope

### 2.1 Objectives

1. Declare typed relations between units ([ADR-0014](../adr/0014-physical-representation-of-relations.md)).
2. Primary index and basic filters.
3. Secondary indexes only when a measured workload justifies them.
4. One reference path for all queries: each index is compared against it.

### 2.2 Starting point

The public API of `kerno-units` has no listing or query by type or by field. It exposes `get_unit` by id, `history`, `history_page`, `unit_as_of` and `state_as_of`; the internal walk of heads exists only for verification and retention. Therefore Stage 4 starts with no index and no public reference path: E4-01 creates it.

### 2.3 Out of scope

- Planner, plan explanation and observability: Stage 5.
- Physical suppression, legal erasure and legal retention: ADR-0009 (K-E3-6).
- Inferred relations: Stage 7. In this stage only the part of I-39 that affects the declared ones is met.
- Multiple writers: the single write slot is a property of the engine (I-06).
- Comparison with SQLite: reserved for Stage 9. In this stage the reference is internal.
- Deltas, chunking of payloads and `AsOf::Time`.

## 3. Work packages

All are sequential. "Done" includes `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the suite of the WP and the full suite green, the test count before and after, and each new test seen to fail by mutation.

| WP | Content | Depends on | Acceptance |
|---|---|---|---|
| **E4-00** Entry gates | Approval of these minutes. Decision recorded on ADR-0007 (today *Pending*) and ADR-0014 (today *Proposed*), and on the points of §7 | Stage 3 closed | Minutes approved by the owner; status of ADR-0007 and ADR-0014 updated in `docs/adr/`; no later WP starts before |
| **E4-01** Reference path | Full walk over a `UnitSnapshot`, without indexes, which returns the correct result by definition for each query of the stage | E4-00 | Tests on `Emulated<LogEngine>` and `KernoDb`; no query of the stage is published without its reference path |
| **E4-02** Declared relations (ADR-0014) | Prototype of option 2 (record and two adjacency indexes in the same batch) against option 1 on `kerno-storage`, with criteria 1–5 of ADR-0014 fixed before measuring. Semantics of archiving or deleting a unit with relations (criterion 4) | E4-01 | Prototype and comparison recorded in ADR-0014 ("Evidence"); every relation carries mandatory provenance; new rows in the [failure matrix](failure-matrix.md) for record and indexes (I-21); U2 and U6 re-measured with the thresholds already approved, without changing them |
| **E4-03** Primary index and basic filters | Primary index and filters over the reference path. Freshness status `ready`/`building`/`stale`/`failed` with watermark | E4-01, E4-02 | Results equal to the reference path over the same snapshot (I-32 applied to indexes); rebuilt index equal to a new one (I-24); status and watermark exposed (I-23) |
| **E4-04** Secondary indexes | Only those that a measured workload justifies. Each one has its workload written before it is built | E4-03 | Each secondary index has a documented workload and equivalence with the reference path; no index without a workload |
| **E4-05** Closure | Adversarial review of the stage. Promotion of the stage's invariants. Update of the roadmap and of SAD §9 | all | Criteria of §4 met; review report with no open blockers |

Critical path: E4-00 → E4-01 → E4-02 → E4-03 → E4-04 → E4-05.

## 4. Acceptance criteria

From the [roadmap, Stage 4](../roadmap/README.md#stage-4--relations-and-indexes):

| Criterion | Planned evidence | Status |
|---|---|---|
| Results per index match the reference path (I-32 applied to indexes) | Differential tests index against E4-01 over the same snapshot | met in E4-03 (`tests/index.rs`: `differential_seeded`, `differential_across_scan_pages`; [invariants](invariants.md) §2.2) |
| A rebuilt index is equivalent to a new one (I-24) | Rebuild test against fresh construction | met in E4-03 (`tests/index.rs`: `rebuilt_equals_fresh`) |
| The freshness status is exposed (I-23) | Test of states and watermark, including after a crash | met in E4-03 (`tests/index.rs`: `states_and_watermark`, `index_every_point_*`, [failure matrix](failure-matrix.md) §9) |
| Each secondary index has a documented workload and equivalence with the reference path; no index without a workload (E4-04) | Workload written before measuring, measurement, differential tests per index | met in E4-04 ([stage4-workloads.md](stage4-workloads.md): W1 justifies `IDX_TYPE`; `tests/index.rs`: `check` compares `IndexPath::Primary` and `IndexPath::ByType` with the reference in `differential_*`, `rebuilt_equals_fresh`, `index_every_point_*`; UNIT-38) |
| A declared relation is never confused with an inferred one (I-39) | Test of mandatory provenance and of the origin field | met in E4-02 and E4-05 (`tests/relations.rs`: `provenance_is_stored_and_checked`, `declared_is_never_confused_with_inferred`, `stored_provenance_corruption_is_detected`; `rel_*` vectors of `struct_vectors.rs`; [invariants](invariants.md) §2.3). Closure: [stage4-closure.md](stage4-closure.md), approved |

## 5. Risks inherited from Stage 3

| ID | Risk in Stage 4 | Proposed treatment |
|---|---|---|
| **K-E3-8** | U2 fails on `history`: 1.50× (1.503) of the reference in 1 of 3 passes, without changing the threshold. Relations add records to the `ChangeSet`s and must not worsen history reads | Re-measure U2 in E4-02 with the same threshold. Judge U2 on `history_page` only if the owner decides so (decision 12 of Stage 3). No threshold is lowered |
| **K-E3-9** | U6 fails with concurrent retention: write p99 1.05–1.18× and throughput 0.89–0.94× (1.06× in one pass). Each relation writes three records, inside the single write slot (K-E3-2), and may worsen the p99 during pruning | Re-measure U6 with indexes and relations active in E4-02 and E4-03. The failure stays documented while it is not corrected. No threshold is lowered |

K-E3-7 (reads check less than `verify_units`) is not inherited: it was accepted by design, with review recommended in Stage 9.

Merge policy for the stage: the [Phase 1 correctness gate](phase1-design.md#71-correctness-gate-adr-0012-threshold-1-c-25-blocking) is blocking. Performance failures are recorded as risks and do not block, as happened with U2 and U6 in Stage 3.

## 6. Performance thresholds

These minutes **propose no thresholds** for Stage 4. The objectives T1–T10 of [f1-performance-goals.md](../testing/f1-performance-goals.md) do not change and are not lowered. If the owner wants thresholds for relations or indexes, they are fixed before measuring, as was done with U1–U6 in Stage 3 (ADR-0012). SQLite does not take part in this stage.

## 7. Points the owner must decide

1. **ADR-0014 and ADR-0007.** Today they are *Proposed* and *Pending*. ADR-0014 asks for a prototype at the start of the stage. Its final decision comes from E4-02 and is recorded in the ADR.
2. **Assignment of I-32.** [invariants.md](invariants.md) places it in Stage 5, but the Stage 4 roadmap requires it "applied to indexes". It must be decided whether it is promoted in E4-03 or whether the assignment changes.
3. **Assignment of I-23 and I-24.** `invariants.md` places them in "Phase 5" of the SAD; the Stage 4 roadmap requires them. Same treatment as point 2.
4. **Semantics of archived or deleted units with relations** (ADR-0014, criterion 4): rejection, cascade or a marked dangling relation. It is decided in E4-02 and does not depend on ADR-0009 while there is no physical suppression.

## 8. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-04 | **Approved** (E4-00 closed) | The owner approves or adjusts the scope (§2), the packages (§3), the criteria (§4) and the points of §7 before E4-00 starts |

### Owner decisions on §7 (2026-11-04)

1. ADR-0007 **Accepted**; ADR-0014 stays *Proposed* until E4-02.
2. I-32 is promoted in **E4-03** (applied to indexes).
3. I-23 and I-24 are promoted in **E4-03**.
4. Archived or deleted units with relations: **rejection**.
