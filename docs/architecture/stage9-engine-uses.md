# `UnitDb` accessors that replace `engine()` (E9-08a, corrected in E9-08f and E9-08g)

| Field | Value |
|---|---|
| Package | E9-08a of [the Stage 9 minutes](stage9-record.md) §3; corrections E9-08f (decision D23 of §8.1) and E9-08g |
| Rule | §4, wrapper rule: no accessor returns the engine nor a reference to it |
| Scope | `crates/kerno-units/src/db.rs` and `crates/kerno-units/Cargo.toml` (feature `test-support`) |
| Status of the code | `UnitDb::engine()` no longer exists: `grep -rn "pub fn engine" crates/kerno-units/src` returns nothing. The engine is a `pub(crate)` field; outside `db.rs` only `compact` (retention) and `reclaim` (E9-07) use it, through `inner_engine()` and the public trait `Reclaim` of `kerno-core` |

## 1. Correspondence with the old uses

The uses of `engine()` were split by the method invoked on the engine. As the function no longer exists, no search reproduces the old counts, and this document gives none. Each old use has today its accessor:

| Old use | Current accessor |
|---|---|
| `.snapshot()` (raw keys, dumps, `seq`, retained views) | `raw_snapshot()`, and `raw_get(key)` / `raw_seq()` when only one value or the sequence is read |
| `.commit(batch)` (damaging or repairing bytes) | `raw_commit(entries)` |
| `.stats()` | `storage_stats()` |
| `.flush()` | `hook_flush()` |
| `.wait_idle()` | `hook_wait_idle()` |
| `.compact_range()` | `hook_compact_range(start, end)` |

The test helpers (`settle`, `stat`, `put_all`, `undo_of`…) are written over these accessors.

## 2. Accessors

All on `UnitDb<E>`.

| Accessor | Requires | `#[doc(hidden)]` | Needs `test-support` | Returns | Note |
|---|---|---|---|---|---|
| `raw_snapshot()` | `E: TxnEngine` | Yes | No | `E::Snapshot` | Read-only view; it is not the engine. It calls the same poisoning check as `snapshot` |
| `raw_get(key)` | `E: TxnEngine` | Yes | No | `Option<Vec<u8>>` | Reads through `raw_snapshot`: same check |
| `raw_seq()` | `E: TxnEngine` | Yes | No | `u64` | Reads through `raw_snapshot`: same check |
| `storage_stats()` | `E: TxnEngine` | No | No | `Vec<(String, u64)>` | The engine's counters by name. Empty in `emulated_log` (`Emulated<LogEngine>` has no counters); not empty in `kerno_db` |
| `raw_commit(entries)` | `E: TxnEngine` | Yes | **Yes** | `u64` | A commit of the engine below the layer; `None` deletes the key and, if a key repeats, the last entry wins. It exists only for tests that damage stored bytes. It does not pass through the policy epoch (neither the policy cache nor the retention cursor sees the change), nor through the poisoning check, nor does it retract the derived records nor the suppression that a layer deletion retracts |
| `hook_flush()`, `hook_wait_idle()`, `hook_compact_range(start, end)` | `E: TxnEngine + TestHooks` | Yes | **Yes** | `()` | Test hooks of the engine; today only `KernoDb` implements them |

`test-support` is declared in `crates/kerno-units/Cargo.toml` and off by default. Without it, a build has no write path below the layer nor hooks for arbitrary raw range compaction. The public reclaim calls are two, both on the public trait `Reclaim` of `kerno-core` and not on `TestHooks`: `reclaim()` (E9-07, D14), which compacts the whole key space, and `compact()` (retention, E3-06), which compacts the revision ranges (`REV_META` and `REV_PAYLOAD`). The crate's own unit tests see `raw_commit` and the hooks with `cfg(test)`. The integration tests and the examples that call them declare `required-features = ["test-support"]`: `cargo test -p kerno-units` without the feature compiles and skips those targets, and `cargo test -p kerno-units --features test-support` (or `--all-features`) runs them. The examples are launched with `cargo run --release -p kerno-units --features test-support --example <name>`.

No accessor returns `&E`, `E` or the `UnitDb`.

## 3. Current use

Count of calls, not of lines, with `grep -rnoE "\.(raw_snapshot|raw_get|raw_seq|raw_commit|storage_stats|hook_flush|hook_wait_idle|hook_compact_range)\(" crates examples --include=*.rs`: each match is a call, and a line with two calls counts two. It includes the internal calls of `db.rs` (`raw_get` and `raw_seq` call `raw_snapshot`) and the tests of `accessor_tests`.

| Accessor | Calls |
|---|---|
| `raw_snapshot` | 50 |
| `raw_commit` | 26 |
| `raw_get` | 20 |
| `hook_wait_idle` | 13 |
| `storage_stats` | 9 |
| `hook_flush` | 8 |
| `raw_seq` | 7 |
| `hook_compact_range` | 6 |
| **Total** | **139** (in 137 lines) |

## 4. Verification

Tests in `crates/kerno-units/src/db.rs` (`accessor_tests`), over `Emulated<LogEngine>` (`emulated_log`) and `KernoDb` (`kerno_db`):

- `raw_commit` with several entries in a single commit and with repeated keys (the last one wins, whether a write or a deletion): the sequence advances exactly what a commit of one entry advances, so that one commit per entry does not pass; `raw_get` with two different keys stored and one absent; a `raw_snapshot` view taken before a commit keeps its value.
- `raw_snapshot`, `raw_get` and `raw_seq` return `Storage(Poisoned)` on a poisoned database, as `snapshot` does.
- `storage_stats` empty in `emulated_log`; in `kerno_db`, not empty and with the counter `commits` raised by two after two commits. A counter that the engine does not report makes the test fail; it is not read as zero.
- Hooks, only in `kerno_db`:
  - `hook_flush` raises `flushes` by one and empties the memtable.
  - `hook_wait_idle` waits for the background flush of a rotated memtable (with latency on each synchronisation of the simulated disk) until `immutable_memtables = 0`. The loop that causes the rotation has a maximum of 64 batches and fails if `memtable_rotations` does not change in them. That the test distinguishes a `hook_wait_idle` that does not wait depends on the flush, slowed by the latency, still being in progress when the counters are read; it is not guaranteed by construction.
  - `hook_compact_range` with bounded and non-symmetric limits (`Included(key 10)` / `Excluded(key 20)`), with the tombstones of keys 10 to 19 and those of keys 20 to 31 in two separate L0 tables: exactly the 10 tombstones of the range are discarded, and a later compaction of the whole key space discards the 12 outside, which were still there. With unbounded limits (22 discarded), inverted ones (none) or with the final end included (22) the test fails.
