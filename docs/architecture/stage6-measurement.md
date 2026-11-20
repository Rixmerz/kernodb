# Stage 6 · E6-04 — Measurement and sentinels

Contract of E6-04 ([Stage 6 minutes](stage6-record.md) §3, thresholds of §6, decisions 2, 4 and 5 of §8). Background: the baseline measurement and the method, in [E6-00](stage6-e600-remeasurement.md); the use of the cache in the planner, in [E6-02](stage6-integration.md); reversibility, in [E6-03](stage6-reversibility.md). Guarantees: [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) G7 and [ADR-0015](../adr/0015-privacy-of-observability.md) (I-37).

E6-04 did not change production code, the persistent format or the engine; **E6-04b** (§3b) does touch `cache.rs` and `planner.rs` (only the decision to store and the footprint), without touching the persistent format or the engine. It adds a section to the existing bench (`crates/kerno-units/examples/index_bench.rs`, option `--cache`), the tests `crates/kerno-units/tests/cache_sentinels.rs`, the raw output and this document.

## Result in one line

**E6-04b (final verdict, §8): the worst case without repetitions meets the threshold (real overhead ≈ 0%, measured paired) and the p50 of W1 meets it (−97.9%); the p99 of W1 sits at the edge of the threshold and cannot be certified: +0.5%, +7.8%, +4.7% and +7.5% in four runs of the same code, with a measured real overhead of +3.4% to +4.9% on the read that is not served. The joint criterion of §6 is not met stably: the cache stays disabled by default (minutes §8.2).** The text that follows in §3 and §7 is that of the first attempt of E6-04b (one run of 5 rounds) and is kept as history; where it contradicts §8, §8 prevails. The original measurement of E6-04 is kept in §3 as "before".

## 1. Method

| Field | Value |
|---|---|
| Date | 2026-11-09 (E6-04); 2026-11-11 (E6-04b, current output) |
| Command | `cargo run -q --release -p kerno-units --all-features --example index_bench -- --cache > docs/architecture/stage6-measurement-output.txt` |
| Machine | 4 CPU, no other load: load average 0.42 at the start of the full measurement and 1.14 at the end (`uptime`; the run itself is single-threaded); nothing else was launched during the run (the mutations of §4 were done afterwards) |
| Engine | `KernoDb` on disk (`TempDir` of the bench), 100 unit types, `PAYLOAD` and the data generator of E6-00; seed 42 |
| Scales | 20,000 and 100,000 units (those of E6-00) |
| Measure | latency of each `snapshot.query(filter)` (`Instant`), in ms. Writes and index rebuilds are **not** timed |
| Warm-up | the first 10% of the reads of each run do not count |
| Rounds | 5 per comparison; in each round the sequence is run **once with the cache off and once with it on**, and the order is inverted each round (off→on, on→off, …) to spread thermal and page-cache drift |
| Statistics | p50, p95 and p99 per run. The comparison is given in two ways: **best round** (the method of E6-00: the minimum of each mode) and **median of the rounds**. Both are reported because the p99 is noisy between rounds and a single reading would be choosing |
| Controls | each read compares its ids with the expected result (if cache and reference disagree, the run stops); with the cache off, `stats()` must stay at zero and no answer may come from it; in the load without repetitions, there cannot be more reads served than the recurrences of the single entry that is kept (§3b, point 3) |

### The loads

- **W1** (minutes §8, decision 5): blocks of **one write and four reads of the same query**. 20% of the operations are interleaved writes and 80% reads. The write updates an existing unit (it does not change the size of the sets) and moves `commit_seq`, so the first read after each write **can never** be served (the entry depends exactly on `commit_seq`, G4): of the four reads, three repeat and one does not. Thus 75% of the reads (60% of the operations) are servable repetitions; this is the maximum that one write every five operations allows, not a chosen value. The query of each block is `type = ?` with a different type from the previous one (37 is prime with 100).
  - `w1-index-ready`: after each write the index is rebuilt (not timed), so that all reads can use it, as in the base of E6-00. It is the load that is judged. 400 blocks → 1,440 timed reads per run.
  - `w1-index-stale`: the index is left `Stale` after the first write, so the reads without cache are the reference path (≈ 90 ms at 20,000 units). It is a **complementary** load (the cache wins much more because what it saves is a full walk); it is not used to decide. 150 blocks → 540 reads per run.
  - W1 runs only at 20,000 units: one index rebuild per block at 100,000 (≈ 0.5 s) would multiply the run without adding another decision scale. **It is a limit of the measurement**: W1 at 100,000 has not been measured.
- **Without repetitions** (worst case, §6): one read per block, no writes, walking the 100 types with step 37. The cache keeps one entry per query shape (`type = ?`), so each read falls on the entry of **another** type: it never hits, it replaces it (`Missed{OtherQuery}` and `Deleted{Replaced}`) and pays the whole cache path: footprint, `lookup`, and storing the rows. 4,000 reads at 20,000 units (3,600 timed per run), 1,500 at 100,000 (1,350).

## 2. Raw output

- [`stage6-measurement-output.txt`](stage6-measurement-output.txt): the full output of the command above, unedited, with the line `exit=0` and the final `uptime` (the last two lines are added by the command that saves it). One row per run: `cache,scale,load,mode,round,timed_reads,served_by_the_cache,p50_ms,p95_ms,p99_ms`; the `#` lines are the comparisons.
- Quick reproduction (smoke, 2,000 units): `cargo run -q --release -p kerno-units --all-features --example index_bench -- --cache --quick`.

## 3. Result against the thresholds (§6)

Percentages of the cache on against off (negative = faster). "Best" is the best round of each mode; "median", the median round.

| Threshold (minutes §6) | Load and scale | Best round (before → now) | Median (before → now) | Status now |
|---|---|---|---|---|
| W1, p50 ≥ 20% better | `w1-index-ready`, 20,000 | −97.7% → −98.0% | −97.6% → −97.9% | **meets** |
| W1, p99 ≤ +5% | `w1-index-ready`, 20,000 | +2.0% → +4.7% | +7.8% → +1.3% | **meets** (best round and median) |
| Worst case, p99 ≤ +5% | without repetitions, 20,000 | +20.9% → +9.7% | +11.7% → −2.4% | **meets on the median; not on the best round** |
| Worst case, p99 ≤ +5% | without repetitions, 100,000 | −8.5% → −7.0% | +3.2% → −2.8% | **meets** |
| (complementary) W1 p50 | `w1-index-stale`, 20,000 | −100.0% → −100.0% | −100.0% → −100.0% | meets |
| (complementary) W1 p99 | `w1-index-stale`, 20,000 | −11.9% → −6.7% | −13.7% → −5.8% | meets |
| Writes: no measurable change in U1 | — | — | — | not measured here: the cache does not write to the engine; E6-03 checks it (identical bytes and `commit_seq`) |

The "before" values are those of E6-04 (run of 2026-11-09); the "now" values are those of `stage6-measurement-output.txt`, run of E6-04b. They are different runs of the same machine: the difference between them includes the dispersion between runs (§7).

Absolute values of E6-04 (**before** E6-04b; the current ones are in `stage6-measurement-output.txt`):

| Load | Mode | p50 (range of rounds) | p99 (range of rounds) |
|---|---|---|---|
| `w1-index-ready` 20,000 | off | 1.212 – 1.390 | 4.668 – 5.534 |
| `w1-index-ready` 20,000 | on | 0.028 – 0.032 | 4.760 – 5.716 |
| without repetitions 20,000 | off | 0.857 – 0.913 | 1.018 – 1.809 |
| without repetitions 20,000 | on | 0.936 – 0.974 | 1.231 – 1.705 |
| without repetitions 100,000 | off | 6.490 – 6.922 | 8.591 – 9.673 |
| without repetitions 100,000 | on | 6.814 – 7.077 | 7.864 – 10.215 |

### Honest reading (of E6-04, before E6-04b)

- **p50 of W1: the improvement is the expected one and is not fragile.** A read served from the cache costs ≈ 0.03 ms against ≈ 1.3 ms. 75% of the reads are served; the p50 falls in the served block.
- **p99 of W1: it cannot be claimed as met.** The p99 of W1 is set by the read that is **not** served (the first after each write, 25% of the reads), which with the cache on does the same as without it **plus** the cost of storing the result. The rounds overlap: off 4.67–5.53 ms, on 4.76–5.72 ms. With the method of E6-00 (best round) it gives +2.0%, within; with the median it gives +7.8%, outside. The difference between the two readings is of the order of the dispersion between rounds, so the measurement does not distinguish "within 5%" from "outside"; more rounds would bound it, but that is not an adjustment that belongs here.
- **Worst case: fixed overhead per read of ≈ 0.07 ms** (p50 +9.2% at 20,000 units). It is the cost of computing the footprint, consulting the cache and storing the rows, which shows as a high percentage when the base read costs ≈ 0.9 ms (20,000 units) and is diluted when it costs ≈ 7 ms (100,000 units: p50 +3.3% to +5.0%, p99 within). The p99 is noisy between rounds (at 100,000, from 7.86 to 10.22 ms with the cache on), but at 20,000 the p50 overhead is consistent (off 0.857–0.913 ms against on 0.936–0.974 ms: the ranges do not overlap). The p99 does overlap (1.02–1.81 against 1.23–1.71 ms): the +20.9% of the best round depends on an unusually low off round (1.018 ms), but the median also fails (+11.7%), so the two ways of aggregating give "does not meet".
- **What this measurement does not say.** There is no W1 at 100,000 units, the proportion of repetitions and the size of the cache are not varied, and the load without repetitions uses a single pattern (every read replaces the previous entry). A pattern without replacement (more distinct query shapes than the single type) could cost less or more; it has not been measured.

### Consequence for decision 2 of the minutes (E6-04, before E6-04b)

The criterion of §6 is joint ("net improvement": p50 and p99 of W1 **and** the worst case). With these numbers it is **not met** in a way that can be claimed without discussion: the cache stays **disabled by default** (minutes §8.2), and the subsequent decision —accepting the result with the cache disabled by default, optimising the failure path (for example, not storing until a second read, which would require a counter of its own without reading the observer, G3) or repeating with another load— belongs to the orchestrator. E6-04 stops here: the cache code and the thresholds have not been touched.

## 3b. E6-04b: what was changed in the failure path

Measured with a temporary profile (now removed) in the planner: the cache's own work on a miss is ≈ 2–3 µs per read (footprint ≈ 1.3 µs, `lookup` ≈ 0.5–0.7 µs, decision to store ≈ 0.2–0.9 µs) against ≈ 850 µs of the read. The cost that weighed most was that of **storing** the result of each query that does not repeat (copying the rows and evicting the previous entry), and the dispersion between rounds is much larger than the rest. Changes:

1. **Admission** (`ResultCache::worth_storing`, `cache.rs`): one entry fits per query shape. A newcomer with other literals replaces an entry **that has served nothing** only on its **second appearance** (the first is rejected and remembered in memory, one query per shape). The first query of a shape is stored at once, the same query as the entry, a newcomer against an entry that has already served, and one against an entry whose dependencies no longer match. It does not read the observer (G3), leaves no record and does not change what is served: it only decides whether to store (G1–G7 intact). The rejected state is deleted with `clear`, `set_key` and when the cache is switched off. Tests: 4 unit tests in `cache.rs` and `tests/cache_admission.rs`.
2. **Footprint memoised per shape** (`fingerprint_of`): the footprint depends only on the shape (which conditions are set and the direction), so it is computed once per shape and key and reused, instead of building the shape text and doing BLAKE3 on each read. It is discarded when the key changes. Test: `the_remembered_fingerprint_of_a_shape_is_the_one_computed_from_its_text` compares each of the 8 shapes with the footprint computed from its text (it goes red if the numbering of the slots is altered).
3. **Control of the load without repetitions** (`index_bench.rs`): it used to require 0 reads served. With admission, the entry of the first query is kept and serves the read in which its type comes back (one in 100: 36 of 3,600, 13 of 1,350). The control now requires that there be no more served than those recurrences (`⌈reads/100⌉ + 1`). It is not a threshold: it protects that the load remains the worst case.

## 4. Sentinels (I-37)

`crates/kerno-units/tests/cache_sentinels.rs`, 4 tests (2 × two engines: `Emulated<LogEngine>` and `KernoDb`). `cache.rs` (E6-01) already plants the sentinels of `sentinels.rs` and walks the store; these tests cover what that file leaves open. Test count of `kerno-units`: 305 → 309.

| Test | What it asserts |
|---|---|
| `no_literal_in_anything_the_cache_path_reports` | With the cache on, **at each observation level** (`Off`, `Aggregate`, `Fingerprint`, `Diagnostic`), it passes through the real planner unit queries (type, schema version, both) and relation queries, read twice (miss and hit), a forged entry that does not pass the checks (discrepancy), the per-query switch, a write with sentinels that invalidates by sequence, and the **errors** of the store (entry refused of each class, activating an unknown footprint, entry larger than `max_bytes`, invalid limit, cache off). Then it repeats everything over a unit with the head damaged, so that the read ends in error. It concatenates everything that is reported —`source`, `path` and `fallback` of each read, the plan explanation, the cache's records and counters, the observer's, each error with `Display` and `Debug`— and finds no sentinel (type, payload, metadata, relation type, provenance source and origin, schema version) in text, decimal list or hexadecimal. **Control:** the answers delivered to the caller do contain the sentinels, so the search could find them. In addition, every reported footprint is that of the same shape with other literals (a footprint that mixed in a literal would give itself away). |
| `every_g7_transition_leaves_its_record` | One case for **each leaf** of `Transition` (27: each cause of failure and of miss, each dependency, each reason for deletion, each switch), each on a new store. It requires that the case leave its record with `optimization_version` and `planner_version`, with the footprint of the query (or none in the global switch and in `clear`), with the `commit_seq` of the entry when the record concerns one, and without literals in the record, the counters or the store's errors. `label` has no wildcard: a new variant does not compile until it is named there and in `ALL_LABELS`, and the test requires that the walk has exactly those cases. |

### Mutations tested

Each mutation was applied to `crates/kerno-units/src/cache.rs`, `cargo test -p kerno-units --all-features --release --test cache_sentinels` was run and the file was restored (`git checkout`).

| ID | Mutation | Result |
|---|---|---|
| S-M1 | **Copy a literal into the optimisation's record:** after recording `Used`, write the value of the `schema_version` filter into the `commit_seq` of the record (`r.commit_seq = Some(CommitSeq(u64::from(v)))`) | **Die** `emulated_log::no_literal_in_anything_the_cache_path_reports` and `kerno_db::no_literal_in_anything_the_cache_path_reports`: `schema version sentinel "4071337791" leaked in the output at level Off`. The two transition tests stay green (the case's filter carries no version): the leak is seen by the sentinel, not by the walk |
| S-M2 | Do not leave the `Used` record on a hit (`st.record(..., Transition::Used, ...)` replaced by an expression with no effect) | **All four die**: `Used: no record in […]` in the transition ones and `level Off: no Used record in {…}` in the literal ones |

All the tests go green again when the file is restored (`4 passed`).

## 5. Status of the criteria of the minutes (§4)

| Criterion | Evidence in E6-04 |
|---|---|
| Net improvement measured against the disabled mode, with thresholds fixed beforehand | measured and published (§3); p50 of W1 meets it; the p99s do **not** meet it stably: **not concluded in favour**, decision of the orchestrator |
| Traceability without literals (G7, I-37) | the two tests of §4 and mutations S-M1, S-M2 |

The update of the table of the minutes (§4) corresponds to E6-05, as with the earlier packages.

## 6. What E6-04 does not cover

- W1 at 100,000 units, other proportions of repetition, the influence of `max_entries`/`max_bytes` on the worst case, and the worst-case pattern with more distinct query shapes.
- U1 (writes): not measured; the argument is structural (the cache has no engine handle) and E6-03 checks it by bytes and `commit_seq`.
- Concurrency between reads that share the cache: the measurement is single-threaded.
- One cause of the worst-case overhead measured separately (footprint, `lookup`, storing): it is attributed by reasoning, not by profiling.

## 7. Verification of E6-04b and limits

- Command: `cargo run --release -p kerno-units --example index_bench -- --cache` (5 rounds, output in `stage6-measurement-output.txt`, `exit=0`). Machine of 4 CPU, load ≈ 1 during the run.
- **The measurement of 5 rounds is noisy at the edge of the threshold.** With the same production logic, another run of 5 rounds gave, at 20,000 without repetitions, median p99 +12.1% and +9.2% of p50 before memoising the footprint; one of 15 rounds (only that load, temporary edit of the bench, already reverted) gave median p99 +6.9% and p50 +3.4%. An interleaved comparison query by query (cache off/on in blocks of 100) measured between +1% and +3% on average (9–25 µs over ≈ 850 µs); the off-against-off control gave −0.7%. That is: **the real overhead of the worst case at 20,000 units is of the order of 1–3%, below 5%, but the 5-round p99 reading can land on either side**. That is why the verdict of this document is that of the median, which is the requested criterion, and the best-round verdict is also published, which still fails at 20,000 (+9.7%).
- The rest of the observed overhead (≈ 10–25 µs) is not explained by the cache's own work (≈ 3 µs): the section of the planner that does not touch the cache was also ≈ 15–25 µs slower with the cache on, even without storing anything. It is attributed to CPU-cache and allocator effects, **without a profile that demonstrates it**.
- W1 at 100,000 units and concurrency have still not been measured (see §6).
- Verification: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test -p kerno-units --all-features --release` (result in the report of the package). The thresholds and the existing `tests/cache*.rs` tests were not modified.


## 8. E6-04b: final verdict (four runs, 2026-11-11)

The independent verification of the first attempt repeated the command and obtained FAIL where the committed output said PASS. The measurement was repeated on the machine without other load, with the same production code, twice with the 5-round bench and twice with the 20-round short bench (see below). p99 of the cache on against off, median of rounds:

| Load | 5 rounds, run 1 | 5 rounds, run 2 | 20 rounds, run A | 20 rounds, run B |
|---|---|---|---|---|
| without repetitions, 20,000 (≤ +5%) | +4.0% | −3.8% | +2.9% | +5.3% |
| without repetitions, 100,000 (≤ +5%) | +5.3% | +2.6% | −4.5% | +1.4% |
| W1 `w1-index-ready`, p99 (≤ +5%) | +0.5% | +7.8% | +4.7% | +7.5% |
| W1 `w1-index-ready`, p50 (≤ −20%) | −97.9% | −97.8% | −97.8% | −97.9% |

Each run fails a different threshold with the same binary: **the p99 by rounds does not resolve a 5% on this machine**, neither with 5 rounds nor with 20. Runs A and B are the current raw output (`stage6-measurement-output.txt`); the 5-round ones are kept only in this table.

**Change of the bench (not of the thresholds):** `CACHE_ROUNDS` goes from 5 to 20 and each round is four times shorter (same total work: 1,000 reads without repetition and 100 blocks of W1 at 20,000; 400 reads at 100,000), so that off and on are compared over seconds of difference and not minutes. An informative line with the percentiles of all rounds together is added. The verdict is still the median of rounds against the thresholds of §6.

**Paired measurement** (temporary probe, now removed: the cache is switched on and off every 10 reads or blocks over the same snapshot, and means are compared, which are stable):

| What | Off | On | Difference |
|---|---|---|---|
| Read without repetition, 20,000 (mean, four repetitions) | 819 – 1,213 µs | 823 – 1,216 µs | −0.6% to +0.5% |
| First read after a write in W1 (mean, four repetitions) | 3,588 – 3,667 µs | 3,720 – 3,834 µs | **+3.4% to +4.9%** (≈ 125 – 180 µs) |

Of those ≈ 125 – 180 µs, copying the rows is ≈ 33 µs and storing them (including freeing the previous entry) another ≈ 33 µs; `worth_storing` ≈ 1 µs. The rest is not attributed.

**Verdict by criterion:**

- **S1 (worst case ≤ 5%): meets.** The real overhead is indistinguishable from zero (−0.6% to +0.5% on average); the p99 readings above +5% (one in four at each scale) are dispersion, not cost.
- **S2, p50 of W1 ≥ 20% better: meets** (−97.8%), without fragility.
- **S2, p99 of W1 ≤ +5%: not certifiable.** The real cost of the read that is not served (the one that sets the p99) is +3.4% to +4.9%: below the threshold, but so close that the dispersion takes it above half of the time. Reducing it requires not copying the result when storing it (sharing it between the answer and the entry), which changes the public type `CachedResult`/`QueryOutcome` and the existing tests; it is outside this package.
- **S3: meets.** The existing `tests/cache*.rs` tests pass without changes, the thresholds were not touched and the cache stays disabled by default.

**Consequence (minutes §8.2):** the net improvement of §6 is joint and the p99 of W1 cannot be claimed; the result cache stays **disabled by default** and available as an option. Admission and the memoised footprint are kept: they remove the overhead of the worst case (before +11.7% median at 20,000) without touching any guarantee.
