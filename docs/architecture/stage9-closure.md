# Closure of Stage 9

| Field | Value |
|---|---|
| Date | 2026-11-13 |
| Package | E9-CL of [the Stage 9 minutes](stage9-record.md) (§3) |
| Status | **Closure approved by the owner in person on 2026-11-13** (written that same day). The approval covers the closure of the stage and decisions D1, D2, D3, D4, D12, D15 and D16 as they were recorded; it does not declare 1.0 (D12) nor replace the approval of the legal text of the CLA, which is not written (D15). All packages of the stage are merged; E9-02 was merged on 2026-11-13 (squash `2697980`, PR 78). The stage is closed as engineering work. Decisions D1, D2, D3, D4, D12, D15 and D16, retained on 2026-11-12, are **taken on 2026-11-13 by express delegation of the owner**, revocable (§2). What depends on them and is not done is declared "unverified" or "not done" (§3), not done |
| Scope of this document | Documentation. It changes no code or tests. It compares the documents of each package with the repository and records the limits that each one declares; it **does not repeat** the measurements or the review of each package |
| What is not claimed | That the independent review of each package (minutes §4.10) was redone here: its records are in each package. That there is a 1.0: it is not declared; D12 (taken on 2026-11-13) conditions it on D13 and on real use outside the team |

## 1. Status of the packages, checked in the repository

Checked with `git log` and with the files. All packages are merged, including E9-02, merged on 2026-11-13 (squash `2697980`, PR 78).

| Package | Status | What shows it in the repository |
|---|---|---|
| E9-00 Gateway | Merged | ADR-0012 and ADR-0014 aligned; decisions in the roadmap |
| E9-01 Fixtures | Merged | [stage9-fixtures.md](stage9-fixtures.md); `crates/kerno-units/tests/layer_fixtures.rs`; `tests/vectors/derived-v2/` |
| **E9-02 Public identifiers** | **Merged on 2026-11-13 (D1)**, squash `2697980`, PR 78 | `crates/kerno-units/src/ids.rs` and `crates/kerno-units/tests/public_ids.rs` (49 tests listed without `test-support` and 79 with it, in both engines; fixture `tests/vectors/public-ids-v3/`; [mutations](stage9-mutations-e9-02.md)). I-27 is **verified** |
| E9-03 Minimised audit | Merged | [stage9-audit.md](stage9-audit.md); `tests/redaction_units.rs` |
| E9-04 to E9-06 Export | Merged | `src/export.rs`, `src/import.rs`, `tests/export_import.rs`, `tests/export_physical.rs`; [draft](../file-format/export-v0-draft.md) |
| E9-07 Public reclaim | Merged | trait `Reclaim` in `kerno-core/src/contract.rs`; `src/reclaim.rs`; `tests/reclaim.rs` |
| E9-08a to E9-08g Wrapper | Merged | `grep -rn "pub fn engine" crates/kerno-units/src` with no results (T-EN1); [uses of `engine()`](stage9-engine-uses.md) |
| E9-09 Derived reads | Merged | `src/derived_query.rs`; `tests/derived_query.rs` |
| E9-10, E9-11 Vectors and errors | Merged | `tests/derived_replace.rs`; `tests/errors_api.rs` |
| E9-12 Derived stress | Merged | `tests/derived_stress.rs` |
| E9-13 Decoders | Merged | `tests/decoders_robust.rs` |
| E9-14 Fuzzing | Merged | section "Stage 9 run on the final code" of `crates/kerno-storage/fuzz/README.md` |
| E9-15 Mutations of E8 | Merged | [stage9-mutations-e8.md](stage9-mutations-e8.md) |
| E9-16 Review of K-E3-7 | Merged | [stage9-k-e3-7.md](stage9-k-e3-7.md) (with the result of E9-22 in its §8) |
| E9-17, E9-18 SQLite | Merged | [E9 report](../testing/sqlite-report-e9.md); previous report without third-party names |
| E9-19, E9-20 Set and measurement | Merged | [set](stage9-evaluation-set.md); [measurement](stage9-measurement.md) and its [raw output](stage9-measurement-output.txt) |
| E9-21 Example | Merged | `examples/agent-memory/`; disposition of the fifteen points in `FRICTION.md` |
| E9-22 Findings K1 and K2 | Merged (package opened after E9-16) | [stage9-k-e3-7.md](stage9-k-e3-7.md) §8 |

**Dependencies (I-31).** `git log 52f026f..HEAD -- Cargo.toml Cargo.lock` returns no commits. No new dependency in the stage.

## 2. Decisions

**Taken by delegation of the owner (2026-11-12). All revocable by them:** D5, D6, D7, D8, D9, D10 (as far as the draft goes), D11, D13 (as a criterion), D14, D17, D18, D19, D20, D21, D22 and D23. The table with the option of each one is in [minutes §8.1](stage9-record.md). If the owner revokes one, whatever depends on it is reviewed; none changes the format on disk.

**Retained on 2026-11-12 and taken on 2026-11-13 by express delegation of the owner, revocable** (the owner, in person, asked the delegate to take the best decision on each pending point). Each one is taken with the option that the minutes recommended. What remains unverified or not done:

| Decision | Option taken | What remains **unverified** or not done |
|---|---|---|
| D1 | (a): opaque random token with a persisted mapping (option 1 of ADR-0005) | E9-02 merged on 2026-11-13 (squash `2697980`, PR 78). ADR-0005 stays **Accepted** with that option |
| D2 | (a): per-block AEAD, as the design to use if encryption is built | Nothing is implemented. ADR-0006 remains **Pending** |
| D3 | (b): no encryption in 1.0; 1.0 does not promise confidentiality at rest or key management, and declares it | K-1 to K-4 of ADR-0009 unmeasured, because there is no encryption |
| D4 | (a): no authenticated integrity in 1.0; the limit stays in the threat model §6.6 and `verify()` with `deep` is the resource | H-2 stays closed as an accepted and documented limit (§5 of this document, §6) |
| D12 | (b): 1.0 is declared only after D13 and real use outside the team | **1.0 is not declared**. The export format is not frozen and no semver is promised |
| D15 | (b) as the direction: broad non-exclusive licence with the right to relicense | The legal text of the CLA is **not written** and needs the owner's approval before accepting any external contribution. None is accepted |
| D16 | (b): ADR-0016 stays **pending confirmation** until G1 to G7 are reviewed. **Closed on 2026-11-13:** review done, ADR-0016 **accepted and confirmed** with amendments | The review was done as a separate task ([ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md), "Review of 2026-11-13"). Nothing in the stage depends on ADR-0016. Open work it leaves: the plan explanation does not list the cache, the creation evidence is constant (H-3) and `set_key` leaves no record |

## 3. Common criteria (minutes §4) and thresholds (minutes §5)

The figures in this table are **cited** from the documents of each package. Only the tests of §4 (invariants) were rerun.

| Code | Result | Source and caveat |
|---|---|---|
| T-EX1, T-EX2 | Met according to E9-05 and E9-06 | `export_import.rs`, `export_physical.rs`. The format is an **unfrozen draft** (D10, D12) |
| T-RE1, T-RE2 | Met according to E9-07 | `tests/reclaim.rs`. It is not a promise of physical deletion (ADR-0009, option 3) |
| T-FZ1 | Met with caveats | 15 targets, 1 801 s each, without failures. Caveats in §5, point 7 |
| T-PR1 | Met according to E9-13 | `decoders_robust.rs` |
| T-ST1 | Met with a limit | Caveat in §5, point 4 |
| T-FR1 | 15 of 15 with disposition | `examples/agent-memory/FRICTION.md`. Point 6: E9-02 (`PublicId`) merged on 2026-11-13; the identity part of point 3 is a format change that D12 does not decide |
| T-EN1 | Met | `grep -rn "pub fn engine" crates/kerno-units/src`: no results |
| T-MU1 | Met with caveats | E9-15 (19 of 19 tests), E9-03, E9-20. Caveats in §5, points 5, 6, 9 and 10 |
| T-SQ1, T-NM | Met for the new report | Load precondition noted in the report; no third-party names other than SQLite. The load of the measurements of E9-14 and E9-20 was not that of an idle machine (§5) |
| T-CNT | **Not rerun here** | Each package noted its count; this closure adds no tests |
| T-E10-V1, V2, R, Q1, Q2, Q3, S1, S2, Q5 | Met in the single measurement | [stage9-measurement.md](stage9-measurement.md) §2. Caveats in §5, point 9 |

**Latency.** No absolute threshold (D7). The P1 and P2 ratios were measured with the machine loaded.

## 4. Invariants

`cargo test -p kerno-units --all-features --release --test derived_suppression --test consolidation --test reclaim --test redaction_units` was run in release mode:

```text
consolidation:      test result: ok. 30 passed; 0 failed
derived_suppression: test result: ok. 19 passed; 0 failed
reclaim:             test result: ok. 10 passed; 0 failed
redaction_units:     test result: ok. 3 passed; 0 failed
```

| Invariant | Result of the closure |
|---|---|
| **I-29** for `kerno-units` | **Promoted** (D19, E9-03 passes). With the limit of §5, point 5 |
| **I-42** | **Promoted** (D19): verified again with `reclaim`, which does not write to the layer (`reclaim_changes_no_visible_state`) |
| **I-43** | **Promoted** (D19): verified again with `the_original_stays_readable_through_every_summary_operation` and `reads_of_a_source_are_the_same_with_and_without_summaries` |
| **I-28** | **Not applicable in 1.0** (D19; D3 taken on 2026-11-13: no encryption in 1.0). What can be said today: `verify()` finds no unknown files in the directory (`lifecycle_soak.rs`) |
| **I-30** | Verified **for the draft** and for reimport. The freezing of the format is not done: D12 (taken on 2026-11-13) leaves it for after D13 and real use |
| **I-27** | **Verified** (2026-11-13, in E9-02 merged that day) by `crates/kerno-units/tests/public_ids.rs` (`tampered_text_is_refused_with_its_own_error`, `tampered_but_well_formed_tokens_name_nothing`); mutation record in [stage9-mutations-e9-02.md](stage9-mutations-e9-02.md) |
| **I-31** | Verified by review of dependencies (§1) |
| I-N1 (reclaim) | Recorded as **I-44** in [invariants](invariants.md) §2.7 |
| I-N2 (wrapper) | Recorded as **I-45** in [invariants](invariants.md) §2.7 |

The promotion of I-29, I-42, I-43, I-44 and I-45 is a choice **by delegation and revocable** (D19). The two new ones (I-44, I-45) are proposals from the minutes that this closure numbers; they are not approved by the owner.

## 5. Known limits and open points

Each point is a limit or a pending item. Point 1 is resolved; the rest remain open.

1. **Roadmap, status of Phase 1. Resolved on 2026-11-13** by express delegation of the owner: the phase table and the task table of SAD §20 in `docs/roadmap/README.md` say "Closed" for Phase 1 with the date 2026-11-11, which is the one the text of the section "Phase 1: work plan" gives. The Phase 1 report (`docs/testing/f1-report.md`) is dated 2026-10-29 and describes the status at that time, before the closure.
2. **K-E3-7 (E9-16 and E9-22).** *K1*: corrected from the end that keeps its adjacency entry; **still open** from the end that lost it, where `relations_of` and `query_relations` return a short answer without `Fallback`; `verify_units` (UNIT-36) reports it. *K2*: corrected for `IDX_PRIMARY` with the count intact; open with the count lowered and for `IDX_TYPE`. *K3*: not reproduced. *K4* (`change_set_info` does not cross the members with their `REV_META`) and *K5* (`get_relation` does not check `created_seq` or the ends): **still open, minor and without change**. The independent review of E9-16 is listed as pending in its record.
3. **Message of `NoValidSources`.** The text "no source of the derived datum is valid any more" was not changed because `crates/kerno-units/tests/consolidation.rs` (line 496) fixes that text. Improving it would require changing that test, which belongs to another package.
4. **E9-12, derived stress (F-09).** The test checks that no derived item remains `ready` after a change in its source has come back. **A late lock after the commit still passes**: the exact lock order of the saved commit has no deterministic test. F-09 is covered only in part.
5. **E9-03, redaction.** There is no mutation evidence for the paths of the `verify` report or the observations: the text of `VerifyFinding` only carries record keys and there is no production line that the test could be seen to fail on without inventing a leak. The test checks that those paths were exercised and that no text carries a sentinel. Fragments under 8 bytes are not searched for.
6. **E9-08g.** No test fails if `compact()` loses its poisoning check or if it widens its range (finding F3 of its review).
7. **E9-14, fuzzing (H-7).** The machine was **not** idle (mean load of about 4 on 4 CPUs). `wal`, `manifest` and `open` reached only between 4.5 and 6.7 million executions, two orders of magnitude fewer than the pure decoders; 30 minutes is little for them. **H-7 is marked verified with those limits**: 15 targets, 1 801 s each, without failures, without new entries in the corpus. A longer run of those three is the natural next step.
8. **E9-21, example.** (a) By delegated decision, its tests run only over `kerno_db`, because the example does not depend on `kerno-testkit`. (b) The row of point 15 of the minutes asks for `KeepLast(1)` through `set_policy`, but the public API does not allow it (`set_policy` only receives a `HistoryPolicy`); the example prunes explicitly (`prune_history`) and its catalogue gains a revision per write until it is pruned.
9. **E9-20, measurement.** It was measured **once**, with other load active on the machine: P1 and P2 were **not measured at rest**. The time bench does not run over the frozen set, but over a generated history from the same seed; P2 times only 25 queries. The quality thresholds **are printed, not asserted** in the test. F-08 remains: P2 does not measure the reading of persisted vectors. The set is synthetic and has 2 000 units.
10. **E9-15, mutations of E8.** Two mutations (D3 and D4) make **one** test fail and not two, as the document explains: `quality_over_the_frozen_set` cannot see them. There is no separate mutation for P4 or for the report identical between engines.
11. **Possibly intermittent test.** `m0_profile::shared_range::space_is_reclaimed_after_range_deletes` (storage) failed once and passed when repeated during the stage. **Suspected intermittent test, to be investigated**; it was not investigated here and was not changed.
12. **Repository history.** Some commits of `main` (among them `c779b39`) carry authorship trailers that the project's rules prohibit. They cannot be removed without rewriting the history, which is not allowed. **Known and accepted stain.**

## 6. Threat model and DESIGN

- [Threat model](../security/threat-model.md): T-03 and T-04 remain **open** (no encryption or authentication in 1.0; D2, D3 and D4 taken on 2026-11-13). **H-2**: the engine does not protect against a deliberate rewrite with a valid CRC; the defence available is `verify()` with `deep`. H-2 stays closed as an accepted and documented limit (D4). T-09 remains **open**; the public reclaim is a mechanism, not physical deletion. T-10: `ci.yml` does not run fuzzing or dependency auditing; it remains open.
- [`DESIGN.md`](../../crates/kerno-storage/DESIGN.md): H-9 specified in the section "What can survive".
- **H-5 (what is not measured)**: C-5 (blocks freed from the medium), memory, caches, tombstones, deletion thread and handle. Not measured.

## 7. Errata

- **Minutes §2.3 c.** `DerivedDb::close` returns `Result<(), UnitError>`, not `()`; it discards the `CloseReport` because the layer does not depend on `kerno-storage` (D9). Recorded in FRICTION 11 as a declared limit.
- **Minutes §2.3 f.** The citation of `compact_range` in ADR-0009 (C-5) is `db.rs:2009`; the correct line when the minutes were written was `db.rs:2011`. The ADR is not edited (accepted); **the citation is corrected here**. If the code changes, the line is searched again with `grep -n "pub fn compact_range" crates/kerno-storage/src/db.rs`.
- **ADR-0020, fixture.** The citation `derived-v2/db-with-vector` now exists, rebuilt in E9-01; its bytes are not reproduced between builds (the integrity is given by `MANIFEST`). The T5 check on `d01f646` was done once and the file tree does not stay identical (the opening of the engine creates `LOCK` and a session record).
- **H-12.** The citation `sst.rs:332` of the [advance minutes](stage9-early-record.md) becomes `sst.rs:335`.

## 8. Inherited findings

| Finding | Status at closure |
|---|---|
| H-2 | Closed as an accepted and documented limit (D4, taken on 2026-11-13) |
| H-3 | Accepted after the fact (D20 i) |
| H-4 | Remains contradicted, not rewritten (D17) |
| H-5 | Not measured; declared (§6) |
| H-6, H-8 | Closed by E9-03 |
| H-7 | **Verified with the limits of point 7** |
| H-9, H-12 | Closed in this package |
| H-13 | Reviewed (E9-16); K1, K2, K4 and K5 with the status of point 2 |
| H-14 | I-27: verified by E9-02 (D1), merged on 2026-11-13. I-28: not applicable. I-30: verified for the draft |
| F-02, F-03 | E9-11 |
| F-04 | E9-10 |
| F-05 | Open: there is no format on disk for summaries (D8) |
| F-06, F-07 | Measured in E9-20, with the caveats of point 9 |
| F-08 | Open, declared limit |
| F-09 | Partial (point 4) |
| F-10 | Evidence in E9-15, with the caveats of point 10 |
| F-11 | Checked, with the caveat of §7 |
| F-13 | Fifteen points with disposition (T-FR1) |
| F-12 | See below |

**F-12, review rounds of E8-03 and E8-04.** What can be rebuilt today are the commits of each package, not the rounds: `6bbadbd` and `23af819` (E8-03, merged into `d01f646`) and `ea533ca` (E8-04, merged later). The review rounds have no record in the repository; they are not invented. **Still open as a lack of record.**

## 9. Public surface changes (R-E9-17)

Pre-1.0, with no promise to break (D9): `Reclaim` in `kerno-core`; `UnitDb::reclaim`, `UnitDb::compact` and `DerivedDb::reclaim`; `UnitDb::engine` withdrawn; accessors `raw_*` and hooks under the `test-support` feature (D23); `DerivedDb::snapshot`, `query`, `history`, `derived_of`; public replacement of vectors; `RevisionId::first`; `UnitError::is_locked`.

## 10. What comes next

Owner approval of the text of the CLA before accepting any external contribution (D15); review of G1 to G7 of ADR-0016 done and D16 closed (2026-11-13; its open work is in the ADR); declaration of 1.0 only after D13 and real use outside the team (D12); investigate the intermittent test (point 11); a longer fuzzing run of `wal`, `manifest` and `open`; K1/K2/K4/K5 open (point 2). The status of Phase 1 in the roadmap is already resolved (point 1).
