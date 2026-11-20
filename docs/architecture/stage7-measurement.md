# Stage 7 · E7-05 — Measurement of associative memory

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Measured.** Quality: §6.2 **not met** (threshold T4 is unattainable as written, and the ranking falls below the simple order). Performance: see §4 |
| Minutes | [Stage 7](stage7-record.md) §6.2 (quality), §6.3 (performance), §7 point 3 (default ranking), row E7-05 |
| Set | `crates/kerno-testkit/data/assoc-eval/` (`MANIFEST` verified before measuring) · [evaluation set](stage7-evaluation-set.md) |
| Code | `crates/kerno-units/tests/assoc_measure.rs` (quality; runs with `cargo test`) · `crates/kerno-units/examples/assoc_bench.rs` (performance; does not run with `cargo test`) · `crates/kerno-units/tests/harness/assoc.rs` (query translation, shared with E7-02 and E7-03) |
| Raw output | [stage7-measurement-output.txt](stage7-measurement-output.txt) |

This document changes no threshold, no expected answer and no ranking weight. What is stated here comes from the raw output; what is not stated is in §5.

## One-line result

**The ranking does not become the default order.** With the default weights, the ranked query meets recall@10 (0.939), nDCG@10 (0.912), at least 10 % above B1, and B1 in the four classes, but it stays below the simple order "hops, then identifier" (nDCG@10 0.942). The threshold of 10 % above the simple order (T4) requires 1.036, and nDCG cannot exceed 1: it cannot be met. The default order remains "hops, then identifier".

## 1. Method

### 1.1 Translation rule (fixed before looking at results)

Each query in the file `queries.txt` is translated into an `AssocQuery` with a single rule, the same for every query of every class. The rule is that of `corpus_query` in `tests/harness/assoc.rs`, written in E7-02 and used by the E7-02 and E7-03 tests; this measurement reuses it without changes. It uses only what the query file gives:

| Field of `queries.txt` | Field of `AssocQuery` |
|---|---|
| `seed` (unit index) | `seeds = [id of that unit]` |
| `direction` (`out`, `in`, `both`) | `direction` = `Out`, `In` or `Both` |
| `types` (list) | `relation_types` = the set of those types |
| `depth` | `max_depth` |
| `window` (`lo-hi` or `-`) | `window` = `SeqWindow { lo, hi }`, or none. The steps are `commit_seq`: the loader checks that each change set is committed with its step as sequence |
| `classes` (list or `-`) | `provenance.kinds`, or none |
| `actor` (name or `-`) | `provenance.actor`, or none |
| `source` (source or `-`) | `provenance.source`, or none |
| `type` of the seed | not part of `AssocQuery`; used only by B0 |
| `id`, `class`, `E` answers | not part of the query: they are the evaluation |
| *(not in the file)* `limit` | `MAX_ASSOC_LIMIT` = 1 000, for all |
| *(not in the file)* `visit_budget` | `MAX_ASSOC_VISITS` = 100 000, for all |
| *(cut)* | 10, in the metrics, over each arm's order |

The `limit` is set to the maximum, and not to the cut of 10, for one reason: the ranking only reorders the results that the query returns (`rank_assoc`). With a limit of 10, the ranking could only reorder the first ten of the simple order, and could not raise to the top anything the traversal did not return. The cut of 10 is applied afterwards, in the evaluation. No query of the set reached the limit: all end with `Exhausted`, and none exhausted the visit budget (§3, checks).

### 1.2 Arms

- **B1**: one-hop neighbours through `relations_of`, outwards and then inwards, without filter, cut at 10. It is the same code as `tests/assoc_baseline.rs`; the result is compared with the raw output of E7-00 (§3).
- **Simple order**: `reference_assoc_query` with the rule of §1.1. Its order is "distance, then identifier", by contract of `AssocEnding::LimitReached`.
- **Ranking**: the same result sorted with `RankMode::On`, with the default weights (`RankWeights::default()`: hops 1 000, type 500, recency 400, provenance 250, similarity 1 500), without preferred types and **without vectors**.

**Vector signal.** It is not measured for quality. The set contains no vectors (evaluation set, §8 point 7): similarity is 0 for all results, and the quality arm does not exercise it.

### 1.3 Checks before any figure

All run inside the test and, if any fails, the test fails before printing the figures:

| Check | Result (raw output) |
|---|---|
| `MANIFEST`: SHA-256 of `corpus.txt` and `queries.txt` | 2 of 2 match |
| I-32: index = reference for each query | 64 of 64 |
| Planner = reference | 64 of 64, 0 fallbacks to the reference path |
| I-41: sum of the contributions = score, in diagnostic mode | 757 hits of 757 |
| I-40: each hit cites existing relations and revisions, with their fields, on a real path from a seed, and meets the query's filter | 757 of 757 hits |
| B1 reproduced from the E7-00 output | 64 of 64 queries, with the three printed decimals |
| Query termination | 64 `Exhausted`; no `LimitReached`; no `BudgetExhausted` |

## 2. Quality results

Means over the queries, cut at 10. Source: the `summary` table of the raw output.

### 2.1 Summary

| Scope | Queries | B1 R@10 | B1 nDCG@10 | Simple order R@10 | Simple order nDCG@10 | Ranking R@10 | Ranking nDCG@10 |
|---|---|---|---|---|---|---|---|
| **overall** | 64 | 0.731 | 0.500 | **0.956** | **0.942** | **0.939** | **0.912** |
| `multi_hop` | 16 | 0.658 | 0.473 | 0.868 | 0.960 | 0.819 | 0.871 |
| `temporal` | 16 | 0.641 | 0.493 | 0.973 | 0.978 | 0.981 | 0.970 |
| `provenance` | 16 | 0.864 | 0.510 | 0.982 | 0.849 | 0.967 | 0.842 |
| `combined` | 16 | 0.761 | 0.523 | 1.000 | 0.981 | 0.991 | 0.965 |

Reading the table:

- B1 finds many of the relevant ones (recall 0.731) but orders them badly (nDCG 0.500). It is the reading E7-00 already made.
- The simple order already beats B1 in every class, by a wide margin. Its nDCG is 0.942 overall.
- The ranking with the default weights **lowers** nDCG relative to the simple order in the four classes (in `provenance` the difference is 0.007). It also lowers recall overall (0.956 → 0.939).

### 2.2 Per query

The full table of the 64 queries (B1, simple order and ranking, with their termination) is in the raw output, under `per query`. The queries where the ranking loses most against the simple order are the `multi_hop` ones of depth 4 with direction `out` and `in` (`MH-06`: nDCG 0.884 → 0.552; `MH-10`: 0.929 → 0.548) and `CB-12` (0.966 → 0.696). It has not been analysed which signal causes each loss: it was not the object of this measurement, and nothing is adjusted here.

## 3. Thresholds of §6.2

Each threshold, with its number and its status. A threshold that is not met is recorded as such; it is not modified or recalculated with other weights.

| Threshold (minutes §6.2) | Bound | Measured value | Status |
|---|---|---|---|
| **T1** recall@10 with ranking, overall | ≥ 0.90 | **0.939** | **met** |
| **T2** nDCG@10 with ranking, overall | ≥ 0.80 | **0.912** | **met** |
| **T3** nDCG@10 with ranking ≥ 1.10 × B1 | ≥ 0.550 (B1 = 0.500) | **0.912** | **met** |
| **T4** nDCG@10 with ranking ≥ 1.10 × simple order | ≥ 1.036 (simple order = 0.942) | **0.912** | **not met**: see note |
| **T5** nDCG@10 with ranking ≥ B1, in `multi_hop` | ≥ 0.473 | 0.871 | met |
| **T5** … in `temporal` | ≥ 0.493 | 0.970 | met |
| **T5** … in `provenance` | ≥ 0.510 | 0.842 | met (margin 0.332) |
| **T5** … in `combined` | ≥ 0.523 | 0.965 | met |
| **C1** hits with verifiable trace (I-40; correctness criterion, not a quality threshold) | 100 % | **757 of 757** | **met** |

**Note on T4.** The threshold asks that the ranking exceed the nDCG of the simple order by 10 %. With the simple order at 0.942, the bound is 1.036. nDCG is bounded by 1, so **no** ranking can meet T4 while the simple order has an nDCG above 0.909. This is a defect of the threshold as written in §6.2, not a result of the measurement, and it is not corrected here: the threshold does not change in this stage. The owner needs to decide whether §6.2 is rewritten before measuring again. With the threshold as it stands, no possible ranking meets it.

**Verdict of §6.2 as a whole: not met.** T4 is not met, and the ranking also falls below the simple order (0.912 against 0.942), which is what §7 point 3 of the minutes looks at to decide the default order.

## 4. Performance (§6.3)

Scale: 20 000 units, 39 958 relations, 3 000 change sets, `KernoDb` on disk (real file system, without the simulator), the 64 queries of the performance set with the rule of §1.1 and the depth replaced by 1, 2 or 3. Each run measures 512 pairs (64 queries × 8 counted rounds) per path and depth. The two paths run over the same snapshot, one after the other, with the order alternated pair by pair.

**Protocol** (fixed before the first run; `crates/kerno-units/examples/assoc_bench.rs`):

- three invocations of the command, in separate processes, each with four runs: **twelve runs**, all published;
- each run goes through the rounds at one depth; the first four rounds of each run are warm-up and do not count (see the note below);
- p50 and p99 per path, with the nearest-rank method;
- the load (average of 1, 5 and 15 minutes from `/proc/loadavg`) is recorded before and after each run; the machine has 4 CPUs;
- the invocations ran one by one, never in parallel.

**Verdict of §6.3 (depth 2, p50):** the p50 of the index traversal is **not worse** than that of the reference, in all **twelve** runs. Index: between 0.147 and 0.159 ms; reference: between 103.9 and 110.5 ms. The index/reference ratio is between 0.138 % and 0.144 %. **Met.**

Table per run, depth 2 (times in milliseconds):

| Invocation | Run | Reference p50 | Index p50 | Reference p99 | Index p99 | Average load 1 min (before → after) |
|---|---|---|---|---|---|---|
| 1 | 1 | 104.9543 | 0.1481 | 186.7349 | 0.4213 | 6.82 → 4.87 |
| 1 | 2 | 109.2580 | 0.1562 | 202.2549 | 0.4412 | 2.77 → 4.62 |
| 1 | 3 | 106.9097 | 0.1491 | 200.4527 | 0.4123 | 3.83 → 3.69 |
| 1 | 4 | 104.6953 | 0.1500 | 187.5749 | 0.3930 | 2.70 → 1.48 |
| 2 | 1 | 103.8506 | 0.1470 | 193.3364 | 0.4322 | 0.98 → 1.00 |
| 2 | 2 | 106.7527 | 0.1501 | 194.6790 | 0.4264 | 1.03 → 1.01 |
| 2 | 3 | 108.2784 | 0.1547 | 204.9643 | 0.4138 | 1.09 → 1.02 |
| 2 | 4 | 108.2131 | 0.1549 | 195.6016 | 0.4219 | 1.00 → 1.00 |
| 3 | 1 | 110.5134 | 0.1588 | 200.5446 | 0.4349 | 0.96 → 0.99 |
| 3 | 2 | 108.2287 | 0.1493 | 208.5447 | 0.4095 | 1.00 → 1.00 |
| 3 | 3 | 107.6515 | 0.1523 | 187.3676 | 0.4103 | 1.02 → 1.00 |
| 3 | 4 | 108.4628 | 0.1527 | 196.9757 | 0.4251 | 1.06 → 1.02 |

Range across the twelve runs: reference p50 103.85–110.51 ms, index p50 0.147–0.159 ms; reference p99 186.7–208.5 ms, index p99 0.393–0.441 ms.

**Depths 1 and 3 (informative; no threshold).** Across the twelve runs:

| Depth | Entries visited (mean) | Hits (mean) | Reference p50 (range) | Index p50 (range) | Reference p99 (range) | Index p99 (range) |
|---|---|---|---|---|---|---|
| 1 | 3.3 | 2.7 | 26.5–27.9 ms | 0.059–0.066 ms | 32.0–42.6 ms | 0.127–0.175 ms |
| 2 | 11.1 | 6.3 | 103.9–110.5 ms | 0.147–0.159 ms | 186.7–208.5 ms | 0.393–0.441 ms |
| 3 | 25.1 | 12.9 | 165.1–172.9 ms | 0.241–0.253 ms | 616.5–654.9 ms | 1.220–1.461 ms |

The entries visited and the hits are the same in both paths; each pair is compared before counting (the results are equal, see §1.3).

**Note on warm-up.** The header of `assoc_bench.rs` says that warm-up is 10 % of the rounds. The committed runs were made with `--reps 12` and four warm-up rounds, that is, a third of the rounds, not a tenth: the 10 % bound corresponds to the default value (`--reps 40`) and not to this command. With more warm-up, more samples are discarded, not fewer, and the verdict does not depend on it. It is recorded as a deviation from the header, not from the measured protocol.

**Honest reading.**

- The effect is large and does not depend on the load: the index traversal takes ≈ 0.15 ms, the reference ≈ 105 ms, at depth 2. The reference walks the relations at each hop (`reference_relations_of`), and the adjacency index reads only the unit's ranges. The ratio is about 700 times, and it holds in all twelve runs, including those of the first invocation, which started with a load of ≈ 8.7 over 4 CPUs.
- **This ratio is not that of a system in real use.** The reference is the E7-01 oracle and not a production path. What is stated is that the index path is not worse than the reference path, which is what §6.3 asks.
- As the §6.3 threshold is a comparison between paths, the measurement does not fix an absolute time to meet: there is no absolute latency threshold in the minutes.

## 4.1 Performance result against §6.3

| Criterion | Number | Status |
|---|---|---|
| With 20 000 units and depth 2, the index p50 is not worse than the reference p50 (twelve runs) | index p50 ≤ reference p50 in 12 of 12 runs; ratio 0.138 %–0.144 % | **met** |
| p50, p99 and entries visited by depth (1 to 3) | reported (§4) | informative |

## 5. What is not claimed, and why

1. **The vector signal is not measured for quality.** The set has no vectors. ADR-0011 says the caller supplies the vectors, so measuring it requires a set with vectors, which does not exist at this stage.
2. **The simple order may benefit from the structure of the set.** Identifiers are assigned in creation order, and the simple order breaks ties by identifier. The set corrects the single-field leaks that were known (evaluation set, §9), but leaves a residual structural signal: the index distance between origin and destination of a true `related` is right 0.729 of the time over the corpus (evaluation set, §8 point 3). A control that breaks ties in reverse or at random has not been run, so it is not known how much of the 0.942 comes from relevance and how much from that structure. The absolute figures of the simple order should not be read as the method's quality.
3. **Synthetic set, same author for the narrative and the queries** (evaluation set, §8 points 1 and 2). Small differences are not significant: with 64 queries and between 1 and 9 relevant per query, the variance of the mean is high.
4. **A single set of weights, without tuning.** The ranking is measured with the default weights, as agreed. A ranking with other weights would be another method: it would need a different measurement set, because tuning weights on this set would contaminate it.
5. **T4 cannot be met** (§3). With this threshold, no ranking passes §6.2 even if it beats the simple order on everything else. The measurement says nothing about a ranking with another threshold.
6. **Performance on a shared machine**: the average load was ≈ 8.7 over 4 CPUs when the first invocation started, and ≈ 1 in the other two. The verdict of §6.3 was met in all three, and the difference is almost three orders of magnitude, so the load does not decide it; it does affect the absolute times, which are not asserted beyond what is recorded in §4.
7. **Not measured**: a quality scale other than 2 000 units, a 100 000 scale, depth 4 in performance, concurrency between reads, vectors.

## 6. Recommendation

**The default order remains "hops, then identifier".** The ranking stays as an option, switched off by default, as E7-04 left it.

Reasons, in order of weight:

1. §7 point 3 of the minutes: the ranking is activated by default only if it meets §6.2. It does not meet it (T4), and threshold T4 itself cannot be met as written.
2. Even without looking at T4, the ranking with default weights worsens the simple order in nDCG in the four classes (set: 0.912 against 0.942), and in recall (0.939 against 0.956).
3. What the ranking does well (beating B1 with room, meeting T1 to T3) the simple order also does, and better.

**Decisions left to the owner** (the roadmap, Stage 7, "Cut-off point in E7-05", says that Stage 8 does not start until the owner decides how to proceed):

- Accept the negative result, close the stage with the ranking as an option (minutes §5, risk "The ranking does not improve on the simple order"), or
- revise §6.2 (in particular T4) and repeat the measurement with a new set, not this one, because the set has already seen the results.

## 7. Reproduction

```text
# quality (every time cargo test runs; also in CI)
cargo test -p kerno-units --release --test assoc_measure -- --nocapture --exact quality_over_the_frozen_set

# performance (not run by cargo test; three invocations of four runs)
cargo run --release -p kerno-units --example assoc_bench -- --runs 4 --reps 12 --dir <temporary directory>
```

The raw output [stage7-measurement-output.txt](stage7-measurement-output.txt) is that of these commands, in this order: quality, and then the three performance invocations, with the machine's load in each run.

## 8. Verification

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all --check` | 0 (after applying `cargo fmt --all` to the two new files) |
| Lints, with warnings as errors | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| Suite of `kerno-testkit` and `kerno-units` | `cargo test -p kerno-testkit -p kerno-units --all-features --release` | 0; **645 passed, 0 failed** |
| Test count | before → after | 644 → 645 (`assoc_measure::quality_over_the_frozen_set`). The change does not touch any other test file |
| Mutation 1 of the test: compare the wrong file in the `MANIFEST` | change `CORPUS.as_bytes()` to `QUERIES.as_bytes()` | **fails** (the test ends red; the exact message was not copied) |
| Mutation 2: invert the trace comparison (I-40) | `trace != kept` → `trace == kept` | **fails**: "MH-01: a hit does not verify (I-40): … the trace differs from the stored relation" |
| Mutation 3: invert the contributions check (I-41) | `assert_eq!` → `assert_ne!` | **fails**: "MH-01: the contributions do not add up to the score (I-41)" |

Each mutation was made on the test file, the test was run red, and the original text was restored (checked with `grep` before continuing). No production code was touched: `assoc.rs`, `assoc_rank.rs`, the planner, the frozen set, the persistent format or any threshold.

**Deviations and what was corrected after measuring.**

- The header of `assoc_bench.rs` described warm-up as 10 % of the rounds; the committed runs used four of twelve rounds (see §4). The header was corrected after measuring; it changes no calculation.
- After measuring, `cargo fmt` reformatted the two example and test files. It changes no instruction, but the performance binary in the raw output was compiled before that change and before the header correction.
- The committed quality output is from a second run of the same test, with no changes in the measuring code; it reproduces the first run (same figures, same verdicts).
