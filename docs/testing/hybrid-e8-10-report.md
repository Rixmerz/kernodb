# Hybrid retrieval on the frozen set E8-10: one pre-registered measurement

## 1. Status

**criterion Q4 on E8-10: not demonstrated.**

The primary arm (HNSW vectors fused with the graph by weighted RRF) fails Q4 as written in one
of its five readings: the `temporal` class loses 0.0924 nDCG@10 against the graph-only arm
(bound -0.01). Overall it is inside the bound (-0.0051). Nothing was retuned after the run; the
thresholds and the frozen set are untouched.

## 2. What was measured

* The set: `crates/kerno-testkit/data/derived-eval-2` (E9-19), 2000 units, 5062 relations, 300
  change sets, 100 queries (25 per class), every query with a query vector. The MANIFEST is
  checked before anything runs (T-E10-R) and matched:

  | file | SHA-256 |
  |---|---|
  | `corpus.txt` | `4e599e109656f32a1d63c6b720c47942d23a11a28808c4bca3af275a9ae84c34` |
  | `vectors.txt` | `ab84850aeaa94cf57ebefbd7e6b9626b79c41a308693c29508eb1e7547fab25a` |
  | `queries.txt` | `aea4e1208266c1fefb6cfb8e84718a6748488493cad44ad22f632e25afb25d84` |

* The code: commit `16ed9f2` (the line `commit:` of the output file). The harness is
  `crates/kerno-units/tests/hybrid_measure.rs`; the query is `UnitSnapshot::hybrid_search`
  (`crates/kerno-units/src/hybrid.rs`), exposed by `kerno::Db::hybrid_search`.
* The output: `docs/testing/hybrid-e8-10-output.txt`, from one run on both engines
  (`Emulated<LogEngine>` and `KernoDb`, over a seeded `SimVfs`). The two engines print
  identical reports (154 lines each).

## 3. Decisions fixed before the run

These are the contents of the header of `hybrid_measure.rs`, written before the first run.

* **Parameters** are the library defaults, not chosen from E8-10: RRF `k = 60`, weights 1/1/1,
  100 candidates per leg, cut 10. HNSW: `AnnConfig::new(16, Cosine)` (M 16, ef_construction 200,
  ef_search 64, seed 0). Fusion is a union with integer fixed-point scores; ties by unit id.
  When the graph leg runs, its seeds are removed from the other legs ("the seeds are not hits").
* **Arms**, all 100 queries, cut 10:
  * `graph`: graph leg only. It is the "without the vector signal" arm of Q4 as written.
  * `ann`: HNSW leg only. `exact`: exact `VectorSet` leg only, the arm "without signal" of
    T-E10-S1 and S2.
  * `fused`: HNSW + graph. **Primary.**
  * `ranked`: `rank_assoc` with `RankMode::On`, default weights, no vectors (second reading of
    S1 and S2, as in E9-20). `brute`: independent brute-force cosine order (reference of Q1).
  * `keyword`, `fused_kg` (text + graph), `fused_all` (text + HNSW + graph). **Secondary,
    informational.**
* **Verdicts on the primary arm:** Q4 as written: `fused - graph >= -0.01` in nDCG@10, overall
  and in each of the 4 classes. T-E10-S1: `fused - exact >= 0.02`. T-E10-S2: `fused - exact >=
  -0.01` per class. T-E10-Q2: fused nDCG@10 >= 0.80. T-E10-Q3: fused recall@10 >= 0.90.
  T-E10-Q5 over {graph, ann, exact, fused, ranked, brute} x 5 scopes. T-E10-V1, V2 (exact <=
  0.96), Q1 and R.
* **Headline rule:** "demonstrated" if and only if Q4 as written (all five), S1, S2, V1, V2, Q1,
  Q5 and R hold. Q2 and Q3 are printed as the floors of acta 9 §5 and are not in the headline.
* **One recorded run.** Thresholds are printed and not asserted. Unjudged units in a top 10 are
  excluded from the list before the metrics and counted (as in E9-20).

**Known limitation of the keyword leg.** The frozen queries carry no text
(`crates/kerno-testkit/src/derived_eval.rs:260`, struct `Query`: seed, direction, kinds, depth,
window, provenance, vector, judgments). The keyword leg is therefore evaluated only through a
declared derivation rule: the query text is the label of the seed unit (for example
`s0000.d0`). That is close to a same-session oracle, so every arm that uses it is secondary and
informational and none of it enters the headline.

## 4. Primary results

Bound, value and verdict, copied from the output.

| Criterion | Bound | Value | Verdict |
|---|---|---|---|
| Q4 as written, all-100 (`fused - graph`) | >= -0.01 | -0.0051 | met |
| Q4, vector_neighbourhood | >= -0.01 | -0.0029 | met |
| Q4, temporal | >= -0.01 | **-0.0924** | **not met** |
| Q4, provenance | >= -0.01 | +0.0440 | met |
| Q4, combined | >= -0.01 | +0.0308 | met |
| T-E10-S1 (`fused - exact`, overall) | >= +0.02 | +0.4748 | met |
| T-E10-S2, vector_neighbourhood | >= -0.01 | +0.2952 | met |
| T-E10-S2, temporal | >= -0.01 | +0.5408 | met |
| T-E10-S2, provenance | >= -0.01 | +0.5284 | met |
| T-E10-S2, combined | >= -0.01 | +0.5346 | met |
| T-E10-Q2 (fused nDCG@10) | >= 0.80 | 0.8963 | met (floor, not in headline) |
| T-E10-Q3 (fused recall@10) | >= 0.90 | 0.8874 | not met (floor, not in headline) |
| T-E10-Q5 (6 arms x 5 scopes) | <= 10 % unjudged | worst 1.6 % (`ranked`, combined) | met |
| T-E10-V1 | >= 25 per class | 25, 25, 25, 25 | met |
| T-E10-V2 (exact nDCG@10) | <= 0.96 | 0.4216 | met |
| T-E10-Q1 (exact against brute force) | recall 1.000 | smallest 1.000 | met |
| T-E10-R (MANIFEST) | match | match | met |

Second reading of S1 and S2 (without signal = the `ranked` arm, informational): S1 -0.0124 (not
met), S2 vector_neighbourhood -0.0029 (met), temporal -0.0816 (not met), provenance +0.0804
(met), combined -0.0455 (not met).

Anchors that must agree for the figures to be trusted, all asserted by the test: the `graph`
arm equals the reference order cut at 10 for every query; the `exact` arm's nDCG@10 equals the
testkit's independent `arm_without_signal` within 1e-12 (0.4216); both engines print the same
report. HNSW recall@10 against brute force: smallest 1.000, mean 1.000, identical lists in 100
of 100 queries; the seed unit is in the HNSW top 10 for 96 of 100 queries.

## 5. Means per arm (nDCG@10; recall@10 is in the output file)

| Arm | all-100 | vector_neighbourhood | temporal | provenance | combined |
|---|---|---|---|---|---|
| graph | 0.9014 | 1.0000 | 0.9815 | 0.7518 | 0.8726 |
| ann = exact = brute | 0.4216 | 0.7018 | 0.3483 | 0.2674 | 0.3687 |
| **fused** | 0.8963 | 0.9971 | 0.8891 | 0.7958 | 0.9034 |
| ranked | 0.9087 | 1.0000 | 0.9707 | 0.7154 | 0.9488 |
| keyword (secondary) | 0.2521 | 0.2386 | 0.3038 | 0.1496 | 0.3163 |
| fused_kg (secondary) | 0.8807 | 0.9654 | 0.8706 | 0.8118 | 0.8751 |
| fused_all (secondary) | 0.9273 | 0.9696 | 0.9352 | 0.8361 | 0.9684 |

The graph, exact and ranked figures reproduce E9-20 (`simple` 0.9014, `ranked` 0.9087,
`vector` 0.4216).

## 6. Secondary (keyword) results

Informational, through the seed-label derivation rule of §3.

* Q4 as written, `fused_all - fused_kg`: all-100 +0.0466, vector_neighbourhood +0.0042,
  temporal +0.0646, provenance +0.0243, combined +0.0933 (all met).
* S1 `fused_all - exact` +0.5058; S2 per class +0.2678, +0.5869, +0.5687, +0.5997 (all met).
* Q2 0.9273 (met), Q3 0.9561 (met).
* **Q5 not met** for the keyword arms: 72.7 % of the `keyword` positions, 40.5 % of `fused_kg`
  and 27.1 % of `fused_all` are unjudged. The judgments grade what the walk reaches and the
  vector neighbours; same-session units the walk does not reach are unjudged. By the rule of
  acta 8 Q5 ("if it fails, it is not measured: more is judged"), these arms' nDCG figures are
  not valid measurements. They are printed and not interpreted further.

## 7. Reading

What the output supports, and no more.

* The headline is "not demonstrated" because of one reading, Q4 in the `temporal` class. Overall
  the fused arm is 0.0051 below graph-only.
* Against the exact vector arm (the reading S1 and S2 were written for), the fused arm is far
  above: +0.4748 overall and in every class. Against the arms without vectors (`graph` for Q4, `ranked` in the second reading), it is not above overall.
* The classes move in opposite directions against `graph`: provenance +0.0440 and combined
  +0.0308 gain; temporal -0.0924 loses; vector_neighbourhood is flat (-0.0029).
* In `temporal`, the graph arm's recall@10 is 0.9451 and the fused arm's 0.8313: relevant units
  the graph leg finds are not in the fused top 10. The graph arm fills only 706 of 1000 top-10
  positions over the 100 queries (many walks find fewer than 10 units); the fused arm fills all
  1000. The judgments grade the vector neighbours that no link reaches as 0
  (`crates/kerno-testkit/src/derived_eval.rs:14-15`), so a union fusion that
  places such neighbours among the walk's hits is penalised by how the set is graded. This
  explanation fits the figures; the run does not isolate it.
* HNSW is not a source of the gap: its top-10 lists equal brute force in 100 of 100 queries.
* The keyword results of §6 say nothing about real text: the derivation rule is near an
  oracle and the arms fail Q5.

A graph-anchored mode (the existing `rank_assoc` with `VectorSignal` restricts to the walk's
hits) or other weights might close the `temporal` gap. Neither can be tried on E8-10 without
tuning on the set; see §8.

## 8. Limits

* The set is synthetic: 2000 units, seeded generators, no real workload.
* The text of a unit is its label (`s0000.d0`); BM25 has no real language to rank.
* The vectors are seeded integers, not the output of an embedding model.
* The judgments have a single author.
* One machine, one run. The re-run produced an output file that differs from the recorded one
  only in the `commit:` line and the `finished in N s` line of the test harness.
* Thresholds are printed and not asserted; the frozen set and thresholds were not changed.
* **E8-10 has now seen the hybrid query.** Any later fusion mode, weights or parameters must
  be measured on a new frozen set; tuning on E8-10 would void its use as a held-out set.

## 9. How to reproduce

```text
cargo test -p kerno-units --all-features --release --test hybrid_measure -- --nocapture --exact hybrid_quality_over_the_second_frozen_set
```

About 8 seconds once built. The output is deterministic except for the timing line.

## 10. Mutation table

Each mutation was applied alone to a scratch copy of the tree (`/tmp/kernodb-mut`, separate
target directory) and reverted. Every one turned the named test red.

| N | File | Mutation | Failing test(s) |
|---|---|---|---|
| M1 | `hybrid.rs` | `RRF_SCALE / (k + rank) * weight` (division before the multiplication) | `the_weight_multiplies_before_the_division` |
| M2 | `hybrid.rs` | tie-break `a.unit.cmp(&b.unit)` -> `b.unit.cmp(&a.unit)` | `ties_are_broken_by_unit_id` |
| M3a | `hybrid.rs` | keyword-leg seed filter `!seeds.contains(u)` -> always true | `graph_seeds_are_not_hits` |
| M3b | `hybrid.rs` | vector-leg seed filter `seeds.contains(&unit)` -> `false` | `graph_seeds_are_not_hits`, `fused_scores_are_the_weighted_rrf_of_the_leg_ranks` |
| M4 | `hybrid.rs` | `lifecycle == Active` -> `!= Deleted` | `vector_hits_of_inactive_units_are_dropped` |
| M5 | `hybrid.rs` | `weights.vector > 0` -> `>= 0` | `a_leg_of_weight_zero_is_not_run` |
| M6 | `hybrid_measure.rs` | Q4 `gain >= Q4_MIN_GAIN` -> `gain > Q4_MIN_GAIN` | `every_verdict_flips_at_its_own_bound` |
| M7 | `hybrid_measure.rs` | `.filter(\|u\| judged.contains(u))` -> `.filter(\|_\| true)` | `unjudged_units_are_excluded_and_counted` |
| M8 | `kerno/src/lib.rs` | `hybrid_search` body -> `Ok(Vec::new())` | `hybrid_search_fuses_text_hnsw_and_graph`, `hybrid_search_refuses_an_invalid_query` |

Printed `FAILED` lines, verbatim: `test the_weight_multiplies_before_the_division ... FAILED`,
`test ties_are_broken_by_unit_id ... FAILED`, `test graph_seeds_are_not_hits ... FAILED`,
`test vector_hits_of_inactive_units_are_dropped ... FAILED`,
`test a_leg_of_weight_zero_is_not_run ... FAILED`,
`test every_verdict_flips_at_its_own_bound ... FAILED`,
`test unjudged_units_are_excluded_and_counted ... FAILED`,
`test hybrid_search_fuses_text_hnsw_and_graph ... FAILED`.
