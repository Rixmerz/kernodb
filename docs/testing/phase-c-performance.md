# Phase C performance: U2 (K-E3-8), U6 (K-E3-9), space and as-of cost

Roadmap Phase C asks to close the two performance gaps that Stage 3 left open
([closure, §5](../architecture/stage3-closure.md)). This report says what was measured before,
what changed, and what was measured after. **One gap is closed. The other is not, and the
numbers say why.** §5 adds two measurements that close no gap: space per revision (U5) and the
cost of an as-of read (T1).

| Gap | Metric | Threshold | Before | After | Status |
|---|---|---|---|---|---|
| K-E3-8 | U2: p99 of `history(id, latest(100))` against a scan of 100 keys | ≤ 1.5× | 1.53–1.77× (3 of 3 runs fail); 1.46–2.60× in the same-session runs | 0.85–1.07× (4 of 4 pass) | **Closed** |
| K-E3-9 | U6: writer p99 and throughput while `run_retention` runs | p99 ≤ the worse "before" window, throughput ≥ the lower one | p99 1.14–2.21×, throughput 0.73–0.85× (every run fails) | p99 1.04–1.93×, throughput 0.79–0.90× (4 of 4 runs fail); retention itself 1.1–2.6× faster (A/B, §3.5) | **Open** (§3.6) |

## 1. Method and its limits

- **Harness.** `unit_bench` (`crates/kerno-units/examples/unit_bench.rs`), unchanged methodology:
  same options and warmup for the layer and for its plain-engine reference, rounds alternating, full
  mode, seed `0xE309`.
  `cargo run --release -p kerno-units --features test-support --example unit_bench -- --only U2`
  (or `--only U6`).
- **Four options were added to the example, no threshold or window was touched**: `--diag` prints
  what the engine counted in each U6 window (stalls, slowdowns, the transaction gate, bytes
  flushed and compacted), `--pacing UNITS,RATIO` runs with another
  `RetentionPacing`, `--budget UNITS,OPS` runs U6 with one retention budget only, and `--control`
  runs the same writers for as long again with no retention after each retention window.
- **The runs are on tmpfs** (`/tmp`, the example's default `--dir`). `fsync` is free there, so a
  commit costs microseconds instead of the milliseconds of a disk, and U6 is CPU-bound. They are
  therefore **not comparable with the Stage 3 and Stage 5 numbers**, which were measured with a real
  `fsync` (writes of 3 000–4 000 ops/s against 15 000–25 000 here). Compare rows of this report
  with each other.
- **The machine was shared and loaded by other jobs.** The load average was 25–30 on 14 hardware
  threads during the runs of §2 (each raw file starts with the `loadavg` it ran under; the runs
  made before the machine restarted, §2.1, ran at a load of about 6–9). Latency percentiles are
  inflated by preemption at that load, so §2 also gives medians, which are steadier, and
  alternates before and after.
- Raw outputs: [`spikes/storage/results/2026-11-18-phase-c/`](../../spikes/storage/results/2026-11-18-phase-c/).

## 2. K-E3-8 / U2: closed

### 2.1 Cause

`history` read its `REV_META` entries with an engine `scan` that returns every key and value as an
owned `Vec` (two allocations per entry), decoded each value into an owned `RevisionMeta`, and built
a `Revision` from that. The reference it is compared with does one scan of 100 keys of 100 B. The
lightweight `history_page` (E3-09c) already avoided the copies and sat at 0.8–1.07× of the
reference; `history`, which has to hand out owned revisions, sat at 1.5–1.8×. The median showed it:
about 29 µs against 18 µs for the reference.

### 2.2 Change

`history` now takes the same single pass as `history_page` (`scan_history`, over the engine's
`scan_visit`, which lends each key and value without copying): each `REV_META` is decoded in
place and made a `Revision` as the engine hands it over. The two reads share the missing-entry
check and the `commit_seq` order check (`check_commit_order`, IU-3). Same queries, same errors for
a single fault, same results (the existing differential suites pass unchanged). When a page has
more than one fault, the one reported can differ from before: every `REV_META` is now decoded
before any order check or payload read, in both reads, so a damaged value is reported ahead of an
out-of-order one.

### 2.3 Before and after

Runs of the old (commit `e015af8`) and the new binary alternated in one session, load 25–30:

| Run | Before: p99 history / scan | Before: p50 history / scan | After: p99 history / scan | After: p50 history / scan |
|---|---|---|---|---|
| 1 | 1.54× FAIL | 1.60× | 1.07× PASS | 0.98× |
| 2 | 1.46× PASS | 1.64× | 0.96× PASS | 0.90× |
| 3 | 1.52× FAIL | 1.65× | 0.85× PASS | 0.99× |
| 4 | 2.60× FAIL | 1.60× | 0.99× PASS | 1.10× |

Before the change, in a quieter session (load 6–9; `before-U2-run{1,2,3}.txt`): 1.54×, 1.53× and
1.77×, all FAIL, with the median at 28–30 µs against 18 µs. After, in that session
(the final shape of the change; those outputs were lost when `/tmp` was cleared and only the
printed ratios were kept): 1.05×, 1.06× and 1.18×, all PASS, median 19–20 µs. In both sessions the
median of `history` fell from about 1.6× the reference to about 1.0×.

A first version of the change, which still went through an intermediate page and only avoided the
`scan` copies, measured 1.24×, 1.56× and 1.44× at p99 and looked like a partial win; its median
(27–30 µs, unchanged from before) showed it was not. The p99 of the reference moves by ±30 % between
runs, so judge this metric by the median and by alternating runs.

The threshold is not lowered or judged on another method: U2 is still `history`, not
`history_page`.

### 2.4 Tests

`history` and `history_page` now share three branches that no test distinguished. A sweep of
22 one-line mutants over the new code found 11 that survived the existing suites: the error for a
missing `REV_META` did not name the right revision (a gap in the middle, the last revision
missing), the first-or-last choice among several undecodable entries, an undecodable entry at all,
an equal `commit_seq`, and a `commit_seq` above the head's. `history_checks_every_entry`
(`crates/kerno-units/tests/history.rs`) pins each of them for both reads and both orders, and a page that does not
reach the damage must still succeed. After it, all 22 mutants are killed.

## 3. K-E3-9 / U6: open (the cost of retention itself reduced, the gap not closed)

### 3.1 Baseline

Three runs, both retention budgets (`before-U6-run{1,2,3}.txt`, tmpfs, load 6–9):

| Budget (units/ops) | Write p99 during / before | Throughput during / before | Read p99 during / before |
|---|---|---|---|
| 1000/10000 | 1.25×, 1.38×, 1.39× | 0.85×, 0.81×, 0.80× | 1.03×, 0.99×, 1.16× |
| 25/100 | 1.35×, 2.21×, 1.38× | 0.82×, 0.73×, 0.80× | 1.03×, 1.61×, 1.23× |

Every run fails the write p99 and the throughput condition, as in Stages 3, 4 and 5.

### 3.2 What the engine counts

One run with `--diag` (`diag-U6-default-tmpfs.txt`, budget 1000/10000, load about 30, the second
budget was stopped by hand after 20 minutes) shows where the extra cost comes from. Totals over
the five rounds:

| | Before windows | During retention |
|---|---|---|
| Writer throughput | 16 405 ops/s | 12 530 ops/s |
| Bytes flushed | 21 MB/s | 16 MB/s |
| Bytes compacted | 134 MB/s | 150 MB/s |
| Compaction bytes per flushed byte | 6.4 | 9.1 |
| **Compaction bytes per write** | **8 185** | **11 946 (+46 %)** |

During retention the engine compacts about 46 % more bytes for each write the writers make, and the
compaction work is already running at about 135 MB/s in the "before" windows. The engine's
`slowdown_*` counters are zero in this budget (the other budget counted 465 slowed commits out of
1.9 million), so this is not write stalls: it is the writers sharing the CPU and the memory
bandwidth with more background work. This is the cause the Stage 3 closure named ("rotations,
flushes and compactions of the background work that pruning generates"), with a number on it.

What is *not* the main cause: the write slot. Retention is paced to be busy about a tenth of the
time (`RetentionPacing { units_per_change_set: 16, pause_ratio: 9 }`), in change sets of 16 units
with deferred durability, so it holds the slot for a small fraction of the time and without an
`fsync`.

### 3.3 What was tried

- **Pacing, to find out whether the interference follows the rate of retention.** `--pacing 16,0`
  (no pause between change sets) is in §3.4: it does.
- **Nothing shipped for U6.** A pacing slow enough to hide the cost (the pause is 9× the work
  today; a 5× slower pass would need 49×) makes one retention pass of the benchmark last about 30
  minutes per budget, and it would trade the writer's gap for a retention backlog under load. A
  change to the engine's compaction of range deletions (picking files by tombstone density, dropping
  covered data at the first level that sees the tombstone) is the change that attacks the cause, it
  lives in `kerno-storage`, and it changes which file operations happen when, which the
  deterministic crash sweeps and the vector tests number. It is a design task with its own tests, not a tuning
  of this report's size, and it was not attempted.

### 3.4 No-pause run

`diag-U6-nopause-tmpfs.txt`, load about 25, `--pacing 16,0`, same code:

| Budget | Retention rate | Write p99 during / before | Throughput during / before | Read p99 during / before |
|---|---|---|---|---|
| 1000/10000 | 101 684 revisions/s (a pass of 16 s) | 3.56× | 0.39× | 1.55× |
| 25/100 | 70 812 revisions/s (a pass of 25 s) | 6.22× | 0.34× | 1.60× |

Against the default pacing (about 17 800 revisions/s in the diagnostic run: write p99 1.14×,
throughput 0.83×), a retention five to six times faster costs the writers 61 to 66 % of their
throughput instead of 17 %. The interference follows the rate of retention, as §3.2 predicts. If it
kept falling in proportion, a retention pass about ten times slower than today's would be needed to
bring the throughput loss under the 2 % that separates the "before" windows from each other; that
extrapolation was not measured (five rounds of passes that slow take more than an hour in this benchmark).

### 3.5 What was shipped for U6, and what it did not close

**Retention no longer re-plans a hot unit from its first revision under the write slot.**
`run_retention` plans on a snapshot (phase 1) and writes under the write slot (phase 2), where a
unit whose head moved since the snapshot used to be planned again with `plan_unit`, reading its
whole `REV_META` history inside the slot. A unit that writers update all the time is stale
whenever retention reaches it, so every change set of such units held the slot for 10–25 ms.
`replan_unit` (`crates/kerno-units/src/retention.rs`) now keeps the snapshot's plan, written
against the current head, for a unit that only grew (same `oldest_rev`, a later head, same history
policy): what it prunes is below the snapshot's head and did not change, and what the unit gained
since is left for the next pass, as for a unit written after the pass. Nothing is read under the
slot for it. Any other change (a moved policy, a moved `oldest_rev`) still plans the unit again from
its first revision. Tests: `replan_keeps_the_snapshot_plan_of_a_unit_that_only_grew` and
`replan_starts_over_when_the_old_plan_may_not_hold` (unit tests, one case per rule and policy, and
that nothing is read), `kerno_retention_replans_a_unit_updated_during_the_pause` and
`kerno_retention_replans_from_the_start_when_a_policy_moves_in_the_pause` (end to end; a one-line
mutant that drops the `policy_moved` argument turns the second red).

**A/B of that change**, same binary, alternating, budget 1000/10000, tmpfs, load 3–14
(`ab-{before,after}-U6-run{1,2,3}.txt`; "before" is the old full re-plan behind a temporary
switch, not in the tree):

| | Write p99 during / before | Throughput during / before | Retention rate |
|---|---|---|---|
| Old re-plan, 3 runs | 1.21×, 2.07×, 2.76× | 0.83×, 0.53×, 0.70× | 15.8k, 10.3k, 22.3k revisions/s |
| New re-plan, 3 runs | 1.02×, 0.95×, 1.17× | 0.88×, 0.90×, 0.76× | 24.9k, 26.8k, 23.2k revisions/s |

**Final runs of the shipped code** (`final-U6-run{1,2,3}.txt`, budget 1000/10000, tmpfs, load 4–12,
`final.sh`):

| Run | Write p99 during / before | Throughput during / before | Read p99 | Retention rate |
|---|---|---|---|---|
| 1 | 1.93× | 0.79× | 1.46× | 35.8k revisions/s |
| 2 | 1.04× | 0.89× | 0.79× | 49.0k revisions/s |
| 3 | 1.81× | 0.80× | 1.01× | 59.7k revisions/s |
| 4, `--pacing 16,29` (three times slower in the formula) | 1.20× | 0.90× | 1.17× | 27.6k revisions/s |

**Every run still fails both conditions (throughput 0.76–0.90×; write p99 above 1× in the four final runs, 1.04–1.93×).**
The condition (throughput ≥ the lower "before" window) leaves a margin of about 2–10 % between
the two "before" windows alone, and the best run is 10 % below it.

**What the control says.** With `--control` (`control-after-U6-run1.txt`,
`baseline-control-U6-run1.txt`) the same writers run for as long as the retention window with no
retention: 23 702 ops/s against 24 120 ops/s before (0.98×) and 30 687 against 30 764 (1.00×), while
the retention windows ran at 0.86× and 0.85×. So the length of the window costs nothing, and the
gap is what retention makes the engine do, as §3.2 measured: the compaction of the range deletions
and heads it writes. This change removes the part of that cost that was in the units layer (reads
under the slot); the part in the engine is what is left.

**What was tried and not kept.** A first-byte bound on the memtable's range tombstones and on the
fragment check of a point read (`kerno-storage`), to take the tombstone lock off the read path of
keys no tombstone can cover, was written and removed: it was not measured separately from the
re-plan change (the run-to-run spread of U6, 10–20 %, hides anything it could give), and it
touched the read path of the engine with nothing to show for it. Nor did a slower pacing close it
(run 4 above: 0.90×, the same as the others).

### 3.6 Recommendation

Keep K-E3-9 open. The units-layer cost is removed (the A/B shows retention 1.1–2.6× faster at the
same pacing, and none of the seven runs made with it falls to the 0.53× throughput of the old code; the machine's load
varied between runs, so the A/B is a direction, not a precise factor), and the remaining gap is the
engine's: compaction cost per write during retention. The next step is in `kerno-storage`: pick
files for compaction by tombstone density and drop covered data at the first level that sees the
tombstone. It changes which file operations happen when, which the deterministic crash sweeps number,
so it is its own task with its own tests. Until then, a workload that cannot afford the
interference can raise `UnitConfig::retention_pacing.pause_ratio` or run `run_retention` in its
quiet hours. The threshold was not touched.

### 3.7 Re-measured on a real disk (2026-11-19), and why retention is not moved off the writer's path

The roadmap item was re-attempted with the idea of taking retention off the writers' hot path, in
a background task or after the commit. It already is off it: in U6 `run_retention` runs on its own
thread while the writers keep going. §3.7.1 measures how long it holds the write slot, and the
answer is under 1 % of the time, so moving the work somewhere else would not change the result; no
such change was made. The comparison in U6 is writers during retention against writers before it;
it is not a comparison with SQLite (that one is [sqlite-benchmark](sqlite-benchmark.md)).

What was run: the unchanged `unit_bench --only U6` (full mode, seed `0xE309`, both budgets),
three runs of the code before the replan change and three with it, on the btrfs disk
(`--dir target/bench-scratch`, so `fsync` is real and the writers are bound by it: about 600-700
writes/s; load 3-5). Raw outputs:
[`2026-11-19-k-e3-9-disk/`](../../spikes/storage/results/2026-11-19-k-e3-9-disk/).

| Code | Budget | Write p99 during / before | Throughput during / before | Read p99 during / before | Verdicts passing |
|---|---|---|---|---|---|
| Before the replan change | 1000/10000 | 1.14x, 0.98x, 0.87x | 0.89x, 1.08x, 1.08x | 0.98x, 0.86x, 1.01x | 1 of 3 |
| Before the replan change | 25/100 | 0.58x, 0.91x, 1.10x | 1.11x, 0.96x, 0.99x | 1.07x, 1.23x, 1.06x | 0 of 3 |
| With the replan change | 1000/10000 | 1.00x, 0.79x, 0.92x | 0.95x, 0.95x, 1.05x | 0.98x, 1.09x, 0.95x | 1 of 3 |
| With the replan change | 25/100 | 0.77x, 0.70x, 1.14x | 1.02x, 0.98x, 0.97x | 1.30x, 1.03x, 1.16x | 0 of 3 |

What this shows, and what it does not:

- **On a disk the gap is small and the benchmark is mostly noise.** The two "before" windows of
  one run differ from each other by 5-30 % (for example 540 and 695 writes/s in one run), more
  than the effect being measured, and the verdict compares "during" with the worse of them on three
  metrics at once, one of which is a read of 6 microseconds. Of 12 verdicts, 2 pass; the failures
  are the read p99 (1.03-1.30x), the throughput (0.89-0.99x) and, in two runs, the write p99 (1.10x,
  1.14x), by margins of the same size as the spread between the "before" windows.
- **The replan change does not show up in these runs** (1 passing verdict of 6 before it, 1 of 6
  with it; retention 8.2-9.6k against 8.8-10.2k revisions/s). Its value was measured in §3.5 on
  tmpfs, where it was CPU-bound; it stays because it removes a read of the whole history under the
  write slot, not because this table moves.
- **The target is not met, so the roadmap item stays unticked.** A tick would need the verdict
  to pass in every run, and with this spread it would also need the criterion to be stated on
  medians or on more rounds; that is a decision about the benchmark, not a change to make here.
  The criterion was not touched.
- Not measured: the tmpfs runs of §3.5 were not repeated with this build, and `--indexes` and
  `--relations` were not run.

#### 3.7.1 Where the disk gap comes from

Two more runs on the same disk with `--budget 1000,10000 --diag --control`
(`disk-diag-control-run{1,2}.txt`), and one with a temporary timer around each retention change
set (`disk-slot-hold-per-change-set.txt`, the timer is not in the tree). Five rounds each.

| | Run 1 | Run 2 |
|---|---|---|
| "Before" windows, 2 x 15 s: writes/s | 717 and 746 | 773 and 733 |
| Control, no retention, as long as the retention windows (126 s, 124 s): writes/s | 686 | 703 |
| During retention: writes/s | 652 | 678 |
| Control / the lower "before" (what the window length alone costs) | 0.96x | 0.96x |
| During / control (what retention costs) | 0.95x | 0.96x |
| During / the lower "before" (the U6 verdict) | 0.91x | 0.92x |
| Time retention held the write slot, per run of 5 passes | 1.1 s of 127 s (0.9 %) | not timed |
| Time writers and retention waited for the gate: during / control | 18.9 s / 4.8 s | 18.7 s / 4.8 s |

- **Retention does not hold the writers up.** A change set of 16 units takes the slot for a median
  of 0.31 ms (building plus commit), p99 0.77 ms, longest 2.2 ms; the 3 185 of them in the timed
  run of five passes held it for 1.1 s in 127. Retention itself waited 10.6 s for the slot in that run, which is
  most of the extra gate wait above; what is left for the writers is about 3 s in 125 (2-3 %).
- **About half of the gap is not retention.** A window of the same length with no retention
  at all already runs at 0.96x of the short "before" windows: over 25 s the writers' own 1 KiB
  updates fill the write buffer and cause flushes and compactions (19.6 MB flushed and 21-31 MB
  compacted in the control, 0-1 flushes and no compaction in a 3 s window). So even a retention
  that cost nothing would fail this verdict about half the time on this disk: the criterion
  compares a long window with short ones.
- **The other half is the disk.** Per pass, retention adds about 7 MB of flush (26-27 MB against 19.6 MB in the
  control) and 65-70 MB of compaction (87 and 93 MB on average, 29-134 MB per pass, against about
  20-30 MB in the control). It writes the same device the writers `fsync` to, and costs them 4-5 %
  of their throughput. That compaction is the engine's work on the range
  deletions (§3.6); the units layer cannot take it away, only spread it with `pause_ratio`.
- **What would close it:** an engine change to compaction of range deletions, and a U6 whose
  "before" windows are as long as the window they are compared with (the control above is the
  right reference). Neither was made: the first is its own task, and the second changes the
  criterion, which is for the owner to decide.

### 3.8 Option (a): U6 judged on medians against the control window (row OPS-08)

Written **before any run of `--medians`**; the runs and the verdict follow in §3.8.2 and nothing
below was changed after them. The owner approved option (a) of
[`COMPLETION-PLAN.md`](../COMPLETION-PLAN.md) ("U6: the open decision"). Option (b), an engine
change to compaction, is not part of this work.

#### 3.8.1 Pre-registration

`unit_bench --only U6 --medians` (new flag; `--medians` implies `--control`). The existing verdict
(during against the worse of the two short "before" windows, rounds pooled) is printed unchanged,
with its thresholds. The new verdict `U6m` is printed beside it.

- **Statistic.** In each round, the same writers and reader run in four windows: two "before"
  windows of 3 s, the retention window (from the first `run_retention` call to the call that
  reports `complete`), and a control window of the same length with no retention. For each round
  take during / control for **write p99**, **throughput** and **read p99**. The statistic is the
  **median over the rounds** of each ratio.
- **Number of runs.** 7 rounds per budget in one invocation (`MEDIAN_ROUNDS`), both budgets
  (1000 units / 10 000 ops and 25 / 100), seed `0xE309`, full mode, default pacing, no
  `--relations`, no `--indexes`. **Disk: 3 invocations** (so 3 medians per budget). **tmpfs: 1
  invocation**, as a reference only: it is CPU-bound and does not count towards the verdict.
- **Medium.** The disk is the btrfs volume of this checkout (`--dir target/bench-scratch`), where
  `fsync` is real. The tmpfs reference is `/tmp` (the example's default `--dir`).
- **Noise tolerance.** In each round, the spread between the two "before" windows is
  `|a - b| / max(a, b)` for throughput, write p99 and read p99. Its **median over the rounds** is
  the noise of that metric in that invocation. The tolerance is that noise, with a floor of
  **2 %** for throughput (the gap between two "before" windows that §3.4 measured) and **10 %**
  for each p99 (the plan's fixed value). The "before" windows are 3 s, shorter than the windows
  they bound, so this noise estimate is, if anything, too generous.
- **Rule for one invocation and one budget.** `U6m` fails when the median throughput ratio is
  below `1 - tolerance`, or the median write p99 ratio or the median read p99 ratio is above
  `1 + tolerance`. Beside it the bench prints the plan's **fixed reading** (throughput median
  < 0.98, or write p99 median > 1.10; no read p99), for the case where the noise is large enough
  to hide a real 5 %.
- **A real gap** after (a) is declared when `U6m` fails in at least 2 of the 3 disk invocations
  for either budget. If `U6m` passes but the fixed reading fails in 2 of 3, the report says the
  result is inside the measured noise and does not call it closed. In both cases the work stops
  there: compaction is not changed (option (b) needs the owner).
- **The machine is not idle.** Other agents compile and run on it during the runs. Each raw
  file starts and ends with `/proc/loadavg`. A run that is hit by that load is **not repeated
  until it passes**: every invocation made is reported.

## 4. What this report does not show

- U6 on a real disk, outside §3.7. The runs of §1 to §3.6 are on tmpfs; §3.7 has the disk runs. The Stage 5 numbers on a disk had a smaller gap
  (writer p99 1.06–1.18×, throughput 0.93–1.01×) because the writers were bound by `fsync`, not by
  CPU. The gap on a disk with the new `history` is not re-measured: `history` does not run in U6.
- A quiet machine. §1.
- `state_as_of` cost: T1 measures `unit_as_of` only (row OPS-07).
- T1 with data larger than the block cache: all of T1's data fits in the 64 MiB cache (§5.2).
- U6 with indexes and relations (`--indexes --relations`), re-measured in Stage 5: not repeated.

## 5. Space, as-of cost and recovery time (rows OPS-05, OPS-07, OPS-10)

Measured on 2026-11-19 at commit `21c4f5a`, full mode, seed `0xE309`, on the btrfs disk
(`--dir target/bench-scratch`), with:

`cargo run --release -p kerno-units --features test-support --example unit_bench -- --only U5,T1 --dir target/bench-scratch`

Three runs, raw output (each starts and ends with `cat /proc/loadavg`):
[run1](raw/2026-11-19-ops-05-07/run1.txt), [run2](raw/2026-11-19-ops-05-07/run2.txt),
[run3](raw/2026-11-19-ops-05-07/run3.txt).

### 5.1 Space per revision (U5, row OPS-05)

100 units x 100 revisions = 10 000 revisions per payload size, default options (no WAL
preallocation), after a full `compact_range`. "On disk" is the sum of the file sizes in the
directory after `close`, minus the same sum for an empty database (178 B), divided by the
revisions. They are apparent file sizes, not allocated blocks. Byte counts do not depend on load:
the three runs gave the same figures (the only difference is 2 B in the plain 16 KiB directory
of run 1, out of about 164.8 MB).

| Payload | Revisions | Live bytes per revision, units / plain | On disk after close per revision, units / plain | Units on disk minus payload |
|---|---|---|---|---|
| 100 B | 10 000 | 315.3 / 127.4 | 315.3 / 127.4 | 215.3 B |
| 1 KiB | 10 000 | 1257.7 / 1068.0 | 1257.7 / 1068.0 | 233.7 B |
| 16 KiB | 10 000 | 16680.3 / 16483.8 | 16680.4 / 16483.9 | 296.4 B |

The 1 KiB figures equal the 2026-11-14 idle log (1257.7 and 1068.0,
[unit-bench.log](raw/2026-11-14-idle/unit-bench.log)). The on-disk figure matches the live one to
0.1 B, so after a full compaction and `close` nothing else in the directory adds to it.

### 5.2 Cost of an as-of read (T1, row OPS-07)

10 units with 1 KiB payloads per depth; each read picks a unit and a revision below the head, so
it takes the binary search over `REV_META`, and checks that it got that revision. 25 000 reads per
depth and run, one thread, warm data. The machine was shared: the 1-minute load average at the start
and end of each run was 4.99 / 4.46 (run 1), 4.46 / 5.95 (run 2) and 4.99 / 6.19 (run 3), so these
latencies are provisional.

| Depth | Run 1 p50 / p99 (ms) | Run 2 p50 / p99 (ms) | Run 3 p50 / p99 (ms) |
|---|---|---|---|
| 10 | 0.0091 / 0.0231 | 0.0091 / 0.0141 | 0.0090 / 0.0131 |
| 100 | 0.0129 / 0.0186 | 0.0130 / 0.0181 | 0.0129 / 0.0180 |
| 1000 | 0.0186 / 0.0256 | 0.0189 / 0.0282 | 0.0184 / 0.0269 |

The whole data set fits in the 64 MiB block cache, so T1 measures CPU and cache cost, not device
reads. For comparison, U3 at depth 1000 on the 2026-11-14 idle run had p50 0.0112 ms and p99
0.0154 ms, lower than these loaded runs. The OPS-01 idle-machine run re-measures these latencies.
`state_as_of` is not measured here.
