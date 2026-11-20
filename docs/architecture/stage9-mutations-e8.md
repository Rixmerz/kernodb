# E9-15: mutation evidence of E8-06 and E8-07

Package E9-15 of Stage 9 ([minutes](stage9-record.md), §3; decision D20 (ii); finding F-10 of [`stage8-closure.md`](stage8-closure.md)). This document records, for each test of `crates/kerno-units/tests/consolidation.rs` (E8-06) and of `crates/kerno-units/tests/derived_measure.rs` (E8-07), a mutation applied to a copy of the tree, the mutated line and the output of the test. It changes no code nor tests.

## 1. Method

- **Base:** commit `5f7aadc`. The lines cited are those of that tree. Every path in the column "File:line" is complete from the root of the repository: `crates/kerno-units/src/consolidation.rs` (production) and `crates/kerno-units/tests/consolidation.rs` (the test) are different files, and **no mutation touches the consolidation test**.
- **Copy:** a single copy of the tree (without `.git` nor `target`) with its own `target` directory, compiled only in release and deleted at the end. The working tree was not touched.
- **One mutation at a time.** Each mutation was applied with `sed` to one line of the copy; before compiling, `git diff --no-index` against the original file confirmed that the change was applied. After the run the file was restored from the original and `git diff --no-index --quiet` confirmed it; at the end, `git diff --no-index --stat crates <copy>/crates` gave no differences (exit 0).
- **Command of each mutation** (in the copy): `cargo test -p kerno-units --all-features --release --test consolidation` (rows M1 to M15) or `--test derived_measure` (rows D1 to D4).
- **"Result" column:** the totals of the line `test result: FAILED. N passed; M failed; ...` of that run, copied from the record. Each run ended with `error: test failed` (exit code 101).
- **What is mutated.** Production code when the test checks production. Where the logic checked lives in the measurement test itself (`check_manifest`, `score`), that function is mutated; where the input is a frozen datum, the datum is altered in the copy.
- **Machine:** 1-minute load (`load1`) 0.77 at the start and 1.78 before repeating D3 and D4; 4.2 GB free before copying. No other build work ran at the same time (§3.2).

### Test count

- Baseline in the copy, without mutation: `consolidation` → `test result: ok. 30 passed; 0 failed` (15 tests × 2 engines); `derived_measure` → `test result: ok. 4 passed; 0 failed`.
- This package adds and removes no tests: the count before and after is the same (T-CNT).

### The two engines

The consolidation tests are instantiated with `emulated_log` and with `kerno_db` (macro `engines!`). In **each** row of §2 the output contains `test emulated_log::<test> ... FAILED` **and** `test kerno_db::<test> ... FAILED` for the target test. `quality_over_the_frozen_set` measures the two engines within a single test.

No mutation left the target test green: there is no finding of that kind (§4.4).

## 2. Tests of `crates/kerno-units/tests/consolidation.rs` (E8-06): 15 tests, 15 mutations

| # | Test | File:line | Change | Result |
|---|---|---|---|---|
| M1 | `a_summary_records_its_exact_sources_and_a_full_descriptor` | `crates/kerno-units/src/consolidation.rs:108` | `len().min(self.max_bytes)` → `len().min(self.max_bytes + 1)` | 20 passed; 10 failed |
| M2 | `errors_say_what_happened_without_content` | `crates/kerno-units/src/consolidation.rs:133` | text `reserved name` → `reserved nam` | 28 passed; 2 failed |
| M3 | `a_summary_with_no_sources_is_an_error` | `crates/kerno-units/src/consolidation.rs:198` | `if units.is_empty() {` → `if false {` | 28 passed; 2 failed |
| M4 | `the_kind_parameter_is_reserved_and_separates_summaries` | `crates/kerno-units/src/consolidation.rs:154` | `.is_some()` → `.is_none()` (widens) | 2 passed; 28 failed |
| M5 | `a_summarizer_that_gives_nothing_leaves_a_failed_summary` | `crates/kerno-units/src/consolidation.rs:203` | `summarizer.summarize(inputs)` → `Some(Vec::new())` | 26 passed; 4 failed |
| M6 | `the_original_stays_readable_through_every_summary_operation` | `crates/kerno-units/src/consolidation.rs:240` | `self.value(id)` → `self.value(id).or_else(\|\| Some(b"x".to_vec()))` | 22 passed; 8 failed |
| M7 | `a_source_change_marks_the_summary_stale_in_the_same_changeset` | `crates/kerno-units/src/derived_stale.rs:599` | `if inner.registry.transition(..)` → `if false && inner.registry.transition(..)` | 18 passed; 12 failed |
| M8 | `a_new_method_or_version_creates_a_new_summary_and_supersedes_the_old` | `crates/kerno-units/src/consolidation.rs:230` | `summarizer.version()` → `summarizer.version() + 1` | 24 passed; 6 failed |
| M9 | `rebuilding_needs_the_summarizer_that_made_the_summary` | `crates/kerno-units/src/consolidation.rs:166` | `d.version() == s.version()` → `true` | 28 passed; 2 failed |
| M10 | `deleting_any_one_source_deletes_the_summary_in_the_same_changeset` | `crates/kerno-units/src/derived_stale.rs:580` | `crate::suppression::remove_dependents_of_unit(&mut inner, unit);` → `let _ = unit;` | 24 passed; 6 failed |
| M11 | `a_stale_or_superseded_summary_is_deleted_with_its_source_too` | `crates/kerno-units/src/suppression.rs:59` | `inner.registry.dependents_of_unit(unit);` → `inner.registry.dependents_of_unit(unit).into_iter().take(1).collect::<Vec<_>>();` | 26 passed; 4 failed |
| M12 | `summaries_are_rebuilt_on_open_from_their_sources` | `crates/kerno-units/src/consolidation.rs:183` | `if !made_by(d, summarizer)? {` → `if false {` | 28 passed; 2 failed |
| M13 | `a_summary_leaves_nothing_on_disk` | `crates/kerno-units/src/consolidation.rs:201` (insertion before the line) | `build_summary` opens a `ChangeSet`, creates a unit (`payload` `m`) and commits, before building | 26 passed; 4 failed |
| M14 | `reads_of_a_source_are_the_same_with_and_without_summaries` | `crates/kerno-units/src/consolidation.rs:201` (insertion before the line) | `build_summary` updates the first source to `zz` and commits, before building (widens) | 8 passed; 22 failed |
| M15 | `a_failed_commit_leaves_the_summaries_and_the_sources_as_they_were` | `crates/kerno-units/src/derived_stale.rs:551` | `let info = self.cs.commit()?;` → `match self.cs.commit() { Ok(i) => i, Err(e) => { inner.values.clear(); return Err(e); } }` | 28 passed; 2 failed |

Other tests that the same mutation turns red (besides the target, always in both engines): M5, `a_summary_records_its_exact_sources_and_a_full_descriptor`; M8, `a_summary_leaves_nothing_on_disk` and `the_original_stays_readable_through_every_summary_operation`; M10, `a_stale_or_superseded_summary_is_deleted_with_its_source_too` and `the_original_stays_readable_through_every_summary_operation`; M11, `deleting_any_one_source_deletes_the_summary_in_the_same_changeset`; M13, `reads_of_a_source_are_the_same_with_and_without_summaries`. In M2, M3, M9, M12 and M15 the target is the only one in red.

## 3. Tests of `crates/kerno-units/tests/derived_measure.rs` (E8-07): 4 tests, 4 mutations

| # | Test | File:line | Change | Result | `FAILED` lines |
|---|---|---|---|---|---|
| D1 | `quality_over_the_frozen_set` (P3) | `crates/kerno-units/src/derived_stale.rs:589` | `s.rev != head` → `s.rev == head` | 3 passed; 1 failed | `quality_over_the_frozen_set` |
| D2 | `the_manifest_of_the_frozen_set_matches` | `crates/kerno-testkit/data/derived-eval/vectors.txt` (end of the file) | a line break is added at the end of the file | 1 passed; 3 failed | `the_manifest_of_the_frozen_set_matches`, `quality_over_the_frozen_set`, `a_manifest_that_does_not_match_refuses_to_measure` |
| D3 | `a_manifest_that_does_not_match_refuses_to_measure` | `crates/kerno-units/tests/derived_measure.rs:92` | `if &got != digest {` → `if false && &got != digest {` | 3 passed; 1 failed | `a_manifest_that_does_not_match_refuses_to_measure` |
| D4 | `unjudged_units_are_excluded_and_counted` | `crates/kerno-units/tests/derived_measure.rs:119` | `.filter(\|u\| judged.contains(u))` → `.filter(\|_\| true)` | 3 passed; 1 failed | `unjudged_units_are_excluded_and_counted` |

**D3 and D4 were run twice** in the same copy, with the same result both times: `test result: FAILED. 3 passed; 1 failed`, and `quality_over_the_frozen_set` green. An earlier review of this record noted `2 passed; 2 failed` for both rows, with `quality_over_the_frozen_set` also red; it was not reproduced here. By the code, that test cannot see either of the two mutations: with D3, `check_manifest` still returns `Ok` because the frozen files match the `MANIFEST`; with D4, `score` only changes figures that the test prints (the means and Q5) and compares them with no value. Its assertions are the equality of the two engines (both with the same mutation) and P3/P4, which do not read `score`. If a run shows that test red with D3 or D4, the cause is outside the mutation and is recorded separately; the figure of the table is the one observed.

## 4. Result and limits

- **T-MU1:** tests with their mutation recorded = tests in scope (15 of consolidation and 4 of measurement, 19 of 19). It closes F-10 of E8 as far as E8-06 and E8-07 are concerned.
- Mutations M4 and M14 are wide: the target test fails, but also almost all the others. They show that the test sees the mutation, not that it is the only one that sees it. M13 and M14 insert a write into `build_summary`; for "a summary writes nothing" there is no narrower mutation with this code.
- `quality_over_the_frozen_set` also checks P4 and the equality of the two engines; D1 exercises only P3. No separate mutation was recorded for P4 nor for the report identical between engines.
- Only the two test files were run, before and after each mutation, in the copy. This package does not change code nor tests.
