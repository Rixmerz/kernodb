# Stage 6 · E6-02 — The result cache as a planner candidate

Contract of E6-02 ([Stage 6 minutes](stage6-record.md) §3 and decisions of §8). Guarantees: [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) (G1 to G7). Dependency model and store: [E6-01](stage6-dependencies.md). Footprint and plan: [stage5-footprint-and-plan.md](stage5-footprint-and-plan.md). The code is in `crates/kerno-units/src/planner.rs` (`answer_units`, `answer_relations`, `store`), `cache.rs` (`lookup`, `CachedQuery`, `MissCause`) and `db.rs` (`UnitDb::result_cache`).

It does not change the persistent format, the engine or any other crate. The cache stays **off by default** (minutes §8.2) and E6-02 measures nothing: measurement is E6-03/E6-04.

## 1. Path of `query` and `query_relations`

```
query(filter)
  └─ answer_units(filter)
       ├─ cache off ............................ plan_units(filter)      (the body from before, one atomic load of cost)
       ├─ no footprint key ..................... plan_units(filter)
       ├─ index_stats() fails .................. Fallback{PreconditionFailed}, plan_units(filter)
       └─ lookup(footprint, exact query, Dependencies::current(seq, index state))
            ├─ Hit ──► check_units(filter, rows)
            │            ├─ ok ........ returns the rows (path = ResultCache, source = Cache)
            │            └─ fails ..... Fallback{Discrepancy} + entry deleted,
            │                          the reference path answers, fallback = Some(ResultCache, Invalid(violation))
            ├─ Invalidated(dep) ........ Fallback{Invalidated}, planner
            ├─ Disabled ................ Fallback{Disabled}, planner
            └─ Miss / Inactive ......... (already recorded as Missed{cause}), planner
       ── after any path that did not serve from the cache: store(footprint, query, dependencies, rows)
```

`query_relations` is the same with `check_relations` and without index state (I-23: the dependencies carry `None`).

- **Switches.** Both —global and per query— are checked in `lookup` (E6-01), and the global one also before touching anything in `answer_*`. With both on, the cache is consulted; with either off, the path is the previous one.
- **Current dependencies.** They are built from the snapshot being read: its `commit_seq`, and for units the `IndexStatus` read from `IDX_META` in that same use. `lookup` compares the five dependencies of the entry with them (`first_change`), on each use (G4). The entry is served only if all match; if not, it is deleted and the first one that differs is named. A snapshot older than the entry also invalidates it —a foreign `commit_seq` is never served—, at the cost of two snapshots with different sequences taking turns at the entry.
- **One entry per query shape, with its literals.** The `Fingerprint` ignores literals (ADR-0015): `unit_type = "a"` and `unit_type = "b"` share a footprint. An entry that served the other would return foreign rows, so `CacheEntry` stores the exact query (`CachedQuery`, in memory only, never in the record) and `lookup` serves it only to an equal one; to another of the same shape it answers `Missed { OtherQuery }` without touching the entry. Consequence: queries that differ only in literals take turns at the slot.
- **Footprint key.** The cache generates its own the first time it is used (`fingerprint_of`); it is not comparable with that of another store or with that of the `Observer`. If the system cannot provide a key, `query` runs without the cache. `set_key` (for tests) deletes the learned state, because footprints under another key are not comparable.

## 2. Guarantees, one by one

| | How it is met in this package |
|---|---|
| G1 equivalence | The result served is that of an earlier execution at the same `commit_seq` and index state, with the same validations. The differential test checks it against the cache off. |
| G2 fallback | Every failure (precondition, invalidation, deactivation, discrepancy) executes a path without learned state and leaves `Fallback{cause}`. After a discrepancy the **reference path** answers, not the planner. |
| G3 observation is not authority | `answer_*` does not read the `Observer`. Creating an entry needs a real execution whose rows are stored; the evidence count is kept by the cache (one execution, `Evidence { observations: 1 }`), not by the observer. The observer's level neither activates, deactivates, keeps nor deletes entries. |
| G4 dependencies | `Dependencies::current` on each use; a commit moves `commit_seq` and makes earlier entries unservable. |
| G5 same validations | `check_units` / `check_relations` on the cached rows before returning them. |
| G6 reversible | Turning off deletes all entries (`Deleted{Disabled}`), returns to `plan_*` and touches no data, `commit_seq` or indexes. |
| G7 log | Each hit (`Used`), miss (`Missed{NoEntry, OtherQuery, NotActive, Expired}`), storage (`Created`, `Activated`, `Deleted{Replaced}`), invalidation (`Invalidated{dependency}`) and fallback (`Fallback{cause}`) is a `Transition` with footprint and versions; no field carries a filter, unit or row. |

**What decides to create an entry.** The minutes (§3) say "when an observation justifies it"; the package reduces this to the minimum that meets G3: a query that was not served from the cache and finished well stores its answer. There is no repetition threshold, because any such threshold would have needed to read the observer. A result larger than `max_bytes` is not stored and the query does not fail (the cache is advisory).

## 3. Changes to the public surface

Additions, no removals: `PlanPath::ResultCache` (outside `candidates_of`: it is chosen by exact match of dependencies, not by cost, and a `Plan` never lists it), `QuerySource::Cache { index }`, `PlanKind::Cache` (the observation of an execution served from the cache carries that path), `Transition::Missed { cause }` with `MissCause`, `CachedQuery`, `UnitDb::result_cache()`, `ResultCache::fingerprint_of` / `set_key`. `CacheEntry::new` (which additionally receives the `CachedQuery`) and `ResultCache::lookup` (which receives the query) change signature. With the cache off, the output of `query` and `query_relations` is the same as before: the same `QueryOutcome` / `RelationsOutcome`, byte for byte (checked by `turning_the_cache_off_restores_the_reference_behaviour`).

## 4. Tests

Test count (sum of the `test result` lines of `cargo test -p kerno-units --all-features --release`): 285 after E6-01 → 295 after E6-02. The 10 new ones are `tests/cache_planner.rs`, 5 tests × 2 engines (`Emulated<LogEngine>` and `KernoDb`).

| Requirement | Test |
|---|---|
| G1: identical results with the cache off, on and invalidated midway by interleaved writes | `cache_on_off_and_invalidated_give_identical_results`. Three identical databases, ten rounds of reads with a write between round and round (creates, updates, relates, rebuilds the index); an old snapshot is reread after each write. It is not empty: it requires hits between writes and `Invalidated{CommitSeq}` afterwards, and that the one with the cache off records nothing |
| G2 and G5: a failed check discards the entry and the reference answers | `a_failed_check_discards_the_entry_and_the_reference_path_answers`. Falsified entries, valid in all their dependencies and with rows of another query, unordered, archived (`NotMatching`, `OutOfOrder`, `NotActive`) and one of relations: none is returned, `fallback = Invalid(violation)`, `Fallback{Discrepancy}` and `Deleted{Discrepancy}` in the log |
| G6: turning off restores the reference | `turning_the_cache_off_restores_the_reference_behaviour`. Off by default: full output equal to a database without cache and an empty log; on, it serves; off again, it returns to the base output, with no entries, without writing `commit_seq` and without `Used`; the per-query switch |
| G3: observations are not authority | `observations_never_act_as_authority`. Twenty observations of the query do not create nor serve an entry; at the four observer levels, and after emptying it, the cache behaves the same; what was observed does not keep an entry alive after a write |
| G7: each use is a transition, without literals | `every_use_of_the_cache_is_a_transition`, including the `OtherQuery` miss and the search for `doc`, `img`, `cites` in all records |

## 5. Mutations

Method of the earlier stages, with one change: all files are already in git, so restoration was checked with `git diff`. Each mutation is an edit of the production code, `cargo test -p kerno-units --all-features --release --test cache_planner` (10 tests) was run and it was reverted. Real output:

| Mutation | Result |
|---|---|
| `check_units` is not called on a hit (`planner.rs`) | `a_failed_check_...` `FAILED` in both engines; `8 passed; 2 failed` |
| `check_relations` is not called on a hit | same; `8 passed; 2 failed` |
| `lookup` ignores the dependency changes (`first_change(now).filter(\|_\| false)`) | `cache_on_off_and_invalidated_...`, `every_use_of_the_cache_...`, `observations_never_...` `FAILED` in both engines; `4 passed; 6 failed` |
| `lookup` ignores the equality of the query (`if !same_query && false`) | `cache_on_off_and_invalidated_...` and `every_use_of_the_cache_...` `FAILED`; `6 passed; 4 failed` |
| `answer_units` also requires `self.observer.stats().retained != 0` (the observer becomes an authority) | `observations_never_act_as_authority` `FAILED` (in this mutation all 10 fail: with the observer at `Off` the cache stops being used; that it is not selective does not matter, what it proves is that depending on the observer is detected) |
| The `Fallback{Discrepancy}` is removed from the log | `a_failed_check_...` `FAILED`; `8 passed; 2 failed` |
| `store` is removed after the units query | all 10 `FAILED` (no entry ever comes to exist) |
| `drop_all_entries` is removed from `set_enabled` | `turning_the_cache_off_...` `FAILED` in both engines; `8 passed; 2 failed` |

The first run of the suite when the task was resumed found `lookup` with the `.filter(|_| false)` of the third row already in place —a mutation that the previous attempt left unreverted—: `an_entry_never_serves_another_commit_seq`, `each_declared_dependency_invalidates_the_entry_and_is_named` and `every_transition_leaves_a_record` were failing. It was reverted and the suite went green.

## 6. What remains

- E6-03: full reversibility (logical state, engine bytes and `commit_seq` before/after) and the suite with adaptation off.
- E6-04: measurement with the thresholds of §6 of the minutes. A creation criterion based on repetition, if the measurement asked for one, would have to keep not reading the observer (G3).
- The cost of two snapshots with different sequences taking turns at one entry remains open; the measurement will show whether it matters.
