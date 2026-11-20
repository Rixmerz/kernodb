# Stage 7 entry minutes: associative memory

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Approved** on 2026-11-11 (decision delegated by the owner; revocable) |
| Closure | [Closure of Stage 7](stage7-closure.md), 2026-11-12: **closed and approved by the owner on 2026-11-12**. No open blockers |
| Act | Proposal of scope and of work packages E7-00 to E7-06 |
| Documents | [Roadmap, Stage 7](../roadmap/README.md#stage-7--associative-memory) · [Stage 6 closure](stage6-closure.md) · [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) (accepted on 2026-11-11) · [evaluation set](stage7-evaluation-set.md) · [associative query (E7-01)](stage7-associative-query.md) · [indexed execution (E7-02)](stage7-indexed-execution.md) · [temporal context and provenance (E7-03)](stage7-context-and-provenance.md) · [ADR-0014](../adr/0014-physical-representation-of-relations.md) · [ADR-0015](../adr/0015-privacy-of-observability.md) · [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) (proposed) · [invariants](invariants.md) · spec v0.2.0 §11, §12, §16 |

## 0. Status

These minutes are **approved** (§8). The WPs are sequential, as in Stages 3 to 6. The stage is closed and approved by the owner on 2026-11-12 ([closure](stage7-closure.md)): the decisions on the start of Stage 8 and on T4 are in §8 of the closure.

## 1. Start condition

| Condition | Status |
|---|---|
| Stages 4 and 5 closed (roadmap: relation traversals and plan explanation available) | Met: [Stage 4 closure](stage4-closure.md) and [Stage 5 closure](stage5-closure.md), approved |
| Stage 6 closed | Met: [closure](stage6-closure.md) approved by delegation on 2026-11-11; the result cache stays off by default |
| ADR-0011 (vectors and hybrid retrieval) | **Met**: accepted on 2026-11-11 in E7-00, with option 1 (§7, point 1). See [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) and the [evaluation set](stage7-evaluation-set.md) |
| ADR-0018 (versioning of derived data) | *Proposed*. Not needed: this stage does not persist derived data (§2.3) |
| Narrative threads experiment | Closed as NO-GO ([report](../../experiments/narrative-threads/results/REPORT.md)); not integrated |

## 2. Scope

### 2.1 Objectives

1. **Associative query**: from one or more seed units, retrieve related units by following typed relations across more than one hop, with mandatory bounds (depth, number of results and budget of visited entries).
2. **Temporal context**: restrict the traversal by `commit_seq` (relations and revisions created within a window) and answer over a snapshot.
3. **Provenance**: filter by the provenance of the relation (`Declared`, `Imported`, `Algorithmic`, `Inferred`) and by the actor or the source of the `ChangeSet` that created it.
4. **Traceability**: each result carries its trace: the relations of the path that reached it, the revision of each unit and the commit sequence of each step.
5. **Deterministic ranking with a diagnostic mode** that explains how much each signal contributes.
6. **Exact vector scoring** as an optional signal and baseline (§7, point 1).
7. Retrieval quality measured with a set of queries and expected answers **fixed before writing the retrieval code**.

### 2.2 Starting point

`kerno-units` has relations declared with mandatory provenance (ADR-0014), the adjacency index and `UnitSnapshot::relations_of` at one hop, with its reference path (`reference_relations_of`, I-32). Each relation stores `created_seq`; the `ChangeSet` stores actor, source and reason. The planner explains its plan (Stage 5) and the observations carry no literals outside `diagnostic` (I-37). There is no multi-hop traversal, no ranking and no vectors.

### 2.3 Out of scope

- Approximate vector index (ANN): only with a benchmark that justifies it (roadmap).
- Generating embeddings, summaries or inferred relations: Stage 8, behind adapters (I-31).
- **Persisting vectors or other derived data**: requires ADR-0018 and ADR-0009; it is Stage 8. No change to the persistent format in this stage.
- Full-text search (BM25).
- A universal ranking formula (spec §11): the weights are configuration, with a default value measured.
- Using the Stage 6 result cache in associative queries.
- Narrative retrieval (experiment closed as NO-GO).

## 3. Work packages

All are sequential. «Done» includes `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, the WP's suite and the full suite green, the test count before and after, and each new test seen to fail by mutation.

| WP | Content | Depends on | Acceptance |
|---|---|---|---|
| **E7-00** Entry gates | These minutes approved. ADR-0011 decided according to §7, point 1. **Evaluation set frozen** (§6.1): corpus, queries, expected answers with relevance grade and `MANIFEST` with SHA-256, committed before any retrieval code. Metrics of §6 implemented and tested with fixed answers. Baselines B0 and B1 measured over the set | Stage 6 closed | Set and thresholds committed; a test checks the SHA-256s; no later WP starts before |
| **E7-01** Query model and reference path | `AssocQuery` type (seeds, relation types, direction, maximum depth, result limit, visit budget, `commit_seq` window, provenance filter). Mandatory bounds with hard maxima. Reference path: breadth-first traversal over `reference_relations_of`, with deterministic order. Each result with its trace | E7-00 | Fixed vectors of queries and results; a query without bounds is rejected; cycles and repeated relations do not duplicate results; exhausting the budget is reported and not confused with «nothing more» |
| **E7-02** Index execution in the planner ([done](stage7-indexed-execution.md)) | The traversal uses the adjacency index as the planner's path; its answer is checked (like `check_relations`) and any failure falls back to the reference path with the cause recorded. Explanation of the plan and observations without literals | E7-01 | Differential test with seed against the reference path in both engines (I-32); injection of false index answers; I-37 sentinels over associative queries |
| **E7-03** Temporal context and provenance ([done](stage7-context-and-provenance.md)) | `commit_seq` window and provenance, actor and source filters applied in both paths. Verifiable trace: each cited relation and revision exists in the snapshot | E7-02 | I-40 (§4): test that walks each result and finds its sources; an inferred relation never appears as declared in the trace (I-39); the clock orders nothing (I-26) |
| **E7-04** Ranking and diagnostic mode ([done](stage7-ranking.md)) | Signals: distance in hops, relation type, recency by `commit_seq`, provenance class and, if vectors are supplied, exact similarity. Deterministic score; ties resolved by identifier. Diagnostic mode returns the contribution of each signal. Vectors: supplied by the caller, in memory; dimension and metric are checked; exact brute-force search | E7-03 | I-41 (§4): the sum of the contributions is the score; same state and same query give the same order; incompatible dimensions or metrics are rejected; with the ranking switched off the order is that of the reference path |
| **E7-05** Measurement ([done](stage7-measurement.md)) | Quality over the frozen set against the thresholds of §6.2. Performance according to §6.3. Committed raw output | E7-04 | Committed measurements; no threshold or expected answer changes after E7-00 |
| **E7-06** Closure ([done, approved on 2026-11-12](stage7-closure.md)) | Adversarial review. Promotion of I-40 and I-41. Update of the roadmap and the SAD | all | Criteria of §4 met or recorded as not claimed; report without open blockers |

Critical path: E7-00 → E7-01 → E7-02 → E7-03 → E7-04 → E7-05 → E7-06.

## 4. Acceptance criteria

From the [roadmap, Stage 7](../roadmap/README.md#stage-7--associative-memory) and from spec §11 and §12:

| Criterion | Invariant | Expected evidence | Status |
|---|---|---|---|
| Quality evaluable with a set fixed before measuring | — | E7-00 (frozen set), E7-05 | **met**. The set was corrected once before all retrieval code (`6b2a3cb`); see the [closure](stage7-closure.md) §2 and R-1 |
| Every answer traceable to its sources | **I-40** (new): each result of an associative query cites the relations and revisions that justify it, and all of them exist in the snapshot queried | E7-03 | **met** (closure §2 and §6.1; promoted and approved on 2026-11-12) |
| The ranking explains its signals | **I-41** (new): the score is deterministic and the diagnostic mode gives the contribution of each signal, whose sum is the score | E7-04 | **met** (closure §2 and §6.2; promoted and approved on 2026-11-12) |
| Equivalence with the reference path | I-32 | Differential test (E7-02) | **met with H-1**: not claimed for stores of 64 relations or more with an entry missing at both ends (closure R-2) |
| An inferred relation is not presented as declared | I-39 | E7-03 | **met** (`inferred_is_never_declared_in_a_trace`) |
| No literals outside `diagnostic` | I-37 | Sentinels (E7-02, E7-04) | **met** (`tests/sentinels.rs` and `assoc_rank.rs`) |
| The core needs no network or model | I-31 | Dependency review (E7-06): vectors are supplied by the caller | **met** (no change in `Cargo.toml` or `Cargo.lock` in the stage) |
| No ANN before a benchmark that justifies it | — | Out of scope (§2.3) | met by scope |

## 5. Risks

| ID | Risk | Proposed treatment |
|---|---|---|
| K-E3-8, K-E3-9 | U2 in `history`; U6 with concurrent retention | No changes; this stage does not change writing |
| H-1 (inherited) | Checks at query time do not guarantee completeness | Equivalence is shown with the differential test |
| **New** | The evaluation set is adjusted to the implementation | It is frozen with SHA-256 before writing retrieval; the expected answers come from the narrative with which the corpus is generated, not from the code of `kerno-units`; the generator lives in `kerno-testkit`, which does not depend on `kerno-units` |
| **New** | A synthetic set does not represent real use | It is declared as a limit in the closure; the thresholds measure this implementation over this set, not production quality |
| **New** | Multi-hop traversal grows without bound on highly connected nodes | Mandatory bounds and visit budget; exhausting it is reported in the result |
| **New** | The ranking does not improve on the simple order | The stage closes with the negative result documented and the default order stays «hops, then identifier» (§7, point 3) |
| **New** | The vector signal fixes assumptions of ADR-0011 that are hard to reverse | Vectors exist only in memory and are supplied by the caller; nothing is written to the format |

Merge policy: the [Phase 1 correctness gate](phase1-design.md#71-correctness-gate-adr-0012-threshold-1-c-25-blocking) is blocking. Performance shortfalls are recorded as risks and do not block. The quality thresholds decide the default ranking, not the merge.

## 6. Evaluation set and thresholds

### 6.1 Evaluation set (frozen in E7-00)

- **Corpus**: generated with a fixed seed from an agent-memory narrative (sessions, tasks, decisions, facts, sources), with relations of the four provenance classes and distractor relations. Quality scale: about 2 000 units. Performance scale: 20 000.
- **Queries**: at least 60, spread over four classes (at least 12 per class): multi-hop, temporal, provenance and combined.
- **Expected answers**: derived from the narrative, with a relevance grade (0 to 3), not from the definition of the query.
- **Baselines**: B0, filter by type (existing query); B1, one-hop neighbours in index order.

### 6.2 Quality (over the 60 queries, cut at 10)

- Recall@10 of the associative query with ranking ≥ 0.90.
- nDCG@10 ≥ 0.80.
- nDCG@10 at least 10 % better than B1 and than the simple order «hops, then identifier».
- In no query class does nDCG@10 fall below that of B1.
- Trace: 100 % of results with verifiable trace (I-40). This is not a quality threshold; it is a correctness criterion.

**Owner's decision, 2026-11-12:** T4 is not rewritten. The text of this section stays as it is. If the ranking is measured again, it will be with a new evaluation set and a threshold defined before measuring ([closure §8.2](stage7-closure.md)).

### 6.3 Performance (informative, except the first line)

- With 20 000 units and depth 2, the p50 of the index traversal is not worse than that of the reference path.
- p50, p99 and entries visited by depth (1 to 3) are reported. No threshold: there is no prior measurement to start from.
- T1–T10 and U1–U6 do not change; this stage does not write to the engine.

## 7. Points the owner must decide

1. **ADR-0011.** Proposal: accept option 1 (exact search first) with this limit for Stage 7: vectors are supplied by the caller and live in memory; the dimension and metric contracts are fixed (cosine and dot product); storing vectors as derived data tied to their source revision (I-25) is left for Stage 8 with ADR-0018.
2. **Evaluation set.** Proposal: the one in §6.1, synthetic and seeded, frozen in E7-00. The corpus of the narrative-threads experiment is not reused (external text and models).
3. **Default ranking.** Proposal: switched on by default only if it meets §6.2; otherwise, the order «hops, then identifier» and the ranking stays as an option.
4. **Thresholds of §6**, fixed before measuring.
5. **Hard bounds.** Proposal: maximum depth 4, 1 000 results and 100 000 visited entries per query; the caller may lower them, not raise them.

The safety order against AI (roadmap, pending decision 1) does not block this stage: nothing it delivers is persisted as derived data. It must be decided before Stage 8.

## 8. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-11 | **Approved** | The owner delegated the decisions («do what is best and most correct; take the reins yourself») and asked to start with these minutes. Decisions of §7: (1) ADR-0011, option 1 with vectors in memory supplied by the caller; persistence in Stage 8; (2) synthetic set of §6.1, frozen in E7-00; (3) default ranking only if it meets §6.2; (4) thresholds of §6 as they are; (5) hard bounds of depth 4, 1 000 results and 100 000 visits. The owner may revoke any of them |
| 2026-11-12 | **Closure pending approval** | [Closure of Stage 7](stage7-closure.md): adversarial review without blockers; criteria of §4 met (I-32 with H-1); T4 not met due to the wording of its own definition; I-40 and I-41 proposed; owner's decisions on Stage 8 and on T4 pending (closure §8) |
| 2026-11-12 | **Closure approved by the owner** | I-40 and I-41 promoted and approved. Decisions of §8 of the closure: Stage 8 starts with the ranking switched off by default and marked as experimental; T4 is not rewritten now (§6.2) |
