# Stage 3 report — `U` metrics (WP E3-09)

Status: results of 2026-11-01 · Related to: [Stage 3 design](../architecture/stage3-design.md) §9.4 and decision 12, [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md), [targets T1–T10](f1-performance-goals.md) (untouched), [Phase 1 report](f1-report.md)

This report measures U1–U6 with the thresholds that the owner approved **before** measuring (decision 12, 2026-11-01). The thresholds have not been adjusted to the results: what does not meet them is listed as **not met**. The only reference is internal (simple keys in `KernoDb`); the comparison with SQLite is left for Stage 9.

## 1. Summary

| Id | Threshold (decision 12) | Layer | Reference | Ratio | Result |
|---|---|---|---|---|---|
| U1 | p99 ≤ 1.5× | `update` 1 KiB `Sync`: p99 **0.557 ms** (p50 0.221) | put 1 KiB `Sync`: p99 **0.428 ms** (p50 0.175) | **1.30×** | **Met** (full run); in the quick run 1.82× (100 samples, no statistical value) |
| U2 | p99 ≤ 1.5× | `history(id, 100 rev)` metadata only: before p99 0.1061 ms (p50 0.048); **after p99 0.0541 ms** (p50 0.028) | snapshot + `scan` of 100 keys of 100 B: before 0.0580 ms; after **0.0367 ms** (p50 0.017) | before 1.83×; **after 1.47×** (repeat: 1.53×) | **On the limit**: met in the full run, narrowly not met in the repeat (see §3) |
| U3 | reported; warning if > 5× a `get` | `unit_as_of` with 1 000 revisions: p99 **0.0326 ms** | snapshot + `get` 1 KiB: p99 **0.0063 ms** | **5.13×** | **Warning** (> 5×). In an earlier identical run it came out 4.37×: it is at the limit |
| U4 | reported | see §4 | | | Informative |
| U5 | reported | 1 257.7 B per revision of 1 KiB | 1 068.0 B per key of 1 KiB | 1.178× | Informative |
| U6 | p99 and write throughput during pruning no worse than before, with the same load | p99 write during: before the fix 5 693 ms / 217.7 ms; **after 2.373 ms** (default budget) / **3.324 ms** (25 units) | "before" windows: 2.069 ms / 2.096 ms | after: 1.15× / 1.59× (p99 write); throughput 1.14× / 0.76×; p99 read 1.87× / 6.07× | **Not met** (both budgets), although the write p99 drops ~2 400× |

WAL byte cost of an `update` (pending E3-08): **1 416 B** per `update` of 1 KiB versus **1 077 B** per put of 1 KiB (+339 B, 1.31×); with 64 KiB, 65 929 B versus 65 590 B (+339 B, 1.01×). The overhead is fixed per `update` (keys `10`, `20`, `21`, `31`, `30` and their headers).

## 2. Environment, configuration and reproduction

| | |
|---|---|
| Machine | KVM VM, Intel Xeon @ 2.10 GHz, 4 vCPU (1 thread per core), 15 GiB RAM, virtio disk `/dev/vda` ext4 (90% full). Same VM class as M0/F1; it is not the target machine (C-09) |
| Software | Linux 6.18.44, rustc 1.97.0, `release` profile of the workspace |
| Commit | base `155ccf9` + this WP |
| Engine | `KernoDb` with `Options::default()`: `Sync`, cache 64 MiB, write buffer 8 MiB; `StdVfs` over `/tmp`; background threads by default |
| Layer | `UnitConfig::with_sources(FixedClock::stepping(.., 1), CounterIds::new(1))`, `Versioned` policy, rest by default |
| Seed | `0xE309` (each metric derives its own with a fixed XOR) |
| Method | layer and reference in different databases with the same options, in the same process; in U1–U3 they alternate by rounds (5 rounds, the order is inverted in each one) and each round discards a warm-up (U1: 200 operations; U2/U3: 10% of the queries). Percentiles by nearest rank. U2/U3 measure after `flush` + `wait_idle` of both databases |

Commands:

```
cargo run --release -p kerno-units --example unit_bench              # full, ~7.5 min (U6 ~7.4 min)
cargo run --release -p kerno-units --example unit_bench -- --quick   # quick, ~3 s (CI: bench-smoke job)
# options: --dir <path> --seed <u64> --only U1,U3
```

Raw output: [`unit_bench-full.txt`](../../spikes/storage/results/2026-11-01-e3/unit_bench-full.txt), [`unit_bench-quick.txt`](../../spikes/storage/results/2026-11-01-e3/unit_bench-quick.txt), environment in [`env.txt`](../../spikes/storage/results/2026-11-01-e3/env.txt). The quick run is a smoke test: its figures are not useful for deciding thresholds.

Full-run sizes: U1 1 000 units/keys, 5 × 1 000 measured operations per side; U2 100 units × 100 revisions of 100 B versus 10 000 keys of 100 B, 25 000 queries per side; U3 10 units × 1 000 revisions of 1 KiB versus 10 000 keys of 1 KiB, 25 000 queries; U4 100 units, 2 000 `update` of 1 KiB and 500 of 64 KiB; U5 100 units × 100 revisions of 1 KiB; U6 10 000 cold units with 20 new revisions of 100 B per round (5 rounds) and 100 hot units.

## 3. U1–U3: latencies

U1 per round (p99 in ms, `update` / put): 0.448/0.504 · 0.548/0.466 · 0.529/0.466 · 1.232/0.393 · 0.449/0.393. The aggregate p99 meets (1.30×), but one round (3) exceeds 1.5× on its own: the margin is not wide and depends on the variance of the VM's `fdatasync`. The mean is 0.245 ms versus 0.195 ms (1.25×).

U2 (before the fix): `history` took a snapshot, read the head and the policy (`get`), scanned 100 `REV_META` (it was already a single `scan`, with no point reads per revision) and decoded and validated each one; p50 2.8×, p99 1.83×.

Profile (callgrind and timing by phases, 100 revisions): engine `scan` ≈ 15–17 µs (same as the reference), `REV_META` decoding ≈ 10 µs (≈ 100 ns per record with the field-by-field reader), `Key::decode` per entry ≈ 1 µs, head + policy ≈ 1.6 µs. Changes (without touching the on-disk format):

- `RevisionMeta::decode` has a fast path of fixed offsets that accepts only what the checked decoder accepts, with the same result; on any anomaly it falls back to the checked decoder, which names the exact rule (test `rev_meta_fast_path_agrees_with_checked` with 20 000 valid and mutated values). Decoding 100 records goes from ≈ 10 µs to ≈ 3.6 µs.
- `history` compares the bytes of the expected key instead of decoding each key.

After: p50 0.028 ms versus 0.017 ms (1.65×), p99 **0.0541 versus 0.0367 ms = 1.47×** in the full run; in an immediate repeat of U1/U2, 0.0597 versus 0.0389 ms = **1.53×**; in five separate earlier U2 runs, 1.28×–1.54× (four of five meet it). That is, U2 is **on the limit**, not meeting it with margin. What remains above the reference is structural: decoding and validating 100 `REV_META`, building 100 `Revision` (224 B each) and two reads before the `scan`. The threshold is not lowered.

U3: 5.13× a `get` in this run (4.37× in an identical earlier run), 0.0326 ms absolute. The warning is marked. **The inverted index is not added** (§3.5): its cost (extra write and space per revision, format under `layer_version`) must be justified first against an absolute latency of tens of µs. For context, `get_unit` (head) p99 0.0075 ms.

## 4. U4: write amplification

| Size | Side | WAL bytes per op. | Total WAL | Flush | Compaction | (WAL+flush+compaction)/payload |
|---|---|---|---|---|---|---|
| 1 KiB (2 000 ops) | `update` | 1 416 | 2 832 000 | 2 632 819 | 0 | 2.67 |
| 1 KiB | put | 1 077 | 2 154 000 | 107 258 | 0 | 1.10 |
| 64 KiB (500 ops) | `update` | 65 929 | 32 964 500 | 32 982 172 | 39 539 895 | 3.22 |
| 64 KiB | put | 65 590 | 32 795 000 | 19 101 564 | 6 563 980 | 1.78 |

The difference in flush/compaction is structural: puts overwrite 100 keys (the memtable keeps the last version), while each `update` keeps a new revision (decision 4: full copy). It is the expected cost of history, not a defect. The WAL bytes confirm the hypothesis of §9.3: the overhead is in fixed bytes (+339 B per `update`), not in `fsync`.

## 5. U5: space per revision

After 10 000 revisions of 1 KiB (100 units × 100) and full compaction (`compact_range` of the whole range): 12 577 291 B in live tables, **1 257.7 B per revision**, versus 10 679 854 B for 10 000 distinct keys of 1 KiB (**1 068.0 B**). Overhead **+189.7 B per revision (1.178×)**: `REV_META`, key of `REV_PAYLOAD` and the part of `CHANGESET`/member.

## 6. U6: retention with concurrent writes

Identical load in the three windows: 2 writer threads in a closed loop (`update` 1 KiB `Sync` over 100 hot units) and 1 reader (`get_unit`). Two "before" windows of 3 s (A and B, to see the noise) and the "during" window = `run_retention(KeepLast(1))` repeated until `complete`. 5 rounds. Low load = 1 writer only.

| Budget | Window | p99 write (ms) | p50 write (ms) | Throughput (ops/s) | p99 read (ms) |
|---|---|---|---|---|---|
| default (1 000 u./10 000 ops) | low load | 0.581 | 0.221 | 4 018 | — |
| | before A / B | 1.761 / 2.379 | 0.624 / 0.687 | 2 959 / 2 584 | 0.0171 / 0.0224 |
| | **during** | **5 693** | 1 420 | **1.1** | 0.0762 |
| 25 u./100 ops | low load | 0.790 | 0.221 | 3 745 | — |
| | before A / B | 1.940 / 2.193 | 0.663 / 0.601 | 2 748 / 2 963 | 0.0234 / 0.0072 |
| | **during** | **217.7** | 74.4 | **24.6** | 0.1255 |

Retention throughput (before the fix): 7 083 revisions/s with the default budget and 6 681 with 25 units; concurrent writing was almost blocked. Cause: `run_retention` took the write slot at the start and, inside, walked **all** the heads from the beginning on every call (the lookups within a transaction with thousands of pending range deletes are also quadratic).

### 6.1 Fix (in this WP)

- **Planning outside the slot**: each call reads heads and `REV_META` in a snapshot, without the slot. Under the slot it only checks that the head of each planned unit (identical bytes) and the policy of its type are unchanged, re-plans within the transaction those that changed, and writes. If the head did not change, its retained revisions do not either (every write of the layer that touches them rewrites the head); `set_policy` increments an epoch that invalidates the cursor.
- **Resumable in-memory cursor**: an incomplete pass continues at the first unfinished unit (same policy, no `KeepFor`, no policy change). A complete pass, an error or a reopen start again from the first unit. After a crash there is nothing to recover (the pruning of each unit is still a single engine commit, I-21/IU-7).
- **Bounded traversal work**: one call reads at most 4 096 heads without work; it can return `complete = false` with no change set.
- **Ranges first**: the change set writes all the range deletes before the points (rewritten heads, members, `REV_META`), which never fall in those ranges; the result is the same and the engine does not re-examine the points already written with each new range.

The existing tests (including the comparisons with the model of `policies.rs`, `retention.rs` and the crash sweeps of `tests/harness/units.rs`) pass unchanged.

### 6.2 After the fix (full run, `unit_bench-full-after.txt`)

| Budget | Window | p99 write (ms) | p50 write (ms) | Throughput (ops/s) | p99 read (ms) |
|---|---|---|---|---|---|
| default | low load | 0.647 | — | — | — |
| | before A / B | 2.069 / 1.725 | 0.622 / 0.571 | 2 839 / 3 128 | 0.0222 / 0.0135 |
| | **during** | **2.373** (before 5 693) | 0.510 | **3 232** (before 1.1) | 0.0415 |
| 25 u./100 ops | low load | 0.493 | — | — | — |
| | before A / B | 1.606 / 2.096 | 0.575 / 0.581 | 3 123 / 3 026 | 0.0140 / 0.0118 |
| | **during** | **3.324** (before 217.7) | 0.741 | **2 290** (before 24.6) | 0.0852 |

Retention: 18 163 revisions/s (default) and 19 294 (25 units), versus 7 083 / 6 681. **U6 still fails** with both budgets: with the default, throughput now meets (1.14×) but the write p99 is 1.15× the worst "before" and the read p99 1.87×; with 25 units, p99 1.59×, throughput 0.76× and read 6.07× (2 020 calls, each with its `fsync`).

Remaining cause (measured): reading and traversal suffer the engine's cost with many ranges in the memtable (`Memtable::covering_seq` linear over up to 4 096 ranges per `get`, and `Memtable::fragments` recomputed on each `scan`; both marked `ponytail` in `kerno-storage`). Experiment **not included** in the commit (§9.5 of the design requires its own WP with a comparison of T1–T10 to touch `kerno-storage`): with a fragment cache in the memtable, the read p99 during pruning was 1.12×, and retention rose to ≈ 235 000–388 000 revisions/s, but then retention, going so fast in a closed loop, occupies the slot a good part of the time (≈ 50 ms per set of 1 000 units) and the write p99 worsened to 30–76 ms (`unit_bench-u6-experiment-engine-fragcache.txt`). Meeting U6 as defined (retention in a closed loop with no effect on p99) also requires retention pacing or shorter change sets in slot time; it was not done because it would change the API contract (one change set per call) or the benchmark's methodology.

## 7. Pending and uncertainties

- After E3-09b (§8): U2 meets in 2 of 3 runs; U6 still fails in all 3. Before E3-09b: U6 fails (much less than before); U2 is on the limit (1.47× / 1.53× in two runs); U3 is at the 5× limit with variance between runs (4.85× in the later run); U1 meets (1.25× / 1.43×).
- Raw output after the fix: `unit_bench-full-after.txt`, `unit_bench-u1u2-after-rerun.txt`; the previous one is kept in `unit_bench-full.txt`.
- A single machine (VM with a virtual disk); the target machine is still unnamed (C-09).
- `docs/testing/f1-performance-goals.md` (T1–T10) has not been modified.

## 8. WP E3-09b: memtable range index, retention pacing and fixed cost of `history`

Approved by the owner within PR #13. Goal: U2 and U6 meet stably without lowering thresholds, without touching T1–T10 or the methodology of `unit_bench`. **Result: not reached.** U2 meets in 2 of 3 full runs; U6 fails in the 3 (much closer than before). Details below.

### 8.1 Changes

- **`kerno-storage` (own WP according to §9.5 of the design):** the memtable keeps its fragmented range deletes incrementally (`RangeIndex`: a `BTreeMap` of disjoint pieces, each with the sequences that cover it). Adding a range costs `O(log n + k)` under the range lock (no rebuild); `covering_seq` goes from linear to `O(log n)`; `fragments(seq)` is walked from the index and, for readers at or after the last range, a list (`Arc`) built on the first read after a change is shared, not on the write. Without ranges there is no cost or allocation. No format changes; the two `ponytail` marks disappear. New test `range_index_matches_the_reference` (300 seeds, against `rangedel::fragment` and a linear pass, at every snapshot), seen to fail by mutation.
- **`kerno-units`, retention pacing (decision 12b of the design):** `UnitConfig::retention_pacing` (`RetentionPacing { units_per_change_set: 16, pause_ratio: 9 }` by default). A `run_retention` call writes its budget in several change sets of at most 16 units and, after each, pauses 9× the work since the previous pause (retention busy ~10% of the time). `RetentionReport` adds `change_sets` (all, in order); `change_set` is the last. The pruning of each unit stays in a single change set (I-21/IU-7).
- **`kerno-units`, `history`:** policy cache by epoch shared by the snapshots (the epoch goes up before and after every commit that can change a policy; a snapshot taken with the epoch odd or changing does not use the cache); page built directly in the requested order (without `reverse`) and `REV_META` decoded inline. Same on-disk format. New test `policy_cache_follows_snapshots`; removing the epoch check makes that test and the crash sweeps fail.
- **Crash sweeps:** `retention_every_point` is extended with `paced_power_loss_sync` and `paced_process_kill_buffered` (1 unit per change set): after each crash the store is exactly a prefix of the call's change sets, `verify_units` clean, heads intact and retention finishes the work. The harness of `model_crash`/`changeset_every_point` (`tests/harness/units.rs`), whose model is "one step = one commit with predicted sequence", fixes `units_per_change_set = usize::MAX`.

### 8.2 T1–T10 before and after (`spike-bench`, same session, interleaved binaries)

Method: "before" binary (HEAD `0cdd10f`) and "after" (this WP) of `spike-bench`, each load×repetition run alternating before → after → `lsm`/`btree`/`fjall` (references of the same session), scale `medium`, `Sync`, 3 repetitions (seeds 42–44), `concurrent_commits` with 8 threads and separately with 1; `bulk_load` `large` 1 repetition. Data and script in `spikes/storage/results/2026-11-01-e3/t1-t10-e3-09b/`. Medians of `kerno`:

| Id | Metric | Before | After | After/before | References (same session) |
|---|---|---|---|---|---|
| T1 | `agent_memory` p99 write | 2 257 µs | 2 314 µs | 1.03 | fjall 760 · btree 922 · lsm 5 620 |
| T2 | LiveSpec commit p99 | 2 605 µs | 2 265 µs | 0.87 | lsm 4 112 |
| | Cauce step commit p99 | 885 µs | 930 µs | 1.05 | lsm 464 |
| | `append_seq` insert / update p99 | 1 756 / 3 097 µs | 1 798 / 2 927 µs | 1.02 / 0.95 | lsm 3 830 / 5 599 |
| | `history_sim` p99 | 5 022 µs | 4 870 µs | 0.97 | lsm 8 368 |
| T3 | module-scan / symbol-get p99 | 575 / 29.6 µs | 587 / 30.2 µs | 1.02 / 1.02 | lsm 518 / 28.0 |
| | step-read / recall / recent-scan p99 | 81.4 / 53.9 / 63.2 µs | 81.6 / 51.5 / 58.8 µs | 1.00 / 0.95 / 0.93 | lsm 80.3 / 51.0 / 57.4 |
| T4 | close `bulk_load` medium / large | 185.5 / 375.7 ms | 194.8 / 336.3 ms | 1.05 / 0.90 | |
| T5 | `bulk_load` / `point_reads` 1h / 4h / `range_scans` (kops) | 162 / 250 / 841 / 11.7 | 181 / 244 / 800 / 11.8 | 1.11 / 0.98 / 0.95 / 1.01 | lsm 187 / 273 / 866 / 15.3 |
| | `small_commits` / `point_deletes` / `cauce` / `agent_memory` (ops/s) | 5 661 / 5 327 / 3 938 / 1 438 | 5 695 / 5 649 / 3 852 / 1 420 | 1.01 / 1.06 / 0.98 / 0.99 | |
| | `mixed` kops / `rmw_counters` txn/s | 67.0 / 3 790 | 65.9 / 4 133 | 0.98 / 1.09 | |
| T6 | `fsync`/commit 1 and 8 writers; `cc` 8 threads | 1.00 · 0.218 · 15 160/s | 1.00 · 0.218 · 15 300/s | 1.00 · 1.01 | |
| | `concurrent_commits` 1 thread | 5 952/s | 5 509/s | 0.93 | repeat ×6: 5 228 → 5 459 (1.04) |
| T7 | amplification `small_commits` / `bulk_load` | 4.05 / 3.33 | 1.17 / 3.31 | — / 1.00 | (T7 of `small_commits` is unstable between repetitions, already documented in F1) |
| T8 | reopen / recovery (`bulk_load` medium) | 3.09 / 7.34 ms | 4.72 / 8.22 ms | 1.53 / 1.12 | see repeat |
| T9 | `rmw_counters` p99 | 4 375 µs | 3 453 µs | 0.79 | |

The reopen/recovery differences and `concurrent_commits` with 1 thread were repeated because they exceeded the expected noise (no point path changed): 9 interleaved repetitions per side of `bulk_load` with the final binary give reopen 3.46 → 3.52 ms (1.02), recovery 8.64 → 7.25 ms (0.84), close 183.6 → 194.2 ms (1.06; ranges 171–282 / 178–207); `concurrent_commits` 1 thread ×6: 1.04. **Conclusion: no regression of T1–T10 above the noise between repetitions.** The final binary differs from the one measured in the table only in not allocating an empty `Arc` per `scan` when there are no ranges.

### 8.3 Full `unit_bench`, 3 runs (`unit_bench-full-e3-09b-run{1,2,3}.txt`)

| Id | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| U1 (≤ 1.5×) | 0.595 / 0.349 ms = 1.71× **not met** | 0.634 / 0.553 = 1.15× | 0.600 / 0.674 = 0.89× |
| U2 (≤ 1.5×) | 0.0555 / 0.0332 ms = **1.67× not met** | 0.0613 / 0.0425 = 1.44× | 0.0606 / 0.0461 = 1.32× |
| U3 (warning > 5×) | 6.79× warning | 5.53× warning | 4.71× |
| U4 / U5 | unchanged (+339 B; 1.178×) | same | same |
| U6 default: p99 write / throughput / p99 read | 1.26× / 0.91× / 0.87× | 1.09× / 0.95× / 1.02× | 1.08× / 0.98× / 0.69× |
| U6 25 u.: p99 write / throughput / p99 read | 1.11× / 0.96× / 1.90× | 1.17× / 0.95× / 0.73× | 0.98× / 0.95× / 2.04× |
| U6 verdict | not met ×2 | not met ×2 | not met ×2 |
| Retention (rev/s, default / 25 u.) | 16 112 / 13 997 | 17 886 / 13 226 | 18 898 / 13 794 |

U1 was not touched in this WP; its failure in run 1 is variation of the reference's p99 (0.349 versus 0.553 and 0.674 ms in the others), not of the layer (0.595–0.634 ms in all three).

### 8.4 Why U2 and U6 still do not meet stably

- **U2.** Measured breakdown (temporary instrumentation, not confirmed): head + policy ≈ 1.0–1.3 µs (before ~3 engine reads, now 1); `scan` of 100 `REV_META` ≈ 15.5 µs (same as the whole reference `scan`); decoding and building 100 `Revision` ≈ 9–10 µs (≈ 46 ns per decode already via the fast path, the rest is construction). The fixed cost the WP asked for is now ~5% of the total; what remains is per revision. The p50 ratio is ~1.6× and the p99 of both sides varies ±25% between runs, so the verdict oscillates around 1.5×. Lowering it further would require not materialising `Revision` (API change) or a copy-free `scan` in the engine.
- **U6.** With the range index, reading during pruning stopped degrading systematically (0.69–1.02× by default), and with pacing the write p99 sits at 1.08–1.26× (before E3-09: 2 400×; after E3-09: 1.15×). But the criterion requires not worsening *anything* against the worst "before" window, and pruning 1.1 M revisions per round generates unavoidable background work (each change set with its `fsync`, memtable rotations per 4 096 ranges, flushes and compactions) that costs 2–9% of writer throughput even though retention is only active ~10% of the time. Pacing sweep (one U6 run each): 32 u./1× → throughput 0.70–0.76×; 8 u./4× (pause by time under the slot) → 0.88–0.95×; 4 u./9× → 0.90–0.94×; 16 u./9× (pause by total work, chosen) → 0.95–0.98×. Throughput does not reach 1.0× when the duty cycle is lowered further, and the reader's read p99 (tens of µs) varies 2–3× between "before" windows of the same run, so it also oscillates. Meeting U6 as defined would require, at a minimum, retention commits without their own `fsync` (per-commit durability in the engine, which does not exist in the contract) or that pruning generate no flushes or compactions during the window; neither fits in this WP without changing the engine's contract.

### 8.5 CI fixes on `7a4f06d`

- **`policy_model` (seed 39, `SPIKE_SEEDS=1000`): retention that does not advance.** It was neither the bounded scan nor the pacing of E3-09b: with `RetentionBudget { max_units: 1, max_ops: 5 }` (valid: the design requires `max_ops ≥ 4` and a resumable retention), a unit `Audit` that needed pruning (4 operations) and also dropping some content (2 for each) never fit, because `fits` (and the same rule in `UnitModel::retain_within`) required pruning plus at least one content. Each call returned `complete = false` without a change set, forever; the model gave the same answer, so the test caught it by its rule "an empty call is complete". Fix in code and model alike: pruning alone (the contents for the next call) is enough progress; without pruning, the minimum is still one content. The test's rule does not change.
- **"scenario A on LazyFS".** The failure is not in the engine: the runner ships CMake 4, which no longer accepts `cmake_minimum_required` < 3.5, and the `toml` dependency that LazyFS downloads declares it, so `libpcache` did not build (`RESULT: PENDING`, exit 2). `run-lazyfs.sh` exports `CMAKE_POLICY_VERSION_MINIMUM=3.5` (CMake 4 reads it from the environment; earlier versions ignore it). The full scenario could not be run here: this environment's proxy returns 403 for the LazyFS dependency downloads (`spdlog`), so it remains pending the next CI run.

## 9. WP E3-09c: retention with deferred durability and a light history view

Approved by the owner in PR #13 (decision 12c of the design). Thresholds, T1–T10 and the `unit_bench` methodology unchanged. **Result: U1 meets in 3 of 3; U2 (defined over `history`) meets in 2 of 3 (1.50× in run 1, on the limit); U6 still fails in all 3.**

### 9.1 Changes

- **Contract (`kerno-core`):** `Txn::commit_deferred` (default `commit`) and `Snapshot::scan_visit` (default `scan`); no existing engine changes behaviour.
- **`kerno-storage` (own WP according to §9.5 of the design):** `WriteTxn::commit_deferred`; the group leader skips the `fsync` if all the accepted commits of the group are deferred, and `durable_seq` only advances after an `fsync` (it also fixed that a group with no accepted commits nor `sync()` marked it durable). Rotation syncs the previous WAL if it has unsynced frames, also in `Sync`; `sync()` does an `fsync` if there are pending deferred frames. `Version::scan_with` serves `scan_visit` without copying each entry. No format change.
- **`kerno-units`:** the `Retention` change sets are committed deferred (`CommitInfo::durable = false`); `history_page` / `HistoryPage` / `RevisionView` (same checks and errors as `history`; `history` does not change). `unit_bench` U2 still judges `history`; `history_page` is measured in the same pass, after the two judged sides and with its own random stream, and is reported without a verdict.
- **Tests:** `kerno-storage/tests/deferred.rs` (accounting of `fsync`/`durable_seq`; crash before each mutating I/O with deferred and durable commits mixed and rotations with unsynced frames, in `Sync`/`Buffered`, power loss and process death: always a prefix that contains every acknowledged durable commit). In `kerno-units`, the harness records the last acknowledged durable step, and `changeset_every_point`, `model_crash`, `recovery_reentrant`, `fsync_failure` and `torn_write` accept losing only the later retention steps; `retention_every_point` adds a user set after the retention (durable, and with it all the earlier retention) and checks that the retention does not `fsync`. `history_page` = `history` equivalence in `history_model` (pages in both orders) and in the error and corruption cases. All green with `SPIKE_SEEDS=1000`, `KERNO_KILL_ITERS=200`.

### 9.2 T1–T10 before and after (`spikes/storage/results/2026-11-04-e3/t1-t10-e3-09c/`)

Same method as §8.2 (interleaved binaries, `medium`, `Sync`, 3 repetitions, seeds 42–44, `concurrent_commits` with 8 and with 1 thread, `bulk_load` `large` 1 repetition), "before" = `4be9053`. To avoid spending more machine time than necessary, the references (`lsm`/`btree`/`fjall`) were **not repeated**: the comparison is only `kerno` before/after. Medians (after/before): T1 `agent_memory` p99 write 2 345 → 2 281 µs (0.97); T2 LiveSpec 2 869 → 2 869 µs (1.00), Cauce 886 → 976 µs (1.10), `append_seq` insert/update 1 570/3 348 → 1 863/2 783 µs (1.19/0.83), `history_sim` 5 288 → 4 809 µs (0.91); T3 module-scan/symbol-get 565/32.5 → 577/31.1 µs; T5 `bulk_load` 150 → 181 kops, `point_reads` 1h/4h 216/774 → 228/690 kops, `range_scans` 11.6 → 11.9 kops, `small_commits` 5 636 → 5 702/s, `mixed` 76.7 → 79.4 kops, `rmw_counters` 4 770 → 4 556 txn/s; T6 `fsync`/commit 1.00 and 0.216 unchanged, `cc` 8 threads 15 540 → 15 680/s, 1 thread 6 163 → 6 162/s; T7 unchanged (1.17 / 3.30–3.33); T8 reopen/recovery `bulk_load` 3.64/8.77 → 4.35/8.06 ms; T9 `rmw_counters` p99 3 234 → 3 189 µs. The three that moved most were repeated 6 more times per side (seeds 45–50, `recheck-*.jsonl`): `small_commits` p99 469 → 401 µs (0.85), `point_deletes` reopen 3.61 → 3.22 ms (0.89), `append_seq` insert p99 1 846 → 1 695 µs (0.92). **Conclusion: no regression of T1–T10 above the noise between repetitions.**

### 9.3 Full `unit_bench`, 3 runs (`unit_bench-full-e3-09c-run{1,2,3}.txt`)

The three runs were repeated in full because a first batch coincided with a compilation (`clippy`) on the same machine; only the runs on a quiet machine are listed.

| Id | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| U1 (≤ 1.5×) | 0.505 / 0.481 ms = 1.05× | 0.554 / 0.506 = 1.09× | 0.491 / 0.447 = 1.10× |
| U2 `history` (≤ 1.5×) | 0.0520 / 0.0346 ms = **1.50× not met** (1.503) | 0.0607 / 0.0479 = 1.27× | 0.0505 / 0.0533 = 0.95× |
| `history_page` (informative) | 0.0313 ms = 0.90× | 0.0422 = 0.88× | 0.0310 = 0.58× |
| U3 (warning > 5×) | 5.03× warning | 4.66× | 4.78× |
| U4 / U5 | unchanged (+339 B; 1.178×) | same | same |
| U6 default: p99 write / throughput / p99 read | 1.18× / 0.89× / 1.28× | 1.09× / 0.93× / 0.43× | 1.12× / 0.92× / 2.54× |
| U6 25 u.: p99 write / throughput / p99 read | 1.12× / 0.94× / 1.22× | 1.07× / 1.06× / 0.36× | 1.05× / 0.94× / 1.08× |
| U6 verdict | not met ×2 | not met ×2 | not met ×2 |
| Retention (rev/s, default / 25 u.) | 21 808 / 18 757 | 27 719 / 19 021 | 26 378 / 19 121 |

### 9.4 Reading

- **U2.** The light view sits at 0.58–0.90× of the reference `scan`, that is, the layer no longer adds its own cost when `Revision` is not materialised; `history` still oscillates around 1.5× because of the construction of 100 `Revision` (≈ 10 µs) and the variation of the reference's p99. Judging U2 over `history_page` would change the definition of decision 12 and is left to the owner.
- **U6.** Without its own `fsync` the retention is ~30–50% faster (21 800–27 700 rev/s versus 16 100–18 900 in §8.3), and the write p99 during pruning sits at 1.05–1.18×, but the writer's throughput remains at 0.89–0.94× (1.06× once) and the reader's p99 oscillates 0.36–2.54× with "before" windows that vary 2–3× between each other. What remains is the background work that pruning generates (range rotations, flushes, compactions), not the `fsync`; no threshold was changed.
