# Temporal context and provenance of the associative query (E7-03)

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Done** in E7-03 |
| Minutes | [Stage 7](stage7-record.md) §3 (row E7-03), §4 (I-39, I-40), §2.1 (targets 2 to 4) |
| Code | None in production: `crates/kerno-units/src/assoc.rs` does not change. New tests in `crates/kerno-units/tests/assoc_context.rs`; `tests/harness/assoc.rs` gains `load_corpus_with_relations` and makes `story_provenance` public |
| Tests | `crates/kerno-units/tests/assoc_context.rs` (7 functions × 2 engines) |
| Earlier documents | [associative query (E7-01)](stage7-associative-query.md) · [indexed execution (E7-02)](stage7-indexed-execution.md) · [evaluation set](stage7-evaluation-set.md) |

This document describes the code as it is. Every figure comes from the commands in §8 or from the mutations in §7. This package does not change the persistent format, the evaluation set, any threshold of the minutes, nor the behaviour of the three paths (`reference_assoc_query`, `indexed_assoc_query`, `assoc_query`), and it does not touch the ranking (E7-04).

## 1. What there was and what was missing

The `commit_seq` window and the class, actor and source filters were already applied in both paths since E7-01 and E7-02, and for one reason: the traversal is one (`traverse`, in `assoc.rs`) and the paths differ only in where they read the neighbours from. That is why no new code was needed, and none was added. What was not tested, against the E7-03 row of the minutes:

| Gap | What there was | What was missing |
|---|---|---|
| Verifiable trace on the index path and on the planner's path | `frozen_corpus_traces_are_verifiable` (E7-01) verifies the traces of the reference path only. The E7-02 differential test compares the three paths with each other | Verifying the traces of **each** path against something other than another path |
| Existence of the cited revision | `revision == unit.head`, read from the library itself | That the revision can be read in the snapshot, that its `commit_seq` does not exceed the snapshot's and that no later one is visible. The graphs of the differential tests **never update a unit**: every head is revision 1, so that field could never differ |
| Oracle for the window and the filters | Hand vectors (E7-01) and, for the corpus, `admitted`, which reads the actor and the source with `snap.change_set` (the library) | A brute-force oracle that reads nothing from the library, with combinations and empty answers, over the three paths and over the frozen corpus |
| I-39 in the trace | `relations::declared_is_never_confused_with_inferred` (relation layer, no associative query) | That the trace of an associative query, with parallel relations of different classes between the same endpoints, never shows an inferred, imported or algorithmic relation as declared |
| I-26 | `history::ordering_ignores_clock` (history) | That neither the order, nor the path, nor the window, nor the filters of an associative query depend on timestamps |

## 2. The model and the oracle

The expectations do not come from the library. The test keeps, while writing, a **model**: each relation with its identifier, endpoints, type, `commit_seq`, full provenance, and the actor and source of its change set; and, for each unit, the `commit_seq` of each of its revisions. For the frozen corpus the model is built from the narrative (`corpus.changes`, `corpus.relations`, `corpus.units`), not from the database.

`oracle(model, seq, query)` is the breadth-first traversal rewritten from the order documented in `assoc.rs` (levels by identifier, `Out` before `In`, `(type, id)` by bytes) over **all** the relations at once, and it returns the results, their paths and the entries examined. `assert_answer` compares every field with it: unit, distance, path relations, type, endpoints, `created_seq` and class of each step, revision, `visited` and the ending `Exhausted`. It is only valid for queries whose limit and budget are not reached; with tight bounds `verify_trace` is used instead (§3).

The test world (`World`) grows in change sets of 1 to 4 relations with random actors and sources (sometimes absent), the four provenance classes, self and parallel relations, and 0 to 2 **updated units** per set. Intermediate snapshots are taken (`old` and `new`). Identifiers and sequences depend only on the seed, never on the clock.

## 3. What `verify_trace` checks (I-40)

For each result of each path, with any bound:

- the order `(distance, unit)` without repeats; no seed is a result; `path.len() == distance`;
- **the revision**: it is the head that the model says in that snapshot; `get_revision(unit, revision)` reads it; its `commit_seq` is the model's and does not exceed the snapshot's; `get_revision(unit, revision + 1)` is `RevisionNotFound`;
- **each relation of the path**: it exists in the model with a `created_seq` not later than the snapshot; the step repeats type, endpoints, `created_seq` and class; `get_relation` returns it with the same endpoints and type and with full provenance (origin and confidence, not only the class); the query lets it through (type, window, class, actor, source); none is repeated;
- **the path is a real traversal**: it starts from a seed, each hop leaves the unit it is on in a direction the query follows, and the unit at each intermediate position is a result of the same answer with that position's distance. That last condition is what makes «the revision of each unit» of the minutes cited: each unit on the path is a seed or a result with its own revision.

## 4. Tests

All run over `Emulated<LogEngine>` and `KernoDb`, over the three paths (`reference_assoc_query`, `indexed_assoc_query`, `assoc_query` without fallback).

| Test | What it shows |
|---|---|
| `every_cited_relation_and_revision_exists_in_the_snapshot` (I-40) | 6 seeds, 50 queries generated per snapshot (`old` and `new`), with often tight bounds, all the fields of the query and the three endings. The three paths give the same result and each one passes `verify_trace`. It requires more than 500 results seen, empty answers, more than 50 results whose unit has a head greater than 1, and the three endings |
| `an_old_snapshot_never_cites_what_came_after_it` (I-40) | A snapshot taken before 10 more change sets never cites a relation written later nor a later revision; over the new one, those same relations do appear (more than 50 steps), so the test is not empty |
| `frozen_corpus_cites_only_what_the_snapshot_holds` (I-40) | The 64 queries of the frozen set, each on the three paths: equal to the oracle built from the narrative (hits, paths, `visited`) and with `verify_trace`; each expected answer of the set is a result; more than 500 steps walked. Also each query with three tight bounds: the paths agree and the traces are verified |
| `window_and_provenance_filters_equal_the_brute_force_oracle` | 4 seeds × 2 snapshots. Window: all the pairs of a grid that includes 0, 1, 2, 3, the half, the last sequence minus one, the last, one more and a much later one. Classes: the 15 non-empty sets. Each actor and each source, and one that nobody has. 40 combinations per snapshot with types, directions, several seeds, window and actor always present. The three paths equal the oracle. It requires more than 40 empty answers and more than 200 non-empty, and that **removing each condition (window, classes, actor, source) changes the answer** in more than 20 queries: otherwise the condition decided nothing in the test |
| `frozen_corpus_filters_equal_the_oracle_one_condition_at_a_time` | The 64 queries of the set with their own window, classes, actor and source, as they are and removing one condition each time, against the oracle of the narrative. Each condition changes the answer in more than 3 queries of the set |
| `inferred_is_never_declared_in_a_trace` (I-39) | Hand-made graph with four parallel relations `a→b` of the same type (declared, imported, algorithmic, inferred) and two more hops (one inferred, one declared). Exact vectors of unit, relation and shown class for no filter, `{Inferred}`, `{Declared}`, `{Imported}`, `{Algorithmic}`, `{Imported, Algorithmic}` and `{Imported, Inferred}`, on the three paths. The stored record keeps origin and confidence, and the trace says `Declared` only for a declared record. Over generated worlds, no step shows a class other than the model's, and the four classes appear |
| `the_clock_orders_nothing` (I-26) | The same history (same identifiers and sequences) under three clocks: constant, **backwards** with large jumps and increasing. It checks that the timestamps of the revisions and of the change sets differ, and that the backwards clock's decreases with the sequence. 4 seeds × 40 queries (half with a window): the `Debug` text of the result is identical on the three paths and the three clocks, and the wide-bound ones equal the oracle of each history |

Already covered before and not repeated: `assoc_indexed::differential_seeded` sets every field of `AssocQuery` and requires the three paths to match (including errors and index failures), and `frozen_corpus_indexed_equals_reference` does it over the set. **It is not extended**: it already covers window, classes, actor and source. What it did not cover (heads greater than 1) is covered by `every_cited_*` with its updated units.

## 5. Decisions and limits

- **The window applies to relations**, to their `created_seq` (E7-01, §2). The minutes (§2.1, point 2) speak of «relations and revisions created within a window»; the trace cites each unit's head revision **in the snapshot**, with its `commit_seq`, but does not filter units by the `commit_seq` of their revision. It was not changed: that would be another semantics (and another field of the query) that neither the E7-03 row nor the evaluation set asks for, and E7-04 already uses recency by `commit_seq` as a ranking signal, not as a filter. It is recorded in case the owner wants it different.
- **The trace cites the result's revision, not one per step.** Intermediate units are seeds or results of the same answer (§3), each with its revision; `verify_trace` requires it.
- **I-40 is tested, not checked at query time.** There is no public verifier: the caller can check a result with `get_relation`, `get_revision` and `change_set` over the same snapshot, as `verify_trace` does. The check of each index answer (E7-02, §4) remains the usual one and does not see completeness (H-1).
- **Inferred relations.** This stage does not create inferred relations (Stage 8); I-39 is tested with relations written by the test with that class, which is what the relation layer admits today. «Pending» in the I-39 row of `invariants.md` (inferred relations produced by a model) remains so.
- **The clock.** No line of `assoc.rs` nor of the planner's path reads a clock. The test does not prove its absence: it shows that, with three different clocks and the same history, nothing changes (§7, M15 and M16).
- **Copy.** `assoc_context.rs` does not duplicate the corpus loader: it uses `load_corpus_with_relations`, which `load_corpus` now calls. `tests/assoc_reference.rs` keeps its own copy (see the «Copy» limit of [E7-02](stage7-indexed-execution.md#10-limits-and-risks)); it was not touched.

## 6. Dependencies

None new; `Cargo.toml` does not change.

## 7. Mutations

Each mutation was applied with `sed` to unmutated `crates/kerno-units/src/assoc.rs`, `cargo test -p kerno-units --release --test assoc_context` was run, the red tests were noted, and the file was restored from a copy (checked byte by byte with `cmp`; `git status` shows no changes in `src/`). All 16 compiled and all 16 were detected. «All» means the seven functions (14 tests with the two engines); each abbreviated name fails in its two engines.

| # | Mutation | Tests that failed |
|---|---|---|
| M01 | The window excludes its upper bound (`> w.hi` → `>= w.hi`) | `window_and_provenance_…`, `the_clock_orders_nothing` |
| M02 | The window excludes its lower bound (`< w.lo` → `<= w.lo`) | `window_and_provenance_…`, `the_clock_orders_nothing`, `frozen_corpus_filters_…`, `frozen_corpus_cites_…` |
| M03 | The class filter inverted | all except `an_old_snapshot_…` |
| M04 | The actor filter always true | `every_cited_…`, `the_clock_…`, `window_and_provenance_…`, `frozen_corpus_filters_…`, `frozen_corpus_cites_…` |
| M05 | The source filter always true | the same five as M04 |
| M06 | The actor filter compares with the source | the same five as M04 |
| M07 | The result's revision is the one after the head | all except `inferred_is_never_declared_…` |
| M08 | The step cites a relation identifier that does not exist | all |
| M09 | A step of an inferred relation is shown as `Declared` | `inferred_is_never_declared_…`, `every_cited_…`, `window_and_provenance_…`, `the_clock_…`, `frozen_corpus_cites_…`, `frozen_corpus_filters_…` |
| M10 | The step's `created_seq` is one more | `every_cited_…`, `the_clock_…`, `window_and_provenance_…`, `frozen_corpus_cites_…`, `frozen_corpus_filters_…` |
| M11 | The index path ignores the window | the same five as M04 |
| M12 | The index path ignores the classes | `inferred_is_never_declared_…`, and the same five as M04 |
| M13 | The index path ignores the actor | the same five as M04 |
| M14 | The index path ignores the source | the same five as M04 |
| M15 | Each level's frontier is ordered by the change set's timestamp | **only** `the_clock_orders_nothing` |
| M16 | The window also discards relations according to the change set's timestamp | **only** `the_clock_orders_nothing` |

What the table says and what it does not:

- `the_clock_orders_nothing` is the only one that kills M15 and M16: with a constant clock the frontier ordered by time keeps the identifier order, and only the backwards-moving clock changes it. The other tests do not see them.
- `inferred_is_never_declared_in_a_trace` kills M09 (the I-39 mutation of its own), M03, M08 and M12.
- M01 (the window's upper bound) is **not** detected by the frozen-corpus tests: their windows have no relation exactly at the upper bound. It is detected by `window_and_provenance_…` (exhaustive grid) and `the_clock_…`.
- `an_old_snapshot_never_cites_what_came_after_it` fails only with M07 and M08, and no mutation of the production code kills it alone: snapshot isolation is provided by the engine, and the test fixes that the trace does not depart from it. It is kept as a regression test of I-40, not as a guard of one concrete line.
- The tests of E7-01 and E7-02 stay green under the unmutated code; their own mutations are in [E7-01 §9.1](stage7-associative-query.md#91-mutations) and [E7-02 §8](stage7-indexed-execution.md#8-mutations).

## 8. Verification

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_context
```

Test count of `kerno-testkit` + `kerno-units` with `--release --all-features`: **625 passed, 0 failed, 8 ignored** with this change; before it was 611 (E7-02). The 14 new ones are the 7 functions of `assoc_context` × 2 engines; the runner counts them (`assoc_context`: `14 passed`, about 11 s).

## 9. Limits and risks

- **Coverage of the oracle.** The oracle only covers queries whose limit and budget are not reached. With tight bounds what is checked is `verify_trace` and the equality between paths, not the exact answer; the budget and the endings of E7-01 remain covered by their hand-made tests.
- **Small worlds.** The generated worlds have 9 to 13 units and about 20 to 40 relations: dense so that there are parallel relations, cycles and several paths to the same result. The frozen corpus provides the 2 000-unit scale.
- **The model belongs to the test.** If the test and the library shared a misreading of the documented order, the oracle would share it too. Mitigation: the order was written from the text of `assoc.rs`, and the hand vectors of E7-01 remain independent of both.
- **Test time.** The corpus tests walk the reference path, which is linear in the relations per expansion; `assoc_context` takes about 11 s in `--release`. Nothing is claimed about performance (E7-05).

## 10. Related documents

- [Stage 7 minutes](stage7-record.md) §3 (E7-03), §4 (I-39, I-40)
- [Associative query (E7-01)](stage7-associative-query.md) · [indexed execution (E7-02)](stage7-indexed-execution.md)
- [Evaluation set](stage7-evaluation-set.md) (frozen corpus and queries, unchanged)
- [ADR-0014](../adr/0014-physical-representation-of-relations.md): relations and provenance
- [Invariants](invariants.md): I-26 (the clock does not order), I-39 (inferred is not declared)
