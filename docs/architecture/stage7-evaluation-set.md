# Evaluation set for associative memory (E7-00)

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Frozen** in E7-00, with a correction made the same day and before any retrieval code existed (§9). The files in `data/assoc-eval/` do not change after this date (minutes §6.1, §7 point 2) |
| Minutes | [Stage 7](stage7-record.md) §6.1 (set), §6.2 (quality), §7 point 2 |
| Associated decision | [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) (accepted in E7-00). This document is the link to its evidence section |
| Code | `crates/kerno-testkit/src/assoc_eval.rs` (generator and formats), `assoc_metrics.rs` (metrics), `sha256.rs` (SHA-256 of the manifest), `examples/freeze_assoc_eval.rs` (writing the files) |
| Data | `crates/kerno-testkit/data/assoc-eval/`: `corpus.txt`, `queries.txt`, `MANIFEST` |
| Measurement | `crates/kerno-units/tests/assoc_baseline.rs`; raw output in [stage7-e700-output.txt](stage7-e700-output.txt) |

This document describes the set as it is in the code and in the files. Every figure has been obtained from those files or from the output of the commands indicated. No number has been taken from the intent of the design.

## 1. What is frozen

The set has two parts that are generated together from one seed:

- **Corpus**: 2 000 units, 3 958 relations and 300 change sets (quality scale).
- **Queries**: 64 queries, each with its seed, its constraints and its expected answers with a relevance grade from 0 to 3. There are 533 expected answers in total: 143 of grade 3, 122 of grade 2, 57 of grade 1 and 211 of grade 0 (named distractors).

The counts in the table below were tallied from the frozen files with a reading script (they were not taken from the header comments of the files).

| Element | Count |
|---|---|
| Units | 2 000: 800 `fact`, 500 `source`, 300 `task`, 300 `decision`, 100 `session` |
| Relations | 3 958: 800 `cites`, 600 `same_source`, 573 `related`, 500 `about`, 300 `in_session`, 300 `produced_by`, 300 `supports`, 297 `mentions`, 288 `revises` |
| Relations by provenance | 2 585 `declared`, 600 `algorithmic`, 573 `inferred`, 200 `imported` |
| Change sets | 300: 100 from an agent (source `transcript/NNNN`), 100 from the importer (`wiki-export/NNNN`), 100 from the consolidator (`job/consolidate/NNNN`) |
| Queries by class | 16 `multi_hop`, 16 `temporal`, 16 `provenance`, 16 `combined` |
| Seeds | 60 decisions and 4 facts (the four `PV-SS` queries) |
| Answers per query | between 2 and 20; between 2 and 9 relevant (grade ≥ 1) in `multi_hop`, 5 to 8 in `temporal`, 4 to 7 in `combined` and 1 to 7 in `provenance` |

## 2. How the narrative is built

The generator writes, for each session `s`, an agent memory as a story. The story has three change sets per session, in this logical order.

### 2.1 Seed and scales

- Seed: `SEED = 20_261_111` (the date of the entry record, 2026-11-11). It is a `u64`.
- Generator: SplitMix64 (`crates/kerno-testkit/src/rng.rs`), with the seed mixed with a fixed constant. The queries use another stream: the seed XORed with `0x0E70_0000_0000_0001`, so as not to share numbers with the corpus.
- Quality scale: 100 sessions, 2 000 units, 300 change sets. This is the frozen set.
- Performance scale: 1 000 sessions, 20 000 units, 3 000 change sets and the same 64 queries (the test `performance_scale_is_deterministic` checks the generation, the number of units and of change sets). Its relations are not frozen.

### 2.2 Units of a session

Each session creates 20 units in its first change set, in this local index order:

| Index (session 0) | Type | Label | Role |
|---|---|---|---|
| 0 | `session` | `s0000` | The session |
| 1–3 | `task` | `s0000.t0`–`t2` | Three tasks of the session |
| 4–6 | `decision` | `s0000.d0`–`d2` | Three decisions on the same topic |
| 7–14 | `fact` | `s0000.f0`–`f7` | Eight facts (evidence and mentions) |
| 15–19 | `source` | `s0000.src0`–`src4` | Five sources cited by the facts |

The topic of the session is chosen at random from 12. The decisions on the same topic form a **chain**: each decision `revises` the previous one on the topic. Thus a chain crosses sessions.

Facts `F0`, `F2` (and `F1`, which arrives by import) support `D0`, `D1` and `D2` with a `supports` relation. Facts `F3`..`F7` are `about` tasks. Each fact `cites` one of the five sources. The assignment is shuffled in each session with the seeded generator: three pairs of facts share a source (two sources are cited only once) and which facts form each pair changes from one session to another.

### 2.3 Relations and provenance

The four provenance classes of [ADR-0014](../adr/0014-physical-representation-of-relations.md) appear as follows:

| Provenance | Where it is created | What it covers |
|---|---|---|
| `declared` | First change set of the session | `in_session`, `produced_by`, `supports` (from `F0` and `F2`), `mentions`, `about`, `cites`, `revises` |
| `imported <source>` | Second change set (importer, `wiki-export`) | The `supports` from `F1` to `D1` and an `about` from `F6` to `T0` |
| `algorithmic <origin>` | Third change set (consolidator), origin `dedup-v2` | Six `same_source` between facts: three true and three false, in random order and orientation (see 2.6) |
| `inferred <origin> <confidence>` | Third change set, origin `consolidator-v1` | Up to two `related` per decision: one true and one false, written in random order (see 2.6) |

The `mentions` relation points from a fact to a decision on another topic from an earlier session. It is the chain's distractor (see 2.6).

### 2.4 Actors and sources

The word «source» is used in two different senses in the set. They must not be confused.

- **Source of a change set** (`C ... <source>`): the text the change comes from. For the agent, `transcript/NNNN`; for the importer, `wiki-export/NNNN`; for the consolidator, `job/consolidate/NNNN`. The `PV-SR` queries filter by this source.
- **Unit of type `source`**: one of the five sources cited by the facts (`cites`). It is not the origin of a change.

The actors of the change sets are `agent-alpha`, `agent-beta` and `agent-gamma` (one per session, chosen at random: 33, 39 and 28 sessions respectively), `importer` and `consolidator`. The `PV-AC` queries filter by the actor of the seed's session, and the pairs of the `combined` class do so too.

### 2.5 Logical order of changes

The history has whole steps, not a clock. For session `s` the steps are:

- `3s+1`: the agent writes the transcript. It creates the session, the tasks, the decisions, the facts, the sources and the `declared` relations.
- `3s+2`: the importer creates the imported `supports` and `about`.
- `3s+3`: the consolidator adds the inferred `related` and the algorithmic `same_source`.

Each relation is created in a step that is not earlier than that of its endpoints. The parser checks this (see 3.1). The time windows of the queries are intervals of these steps.

### 2.6 Distractors

A distractor is a unit that a query reaches within its bounds and that is not relevant. They are named in the expected answers with grade 0. There are four sources of distractors:

1. **Mentions across topics**: the `mentions` from a fact to a decision on another topic.
2. **False `related` links**: the consolidator points to a decision on another topic with confidence uniformly drawn between 300 and 800 per mille. The true ones point to the decision two positions back in the chain with confidence uniformly drawn between 450 and 950 (count: 276 true and 297 false). The ranges overlap between 450 and 800: the confidence is informative, but not sufficient. Over the 573 links, the best confidence threshold gets 0.675 right (the test requires less than 0.85). The two links of a decision are written in random order.
3. **False `same_source`**: three pairs of facts that do not share a source, chosen at random among all pairs with different sources in the session. The true ones are the three pairs that do cite the same source. All six are written in random order and with random orientation (which fact is the origin and which the destination). No index, offset within the session, distance between indices or creation position decides the truth (§9).
4. **Decisions and evidence at distance 3 or more** in the chain: they appear with grade 0 when the traversal reaches them (see 5.1).

## 3. File formats

The three files are text with one line per record. Empty lines and lines starting with `#` are ignored on reading. The separators are spaces. No field may contain spaces; labels, sources and origins are words without spaces.

### 3.1 `corpus.txt`

```text
# kerno-testkit assoc-eval corpus v1
seed 20261111 scale quality
counts units 2000 relations 3958 changes 300
C 1 agent-alpha transcript/0000
U 0 1 session s0000
U 1 1 task s0000.t0
R 0 1 1 0 in_session declared
```

Fields, line by line:

| Line | Form | Meaning |
|---|---|---|
| header | `# kerno-testkit assoc-eval corpus v1` | Version comment (not read) |
| seed | `seed <u64> scale <quality\|performance>` | Seed and scale with which it was generated |
| counts | `counts units <n> relations <n> changes <n>` | Totals. The reader compares them with the records and rejects the file if they do not add up |
| change set | `C <step> <actor> <source>` | Opens a change set. The step must be the next after the previous one (1, 2, 3, ...) |
| unit | `U <index> <step> <type> <label>` | Creates a unit in the current change set. The index is its position (0, 1, 2, ...) |
| relation | `R <index> <step> <from> <to> <type> <provenance>` | Creates a relation. The unit indices are those of the `U` lines. Both endpoints must exist and have been created in a step not later than that of the relation |
| provenance | `declared` \| `imported <source>` \| `algorithmic <origin>` \| `inferred <origin> <confidence>` | The ADR-0014 class with its data. The confidence goes from 0 to 1000 (per mille) |

Writing order: each `C` is followed by its `U` lines and then by its `R` lines. The reader rejects a unit or relation whose index is not the next one, or whose step is not that of the change set being read.

Real examples from the file:

```text
C 2 importer wiki-export/0000
R 22 2 8 5 supports imported wiki-export
C 3 consolidator job/consolidate/0000
R 24 3 6 4 related inferred consolidator-v1 473
R 25 3 9 10 same_source algorithmic dedup-v2
```

### 3.2 `queries.txt`

```text
# kerno-testkit assoc-eval queries v1
seed 20261111 scale quality cut 10
counts queries 64
Q TM-01 temporal 506 decision both revises,supports,mentions 3 76-84 - - -
E 6 0
E 104 0
E 166 0
E 504 2
E 505 3
E 507 1
E 508 2
E 509 3
```

This is query `TM-01` as it is in the file: eight expected answers, three of grade 0, one of grade 1, two of grade 2 and two of grade 3.

| Line | Form | Meaning |
|---|---|---|
| header | `seed <u64> scale <scale> cut <n>` | The reader rejects a cut other than 10 (`CUT`) |
| counts | `counts queries <n>` | Number of queries. It must match the `Q` lines read |
| query | `Q <id> <class> <seed> <type> <direction> <types> <depth> <window> <classes> <actor> <source>` | One query. Each field is described below |
| answer | `E <index> <grade>` | One expected answer. It goes after its `Q`. The grade must be from 0 to 3 |

Fields of `Q`, in order:

| Field | Values | Meaning |
|---|---|---|
| `id` | `MH-01`..`MH-16`, `TM-01`..`TM-16`, `CB-01`..`CB-16`, `PV-RE-1`..`PV-RE-4`, `PV-SS-1`..`PV-SS-4`, `PV-AC-1`..`PV-AC-4`, `PV-SR-1`..`PV-SR-4` | The prefix names the family |
| `class` | `multi_hop`, `temporal`, `provenance`, `combined` | Class of the query |
| `seed` | unit index | The starting unit |
| `type` | type of the seed (`decision` or `fact`) | Used by B0 |
| `direction` | `out`, `in`, `both` | Which way the traversal may go, relative to the unit it is on |
| `types` | comma-separated list of relation types | Types the traversal may follow. Never empty |
| `depth` | integer | Maximum number of hops |
| `window` | `-` or `lo-hi` | Closed interval of creation steps of the relation |
| `classes` | `-` or a list of `declared`, `imported`, `algorithmic`, `inferred` | Admitted provenance classes. `-` admits all |
| `actor` | `-` or a name | Actor of the change set that created the relation |
| `source` | `-` or a source | Source of the change set that created the relation |

The `Q` fields are read in this order and without optional fields: a `-` always takes its place. The `E` lines of a query are sorted by index, without repeats, and the reader checks this (the test `quality_set_has_the_agreed_shape` does it).

One query of the set, as it is:

```text
Q PV-SR-1 provenance 244 decision both revises,supports,mentions 3 - - - transcript/0012
```

It is a query with no window and no provenance restriction, which follows the chain of its topic and is limited to the transcript source of the seed's session.

### 3.3 `MANIFEST`

```text
72aebc3de4479fbd92aec7fb223b8d06b168138603661650a5ece76091797757  corpus.txt
18bab21888e511898ae93cc4a3df5e2ede7b64668955869cd2a4e3268bf6c018  queries.txt
```

`sha256sum` format: 64 lowercase hexadecimal digits, two spaces and the file name, one line per file, in this order. It lists exactly the two frozen files. It is checked with:

```text
$ cd crates/kerno-testkit/data/assoc-eval && sha256sum -c MANIFEST
corpus.txt: OK
queries.txt: OK
```

The two checks came out in that order in the run that produced this document.

### 3.4 Regeneration and verification

```text
cargo run --release -p kerno-testkit --example freeze_assoc_eval
```

This command writes the three files from the seed. On the E7-00 tree it rewrote the files with the same content: afterwards `git status` showed no change in `data/assoc-eval/`. The test `regenerated_set_is_byte_identical` checks the same thing without writing anything. The correction of §9 did change the content: it was regenerated once with that command, the `MANIFEST` was checked with `sha256sum -c`, and since then the command again leaves the tree unchanged.

## 4. Baselines B0 and B1

The two baselines are measured over a real `UnitDb` (over `KernoDb`, on the simulated file system). The 300 change transactions are loaded with their actor and source. Before measuring, the test checks that each relation of the corpus exists in the database with its type, its full provenance and its endpoints, and that there are 2 000 active units.

### 4.1 Definitions

- **B0, filter by type**: the existing query by unit type (`UnitSnapshot::query` with the seed's type). It keeps the first ten in the order the query returns, which is order by identifier. Identifiers are assigned in creation order, so the order is the corpus index order. It uses no constraint of the query.
- **B1, one-hop neighbours**: `relations_of` of the seed outwards (`Direction::Out`), then inwards (`Direction::In`), without type filter. Each group comes out in the order of the adjacency index (relation type and then relation identifier). Repeats are removed (the first occurrence wins) and it is cut at ten. It applies no constraint of the query: neither type, nor depth, nor window, nor provenance.

Both baselines are evaluated with the metrics of section 6, with a cut at 10.

### 4.2 Results

Copied from the raw output, [stage7-e700-output.txt](stage7-e700-output.txt), with the decimal point of the output:

| Scope | Queries | B0 R@10 | B0 nDCG@10 | B1 R@10 | B1 nDCG@10 |
|---|---|---|---|---|---|
| overall | 64 | 0.000 | 0.000 | 0.731 | 0.500 |
| multi_hop | 16 | 0.000 | 0.000 | 0.658 | 0.473 |
| temporal | 16 | 0.000 | 0.000 | 0.641 | 0.493 |
| provenance | 16 | 0.000 | 0.000 | 0.864 | 0.510 |
| combined | 16 | 0.000 | 0.000 | 0.761 | 0.523 |

Per query, B1 reaches recall@10 of 1.000 in twelve of the 64 queries: `MH-03`, `MH-07`, `MH-11`, `MH-15`, `PV-RE-1`..`PV-RE-4` and `PV-SS-1`..`PV-SS-4`. B0 has recall@10 of 0.000 in all 64.

### 4.3 How to reproduce

```text
cargo test -p kerno-units --release --test assoc_baseline -- --nocapture --exact baselines_over_the_frozen_set
```

The output of this run is identical, line by line, to [stage7-e700-output.txt](stage7-e700-output.txt) except for the build lines and the timings.

### 4.4 Honest reading of the numbers

**B0 is zero in every query, and that is structural.** B0 uses the seed only to read its type. It always returns the same ten units of the type: for decisions, indices 4, 5, 6, 24, 25, 26, 44, 45, 46 and 64 (the decisions of sessions 0 to 3); for the facts of the four `PV-SS` queries, indices 7 to 14, 27 and 28. None of those units has a grade greater than 0 in any expected answer of its type. That is not a measurement fault: a filter by type that ignores the seed cannot hit a set where relevance depends on the topic. That is why B0 is not a useful floor for comparing rankings. The criterion of §6.2 of the minutes compares with B1 and with the simple order, not with B0.

**B1 does reach relevance in some classes, but orders it badly.** Its mean recall@10 is 0.731, while its mean nDCG@10 is 0.500. It finds many of the relevant answers, but places them behind neighbours of other types and behind distractors. nDCG is what separates B1 from a sensible ranking. For reference, 10 % better than B1 on average would be 0.550, and the absolute threshold of §6.2 is 0.80. These numbers decide nothing in E7-00; they are the base that E7-05 must beat.

**Classes in which one hop already reaches everything relevant.** These are the twelve queries with recall@10 of 1.000:

- **`PV-RE-1`..`PV-RE-4` and `PV-SS-1`..`PV-SS-4`**: these queries are one hop by construction (depth 1, over `related` or `same_source`). B1 reaches exactly the same relevant units. What B1 loses is the order: the distractors (one or two false `related` in `PV-RE`, one or two false `same_source` in `PV-SS`) appear alongside the relevant ones, and nDCG sits between 0.387 and 0.624. These queries measure the provenance filter, not the traversal.
- **`MH-03`, `MH-07`, `MH-11` and `MH-15`**: these are the queries with direction `out` only through `revises`, depth 4. Their relevant answers are the previous decision (grade 3) and the one two positions back (grade 2). The first is an outgoing `revises`. The second is the target of the true consolidator `related`, which leaves the decision. That is why one hop reaches them without traversing the chain. Example: in `MH-03`, seed `545` has the `revises` to `544` (grade 3) and the true `related` to `306` (confidence 808, grade 2); B1 retrieves both. nDCG is 0.552 because the task of the seed and the false `related` occupy positions in front.

In the `temporal` and `combined` classes no single hop reaches 100 %: the maximum recall@10 of B1 is 0.667 in `temporal` and 0.800 in `combined`. There, the time window, the provenance filter or the depth are needed to separate what is relevant from what is reachable.

**Limit of the reading.** The two classes in which one hop suffices are the ones that help B1 most by construction: a one-hop query does not measure multi-hop traversals. That is why B1's result on `PV-RE` and `PV-SS` should not be read as a signal of the method's quality.

## 5. Expected answers and grades

### 5.1 Where they come from

The expected answers **do not** come from traversing the generated graph. They come from the story, with two bookkeeping pieces that the generator keeps apart from the corpus:

- The **chain of each topic**: the list of its decisions in confirmation order, and for each decision its `revises` and its `supports` (the fact that supports it). The traversal of `walk_chain` follows this list, not the relations of the corpus.
- The **constraints** of the query, applied to the relation the traversal wants to follow: type, direction, provenance class, window, actor and source.

The procedure of `expect_chain` for a chain query from seed `s`:

1. Walk the chain backwards and forwards, one `revises` per step, as long as the relation of the step is admitted by the constraints and the depth has not reached the maximum.
2. Each decision reached at depth `d` has grade `3` if `d = 1`, `2` if `d = 2` and `0` otherwise (named distractor).
3. For each decision reached (and for the seed, with `d = 0`) the `supports` that backs it counts if it is admitted and if it fits within the depth. Its fact has grade `3` if `d = 0`, `2` if `d = 1`, `1` if `d = 2` and `0` otherwise.
4. The fact of that evidence has a mention (`mentions`) to a decision on another topic. If the mention is admitted and fits within the depth, the mentioned decision enters with grade 0.

The queries by relation type (`PV-RE`, `PV-SS`) do not use the chain: each answer is a neighbour of the seed by the requested relation (`expect_links`). A neighbour by a true relation gets grade 2 in `PV-RE` (`related`) and grade 3 in `PV-SS` (`same_source`); by a false relation, grade 0.

The test `expected_answers_are_reachable_within_the_bounds` checks that each answer in the file is reachable within the bounds of its query, using a breadth-first search over the corpus. That search **checks** the frozen answers; it does not produce them. The test `every_query_has_a_reachable_distractor` checks that each query names at least one reachable distractor, so that the bounds do not decide the query on their own.

### 5.2 Grades

| Grade | Meaning | Gain `2^g − 1` |
|---|---|---|
| 3 | Decision one revision away (`d = 1`) and evidence of the seed (`d = 0`); true `same_source` (`PV-SS` queries) | 7 |
| 2 | Decision two revisions away; evidence one revision away; true `related` (`PV-RE` queries) | 3 |
| 1 | Evidence two revisions away | 1 |
| 0 | Named distractor (decision three or more revisions away, evidence three or more, mention, false `related`, false `same_source`) | 0 |

Relevant means grade ≥ 1. Distribution in the set: 143 of grade 3, 122 of grade 2, 57 of grade 1 and 211 of grade 0.

### 5.3 The query classes

| Class | Queries | Seed | Constraints | What it tests |
|---|---|---|---|---|
| `multi_hop` (`MH-01`..`MH-16`) | 16 | decisions from session 5 onwards, with at least 4 decisions before and 4 after on their topic, and a mention in the evidence | Four forms by `n mod 4`: `both` with three types and depth 3 (0); `both` with depth 4 (1); `out` only `revises`, depth 4 (2); `in` with `revises` and `supports`, depth 4 (3). No window or provenance | Traversal of the chain and of its evidence |
| `temporal` (`TM-01`..`TM-16`) | 16 | the same | `both`, three types, depth 3 or 4 depending on whether `n` is odd, and a window of steps around the seed's session | Temporal context |
| `combined` (`CB-01`..`CB-16`) | 16 | decisions with `declared` evidence | Window, `declared` provenance only, depth 3 or 4, and the session's actor in the even queries | Provenance and time together |
| `provenance` (`PV-RE`, `PV-SS`, `PV-AC`, `PV-SR`) | 16 | decisions (`PV-RE`, `PV-AC`, `PV-SR`) or facts that have one true and one false `same_source` (`PV-SS`) | `PV-RE`: one-hop `related` with `inferred` provenance; `PV-SS`: one-hop `same_source` with `algorithmic` provenance; `PV-AC`: chain with the session's actor; `PV-SR`: chain with the session's transcript source | Provenance, actor and source |

Two details of these definitions affect the reading:

- A window `lo-hi` is a closed interval of **relation creation** steps, not of units. It always includes the three steps of the seed's session and up to two sessions on each side.
- The `multi_hop` queries have no window, so their answer set is that of the whole chain within the depth.

## 6. Metrics

Implemented in `crates/kerno-testkit/src/assoc_metrics.rs`. A result list is the order of unit indices that the query returns, best first.

Common conventions:

- A unit repeated in the list counts **once**, at its first position. The repetition **occupies** its position: positions keep counting from the original list.
- A unit that does not appear in the expected answers has grade 0.
- A list shorter than `k` leaves the missing positions empty. It is not shortened to `k`.
- With `k = 0` the three metrics are 0.

Let `R` be the set of answers with grade ≥ 1, `r_p` the grade at position `p` (1 is the first), `k` the cut and `gain(g) = 2^g − 1`:

- **recall@k** = |{relevant units among the first `k` positions}| / |R|. If `R` is empty, it is 0.
- **precision@k** = |{relevant units among the first `k` positions}| / `k`.
- **DCG@k** = Σ over the first `k` positions (without repeats) of `gain(r_p) / log2(p + 1)`.
- **IDCG@k** = Σ for `i = 1..k` of `gain(g_i) / log2(i + 1)`, where `g_1 ≥ g_2 ≥ ...` are all the grades of the expected answers sorted from highest to lowest (including the 0s, which add nothing).
- **nDCG@k** = DCG@k / IDCG@k. It is 0 if `k = 0` or if IDCG is 0 (nothing relevant).

Edge cases:

- **Empty results**: DCG = 0 and recall = precision = nDCG = 0.
- **`k` greater than the list**: recall and DCG count only the positions that exist. Precision still divides by `k`: with a single relevant unit and `k = 10`, precision is 0.1. IDCG uses the `k` best expected grades, not the length of the list.
- **Precision and recall with few relevant units**: with more than 10 relevant units, precision@10 never reaches 1. In this set no query has more than 9 relevant units, so recall@10 can reach 1.
- **Grade outside the scale**: `gain` panics with a grade greater than 3. The file reader also rejects it.

Example from the test `hand_computed_example`: results `[1, 9, 2]`, with 1 of grade 3, 2 of grade 2 and 3 of grade 1, with `k = 3`. DCG = 7/log2(2) + 0 + 3/log2(4) = 8.5. Recall = 2/3 and precision = 2/3.

## 7. New tests and mutations

E7-00 added 20 tests, and the correction of §9 added 2 more in `tests/assoc_eval.rs`. The current counts, with the new ones marked, are these:

| Set | After | New | Before (derived) |
|---|---|---|---|
| `kerno-testkit` lib (`assoc_metrics` 9, `sha256` 2) | 90 passed | 11 | 79 |
| `kerno-testkit` `tests/assoc_eval.rs` | 10 passed | 10 | 0 |
| `kerno-units` `tests/assoc_baseline.rs` | 1 passed | 1 | 0 |

The «before» count is the after count minus the new tests, because the E7-00 commit only adds new tests (the 8 of E7-00 plus the 2 of §9 in `tests/assoc_eval.rs`). It was not run on the previous tree.

### 7.1 Mutations

Each new test was run against a version of the code with a mutation. Afterwards the file was restored with `git checkout -- <file>`. After the last restoration of E7-00, no code file appeared modified: `git status` listed only the documents of this WP and `experiments/narrative-threads/.venv`, which does not belong to this work. The line numbers are those of the unmutated code. Those in `src/assoc_eval.rs` were re-placed after the correction of §9, which inserted code above them (those in `assoc_metrics.rs`, `sha256.rs` and `db.rs` did not move). The mutations in the table were run in E7-00, when `tests/assoc_eval.rs` had 8 tests: the «passed» counts are from that date and were not repeated after the correction.

| Test | Mutation (file:line) | Tests that failed |
|---|---|---|
| `regenerated_set_is_byte_identical` | `src/assoc_eval.rs:895`: corpus header `v1` → `v2` | `regenerated_set_is_byte_identical` (7 passed) |
| `manifest_matches_committed_files` | `src/sha256.rs:6`: constant `K[0]` `0x428a2f98` → `0x428a2f99` | `manifest_matches_committed_files` (7 passed) |
| `committed_files_parse_back` | `src/assoc_eval.rs:1114`: source of `algorithmic` with suffix `x` | `committed_files_parse_back` (7 passed) |
| `parser_refuses_a_unit_out_of_order` | `src/assoc_eval.rs:1062`: the reader stops checking the unit index | `parser_refuses_a_unit_out_of_order` (7 passed) |
| `quality_set_has_the_agreed_shape` | `src/assoc_eval.rs:248`: `temporal` is read as `multi_hop` | `committed_files_parse_back`, `quality_set_has_the_agreed_shape` (6 passed) |
| `expected_answers_are_reachable_within_the_bounds` | `src/assoc_eval.rs:225`: `out` and `in` swapped when reading the direction | `expected_answers_are_reachable_within_the_bounds`, `every_query_has_a_reachable_distractor`, `committed_files_parse_back` (5 passed) |
| `every_query_has_a_reachable_distractor` | `src/assoc_eval.rs:1198`: actor `-` read as the literal filter `-` | `every_query_has_a_reachable_distractor`, `expected_answers_are_reachable_within_the_bounds`, `committed_files_parse_back` (5 passed) |
| `performance_scale_is_deterministic` | `src/assoc_eval.rs:110`: performance scale from 1000 to 999 sessions | `performance_scale_is_deterministic` (7 passed) |
| `gains_follow_two_to_the_grade_minus_one` | `src/assoc_metrics.rs:29`: gain `2^g − 1` → `g` | `gains_follow_two_to_the_grade_minus_one`, `k_larger_than_the_result_list`, `hand_computed_example`, `repeats_count_once` (5 passed) |
| `a_grade_outside_the_scale_is_refused` | `src/assoc_metrics.rs:28`: the panic limit raised to 100 | `a_grade_outside_the_scale_is_refused` (8 passed) |
| `hand_computed_example` | `src/assoc_metrics.rs:75`: discount `log2(p + 1)` → `log2(p + 2)` | `hand_computed_example`, `ties_do_not_depend_on_their_order`, `repeats_count_once`, `k_larger_than_the_result_list` (5 passed) |
| `empty_results_score_zero` | `src/assoc_metrics.rs:79`: `nDCG` is 1 with an empty list | `empty_results_score_zero` (8 passed) |
| `no_distractor_is_relevant_and_zero_cut_scores_zero` | `src/assoc_metrics.rs:66`: `precision` without the `k = 0` guard | `no_distractor_is_relevant_and_zero_cut_scores_zero` (8 passed) |
| `expected_without_relevant_units_scores_zero` | `src/assoc_metrics.rs:57`: `recall` without the «no relevant units» guard | `expected_without_relevant_units_scores_zero` (8 passed) |
| `ties_do_not_depend_on_their_order` | `src/assoc_metrics.rs:84`: ideal order ascending instead of descending | `hand_computed_example`, `k_larger_than_the_result_list`, `ties_do_not_depend_on_their_order` (6 passed) |
| `k_larger_than_the_result_list` | `src/assoc_metrics.rs:70`: precision divides by `min(k, length)` | `empty_results_score_zero`, `k_larger_than_the_result_list` (7 passed) |
| `repeats_count_once` | `src/assoc_metrics.rs:42`: repeat removal disabled | `repeats_count_once` (8 passed) |
| `standard_vectors` | `src/sha256.rs:6`: the same constant `K[0]` as above | `standard_vectors`, `padding_boundaries` (lib: 88 passed, 2 fail). The integration one, `manifest_matches_committed_files`, also fails |
| `padding_boundaries` | `src/sha256.rs:32`: padding threshold `< 56` → `< 55` | `padding_boundaries` (only this; `standard_vectors` still passes) |
| `confidence_alone_does_not_separate_related_links` | §9: the old ranges (true 700–950, false 300–600) temporarily restored and the files regenerated | `confidence_alone_does_not_separate_related_links` (9 passed): «a confidence threshold separates every true related link from every false one» |
| `same_source_truth_is_not_readable_from_one_field` | `tests/assoc_eval.rs`: the 0.9 hit bound lowered to 0.5 (restored afterwards) | `same_source_truth_is_not_readable_from_one_field` (9 passed): «offset of the end: a threshold gets 0.517 of the same_source links right». It was not run against the generator from before §9, whose `same_source` links were separated by distance (see §9) |
| `baselines_over_the_frozen_set` | `kerno-units/src/db.rs:749`: `relations_of` keeps only the first edge of each range (change in the reference path used by B1) | `baselines_over_the_frozen_set`, in `tests/assoc_baseline.rs:130` (the «relation missing or altered» assertion) |

Commands used for each group:

```text
cargo test -p kerno-testkit --all-features --release --test assoc_eval
cargo test -p kerno-testkit --all-features --release --lib assoc_metrics
cargo test -p kerno-testkit --all-features --release --lib sha256
cargo test -p kerno-units --release --test assoc_baseline
```

Two observations about the mutations:

- Tests that only read the frozen files do not see changes in the generator. That is why the mutation of `build_queries` was not used: the shape tests would have stayed green. The mutations were made on the reader, which those tests do use.
- The mutation of the baseline changes code in `kerno-units`, which is the reference path of `relations_of`. It is intentional: B1 depends on it, and the test must fail if that path returns fewer neighbours than there are.

### 7.2 Verification gate

The gate of the assignment, `cargo fmt --all --check && cargo test -p kerno-testkit --all-features --release`, passes on this tree. The output is recorded in the closing report of this task.

## 8. Limits of a synthetic set

This set measures the implementation over a generated narrative. It does not measure the quality of memory in real use. The limits the reader must keep in mind:

1. **Same author for the narrative and for the queries.** The generator, the query families and the grade rules were written by the same person, before the retrieval code was written. Freezing protects against adjusting the answers to the code, but not against a generator assumption that the code shares with it.
2. **The grades are a choice.** Grades 3, 2 and 1 by distance and the grade-0 distractors are conventions of this document, not relevance judgements by people.
3. **The `related` confidence is informative, but not sufficient.** The true ones have confidence 450–950 and the false ones 300–800 (§9). A ranking that uses only confidence gets 0.675 of the 573 links right with the best threshold. Even so, confidence still helps, and it must be taken into account when reading the `PV-RE` class. One structural signal has not been removed: the true target is two steps back in the chain of its topic, and so it tends to be closer in index than the false one. The best threshold on index distance gets 0.729 right over the corpus, and over the four `PV-RE` queries (13 answers, 5 of grade 0) a threshold on that distance does separate them without error. With so few answers it is a coincidence of the chosen seeds, not a property of the generator, but it is worth remembering when reading `PV-RE`.
4. **The `same_source` source matches `cites`.** The true and false `same_source` pairs are consistent with the cited sources, so the set has no case in which the algorithmic relation and the story disagree in a subtle way.
5. **Regular structure.** Each session has exactly three decisions on one topic, eight facts and five sources; each topic uses a linear chain. Real memory has irregular structures, branches and decisions that contradict each other.
6. **Small vocabulary.** Twelve topics, three agents, three transcript sources, one importer and one consolidator. The labels are strings such as `s0000.d0`, with no natural text.
7. **No vectors.** The set contains no vectors. ADR-0011 says that the caller supplies the vectors, so the vector signal is not measured by this set on its own: E7-04 will have to provide vectors to measure it.
8. **Sixty-four queries.** With 16 per class and 1 to 9 relevant per query, the variance of a mean is high. A small difference between two methods is not significant in this set.
9. **Performance not measured.** The 20 000-unit scale is generated and tested in form, but E7-00 does not measure times. The measurements of §6.3 of the minutes are from E7-05.

What the set does guarantee, and what it is designed for: that any change in the generator, in the format or in the reading of the files is detected; that the expected answers are reachable within their bounds; and that the baseline B1 can be measured over them.

## 9. Correction before use: single-field leaks

This section records a correction to the generator made the same day as the freeze (2026-11-11), **before any recovery, traversal or ranking code existed**: at the time of the correction the only code that read the set was that of the two baselines B0 and B1. That is why the files of `data/assoc-eval/` were regenerated and the threshold of the minutes was not touched. No threshold of [stage7-record.md](stage7-record.md) changed.

### 9.1 What was leaking

The confidence of the inferred `related` links was drawn from two ranges that did not overlap: 700–950 for the true ones and 300–600 for the false ones. A signal that read only the confidence (a threshold between 600 and 700) classified the 573 inferred links correctly. The set would have rewarded a trivial rule instead of the quality of retrieval. The original version of this document described it in §2.6 and §8, but treated it as a property of the set and not as a defect.

| | Before | Now |
|---|---|---|
| Confidence of true `related` (per mille) | 700–950 | 450–950 |
| Confidence of false `related` (per mille) | 300–600 | 300–800 |
| Overlap | none | 450–800 |
| Best confidence threshold | 1.000 accuracy | 0.675 accuracy |

The ranges are inclusive and come from the same seeded generator. They live in the constants `TRUE_CONFIDENCE` and `FALSE_CONFIDENCE` of `src/assoc_eval.rs`.

### 9.2 Which other fields were checked

The generator was reviewed looking for another field that, on its own, separated the relevant from the irrelevant. The frozen files were read with a temporary program (not kept) that, for each query family, compared the grade-0 answers with the graded ones by each field, and that also measured over the whole corpus the true and false `related` and `same_source` links. «Separates» means that a threshold in either direction, or a disjoint set of values, makes no error.

| Field | Where | Result before | Result now |
|---|---|---|---|
| Confidence | `related` | **Leak**: separates without error | Overlaps; best threshold 0.675 |
| Relation type | all families | `mentions` gives only grade 0, but `revises` and `supports` give grade 0 and higher: does not separate | Same |
| Provenance class | all | No leak (`related` is always `inferred`, `same_source` always `algorithmic`; in the chains there are `declared` and `imported` with both grades) | Same |
| Provenance origin | all | No leak (`consolidator-v1` and `dedup-v2` constant in each family; in the chains `wiki-export` appears with both grades) | Same |
| Actor | all | No leak (`consolidator` constant in `PV-RE` and `PV-SS`; in the chains all actors appear with both grades) | Same |
| Change-set source | all | No leak (`job/consolidate` constant; `transcript` and `wiki-export` with both grades in the chains) | Same |
| Unit type and label (`d0`..`d2`, `f0`..`f7`) | chains, `PV-RE` | No leak | Same |
| Creation position in the change set | `related` | Partial leak: the true one was written before the false one; the parity of the relation index was right 0.806 | Random order; best threshold 0.532 (parity: 0.529) |
| Creation position | `same_source` | Partial leak: the true ones were written first; the position within the set was right 0.958 | Random order; 0.537 |
| Offset of the endpoint within the session, index distance and parity | `same_source` | **Leak**: the true ones were always `(0,5)`, `(1,6)`, `(2,7)`, at distance 5 and with the endpoint at `f5`..`f7`; the false ones at distance 1 to 3 and with the endpoint at `f3` or `f4`. They separated without error | Random pairs and orientation; no threshold exceeds 0.520 |
| Session membership, creation step, index ranges | `related`, `same_source` | No leak: all links are written in the consolidator's step, and the ranges overlap | Same |
| Index distance between origin and destination | `related` | No leak in the corpus (best threshold 0.733); the true one is usually closer because of the chain's structure | Best threshold 0.729; see §8 point 3 |
| Depth, direction and step of the relation that leads to the answer | chains | No leak | Same |

The `same_source` leak had a structural cause: the source cited by a fact was `j mod 5`, so the only pairs sharing a source were `(j, j+5)`, and the truth depended on the index difference. It is corrected by shuffling, per session, which fact cites which source (three pairs share a source, and which ones changes), choosing the false ones among all pairs with different sources, and writing the six links in random order and orientation. The seeds of `PV-SS` are now any fact with one true and one false link, not only `f0`..`f2`. The `related` links of each decision (true and false) are now written in random order.

### 9.3 What changed in the files and in the figures

The counts of units, relations by type and by provenance, change sets and queries by class do not change. What changes is the content of the `cites`, `related` and `same_source` relations, the seeds of `PV-SS` and of the other queries (the generator consumes random numbers in another order), the expected answers (527 → 533) and the B1 figures. The two `MANIFEST` hashes and the raw output of B1 are updated in this document. The queries remain 64, with 16 per class (at least 60 and at least 12 per class).

### 9.4 Test that guards it

`tests/assoc_eval.rs` adds `confidence_alone_does_not_separate_related_links`, which fails if a threshold on confidence separates the true `related` links from the false ones without error or hits 0.85 or more; and `same_source_truth_is_not_readable_from_one_field`, which does the same with the offset, distance, parity and creation position of the `same_source` links (bound 0.9). The truth is read from the corpus (two `revises` back; same source cited), not from the generator, and the first test also checks that this reading matches the grades of the `PV-RE` queries. The first test was seen to fail with the old ranges: they were temporarily restored, the files regenerated, and the test failed with «a confidence threshold separates every true related link from every false one»; then the new ranges were restored and it was regenerated again.

## 10. Related documents

- [Stage 7 minutes](stage7-record.md) §6 (set, thresholds) and §7 (decisions)
- [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md): exact vectors first, decided in E7-00
- [ADR-0014](../adr/0014-physical-representation-of-relations.md): the four provenance classes
- [Raw output of E7-00](stage7-e700-output.txt), regenerated after the correction of §9
