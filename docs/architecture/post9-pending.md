# Open items after Stage 9

| Field | Value |
|---|---|
| Date | 2026-11-13 |
| Source | [Closure of Stage 9](stage9-closure.md) §5, the twelve points, in their order |
| What it is | The list of the open points of the closure with their current status. It is updated per later package; the closure is not edited |
| Package that opens it | **P1** (K1). **P2** updates points 3, 4, 5, 6, 10 and 11. **P3** closes the three open items of the review of ADR-0016 and H-3 of Stage 6 (section "Package P3"). **P4** (documentation) writes the 1.0 requirements, the CLA draft and the record of the decision on the randomness of identifiers (sections "Package P4" and "Decision by delegation of 2026-11-13"). The other points are as the closure left them: "no change" means that neither P1 nor P2 touched them, not that they have been reviewed again |

| # | Closure point | Status | Note |
|---|---|---|---|
| 1 | Roadmap, status of Phase 1 | Resolved (2026-11-13, in the closure) | No change |
| 2 | K-E3-7: K1, K2, K3, K4, K5 | **Partial**: K1 closed in P1 (`0c2c3d5`); K2, K4 and K5 open; K3 not reproduced | Breakdown below |
| 3 | `NoValidSources` message | **Resolved in P2** (`2356ff3`) | The text now names the datum: "no source of derived datum <id> is valid any more". `derived_stale.rs` (`Display`) and the two tests that fixed it (`consolidation.rs`, `derived_stale.rs`) change. Mutation Q7 |
| 4 | E9-12, derived-data stress (F-09) | **Resolved in P2** (`2356ff3`) in what the point asked: the "lock, commit" order | `crates/kerno-units/tests/derived_lock_order.rs`, two engines. A wrapper engine asks, from inside its own commit and on the thread that confirms, whether the registry lock is taken; a late lock gives `false` (mutation Q5). A new hook is needed, `DerivedDb::hook_registry_locked`, only with `test-support`: the hooks that already existed (`hook_flush`, `hook_wait_idle`, `hook_compact_range` of `UnitDb`) act on the engine and none sees the registry lock, which belongs to `DerivedDb`. Mutations Q5 and Q6. **It is not tested** that the lock stays taken during the marking: no external observation sees it without a competing reader |
| 5 | E9-03, redaction | **Resolved in P2** (`2356ff3`): there is already a mutation | The closure said that there was no production line that a mutation could break to leak content; there are: in `verify.rs:643` the text of a load finding is formed with the stored bytes (`stored`) in scope, and in `fingerprint.rs:92` the shape text of the observation is formed with the query filter (`filter.unit_type`) in scope. What enters those texts today are keys, lengths and condition names, not content. Mutations Q8 (adds the filter type to the shape text) and Q9 (adds the stored bytes to the finding): both red in the two engines. In addition `redaction_units.rs` now asserts that the load path of `verify` ran (`payload_findings >= 2`) and that a shape text was recorded (`shape_texts >= 1`); mutations Q10 and Q11. Fragments of less than 8 bytes are still not searched |
| 6 | E9-08g | **Resolved in P2** (`2356ff3`) | `compact()` has tests that fail without its poisoning check (`compact_refuses_a_poisoned_handle_{emulated_log,kerno_db}`, `reclaim.rs`; mutation Q1) and if its range widens (`compact_asks_for_the_revision_ranges`, `reclaim.rs`, and `compact_covers_exactly_the_revision_tags_kerno_db`, `db.rs`; mutations Q2 to Q4). `Emulated<LogEngine>` does not implement `Reclaim`: it runs behind the wrapper engine of `reclaim.rs` |
| 7 | E9-14, fuzzing (H-7) | **Open by nature** | `wal`, `manifest` and `open` only reached between 4.5 and 6.7 million runs in 1,801 s per target (`crates/kerno-storage/fuzz/README.md`, section of E9-14), with the machine not idle. A longer run of those three targets **on a machine at rest** is needed. That run is separate, does not exist yet, and its record will be added here when it exists |
| 8 | E9-21, example | Open, declared limit | No change. `set_policy` only receives a `HistoryPolicy` (`None`, `Audit`, `Versioned`; `crates/kerno-units/src/types.rs:144`), without `KeepLast(1)`. The example prunes with `prune_history` (`examples/agent-memory/src/lib.rs:576`). Its tests run only over `kerno_db`, by delegated decision |
| 9 | E9-20, measurement | **Open by nature** | P1 and P2 were measured **once**, with the machine loaded (load of 2.27 to 2.67 before the quality measurement; `stage9-measurement.md` §1). They need to be measured again **at rest**, in a separate run that does not exist yet. Meanwhile, the quality thresholds are printed and not asserted in the test (measurement §5) |
| 10 | E9-15, mutations of E8 | **Resolved in P2** (`2356ff3`): recorded | Q12 (`vector_store.rs:1230`, the reconstructed vector inverted) turns `quality_over_the_frozen_set` red. Report identical between engines: Q13, a mutation **of the test** (a figure of the report depends on the engine). The `kerno-units` code that walks the measurement has no branch that depends on the engine, so a mutation of its own does not separate the two reports; only an engine that departs from the contract separates them. The candidate tested in `kerno-storage` (Q13b) does not change the measurement and is seen by the engine's transaction tests (Q13c). See [post9-mutations.md](post9-mutations.md), P2 |
| 11 | Possibly intermittent test (`space_is_reclaimed_after_range_deletes`) | **Resolved in P2** (`2356ff3`): cause found, a timing assumption of the test, shared with `functional::space_is_reclaimed` | See the following section |
| 12 | Repository history | **Accepted blemish** (closure §5, point 12) | No change: the history is not rewritten. The commits of `main` with authorship trailers that the project rules prohibit, among them `c779b39`, remain in it |

## Point 2, breakdown

| Finding | Status | What shows it |
|---|---|---|
| **K1** | **Closed in P1 from the end that lost its entry**, for relations with a count record (layer 4). **Open, declared**: relations of a database from before layer 4 until a relation of its `(unit, direction, type)` is created or deleted, and the double damage (entry and count lost together) | [stage9-k-e3-7.md](stage9-k-e3-7.md) §9; [units-v4.md](../file-format/units-v4.md) §7; `crates/kerno-units/tests/rel_counts.rs`; [post9-mutations.md](post9-mutations.md) |
| K2 | Open, no change: corrected only for `IDX_PRIMARY` with the count intact; open with the count lowered and for `IDX_TYPE` | [stage9-k-e3-7.md](stage9-k-e3-7.md) §8 |
| K3 | Not reproduced, no change | `get_unit_checks_oldest_rev` |
| K4 | Open, minor, **no change**. P1 does not close it | `change_set_info` still does not cross the members with their `REV_META` |
| K5 | Open, minor, **no change**. P1 does not close it | `get_relation` still does not check `created_seq` nor the ends |

The independent review of E9-16 is still pending, and so is that of P1: the author of P1 is not its reviewer (minutes §4.10).

## Effect of P1 on a limit of Stage 7

The limit H-1 of [stage7-closure.md](stage7-closure.md) (R-2): with 64 relations or more, a relation created later and without either of its two adjacency entries was not detected at query time. With the counts of P1 it **is detected** (`Fallback` `Corrupt` at the hop) in a database with counts. The test that fixed the limit, `completeness_is_not_a_query_time_check`, is now called `a_relation_lost_with_both_entries_is_caught_by_its_counts` in `crates/kerno-units/tests/assoc_indexed.rs` and asserts the opposite; the closure of Stage 7 is not edited and its citation stays as the state at that time. The same limit persists for relations of a database from before layer 4 without a count.

## Point 11, the intermittent test

**What was done.** The test was read (`space_is_reclaimed_after_range_deletes`, `crates/kerno-testkit/src/harness/range.rs`) and the profile that makes it fail (`M0` of `crates/kerno-storage/src/testing.rs`: `CompactionConfig::m0`, with background threads to flush and compact, a single compaction at a time and a write brake that only sleeps 1 ms per write). The binary `shared_ranges` (release, `-p kerno-storage --all-features`) was run with `--exact m0_profile::shared_range::space_is_reclaimed_after_range_deletes`, one run per process. "With load" means six loops of 100 runs in parallel while the whole `cargo test --release -p kerno-storage --all-features` was running at the same time, on a machine with 4 CPU (1-minute load average between 3.2 and 3.6 at the end of each run).

| Test | Condition | Runs | Failures |
|---|---|---|---|
| `m0_profile::shared_range::space_is_reclaimed_after_range_deletes` | Alone, base tree | 200 | 0 |
| the same | With load, base tree | 600 | **4** (1, 0, 0, 1, 2, 0 per loop) |
| the same | Alone, with the change | 200 | 0 |
| the same | With load, with the change | 600 | 0 |
| `serial::shared_functional::space_is_reclaimed` (binary `shared`) | With load, base tree (three loops of 100 next to three of the change and the whole suite) | 300 | **1** |
| the same | With the same load, with the change | 300 | 0 |

**The failures.** The four of the test of point 11 are the same assertion, that of the size after refilling, never that of the keys: "cycle 7: 4843423 bytes on disk after refill, first fill used 910690 (bound 4691336, …)", "cycle 5: 3318243 … (bound 3070324, …)", "cycle 6: 4238167 … (bound 3459008, …)" and "cycle 4: 3716764 … (bound 3082648, …)". The data do not change from one run to another (fixed seed, `SimVfs`, same writes); what changes is how far the background threads have advanced when the test measures. Even the first fill, which fixes the bound, comes out different with load (from 505,437 to 910,690 bytes).

**Cause: a timing assumption of the test, not a defect of the engine.** The test measured the directory just after the last write of each refill and assumed that compaction had already caught up with the writer. With `M0` nothing guarantees it: the write returns before the background flushes and compacts, and with the machine loaded the single compaction thread can remain a whole cycle behind; the sample then sees the garbage of the previous cycles that is already queued to go. What confirms it is that, with the same engine and without writing anything else, waiting for the size to go below the bound is enough in the 600 runs with load: the engine does reclaim, only later. The sister test `functional::space_is_reclaimed` has the same assumption and failed in the same way with load (in one run of the whole suite, profile `serial`: "cycle 4: 3293880 bytes on disk after refill, first fill used 497008"; and once in the 300 of the table); it is corrected with it, in the same place.

**The change.** `settled_size` (`crates/kerno-testkit/src/harness/mod.rs`), used by the two space tests for each refill except the first: if the size exceeds the bound, it is measured again every 5 ms, for up to 30 s, while it stays above. An engine that does not reclaim stays above for the whole wait and fails with the same message and the size it keeps; the bound (`4 * first fill + 1 MiB`), the number of cycles and the check that the points only delete the ranges do not change. An engine that does not publish statistics (`Engine::stats` empty: the reference engine and the leaking engines of the harness's own tests) has no background work to wait for and is measured only once, as before, so those tests do not take longer. The wait polls a condition because the `RangeDeleteEngine` contract has no "wait for rest" (`TestHooks::wait_idle` belongs to `KernoDb`, not to the contract used by the harness).

**Mutations** ([post9-mutations.md](post9-mutations.md), P2): Q14 (`compaction.rs`, compaction stops removing the points that a range covers) turns the key assertion red; Q15 (`deleter.rs`, obsolete files are never deleted) turns the size assertion of the two tests red, in all profiles, after exhausting the wait; Q16 (`settled_size` returns 0 without measuring) turns red the harness test that requires an engine that does not reclaim to fail.

**Limits.** (1) The first fill is still measured once, without waiting: with load it comes out larger, and that only loosens the bound in that run, as before. (2) With an engine that does not reclaim, the test takes 30 s more to fail. (3) Not having failed in 600 runs with load does not prove that it cannot fail: a background that stops for more than 30 s would make it fail.

## Package P3: open work of ADR-0016 and H-3

Origin: "Review of 2026-11-13" of [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md), point 4, and H-3 of the [closure of Stage 6](stage6-closure.md). Mutations: [post9-mutations.md](post9-mutations.md), P3.

| # | Point | Status | What shows it |
|---|---|---|---|
| P3-1 | The plan explanation (`Plan`, `explain_*`) did not say whether the cache was consulted nor whether learned state was used | **Resolved in P3** | `Plan::cache()` (`CacheView`) and `Plan::answering_path()`; `explain_filter` and `explain_relations` compute them with `ResultCache::peek` without touching the cache. Test in both engines: `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers` and `the_plan_does_not_announce_an_entry_that_fails_its_check` (`cache_planner.rs`), which compare the plan with the `path` and the `source` of the next read. `explain_assoc` returns `NotApplicable`: the associative query does not consult the cache |
| P3-2 | Constant creation evidence `{1, 0}` | **Resolved in P3**, with a behaviour change | The evidence is that of the observer (`Observer::evidence_of`); without it the entry is not created, so it is not activated. With the observer below `Fingerprint` (its default value is `Off`) the cache on stores nothing. `observations_never_act_as_authority` and `relation_evidence_comes_from_the_observer` (`cache_planner.rs`) |
| P3-3 | The entries carry no cost | **Resolved in P3** | `CacheEntry::cost_ns` (nanoseconds measured for the execution it replaces), `Transition::Created { evidence, cost_ns }`, `CacheView::Hit { cost_ns }`; `NoCost` if it is missing. Limit: a single measurement per entry, not a cost model |
| P3-4 | `set_key` deletes without a record | **Resolved in P3** | `Transition::KeyReplaced { entries }` in the new log. `replacing_the_key_leaves_a_record_of_the_wipe` (`cache_planner.rs`, two engines) and `set_key_leaves_one_record_of_the_wipe_in_the_new_log` (`cache.rs`) |

**Effect on other pieces.** The bench `index_bench` raises the observer to `Fingerprint` in the arm with cache (without that the cache would store nothing) and therefore its measurement of that arm includes the cost of observing: it is not comparable with that of Stage 6. The tests `cache_admission`, `cache_forget`, `cache_reversibility` and `cache_sentinels` raise the observer for the same reason.

## Package P4: 1.0 requirements, CLA draft and identifier record

P4 is documentation. It does not change code, tests, `Cargo.toml`, `Cargo.lock` or `LICENSE`.

| Deliverable | Status |
|---|---|
| [requirements-1.0.md](requirements-1.0.md): audit of the seven requirements of D13 | Written. Six requirements are in "Met with declared limits" and one in "Met". **1.0 is not declared** |
| [cla-draft.md](../legal/cla-draft.md): draft of the CLA | Written as a **draft without force**. It awaits the review of a lawyer and the approval of the owner. Until then no external contribution is accepted |
| Decision on the randomness of identifiers (next section) | Recorded as an accepted limit, on this page and in the notes of [ADR-0005](../adr/0005-public-identifiers.md) |
| Note in [ADR-0019](../adr/0019-license-and-contributions.md) | One line: the draft exists and awaits the owner. The text of ADR-0019 is not rewritten |
| Link from Stage 9 of [docs/roadmap/README.md](../roadmap/README.md) | Added. The root README is not touched, and `CONTRIBUTING.md` is not created |

## Decision by delegation of 2026-11-13: randomness of public identifiers

| Field | Value |
|---|---|
| Date | 2026-11-13 |
| Who | Taken by express delegation of the owner. Revocable by him |
| Decision | Public identifiers (`PublicId`, option 1 of ADR-0005) read their randomness from the operating system source through `std`: `/dev/urandom`. On a platform without that source, creating a public identifier fails with a typed error and creates nothing |
| Dependencies | None new. The `Cargo.toml` and `Cargo.lock` of the repository have not changed since `52f026f` |
| Code | `crates/kerno-units/src/ids.rs` (`os_random` and `mint`). The error is `UnitError::Storage(kerno_core::Error::Io)` |
| Accepted limit | On Windows, WASI and other platforms without `/dev/urandom`, `create_public` returns that error. Resolving and identifying tokens already issued keeps working. A read with an error or short, or a token whose 16 bytes are equal, also fails and is not retried |
| What lifts it | (a) A randomness API in the standard library for the platforms the project supports. (b) The decision to accept a dependency, `getrandom`, which `ids.rs` names as the extension path. That decision goes through an express resolution and through I-31 |
| Where it is recorded | Here, and in the "Notes" section of [ADR-0005](../adr/0005-public-identifiers.md) |

**Note on the dependency rule.** The sentence "no third-party dependency in any engine crate" does not hold in this repository as it stands. `kerno-core` has no dependencies, and `CONTRIBUTING.md` guards it with `cargo tree`. But `kerno-storage` depends on `crc32c`, `parking_lot`, `quick_cache` and `crossbeam-skiplist`, and `kerno-units` on `crc32c` and `blake3`. The rule that truly applies to this decision is I-31 (minutes §4.8): no `Cargo.toml` nor `Cargo.lock` changes without a decision that requires it. For this decision no dependency is added.
