# Decision on condition 1 of ADR-0001 (WP05)

> **Open decision.** T1 is still not met after the conditional step, and this document records a **candidate** for NO-GO, not a NO-GO. All figures come from a machine shared with other packages (1-minute load between 0.7 and 7.4 in the WP05 sessions), and the spread between runs of T1 (2.0–3.8 ms) is of the order of the distance to the target. The decision waits for the WP14 repetition with the machine at rest, and the owner takes it. **Update:** lever 1 of §3 (preallocated and recycled WAL) is already implemented and measured ([§8 of the diagnosis](f1-queue-diagnosis.md#8-wal-preallocation-and-recycling)); T1 is still not met (2.08× fjall and 1.93× btree) and the status does not change. **Second update:** levers 4 and 5 of §3 (dynamic levels and byte priority; incremental flush with `sync_file_range`) are implemented and measured ([§9 of the diagnosis](f1-queue-diagnosis.md#9-follow-up-on-condition-1-wal_preallocate-incremental-flush-and-dynamic-levels)); with the new default values T1 is at 2.99× fjall and 1.67× btree (the previous default value, measured in the same session: 4.74× and 2.65×), it is still not met and the status does not change.

| Field | Value |
|---|---|
| Status | **Candidate for NO-GO, provisional.** The owner decides; the reference measurement is the WP14 one with the machine at rest |
| Date | 2026-10-28 |
| Origin | Decision rule of [§5.4 of the Phase 1 design](../architecture/phase1-design.md): T1 unmet by > 25% ⇒ conditional step; still unmet ⇒ this document |
| Data and analysis | [WP05 results](f1-queue-diagnosis.md#7-wp05-results), [`spikes/storage/results/2026-10-28-f1-wp05/`](../../spikes/storage/results/2026-10-28-f1-wp05) |
| Targets | [f1-performance-goals.md](f1-performance-goals.md) (T1 and T4) |

## 1. What was measured

| Target | Result of `kerno` with the default levers | Target |
|---|---|---|
| T1, `agent_memory` write p99, versus fjall | 2.65× (2.43× with subcompactions and 2 threads) | ≤ 2.0× |
| T1, versus spike-btree | 1.93× (1.77×) | ≤ 1.5× |
| T1 with the preallocated and recycled WAL (lever 1; 15 repetitions over two sessions, load 1.1–1.6) | 2.08× (2.44× without the lever) | ≤ 2.0×: **not met** |
| T1 with the preallocated WAL, versus spike-btree | 1.93× (2.27×) | ≤ 1.5×: **not met** |
| T1 with levers 4 and 5 (`bytes_per_writeback` + `dynamic_base_level`, both on when measured; today `dynamic_base_level` is off by default; 5 repetitions, load 1.1–2.4) | 2.99× (4.74× with the previous default values, same session) | ≤ 2.0×: **not met** |
| T1 with levers 4 and 5, versus spike-btree | 1.67× (2.65×) | ≤ 1.5×: **not met** |
| T4, close after `bulk_load` `medium` | 204 ms with the close levers (587 ms without them) | ≤ 250 ms: **met** |
| T4, close after `bulk_load` `large` | 376 ms (1 270 ms) | ≤ 1 000 ms: **met** |

M0 was at 5.8 ms of write p99 (3.71× fjall, 5.36× btree), 759 ms and 3 587 ms of close. The WP05 levers halve the p99 (5.1 ms with the `m0` profile versus 2.6 ms in the same sessions), but they do not reach T1; T4 is reached with the close levers ([§7.4 of the results](f1-queue-diagnosis.md#74-t2t5-reportmd-largejsonl-median-of-3-repetitions)). In the final session (`results.jsonl`, load 1.1–1.2) T1 stands at 3.12× fjall and 1.85× btree. The load was repeated with the machine less loaded (5 repetitions, load 1.3–1.7) and the conditional step was applied (subcompactions, `max_subcompactions = 4`, and `max_background_compactions = 2`); the effect of the conditional step (−8%) is within the spread.

## 2. Why it does not reach the target

According to the diagnosis (§3–4 of the [WP04 report](f1-queue-diagnosis.md)), 99% of the tail is the `fdatasync` of the WAL, which is slow while another write is pending on the disk (the engine's own or other processes'): 12 ms of p99 with background writes versus 4 ms without them, and 0.42 ms in isolation. The WP05 levers reduce the amount and the duration of background writes, but they do not eliminate them, and none changes what the `fdatasync` has to confirm. The tail that remains is the one any engine with its own WAL pays on this filesystem. fjall, which has a smaller tail, also writes less in the background (`write_amp` 1.6 versus 3.0 in `bulk_load`); this document does not determine whether that is the cause.

The `bulk_load` close was stuck in its mandatory stretch (compact until the L0 metric drops below `l0_trigger`), which with a random load rewrites all of L1 for each L0 file that drops. Without relaxing that floor, the close now takes only the L0 files it needs, splits that compaction into subintervals, does not sync by stretches, and does not start a budgeted compaction that will not fit in the budget: 204 ms in `medium` and 376 ms in `large`.

## 3. Next levers

In order of the relation between expected effect and cost, and all **to be measured** before any is adopted:

1. **WAL recycling and preallocation** (design §9/§10.2): a WAL with its size already allocated and written, so that its `fdatasync` does not confirm metadata changes of the filesystem journal. It directly attacks the mechanism of the tail (H3). **Done** (`Config::wal_preallocate`, off by default, no format change; [§8 of the diagnosis](f1-queue-diagnosis.md#8-wal-preallocation-and-recycling)). Result: the `agent_memory` write p50 drops 40%, and the T2 p99s of LiveSpec, `append_seq` and `history_sim` drop 30–40%; the T1 p99 improves 15% at the median, without separating the distributions; the `fdatasync` microbenchmark shows a gain only with the machine loaded by other processes. Costs: +1.0 `write_amp` in `bulk_load`, +13 ms of `reopen_ms` and up to one more WAL on disk. T1 is not met.
2. **Group commit** for concurrent writers (it reduces the number of `fdatasync` per commit; it does not help a single serial writer such as the one in `agent_memory`, so its effect on T1 is smaller).
3. (Done for T4: subcompactions only on close, `close_subcompactions = 8`; the background ones stay at `max_subcompactions = 1` because at 4 they did not improve T1.)
4. **Reduce background writes**: dynamic level sizes and compaction by byte priority; the limit is space amplification. **Done** ([§9.3 of the diagnosis](f1-queue-diagnosis.md#93-lever-4-dynamic-levels-and-byte-priority)). `dynamic_base_level` (L0 compacts directly into L2 while L1 is empty and L2 is small; off by default, pending the owner): −30% of T1 p99 in 6 of 6 repetitions, −7% of compaction bytes, 22 → 17 compactions; cost: +54% of the LiveSpec commit p99 (still at 0.74× `lsm`) and the clean close does not skip L1. `file_picker = MinOverlap` (the file that rewrites the fewest bytes of the level below): −17% of p99 and −23% of compaction bytes, but it delayed the files with the most garbage and failed the space-reclaim test after range deletes; the picker now passes first the files with range-delete fragments and the test passes (figures not repeated with that correction): **off**. `write_amp` of `bulk_load` does not change.
5. **Write the SSTs with direct I/O or with `sync_file_range`** so that the kernel flush does not compete with the WAL (filesystem-dependent; R-S3 of the diagnosis). **Done with `sync_file_range`** (`File::start_writeback` / `wait_writeback`, `bytes_per_writeback`; direct I/O was not tried) ([§9.2](f1-queue-diagnosis.md#92-lever-5-incremental-flush-of-the-ssts-with-sync_file_range)). Requesting the flush without waiting does not bound the tail (WAL `fdatasync` p99 10 ms, as with nothing); with the wait on the same range, 3.6 ms versus 2.5 ms with `bytes_per_sync` in the microbenchmark. In the engine, replacing the intermediate `sync_data` calls with that flush with a wait lowers T1 p99 by between 10% and 24% and the Cauce (T2) p99 by between 21% and 34%: on by default (`bytes_per_sync = None`, `bytes_per_writeback = 1 MiB`).

## 4. Contingency (R-10/R-11)

The design refers to the contingency R-10/R-11 (C-33) at this point, without detailing it. Two paths are proposed for the owner to choose whether to confirm the NO-GO after the WP14 repetition:

- **Keep the own engine** and record T1 and T4 as documented misses (the close of condition 1 becomes "bounded tail at 2–3× instead of 11×"), attacking lever 2 of §3 as Phase 2 work (1, 4 and 5 are already done and are not enough).
- **Or reopen the choice of engine** (ADR-0001, condition 1) with the figures of §1: fjall remains 2.4–2.9× better on this load at the cost of 1.5× space amplification, 420 ms of reopen and 136 MB of peak resident memory (`report.md`).

The sequential execution of Phase 1 continues meanwhile, and the v1 format does not depend on this decision.

## 5. Limits of this decision

- All figures come from a shared machine (1-minute load between 0.7 and 7.4) and from a single filesystem (ext4 on a virtual disk). The ratio with fjall varied between 2.43× and 2.87× depending on the session.
- The conditional step was measured with 15 interleaved repetitions, but an 8% effect is not distinguishable from the spread between runs (2.0–3.8 ms).
- The Cauce T2 (`step_commit_p99_us` 2.31× `lsm`) was attributed to the intermediate `sync_data` calls of `bytes_per_sync` in the background compactions: removing them leaves it at 1.18× `lsm`, within the target, but raises the T1 p99 from 2.3 to 3.4 ms. It is a conflict between two targets and the owner decides: the default value stays at 1 MiB, which favours T1 ([§7.4 of the results](f1-queue-diagnosis.md#74-t2t5-reportmd-largejsonl-median-of-3-repetitions)).
- The measurement of lever 1 ([§8.3](f1-queue-diagnosis.md#83-agent_memory-and-t1)) has the same limitation (1-minute load 1.1–2.0; quartiles of the T1 p99 with lever 1 at 708–2 466 µs): the 15% improvement does not separate the distributions. Its firmest evidence is the p50 (−40%), which is not the T1 target. With it, `module_scan_p99_us` versus `lsm` goes from 1.16× to 1.32× in one session (10–14% in three, within the spread and unexplained), above the 1.2× target.
- The LiveSpec `module_scan_p99_us` meets the target versus `lsm` (0.93×; the earlier 1.24× was noise at load 5+) and does not meet it versus btree (2.02×), as M0 did (2.18×): it is not caused by a WP05 lever.

- The measurement of levers 4 and 5 ([§9 of the diagnosis](f1-queue-diagnosis.md#9-follow-up-on-condition-1-wal_preallocate-incremental-flush-and-dynamic-levels)) has the same limitations: 1-minute load 1.1–2.4 in the final session, 6 repetitions per A/B, and the ratio with fjall depends on fjall measuring 783 µs in that session (1 044–1 059 µs in those of §8). The improvements of `dynamic_base_level` and of the combination (−30%) repeat in 6 of 6 repetitions; the 10–15% ones do not separate the distributions.
- `wal_preallocate` was left off after 15 more pairs ([§9.1](f1-queue-diagnosis.md#91-wal_preallocate-does-it-help-it-stays-off)).
