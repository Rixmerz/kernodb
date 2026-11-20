# Stage 6 · E6-03 — Reversibility and deactivation

Contract of E6-03 ([Stage 6 minutes](stage6-record.md) §3 and §4). Guarantee: [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) G6 (adaptation is reversible). Invariants: I-34 (optimisations part) and I-36. Background: the store and its dependencies in [E6-01](stage6-dependencies.md); the use in the planner in [E6-02](stage6-integration.md). The tests are in `crates/kerno-units/tests/cache_reversibility.rs`; the fixtures they share with E6-02, in `tests/harness/cache_script.rs`.

It does not change production code, the persistent format, the engine or any other crate. E6-03 only adds tests and this document. It does not measure performance: that is E6-04.

## 1. What is asserted

The learned state (the result cache) is derived data, in memory, outside every transaction (ADR-0016, decision). G6 requires that enabling it, disabling it, invalidating it or deleting it changes **nothing** of what the user confirmed. "Nothing" is checked with three measures at once, before and after each operation:

| Measure | How it is taken | What it covers |
|---|---|---|
| Engine bytes | `raw(db)`: all the key/value pairs of the engine in one snapshot | confirmed data, revisions, relations, indexes, index metadata |
| `commit_seq` | `seq(db)`: the sequence of the engine's snapshot | that no operation confirms anything, not even an empty transaction of null effect |
| Logical state | public API: answers of the five unit queries and of the relation queries, history of each unit, `change_sets_since(0)`, `index_status` | that the queries still mean the same (G1, G6) |

In addition, `verify_units` (E3-07) finds nothing after each operation.

## 2. The tests

Each one runs on both engines (`Emulated<LogEngine>` and `KernoDb`): 10 tests in total.

| Test | Criterion | What it does |
|---|---|---|
| `every_operation_on_the_cache_leaves_the_data_as_it_was` | I-34, G6 | On a database with history, relations and a built index, and with the cache warm (entries and hits > 0, so that it is not vacuous): enable, disable, disable and re-enable a query, delete an entry, change the footprint key, lower the limits and `clear()`. Compares bytes, `commit_seq` and logical state before and after, checks that the operation did act on the learned state (entries, switch) and that reading afterwards does not write either |
| `invalidating_by_each_dependency_leaves_the_data_as_it_was` | I-34, G4, G6 | Plants a valid entry and queries with current versions that differ in **one** of the five dependencies (schema, planner, index state, watermark, `commit_seq`). The entry is deleted, its `Invalidated { dependency }` remains and the data do not change. The list of dependencies is written in the test, not taken from `Dependency::ALL` (see M6) |
| `a_cache_in_use_never_changes_what_a_commit_writes` | I-34, G6 | Two databases with the same script of ten rounds of reads and writes; one with the cache on, whose entries each write invalidates. After each round, and after a `clear()` midway, bytes and `commit_seq` are equal to those of the database without cache |
| `with_the_cache_off_no_optimization_takes_part` | I-36, G6 | With the cache at its default (off) the full script runs: no read comes from `ResultCache`, none has `QuerySource::Cache` or `fallback`, and `stats() == CacheStats::default()` —the usage counter (`reads`, `hits`, `writes`, …) is at zero— and the log is empty. A direct `lookup` returns `Disabled` and moves nothing |
| `a_cache_switched_off_after_use_stops_taking_part` | I-36, G6 | After using the cache, switching it off (global) leaves the entries at zero, and the counters and log still throughout the script, answers equal to those of a database that never had it, and equal bytes. Switching off a query: it still does not hit, neither reads nor writes, a direct `lookup` returns `Disabled` (not `Miss`) and leaves no record; the other queries keep being served |

### The full suite with adaptation disabled

The cache is off by default (minutes §8.2) and the rest of the `kerno-units` suite does not turn it on, so `cargo test -p kerno-units --all-features --release` **is** the full suite with adaptation disabled; there is no separate mode to switch on. What makes it an assertion and not a coincidence is `with_the_cache_off_no_optimization_takes_part`, which fixes the default value and the counter at zero. Result of this stage: 305 tests pass (295 before E6-03 + 10 new), 1 ignored already existing (`struct_vectors`).

## 3. Mutations tested

Each mutation is applied to the production code, `cargo test -p kerno-units --all-features --release --test cache_reversibility` is run, which tests die is noted, and the file is restored (`git checkout`). All die.

| ID | Mutation | File | Die |
|---|---|---|---|
| M1 | Disabling leaves the cache consulted: `set_enabled` always stores `true` | `cache.rs` | `a_cache_switched_off_after_use_stops_taking_part`, `every_operation_…` |
| M2 | Disabling does not remove the entries | `cache.rs` | the same |
| M3 | `clear()` does not delete the entries | `cache.rs` | `every_operation_…` |
| M4 | `delete()` removes nothing | `cache.rs` | `every_operation_…` |
| M5 | `lookup` ignores the per-query switch | `cache.rs` | `a_cache_switched_off_after_use_stops_taking_part` |
| M6 | `CommitSeq` outside the checked dependencies (`Dependency::ALL`) | `cache.rs` | `invalidating_by_each_dependency_…` |
| M7 | Cache on by default | `cache.rs` | `with_the_cache_off_…`, `invalidating_by_each_dependency_…` |
| M8 | `UnitDb::result_cache()` commits an engine transaction (put and delete a key: moves `commit_seq`, leaves no bytes) | `db.rs` | `a_cache_in_use_…`, `a_cache_switched_off_…`, `every_operation_…`, `invalidating_by_each_dependency_…` |
| M9 | `UnitDb::result_cache()` creates a unit (moves bytes and `commit_seq`) | `db.rs` | the same |
| M10 | A `lookup` with the cache off counts a read | `cache.rs` | `with_the_cache_off_…` |

Notes on what was learnt by testing:

- **"`clear()` that raises `commit_seq`" cannot be written as such.** `ResultCache` has no engine handle: the type already prevents a store operation from writing. The closest thing that does compile is to make the only path to the store, `UnitDb::result_cache()`, write (M8, M9); each test that calls a store operation goes through it.
- **Four mutations survived the first version of the tests, and the tests were corrected:** M5 (the cache already prevented creating entries for an off query, so `Disabled` and `Miss` were indistinguishable by the counters; now a direct `lookup` is called and `Disabled` with no record is required), M6 (the test walked `Dependency::ALL`, the same list the mutation trimmed), M10 (the planner never calls `lookup` with the cache off, so it must be called directly) and M8 in its empty-transaction form (an empty engine transaction does not consume a sequence: equivalent; it was replaced by putting and deleting a key).
- **Order matters in the tests.** `freeze` reads through the planner; with the cache on, that fills it. The operation's statistics are taken before freezing the "after", and in the dependencies test the "before" is frozen with the cache off.

## 4. Status of the criteria of the minutes (§4)

| Criterion | Evidence in E6-03 |
|---|---|
| Invalidating, deactivating or deleting does not alter data (I-34, G6) | the first three tests of §2, with M1 to M9 |
| Deactivation verified (I-36, G6) | the last two tests of §2, with M1, M5, M7, M10 and the full suite |

The status in the table of the minutes is not changed here: the full promotion of I-34 and of I-36 and the update of §4 correspond to E6-05, as with E6-01 and E6-02.

## 5. What E6-03 does not cover

- **Performance.** Neither the cost of the cache when off nor the net improvement: E6-04.
- **Persistence of the learned state.** It does not exist: the cache lives in memory and disappears when the database closes, so "deleting the state" after a restart is trivial. If a later stage persisted it, this test would have to be extended to the bytes of that store.
- **Concurrency between switching off and a read in progress.** `lookup`, `create` and `fallback` check the switch again under the lock (E6-01); there is no specific race test here.
- **The observer of Stage 5.** Its level and its records are independent of the cache (G3, tested in E6-02); here it is only checked that switching the cache off does not depend on it.
