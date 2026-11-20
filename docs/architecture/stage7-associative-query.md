# Associative query: model and reference path (E7-01)

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Done** in E7-01 |
| Minutes | [Stage 7](stage7-record.md) §3 (row E7-01), §4, §8 (decision 5: hard bounds) |
| Code | `crates/kerno-units/src/assoc.rs` (model and reference path), exported from `lib.rs` |
| Tests | `crates/kerno-units/tests/assoc_reference.rs` |
| Evaluation set | [stage7-evaluation-set.md](stage7-evaluation-set.md) (frozen; this package does not modify it) |

This document describes the code as it is. Every figure comes from the commands in §11 or from the mutations in §9.1. This package does not use the adjacency index (it is E7-02), does not touch `planner.rs`, does not order by score (it is E7-04) and does not change the persistent format.

## 1. What it adds

- `AssocQuery`: the query, with mandatory bounds and their hard maxima.
- `UnitSnapshot::reference_assoc_query`: the reference path, a breadth-first traversal built **only** over `reference_relations_of` (never over `relations_of`). It is the definition of what the index path of E7-02 must return over the same snapshot (I-32).
- `AssocResult`: the results, each with its trace, and the reason why the traversal ended.

## 2. The query model

| `AssocQuery` field | Type | Meaning |
|---|---|---|
| `seeds` | `Vec<UnitId>` | One or more seeds. Repeated ones count once |
| `relation_types` | `Option<BTreeSet<String>>` | Relation types that are followed; `None`, any |
| `direction` | `AssocDirection` | `Out` (relations leaving the unit), `In` (those arriving) or `Both` |
| `max_depth` | `u32` | Maximum hops from a seed |
| `limit` | `usize` | Maximum results |
| `visit_budget` | `u64` | Maximum relation entries examined |
| `window` | `Option<SeqWindow>` | Inclusive window of `commit_seq`, applied to the `created_seq` of each relation that is followed |
| `provenance` | `ProvenanceFilter` | `kinds` (classes of `ProvenanceKind`), `actor` and `source` of the change set that created the relation; each field present must be satisfied |

`AssocDirection` is a type of its own because `Direction` has only `Out` and `In`, and `Direction` is the one for one-hop reads. `AssocQuery::new(seeds, max_depth, limit, visit_budget)` requires the three bounds (they have no default value) and leaves the rest at «any type, both directions, no window, no filter».

The filter's actor and source are compared with those of the `CHANGESET` record at the relation's `created_seq` (`UnitSnapshot::change_set`). The relation does not store an actor or a source: it stores only its `created_seq` (ADR-0014). A change set without an actor does not satisfy an actor filter. A `created_seq` without a `CHANGESET` record is `Storage(Corruption)`. Records read are remembered within one query, by `commit_seq`.

## 3. Bounds

The hard maxima are public constants (minutes §8, decision 5). The caller may lower the bounds, not raise them.

| Constant | Value | Rejected |
|---|---|---|
| `MAX_ASSOC_DEPTH` | 4 | `max_depth` 0 or greater than 4 |
| `MAX_ASSOC_LIMIT` | 1 000 | `limit` 0 or greater than 1 000 |
| `MAX_ASSOC_VISITS` | 100 000 | `visit_budget` 0 or greater than 100 000 |

`AssocQuery::validate` checks the query without reading anything, and `reference_assoc_query` calls it before opening any unit. The errors are of type `AssocQueryError`, and `AssocError::Query` carries them:

| Variant | When |
|---|---|
| `NoSeeds` | No seeds |
| `DepthOutOfRange { depth }` | Depth outside `1..=4` |
| `LimitOutOfRange { limit }` | Limit outside `1..=1000` |
| `BudgetOutOfRange { budget }` | Budget outside `1..=100000` |
| `EmptyRelationTypes` | Set of types present and empty (it would follow nothing) |
| `EmptyProvenanceKinds` | Set of classes present and empty |
| `InvertedWindow { lo, hi }` | Window with `lo > hi` |

If more than one check fails, the first in this order wins: seeds, depth, limit, budget, types, classes, window. The values `1`, `4`, `1000` and `100000` are valid.

A seed that is not an active unit in the snapshot (it does not exist, is archived or is deleted) rejects the whole query with `AssocError::SeedNotActive { unit }`, even if there are other good seeds. It is a rejection at run time, not at validation, because it depends on the snapshot.

## 4. Order and tie-breaking (I-26)

Results come out in the order **(distance in hops, unit identifier)**. It is the «hops, then identifier» order of the minutes (§6.2, §7 point 3), and therefore the one E7-04 returns with ranking switched off.

Which path a result takes is decided by the order of examination, which is the documented tie-break:

1. The units of one level are expanded in ascending identifier order. The seeds are level 0 and are sorted the same way: the order in which the caller passes them changes nothing.
2. For one unit, first the relations leaving it (`Out`) and then those arriving (`In`), if the query follows both.
3. Within one direction, by the bytes of the relation type and then by the bytes of the relation identifier (the order of `reference_relations_of`).

A unit is returned only once, with the first path that reaches it in that order. Seeds are not results, even if another seed reaches them. A cycle, a parallel relation (same endpoints and type) or a relation of a unit with itself does not duplicate results: the unit is already seen.

Nothing depends on the iteration of a hash map (`BTreeSet` and `BTreeMap` are used) or on a clock. A level is examined in full and its results are sorted by identifier; that is the only consequence of this order worth knowing (§6).

## 5. Visit budget

An entry is **examined** when the traversal looks at a relation adjacent to the unit it is expanding, in a direction it follows and with a type from the query's set of types (any if there is no set). It is counted before the window and the provenance filter are applied, which need the relation's record, and it is counted even if it does not lead to a new unit. Two consequences that the tests fix:

- Entries of a type that the query does not follow are not examined. This is what a range of the type index reads, so the index path of E7-02 counts the same thing.
- A relation of a unit with itself has two adjacency entries: with `Both` it is examined twice.

The budget is checked just before examining an entry. A budget that exactly suffices to finish is not exhausted; one less, it is.

## 6. Ending reasons

`AssocResult::ending` is one of three, and only one:

| `AssocEnding` | Means | What callers may assume |
|---|---|---|
| `Exhausted` | Nothing was left within the query's bounds (also when the traversal reached the maximum depth) | The results are the **whole** answer |
| `LimitReached` | There are more results than `limit`. One more was found | The results are the first `limit` of the answer in the order of §4 |
| `BudgetExhausted` | The budget ran out before a decision could be made | The results are valid but some may be missing |

Clarifications:

- «Nothing left» includes the depth limit: a depth-1 query that returns all the neighbours ends `Exhausted`, because depth is part of the question.
- With `limit = k` and exactly `k` possible results, the reason is `Exhausted`, not `LimitReached`. `LimitReached` requires having seen a result `k + 1`. When the first `k` are already complete, the traversal stops as soon as the next one appears; if the limit is crossed within a level, the level is finished and cut by identifier.
- If the budget runs out in the middle of a level, the results of that level already discovered come out sorted by identifier, and units with a smaller identifier than some returned ones may be missing. Each returned result is one of the complete answer, with the same path (the test `bounded_answers_agree_with_the_unbounded_one` checks it for every budget). That is why `BudgetExhausted` does not promise a prefix and `LimitReached` does. If the budget runs out and there are more than `limit` results, it is cut at `limit` and the reason remains `BudgetExhausted`: it is the weakest claim and nothing is lost by it.
- `visited` reports the entries examined and never exceeds the budget.

## 7. The trace

Each result (`AssocHit`) carries:

| Field | Content |
|---|---|
| `unit` | The unit |
| `distance` | Hops from the nearest seed; equal to `path.len()` |
| `revision` | `RevNo` of the unit's head in the snapshot |
| `path` | `Vec<TraceStep>`: the relations followed, from the seed to the unit, in order |

Each `TraceStep` stores `relation` (identifier), `relation_type`, `from`, `to`, `created_seq` and `provenance` (an `ProvenanceKind`). `from` and `to` are the endpoints of the relation as it was stored, not the direction in which it was followed: a relation `u2 → u1` followed from `u1` appears with `from = u2`. The step carries the provenance class and not its data, and an inferred relation never appears as declared (I-39).

With this trace, the caller can check a result with `get_relation`, `get_unit` and `change_set` over the same snapshot. The test `frozen_corpus_traces_are_verifiable` does so (§9.2). The general test of I-40 is from E7-03.

## 8. Archived and deleted units

The layer **does not allow** archiving or deleting a unit that a relation touches (ADR-0014; error `UnitHasRelations`). In a healthy snapshot, every endpoint of every relation is `Active`, so there is nothing to decide. The traversal decides anyway what to do if it is not (a damage that `verify_units` reports as UNIT-36):

| Case | Decision |
|---|---|
| Seed nonexistent, archived or deleted | The query is rejected with `SeedNotActive` |
| Other endpoint nonexistent, archived or deleted | It is treated as absent: it is not returned and not traversed. The entry that led to it counts as examined |

It is the rule of the Stage 4 reference path («a query sees only active units», [`reference.rs`](../../crates/kerno-units/src/reference.rs)). It was chosen over the alternatives for this reason: returning the unit with its state would force the caller to filter, and failing with corruption would turn a read query into a verifier. The damage is reported with `verify_units`. A snapshot taken before archiving still sees the unit and its relations: archiving has no other effect on a query.

## 9. Tests and mutations

`tests/assoc_reference.rs` has **19 test functions**, run over `Emulated<LogEngine>` and over `KernoDb`: **38** tests. The expectations are vectors written by hand over two small graphs and, for the frozen corpus, an independent breadth-first traversal written in the test itself over all the relations (`oracle`).

| Graph | Content |
|---|---|
| Diamond | 7 units `a..g`; 9 relations: `a→b`, `a→c`, `b→d`, `c→d`, `d→a` (cycle), `d→e` (type `u`), `e→f`, `a→b` (parallel), `c→c` (self). `g` is isolated |
| Provenance | 6 units and 7 relations of the four classes, with actors `alice`, `importer`, `bot`, one without an actor, and sources `doc/1`, `doc/2`, `wiki/1`, `job/1`, `job/2` |

Test count of the `kerno-testkit` + `kerno-units` package with `--release --all-features`: **586 passed** with this change; the previous count is **548** (586 − 38; derived, not run on the previous tree). The runner counts the 38: `assoc_reference` appears with `38 passed`.

### 9.1 Mutations

Each mutation was applied to the unmutated code, `cargo test -p kerno-units --release --test assoc_reference` was run, the tests that failed were noted, and the file was restored with `git checkout -- <file>`. After the last one, `git diff` showed no changes in the working tree. The line numbers are those of `src/assoc.rs` unmutated. All names are of test functions; each one failed in both of its engines. «Others» are the tests that also failed.

| Test | Mutation (line) | Failed | Others that failed |
|---|---|---|---|
| `validation_rejects_every_unbounded_or_malformed_query` | 153: `self.max_depth == 0 \|\|` removed (depth 0 accepted) | that one | `seeds_must_be_active_units` |
| `hard_maxima_are_public_and_accepted` | 153: `> MAX_ASSOC_DEPTH` → `>=` (depth 4 rejected) | that one | 13 more that use depth 4 |
| `seeds_must_be_active_units` | 311: the active-seed check becomes `if false` | that one | `ends_that_are_not_active_are_absent` |
| `seeds_are_not_results_and_order_does_not_depend_on_them` | 319: `seen` starts empty (seeds may come back as results) | that one | 11 more |
| `out_traversal_vector` | 337: `!seen.insert(there)` → `seen.contains(&there)` (the last path wins) | that one | 12 more |
| `in_traversal_vector` | 85: `In` follows `Direction::Out` | that one | `frozen_corpus_traces_are_verifiable` |
| `both_traversal_vector_with_cycle_and_self_relation` | 86: `Both` follows only `Out` | that one | `ends_that_are_not_active_are_absent`, `frozen_corpus_traces_are_verifiable`, `relation_type_filter`, `seeds_are_not_results_and_order_does_not_depend_on_them`, `trace_carries_the_fields_of_each_relation_and_the_head_revision` |
| `trace_carries_the_fields_of_each_relation_and_the_head_revision` | 349: `revision: unit.head` → `RevNo(1)` | only that one | — |
| `trace_carries_the_fields_of_each_relation_and_the_head_revision` | 379: the step's class always `Declared` | that one | `provenance_filters_by_kind_actor_and_source`, `frozen_corpus_traces_are_verifiable` |
| `relation_type_filter` | 369: the type filter always returns `true` | that one | `the_three_endings_are_distinct`, `frozen_corpus_traces_are_verifiable` |
| `window_is_inclusive_and_applies_to_each_relation_followed` | 394: `<`/`>` → `<=`/`>=` (exclusive window) | that one | `provenance_filters_by_kind_actor_and_source`, `frozen_corpus_traces_are_verifiable` |
| `provenance_filters_by_kind_actor_and_source` | 405: the actor filter returns `true` | that one | `frozen_corpus_traces_are_verifiable` |
| `provenance_filters_by_kind_actor_and_source` | 398: the class filter inverted | that one | `frozen_corpus_traces_are_verifiable` |
| `provenance_filters_by_kind_actor_and_source` | 406: the source filter returns `true` | that one | `frozen_corpus_traces_are_verifiable` |
| `the_three_endings_are_distinct` | 429: the budget ending is reported as `LimitReached` | that one | `bounded_answers_agree_with_the_unbounded_one` |
| `bounded_answers_agree_with_the_unbounded_one` | 357: no cut at `limit` after a level | that one | `the_three_endings_are_distinct`, `frozen_corpus_traces_are_verifiable` |
| `bounded_answers_agree_with_the_unbounded_one` | 329: `==` → `>` (one more budget entry examined) | that one | `the_three_endings_are_distinct` |
| `same_logical_graph_gives_byte_identical_results` | 354: the frontier walked in descending identifier order | that one | `in_traversal_vector`, `out_traversal_vector`, `provenance_filters_by_kind_actor_and_source`, `relation_type_filter` |
| `repeated_runs_are_byte_identical` | 364: `visited` adds a clock bit | that one | 10 more that compare `visited`; the number varies between runs because the clock bit changes |
| `a_snapshot_does_not_see_later_commits` | 307: the result is memoised per query and not per snapshot (variable per thread) | only that one | — |
| `ends_that_are_not_active_are_absent` | 340: `reference_get` → `get_unit` (archived and deleted units are returned) | only that one | — |
| `the_reference_path_never_reads_the_adjacency_index` | 327: `reference_relations_of` → `relations_of` | only that one | — |
| `frozen_corpus_traces_are_verifiable` | 346: the path is stored reversed (`p.reverse()`) | that one | 8 more (the vectors) |

Two clarifications about this table:

- With the adjacency index already destroyed in the test `the_reference_path_never_reads_the_adjacency_index`, the reference path answers the same: it is the test that it does not read it. If mutation 327 is applied, `relations_of` returns empty and the result changes.
- `same_logical_graph_gives_byte_identical_results` writes the same graph with identifiers fixed by hand (a test identifier source) and with the units and relations created in four different orders, each one's relations in a single change set so that they share the same `created_seq`. It compares the `Debug` representation of the results. It only makes sense with fixed identifiers: with a counter, the creation order changes the identifiers and, with them, the logical graph. A mutation that depended on insertion order has nothing to rest on in this path (the traversal reads by key, not by write order), so the mutation in the table is the one on the expansion order.

### 9.2 What the frozen corpus checks

`frozen_corpus_traces_are_verifiable` loads the corpus (2 000 units, 3 958 relations, 300 change sets) and runs the 64 queries with the constraints of the file (direction, types, depth, window, classes, actor and source) and the hard maxima. For each result it checks:

- order `(distance, unit)` without repeats, and that no seed is a result;
- each relation of the path exists in the snapshot with the same endpoints, type, `created_seq` and provenance class, and meets the query's type, window, class, actor and source;
- each hop connects with the next one, from a seed to the unit, in a direction the query follows;
- the unit is active and `revision` is its head.

It also compares the results with the independent breadth-first traversal: all 64 end `Exhausted` and the result set and its distances are those of the test's traversal. All the expected answers of the set (including the grade-0 distractors) are among the results, 56 of the 64 queries reach distance 2 or more, and a three-seed query without constraints ends `LimitReached` with the first 1 000 of the breadth-first traversal. This check does not measure retrieval quality (it is E7-05) and does not change any data of the set.

## 10. Limits and risks of this package

- **Cost**: each expansion calls `reference_relations_of`, which reads all the `REL` records (linear in the number of relations). It is the reference path, correct by definition and slow on purpose. The corpus test takes about six seconds for the 38 tests in `--release`. Nothing is claimed about performance (it is E7-05).
- **`BudgetExhausted` is not a prefix** (§6). Whoever needs the prefix must use the limit, not the budget.
- **An inactive seed rejects the whole query** (§3). It is a choice; the seed could be omitted and reported, which would add a field to the result that nobody has asked for.
- **No ranking or vectors**: the order is «hops, then identifier».
- **Dependency**: none new. `Cargo.toml` does not change.

## 11. Verification

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_reference
```

The last line gives `test result: ok. 38 passed` over both engines.

## 12. Related documents

- [Stage 7 minutes](stage7-record.md) §3 (E7-01), §4, §8
- [Evaluation set](stage7-evaluation-set.md) (frozen corpus and queries)
- [ADR-0014](../adr/0014-physical-representation-of-relations.md): relations and provenance
- [Invariants](invariants.md): I-26 (the clock does not order), I-32 (equivalence with the reference path), I-39 (inferred is not declared)
