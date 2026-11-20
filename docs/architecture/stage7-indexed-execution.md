# Indexed execution of the associative query (E7-02)

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Done** in E7-02 |
| Minutes | [Stage 7](stage7-record.md) §3 (row E7-02), §4, §5 (H-1) |
| Code | `crates/kerno-units/src/assoc.rs` (shared traversal and index path), `planner.rs` (candidates, probes, check and fallback), `plan.rs`, `fingerprint.rs`, `observation.rs` |
| Tests | `crates/kerno-units/tests/assoc_indexed.rs`, `tests/sentinels.rs`, `tests/plan.rs` and the unit tests of `planner.rs` |
| Earlier documents | [associative query (E7-01)](stage7-associative-query.md) · [sentinels (E5-04)](stage5-sentinels.md) · [ADR-0014](../adr/0014-physical-representation-of-relations.md) |

This document describes the code as it is. Every figure comes from the commands in §9 or from the mutations in §8. This package does not change the persistent format, the evaluation set, any threshold of the minutes, or the behaviour of `reference_assoc_query`. It does not use the result cache (minutes §2.3).

## 1. What it adds

- `UnitSnapshot::indexed_assoc_query`: the same breadth-first traversal as the reference path, but with the neighbours of each hop read from `relations_of` (the adjacency index) and each answer checked. It is the path alone, chosen by the caller; a false answer from the index is `AssocError::IndexAnswer(Violation)`.
- `UnitSnapshot::assoc_query`: the planner's path. It returns `AssocOutcome { result, path, fallback }`. It chooses between `PlanPath::AssocReference` and `PlanPath::AssocAdjacency`, checks, and if anything fails, answers with the reference path and records it.
- `UnitSnapshot::explain_assoc`: the `Plan` that `assoc_query` follows in that snapshot.
- `QueryKind::Associative`, `NormalizedQuery::of_assoc` and `Observed::Assoc`: the fingerprint and the observation of these queries.

## 2. There is only one traversal

`reference_assoc_query` and the index path call the same `traverse` (in `assoc.rs`); only the function that gives the neighbours of a unit in one direction changes:

| Path | Neighbours of `(unit, direction)` |
|---|---|
| Reference | `reference_relations_of(unit, direction, None)` |
| Index | `relations_of(unit, direction, type)`, one read per type of the query's set of types (a single one with `None`), checked |

By construction, the order of examination, the visit budget, the window, the provenance filter, the three endings and the rule for units that are not active are those of E7-01. The reads by type are done in byte order of the type (that of `BTreeSet<String>`), so concatenated they give the `(type, id)` order of `reference_relations_of`, and the entries of a type that the query does not follow are never read. The budget counts the same thing as on the reference path because the type filter is still applied before counting.

That the traversal is shared does not replace the differential test: the data each path reads are different (§7).

## 3. The plan

The candidates, in tie-break order (`PlanPath::candidates_of`):

| Candidate | Is |
|---|---|
| `AssocReference` | Traversal over `reference_relations_of`. Always usable |
| `AssocAdjacency` | Traversal over the index. May be unusable (§3.2) |

An associative query has no index state (`plan.index()` is `None`): the adjacency entries are written in the same `ChangeSet` as the relation (ADR-0014) and have no watermark, so there is no «`Ready`» to read. What the planner knows about them is what it sees in the key ranges.

### 3.1 Cost

The cost is that of the first hop, in key-value reads; the following hops repeat the same exchange. `expansions` = distinct seeds × directions followed; `a` = entries of the first hop, probed per seed and direction (without looking at the type: it is an upper bound):

| Candidate | Cost |
|---|---|
| `AssocReference` | `r × expansions` (each expansion reads all the `REL`s) |
| `AssocAdjacency` | `2 × a` (the entry and the record of each neighbour) |

`r` and `a` are probes of at most `PROBE` = 64 entries, as in `decide_relations`: lower bounds when the probe comes back full. The cheapest of the usable ones wins, and the first in the table's order on a tie (a store with no relations costs 0 and 0, and the reference answers).

ponytail: the cost of the first hop decides for everyone and the probes do not distinguish types. Path to improve: a count per unit and type, if a load shows the probe picks the slower path.

### 3.2 When the index is not usable

The planner also probes the whole `REL`, `REL_OUT` and `REL_IN` ranges. A healthy store writes the record and the two entries in a single `ChangeSet`, so all three have the same number of entries. From the three probes (`adjacency_unusable`):

| Condition | `Unusable` |
|---|---|
| There are relations and `REL_OUT` or `REL_IN` has no entry | `AdjacencyAbsent` |
| A complete range (fewer than 64) has fewer entries than another range, complete or not | `AdjacencyInconsistent` |

A complete range is the exact count and bounds the others from above. A full probe is only a lower bound: another range having 64 contradicts nothing. That is why this check **only sees what a probe can see**: in a store with fewer than 64 relations, any entry that is missing or extra; in a larger one, a missing index. It is the part of the completeness check that the minutes allow to be done without walking all the records.

With the index unusable, `Decision::new` accepts it only for `AssocAdjacency` and for those two causes (`PlanError::FallbackUnusable` in any other case), never chooses it, and the reason of the plan is `Reason::AdjacencyUnusable(cause)`. The reference cannot be unusable.

## 4. The checks on each answer from the index

Each read of the index (`check_adjacent`, in `planner.rs`) goes through this before the traversal uses a relation:

| # | Check | If it fails |
|---|---|---|
| 1 | `relations_of` compares each entry with its `REL` record: absent record, or different endpoints or type | `Storage(Corruption)` → `FallbackCause::Corrupt` |
| 2 | `check_relations`: only relations of the requested unit and direction and of the requested type, in strict `(type, id)` order | `Violation::NotMatching`, `OutOfOrder` |
| 3 | No relation with a `created_seq` later than the snapshot: no entry of this snapshot can point to it | `Violation::BeyondSnapshot` |
| 4 | The mirror entry, at the other end of the relation, exists and names this endpoint | `Violation::Unmirrored` |

Checks 2 to 4 are those of `check_relations` from Stage 5 plus two new ones. Check 4 costs one point read per relation examined and is what detects, in a large store, an entry whose partner is missing or does not match it.

**What is not checked (H-1).** An entry missing at both ends of a relation leaves nothing to compare, and an entry missing only on the side being followed is invisible if the other end is not expanded. In a store of 64 relations or more, the index traversal then answers less than the reference without warning. `verify_units` (UNIT-36) sees it, and the differential test rules it out on healthy stores; the minutes already record it as H-1. The test `completeness_is_not_a_query_time_check` fixes this limit so that nobody takes it as covered.

## 5. The fallback and its cause

If any of the checks on a hop fails, or the index reports damage, the whole traversal is abandoned and the reference path answers from the beginning. The result of `assoc_query` says so in `AssocOutcome::fallback` (and the observation repeats it):

| `fallback` | When | Path that answers |
|---|---|---|
| `None` | The chosen path answered and passed its checks | The chosen one |
| `Fallback { abandoned: AssocAdjacency, cause: Unusable(AdjacencyAbsent \| AdjacencyInconsistent) }` | The plan marked the index unusable (§3.2); it was not read | `AssocReference` |
| `… cause: Invalid(NotMatching \| OutOfOrder \| BeyondSnapshot \| Unmirrored)` | The index was chosen and a check of the hop rejected it (§4, 2 to 4) | `AssocReference` |
| `… cause: Corrupt` | The index was chosen and reported damage (§4, 1) | `AssocReference` |

Three clarifications:

- A query that the model rejects (`AssocQueryError`) or with a seed that is not an active unit does not reach the planner: the error is the same one the reference gives (`active_seeds`, common to both).
- An error that is not damage is not hidden with a second attempt, as in `execute_units`. If the damage is in the data itself (an unreadable head, for example), the reference fails the same way and the error is returned.
- An unusable index counts as a recorded fallback even if the plan did not choose it, unlike `query` with a unit index `Stale`, which leaves no `fallback`. This is deliberate: the adjacency index has no state that the observation can read, and without this a damaged store and a store where the reference is cheaper would be indistinguishable in the observations.

## 6. Explanation and observations without literals

`explain_assoc` returns a `Plan`: the fingerprint of the query's shape, the two candidates with cost and usability, the chosen one and the reason. It carries no seed, type, actor, source, window or figure of the query.

The shape (`NormalizedQuery::of_assoc`, visible only in `diagnostic`) replaces everything with markers and carries only structure:

```text
assoc[seeds=?unit_ids,dir=<out|in|both>,depth=?u32,limit=?u32,budget=?u64
      [,types=?texts][,window=?seqs][,kinds=?kinds][,actor=?bytes][,source=?bytes]]
```

The direction is structure (three values). Neither the number of seeds, of types nor of classes enters the shape. The fingerprint is the usual one: keyed BLAKE3, `assoc/blake3-keyed/v1/…`.

`assoc_query` reports to the observer just like `query` and `query_relations`: path, rows (`hits.len()`), `commit_seq`, `fallback` and, if it fails, the class of the error (`ErrorKind::Invalid` for a rejected query or an inactive seed, and the class of the read error otherwise), never its message. One counter per path: `PATHS` goes to 8.

The error message the caller receives still cites its own bounds (`window … has its bounds inverted`) and the missing seed: it is the E7-01 error and is neither an observation nor an explanation. The tests fix it (§7).

`PLANNER_VERSION` stays at 1: the candidates, the cost and the choice of no earlier query change.

## 7. Tests

All run over `Emulated<LogEngine>` and `KernoDb`, except the unit tests of `planner.rs` (only the emulated one, like the earlier ones in the same module). The reference is the oracle: it never reads the index, so damage to the index cannot change what it answers.

| Test | What it shows |
|---|---|
| `assoc_indexed::differential_seeded` (I-32) | 6 seeds, each with 60 queries generated over two snapshots (one earlier than more relations) over graphs of 10 to 17 units with self-relations, parallel relations, a hub, 4 types (two with a common prefix) and the 4 provenance classes, with varied actors and sources. The queries set every field of `AssocQuery`: repeated, absent and empty seeds, sets of types (with one nonexistent), direction, depth 0 to 6, limit, budget, window (also inverted), classes, actor and source. The index path, the planner's and the reference's give the same result (hits, order, traces, visits, ending) or the same error; the plan is deterministic; the path executed is the one explained; no healthy store falls back. It requires that both paths, the three endings, the rejections and every field were covered |
| `differential_across_scan_pages` | 600 relations over 3 units and a fourth that reaches them in one hop: the scan of entries crosses pages of 256, with budgets around those boundaries and all the sets of types, directions and limits |
| `frozen_corpus_indexed_equals_reference` | The 64 queries of the frozen set (`crates/kerno-testkit/data/assoc-eval`, untouched), each also with three tight bounds, and one of three seeds with the hard maxima: the three endings, same result on the three paths. Also a clean `verify_units` |
| `false_index_answers_are_detected_and_answered_through_the_fallback` | 10 damages written to the engine, each in a small store (6 relations) and in a large one (more than 64): §7.1 |
| `completeness_is_not_a_query_time_check` | The limit of §4: the relation with no entries in a large store is not detected and the answer is worse than the reference; in the small one, the planner sees it. In both cases `verify_units` gives UNIT-36 |
| `a_failure_that_is_not_damage_is_not_hidden` | Inactive seed and rejected query give the same error by both paths; a damaged head also fails in the reference and the error is returned |
| `the_plan_compares_the_first_hop_and_says_why` | Costs 12 and 6, 6 and 2, 12 and 0 in hand-made vectors; store with no relations; the fingerprint starts with `assoc/blake3-keyed/v1/` |
| `observations_record_the_path_the_rows_and_the_class_of_an_error` | Path, rows, `commit_seq`, exact shapes in `diagnostic`, class `Invalid` with no path or rows for the rejected ones, per-path counters (no other path counts) and the choice of the reference with a hub |
| `the_shape_ignores_literals_and_sees_structure` | Other literals, same fingerprint and same text; each condition and the direction, different fingerprint |
| `sentinels::no_sentinel_in_any_associative_plan_observation_or_error` (I-37) | Sentinels in the relation type (and an oversized one), the actor and source of the change set, the provenance source, the upper bound of the window, the depth and the seeds' identifiers. They are searched in the plan, the fingerprint, the path and the fallback, the counters and the records, at the four levels; and in the errors, except the figures and the seed that the rejected query cites to its own caller. Controls: the answer contains the type, the change set contains actor and source, the errors contain the bounds. Each fingerprint reported is that of a shape with other literals |
| `sentinels::levels_below_diagnostic_carry_no_shape_text` | Extended: the associative shape exists only in `diagnostic` |
| `sentinels::no_sentinel_in_any_observability_explanation_or_error_output` | Extended: after damaging a head, the associative queries fail or fall back and still carry no sentinels |
| `plan::associative_plans_have_their_own_candidates_and_choice` and `plan_holds_no_literal` | Exact candidates and in order; the adjacency index may be unusable only for its two causes; the reference, never; no index state; tie goes to the reference; the plan carries no seed, type, actor, source or window |
| `planner::tests::*` (4) | The probe table of §3.2, the costs of the first hop, each `Violation` injected in a hop (relation of another unit, out of order, repeated, of another type, later than the snapshot, without entries), the damage (`Corrupt`), the error that is not damage, and the unusable index that is not read |

### 7.1 False answers from the index

Each damage is written with `put_all` directly to the engine, over a healthy store of a ring `a→b a→c b→d c→d(u) d→e e→a` and the query from `a`, depth 4, both directions. The reference's answer is computed first. After damaging, it is checked that `assoc_query` returns exactly the reference's (which does not read the index), that the path is `AssocReference`, that `fallback` is the expected one, that the plan announces it, and that the observation repeats the path and the fallback. When the original bytes are returned, the index answers again without fallback and `verify_units` is clean.

| Damage | Small store (6 relations) | Large store (more than 64) |
|---|---|---|
| The mirror entry of a relation being followed is missing | `Unusable(AdjacencyInconsistent)` | `Invalid(Unmirrored)` |
| The entry on the followed side is missing and is seen from the other end | `Unusable(AdjacencyInconsistent)` | `Invalid(Unmirrored)` |
| The mirror entry names another unit | `Invalid(Unmirrored)` | `Invalid(Unmirrored)` |
| Extra entry without a record | `Unusable(AdjacencyInconsistent)` | `Corrupt` |
| Extra entry with a record and without a mirror | `Unusable(AdjacencyInconsistent)` | `Invalid(Unmirrored)` |
| An entry names the wrong unit | `Corrupt` | `Corrupt` |
| Wrong revision: record with `created_seq` later than the snapshot | `Invalid(BeyondSnapshot)` | `Invalid(BeyondSnapshot)` |
| Stale index: entries of a relation deleted later | `Unusable(AdjacencyInconsistent)` | `Corrupt` |
| Stale index: a relation created later has no entries | `Unusable(AdjacencyInconsistent)` | **not detected** (§4) |
| Absent index: no outgoing entry | `Unusable(AdjacencyAbsent)` | `Unusable(AdjacencyAbsent)` |

The damages that change the data (falsified record, record with a future revision) are compared with the reference over the damaged store, and it is checked that this answer differs from the healthy one, so that the test is not empty; the others, which touch only the index, must give the same answer as before damaging.

## 8. Mutations

Each mutation was applied to unmutated code with a script that substitutes the text, runs `cargo test -p kerno-units --release --no-fail-fast` over the library and the files `assoc_indexed`, `sentinels`, `plan`, `assoc_reference`, `relations` and `observation`, notes the red tests and restores the file (checked byte by byte). All 29 compiled and all 29 were detected. Two of them came out of reviewing which mutants would survive and produced one more test each: the mirror entry naming another unit (M06) and a counter per path shared (M19). «Test» abbreviates the function name; those of `assoc_indexed` and `sentinels` fail in their two engines.

| # | Mutation | Tests that failed |
|---|---|---|
| M01 | The index path reads the types in descending order | `differential_seeded`, `frozen_corpus_indexed_equals_reference` |
| M02 | `indexed_assoc_query` always reads the outgoing range | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_indexed_equals_reference`, `false_index_answers_…` |
| M03 | `indexed_assoc_query` ignores the type of the hop | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_indexed_equals_reference` |
| M04 | The planner never runs the index path | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_…`, `false_index_answers_…`, `completeness_is_not_a_query_time_check`, `observations_record_…`, `a_wrong_hop_…` |
| M05 | A missing mirror entry counts as present | `false_index_answers_…`, `a_wrong_hop_…` |
| M06 | The mirror entry is not compared with this endpoint | `false_index_answers_…` |
| M07 | A relation created at the snapshot's sequence is taken as later | `differential_seeded`, `differential_across_scan_pages`, `frozen_corpus_…`, `observations_record_…`, `a_wrong_hop_…` |
| M08 | The check of the relation later than the snapshot is skipped | `false_index_answers_…`, `a_wrong_hop_…` |
| M09 | `check_relations` is not applied to a hop | `a_wrong_hop_…` |
| M10 | An absent index is not recognised | `false_index_answers_…`, `the_adjacency_index_is_usable_only_when_the_probes_can_agree`, `an_unusable_adjacency_path_is_not_run_…` |
| M11 | Inconsistent probes are not recognised | `false_index_answers_…`, `completeness_is_not_a_query_time_check`, `the_adjacency_index_is_usable_…`, `assoc_costs_compare_the_first_hop` |
| M12 | The index damage is returned instead of falling back | `false_index_answers_…`, `a_wrong_hop_…` |
| M13 | An unusable path is recorded as damage | `false_index_answers_…`, `completeness_is_not_a_query_time_check`, `an_unusable_adjacency_path_is_not_run_…` |
| M14 | A false answer is recorded as damage | `false_index_answers_…`, `a_wrong_hop_…` |
| M15 | The cost of the reference ignores the expansions | `the_plan_compares_the_first_hop_…`, `assoc_costs_compare_the_first_hop`, `false_index_answers_…`, `completeness_…`, `observations_record_…` |
| M16 | The cost of the index counts each entry once | `the_plan_compares_the_first_hop_…`, `assoc_costs_compare_the_first_hop`, `differential_seeded` |
| M17 | `assoc_query` does not report to the observer | `observations_record_…`, `levels_below_diagnostic_carry_no_shape_text`, `no_sentinel_in_any_associative_plan_…`, `false_index_answers_…`, `completeness_…` |
| M18 | A rejected query is observed as `Other` | `observations_record_…` |
| M19 | The index path shares the first path's counter | `observations_record_…` |
| M20 | The shape forgets the window | `the_shape_ignores_literals_and_sees_structure`, `levels_below_diagnostic_…`, `observations_record_…` |
| M21 | The shape carries the relation types | `no_sentinel_in_any_associative_plan_…`, `no_sentinel_in_any_observability_…`, `the_shape_ignores_literals_…`, `levels_below_diagnostic_…`, `observations_record_…` |
| M22 | The shape carries the actor | the same five as M21 |
| M23 | The associative reference can be unusable | `associative_plans_have_their_own_candidates_and_choice` |
| M24 | The reason for an unusable index is «lower cost» | `associative_plans_…`, `false_index_answers_…`, `completeness_…` |
| M25 | The expansions are not counted | `the_plan_compares_the_first_hop_…`, `differential_seeded`, `differential_across_scan_pages`, `false_index_answers_…`, `a_wrong_hop_…`, `observations_record_…`, `completeness_…` |
| M26 | `explain_assoc` plans a rejected query | `differential_seeded` |
| M27 | An error that is not damage falls back to the reference | `a_wrong_hop_…` |
| M28 | An inactive seed is accepted | `a_failure_that_is_not_damage_…`, `differential_seeded`, `observations_record_…`, `no_sentinel_in_any_associative_plan_…`, and from E7-01 `seeds_must_be_active_units`, `ends_that_are_not_active_are_absent` |
| M29 | The fallback caused by an unusable index is not recorded | `false_index_answers_…`, `completeness_…`, `an_unusable_adjacency_path_is_not_run_…` |

## 9. Verification

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_indexed
```

Test count of `kerno-testkit` + `kerno-units` with `--release --all-features`: **611 passed, 0 failed, 8 ignored** with this change; before it was 586 (E7-01). The 25 new ones are 18 of `assoc_indexed` (9 × 2 engines), 2 of `sentinels` (1 × 2), 1 of `plan` and 4 unit tests of `planner`; the runner counts them (`assoc_indexed`: `18 passed`).

## 10. Limits and risks

- **Completeness (H-1).** The index answer can be incomplete without warning when entries are missing at both ends of a relation and the store has 64 relations or more (§4). It is a property of the check at query time, not a fault of this path; `verify_units` and the differential test over healthy stores cover it.
- **Cost of the check.** Each relation examined costs one more point read (the mirror), and each planned query up to 192 probe entries plus those of the first hop. Nothing is claimed about performance (E7-05).
- **Probes are lower bounds.** In large stores the planner sees little: the cost and the detection of §3.2 are an estimate. The path chosen is always correct; it may only not be the cheapest.
- **A single `ChangeSet` of relations.** The damage tests use relations from one change set per relation; the behaviour does not depend on it.
- **Dependencies.** None new; `Cargo.toml` does not change.
- **Copy.** `tests/harness/assoc.rs` repeats the corpus loader and the query conversion of `tests/assoc_reference.rs`, which was not touched. Pending: that `assoc_reference.rs` use the common module.

## 11. Related documents

- [Stage 7 minutes](stage7-record.md) §3 (E7-02), §4, §5 (H-1)
- [Associative query (E7-01)](stage7-associative-query.md)
- [Sentinels (E5-04)](stage5-sentinels.md)
- [ADR-0014](../adr/0014-physical-representation-of-relations.md): relations and transactional adjacency
- [ADR-0015](../adr/0015-privacy-of-observability.md)
- [Invariants](invariants.md): I-32, I-37
