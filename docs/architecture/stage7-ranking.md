# Ranking of associative results and diagnostic mode (E7-04)

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Done** in E7-04 |
| Minutes | [Stage 7](stage7-record.md) §3 (row E7-04), §4 (I-41, I-37), §7 point 3 |
| ADR | [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md): vectors in memory, cosine and dot product, exact search |
| Code | `crates/kerno-units/src/assoc_rank.rs`, exported from `lib.rs` with one line (`pub mod assoc_rank;`) |
| Tests | `crates/kerno-units/tests/assoc_rank.rs` |
| Earlier documents | [associative query (E7-01)](stage7-associative-query.md) · [indexed execution (E7-02)](stage7-indexed-execution.md) |

This document describes the code as it is. Every figure comes from the commands in §9 or from the mutations in §8. This package does not change the persistent format, the evaluation set, any threshold of the minutes, nor `assoc.rs`, the planner, the minutes or the roadmap. **It does not read the expected answers of the frozen set**, and the weights were not tuned against it: there is no query of it in the tests. Nor does it decide whether the ranking is the default order: that is decided by the measurement of E7-05 (minutes §7 point 3). Here the default value is **off**.

## 1. What it adds

- `UnitSnapshot::rank_assoc(&AssocResult, &RankOptions) -> Result<Vec<RankedHit>, RankError>`: sorts the results of an associative query over the snapshot in which it was run.
- `RankMode`: `Off` (default), `On` and `Diagnostic`.
- `RankWeights` and the constants `WEIGHT_*`: the weights, with their reason (§4).
- `VectorSet`, `VectorQuery`, `VectorMetric`, `VectorError`: the vectors of ADR-0011 and the exact search (§6).

It is a module of its own because `assoc.rs` is edited by another package (E7-03). The caller passes the result it already has; the module runs no query.

## 2. Modes

| Mode | Order | `score` | `contributions` | Reads the snapshot |
|---|---|---|---|---|
| `Off` (default) | That of the result: that of the reference path, «hops, then identifier» | `None` | `None` | No |
| `On` | `(score descending, identifier ascending)` | `Some` | `None` | Yes |
| `Diagnostic` | That of `On` | `Some` | `Some`: the contribution of each signal | Yes |

With `Off` the order is that of `AssocResult::hits` untouched, so it matches that of the reference path and, by I-32, that of the index path. If vectors are supplied, they are checked in the three modes and before anything else.

`RankedHit` carries `unit`, `distance`, `score`, `contributions` and `similarity` (the exact similarity in fixed point, only in `Diagnostic` and only if the unit has a vector). It does not carry the path or the relation types: the caller has those in its `AssocResult`.

## 3. Signals and formula

All contributions are integers (`i64`). A higher score is better. The score is **exactly** the sum of the five contributions (I-41): `Contributions::sum` is the definition of the score, and the test compares each contribution with an oracle written separately (§7).

`score = hops + relation_type + recency + provenance + similarity`

| Signal | Reads | Contribution |
|---|---|---|
| Hops | `distance` | `w_hops · (MAX_ASSOC_DEPTH + 1 − distance)`: 4 weight steps for one hop, 1 for four |
| Relation type | the types of the steps of the path | `w_type · preferred_steps / steps`, integer division (towards zero). It is 0 if the `preferred_types` set is empty |
| Recency | the most recent `commit_seq` the result depends on | `w_recency · H / (H + age)`, with `H = RECENCY_HALF_LIFE = 64` and `age = snapshot_seq − most_recent` |
| Provenance | the weakest class on the path | `w_prov · confidence(class)`: `Declared` 3, `Imported` 2, `Algorithmic` 1, `Inferred` 0 |
| Similarity | the exact similarity with the caller's vector | `w_sim · clamp(sim, −1, 1)`, with `sim` in fixed point (`SIMILARITY_SCALE` = 1 000 000 for 1.0). It is 0 if there are no vectors or the unit has no vector |

Decisions in the definition:

- **Relation type.** The meaning of a type belongs to the caller's domain, not to the core; that is why the signal is relative to a set that the person indicates (`RankOptions::preferred_types`) and not to a fixed list of types. Without a set the signal does not exist (0 for all). Measuring it with a concrete set is work for E7-05.
- **Provenance: the weakest link.** A chain is as reliable as its least reliable step; a two-step path with one inferred step does not present better than the inferred one (I-39). The order `Declared > Imported > Algorithmic > Inferred` is that of the `ProvenanceKind` enum and follows the idea of ADR-0014 (the explicit assertion before the derived one; the numeric confidence exists only in `Inferred`); ADR-0014 does not fix a confidence scale, so this is a choice of this package, not measured.
- **Recency.** It takes the most recent of the unit's head revision (`Unit::updated_seq`, read from the snapshot) and the relation that reached it (`created_seq` of the last step). It is a commit sequence, never a clock (I-26). `H` is the age at which half the weight remains; the function is monotonic, bounded in `[0, w]` and uses only integers. An age above `u32::MAX` weighs as `u32::MAX` (the contribution is already 0 there).
- **A unit without a vector** contributes 0, as does one with cosine 0; the diagnostic output distinguishes them (`similarity: None` against `Some(0)`).
- **A result from another snapshot** (a unit that is no longer active, or whose head revision is not that of the result) is rejected with `RankError::NotFromThisSnapshot`, which does not name the unit. With `Off` nothing is read and there is no check.
- **Bound.** The ranking reorders what the query returned. A result that the query cut (`LimitReached`, `BudgetExhausted`) is not seen: ranking before the cut would require changing the traversal of `assoc.rs` (§10).

## 4. Weights and why

The constants are public and `RankWeights::default()` uses them. A weight of 0 switches a signal off and a negative one inverts it (the I-41 test uses random weights, including negative ones).

| Constant | Value | Reason |
|---|---|---|
| `WEIGHT_HOPS` | 1 000 | It is the unit in which the others are measured. Graph proximity is the structural evidence of association and the only signal every result has without anyone supplying it |
| `WEIGHT_PROVENANCE` | 250 per step (0 to 750) | 3 · 250 < 1 000: provenance alone never overturns a hop |
| `WEIGHT_TYPE` | 500 | Below one hop: the caller's taste for a type refines, it does not outrank proximity |
| `WEIGHT_RECENCY` | 400 | The weakest: something touched recently is not for that reason more related; it only breaks ties between close contenders |
| `WEIGHT_SIMILARITY` | 1 500 | Between one and two hops: a strong semantic match on its own can beat one hop of distance, never two |
| `RECENCY_HALF_LIFE` | 64 commits | A scale of tens of commits. The exact value has no measurement behind it: it is a choice to review in E7-05 |

The relations between the weights are the justification of the values, and that is why they are fixed at compile time (`const _: () = assert!(…)`): no non-hop signal on its own beats one hop (`3·p < h`, `t < h`, `r < h`); the two strongest added together do (`3·p + t > h`); similarity beats one and not two (`h < s < 2h`). **The concrete values are not tuned**: they were not measured against the frozen set nor against any other. If E7-05 changes a value, it changes the constant and this table in the same change, saying which measurement justifies it.

## 5. Determinism

- Everything is integer except the vector similarity (§6). No order compares an `f32`/`f64`.
- The order is total: `(score descending, unit identifier ascending)`. A tie in score is resolved by the identifier.
- Nothing depends on a hash map (`BTreeSet` and `BTreeMap` are used), on the clock, on the order in which the caller passed the vectors, nor on the path that produced the result: by I-32 the result is the same through the reference, the index and the planner, and the test checks it over the full ranking, contributions included.
- Sums cannot overflow: the weights are `i32`, each contribution is a weight times a small factor and `i64` is more than enough.

## 6. Vectors (ADR-0011)

The caller supplies the vectors; they live in memory and are not written anywhere. The module depends on no network and no model (I-31) and adds no dependencies (`Cargo.toml` does not change).

| Type | Content |
|---|---|
| `VectorSet` | One dimension (greater than 0), a metric and the vectors per unit. `insert` checks each vector; `search(query, k)` is the exact brute-force search; `similarity(unit, query)` gives that of one unit |
| `VectorQuery` | The metric and the query vector, checked at construction |
| `VectorMetric` | `Cosine` or `Dot` |
| `VectorError` | `ZeroDimension`, `Empty`, `DimensionMismatch { expected, got }`, `MetricMismatch { set, query }`, `NonFinite { index }`, `ZeroVector` |

Rejections, all before sorting anything and in the three modes:

| Case | Error | Where |
|---|---|---|
| Set of dimension 0 | `ZeroDimension` | `VectorSet::new` |
| Empty vector | `Empty` | `insert`, `VectorQuery::new` |
| Dimension different from the set's | `DimensionMismatch` | `insert`, `VectorSet::check` (query) |
| Query metric different from the set's | `MetricMismatch` | `VectorSet::check` |
| NaN or infinite component (with its position) | `NonFinite` | `insert`, `VectorQuery::new` |
| Zero-norm vector with cosine (`0.0` and `-0.0` are) | `ZeroVector` | `insert`, `VectorQuery::new` |

Check order: empty, dimension, non-finite, zero; and in the comparison, metric and then dimension. A rejected vector is not stored. With dot product a zero vector is valid. No error carries a value of a vector, only positions and dimensions.

### How the vector signal is made stable

It is the only operation with floating point. It is made stable as follows, and it is stated here so that it can be discussed:

1. Every input is `f32`, which `f64` represents without error, and the products of two `f32` are exact in `f64`.
2. The components are traversed in index order with one accumulator per magnitude (`dot`, `‖a‖²`, `‖b‖²`), and only `+`, `*`, `/` and `sqrt` are used, which IEEE-754 defines as correctly rounded. Rust does not reassociate or fuse (FMA) floating-point operations, and no function of `libm` (`exp`, `ln`, `pow`, `acos`) is called. Therefore the same vector gives the same bits on any IEEE-754 platform that Rust supports, in any run.
3. The result is **rounded to fixed point** (`round`, half away from zero) to six decimals before any other figure is touched. The score contains no float and no order compares one. An `f64` outside the range of `i64` saturates; a cosine that rounding leaves one unit in the last place above 1.0 returns to 1 000 000 through the same rounding (that is why there is no `clamp` when computing it; the `clamp` is applied when converting into a contribution).

Honest limit: two platforms that differed in an `f64` exactly at a rounding boundary of the sixth digit would give a one-unit difference in the similarity. The design avoids it by construction (point 2) but does not prove it with another platform: it was tested only on this one. A test on this tree would not detect it.

### Dot product and the score

The dot product is not bounded. The exact similarity is reported without clipping (`similarity` in the diagnostic, `search`), but the contribution to the score is clipped to `[−1, 1]` before multiplying by `WEIGHT_SIMILARITY`: **above 1.0 the dot product stops distinguishing** units in the score. Whoever needs resolution with large magnitudes must normalise the scale of their vectors or use `VectorSet::search` directly. No scale parameter was added because nobody asked for one.

## 7. Tests

`tests/assoc_rank.rs` has 10 functions: 9 run over `Emulated<LogEngine>` and `KernoDb` (18 tests) and the similarity one, which touches no engine, once: **19 tests**. The frozen set is not used.

| Test | What it shows |
|---|---|
| `contributions_add_up_to_the_score_and_match_an_oracle` (I-41) | 8 seeds, 2 metrics, 25 queries each over graphs of 8 to 14 units, random weights (also negative), sets of preferred types and vectors over most units. For each result: the five contributions are those of an oracle that starts from `get_relation` and `get_unit` (not from the trace), the score is their sum, the order is `(score, id)`, and `On` sorts like `Diagnostic` without showing contributions. It requires more than 400 ordered results, more than 100 with a vector and more than 20 with some negative contribution |
| `hand_vectors_without_a_vector_signal` | A world of five units with each figure computed by hand with the default weights (§7.1): 5 626, 4 650, 4 382 and 3 643, and the order `b, d, c, e` against the reference order `b, c, d, e` |
| `hand_vectors_with_cosine_and_dot` | The same with cosine (`+1500`, `0`, `−1500`, and the unit without a vector unchanged) and with dot product (0.25 stays, 2.0 is clipped to 1.0 and reported as 2 000 000). It fixes the default values of the weights |
| `same_state_and_query_give_the_same_order` | 6 worlds, 30 queries each: the full ranking, contributions included, is the same for the result of the reference path, the index path and the planner, repeated, and from a second snapshot of the same state |
| `ties_are_broken_by_identifier` | Four leaves that tie on all signals come out by ascending identifier; with all weights at 0, everything ties and the order is the pure identifier, different from the reference (which puts the nearest first) |
| `ranking_off_is_the_reference_order` | 5 worlds, 40 queries: with default options, the order is that of the reference path and that of the index path, without score; `Off` is the default mode; the reference order is `(hops, id)`; with `On` the order changes in more than 10 cases (the test is not empty) |
| `a_result_from_another_state_is_refused_unless_ranking_is_off` | After modifying a unit, the old result is refused in `On` and `Diagnostic` and accepted in `Off`; over its own snapshot it is accepted |
| `every_vector_rejection` | All the cases of §6, with both metrics; a rejected vector is not stored; the metric-dimension order; and that `rank_assoc` rejects in the three modes, also with a result with no results |
| `similarity_matches_an_independent_oracle` | Eighth-valued vectors (`k/8`, exact in `f32` and `f64`), 5 dimensions, 2 metrics, 6 queries each (more than 1 000 comparisons). For the dot product the oracle is exact: `15 625 · D`. For cosine, an integer check that `m` is within half a unit of `10⁶ · D / √(‖a‖²‖b‖²)`, without floats. The search compares all vectors once, sorts `(similarity desc, id asc)` and `k` gives a prefix. Hand values (1.0; 0.6; 0; 50 000 000) and saturation with `f32::MAX` |
| `sentinels_no_literal_outside_the_diagnostic_output` (I-37) | A relation type and sentinel components. Neither the `Debug` of `VectorSet`, `VectorQuery`, `RankOptions`, nor the output with `Off` and `On`, nor any of the errors (`Display` and `Debug`: seven of vectors, one of dimension in `rank_assoc`, another `NotFromThisSnapshot`) contain them, nor do they name a unit; the diagnostic output, which carries only integers, does not either. Controls: the result does contain the type, and the output contains `RankedHit` and `Contributions`. The ranking leaves no record in the observer at `diagnostic` level |

### 7.1 The vectors by hand

World: units `a b c d e` (seq 1); `a –cites→ b` Declared (seq 2); `a –mentions→ c` Inferred (seq 3); `b –cites→ d` Declared (seq 4); `c –cites→ e` Imported (seq 5); `d` is updated (seq 6). Query from `a`, outgoing, depth 2; preferred type `cites`; the snapshot is at seq 6. Default weights.

| Unit | Hops | Type | Recency | Provenance | Total |
|---|---|---|---|---|---|
| `b` | 4 000 | 500 (1 of 1) | 376 (`400·64/(64+4)`, most recent seq 2) | 750 (Declared) | **5 626** |
| `d` | 3 000 | 500 (2 of 2) | 400 (seq 6, age 0) | 750 (Declared, Declared) | **4 650** |
| `c` | 4 000 | 0 (`mentions`) | 382 (`400·64/(64+3)`, seq 3) | 0 (Inferred) | **4 382** |
| `e` | 3 000 | 250 (1 of 2) | 393 (`400·64/(64+1)`, seq 5) | 0 (Inferred is the weakest) | **3 643** |

Each entry goes through integer division: `25 600 / 68 = 376.5…` → 376.

## 8. Mutations

Each mutation was applied to unmutated `assoc_rank.rs` with a script that substitutes a text, runs `cargo test -p kerno-units --release --test assoc_rank --no-fail-fast`, notes the red tests and restores the file (checked with `diff` against a copy). All 42 compiled except a first attempt at M34 with an incomplete change, which was redone; all 42 were detected. A mutation that survived did not appear, but an earlier review showed that the `clamp(-1, 1)` of the cosine was redundant with the fixed-point rounding (§6) and it was removed instead of being left unguarded. «Test» abbreviates the function name; all fail in their two engines except `similarity_matches_an_independent_oracle`.

| # | Mutation | Tests that failed |
|---|---|---|
| M01 | The tie-break becomes identifier descending | `contributions_…`, `ties_are_broken_by_identifier` |
| M02 | The score sorts from lowest to highest | `contributions_…`, `hand_vectors_without_…`, `hand_vectors_with_…` |
| M03 | `Off` sorts by score | `ranking_off_is_the_reference_order`, `a_result_from_another_state_…` |
| M04 | The default mode is `On` | the same two |
| M05 | Hops score more the further away | `contributions_…`, the two hand-vector ones |
| M06 | The type is not divided by the number of steps | the same three |
| M07 | Provenance takes the strongest link | the same three |
| M08 | `Imported` is trusted as `Declared` | `contributions_…` |
| M09 | Recency ignores the relation that reached the unit | the same three |
| M10 | Recency ignores the age | the same three |
| M11 | Similarity is not clipped to `[−1, 1]` | `contributions_…`, `hand_vectors_with_…` |
| M12 | The sum omits the similarity | the same two |
| M13 | The score is the sum plus 1 | `contributions_…`, the two hand-vector ones, `ties_…` |
| M14 | The diagnostic does not show the contributions | `contributions_…`, the two by hand, `sentinels_…` |
| M15 | The vectors are not checked | `every_vector_rejection` |
| M16 | `Off` does not check the vectors | `every_vector_rejection` |
| M17 | The dimension is checked before the metric | `every_vector_rejection` |
| M18 | The metric is not checked | `every_vector_rejection`, `sentinels_…` |
| M19 | A zero vector is accepted with cosine | `every_vector_rejection`, `sentinels_…` |
| M20 | An infinity is accepted | `every_vector_rejection`, `sentinels_…` |
| M21 | `insert` does not check the dimension | `every_vector_rejection`, `sentinels_…` |
| M22 | An empty query vector is accepted | `every_vector_rejection` |
| M23 | `insert` checks the dimension before emptiness | `every_vector_rejection` |
| M24 | A set of dimension 0 is accepted | `every_vector_rejection`, `sentinels_…` |
| M25 | Cosine divides by only one norm | `hand_vectors_with_…`, `similarity_matches_an_independent_oracle` |
| M26 | Similarity is truncated instead of rounded | `similarity_matches_an_independent_oracle` |
| M27 | The search sorts from least to most similar | `similarity_matches_an_independent_oracle` |
| M28 | The search tie goes by identifier descending | `similarity_matches_an_independent_oracle` |
| M29 | The search ignores `k` | `similarity_matches_an_independent_oracle` |
| M30 | Any result is accepted as from this snapshot | `a_result_from_another_state_…`, `sentinels_…` |
| M31 | The sequence of the head revision is 0 | `contributions_…`, the two by hand |
| M32 | The `Debug` of `VectorSet` prints the components | `sentinels_…` |
| M33 | The `Debug` of `VectorQuery` prints the components | `sentinels_…` |
| M34 | The `Debug` of `RankOptions` prints the types | `sentinels_…` |
| M35 | The dot product is computed as cosine | `hand_vectors_with_…`, `similarity_matches_an_independent_oracle` |
| M36 | `WEIGHT_HOPS` goes from 1 000 to 1 100 | the two hand-vector ones |
| M37 | `RECENCY_HALF_LIFE` goes from 64 to 32 | `contributions_…`, the two by hand |
| M38 | A unit without a vector counts as similarity −1 | `contributions_…`, the two by hand |
| M39 | The type signal is inverted | `contributions_…`, the two by hand |
| M40 | Steps are counted by `distance + 1` and not by the path | `contributions_…`, the two by hand |
| M41 | Recency adds a clock bit | `contributions_…`, the two by hand, `same_state_and_query_give_the_same_order`, `ties_…` |
| M42 | Ties are left in the inverted input order, without identifier | `contributions_…`, the two by hand, `ties_…` |

M30 also makes `sentinels_…` fail because that test checks the refusal of a result from another snapshot. The tests were edited after the first round (the numeric sentinel and the similarity function without an engine); M26, M28, M29, M32, M33, M34 and M35 were run again with the final file and are still detected.

## 9. Verification

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p kerno-testkit -p kerno-units --all-features --release
cargo test -p kerno-units --release --test assoc_rank
```

Test count of `kerno-testkit` + `kerno-units` with `--release --all-features`: **630 passed, 0 failed, 8 ignored** with this change; before it was 611 (E7-02). The 19 new ones are those of `assoc_rank`; the runner counts them (`assoc_rank`: `19 passed`).

## 10. Limits, and what would have been added to `assoc.rs`

- **Recency needs a datum that `AssocHit` does not carry.** `AssocHit` carries `revision` (`RevNo`) but not the `commit_seq` of that revision. It is read from the snapshot through the public API: `reference_get(hit.unit)` and `Unit::updated_seq`, by the result's identifier (one read per result, with payload verification). What would have been added to `assoc.rs`: a field `head_seq: CommitSeq` in `AssocHit`, which the traversal already has in hand in `traverse` (`reference_get(there)` returns the unit); it would avoid one read per result, and the `NotFromThisSnapshot` check would be unnecessary for this datum. The trace (`TraceStep`) already carries type, `created_seq` and provenance class, and nothing more was needed.
- **The cut happens before the ranking.** `limit` and `BudgetExhausted` cut by «hops, then identifier»; the ranking only reorders what remained. If E7-05 showed that the relevant ones lie beyond the cut, one would have to rank inside the traversal (a change to `assoc.rs`) or ask for a larger limit.
- **The weights are not measured.** They are a structural justification (§4), not a result. The type signal does not exist without a set of preferred types and there is no default one.
- **Dot product** saturated to ±1.0 in the score (§6).
- **Stability across platforms** by construction, not verified on another platform (§6).
- **Cost.** One unit read per result and, with vectors, one comparison per result; the exact search is linear in the size of the set. Nothing is claimed about performance (E7-05).
- **No observations.** `rank_assoc` does not report to the observer: it is not a planner execution. If E7-05 or E7-06 wanted to count it, it would be a new observation and would be subject to I-37.
- **Dependencies.** None new; `Cargo.toml` does not change.

## 11. Related documents

- [Stage 7 minutes](stage7-record.md) §3 (E7-04), §4 (I-41, I-37), §7 point 3
- [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md)
- [Associative query (E7-01)](stage7-associative-query.md) · [indexed execution (E7-02)](stage7-indexed-execution.md)
- [ADR-0014](../adr/0014-physical-representation-of-relations.md): provenance of relations
- [Invariants](invariants.md): I-26 (the clock does not order), I-32, I-37, I-39
