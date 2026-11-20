# Stage 6 · E6-01 — Dependency model and learned-state store

Contract of E6-01 ([Stage 6 minutes](stage6-record.md) §3 and decisions of §8). The code is in `crates/kerno-units/src/cache.rs`. Guarantees: [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) (G4 explicit and versioned dependencies, G7 transition log); privacy: [ADR-0015](../adr/0015-privacy-of-observability.md). Starting fields: [adaptive execution](adaptive-execution.md), [footprint and plan](stage5-footprint-and-plan.md), [observation](stage5-observation.md).

E6-01 **does not connect** the cache to `query()` or to the planner: that is E6-02, described in [stage6-integration.md](stage6-integration.md) (from there `lookup` also receives the exact query, `CacheEntry::new` stores it, and `Transition::Missed` exists; what follows describes the model and the store as E6-01 fixed them). It does not change the persistent format or any other crate.

## 1. Dependency model (G4)

A `CacheEntry` is pinned by the `Fingerprint` of the query (the keyed footprint of `fingerprint.rs`; no other hash is defined) and carries a `CachedResult` (the rows as the query returned them) with all the dependencies under which it was computed.

| `Dependency` | Version it compares | Changes when |
|---|---|---|
| `SchemaVersion` | `LAYER_VERSION` | the record formats of the layer change |
| `PlannerVersion` | `PLANNER_VERSION` | the planner's behaviour changes |
| `IndexState` | `Ready`=0, `Building`=1, `Stale`=2, `Failed`=3 | the index that was read changes freshness |
| `IndexWatermark` | the `commit_seq` the index reflects | the index is rebuilt |
| `CommitSeq` | the `commit_seq` of the snapshot | any commit |

- **`CommitSeq` requires an exact match.** An entry never serves another `commit_seq`, neither higher nor lower; there is no exception of the kind "what it read did not change" (minutes §2.3).
- **No entry exists without all its dependencies.** `CacheEntry::new` receives a `Dependencies` whose fields are not optional and rejects what could not be explained truthfully: a result of a different query kind from the footprint's (`KindMismatch`), an index state that does not correspond to the kind (a units query records it, a relations query does not, as in `Plan::new`; `IndexStatus`), and absence of evidence (`NoEvidence`).
- **The check comes before each use.** `ResultCache::lookup(fingerprint, &Dependencies::current(...))` compares the entry's set with the current one (`Dependencies::first_change`) in the order of the table. The first that differs is returned as `Lookup::Invalidated(dependency)` and the entry is deleted (I-35). An index state absent on one side and present on the other counts as a change of `IndexState`.
- `version_of` gives the number of each dependency. A relations query has no index (I-23) and an index that was never built has no `watermark`: in both cases the version is `None` and is compared as such.

Possible results of `lookup`: `Hit` (active entry, within its age and with all dependencies equal), `Miss`, `Inactive` (created and not activated), `Invalidated(dependency)` and `Disabled`. A `Hit` does not replace the planner's validation (G5): `query` passes the rows through `check_units` / `check_relations` before returning them (E6-02).

## 2. Store

`ResultCache` is in memory only and separate from user data: it has no database handle, opens no transactions and writes nothing to the engine, so it cannot form part of a user transaction. Its entries contain user content (a result is rows), which is why nothing is persisted or logged.

Bounded by size and age (`CacheConfig`, same style as the retention of `observation.rs`): `max_entries` (256), `max_bytes` (16 MiB, estimated with `CachedResult::approx_bytes`), `max_age` (1 h from creation) and `max_log_records` (1024; the records also expire at `max_age`). The oldest is evicted first. A result larger than `max_bytes` is refused (`TooLarge`). `set_config` validates (no limit may be 0) and applies the limits at once. `clear()` deletes all the learned state —entries, log and counters— and leaves a single `Cleared { entries }`; the switches are kept, as `Observer::clear` keeps the level.

## 3. Switches

- **Global**, **off by default** (minutes §8.2). `set_enabled(false)` removes all entries.
- **Per query**: `disable_query(fingerprint)` / `enable_query`; turning it off removes the entry of that footprint.
- With adaptation off, globally or for that query, no entry is read or written: `create`, `activate` and `lookup` return `Disabled` without touching the store, and `fallback` records nothing. A `lookup` with the global switch off costs one atomic load. The state is checked again under the lock.

## 4. Transition log (G7)

Each decision on an optimisation leaves a `TransitionRecord`:

| `Transition` | When |
|---|---|
| `Created { evidence }` | `create`; the `Evidence` is `observations` (≥ 1) and the window they cover. The entry does not serve yet |
| `Activated` | `activate`; activating an entry already active records nothing |
| `Used` | a `lookup` served the entry |
| `Missed { cause }` | (E6-02) a `lookup` found nothing to serve: `NoEntry`, `OtherQuery` (the entry is for another query of the same shape), `NotActive` or `Expired` |
| `Fallback { cause }` | `fallback`, with `PreconditionFailed`, `Invalidated`, `Disabled` or `Discrepancy` (the last one also deletes the entry) |
| `Invalidated { dependency }` | `lookup` found that dependency had changed |
| `Enabled` / `Disabled { switch }` | switch `Global` or `Query` |
| `Deleted { reason }` | `Requested`, `Replaced`, `EvictedBySize`, `EvictedByAge`, `Disabled`, `Discrepancy` |
| `Cleared { entries }` | `clear` |

Every record carries `optimization_version` (`OPTIMIZATION_VERSION` = 1) and `planner_version` (that of the entry; `PLANNER_VERSION` if the record is not from an entry). Following ADR-0015 the fields are: a local increasing `id`, the footprint (`None` for the global switch and `clear`), the transition, the two versions, the `commit_seq` of the entry and `recorded_at`. There is no field for a filter, a unit, a row or a message: the errors (`CacheError`) are classes with no content.

## 5. Tests

Test count (sum of the `test result` lines of `cargo test -p kerno-units --all-features --release`):

| | Tests |
|---|---|
| Before E6-01 (base tree `666e4c3`) | 263 |
| After E6-01 | 285 |

22 are added: 18 in `cache::tests` (`src/cache.rs`) and 4 in `tests/cache.rs` (2 tests × 2 engines). Before, it was counted with the same command over the base tree in a separate `git worktree`.

| Requirement | Test |
|---|---|
| One test per dependency that changes, entry invalidated and dependency named (I-35) | `each_declared_dependency_invalidates_the_entry_and_is_named` (the `match` is exhaustive: a new dependency does not compile without its case) |
| Never serves another `commit_seq` | `an_entry_never_serves_another_commit_seq`; with real snapshots and a real commit and rebuild, `a_commit_invalidates_the_cached_result_of_an_earlier_snapshot` |
| Fixed vectors of the set | `dependency_vectors`, `first_change_vectors` |
| Size and age bounds | `retention_is_bounded_by_the_number_of_entries`, `..._by_bytes`, `..._by_age`, `the_log_is_bounded_by_size_and_age` |
| Off stores nothing | `disabled_stores_nothing_and_reads_nothing`, `the_per_query_switch_stops_one_query_and_drops_its_entry` |
| Each transition has its record | `every_transition_leaves_a_record` |
| No literal in any record, at any level | `no_filter_literal_in_any_transition_record_at_any_level` (sentinels of `sentinels.rs` seeded in data and filters; control that the stored rows do contain them; the four observation levels; checks that each kind of transition appears, so that it is not empty) |

## 6. Mutations

Method of the earlier stages, with one caveat: `cache.rs` is a new file not tracked by git, so restoration was checked with `cmp` against a copy, not with `git diff`. Each mutation is a `sed -i` on the production code (the last one, on the test), `cargo test -p kerno-units --all-features --release --no-fail-fast --lib cache` and `... --test cache` were run, and the file was restored. Real output:

| Mutation | Result |
|---|---|
| `CommitSeq => Some(self.commit_seq.0)` → `Some(0)` | `--lib`: `FAILED` `an_entry_never_serves_another_commit_seq`, `dependency_vectors`, `each_declared_dependency_invalidates_the_entry_and_is_named`, `every_transition_leaves_a_record`, `first_change_vectors`; `13 passed; 5 failed`. `--test cache`: `emulated_log::a_commit_invalidates_...` and `kerno_db::a_commit_invalidates_...` `FAILED`; `2 passed; 2 failed` |
| `SchemaVersion => Some(1)` | `each_declared_...` and `first_change_vectors` `FAILED`; `16 passed; 2 failed` (`dependency_vectors` stays green: `LAYER_VERSION` is 1 and the vector does not distinguish) |
| `PlannerVersion => Some(1)` | `each_declared_...` and `first_change_vectors` `FAILED`; `16 passed; 2 failed` (same reason) |
| `IndexWatermark => self.index.map(\|_\| 0)` | `dependency_vectors`, `each_declared_...`, `first_change_vectors` `FAILED`; `15 passed; 3 failed`. It is the defect the tree carried when the task was resumed: the test detected it before the mutation |
| `IndexState` always `Ready` | `dependency_vectors`, `each_declared_...`, `first_change_vectors` `FAILED`; `15 passed; 3 failed` |
| Size bound (`while ... max_entries \|\| ... max_bytes` → `while false`) | `retention_is_bounded_by_bytes`, `retention_is_bounded_by_the_number_of_entries` `FAILED`; `16 passed; 2 failed` |
| Age in `lookup` (`if created_at.0 < oldest_kept` → `if false`) | `retention_is_bounded_by_age` `FAILED`; `17 passed; 1 failed` |
| Age at creation (filter of `prune_entries` → `&& false`) | `retention_is_bounded_by_age` `FAILED`; `17 passed; 1 failed` |
| Cache on by default (`AtomicBool::new(true)`) | `disabled_stores_nothing_and_reads_nothing`, `every_transition_leaves_a_record`, `records_carry_versions_...`, `the_log_is_bounded_by_size_and_age` `FAILED`; `14 passed; 4 failed` |
| `create` ignores the two switches | `disabled_stores_nothing_and_reads_nothing` `FAILED`; `17 passed; 1 failed` |
| `lookup` ignores the global switch | `disabled_stores_nothing_and_reads_nothing` `FAILED`; `17 passed; 1 failed` |
| `lookup` ignores the per-query switch | `the_per_query_switch_stops_one_query_and_drops_its_entry` `FAILED`; `17 passed; 1 failed` |
| The `Used` record is removed | `--lib`: `every_transition_leaves_a_record`, `records_carry_versions_...`, `the_log_is_bounded_by_size_and_age` `FAILED`; `15 passed; 3 failed`. `--test cache`: `no_filter_literal_in_any_transition_record_at_any_level` in both engines `FAILED` (the transition kind is missing); `2 passed; 2 failed` |
| The `Invalidated` record always names `CommitSeq` | `each_declared_dependency_invalidates_the_entry_and_is_named` `FAILED`; `17 passed; 1 failed` |
| (on the test) the report of `Lookup::Hit` includes the rows (`format!("{r:?}")`) | `unit type sentinel "SNT-TYPE-7f3a91c2" leaked at level Off:`, in both engines; `2 passed; 2 failed` |

The last mutation simulates a leak that no current production code has: there is no field of `TransitionRecord` nor variant of `CacheError` that holds content, so there is no small production mutation that leaks a literal. What it shows is that the sentinel search can fail with real rows in play.

Two more mutations, on the deletion:

| Mutation | Result |
|---|---|
| `st.drop_all_entries(DeleteReason::Disabled, now);` is commented out in `set_enabled` | `disabled_stores_nothing_and_reads_nothing` and `every_transition_leaves_a_record` `FAILED`; `16 passed; 2 failed` |
| `st.entries.clear();` is commented out in `clear` | `clear_deletes_all_state_and_keeps_the_switches` `FAILED`; `17 passed; 1 failed` |

## 7. What was left for E6-02

Connect `lookup`/`create`/`activate`/`fallback` to the path of `query`, validate the rows served before returning them (G5), and decide when an observation justifies creating the entry. Done in [stage6-integration.md](stage6-integration.md).
