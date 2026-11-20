# Evaluation set for the derived data and the vector search (E8-01)

| Field | Value |
|---|---|
| Date | 2026-11-12 |
| Status | **Frozen** in E8-01, from the commit that adds this document. The files of `data/derived-eval/` do not change after that date. A new judgement requires freezing the `MANIFEST` again before measuring (minutes §6.1). The commit hash is in `git log -- crates/kerno-testkit/data/derived-eval` |
| Minutes | [Stage 8 minutes](stage8-record.md) §6.1 (set), §6.2 (thresholds), §4 (common criteria) and decision D5 |
| Code | `crates/kerno-testkit/src/derived_eval.rs` (generator and formats), `examples/freeze_derived_eval.rs` (writes the files), `tests/derived_eval.rs` (tests). Uses `sha256.rs` and `assoc_eval::manifest_text`, which belong to `kerno-testkit` |
| Data | `crates/kerno-testkit/data/derived-eval/`: `corpus.txt`, `vectors.txt`, `queries.txt`, `MANIFEST` |
| Measurement | None in this document. E8-07 will measure the thresholds of §6.2 over this set |

## 0. What it measures and what it does not

This set measures **the mechanics** of the search over vectors and of the registry of derived data. It does not measure the semantic quality of any model.

The vectors are **deterministic and come from a seed** (minutes, decision D5). They do not come from any model or any provider, and no step of the generation uses an external model. A topic has a centre, and its decisions advance from that centre along its chain (§2.6). By construction, the vector search finds the neighbourhood of the chain: the set says nothing about how closely that search resembles real memory. That is what the minutes call R-E8-5, and it is declared here in §8.

The degrees of relevance come from the story with which the corpus is generated (§3.2), not from the code of `kerno-units`. The generator does not depend on `kerno-units`, and a test checks it.

This set **is not** the one of Stage 7 and is not used to measure anything of Stage 8 that repeats it. It has another seed, another story structure and its own permutation of identifiers (minutes §6.1).

## 1. What is frozen

| Element | Count |
|---|---|
| Units | 2 000: 800 `fact`, 500 `source`, 300 `task`, 300 `decision`, 100 `session` |
| Relations | 5 062: 1 900 `in_session`, 800 `cites`, 600 `same_source`, 495 `mentions`, 577 `related`, 400 `supports`, 290 `revises` |
| Relations by provenance (ADR-0014) | 3 785 `declared`, 600 `algorithmic`, 577 `inferred`, 100 `imported` |
| Change sets | 300: 100 from an agent (`transcript/NNNN`), 100 from the importer (`wiki-export/NNNN`), 100 from the consolidator (`job/consolidate/NNNN`) |
| Vectors | 2 000, dimension 16, one per unit and in identifier order |
| Queries | 100: 25 `vector_neighbourhood`, 25 `temporal`, 25 `provenance` (8 `PV-RE`, 8 `PV-SS`, 9 `PV-DE`) and 25 `combined` |
| Expected answers | 1 193: 265 of grade 3, 241 of grade 2, 87 of grade 1 and 600 of grade 0 |
| Relevant per query | between 1 and 13. There are 16 queries with more than 10 |
| Answers per query | between 3 and 23 |
| File sizes | `corpus.txt`: 7 365 lines, 248 950 bytes. `vectors.txt`: 2 002 lines, 153 200 bytes. `queries.txt`: 1 296 lines, 18 406 bytes |

The counts in the table were counted over the frozen files with `awk` (they were not taken from the header comments). The test `quality_set_has_the_agreed_shape` checks part of them.

The performance scale (20 000 units, 3 000 change sets, 20 000 vectors, the same queries in families) is generated from the same seed. **It is not frozen**: the test `performance_scale_is_deterministic` checks its size and its determinism.

## 2. How the story is built

### 2.1 Seed and scales

- Seed: `SEED = 20_261_112` (the date of these minutes). It differs from that of Stage 7 (`20_261_111`).
- Generator: SplitMix64 (`crates/kerno-testkit/src/rng.rs`). Each stream has its own constant, XOR-ed with the seed:
  - story: the seed;
  - identifier permutation: `0xD8E1_0000_0000_0004`;
  - vectors: `0xD8E1_0000_0000_0003`;
  - queries: `0xD8E1_0000_0000_0002`.
- Quality scale: 100 sessions, 2 000 units, 300 change sets. It is the one that is frozen.
- Performance scale: 1 000 sessions. It is generated, not frozen.

### 2.2 Units of a session

Each session `s` creates 20 units, in this creation order:

| Local index | Type | Label | Role |
|---|---|---|---|
| 0 | `session` | `s0000` | The session |
| 1–3 | `task` | `s0000.t0`–`t2` | Three tasks (they have no graded relations) |
| 4–6 | `decision` | `s0000.d0`–`d2` | Three decisions on one same topic |
| 7–14 | `fact` | `s0000.f0`–`f7` | Eight facts |
| 15–19 | `source` | `s0000.src0`–`src4` | Five sources |

The topic of the session is chosen at random from 10.

### 2.3 Chains, evidence and distractors

- **Chain of a topic.** Each decision `revises` the previous decision of its same topic, even if it is in another session. A topic is a linear chain.
- **Evidence.** Each decision `d_k` has a fact `f_k` that supports it (`supports`). Each fact cites a source (`cites`). The eight citations are shuffled in each session, so three sources are cited by two facts each.
- **Mention distractor.** The facts `f0`..`f4` mention (`mentions`) a decision of another topic from an earlier session. It is a false link.
- **Import.** The importer adds a true `supports` from `f7` to `d1`, with provenance `imported wiki-export`.
- **Inferred links.** The consolidator adds, for each decision, up to two `related` links with provenance `inferred consolidator-v1`:
  - one true, to the decision two positions back in its chain, with confidence uniform between 450 and 950 (only if the decision has at least two earlier ones in its topic);
  - one false, to a decision of another topic from an earlier session, with confidence uniform between 300 and 800.
  The ranges overlap on purpose: the confidence is informative, but it is not enough.
- **Same origin.** The consolidator adds six algorithmic `same_source` links (`dedup-v2`): three true (the pairs that cite the same source) and three false (pairs chosen at random among facts with different sources). All six come out in random order and orientation.

### 2.4 Steps

The history has whole steps, not a clock. For session `s`:

- `3s+1`: the agent. It creates the units and the `in_session`, `revises`, `supports`, `cites` and `mentions` relations.
- `3s+2`: the importer. It creates the imported `supports`.
- `3s+3`: the consolidator. It creates the inferred `related` and the algorithmic `same_source`.

Each link is created in a step no earlier than that of its ends. The reader checks it (§4). The time windows of the queries are intervals of these steps.

### 2.5 Identifiers

The identifiers are a **seeded random permutation** of the creation phase. The file lists the units in creation order, but with their identifier permuted. Thus the tie-break by identifier favours no structure, as §6.1 asks. The test `identifiers_are_a_permutation_of_creation_order` checks that they are a permutation of `0..2000` and that very few fall on their creation position.

### 2.6 Vectors

Each topic has a **centre**: 16 uniform integers between −1 000 and 1 000.

- **Decisions.** The position of a decision is the topic's centre plus a **walk**: each decision of the chain adds a random step, between −40 and 40 per component. Thus decisions close in the chain are close in space. The vector of the decision is that position plus a noise of ±15 per component.
- **Other units.** Centre plus a noise of ±400 per component.
- **Vector queries.** The vector of the query is the seed's position plus a noise of ±15.

The whole vector is computed with integers. The judgement rule of §3.2 compares by cosine in floating point, but only to sort: the square root and the four operations are IEEE 754 and give the same result on any platform, and ties are resolved by identifier.

## 3. Queries and judgements

### 3.1 The four classes

| Class | Identifier | Seed | Traversal | Window | Provenance | What it tests |
|---|---|---|---|---|---|---|
| `vector_neighbourhood` | `VN-01`..`25` | Decision | Both directions, `revises`, depth 2 | None | Any | Neighbourhood of the chain, with the query vector |
| `temporal` | `TE-01`..`25` | Decision of session 2 or later | Both directions, `revises`, `supports`, `mentions`, depth 3 | Sessions ±2 | Any | Temporal context |
| `provenance` | `PV-RE-01`..`08` | Decision | Outgoing, `related`, depth 1 | None | `inferred` | True inferred link against false |
| `provenance` | `PV-SS-01`..`08` | Fact | Both directions, `same_source`, depth 1 | None | `algorithmic` | The same, with `same_source` |
| `provenance` | `PV-DE-01`..`09` | Decision of session 3 or later | Both directions, `revises`, `supports`, `mentions`, depth 3 | None | `declared` | Declared chain, without import or inferences |
| `combined` | `CB-01`..`25` | Decision | Both directions, `revises`, `supports`, `related`, depth 3 | Sessions ±1 | `declared`, `inferred` | Provenance and time at once |

A window `lo-hi` is a closed interval of **creation steps of the links**, not of units.

A query generator chooses seeds at random. A seed is accepted only if the query has at least one relevant answer and at least one distractor other than the seed. If not, it is chosen again.

### 3.2 Grades

The traversal follows **only the true links**. A false link ends the traversal: its end is a leaf distractor, never the path to a relevant unit.

- **Grade 3** at distance 1 through true links, **2** at distance 2 and **1** at distance 3. The maximum depth is 3.
- **Named grade 0**, in this priority order:
  1. the seed, always, so that no query counts it as an answer;
  2. the end of a false link that the traversal finds;
  3. the ten nearest neighbours by cosine to the query vector that are not judged (`VN` only);
  4. the first ten units of the simple order ("hops, then identifier") that are not judged.

The `VN` queries have 10 cosine neighbours, and the cut of §6.2 is 10. That is why the exact top 10 of the vector is fully judged (§3.3).

Grades 3, 2 and 1 are a convention of this document, not judgements of people. See §8.

### 3.3 Unjudged units

What does not appear in `queries.txt` is **unjudged** (minutes §6.1). An arm that returns an unjudged unit in its top 10 does not count it as grade 0: it excludes it from the calculation and its number is reported.

By construction, in this set:

- the exact top 10 of the vector of each `VN` query is judged. The reason: the ten neighbours of point 3 of §3.2 are the ten unjudged units nearest to the vector. If a unit of the exact top 10 were not judged, it would be one of the ten nearest unjudged units, because at most nine units are nearer than it. It would then have received a judgement, which contradicts the hypothesis;
- the first ten of the **simple order** of each query are judged, because point 4 of §3.2 judges them all.

That is why the number of unjudged positions in the top 10 of those two reference arms is **0**. A hybrid arm does not have that guarantee: E8-07 will count its unjudged positions and, if they go past 10 %, the set will be judged further before measuring (minutes §6.1, Q5).

The guarantee depends on the **cosine**. Another metric, such as the Euclidean distance, would give a different top 10 and would not be covered. The vector arm of E8-07 must use cosine for Q5 to be met.

## 4. File formats

The files are text with one line per record. Empty lines and lines that start with `#` are ignored. The fields are separated by spaces and none contains spaces.

### 4.1 `corpus.txt`

```text
# kerno-testkit derived-eval corpus v1
seed 20261112 scale quality
counts units 2000 relations 5062 changes 300
C 1 agent-gamma transcript/0000
U 298 1 session s0000
U 547 1 task s0000.t0
U 1729 1 decision s0000.d0
```

| Line | Form | Meaning |
|---|---|---|
| header | `seed <u64> scale <quality\|performance>` | Seed and scale |
| counts | `counts units <n> relations <n> changes <n>` | Totals. The reader compares them with the records |
| change set | `C <step> <actor> <source>` | Opens a change set. The step is the one after the previous |
| unit | `U <identifier> <step> <type> <label>` | A unit. The identifier is permuted |
| link | `R <step> <from> <to> <type> <provenance>` | A link. The ends must exist and have been created in a step no later |
| provenance | `declared` \| `imported <source>` \| `algorithmic <origin>` \| `inferred <origin> <confidence>` | The ADR-0014 class with its data. The confidence goes from 0 to 1 000 |

### 4.2 `vectors.txt`

```text
# kerno-testkit derived-eval vectors v1
dim 16 counts vectors 2000
V 0 1262 363 -224 458 1029 -107 -871 -468 -285 -426 75 -522 -600 -353 -465 913
```

One line per unit, in identifier order. The reader checks the dimension of each line, the order and the total number.

### 4.3 `queries.txt`

```text
# kerno-testkit derived-eval queries v1
seed 20261112 scale quality cut 10 dim 16
counts queries 100
Q VN-01 vector_neighbourhood 508 both revises 2 - - -796,-1008,566,103,-112,381,-198,57,-601,421,-133,-43,-88,437,716,884
E 157 0
E 230 0
E 232 0
E 508 0
```

`Q` carries: identifier, class, seed, direction, types separated by commas, depth, window (`-` or `lo-hi`), provenances (`-` or a list) and vector (`-` or 16 integers separated by commas). Each `E` is an answer: unit and grade from 0 to 3. The reader requires the answers to be sorted by unit, with no repeats.

### 4.4 `MANIFEST`

Format of `sha256sum`: one record per file, in this order: `corpus.txt`, `vectors.txt`, `queries.txt`.

```text
24cdbaa5d80d6e7a8377010769b70251b7ce6a9942cf129b6595585248d6dfe5  corpus.txt
7645119a705b3c154e44bcc6e914efe2b9a3712260645560d7a3ebbf1f55d538  vectors.txt
ef59c9019ef275c86ef4199757a7be327a9b075c91a21cacae5b8d864f8a24bd  queries.txt
```

## 5. Regeneration and verification

Regenerate from the seed (writes the three files and the `MANIFEST`; over the frozen tree nothing changes):

```text
cargo run --release -p kerno-testkit --example freeze_derived_eval
```

Verify the tests, which compare byte for byte with the frozen files and check the manifest:

```text
cargo test -p kerno-testkit --release --test derived_eval
```

Verify the manifest with the standard tool:

```text
$ cd crates/kerno-testkit/data/derived-eval && sha256sum -c MANIFEST
corpus.txt: OK
vectors.txt: OK
queries.txt: OK
```

The three `OK` lines came out in the run for this document.

## 6. Check of the thresholds of §6.2

It was checked that **each threshold falls within the bound of its metric** over this set, before freezing it. No threshold has changed. The table uses reference values measured over the frozen files with a temporary program that is not kept, as in the Stage 7 document (§9.2). Those that are theoretical bounds or tests are marked as such.

| Code | Threshold | Bound of the metric in this set | Measured reference | Result |
|---|---|---|---|---|
| **Q1** | recall@10 = 1.000 against the reference | Recall is in [0, 1] | It is the same search; it does not depend on the set | **Within the bound** |
| **Q2** | nDCG@10 with vector signal ≥ 0.80, overall | nDCG is in [0, 1] and a perfect ranking gives 1 | Vector-only arm (cosine) over `VN`: mean 0.708, minimum 0.662 | **Within the bound.** A vector-only arm does not reach 0.80 in that class; the threshold will be measured with a hybrid arm in E8-07 |
| **Q3** | recall@10 with vector signal ≥ 0.90, overall | Mean ceiling of a perfect ranking: **0.977** (`thresholds_of_the_frozen_set_are_within_reach`). There are 16 queries with more than 10 relevant, and there the ceiling is 10/R | Vector arm over `VN`: mean recall@10 1.000 | **Within the bound**, with a margin of 0.077 |
| **Q4** | With vector signal, nDCG@10 ≥ without it − 0.01, overall and in each class | It is non-inferiority: the bound is nDCG without signal − 0.01 ≤ 1 | In `VN`: vector 0.708, without signal (simple order) 0.725; bound 0.715 | **Within the bound.** A vector-only arm is 0.007 below the bound in `VN`: it is a result for E8-07, not a broken bound |
| **Q5** | Unjudged units in the top 10 ≤ 10 % of the positions, in each arm | It is a validity condition: at most one position out of ten | 0 unjudged positions in the vector arm and in the simple order, by construction (§3.3) | **Met by construction** in the two reference arms. A hybrid arm is counted in E8-07 |
| **Q6** | Only if the ranking is enabled by default in a later stage: nDCG@10 ≥ simple order + 0.02 | The simple order must stay below 0.98 for the threshold to be reachable | Simple order, mean per class: `combined` 0.702; `provenance` 0.615; `temporal` 0.739; `vector_neighbourhood` 0.725 | **Within the bound.** It is not a closing criterion of Stage 8 |

Note on the simple order. It is "hops, then identifier" over all the admitted links, false ones included, with the seed excluded. It is a reference for the set. The real simple order of the engine will be measured in E8-07, and if it differs from this definition, its top 10 may have unjudged positions.

Conclusion: **no threshold falls outside the bound of its metric.** Under the rule of the minutes, there is no need to stop or change thresholds. There are two observations for the owner, which are not bounds:

1. A **vector-only** arm does not meet Q2 or Q4 in the class `vector_neighbourhood` (0.708 against 0.725 of the simple order, with 0.715 as the bound). That is the result of the measurement of E8-07, not of the set. The thresholds stay as they are (decision D4).
2. Q2 and Q3 "overall" refer to queries with vector signal, which in this set are the 25 of `VN`. The set does not fix which hybrid arm is measured: E8-07 will fix it.

## 7. Tests and mutations

`tests/derived_eval.rs` has fifteen tests:

| Test | What it checks |
|---|---|
| `regenerated_set_is_byte_identical` | The three frozen files are exactly what the seed produces |
| `manifest_matches_committed_files` | The `MANIFEST` lists those three files, in that order, with their SHA-256 |
| `committed_files_parse_back` | The frozen text reads back the same story and the same vectors and queries |
| `parser_refuses_a_repeated_unit` | The reader rejects a repeated identifier, and rejects it **for that reason** |
| `parser_refuses_a_vector_of_the_wrong_size` | The reader rejects a vector with one component too many |
| `quality_set_has_the_agreed_shape` | Sizes, provenance classes, four query classes, grades 0 to 3, and that each query has relevant items and distractors |
| `vector_file_has_the_declared_dimension` | Each line of `vectors.txt` has the declared dimension |
| `expected_answers_are_reachable_within_the_bounds` | Each relevant answer is reachable within the bounds of its query, and each distractor is reachable or is a neighbour of the vector |
| `vector_top_ten_is_fully_judged` | The exact top 10 of the vector of each vector query is judged |
| `identifiers_are_a_permutation_of_creation_order` | The identifiers are a permutation and few fall on their creation position |
| `generation_is_deterministic` | The same seed gives the same history |
| `performance_scale_is_deterministic` | The 20 000-unit scale has its size and is deterministic |
| `thresholds_of_the_frozen_set_are_within_reach` | The recall@10 ceiling of a perfect ranking (Q3) reaches 0.90 |
| `seed_is_not_the_stage_seven_seed` | The seed differs from that of Stage 7 |
| `testkit_does_not_depend_on_kerno_units` | The `Cargo.toml` of `kerno-testkit` does not mention `kerno-units` |

The tests that only read the frozen files do not see changes in the generator: that is why each one has been mutated in its own code or in that of the generator (criterion 4 of §4 of the minutes). Each mutation was reverted with the inverse `sed` and the restoration was checked.

| Mutation | Where | Tests that failed |
|---|---|---|
| `DECISION_NOISE` 15 → 16 | `src/derived_eval.rs:47` | `regenerated_set_is_byte_identical`, `committed_files_parse_back` |
| Seed 20 261 111 | `src/derived_eval.rs:27` | `seed_is_not_the_stage_seven_seed`, `regenerated_set_is_byte_identical`, `committed_files_parse_back` |
| Identifiers not permuted | `src/derived_eval.rs:767` | `regenerated_set_is_byte_identical`, `committed_files_parse_back` |
| Vector without one component | `src/derived_eval.rs:860` | `regenerated_set_is_byte_identical` |
| The reader accepts repeated identifiers | `src/derived_eval.rs:1019` | `parser_refuses_a_repeated_unit` (see below) |
| The vector reader accepts one component too many | `src/derived_eval.rs:1075` | `parser_refuses_a_vector_of_the_wrong_size` |
| Traversal of depth 0 | `tests/derived_eval.rs`, `reach` | `expected_answers_are_reachable_within_the_bounds` |
| Top 15 instead of top 10 | `tests/derived_eval.rs` | `vector_top_ten_is_fully_judged` |
| Required grade 4 | `tests/derived_eval.rs` | `quality_set_has_the_agreed_shape` |
| `vectors.txt` with the bytes of `queries.txt` | `tests/derived_eval.rs` | `manifest_matches_committed_files` |
| 20 001 units | `tests/derived_eval.rs` | `performance_scale_is_deterministic` |
| Dependency on `kerno-units` inverted | `tests/derived_eval.rs` | `testkit_does_not_depend_on_kerno_units` |
| Q3 threshold of 0.99 | `tests/derived_eval.rs` | `thresholds_of_the_frozen_set_are_within_reach` |
| Comparison with another seed | `tests/derived_eval.rs` | `generation_is_deterministic` |
| Fixed identifier at 0 (`fixed < 0`) | `tests/derived_eval.rs` | `identifiers_are_a_permutation_of_creation_order` |
| Line dimension `DIM + 1` | `tests/derived_eval.rs` | `vector_file_has_the_declared_dimension` |

**One mutation that stayed green.** The first version of `parser_refuses_a_repeated_unit` passed with the reader mutated: the reader without the repeat check rejected the same file later on, because a link pointed to an identifier not yet created. The test was right for the wrong reason. It was corrected to require the repeated-identifier message, and the mutation now fails.

Test count: the module `derived_eval.rs` has no tests of its own. The fifteen in `tests/derived_eval.rs` are new and were approved in the last run of that binary. The total count of `kerno-testkit` and `kerno-units` is not in this document.

## 8. Limits

1. **Function vectors, not model vectors.** The vector search finds the neighbourhood because the generator puts it there (§2.6). A high result in `VN` measures that the search and the registry work, not that a real model separates the topics or the decisions. It is the limit R-E8-5 of the minutes.
2. **Same author for the story, the queries and the grades.** The generator, the query families and the grade rules were written by the same team before the Stage 8 code was written. The freezing protects against fitting the answers to the code, but not against an assumption of the generator that the code shares with it (R-E8-2).
3. **Judgements without independence.** There is no external judgement. Grades 3, 2 and 1 are a distance convention, not a human relevance one.
4. **Regular structure.** Each session has exactly three decisions of one topic, eight facts and five sources. Each topic has a single linear chain, and there are ten topics. Real memory has branches, decisions that contradict each other and irregular structures.
5. **Small vocabulary.** Ten topics, three agents, one importer and one consolidator. The labels are strings such as `s0000.d0`, without natural text.
6. **One hundred queries.** With 25 per class and between 1 and 13 relevant, the variance of a mean is high. A small difference between two arms is not significant in this set.
7. **Vector distractors chosen by the set.** The ten cosine neighbours are fixed when the set is generated, before any arm runs, so that none chooses the list. Q5 is met for the vector arm and for the simple order, but not for an arm that uses another metric or that mixes signals.
8. **Fixed metric.** The guarantee of Q5 depends on the cosine (§3.3).
9. **Performance not measured.** The 20 000-unit scale is generated and tested in form. The times are E8-07's.
10. **What it does guarantee.** That any change to the generator, the format or the reader is detected; that the expected answers are reachable within their bounds; that each query has relevant items and distractors; that the two reference arms have no unjudged positions in the top 10; and that each threshold of §6.2 falls within the bound of its metric.

## 9. Status of the gates of E8-00

This set does not depend on the content of ADR-0018, but E8-01 is a package after E8-00 (minutes §3), and in the repository, when this document was written:

- The Stage 8 minutes say in their header and in §10 that they are "proposed, pending the owner's approval". The task of E8-01 indicates that the owner approved them on 2026-11-12, but the text of the minutes does not record it.
- ADR-0018 is listed as **Proposed** in `docs/adr/0018-*.md`.
- ADR-0009 is listed as **Accepted** (2026-11-12, option 3).

This document does not close any of those gates.

## 10. Related documents

- [Stage 8 minutes](stage8-record.md) §3 (E8-01), §4 (common criteria), §6.1 (set), §6.2 (thresholds), §8 (D4, D5) and §9 (R-E8-1 to R-E8-6)
- [Stage 7 evaluation set](stage7-evaluation-set.md): the set of associative memory, with another seed and another structure
- [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md): exact vectors in memory
- [ADR-0014](../adr/0014-physical-representation-of-relations.md): the four provenance classes
- [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md): derived data and their registry
