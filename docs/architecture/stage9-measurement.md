# Stage 9 · E9-20 — Single measurement of E8-10

| Field | Value |
|---|---|
| Date | 2026-11-13 |
| Status | **Measured only once.** All the E8-10 thresholds that minutes 9 §5 sets are met (T-E10-V1, V2, R, Q1, Q2, Q3, S1, S2, Q5), and the two time ratios (P1, P2) are within their bound in the four runs. No threshold, file or weight was changed after measuring; after the run only formatting (`cargo fmt`) was applied to the two code files, and the quality output was reproduced identically afterwards |
| Minutes | [Stage 9](stage9-record.md) §5 (thresholds), §3 row E9-20, decisions D7 and D22 (§8.1); P1 and P2 are defined in the [Stage 8 minutes](stage8-record.md) §6.3 |
| Set | `crates/kerno-testkit/data/derived-eval-2/` (frozen in E9-19) · [evaluation set](stage9-evaluation-set.md) |
| Code | `crates/kerno-units/tests/derived_measure2.rs` (quality; runs with `cargo test`) · `crates/kerno-units/examples/derived_bench2.rs` (P1 and P2; does not run with `cargo test`) |
| Raw output | [stage9-measurement-output.txt](stage9-measurement-output.txt) |

The quality and time figures come from the raw output. The number of tests in `derived_measure2` (5; counted by the `cargo test` runner in the runs of §4) and the results of the 7 mutations (§4; they come from runs on copies of the tree, with the `FAILED` lines as they were printed) do not come from it. In the raw output, "minutes §6.3" refers to the Stage 8 minutes. What is not claimed is in §5.

## 1. Method

- It was measured **once**: one invocation of the quality test and one of `derived_bench2`, in that order. The machine load (`uptime`) before each one is recorded in the header of the raw output: 2.27 / 2.44 / 2.67 before the quality test and 1.25 / 2.16 / 2.57 before `derived_bench2`; the 1-minute load was below 4 in both (T-SQ1). Other work was active on the machine during the run, so the two time ratios (P1, P2) were **not** measured on an idle machine. The quality figures do not depend on the load, and a later repetition of the quality test, done by the review, reproduced them line by line.
- T-E10-R: the test checks the SHA-256 of `corpus.txt`, `vectors.txt` and `queries.txt` against `MANIFEST` before any figure; it matches. If it did not match, the measurement would refuse to run (`a_manifest_that_does_not_match_refuses_to_measure`).
- Arms, cut at 10, over the 100 queries (all carry a query vector): `simple` (reference order), `ranked` (`RankMode::On`, default weights, vector signal off), `vector` (exact cosine search over the persisted vectors, read from the reopened database), `hybrid` (`ranked` with the vector signal) and `brute` (independent brute force over the full vectors of the set).
- The unjudged units in the top 10 are excluded before the metrics are computed and are counted per arm and per class (T-E10-Q5). The nDCG ideal and the recall total of relevant units are still those of the whole judgement list.
- The two engines (`emulated_log` and `kerno_db`) give identical figures: the test compares the 85 lines of each report and fails if they differ.
- **Reading of "no signal".** The minutes use the expression with two meanings (T-E10-V2 and the freezing document: the vector-only arm; Q4 of E8: the `ranked` arm without vectors). The **primary** reading was fixed in the code before measuring as the vector-only arm. The second reading is computed in the same run and reported as informative.

## 2. Quality (minutes 9 §5)

| Threshold | Bound | Measured value | Status |
|---|---|---|---|
| **T-E10-R** `MANIFEST` | equal SHA-256 | matches | **met** |
| **T-E10-V1** queries with a vector, per class | ≥ 25 in each class | 25, 25, 25, 25 | **met** |
| **T-E10-V2** nDCG@10 without signal, overall | ≤ 0.96 | 0.4216 | **met** |
| **T-E10-Q1** recall@10 of the persisted exact search against brute force | = 1.000 | 1.000 (minimum and mean); 100 of 100 lists identical and in the same order | **met** |
| **T-E10-Q2** nDCG@10 with signal, overall | ≥ 0.80 | 0.9738 | **met** |
| **T-E10-Q3** recall@10 with signal, overall | ≥ 0.90 | 0.9594 | **met** |
| **T-E10-S1** nDCG@10 with signal − without signal, overall | ≥ 0.02 | +0.5522 | **met** |
| **T-E10-S2** with − without signal, per class | ≥ −0.01 in each one | vector neighbourhood +0.2982; temporal +0.6505; provenance +0.6312; combined +0.6291 | **met** in all 4 |
| **T-E10-Q5** unjudged in the top 10, per arm and class | ≤ 10 % | maximum 1.6 % (`ranked`, combined class, 4 of 250) | **met** |

Second reading of "no signal" (`ranked` arm, informative): S1 = +0.0651 (met); S2 = +0.0000 (vector neighbourhood), +0.0281 (temporal), +0.1832 (provenance), +0.0490 (combined): also met in the four classes. This reading does not change any verdict.

nDCG@10 per arm, overall: `simple` 0.9014; `ranked` 0.9087; `vector` 0.4216; `hybrid` 0.9738. With T-E10-V2 met (0.4216 ≤ 0.96) the set is not saturated in the primary reading, which is what F-07 asked for. In `ranked` there is a partial ceiling overall (0.9087), and that is why the second reading gives a much smaller improvement (+0.0651) than the primary one. The ceiling noted as F-07 repeats fully in the vector neighbourhood class: `simple`, `ranked` and `hybrid` score 1.0000 nDCG@10 and the second reading gives S2 = +0.0000 in that class.

## 3. Time ratios (P1 and P2)

`derived_bench2` does **not** run over `derived-eval-2`: it uses a generated history at Performance scale with the same seed, without the vectors of `derived_eval2`; that is why P2 times only 25 vector neighbourhood queries. 20 000 units, `KernoDb` on the real file system, 4 runs of 12 rounds of 64 pairs, alternating the order. No absolute latency threshold (D7): each threshold is a ratio of two p50 values from the same session.

| Measure | Bound | Run 1 | Run 2 | Run 3 | Run 4 | Status |
|---|---|---|---|---|---|---|
| **P1** p50 with descriptor / without descriptor | ≤ 1.50 | 1.173 | 1.163 | 1.141 | 1.140 | **met** (0 of 4 over the bound) |
| **P2** p50 persisted / in memory | ≤ 1.10 | 0.997 | 1.001 | 1.000 | 1.000 | **met** (0 of 4 over the bound) |

## 4. Verification of the measurement code

`derived_measure2` has 5 tests (the `cargo test` runner counts 5 in each of the following runs: `4 passed; 1 failed` or `2 passed; 3 failed`; before the package the file did not exist). **7 mutations** were run, each on a copy of the tree, one at a time, in release build and with its own build directory, the copy deleted afterwards. All with `cargo test -p kerno-units --all-features --release --test derived_measure2`, over `crates/kerno-units/tests/derived_measure2.rs`. The last column is the `FAILED` line as it was printed; these results do not come from the raw output but from those runs.

| No. | Line | Change | Test(s) that fail | Printed line |
|---|---|---|---|---|
| 1 | 100 | `if &got != digest` → `if &got == digest` | the next three (result: `2 passed; 3 failed`) | `test the_manifest_of_the_second_frozen_set_matches ... FAILED`; `test a_manifest_that_does_not_match_refuses_to_measure ... FAILED`; `test quality_over_the_second_frozen_set ... FAILED` |
| 2 | 100 | `if &got != digest` → `if false && &got != digest` | `a_manifest_that_does_not_match_refuses_to_measure` | `test a_manifest_that_does_not_match_refuses_to_measure ... FAILED` |
| 3 | 173 | `.filter(\|u\| judged.contains(u))` → `.filter(\|_\| true)` | `unjudged_units_are_excluded_and_counted` | `test unjudged_units_are_excluded_and_counted ... FAILED` |
| 4 | 114 | `gain >= S1_MIN_GAIN` → `gain > S1_MIN_GAIN` | `every_threshold_flips_at_its_own_bound` | `test every_threshold_flips_at_its_own_bound ... FAILED` |
| 5 | 354 | `assert_eq!(reopen.served, UNITS` → `UNITS + 1` | `quality_over_the_second_frozen_set` | `test quality_over_the_second_frozen_set ... FAILED` |
| 6 | 385 | `assert_eq!(indexed, simple` → `assert_ne!(indexed, simple` | `quality_over_the_second_frozen_set` | `test quality_over_the_second_frozen_set ... FAILED` |
| 7 | 371 | `reopen.served` → `reopen.served + std::any::type_name::<E>().len()` (report different per engine) | `quality_over_the_second_frozen_set` (agreement between engines) | `test quality_over_the_second_frozen_set ... FAILED` |

Coverage: `the_manifest_of_the_second_frozen_set_matches` (mutation 1), `a_manifest_that_does_not_match_refuses_to_measure` (1 and 2), `unjudged_units_are_excluded_and_counted` (3), `every_threshold_flips_at_its_own_bound` (4) and `quality_over_the_second_frozen_set` (1, 5, 6 and 7). The five tests each have at least one mutation. Mutation 1 does not touch frozen data.

After the recorded run only `cargo fmt` was applied to the two code files, without changing any printed string; the raw output was not regenerated, and a later repetition of the quality test reproduced its identical figures.

## 5. Limits

- The set is synthetic and has 2 000 units; the figures are not extrapolated to other loads.
- The primary reading of S1 and S2 compares against a weak vector-only arm; that is why the differences are large. The second reading, which is stricter, also passes.
- F-08: P2 does not measure the reading of the persisted vectors. The single read of the 20 000 vectors served into an in-memory set took 9.8 ms and is reported without a bound.
- The quality thresholds are printed in the output, not asserted with `assert` in the test (only internal agreements are asserted: manifest, engines and paths); this is a known limit.
- A single measurement, on a single machine; the minutes do not ask for it to be repeated and it was not repeated.
