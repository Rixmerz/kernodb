# Entry minutes: advance of Stage 9 (hardening) in parallel with Stage 7

| Field | Value |
|---|---|
| Date | 2026-11-11 |
| Status | **Approved by delegation of the owner on 2026-11-11, revocable** |
| Act | Proposal of scope and of the packages E9A-00 to E9A-06: which part of Stage 9 advances before Stage 8 and which part runs in parallel with Stage 7 |
| Documents | [Roadmap (pending 1 and Stage 9)](../roadmap/README.md) · [Stage 7 minutes](stage7-record.md) · [Stage 6 minutes](stage6-record.md) · [SAD §11, §16 (Phases 4 and 6) and §18](SAD.md) · [invariants](invariants.md) · [failure matrix](failure-matrix.md) · [threat model](../security/threat-model.md) · [ADR-0005](../adr/0005-public-identifiers.md), [ADR-0006](../adr/0006-encryption-and-key-management.md), [ADR-0009](../adr/0009-suppression-retention-and-backups.md), [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md), [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md), [ADR-0015](../adr/0015-privacy-of-observability.md) · [Stage 3 closure (K-E3-7)](stage3-closure.md) · [design of `kerno-storage`](../../crates/kerno-storage/DESIGN.md) |

## 0. Status

These minutes are **approved by delegation** (§10), revocable by the owner. They resolve pending decision 1 of the roadmap ("Security order versus AI") in the sense proposed there: what affects derived data advances before Stage 8; the rest stays in Stage 9.

The packages are **not sequential**: §3.1 indicates which ones run at the same time. None writes to `crates/kerno-units` or `crates/kerno-testkit`, which are the surfaces of Stage 7 (commits `34004f0` and `60de6d8`). No package starts Stage 8.

## 1. Start condition

| Condition | Status |
|---|---|
| Pending 1 of the roadmap | Resolved by delegation on 2026-11-11 (§10) |
| Stage 7 in progress | E7-00 and E7-01 done; E7-02 is next. **Does not block**: the E9A packages do not touch its files |
| ADR-0009 (suppression, retention and backups) | **Accepted on 2026-11-12, option 3, under the terms of the ADR** (E9A-00b, done). It no longer blocks entry into Stage 8. It does not authorise any suppression API, which is not in these minutes |
| ADR-0015 (observability privacy) | Accepted on 2026-11-06. It is the basis of E9A-02 |
| ADR-0013 (format v1) | **Resolved on 2026-11-11** (§7, point 3): accepted and format v1 frozen on 2026-10-31. It blocks no E9A; the migrations of Stage 9 remain pending as their own work |
| Stage 8 | Starts by decision of the owner on 2026-11-12 ([Stage 7 closure §8.1](stage7-closure.md)). Its condition on ADR-0009 has been met since 2026-11-12. ADR-0018 was accepted on 2026-11-12 with option 2, in the Stage 8 minutes. Proposal of these minutes, revocable: that E9A-02 is closed before Stage 8 writes any derived data |

## 2. Scope

### 2.1 Objectives

1. Fix the criteria of ADR-0009 **before measuring** and measure what survives a deletion in `kerno-storage` (E9A-00a, E9A-01). Accepting or rejecting ADR-0009 is the owner's decision (E9A-00b).
2. Verify I-29 in `kerno-storage`: errors, verification and metrics without document content (E9A-02). Today it has no automated test.
3. Harden the readers of `kerno-storage` against hostile inputs: opening properties (E9A-03), coverage fuzzing if the owner approves the dependency (E9A-04) and lifecycle stress (E9A-05).

### 2.2 Starting point (what was read)

- **Inputs that `kerno-storage` parses.** The pure decoders are in `crates/kerno-storage/src/format/`: common header of 96 bytes, WAL records and their scanner, data blocks, *footer*, filter, properties, range fragments and MANIFEST records in TLV. `wal.rs`, `sst.rs`, `block.rs` and `manifest.rs` connect them through the `Vfs`. The public API receives keys and values, which `db.rs` limits before writing (`check_put`, `check_delete`, `check_range`).
- **What it emits.** No file of `crates/kerno-storage/src` calls `log`, `tracing`, `println!`, `eprintln!`, `stdout` or `stderr`. The observable output is three things: the `Error` enum of `kerno-core` (seven variants, four of them with a `String`), the `Issue`s of `verify()` (its documentation says "never key or value bytes") and the metrics of `metrics.rs`, which are counters and histograms. `format/mod.rs` declares that decoding errors never carry key or value bytes (T-08). The messages with interpolated data that were read contain file names, rules and numbers (`sst.rs:335`, `db.rs:1446`).
- **Sentinel tests.** There are none in `kerno-storage` or `kerno-testkit` (search for `sentinel` and `centinela`: no results). Under the rule of [invariants §0](invariants.md), I-29 is **pending** for `kerno-storage`.
- **Suppression.** `delete` and `delete_range` exist in the transaction (`txn.rs:150` and `txn.rs:161`), `compact_range` (`db.rs:2011`), the deferred deletion of obsolete tables by the `kerno-delete` thread (`deleter.rs`) and the deletion of WAL after the `FLUSH`. With `wal_preallocate` (default `false`, `config.rs:543`) an already dumped WAL is overwritten with zeros and reused. [DESIGN.md](../../crates/kerno-storage/DESIGN.md) says that `delete_range` and `compact_range` are **mechanism**, without a guarantee of physical deletion, and that "the suppression design with guarantees (ADR-0009) is left for Phase 4". The [threat model §6.5](../security/threat-model.md) leaves T-09 open. There is no encryption: the format reserves AEAD and rejects it as `Incompatible`.
- **Hostile-input tests that already exist.** In `tests/struct_vectors.rs`, `decoders_never_panic_on_random_bytes` generates random inputs from a fixed seed for 14 decoder types: 100 000 per type by default (`KERNO_FUZZ_INPUTS`) and requires that none is accepted. `spec_decoder_agrees` compares 5 000 per type (`KERNO_AGREE_INPUTS`) with an independent decoder. There is also `crafted.rs` (files made with a valid CRC), `damage.rs`, `verify_detects_every_bit_flip` and `tests/model.rs` (proptest over the model). **There is no coverage fuzzing**: there is no `fuzz/` directory and no `libfuzzer` dependency, and the [threat model §6.2](../security/threat-model.md) places it in Phase 6, pending.
- **Existing stress.** `tests/group_commit.rs::stress_handoff_with_random_delays`: four writers, with 1 500 commits each in one case and 1 000 in the other, and random delays in the WAL writes over `SimVfs`. `tests/process_kill.rs`: a real `kill -9` of a writer, 20 iterations by default (`KERNO_KILL_ITERS`) and 200 in the Phase 1 output run ([F1 report](../testing/f1-report.md)).
- **K-E3-7** is in `kerno-units`, not in `kerno-storage`: the reads check less than `verify_units` (§2.3).

### 2.3 Out of scope: stays in Stage 9

- **Public IDs** (ADR-0005, *Proposed*, draft) and I-27. No code in this phase.
- **Encryption and key management** (ADR-0006, *Pending*) and I-28. ADR-0006 forbids fixing key assumptions until it is accepted.
- **Export and import** (I-30). There is no export code in `crates/`.
- **Migrations with fixtures from earlier versions.** They depend on the state of the v1 format (§2.4, point 1).
- **Comparisons with SQLite and thresholds against SQLite.** SQLite appears only in `spikes/storage/spike-bench/Cargo.toml`; no crate in `crates/` uses it, and no E9A needs it. The thresholds belong to the owner (pending 3 of the roadmap; ADR-0012).
- **Review of K-E3-7** ("the reads check less than `verify_units`"). The unit and relation reads are in `kerno-units/src/db.rs`, and the associative query of Stage 7 uses them: `assoc.rs:327` calls `UnitSnapshot::reference_relations_of`. Reviewing them now would mix two surfaces in a single review. **Recommendation:** do it in Stage 9, after E7-06.
- **Backups and expiry windows** (part of ADR-0009). The decision is the ADR's; the implementation is Stage 9's.
- **Coverage fuzzing** (E9A-04): approved on 2026-11-11 (§7, point 2); it runs after E9A-03.
- **Latency thresholds** in stress or fuzzing. ADR-0012 remains *Proposed*; no E9A sets latency thresholds.
- **`large` scale of the benchmark** ([F1 report](../testing/f1-report.md) §5.1). It is a Phase 1 measurement, not a Stage 9 test.
- **Changes to `Config::default`.** No E9A changes default values.

### 2.4 Discrepancies found on reading (not corrected here)

1. **Status of ADR-0013 (resolved on 2026-11-11, §7 point 3).** The freeze was carried out on 2026-10-31 and ADR-0013 has been accepted since that date; the sources that said otherwise were corrected on the same date. Original text of the point: its own text says "Accepted (2026-10-31), conditional on…". `docs/adr/README.md` says "Proposed (WP06; accepted before v1 is frozen in WP14)". The roadmap says that the freezing of format v1 is pending and that ADR-0013 "remains Proposed". [`crates/kerno-storage/DESIGN.md`](../../crates/kerno-storage/DESIGN.md) says "frozen on 2026-10-31 (ADR-0013)" and that the `DRAFT` bit is withdrawn; [invariants I-15](invariants.md) says that, while the freezing is pending, every file carries `DRAFT`. The owner must fix a single status before any migration.
2. **Phase of the fuzzing.** The roadmap puts it in Stage 9. The [threat model §6.2](../security/threat-model.md) and the SAD (Phase 6) put it in Phase 6. These minutes do not decide the phase: they treat coverage fuzzing as a gate (E9A-04).
3. **Stress comment.** The comment in `group_commit.rs` says "10^4 commits"; the code does 4 × 1 500 and 4 × 1 000. E9A-05 corrects it if it touches that file; if not, a separate documentation change.

## 3. Work packages

All of them write only to the files indicated. None changes `src/` of `kerno-storage`, `tests/common/` or `tests/shared.rs`.

| WP | Content | Depends on | Writes | Can run alongside |
|---|---|---|---|---|
| **E9A-00a** Criteria of ADR-0009 | Write into ADR-0009 the decision criteria of the ADR itself (what is removed immediately, what remains subject to compaction, what deadline backups follow), dated, **before** measuring anything | — | `docs/adr/0009-suppression-retention-and-backups.md` (only the criteria section) | all |
| **E9A-01** What survives a deletion (measurement only) | End-to-end test: write values with sentinels, delete with `delete` and `delete_range`, and observe at each stage (before flush, after `compact_range`, after clean close, after reopening) which files of the directory still contain the sentinel. No policy or new API | E9A-00a | `crates/kerno-storage/tests/suppression_survival.rs` | E9A-02, E9A-03, E9A-05 |
| **E9A-02** I-29 in `kerno-storage` | Sentinels in keys and values. Provoke each variant of `Error`, `verify()` over damage, `inspect()` and `metrics()`. Check that no `Display` or `Debug` contains a sentinel or its ASCII form | — (ADR-0015 accepted) | `crates/kerno-storage/tests/redaction.rs` | E9A-01, E9A-03, E9A-05 |
| **E9A-03** Opening properties | proptest over a created database: mutate a file (WAL, SST or MANIFEST) with bit flips, truncations and rewriting with recalculated CRC; open read-write and read-only and call `verify()` | — | `crates/kerno-storage/tests/open_properties.rs` | E9A-01, E9A-02, E9A-05 |
| **E9A-04** Coverage fuzzing | One target per decoder (14) and one for opening, with cargo-fuzz in `crates/kerno-storage/fuzz/`, outside the workspace, with a committed corpus. Non-blocking in CI | E9A-03; dependency and toolchain approved on 2026-11-11 (§7, point 2) | `crates/kerno-storage/fuzz/**` | — (runs after E9A-03) |
| **E9A-05** Lifecycle stress | Cycles of concurrent writes (4 writers), deletes and `delete_range`, `compact_range`, `sync()`, clean close, reopening and `verify()`. Comparison with the model on each reopening | — | `crates/kerno-storage/tests/lifecycle_soak.rs` | E9A-01, E9A-02, E9A-03 |
| **E9A-00b** Decision on ADR-0009 (**done on 2026-11-12**, option 3) | Choose option 1, 2 or 3 and accept ADR-0009, or leave it pending and forbid any suppression API | E9A-00a, E9A-01 | `docs/adr/0009-*.md`, `docs/adr/README.md` | — (owner's decision) |
| **E9A-06** Closure (**done on 2026-11-12**, [closure](stage9-early-closure.md)) | Adversarial review of E9A-01…05. Promotion of I-29 for `kerno-storage` if E9A-02 passes. Update of [invariants](invariants.md) (I-27…I-30 remain in "Phase 4"), [threat model](../security/threat-model.md) (T-06, T-08, T-09) and, if what it declares changes, [DESIGN.md](../../crates/kerno-storage/DESIGN.md) | E9A-00b, E9A-01…05 | `docs/architecture/stage9-early-closure.md`, `invariants.md`, `threat-model.md` | — |

### 3.1 What runs at the same time

- **Now, together with E7-02 to E7-06:** E9A-00a, E9A-01, E9A-02, E9A-03 and E9A-05. Each creates a new file in `crates/kerno-storage/tests/`. None modifies `tests/common/`, `tests/shared.rs` or `src/`, so there are no conflicts between them.
- **After E9A-01 and the owner's decision:** E9A-00b.
- **After E9A-03** (dependency approved on 2026-11-11, §7, point 2): E9A-04. It does not need the others.
- **At the end:** E9A-06, after E9A-00b and E9A-01 to E9A-05.
- **Outside every E9A package:** `crates/kerno-units` and `crates/kerno-testkit`. If an E9A needs a helper from `kerno-testkit`, it copies it into its own file; it does not add it to the shared crate.
- **Finding in `src/`:** if an E9A finds a real fault in `kerno-storage/src`, it records it as a finding and does not correct it. The correction is a separate package, accepted separately.

**Critical path for Stage 8:** E9A-00a → E9A-01 → E9A-00b, together with E9A-02 → E9A-06.

### 3.2 Acceptance criteria per package

Common to all packages with code: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo test -p kerno-storage --all-features --release` green; test count before and after, with the new file counted by the runner; each new test seen failing with a mutation applied to a **copy** of the tree, reverted and checked with `git diff`. No mutation is left in the working tree. Clippy rejects complex return types (`type_complexity`): use a type alias.

- **E9A-00a.** The criteria text is in ADR-0009 and its commit precedes any run of E9A-01. It contains no results.
- **E9A-01.** Before the first run, the test file carries in its header these four predictions, which are not changed afterwards:
  - **P1.** After `delete` without flush, the value remains in the WAL.
  - **P2.** After `FLUSH`, `compact_range` that covers the key and clean close, the value does not appear in any live file of the directory.
  - **P3.** The tombstone keeps the key in some live file until compaction discards it.
  - **P4.** With `wal_preallocate = true`, a dumped WAL is left with zeros and the value does not appear in it.

  Each prediction passes, or is recorded as contradicted with the real output. None is corrected to make it pass. The test is labelled as an **observation of current behaviour**, not as a contract, so that it does not fix a decision of ADR-0009. Test mutation: a `compact_range` that does not discard covered values; P2 must fail.
- **E9A-02.** The test produces **all seven** variants of `kerno_core::Error` (`Io`, `Corruption`, `Locked`, `InvalidArgument`, `Poisoned`, `Incompatible`, `ReadOnly`); if one is missing, the test fails. No sentinel appears, in bytes or in its ASCII form, in `Display` or `Debug`, in `Issue`, in `VerifyReport`, in `inspect()` or in `metrics()`. Test mutation: add the key with `{:?}` to the message at `sst.rs:335`, with the table missing, and the test must fail. If E9A-02 passes, I-29 is promoted for `kerno-storage`, **pending the owner's approval**.
- **E9A-03.** Zero panics. Zero openings that return data different from the model. The only accepted errors are `Corruption` and `Incompatible`. Test mutation: remove the CRC check of an SST block in the copy; the test must fail.
- **E9A-04.** Approved on 2026-11-11: one target per decoder plus one for opening, 30 minutes per target on a local machine, zero failures, zero entries that take more than 1 s, corpus and failing entries committed to the repository.
- **E9A-05.** 200 cycles by default (`KERNO_SOAK_CYCLES`), 4 writers. Zero lost confirmed commits (the recovered sequence is at least the last confirmed one). `verify()` without findings on each reopening and content equal to the model. Test mutation: do not write the `CLEAN` checkpoint at close; the test must fail.
- **E9A-06.** No blocking finding open. The report carries the commands and their real output.

## 4. Acceptance criteria of the stage

| Criterion (roadmap, Stage 9) | How it is met | Status |
|---|---|---|
| Limits and guarantees documented | E9A-01 measures what survives; E9A-06 writes it up ([closure](stage9-early-closure.md)) | **done** (E9A-06) |
| Critical faults resolved or marked as blocking | Each finding is recorded in the [closure](stage9-early-closure.md) with its status; none blocking open | **done** (E9A-06) |
| I-29 verified | E9A-02 (only `kerno-storage`). The observations of Stage 5 are already covered by I-37 and ADR-0015 | **done for `kerno-storage`** (E9A-06, promoted by delegation of the owner on 2026-11-12, revocable). The logging layers remain in Phase 4 |
| I-27, I-28, I-30 verified | Outside these minutes (§2.3) | pending, Stage 9 |
| Fuzzing of external inputs | E9A-03 in the tree; E9A-04 with coverage, approved on 2026-11-11. 30-minute threshold per target met on 2026-11-12: 15 targets, zero failures, no entry over 1 s ([results](../../crates/kerno-storage/fuzz/README.md)) | **done** (closure in E9A-06) |
| Stress tests | E9A-05, in addition to the tests that already exist (§2.2) | **done** (E9A-05; `tests/lifecycle_soak.rs` passes in the suite of 2026-11-12, closure in E9A-06) |
| ADR-0009 before Stage 8 | E9A-00b | **done on 2026-11-12**: ADR-0009 accepted with option 3 |

## 5. Thresholds set before measuring

| Package | Threshold |
|---|---|
| E9A-01 | None for performance. Predictions P1 to P4 (§3.2) are set before running |
| E9A-02 | Seven of seven variants of `Error` produced; zero sentinels in any text |
| E9A-03 | 2 000 cases per file type (WAL, SST, MANIFEST) and per mode (read-write, read-only); zero panics; zero data different from the model |
| E9A-04 | 30 minutes per target; zero failures; zero entries over 1 s |
| E9A-05 | 200 cycles; zero lost confirmed commits; zero `verify()` findings |

These thresholds are set by these minutes, under the delegation of 2026-11-11, and the owner may revoke them. They are not changed after measuring. A threshold that is not met is recorded as a failure; it is not lowered.

## 6. Invariants

| ID | What the advance does | Status after the package |
|---|---|---|
| I-29 | E9A-02 verifies it for `kerno-storage` | **Promoted for `kerno-storage` in E9A-06 (2026-11-12), approved by delegation of the owner, revocable**; remains in Phase 4 for logs and audit |
| I-08 | E9A-03 reinforces it with randomly made inputs | Already verified; no change of status |
| I-02, I-09 | E9A-05 reinforces them with cycles of close, reopening and stress | Already verified; no change of status |
| I-27, I-28, I-30 | Outside these minutes | Pending (Phase 4) |

There is no suppression invariant. If ADR-0009 is accepted, Stage 8 will propose its own; these minutes do not invent it.

## 7. Decisions the owner must take

1. **Accept ADR-0009 (E9A-00b). Resolved on 2026-11-12:** ADR-0009 is accepted with option 3, under the terms recorded in the [ADR](../adr/0009-suppression-retention-and-backups.md). Until that date it was not included in the delegation of 2026-11-11, which covered only pending 1, and without a decision Stage 8 could not start. Original recommendation: decide after E9A-01, with its results in front, which is the order followed.
2. **Coverage fuzzing dependency (E9A-04): approved on 2026-11-11** on these terms: cargo-fuzz with `libfuzzer-sys` and a nightly toolchain, only in `crates/kerno-storage/fuzz`, outside the workspace, non-blocking in CI, and run after E9A-03. Reason for the original question: cargo-fuzz requires `libfuzzer-sys` and a nightly toolchain, and that is a new dependency in the sense of the `ponytail` ladder. Original recommendation: decide it after E9A-03, when it is known what coverage is missing (already decided).
3. **Single status of ADR-0013** (§2.4, point 1). **Resolved on 2026-11-11:** ADR-0013 is **Accepted** (2026-10-31) and the v1 format has been frozen since that day. This is confirmed by ADR-0013 itself, `crates/kerno-storage/src/format/features.rs` and `header.rs` (they reject the `DRAFT` bit as "pre-freeze draft format") and `crates/kerno-storage/DESIGN.md`. The ADR-0013 row in the ADR index, the roadmap line, I-15 and the header of [`v1.md`](../file-format/v1.md) are corrected on that date.
4. **When to review K-E3-7.** Recommendation: Stage 9, after E7-06 (§2.3).

## 8. Risks

| ID | Risk | Treatment |
|---|---|---|
| R-E9A-1 | ADR-0009 not accepted delays Stage 8 | **Resolved on 2026-11-12** (E9A-00b). No suppression code in these minutes |
| R-E9A-2 | E9A-01 fixes a behaviour that ADR-0009 later changes | The tests are labelled as observation; the predictions are recorded before running |
| R-E9A-3 | I-29 is verified over errors, verification and metrics, but `kerno-storage` has no logging layer. When it has one, the rule applies again | E9A-02 is repeated with each new layer that emits text |
| R-E9A-4 | Fixed-seed random tests do not grow in coverage | E9A-03 uses one case per own seed and `PROPTEST_CASES`; E9A-04, approved on 2026-11-11 |
| R-E9A-5 | Stress and fuzzing lengthen CI together with Stage 7 | E9A-05 with 200 cycles by default; it does not enter CI until it is measured |
| R-E9A-6 | T-09 remains open even if ADR-0009 is accepted: no guaranteed physical deletion, and cryptographic deletion depends on the pending ADR-0006 | Limit declared in the threat model; no physical deletion is promised |
| R-E9A-7 | K-E3-7 remains unreviewed until Stage 9 | Accepted by design on 2026-11-04 with a recommendation to review it in Stage 9 |

## 9. What these minutes do not decide

The decisions of §2.3 and §7 remain with their owners. These minutes do not change any ADR, invariant, benchmark threshold or default value, and do not touch code.

## 10. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-11 | **Approved by delegation of the owner, revocable** | The owner delegated pending decision 1 of the roadmap and asked for the hardening work in `kerno-storage` that does not depend on Stages 7 or 8, in parallel with Stage 7. What is decided here: what affects derived data (ADR-0009 in its criteria part, I-29) advances; fuzzing, stress and the opening properties are the hardenings of `kerno-storage` that run now. The rest stays in Stage 9. ADR-0009 is not accepted by this delegation (§7, point 1) |
| 2026-11-11 | **Decisions of the owner:** coverage fuzzing approved on the terms of §7, point 2 (E9A-04 after E9A-03); status of ADR-0013 resolved (§7, point 3) | ADR-0009 remains undecided (§7, point 1) |
| 2026-11-12 | **E9A-00b done: ADR-0009 accepted with option 3** | Decision 1 of §7 resolved. Terms and relation with the criteria in [ADR-0009](../adr/0009-suppression-retention-and-backups.md). T-09 remains open ([threat model §6.5](../security/threat-model.md)) |
| 2026-11-12 | **E9A-06 done: closure of the advance** | Adversarial review of E9A-01 to E9A-05 in the [closure](stage9-early-closure.md). No blocking finding open. I-29 promoted for `kerno-storage` by delegation of the owner, revocable. Invariants, threat model (T-06, T-08, T-09), DESIGN.md (status of ADR-0009) and roadmap updated |
