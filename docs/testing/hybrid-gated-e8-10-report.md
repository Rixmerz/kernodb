# Graph-gated hybrid retrieval on the frozen set E8-10: a second, pre-registered measurement

## 1. Status

**criterion Q4 on E8-10: demonstrated.** One run of the graph-gated arm (HNSW vectors and the graph
under `Fusion::GraphGated`): Q4 as written holds overall and in each of the 4 classes, the
`temporal` class at +0.0108 against the graph-only arm (bound -0.01; #2378's union had -0.0924).
Every other verdict of the headline holds too. The result is weaker evidence than it looks:
read section 8.5 before quoting it.

Sections 2 to 7 were written and committed (`391342b`) before the measurement ran and are
unchanged; only this status and section 8 were written after it. The earlier text of this
section was "Pre-registered. Not measured yet."

## 2. What this is, and what it is not

* Task #2378 measured hybrid retrieval (weighted RRF over HNSW vectors and the graph) once on
  E8-10 and found criterion Q4 **not demonstrated**: the `temporal` class lost 0.0924 nDCG@10
  against the graph-only arm (bound -0.01). Its report (`docs/testing/hybrid-e8-10-report.md`),
  its output and its harness (`tests/hybrid_measure.rs`) are untouched. This is a new attempt
  with its own report, harness and output.
* **It is not a held-out measurement.** #2378 §8 says E8-10 has seen the hybrid query and that
  any later fusion mode must be measured on a new frozen set. This attempt was asked for on E8-10
  anyway, and the method below was chosen after reading #2378's per-class figures. What limits the
  damage: the method was developed and iterated on three fresh seeds of the same generator, never
  on E8-10; its parameters are the library defaults and nothing is tuned per query or per class;
  the frozen files, the queries and every threshold are unchanged; the run happens once. A pass
  here is weaker evidence than a pass on a new frozen set would be, and the report says so again
  in section 8.
* The frozen set is `crates/kerno-testkit/data/derived-eval-2` (E9-19), checked against its
  MANIFEST before anything runs (T-E10-R):

  | file | SHA-256 |
  |---|---|
  | `corpus.txt` | `4e599e109656f32a1d63c6b720c47942d23a11a28808c4bca3af275a9ae84c34` |
  | `vectors.txt` | `ab84850aeaa94cf57ebefbd7e6b9626b79c41a308693c29508eb1e7547fab25a` |
  | `queries.txt` | `aea4e1208266c1fefb6cfb8e84718a6748488493cad44ad22f632e25afb25d84` |

## 3. Why the temporal class failed

Facts, from the generator and the judgments (`crates/kerno-testkit/src/derived_eval.rs`,
`derived_eval2.rs`):

* **Relevance is reachability.** A unit has grade 3, 2 or 1 only if the walk reaches it through
  true links at depth 1, 2 or 3 (`walk`, `grade_at`), under the query's own filters: relation
  kinds, provenance classes and, for `temporal` and `combined`, a window of link creation steps
  (`window`: the sessions within 2, or 1, of the seed's). A unit the walk does not reach is either
  unjudged or grade 0. The ten units nearest to the query vector that the walk did not judge are
  grade 0 on purpose (`judge_neighbours`, `with_vectors`).
* **The vector leg cannot see those filters.** The query vector sits near the seed unit's vector
  (noise 150 per component). Its neighbours are the seed's topic: decisions of the same chain in
  every session, whether or not their links fall inside the window, and the unit's session
  mates. Those that the filtered walk does not reach are grade 0.
* **The union lets them in.** Under weighted RRF with equal weights a vector rank 1 earns 16 393 442
  and a graph rank 5 earns 15 384 615, so vector-only units interleave with the walk's hits and
  push walk hits out of the top 10. #2378 measured exactly that in `temporal`: graph recall@10
  0.9451 against 0.8313 fused, with the graph arm filling only 706 of 1000 top-10 positions.
  #2378 §7 called this explanation a fit to the figures, not an isolated cause.
* **So no vector-only unit can be relevant here, and every one that enters the top 10 can only
  cost.** E8-10 penalises union-style fusion by construction (a note this project keeps). What a
  vector signal can still earn on this set is a better order *among the walk's own hits*, for
  instance pushing the far end of a false link (grade 0, a decision of another topic) below true
  neighbours of the seed's topic. #2378's gains in `provenance` (+0.0440) and `combined` (+0.0308)
  have that shape.

Evidence that this is structural and not a property of one seed: on three fresh seeds the
union (`rrf`) loses -0.0859, -0.0833 and -0.1004 in `temporal` (section 5).

Options considered, and why not:

| Option | Why not |
|---|---|
| Min-max or z-score combination instead of rank fusion | Still a union: vector-only units still compete with the walk's hits, only with other scores. It would also put floats into a fusion that is integer today. |
| A recency or window-aware vector leg (filter vector hits by creation step) | Needs the unit's creation step to line up with the link steps, which is a property of this generator; other classes filter by provenance and kind, which it would not touch; unlinked same-window units would still enter. |
| A recency leg | E8-10 has no recency notion in its judgments: it would be a signal invented for the occasion. |
| Different RRF weights or `k` | #2378 §7: that is tuning on the set. |
| **Gate the legs on the graph** | Removes the displacement at its source and keeps the other legs' vote on order. **Chosen.** |

## 4. The method

`Fusion::GraphGated` (`crates/kerno-units/src/hybrid.rs`, the field `HybridQuery::fusion`;
`Fusion::Rrf`, the union of #2378, stays the default and is unchanged):

1. The scores and the per-leg ranks are those of weighted RRF, untouched: a unit's score is the
   sum over the legs that hold it.
2. **Gate.** When the graph leg ran, every unit it found ranks above every unit it did not.
3. **Hop tier.** Among the walk's hits, a unit one hop from the seeds ranks above a unit two hops
   away, whatever the other legs say.
4. Inside one hop, and among the units the walk did not find, the order is (score descending,
   unit id ascending): the vector and keyword legs refine each hop, and the units no link
   reaches follow, in union order.
5. Without a graph leg, or when the walk finds nothing, no unit has a hop and the two modes
   return the same list.

Parameters, the library defaults and not chosen from E8-10: RRF `k = 60`, weights 1/1/1, 100
candidates per leg, cut 10; HNSW `AnnConfig::new(16, Cosine)` (M 16, ef_construction 200,
ef_search 64, seed 0).

**How the hop tier got there.** The first version had the gate only (step 2, then score). On
the dev seeds it removed most of the loss and not all: `temporal` -0.0353 and -0.0371 on seeds
7001 and 7002 against -0.0859 and -0.0833 for the union, still outside the bound. The score
alone lets the vector leg promote a unit three hops away over a depth-1 neighbour, and the
judgments grade by depth. Adding step 3 (graph distance is the graph's own relevance order; the
other legs refine within a distance) gave the figures of section 5. That is the only
iteration; no weight, `k` or candidate count was varied. The first version's code is not kept;
its two printed rows are in section 5.

**What the design assumes, and what that limits.** It treats the graph as the authority on
which units lead and how near they are. That is what E8-10 grades, so Q4 on this set can only
say that *adding the vector signal does not hurt when the graph carries the relevance*. It
cannot say that the vector signal helps in general, and it says nothing about a query whose
relevance lives in the vectors.

## 5. Development evidence (fresh seeds, not E8-10)

Three seeds of the same generator, `7001`, `7002`, `7003` (E8-10 is `20261203`), built the way
`derived_eval2::generate` builds E8-10 (`dev_set` in the harness). Not frozen, not a measurement
of record. Output: `docs/testing/hybrid-gated-dev-output.txt`
(`cargo test -p kerno-units --all-features --release --test hybrid_gated_measure -- --nocapture --exact gated_fusion_on_fresh_seeds`).
nDCG@10 gain over the graph-only arm:

| seed | arm | all-100 | vector_neighbourhood | temporal | provenance | combined |
|---|---|---|---|---|---|---|
| 7001 | rrf (union) | +0.0155 | -0.0015 | **-0.0859** | +0.1079 | +0.0416 |
| 7001 | gate only (first version) | +0.0481 | -0.0015 | **-0.0353** | +0.1354 | +0.0940 |
| 7001 | **gated** | +0.0676 | +0.0000 | +0.0147 | +0.1495 | +0.1064 |
| 7002 | rrf (union) | -0.0051 | -0.0031 | **-0.0833** | +0.0545 | +0.0114 |
| 7002 | gate only (first version) | +0.0237 | -0.0031 | **-0.0371** | +0.0904 | +0.0447 |
| 7002 | **gated** | +0.0469 | +0.0000 | +0.0131 | +0.0995 | +0.0749 |
| 7003 | rrf (union) | -0.0149 | -0.0031 | **-0.1004** | +0.0043 | +0.0397 |
| 7003 | **gated** | +0.0350 | +0.0000 | +0.0063 | +0.0452 | +0.0886 |

* The union fails the temporal bound on all three seeds, so the failure reproduces off E8-10.
* The gated arm is inside the Q4 bound (-0.01) overall and in every class on all three seeds.
* The "gate only" rows were printed by the first version, before the hop tier was added; the
  third seed's row is missing because that run stopped on an assertion that is only valid for
  E8-10 (below). They are copied from that run's terminal output, not from the output file.
* On seed 7003 the exact vector arm's nDCG@10 differs from the testkit's independent f64
  computation by 1.4e-4 (a near tie that f32 and f64 cosine order differently). On E8-10 the
  two agree within 1e-12 (#2378), and the harness asserts that on E8-10 only; on the dev seeds
  it prints the gap.

## 6. What will be measured

* **Arms**, all 100 queries, cut 10, one run on both engines (`Emulated<LogEngine>` and
  `KernoDb`, over a seeded `SimVfs`), as in #2378:
  * `graph`: graph leg only (the "without the vector signal" arm of Q4 as written).
  * `ann`: HNSW leg only. `exact`: exact `VectorSet` leg only (the arm "without signal" of
    T-E10-S1 and S2).
  * `rrf`: HNSW + graph under `Fusion::Rrf`. **Control**, #2378's primary arm. Informational: it
    must print #2378's figures again (overall -0.0051, temporal -0.0924, against graph).
  * `gated`: HNSW + graph under `Fusion::GraphGated`. **Primary.**
  * `ranked`: `rank_assoc` with `RankMode::On`, default weights, no vectors (second reading of
    S1 and S2). `brute`: independent brute-force cosine order (reference of Q1).
* **Not repeated:** the keyword arms of #2378. The frozen queries carry no text, those arms went
  through a seed-label derivation, they failed Q5 and were uninterpretable, and nothing in this
  change concerns the keyword leg.
* **Verdicts on the primary arm, the thresholds of #2378 unchanged:** Q4 as written:
  `gated - graph >= -0.01` in nDCG@10, overall and in each of the 4 classes. T-E10-S1:
  `gated - exact >= 0.02`. T-E10-S2: `gated - exact >= -0.01` per class. T-E10-Q2: gated
  nDCG@10 >= 0.80. T-E10-Q3: gated recall@10 >= 0.90. T-E10-Q5 over {graph, ann, exact, rrf,
  gated, ranked, brute} x 5 scopes (at most one position in ten unjudged). T-E10-V1, V2 (exact
  <= 0.96), Q1 and R.
* **Headline rule:** "demonstrated" if and only if Q4 as written (all five), S1, S2, V1, V2, Q1,
  Q5 and R hold. Q2 and Q3 are printed as the floors of acta 9 §5 and are not in the headline.
* **One recorded run.** Thresholds are printed and not asserted. Unjudged units in a top 10 are
  excluded from the list before the metrics and counted. What fails the test is what the
  measurement depends on: the MANIFEST, the reopened database serving every vector, the graph
  arm equal to the reference order for every query, the exact arm equal to the testkit's
  independent computation (within 1e-12), the gate holding in every gated answer, the two
  engines printing identical reports. The command:

  ```text
  cargo test -p kerno-units --all-features --release --test hybrid_gated_measure -- --nocapture --exact gated_hybrid_quality_over_the_second_frozen_set
  ```

* **The record.** The output of the first execution of that test on E8-10 is saved verbatim in
  `docs/testing/hybrid-gated-e8-10-output.txt`, with the hash of the pre-registration commit in
  its `commit:` line. The test also runs in the project's CI; it is deterministic, so those
  executions repeat the same figures and are not new measurements. If the run is found to have a
  defect in the harness, that is reported in section 8 and the corrected run is labelled as a
  second run; the first figures stay in the record.

## 7. Prediction, written before the run

From the dev seeds: Q4 met in all five readings (overall and the four classes), S1 and S2 met by a
wide margin (the graph arm is far above the exact arm), `rrf` printing #2378's figures, and
therefore the headline "demonstrated" if V1, V2, Q1, Q5 and R hold as they did in #2378. The
dev seeds are not E8-10, so this is a prediction and not a result: if the run says otherwise, the
run is the result.

## 8. Results

### 8.1 The run

One execution of `gated_hybrid_quality_over_the_second_frozen_set` on E8-10, both engines, on the
pre-registration commit `391342b` (the `commit:` line of `docs/testing/hybrid-gated-e8-10-output.txt`).
The two engines print identical reports (132 lines each). The MANIFEST matched; the exact arm
equals the testkit's independent computation (gap 0); the graph arm equals the reference order
for every query; the gate held in every gated answer.

### 8.2 Primary results

Bound, value and verdict, copied from the output.

| Criterion | Bound | Value | Verdict |
|---|---|---|---|
| Q4 as written, all-100 (`gated - graph`) | >= -0.01 | +0.0485 | met |
| Q4, vector_neighbourhood | >= -0.01 | +0.0000 | met |
| Q4, temporal | >= -0.01 | **+0.0108** | met |
| Q4, provenance | >= -0.01 | +0.0936 | met |
| Q4, combined | >= -0.01 | +0.0896 | met |
| T-E10-S1 (`gated - exact`, overall) | >= +0.02 | +0.5284 | met |
| T-E10-S2, vector_neighbourhood | >= -0.01 | +0.2982 | met |
| T-E10-S2, temporal | >= -0.01 | +0.6440 | met |
| T-E10-S2, provenance | >= -0.01 | +0.5780 | met |
| T-E10-S2, combined | >= -0.01 | +0.5935 | met |
| T-E10-Q2 (gated nDCG@10) | >= 0.80 | 0.9500 | met (floor, not in headline) |
| T-E10-Q3 (gated recall@10) | >= 0.90 | 0.9257 | met (floor, not in headline) |
| T-E10-Q5 (7 arms x 5 scopes) | <= 10 % unjudged | worst 1.6 % (`ranked`, combined) | met |
| T-E10-V1 | >= 25 per class | 25, 25, 25, 25 | met |
| T-E10-V2 (exact nDCG@10) | <= 0.96 | 0.4216 | met |
| T-E10-Q1 (exact against brute force) | recall 1.000 | smallest 1.000 | met |
| T-E10-R (MANIFEST) | match | match | met |

Second reading of S1 and S2 (without signal = the `ranked` arm, informational): S1 +0.0412
(met), S2 vector_neighbourhood +0.0000, temporal +0.0216, provenance +0.1300, combined +0.0134
(all met). In #2378 the same reading was not met for S1, temporal and combined.

### 8.3 Means per arm (nDCG@10; recall@10 is in the output file)

| Arm | all-100 | vector_neighbourhood | temporal | provenance | combined |
|---|---|---|---|---|---|
| graph | 0.9014 | 1.0000 | 0.9815 | 0.7518 | 0.8726 |
| ann = exact = brute | 0.4216 | 0.7018 | 0.3483 | 0.2674 | 0.3687 |
| rrf (control) | 0.8963 | 0.9971 | 0.8891 | 0.7958 | 0.9034 |
| **gated** | 0.9500 | 1.0000 | 0.9923 | 0.8454 | 0.9622 |
| ranked | 0.9087 | 1.0000 | 0.9707 | 0.7154 | 0.9488 |

**The control reproduces #2378.** The Q5 and mean rows of `graph`, `ann`, `exact`, `ranked`,
`brute` and `rrf` (#2378's `fused`) are identical, line for line after the arm name, to those of
`hybrid-e8-10-output.txt`; `rrf - graph` prints -0.0051, -0.0029, -0.0924, +0.0440, +0.0308.
So the only thing that changed between the two measurements is the ordering of the fused list.

Effect of the gate, `gated - rrf`: all-100 +0.0537, vector_neighbourhood +0.0029, temporal
+0.1032, provenance +0.0496, combined +0.0588.

### 8.4 Against the prediction of section 7

Matched: Q4 met in all five readings, S1 and S2 met by a wide margin, `rrf` printing #2378's
figures, headline "demonstrated". Nothing in the run departed from the pre-registration. The
temporal margin on the dev seeds was +0.006 to +0.015 and on E8-10 it is +0.0108.

### 8.5 Reading: what this supports, and what it does not

* **What it supports.** With the graph as the authority on which units lead and how near they
  are, adding the vector signal does not lower nDCG@10 against the graph alone in any class of
  E8-10; it raises it in `temporal` (+0.0108), `provenance` (+0.0936) and `combined`
  (+0.0896). Q4 is a non-inferiority criterion ("with the vector signal, nDCG@10 >= without it
  - 0.01, overall and in each class"), and it holds as written.
* **The temporal failure of #2378 came from the union, not from the vector signal.** Same
  vectors, same index, same weights, same `k`: only the order of the fused list changed, and the
  temporal gap went from -0.0924 to +0.0108. That fits the diagnosis of section 3 and the three
  dev seeds; the run does not isolate which of the displaced units mattered.
* **The gain over the graph arm has the shape section 3 predicted** (it comes from the order
  inside the walk's own hits, since no vector-only unit can be relevant on this set), but the
  run does not measure that decomposition either.
* **It does not say the union is fine.** `Fusion::Rrf` still fails the temporal bound on this set
  (-0.0924) and on the three dev seeds. Its default is unchanged.
* **It does not say the vector signal helps in general.** E8-10 grades reachability through true
  links by depth and grades every vector neighbour that no link reaches as 0, so a method that
  keeps the graph's hits and hop order in front is favoured by construction. The design was
  chosen with that in view (section 4). A pass here means "no harm when the graph carries the
  relevance"; it is silent on queries whose relevance lives in the vectors or in text.
* **It is not a held-out result.** The method was chosen after #2378's figures were known
  (section 2). Three fresh seeds were used to develop it, but they come from the same generator
  and the same judging rule, so they test the method against new draws, not against a new kind of
  set. E8-10 has now been used to judge two fusion modes; any further mode, weight or parameter
  must be measured on a new frozen set.

### 8.6 Limits

* The set is synthetic: 2000 units, seeded generators, no real workload.
* The text of a unit is its label; the keyword leg was not re-measured (section 6).
* The vectors are seeded integers, not the output of an embedding model.
* The judgments have a single author.
* One machine, one recorded run. The test is deterministic; later executions in CI repeat it.
* Thresholds are printed and not asserted; the frozen set, its queries and its thresholds were not
  changed (the three frozen files still match their MANIFEST).
* The hop tier reads `AssocHit::distance`; a graph leg whose `limit` cuts the walk early keeps
  its own cut (`candidates` per leg), and the gate does not look past it.

### 8.7 How to reproduce

```text
cargo test -p kerno-units --all-features --release --test hybrid_gated_measure -- --nocapture --exact gated_hybrid_quality_over_the_second_frozen_set
```

About 35 seconds once built. The output is deterministic except for the timing line.

### 8.8 Mutation table

Each mutation was applied alone to the tree and reverted; every one turned the named test red
(`cargo test -p kerno-units --all-features --release --test hybrid`, or `--test hybrid_gated_measure`
for H1 to H3).

| N | File | Mutation | Failing test(s) |
|---|---|---|---|
| A | `hybrid.rs` | the walk's misses tier `(1, 0)` -> `(0, 0)` (misses no longer after the walk) | `the_gate_puts_the_walks_hits_before_every_other_unit`, `the_gate_keeps_the_rules_of_the_legs`, `inside_the_gate_the_other_legs_still_reorder` |
| B | `hybrid.rs` | hop `(0, hop)` -> `(0, 0)` (no hop tier) | `the_gate_keeps_the_hop_order_of_the_walk` |
| C | `hybrid.rs` | hop -> `u32::MAX - hop` (farthest hop first) | `the_gate_keeps_the_hop_order_of_the_walk` |
| D | `hybrid.rs` | `fusion: Fusion::default()` -> `Fusion::GraphGated` in `HybridQuery::new` | `the_default_fusion_is_the_union` |
| E | `hybrid.rs` | drop the score from the sort key | `inside_the_gate_the_other_legs_still_reorder`, `the_gate_keeps_the_hop_order_of_the_walk`, `vector_hits_of_inactive_units_are_dropped` |
| F | `hybrid.rs` | drop `fusion` from the `Debug` of a query | `the_gate_keeps_the_rules_of_the_legs` |
| G | `hybrid.rs` | `Fusion::Rrf` also gated | `the_gate_puts_the_walks_hits_before_every_other_unit`, `inside_the_gate_the_other_legs_still_reorder`, `the_gate_keeps_the_hop_order_of_the_walk`, `the_default_fusion_is_the_union` |
| H | `hybrid.rs` | record hop 0 for every walk hit | `the_gate_keeps_the_hop_order_of_the_walk` |
| H1 | `hybrid_gated_measure.rs` | Q4 `gain >= Q4_MIN_GAIN` -> `>` | `every_verdict_flips_at_its_own_bound` |
| H2 | `hybrid_gated_measure.rs` | `.filter(\|u\| judged.contains(u))` -> `.filter(\|_\| true)` | `unjudged_units_are_excluded_and_counted` |
| H3 | `hybrid_gated_measure.rs` | headline over every verdict, not only the headline ones | `every_verdict_flips_at_its_own_bound` |
