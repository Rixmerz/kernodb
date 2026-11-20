# Closure of Stage 7

| Field | Value |
|---|---|
| Status | **Closed and approved by the owner on 2026-11-12.** No open blockers (§3.2). The decisions on the start of Stage 8 and on T4 were taken by the owner the same day (§8) |
| Date | 2026-11-12 (E7-06) |
| Act | Adversarial review of E7-00 to E7-05, check of the criteria of §4 of the minutes, thresholds of §6, proposal to promote I-40 and I-41, and update of roadmap, SAD and invariants (minutes §3, row E7-06) |
| Reviewed base | Tree at `52f026f` (merge after the closure of E7-05). This change only touches `docs/` and adds no tests or code |
| Documents | [Stage 7 minutes](stage7-record.md) · [evaluation set](stage7-evaluation-set.md) · [associative query (E7-01)](stage7-associative-query.md) · [indexed execution (E7-02)](stage7-indexed-execution.md) · [temporal context and provenance (E7-03)](stage7-context-and-provenance.md) · [ranking (E7-04)](stage7-ranking.md) · [measurement (E7-05)](stage7-measurement.md) · [raw output](stage7-measurement-output.txt) · [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) · [ADR-0014](../adr/0014-physical-representation-of-relations.md) · [ADR-0015](../adr/0015-privacy-of-observability.md) · [invariants](invariants.md) · [roadmap, Stage 7](../roadmap/README.md#stage-7--associative-memory) · [SAD §9](SAD.md) · [Stage 6 closure](stage6-closure.md) |

This document describes the stage as it stands in the repository. Every figure comes from the committed raw output ([stage7-measurement-output.txt](stage7-measurement-output.txt)) or from a command run for this closure (§5). What could not be checked is stated in §7.

**No threshold, expected answer, weight or file of the frozen set was changed in E7-06.**

## 0. Result in brief

### 0.1 Quality against §6.2 (frozen set, 64 queries, cut at 10)

| Threshold (minutes §6.2) | Measured number | Status |
|---|---|---|
| T1: recall@10 with ranking ≥ 0.90 | **0.939** | met |
| T2: nDCG@10 with ranking ≥ 0.80 | **0.912** | met |
| T3: nDCG@10 with ranking ≥ 1.10 × B1 (≥ 0.550; B1 = 0.500) | **0.912** | met |
| T4: nDCG@10 with ranking ≥ 1.10 × simple order (≥ 1.036; simple order = 0.942) | **0.912** | **not met**. The threshold is unattainable: it demands an nDCG above 1 (§3.2, R-7; [measurement §3](stage7-measurement.md#3-thresholds-of-62)) |
| T5: nDCG@10 with ranking ≥ B1 in each class | multi_hop 0.871 ≥ 0.473; temporal 0.970 ≥ 0.493; provenance 0.842 ≥ 0.510; combined 0.965 ≥ 0.523 | met in the four classes |
| C1: hits with verifiable trace (I-40; correctness criterion) | **757 of 757** | met |

**Verdict on §6.2:** not met, because of T4. T1 to T3 and T5 are met, but they are not enough: §7 point 3 of the minutes activates the ranking by default only if §6.2 is met in full.

### 0.2 Performance against §6.3

| Criterion (minutes §6.3) | Measured number | Status |
|---|---|---|
| With 20 000 units and depth 2, the index p50 is not worse than the reference's | In **12 of 12** runs. Index p50 between 0.147 and 0.159 ms; reference p50 between 103.9 and 110.5 ms; ratio between 0.138 % and 0.144 % | **met** |
| p50, p99 and entries visited by depth (1 to 3) | Informative, no threshold (§4 of the measurement) | reported |

### 0.3 The results, in the form they must be read

1. **The simple order «hops, then identifier» reaches recall@10 0.956 and nDCG@10 0.942**, against 0.731 and 0.500 for the baseline B1.
2. **The ranking with the default weights reaches recall@10 0.939 and nDCG@10 0.912**, below the simple order in nDCG@10 in the four classes. In recall@10 it is below in three of the four: in `temporal` the ranking has 0.981 against 0.973 for the simple order. This claim depends on the tie-break of the simple order (by identifier, which follows the creation order): with other tie-breaks one class changes sign (§3.2, R-6). Overall, the ranking stays below with the three tie-breaks tried.
3. **The ranking does not become the default order.** It stays as an option, switched off by default (`RankMode::Off`), and it is considered experimental. The default order remains «hops, then identifier».
4. **T4 is not attainable as written.** With the simple order at 0.942, the bound is 1.036, and nDCG cannot exceed 1. It is a defect of the threshold in the minutes and it **is recorded as not met, without changing the threshold** (minutes §7 and measurement §3).
5. **The vector signal has not been measured for quality.** The frozen set contains no vectors. The similarity is 0 in every query of the measurement. Measuring it requires a set with vectors, which does not exist today (ADR-0011: the caller supplies the vectors).
6. **The index path meets the performance criterion at 20 000 units and depth 2 in all twelve runs.** The ratio, of about 700 times, is explained in the measurement (§4) and is not that of a system in real use: the reference is the oracle, not a production path.

## 1. Work packages delivered

| WP | Delivery | Commits | Status |
|---|---|---|---|
| E7-00 | Minutes approved (2026-11-11), ADR-0011 accepted, evaluation set frozen with a SHA-256 `MANIFEST`, metrics and baselines B0 and B1 | `5395681` (implementation), `24df5c1` (document) | done. **A later correction of the set** (`6b2a3cb`), before any retrieval code: §3.2, R-1 |
| E7-01 | `AssocQuery` model with hard bounds, breadth-first reference path over `reference_relations_of`, trace per result | `60de6d8` | done |
| E7-02 | Index execution in the planner, check of each answer, fallback to the reference recorded, explanation without literals | `1c377a5` | done |
| E7-03 | `commit_seq` window and provenance, actor and source filters on both paths; verifiable trace (I-40) | `703e657` | done |
| E7-04 | Ranking with diagnostic mode, exact vectors in memory (ADR-0011), `RankMode::Off` by default | `a2d7500`; mark in minutes and roadmap `d13494f` | done |
| E7-05 | Quality measurement over the frozen set and performance measurement; committed raw output | `9be5668` | done. The ranking does not become the default order |
| E7-06 | This closure: adversarial review (§3), check of §4 and §6 (§0 and §2), proposed promotion of I-40 and I-41 (§6), update of roadmap, SAD and invariants | this change | done; **approved by the owner on 2026-11-12** |

No package was added outside the minutes. There are no changes to the persistent format (minutes §2.3) and no new dependencies (criterion of §4 of the minutes on I-31).

## 2. Acceptance criteria (minutes §4)

| Criterion | Invariant | Evidence | Status |
|---|---|---|---|
| Quality evaluable with a set fixed before measuring | — | `MANIFEST` checked in the test `quality_over_the_frozen_set` (2 of 2 files); set fixed in E7-00 and corrected once before all retrieval code (`6b2a3cb`, earlier than `60de6d8`). See R-1 | **met**, with the exception of R-1 against the row of E7-05 |
| Every answer traceable to its sources | **I-40** | `tests/assoc_context.rs`: `every_cited_relation_and_revision_exists_in_the_snapshot`, `an_old_snapshot_never_cites_what_came_after_it`, `frozen_corpus_cites_only_what_the_snapshot_holds` (two engines); the verifier compares with an independent model, not with the trace itself. `tests/assoc_reference.rs`: `frozen_corpus_traces_are_verifiable`. Measurement: 757 of 757 | **met** |
| The ranking explains its signals | **I-41** | `tests/assoc_rank.rs`: `contributions_add_up_to_the_score_and_match_an_oracle` (independent oracle, 8 seeds, 2 metrics); 42 mutations detected in E7-04. The measurement's check is not independent of `Contributions::sum`: R-3 | **met** |
| Equivalence with the reference path | I-32 | Differential test: `tests/assoc_indexed.rs`: `differential_seeded`, `frozen_corpus_indexed_equals_reference` (the 64 frozen queries, each also with three tight bounds: limit 3; budget just below the full visits; limit 2 and budget 7; and a three-seed query with the hard maxima; in both engines). Measurement: 64 of 64 equal to the reference and to the planner, 0 fallbacks. **Open limitation H-1**: with 64 relations or more and an entry missing at both ends, the index answer differs from the reference without fallback (R-2) | **met with H-1**. I-32 **not claimed** for that case |
| An inferred relation is not presented as declared | I-39 | `tests/assoc_context.rs`: `inferred_is_never_declared_in_a_trace` | **met** |
| No literals outside `diagnostic` | I-37 | `tests/sentinels.rs`: `no_sentinel_in_any_associative_plan_observation_or_error` (two engines); `tests/assoc_rank.rs`: `sentinels_no_literal_outside_the_diagnostic_output`. The fingerprint of associative queries uses only markers (`?unit_ids`, `?u32`, `?texts`…), checked in `tests/assoc_indexed.rs` | **met** |
| The core needs no network or model | I-31 | Dependency review: no commit since before E7-00 touches the nine `Cargo.toml` and the repository's `Cargo.lock` (`git log` empty on those paths); `kerno-units` has the same dependencies as before; no reference to a network in `crates/kerno-units/src` nor `crates/kerno-testkit/src`. Vectors are supplied by the caller, in memory | **met** |
| No ANN before a benchmark that justifies it | — | Vector search is exact only (`VectorSet::search`, brute force). No approximate index in `src/` (search for `hnsw`, `ann`, `faiss`, `lsh`, `annoy` and `approximate` in `kerno-units/src` and `kerno-testkit/src`: no results) | **met by scope** |

**Criterion of E7-05 that is not met literally:** «no threshold or expected answer changes after E7-00». The correction of `6b2a3cb` changed the content of the frozen files and the number of expected answers (from 527 to 533). No threshold of the minutes changed. See R-1. This closure records it as **not met literally**; the owner decides whether to accept the exception.

## 3. Adversarial review (E7-06)

### 3.1 What was reviewed and how

**Code and tests, read:**

- `crates/kerno-units/src/assoc.rs` (520 lines): model, bounds validation, traversal, budget, trace, order.
- `crates/kerno-units/src/assoc_rank.rs` (590 lines): weights, signals, determinism, vectors, rejection checks.
- `crates/kerno-units/src/planner.rs`, functions `decide_assoc`, `explain_assoc`, `assoc_query`, `plan_assoc`, `execute_assoc`, `check_adjacent`, `is_mirrored`, `adjacency_unusable`, `assoc_candidates`.
- `crates/kerno-units/src/fingerprint.rs`: `NormalizedQuery::of_assoc`.
- `crates/kerno-units/tests/`: `assoc_reference.rs`, `assoc_indexed.rs` (including `completeness_is_not_a_query_time_check`), `assoc_context.rs`, `assoc_rank.rs`, `assoc_measure.rs`, `assoc_baseline.rs`, `sentinels.rs` (associative section) and `harness/assoc.rs`.
- `crates/kerno-testkit/src/assoc_eval.rs`: format, reader, `expect_chain`, `walk_chain`, `expect_links`. The rest of the generator was not read line by line.
- `crates/kerno-testkit/src/assoc_metrics.rs` and `data/assoc-eval/MANIFEST`, `corpus.txt`, `queries.txt`.
- Git history of the minutes, of `data/assoc-eval`, of `assoc_rank.rs`, of `assoc.rs` and of `harness/assoc.rs`.
- The `etapa7-*.md` documents were checked against the code on the claims cited in §2 and §3.2; they were not read in full line by line.

**Checks run:**

- `cargo test -p kerno-testkit -p kerno-units --all-features --release`: exit 0 (§5).
- `cargo test -p kerno-units --release --test assoc_measure -- --nocapture --exact quality_over_the_frozen_set`: the output reproduces, line by line, the quality block of [stage7-measurement-output.txt](stage7-measurement-output.txt) (from `MANIFEST:` to the threshold summary).
- Mutations in a **copy** of the tree at `HEAD`, with its own build directory, outside the repository: two mutations of E7-05 and one of E7-04, plus a new mutation (§3.3).
- Exploratory controls in the same copy, **not committed to the repository** (§3.4).

**What was checked in the code and is not a finding:**

- No literal reaches a plan: the fingerprint uses markers; `Plan::of` does not receive the query; `AssocError::kind` gives only the class; the messages carry only identifiers and numbers.
- The order is total and integer: `(distance, identifier)` in the traversal and `(score, identifier)` in the ranking; there are no hash maps in `assoc.rs` nor in `assoc_rank.rs`.
- The I-40 verifier in `assoc_context.rs` compares with a model written separately, not with the trace.
- An entry that leads to an inactive unit counts as examined and does not enter the result (`seen` marks each unit once, whether active or not).
- The expected answers are built with the story's ledger (`Ledger`) and with the attributes of each relation, not with the adjacency of the generated graph; `corpus_query` does not read expected answers.

### 3.2 Findings

Each finding states its evidence and whether it is blocking. **Blocking: none.**

| # | Finding | Evidence | Class | Cause | Blocking | Status |
|---|---|---|---|---|---|---|
| R-1 | The frozen set changed after E7-00: `6b2a3cb` regenerated `corpus.txt`, `queries.txt` and `MANIFEST`, and the expected answers went from 527 to 533. It happened before any retrieval code (`60de6d8` is later) and is documented in [set §9](stage7-evaluation-set.md#9-correction-before-use-single-field-leaks). The correction was motivated by single-field leaks detected in the data, not in the code | `git show 6b2a3cb --stat`; `git log --oneline -- crates/kerno-testkit/data/assoc-eval` (two commits: `5395681` and `6b2a3cb`); set §9.3 | deterministic (history) | introduced in E7-00, correction | no | **recorded**: criterion of E7-05 not met literally. The owner accepts or rejects the exception |
| R-2 | **H-1 limit of the index**: with 64 relations or more, a relation with no adjacency entry at either of its two ends is not detected at query time. The index answer differs from the reference without fallback. Only `verify_units` (UNIT-36) sees it | `tests/assoc_indexed.rs:637` (`completeness_is_not_a_query_time_check`): in the large store, `out.result != want`, `fallback == None` and `UNIT-36` in the report; test run green | deterministic (test that fixes the limit) | pre-existing (E7-02; H-1 of the minutes §5) | no | **accepted as risk** (minutes §5); I-32 not claimed in that case. See §7 |
| R-3 | **The I-41 check in the measurement is not independent of `Contributions::sum`.** With `sum()` altered by +1, `assoc_measure` stays green; only 8 tests of `assoc_rank` (oracle) fail | Replicated in the copy (§3.3, mutation M-R4): `test result: ok. 1 passed` in `assoc_measure`; `test result: FAILED. 11 passed; 8 failed` in `assoc_rank` | deterministic | pre-existing (E7-05) | no | **recorded**. The independent evidence of I-41 is the oracle, not the figure «757 of 757» |
| R-4 | **The counter of C1 is tautological.** `traces.0` and `traces.1` add the same number, `simple.hits.len()`. A failure `panic`s before printing, so the figure 757/757 counts checked hits, not an independent count | `crates/kerno-units/tests/assoc_measure.rs`, lines that increment `traces` (just after `trace_failure`) | deterministic (code) | pre-existing (E7-05) | no | **recorded**; no effect on the verdict |
| R-5 | **The judgement convention may favour the simple order.** Every unit that the set does not name counts as grade 0. Over the 64 queries, the top 10 of the simple order has 36 of 476 unnamed positions (7.6 %); that of the ranking, 53 of 476 (11.1 %). No named relevant unit goes unreached (322 of 322) | Exploratory control (§3.4) over the output of `reference_assoc_query` and `rank_assoc`. Deterministic | deterministic (count) | design of the set (§5.2: distractors are «named», only the judged ones) | no | **recorded as a possible bias against the ranking**. The nDCG difference between orders is not a clean measure of relevance |
| R-6 | **The result per class depends on the tie-break of the simple order.** With the tie-break inverted (identifier descending within each distance), the simple order gives nDCG@10 0.932 overall; in `multi_hop` it drops to 0.869, below the ranking (0.871). With a mixed tie-break, 0.942 overall. In `provenance` the simple order rises to 0.897 and 0.886 (against 0.849). Below overall with the three tie-breaks | Exploratory controls (§3.4) | deterministic | design (tie-break by identifier, which follows the creation order) | no | **recorded**. The phrase «below the simple order in the four classes» holds only for the tie-break defined in the minutes; overall, the ranking stays below with any of the three |
| R-7 | **T4 is unattainable as written** (minutes §6.2): it demands 1.036 with an nDCG bounded by 1. Any ranking with the simple order above 0.909 fails T4 | Arithmetic: 1.10 × 0.942 = 1.036; `assoc_measure.rs` prints «T4 … ≥ 1,036 … 0,912 -> not met» | deterministic | defect of the threshold (minutes §6.2) | no | **recorded as not met**. The threshold is not changed (§8.2) |
| R-8 | **The measured configuration of the ranking is partial.** With `RankOptions::default()`, the type signal is 0 (there are no preferred types) and the similarity is 0 (there are no vectors). Only hops, provenance and recency act. Recency has not been measured separately; the set has no unit updates, so recency follows the creation order | `assoc_measure.rs` (`RankOptions { mode: On, ..default() }`); [ranking §1](stage7-ranking.md) | inferential (code) | design of the measurement | no | **recorded**: the conclusion about the ranking holds for that configuration |
| R-9 | **The phrase «weights with a measured default value»** (minutes §2.3) does not describe what happened: the weights were justified before measuring (E7-04) and were not tuned. «Measured» means here evaluated once over the set | `git log -- crates/kerno-units/src/assoc_rank.rs` (a single commit: `a2d7500`, earlier than the measurement); [ranking §4](stage7-ranking.md) | inferential (reading) | wording of the minutes | no | **recorded**. The minutes are not rewritten in this closure |
| R-10 | **B1's traversal does not receive the query's constraints** (type, direction, window, provenance): B1 is «without filter» by definition (minutes §6.1). So the comparison with B1 favours the associative queries by design. The comparison between simple order and ranking is not affected, because both see the same result | `assoc_measure.rs`, `baseline_b1` | inferential (code) | definition of B1 | no | **recorded** |
| R-11 | **Mutation 1 of the measurement without a recorded message.** The document said «fails (the exact message was not copied)» | Replicated (§3.3, M-R1): the message is `MANIFEST: corpus.txt does not match the frozen file` | deterministic | documentary | no | **resolved** in this closure |
| R-12 | **Stability of the vector signal across platforms**: guaranteed by construction (ADR-0011 and [ranking §6](stage7-ranking.md)), not tested on another platform | Reading of `similarity` in `assoc_rank.rs` | inferential | design | no | **recorded** |

**No finding, but recorded so that it is not read otherwise:** the result of the performance measurement is taken from the committed output and was not repeated in E7-06 (§7).

### 3.3 Mutations replicated in a copy of `HEAD` (`52f026f`)

Each mutation was applied to a copy of the tree, with its own build directory. The working tree was not touched. Command: `cargo test -p kerno-units --release …`.

| # | Mutation | What should fail | Actual result |
|---|---|---|---|
| M-R1 | `assoc_measure.rs:188`: `"corpus.txt" => QUERIES.as_bytes()` (from E7-05, mutation 1) | the `MANIFEST` check | **fails**: `assertion left == right failed: MANIFEST: corpus.txt does not match the frozen file` (`assoc_measure.rs:192`) |
| M-R2 | `assoc_measure.rs:132`: `if trace != kept` → `if trace == kept` (from E7-05, mutation 2, I-40) | the I-40 traversal | **fails**: `MH-01: a hit does not verify (I-40): relation …: the trace differs from the stored relation` |
| M-R3 | `assoc_rank.rs:528`: `score: Some(c.sum())` → `Some(c.sum() + 1)` (equivalent to M13 of [ranking §8](stage7-ranking.md#8-mutations)) | the I-41 tests | **fails** in 8 tests of `assoc_rank` (four in each engine: `contributions_…`, `hand_vectors_with_…`, `hand_vectors_without_…`, `ties_…`) and in `assoc_measure`: `MH-01: the contributions do not add up to the score (I-41)` |
| M-R4 | `assoc_rank.rs:435` (body of `Contributions::sum`): `+ self.similarity` → `+ self.similarity + 1` (not a registered mutation; added in E7-06) | the I-41 tests | `assoc_rank`: **fails** in 8 tests. `assoc_measure`: **passes** (`test result: ok. 1 passed`). See R-3 |

The copies of the mutated files were restored from `HEAD` before each new mutation.

### 3.4 Exploratory controls (not committed)

They were run in the same copy, adding auxiliary tests to `tests/assoc_measure.rs` that walk the same 64 queries with the same translation (`corpus_query`). They change no file of the repository, are not part of the committed measurement and are not used to change any number of §0. They serve to discuss R-5 and R-6.

| Control | Overall R@10 | Overall nDCG@10 | multi_hop nDCG | temporal nDCG | provenance nDCG | combined nDCG |
|---|---|---|---|---|---|---|
| Simple order, as measured (tie-break by identifier) | 0.956 | 0.942 | 0.960 | 0.978 | 0.849 | 0.981 |
| Tie-break inverted within each distance | 0.949 | 0.932 | 0.869 | 0.983 | 0.897 | 0.979 |
| Mixed tie-break (deterministic order by hash of the index) | 0.947 | 0.942 | 0.928 | 0.971 | 0.886 | 0.982 |
| Ranking, as measured (reference) | 0.939 | 0.912 | 0.871 | 0.970 | 0.842 | 0.965 |

Unnamed units in the top 10 (R-5): simple order 36 of 476; ranking 53 of 476. Named relevant units reached by the simple order: 322 of 322.

Reading: the overall result of the ranking below the simple order holds with the three tie-breaks. By class, the comparison is sensitive to the tie-break and to the judgement convention. **These figures do not replace the measurement of E7-05 and are not cited as a result of the stage.**

## 4. Measurement (E7-05, committed output)

Source: [stage7-measurement-output.txt](stage7-measurement-output.txt) and [measurement §2 to §4](stage7-measurement.md). Cut at 10; means over the 64 queries.

### 4.1 Quality

| Scope | Queries | B1 R@10 | B1 nDCG@10 | Simple order R@10 | Simple order nDCG@10 | Ranking R@10 | Ranking nDCG@10 |
|---|---|---|---|---|---|---|---|
| **overall** | 64 | 0.731 | 0.500 | **0.956** | **0.942** | **0.939** | **0.912** |
| `multi_hop` | 16 | 0.658 | 0.473 | 0.868 | 0.960 | 0.819 | 0.871 |
| `temporal` | 16 | 0.641 | 0.493 | 0.973 | 0.978 | 0.981 | 0.970 |
| `provenance` | 16 | 0.864 | 0.510 | 0.982 | 0.849 | 0.967 | 0.842 |
| `combined` | 16 | 0.761 | 0.523 | 1.000 | 0.981 | 0.991 | 0.965 |

The endings of the 64 queries of the simple order are all `Exhausted`: none reached the limit of 1 000 results nor exhausted the budget of 100 000 visits. The planner did not fall back to the reference path in any query (`planner fallbacks: 0`).

### 4.2 Performance (depth 2, the twelve runs)

| Invocation | Run | Reference p50 (ms) | Index p50 (ms) | Reference p99 (ms) | Index p99 (ms) |
|---|---|---|---|---|---|
| 1 | 1 to 4 | 104.70 to 109.26 | 0.148 to 0.156 | 187 to 202 | 0.393 to 0.441 |
| 2 | 1 to 4 | 103.85 to 108.28 | 0.147 to 0.155 | 193 to 205 | 0.414 to 0.432 |
| 3 | 1 to 4 | 107.65 to 110.51 | 0.149 to 0.159 | 187 to 209 | 0.410 to 0.435 |

Total range: reference p50 103.9–110.5 ms; index p50 0.147–0.159 ms. Entries visited by depth (mean): 3.3 (depth 1), 11.1 (depth 2), 25.1 (depth 3). The full table per run is in [measurement §4](stage7-measurement.md#4-performance-63).

Deviations recorded in the measurement: warm-up of four rounds out of twelve (a third, not the 10 % of the header; the header was corrected after measuring, without changing any calculation).

## 5. Verification of E7-06

| Check | Command | Result |
|---|---|---|
| Suite of `kerno-testkit` and `kerno-units` | `cargo test -p kerno-testkit -p kerno-units --all-features --release` | exit 0; sum of the `test result` lines: **645 passed, 0 failed, 8 ignored**. Same as in [measurement §8](stage7-measurement.md#8-verification) (645). No new test in E7-06 |
| Quality measurement, reproduction | `cargo test -p kerno-units --release --test assoc_measure -- --nocapture --exact quality_over_the_frozen_set` | `test result: ok. 1 passed`; the block from `MANIFEST:` to the threshold summary matches `stage7-measurement-output.txt` line by line |
| Format | `cargo fmt --all --check` | exit 0 |
| Lint with warnings as errors | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `Finished dev profile [unoptimized + debuginfo] target(s) in 30.52s`; exit 0 |
| Tests cited in §2 | search in the suite's output (`suite`, with `test … ok`) | they appear as `ok`; in files with two engines, in both (`emulated_log` and `kerno`, or `kerno_db` in `sentinels`) |
| Dependencies | `git log` over the nine `Cargo.toml` and the `Cargo.lock` since before E7-00 | no commits |
| Network in the core (I-31) | search for network references in `kerno-units/src` and `kerno-testkit/src` | none |

The performance bench (`assoc_bench`) was not run in E7-06: the numbers of §4.2 are those of the commit (§7).

## 6. Promotion of I-40 and I-41

Both are **promoted and approved by the owner on 2026-11-12**. The evidence of each is in §2 and §3. None of the observations of §3.2 is blocking for them: R-2 (H-1) affects I-32, not I-40 nor I-41; R-3 and R-4 affect how the measurement is cited, not the existence of the independent test.

### 6.1 I-40

**Statement (minutes §4):** each result of an associative query cites the relations and revisions that justify it, and all of them exist in the snapshot queried.

**Evidence:** independent verifier (model written separately) in `tests/assoc_context.rs`, in both engines; test over the frozen set; measurement with 757 of 757 hits. Mutations of E7-03 and mutation M-R2 of §3.3, which make the test fail.

**Limits:** I-40 covers the integrity of the trace (existence, fields, path, head revision, filters). It does not cover that the result set is complete: that is the H-1 part.

### 6.2 I-41

**Statement (minutes §4):** the score is deterministic and the diagnostic mode gives the contribution of each signal, whose sum is the score.

**Evidence:** `contributions_add_up_to_the_score_and_match_an_oracle` (independent oracle over `get_relation` and `get_unit`, described in [ranking §7](stage7-ranking.md#7-tests)); `hand_vectors_*` with values calculated by hand; `same_state_and_query_give_the_same_order`; mutations of E7-04 (M01 to M42) and mutation M-R3 of §3.3.

**Limits:** the check of the measurement (R-3) does not detect a fault inside `sum`. The oracle does. The determinism of vector similarity across platforms is not tested (R-12).

## 7. Open risks and limits

1. **Index H-1 (R-2).** With 64 relations or more, a relation with no adjacency entry at either end is not detected at query time. The index answer can be incomplete without fallback. The test `completeness_is_not_a_query_time_check` fixes the limit: if someone closes it, the test must change. It is covered by `verify_units` (UNIT-36) and by the differential test over healthy stores.
2. **The set is synthetic and of a single author** (set §8, points 1 and 2). The narrative, the queries and the grade rules were written by the same author, and the expected answers walk the chain of decisions that the graph also contains. That is why the simple order, which walks the graph breadth-first, can score high because of that shared origin. **It cannot be stated that the set is easy or hard**: there is no other independent measure of difficulty.
3. **Regular structure** (set §8, point 5): three decisions per topic and session, linear chains, twelve topics. Real memory is more irregular.
4. **Judgement convention (R-5)**: unnamed units are worth 0. It penalises more the arm that has more unnamed units in its top 10; today, the ranking.
5. **Sensitivity to the tie-break (R-6)**: the per-class comparisons change with the tie-break. The overall one does not.
6. **Configuration of the ranking (R-8)**: no type signal and no vectors. Recency has not been measured separately.
7. **Vector signal not measured for quality**: the set has no vectors.
8. **Sixty-four queries**, with 1 to 9 relevant each. A small difference between two methods is not significant (set §8, point 8).
9. **Performance**: a machine of 4 CPUs with an average load of 1 to 9 during the measurement; the absolute figures are not comparable between machines. The twelve runs are from the committed output; **they were not repeated in E7-06**.
10. **Scales not measured**: quality at 2 000 units only; performance at 20 000 only; depth 4 in performance not measured; concurrency between readers not measured.
11. **Stability of the vector similarity across platforms** (R-12): not tested outside this machine.
12. **The default weights are not tuned** (R-9): they were fixed by justification. Changing them requires another measurement with another set, because tuning on this one would contaminate it.
13. **The ranking does not see units that the cut leaves out** (ranking §10): the (hops, identifier) order limits the result before sorting by score. If the relevant ones lie beyond the limit, the ranking does not recover them.
14. **Recency not measured separately**, and the set has no unit updates (R-8).

## 8. Owner decisions

The owner took these two decisions on 2026-11-12. They are recorded here, with the recommendation that the evidence supported at the time of the closure.

### 8.1 Does Stage 8 start?

**Why there is a pending decision:** the [roadmap](../roadmap/README.md#owner-decisions-of-2026-11-11), point 1, says that if the ranking does not meet the quality thresholds of §6.2, Stage 8 does not start until the owner decides how to proceed. The ranking does not meet §6.2 (T4, and below the simple order in nDCG@10).

**Recommendation the evidence supports:** accept this closure with the simple order «hops, then identifier» as the default order and the ranking as an option switched off (measurement §6, first option). The [roadmap](../roadmap/README.md#stage-8--consolidation-and-embeddings) does not list the ranking among the deliverables or the acceptances of Stage 8, so there is no dependency forcing it to be measured first. There is a warning: Stage 8 will produce the embeddings that the ranking's vector signal would consume, and the quality of that signal has not been measured (§7, point 7). If the owner wants to measure it, a set with vectors is needed, which is a new set (§8.2). Do not make Stage 8 conditional on that measurement.

**Decided by the owner on 2026-11-12:** Stage 8 starts. The ranking of associative results is **switched off by default and marked as experimental**. With the ranking switched off, the order of results is «hops, then identifier».

### 8.2 Is T4 rewritten before any new measurement?

**Why there is a pending decision:** T4 (minutes §6.2) cannot be met as written (R-7). Changing it now, with the current set, would be adjusting the threshold to the results.

**Recommendation the evidence supports:** do not change T4 over this set. If the owner wants to measure the ranking again, rewrite T4 as an attainable threshold (for example, not below the simple order, or an improvement in another metric) **before** measuring, leave it frozen with a date, and build a new set (another seed and, if possible, another author for the queries). Do not use this set to tune weights or thresholds. Measuring again with this set adds no new evidence, because it has already seen the results (measurement §6).

**Decided by the owner on 2026-11-12:** T4 is not rewritten now. The text of T4 in the [Stage 7 minutes](stage7-record.md) (§6.2) stays as it is, and it is not used to decide anything about this set. If the ranking is measured again, it will be with a new evaluation set and a threshold defined at that time, before measuring. No weight or threshold is tuned on the frozen set.

## 9. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-12 | **Closed and approved by the owner** | No open blockers (§3.2). Criteria of §2 met, with H-1 (§2, I-32) and with the exception of R-1 against the row of E7-05. Thresholds of §0.1 recorded as measured; T4 not met and not rewritten (§8.2). I-40 and I-41 promoted and approved (§6). Decisions of §8: Stage 8 starts with the ranking switched off by default and marked as experimental |
