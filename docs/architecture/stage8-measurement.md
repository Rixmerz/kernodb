# Stage 8 · E8-07 — Measurement of the derived data

| Field | Value |
|---|---|
| Date | 2026-11-12 |
| Status | **Measured.** Quality: Q1, Q2, Q3 and Q5 are met; Q4 is met where it can be computed and is **not closed**, because 3 of its 4 classes cannot be measured with this set. Q6 is not measured. Performance: P1 to P4 are met |
| Minutes | [Stage 8](stage8-record.md) §6.2 (quality), §6.3 (performance), decisions D4, D5, D9 and D10 (§8, §8.1), row E8-07 |
| Set | `crates/kerno-testkit/data/derived-eval/` (`MANIFEST` checked before measuring) · [evaluation set](stage8-evaluation-set.md) |
| Code | `crates/kerno-units/tests/derived_measure.rs` (quality, P3 and P4; runs with `cargo test`) · `crates/kerno-units/examples/derived_bench.rs` (P1 and P2; does not run with `cargo test`) · `crates/kerno-units/tests/harness/derived.rs` (set loader) |
| Raw output | [stage8-measurement-output.txt](stage8-measurement-output.txt) |

This document does not change any threshold (D4), any expected answer or any weight. Every figure comes from the raw output; what is not claimed is in §6.

## Result in one line

All the thresholds of §6.2 and §6.3 that can be computed are met. **But the set carries a query vector in only 25 of its 100 queries** (those of the vector neighbourhood class), so Q2, Q3 and Q4 are computed over those 25, and Q4 "in each class" cannot be computed for the other three classes: it is reported as **not measurable**, not as met.

## 1. Method

### 1.1 Checks before any figure

They run inside the test; if one fails, the test fails before it prints any figure.

| Check | Result (raw output) |
|---|---|
| `MANIFEST`: SHA-256 of `corpus.txt`, `vectors.txt` and `queries.txt` | "match" in both engines. If it does not match, the test refuses to measure (`a_manifest_that_does_not_match_refuses_to_measure`) |
| Load | 2 000 units, 5 062 relations, 300 change sets; 2 000 vectors persisted, reopened and served |
| Queries | 100 translated and run through the reference path, the indexed one and the planner; 0 planner fallbacks |

### 1.2 Arms (cut at 10, over the 100 queries unless stated)

- **simple**: reference order "distance, then identifier". The identifiers are permuted with a seed, so the tie-break favours no structure.
- **ranked**: the same result with `RankMode::On`, default weights, no vectors. It is the "no signal" of Q4.
- **vector** (only the 25 queries with a vector): exact cosine search over the persisted vectors, read from the reopened database.
- **hybrid** (the same 25): `ranked` with the vector signal, over the persisted vectors. It is the "with signal" of Q2, Q3 and Q4.
- **brute** (the same 25): independent brute-force cosine order over the full vectors of the set. It is the reference for Q1.

The unjudged units in the top 10 are excluded from the list before the metrics are computed and are counted per arm (minutes §6.1). The nDCG ideal and the recall total of relevant units are still those of the whole judgement list.

### 1.3 Which queries carry a vector

| Class | Queries with a vector |
|---|---|
| `vector_neighbourhood` | 25 of 25 |
| `temporal` | 0 of 25 |
| `provenance` | 0 of 25 |
| `combined` | 0 of 25 |

The vector signal can only be applied to a query that carries a vector. Where the threshold says "overall", the figure is that of those 25 queries, and it is labelled that way on each line of the output ("computed on the 25 queries with a vector"). Where it says "in each class", the three classes without a vector cannot be computed.

## 2. Quality (§6.2)

Identical values in `emulated_log` and `kerno_db` (the output compares the 63 lines of each engine and finds them identical).

| Threshold | Bound | Measured value | Over how many queries | Status |
|---|---|---|---|---|
| **Q1** recall@10 of the persisted exact search against brute force | = 1.000 | **1.000**; 25 of 25 lists identical and in the same order | 25 | **met** |
| **Q2** nDCG@10 with signal (hybrid), overall | ≥ 0.80 | **1.000** | 25 | **met** (over the 25) |
| **Q3** recall@10 with signal (hybrid), overall | ≥ 0.90 | **1.000** | 25 | **met** (over the 25) |
| **Q4** nDCG@10 with signal ≥ without signal − 0.01, overall | ≥ 0.990 | 1.000 (without signal 1.000) | 25 | **met** (over the 25) |
| **Q4** … `vector_neighbourhood` | ≥ 0.990 | 1.000 (without signal 1.000) | 25 | **met** |
| **Q4** … `temporal` | — | — | 0 of 25 | **not measurable** |
| **Q4** … `provenance` | — | — | 0 of 25 | **not measurable** |
| **Q4** … `combined` | — | — | 0 of 25 | **not measurable** |
| **Q5** unjudged ≤ 10 % of the top 10, per arm | ≤ 10 % | simple 0.0 % (0 of 708); ranked 1.1 % (8 of 708); vector 0.0 % (0 of 250); hybrid 0.0 % (0 of 95); brute 0.0 % (0 of 250) | per arm | **met** |
| **Q6** | — | — | — | **not measured**: not a closing criterion of Stage 8 |

**Q2, Q3 and Q4 give 1.000 in the arm with signal and also in the arm without it.** A perfect value in both arms is a ceiling effect: these 25 queries do not distinguish one arm from the other, so the thresholds are met but the measurement adds little evidence of quality. The set and the threshold were not changed after measuring; it stands as a limit for the closure.

**Q4 is not closed.** The output says so: "met on the scopes that can be computed; NOT closed: 3 of 4 classes are not measurable". A secondary reading (without signal = simple order) also gives 1.000 against 0.990, and does not replace the primary one.

**Q5 per class** (informative, ranked): `temporal` 5 unjudged of 243 positions, `provenance` 2 of 124, `combined` 1 of 246, `vector_neighbourhood` 0 of 95. None goes past 10 % (the largest is 2.1 %).

### 2.1 Means (cut 10, unjudged excluded)

| Arm | Scope | Queries | recall@10 | nDCG@10 |
|---|---|---|---|---|
| simple | 100 queries | 100 | 0.925 | 0.928 |
| ranked | 100 queries | 100 | 0.927 | 0.933 |
| simple / ranked / hybrid | 25 with a vector | 25 | 1.000 | 1.000 |
| vector (signal only) | 25 with a vector | 25 | 1.000 | 0.708 |
| brute | 25 with a vector | 25 | 1.000 | 0.708 |
| simple | `temporal` / `provenance` / `combined` | 25 each | 0.932 / 0.892 / 0.876 | 0.962 / 0.852 / 0.896 |
| ranked | `temporal` / `provenance` / `combined` | 25 each | 0.912 / 0.885 / 0.911 | 0.932 / 0.839 / 0.960 |

The figures for the three classes without a vector are from the arms without signal; they are not Q2 to Q4. The nDCG of 0.708 for `vector` and `brute` is the same number because they are the same list (Q1). Why it is lower than that of `hybrid` (1.000) has not been analysed; it was not the object of this measurement.

## 3. Performance (§6.3)

### 3.1 P1 and P2: p50 ratios at 20 000 units

`derived_bench`: 20 000 units, 50 962 relations, 3 000 change sets, `KernoDb` on the real file system. The same repetitions as Stage 7: three invocations in separate processes, four runs each (`--runs 4 --reps 12`, 512 pairs per run and arm, four warm-up rounds), twelve runs published. The two sides of each ratio are measured in the same session, pair by pair and alternating the order. No absolute latency threshold (D9).

| Threshold | Bound | Twelve runs (ratio of p50) | Status |
|---|---|---|---|
| **P1** write with descriptor / without descriptor | ≤ 1.50 in each run | 1.165 to 1.238 (maximum 1.238, invocation 1 run 4) | **met**: 0 of 12 over the ceiling |
| **P2** persisted search / in memory | ≤ 1.10 in each run | 0.921 to 1.007 (maximum 1.007, invocation 1 run 4) | **met**: 0 of 12 over the ceiling |

The three invocations give the verdict "met" at their close; no run was averaged or discarded. The mean 1-minute load before each run was between 1.30 and 1.91 (a 4-CPU machine), recorded in the output.

What it shows: the descriptor and its vector cost between 17 % and 24 % more median time per write (p50 between 0.15 and 0.23 ms), well below the ceiling; and, once loaded, searching over the vectors that came from the reopened database takes the same time as searching over those delivered in memory. What it does not show: P2 does not measure the cost of reading persisted vectors, because it compares two searches over the same already-loaded `VectorSet` (the single read of 20 000 served vectors takes about 10 to 11 ms and is reported separately, it is not part of the ratio); it says nothing about the cold cost of opening the database (reopening: 156 to 162 ms). In invocation 3 the P2 ratios come to 0.92 because the in-memory side was slower (≈ 1.95 ms against ≈ 1.80 ms), not because the persisted side was faster: the ratio is of two noisy medians and its ±8 % difference is machine noise. The p99 values are printed and have no threshold.

### 3.2 P3: invalidation, equality

In both engines: after 286 source changes in 29 change sets, there are 2 400 derived items (2 000 persisted vectors and 400 in-memory data from two sources), 400 expected stale and 400 stale, and the rest `ready`. After reopening: 286 persisted data stale, 286 expected, 1 714 served as `ready`, 0 degraded by the opening. **Met**: the set of stale items is exactly that of the derived items of the sources that changed, set equality and not counter equality.

### 3.3 P4: rebuild, equality

In both engines: the 286 rebuilt vectors have identity, descriptor, vector and entry **byte-for-byte equal** to those of a fresh build over the same history (286 of 286), and no stale item remains after rebuilding. **Met.** The time is reported without a threshold and is not comparable between runs:

| Engine | Rebuild 286 vectors | Build the same 286 from scratch |
|---|---|---|
| `emulated_log` | 1 493.7 ms | 1 170.1 ms |
| `kerno_db` | 19.2 ms | 11.8 ms |

The difference between engines is that of the disk simulator and not that of the derived data. Rebuilding was slower than building from scratch in both engines (≈ 1.3 and ≈ 1.6 times). The cause has not been analysed and there is no threshold on this.

## 4. Threshold table

| Code | Status | Note |
|---|---|---|
| Q1 | met | 25 queries with a vector |
| Q2 | met | computed over 25 of 100 queries |
| Q3 | met | computed over 25 of 100 queries |
| Q4 | met where computed; **not closed** | 3 of 4 classes not measurable (no query vector) |
| Q5 | met | maximum per arm 1.1 % |
| Q6 | not measured | outside the closure, per the minutes |
| P1 | met | 12 of 12 runs, maximum 1.238 |
| P2 | met | 12 of 12 runs, maximum 1.007 |
| P3 | met | equality, both engines |
| P4 | met | byte-for-byte equality, both engines |

## 5. Findings

None on the measured code: it was not touched. The measurement found no defects.

## 6. What is not claimed, and why

1. **The vectors are deterministic and seeded (D5).** They measure the mechanics of search, persistence and invalidation, not the semantic quality of any embedding model. That Q2 and Q3 give 1.000 says that the path with the signal orders as the judgement does over these vectors, which the generator built from the same story as the judgements: it is a measurement with little margin for failure, and it must not be read as real retrieval quality.
2. **Q2, Q3 and Q4 are measured over 25 queries**, not over 100. The minutes write "overall" and "in each class"; the frozen set has no query vector in the other three classes. No query vectors were invented for them (that would change the set and require freezing it again). That is why Q4 stays unclosed. Closing it requires a set with vectors in all classes, which is a decision of the owner.
3. **Corpus, queries and judgements were written by the same team.** There is no independent judgement (minutes §6.1, declared limit). With 25 queries per class, small differences are not significant.
4. **A single quality run** (deterministic except for the P4 times). The `ranked` arm uses the default weights, without tuning; Q6 (ranking against simple order) was not measured and must not be inferred from these figures.
5. **Machine load during the runs**: 1-minute mean between 1.30 and 1.91 on 4 CPUs, with no other load started on purpose; it is in the output. The P1 and P2 ratios are of two medians from the same session, so the common load partly cancels, but no absolute time is claimed.
6. **P1 compares two writes in different ways**: with a descriptor the unit is created and its vector is persisted in a change set; without a descriptor only the unit is created. It measures the overhead of adding the derived item to the write, not that of computing the vector (which the caller supplies, ADR-0011).
7. **Not measured**: 100 000 units, concurrency between reads and writes, vectors of another dimension, consolidation (summaries) for performance, approximate search.
8. **Not done in this package**: the mutation of each new test on a copy of the tree (minutes §4 point 4). The tests `a_manifest_that_does_not_match_refuses_to_measure` and `unjudged_units_are_excluded_and_counted` check the two delicate points of the measurement, but no failure of each one was seen with a recorded mutation.

## 7. Reproduction

```text
# calidad, P3 y P4 (cada vez que se ejecuta cargo test)
cargo test -p kerno-units --release --test derived_measure -- --nocapture --exact quality_over_the_frozen_set

# P1 y P2 (no se ejecuta con cargo test; tres invocaciones de cuatro corridas)
cargo run --release -p kerno-units --example derived_bench -- --runs 4 --reps 12 --dir <directorio temporal>
```

The raw output [stage8-measurement-output.txt](stage8-measurement-output.txt) is that of these commands, in this order: the quality test and then the three invocations.

## 8. Verification

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all --check` | 0 |
| Lints | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| Suite | `cargo test -p kerno-testkit -p kerno-units --all-features --release` | 0; **856 passed, 0 failed**. `derived_measure.rs` contributes 4 tests |
| `Cargo.toml` / `Cargo.lock` | — | no changes |
