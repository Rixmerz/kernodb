# Evaluation set of E8-10, freezing (E9-19)

| Field | Value |
|---|---|
| Status | **Frozen.** Not measured: the only figure computed is the non-saturation check (§3) |
| Package | E9-19 of [stage9-record.md](stage9-record.md) §3 |
| Thresholds | T-E10-V1, T-E10-V2 and T-E10-R of §5 of the minutes, unchanged (D22) |
| Measurement | E9-20, once only, on these files |

## 1. What it is

A new set, with its own seed, to measure E8-10. It is built on the same synthetic history as the Stage 8 set (`derived_eval`), with one difference: **all queries carry a query vector, in the four classes**. In the Stage 8 set only the vector-neighbourhood class carried it, and that is why Q4 could not be computed per class (F-06).

| Element | Value |
|---|---|
| Module | `crates/kerno-testkit/src/derived_eval2.rs` |
| Seed | `20261203` (different from those of Stage 7 and Stage 8) |
| Scale | quality: 2 000 units, 300 changes, 100 queries |
| Queries per class | 25 in each of the four classes, all 100 with a vector |
| Query vector | Vector neighbourhood: close to the base of the seed decision, as in Stage 8. The other classes: the vector of the seed unit with noise of ±150 per component |
| Judgements | Grades 1 to 3 come from the true links of the history, not from any engine. For each added vector, the ten nearest units by cosine that the walk did not judge are judged with grade 0, so that the exact top 10 of each query is fully judged |

The set does not measure the quality of any model: it measures mechanics. The vectors come from a function of the seed.

## 2. Frozen files

`crates/kerno-testkit/data/derived-eval-2/`, written by `cargo run --release -p kerno-testkit --example freeze_derived_eval2`. Running it again changes nothing, and `tests/derived_eval2.rs` checks it.

```text
4e599e109656f32a1d63c6b720c47942d23a11a28808c4bca3af275a9ae84c34  corpus.txt
ab84850aeaa94cf57ebefbd7e6b9626b79c41a308693c29508eb1e7547fab25a  vectors.txt
aea4e1208266c1fefb6cfb8e84718a6748488493cad44ad22f632e25afb25d84  queries.txt
```

This is the content of `MANIFEST` (T-E10-R). Before measuring, E9-20 checks that it matches; if not, nothing is measured.

## 3. Non-saturation check (T-E10-V2)

The arm without signal ranks by the query vector alone (exact cosine search). Its nDCG@10 overall must be ≤ 0.96, or the set is regenerated with another seed **before** freezing it. It was computed once, with the first seed:

| Class | nDCG@10 without signal |
|---|---|
| vector neighbourhood | 0.7018 |
| temporal | 0.3483 |
| provenance | 0.2674 |
| combined | 0.3687 |
| **overall** | **0.4216** |

0.4216 ≤ 0.96, so there was no regeneration and the seed is the first one tried. This is the only quality figure computed with the set; the arm with signal and no other is measured. The example `freeze_derived_eval2` repeats the check and writes nothing if the limit is exceeded, and the test `arm_without_signal_is_not_saturated` repeats it over the frozen files.

## 4. Tests

`crates/kerno-testkit/tests/derived_eval2.rs`: 8 tests (the count of `kerno-testkit` goes from 242 to 250). None opens a database, so there is no engine to vary. Each was seen to fail with a mutation on a copy of the tree (seed, omitted vector, saturation limit, neighbour judgements, one byte of a file, vector removed from a query, grade-0 judgements removed, added dependency).

## 5. What comes next

E9-20 measures on these files, once. No file in this package and no threshold is changed after measuring.
