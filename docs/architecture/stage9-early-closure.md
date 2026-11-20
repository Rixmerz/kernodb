# Closure of the Stage 9 advance (E9A-06)

| Field | Value |
|---|---|
| Status | **Closed on 2026-11-12. No blocking finding remains open** (§3). The promotion of I-29 for `kerno-storage` is recorded as **approved by delegation of the owner on 2026-11-12, revocable** (§5) |
| Date | 2026-11-12 (E9A-06) |
| Act | Adversarial review of E9A-01 to E9A-05 by reading their code and tests, checking of their criteria in §3.2 of the minutes, promotion of I-29 and update of [invariants](invariants.md), [threat model](../security/threat-model.md) (T-06, T-08, T-09), [DESIGN.md](../../crates/kerno-storage/DESIGN.md) and [roadmap](../roadmap/README.md) |
| Base reviewed | Tree at `76ff9a4` (merge after the record of the 30 minutes of fuzzing). This change only touches `docs/` and `crates/kerno-storage/DESIGN.md`; it adds or changes no tests or code |
| Documents | [Advance minutes](stage9-early-record.md) · [Stage 7 closure](stage7-closure.md) · [invariants](invariants.md) · [threat model](../security/threat-model.md) · [ADR-0009](../adr/0009-suppression-retention-and-backups.md) · [DESIGN of `kerno-storage`](../../crates/kerno-storage/DESIGN.md) · [fuzzing (E9A-04)](../../crates/kerno-storage/fuzz/README.md) · [roadmap, Stage 9](../roadmap/README.md#stage-9--hardening) |

This document describes the advance as it stands in the repository. Each figure comes from a command run for this review (§4) or from a committed record that is cited as such. What could not be checked is marked in §7 as **unverified**.

## 0. Result in brief

### 0.1 Closure criteria (minutes §3.2, E9A-06)

| Criterion | Result | Evidence |
|---|---|---|
| No blocking finding remains open | **Met** | §3: the fourteen findings have a status; none remains open and blocking. H-2 is the most serious candidate and is explained in §3 |
| The report carries the commands and their real output | **Met** | §2 and §4: literal commands and output lines copied from the run |
| `kerno-storage` suite green with `--all-features --release` | **Met** | `passed=672 failed=0 ignored=4` (§4.1) |
| Each mutation test of the minutes fails with the mutation applied | **Met** for the four mutations the minutes ask for (E9A-01, E9A-02, E9A-03 and E9A-05), plus the two of the correction F-E9A-03-1; all reproduced in a copy (§4.3) | The E9A-03 one (CRC of SST blocks) and the E9A-05 one (`CLEAN` checkpoint) make their tests fail, as do those of E9A-02 and E9A-01 |

### 0.2 What is promoted and what is not

- **I-29 is promoted for `kerno-storage`** (E9A-02 passes and covers what its acceptance line lists; §5). It stays in Phase 4 for the audit and the logs of higher layers.
- **I-27, I-28 and I-30 do not change.** They remain in Phase 4, outside these minutes (minutes §2.3).
- **T-06** stays *mitigated, not closed*. **T-08** stays measured for `kerno-storage`. **T-09** remains *open*: what survives is measured in the scenario of the test, without a guarantee of physical deletion (ADR-0009, option 3).

### 0.3 What has not been checked

- The 30 minutes per fuzzing target **have not been repeated** (it was asked not to). The record of `fuzz/README.md` is cited, and only the committed corpus was checked to reproduce without failures (§4.4).
- The delegation of 2026-11-12 is not recorded in writing in the repository I read (§5.4).

## 1. Packages reviewed

| Package | What the minutes asked | What was delivered (commits) | Checked now | Status |
|---|---|---|---|---|
| E9A-01 | Measure what survives a deletion, with four predictions fixed before running (P1 to P4). No policy or API | `ae94bae`, `2bab299` · `tests/suppression_survival.rs` (552 lines, observation) | Three tests green; P1 to P4 hold; mutation M-D reproduced (§2.1) | **Done**, with findings H-4 and H-5 |
| E9A-02 | I-29: the seven variants of `Error`, no sentinels in any error text, `verify()`, `inspect()` or metrics | `a0c349c`, `2bab299` · `tests/redaction.rs` (584 lines) | Six tests green; mutation M-A reproduced (§2.2) | **Done**, with finding H-6 |
| E9A-03 | Opening properties over WAL, SST and MANIFEST: zero panics, only `Corruption` or `Incompatible`, never data different from the model, 2 000 cases per type and mode | `adaadd`, `9f41531`, `98d6b98`, `5207c32` · `tests/open_properties.rs`. Correction in `src/` for finding F-E9A-03-1: `d89c728` | Thirteen tests green; mutations M-B, M-E and M-E2 reproduced (§2.3) | **Done**, with finding F-E9A-03-1 resolved (H-1), its open limit (H-2) and a process finding (H-3) |
| E9A-04 | Coverage fuzzing: one target per decoder (14) and one for opening, outside the workspace, committed corpus, non-blocking in CI | `4b51114`, `53a16cb`, `43538fa`, `9a50a69` · `crates/kerno-storage/fuzz/` (15 targets, committed corpus, `README.md`) | The 15 targets reproduce their committed corpus without failures; no CI step runs fuzzing. The 30 minutes **unverified** here (§7) | **Done** according to its record |
| E9A-05 | Lifecycle stress: 200 cycles, four writers, clean closes and simulated crashes, comparison with the model on each reopening | `153198c`, `f5fd147` · `tests/lifecycle_soak.rs` (416 lines) | One test green (200 cycles); mutation M-C reproduced in cycle 0 (§2.5) | **Done** |

## 2. Review by package

Each subsection says what the minutes asked, what was delivered, what I read in the code and the tests, and which commands I ran. The output lines are literal copies.

### 2.1 E9A-01: what survives a deletion

**Asked.** Four predictions (minutes §3.2) fixed before the first run, with no new policy or API. The test is labelled as an observation.

**Delivered.** `tests/suppression_survival.rs` with three tests: the default configuration (`observation_of_current_behaviour_default_log`), `wal_preallocate` (`observation_of_current_behaviour_preallocated_log`) and a snapshot open before the deletion (`observation_of_current_behaviour_snapshot_pins_tables`). The header says the predictions did not change after the first run. I checked with `git diff ae94bae HEAD -- crates/kerno-storage/tests/suppression_survival.rs` that the file is identical to its first committed version, although I cannot prove from git that that version was written before running.

**Reading.** The three tests compare the directory (`SimVfs::dump`) in six or seven stages per run. The visibility assertions (`assert_visible_state`) compare `get` and `scan` with the model after each deletion, each dump, the compaction and the reopening.

**Command and output.**

```
$ cargo test -p kerno-storage --all-features --release --test suppression_survival -- --nocapture --test-threads=1
[survival] P1 (delete: value still in the log, no flush): HOLDS (delete value in ["000004.wal"])
[survival] P1 (delete_range: value still in the log, no flush): HOLDS (range value in ["000004.wal"])
[survival] P2 (value in no live file after flush, compact_range, clean close): HOLDS (delete value in [], range value in [])
[survival] P3 (tombstone keeps the key in a live file until compaction discards it): HOLDS (before: ["000007.sst", "000009.sst", "MANIFEST"], after close: [])
[survival] C-2: MANIFEST names a key before compact_range: ["MANIFEST"], after: []
[survival] C-3: range fragments 1 -> 0
[survival] P1 with wal_preallocate: HOLDS (delete value in ["000004.wal"], range value in ["000004.wal"])
[survival] P4 (flushed log is zeros and holds no value): HOLDS (spare is zeros: Some(true); delete value in ["000007.sst"]; range value in ["000007.sst"])
[survival] P2 with wal_preallocate: HOLDS (delete value in [], range value in [])
[survival] P3 with wal_preallocate: HOLDS (before: ["000007.sst", "000009.sst", "MANIFEST"], after close: [])
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

With the snapshot open, the same run gives this line, and at the `compact_range` stage the value and the key are only in `000007.sst`:

```
[survival]   compact_range: rewritten=4 pinned_files=3 pinned_bytes=30990 kept_tables=["000003.sst", "000005.sst", "000007.sst"] retained_bytes=31702
[survival] stage: snapshot dropped
[survival]   files: 000008.wal=96 000010.sst=17062 000011.sst=10423 LOCK=0 MANIFEST=242
```

**Mutation (minutes §3.2: "P2 must fail").** Reproduced in a copy of the tree (§4.3, M-D). With the two edits of the mutation (the compaction tables do not drop covered points or fragments), P2 comes out `CONTRADICTED` with the range value still in a live file, and the three tests fail.

**Findings.** H-4 (the count clause of C-4 does not hold as ADR-0009 writes it), H-5 (what is not measured).

### 2.2 E9A-02: I-29 in `kerno-storage`

**Asked.** Produce the seven variants of `kerno_core::Error`; no sentinel, in bytes or in its ASCII form, in the `Display` or `Debug` of errors, `Issue`, `VerifyReport`, `inspect()` or `metrics()`. Mutation: add the key with `{:?}` to the message at `sst.rs:332` with the table missing, and the test must fail.

**Delivered.** `tests/redaction.rs`: one test per configuration (default, `Buffered`, `inline`, `m0`, `serial`) plus the detector test. The test of each configuration first checks that the scene produces the seven variants (`VARIANTS`) and at least one `Issue`, and checks that the scene itself stores the sentinels on disk ("otherwise "no leak" proves nothing").

**Reading.** The detector looks for each sentinel in eight forms: text, debug text, hexadecimal, hexadecimal with a `0x` prefix, `\x` escapes, ASCII escapes, decimal list and hexadecimal list (in upper or lower case, depending on the form). `detector_finds_every_form_of_every_sentinel` proves that the detector finds each form and gives no false positives on a clean text.

**Command and output.**

```
$ cargo test -p kerno-storage --all-features --release --test redaction
test detector_finds_every_form_of_every_sentinel ... ok
test buffered_durability_leaks_no_content ... ok
test inline_profile_leaks_no_content ... ok
test m0_profile_leaks_no_content ... ok
test default_config_leaks_no_content ... ok
test serial_profile_leaks_no_content ... ok
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.31s
```

**Mutation.** Reproduced in a copy (§4.3, M-A): the key is added to the message of `Table::open` at `src/sst.rs:335`. The line is now at 335, not at 332 as in the minutes, because `d89c728` added imports above it (H-12). With the mutation, the five configuration tests fail with "N of M texts carry a sentinel (I-29)" and the detector stays green.

**Finding.** H-6 (the detector only looks for complete sentinels).

### 2.3 E9A-03: opening properties and F-E9A-03-1

**Asked.** `proptest` over a database with tables, a live WAL and no clean close. Bit mutations, truncations and rewrites with recalculated CRC in WAL, SST and MANIFEST. Open in read-write and in read-only, and call `verify()`. Zero panics, errors only `Corruption` or `Incompatible`, zero data different from the model, 2 000 cases per type and mode.

**Delivered.** `tests/open_properties.rs`: six properties (three file types by two modes) with 2 000 cases each by default (`proptest_config`), fixed seed (`DEFAULT_BASE_SEED`) and `failure_persistence` disabled. Seven regression and control tests accompany the properties, among them the two of F-E9A-03-1.

**Reading of the code and of the change to `src/`.** I read `d89c728` in full. The new check is `format::sst::check_filter_holds_index_keys`: the last key of each data block, which the index already has in memory, must pass the filter. It is called by `Table::open` (in `src/sst.rs`) and by `verify()` (in `src/verify.rs`), not by the `open_sst` codec, so as not to alter the order of rules of v1.md §13.1. The change writes nothing: in the diff I found no new write, and its message and `DESIGN.md` say that it does not change the format. The check costs one hash per block, not a read of data.

**Command and output.**

```
$ cargo test -p kerno-storage --all-features --release --test open_properties
test default_run_draws_the_same_case_seeds_every_time ... ok
test control_unmutated_directory_opens_to_the_final_model_in_both_modes ... ok
test known_limit_get_trusts_a_resealed_filter_missing_interior_keys ... ok
test resealed_wal_payload_rewrite_is_another_valid_commit_not_damage ... ok
test resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify ... ok
test manifest_read_write ... ok
test mutations_reach_both_outcomes_and_change_the_file ... ok
test manifest_read_only ... ok
test sst_read_only ... ok
test wal_rewrite_targets_are_the_frame_header_only ... ok
test wal_read_only ... ok
test sst_read_write ... ok
test wal_read_write ... ok
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.28s
```

**Mutations.** Reproduced in a copy (§4.3):
- M-B (minutes): without the CRC check of SST blocks in `open_block`. `sst_read_write`, `sst_read_only` and `mutations_reach_both_outcomes_and_change_the_file` fail.
- M-E (F-E9A-03-1, opening): without `check_filter_holds_index_keys` in `Table::open`. `resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` fails with "a table whose filter lost every bit opened".
- M-E2 (F-E9A-03-1, `verify()`): without the check in `verify()`. The same test fails with "deep=true: one SST-V3 per table, at its filter block".

**Findings.** H-1 (resolved), H-2 (open limit), H-3 (process).

### 2.4 E9A-04: coverage fuzzing

**Asked.** One target per decoder (14) and one for opening, outside the workspace, with a committed corpus, non-blocking in CI. Approved criterion: 30 minutes per target, zero failures, zero entries that take longer than 1 s, corpus and failing entries committed.

**Delivered.** `crates/kerno-storage/fuzz/`: 15 targets in `fuzz_targets/`, its own workspace (`[workspace]` in its `Cargo.toml`), 31 seeds for opening and 14 seed directories for decoders in `corpus/`, `seed_open.py` which rebuilds `corpus/open` from `tests/vectors/v1/dirs`, and `README.md`.

**Reading.** The number of decoders is fixed by `struct_vectors.rs:2367` (`assert_eq!(kinds.len(), 14)`). The list of target names is the one in the README; I did not compare it one by one with the entry points of that test. I checked that no CI workflow mentions fuzzing (`git grep -i fuzz -- .github`: no results). The `open` target rejects any error other than `Corruption` or `Incompatible`, except opening read-only without `MANIFEST`, which the README documents as the expected rejection (REC-13, DIR-2).

**Command and output (replay of the committed corpus, no fuzzing).** See §4.4. The 15 targets exit with code 0.

**Committed record (not rerun).** `README.md` table "Acceptance of the acta: met on 2026-11-12": 15 targets, 30 minutes each (1 801 seconds), zero failures; each run was made on 4 local CPUs with the same configuration as the README command. 60-second runs of 2026-11-10 are also recorded. This review did **not** rerun the 30 minutes (instruction of the assignment).

**Findings.** None of its own. The coverage limit of the fuzzing (only the readers of `kerno-storage`, only what coverage reached) is stated in `threat-model.md` §6.2.

### 2.5 E9A-05: lifecycle stress

**Asked.** 200 cycles by default, four writers, zero lost confirmed commits, `verify()` without findings and content equal to the model on each reopening. Mutation: do not write the `CLEAN` checkpoint at close; the test must fail.

**Delivered.** `tests/lifecycle_soak.rs`: one test with 200 cycles (`KERNO_SOAK_CYCLES` changes it) and a fixed seed (`DEFAULT_SEED`). Every tenth cycle ends in a simulated crash instead of a clean close. A watchdog stops the test if a cycle blocks; it is a guard against hangs, not a performance threshold.

**Reading.** `verify_closed` runs over each on-disk state before the next reopening, so what is reopened is what was verified. It compares `snap.seq() >= confirmed` (no lost confirmed commit) and the content with the model. After a clean close it requires `CLEAN`, zero WAL, zero orphans and zero unknown files; after a crash it requires that there is WAL to replay.

**Command and output.**

```
$ cargo test -p kerno-storage --all-features --release --test lifecycle_soak -- --nocapture
lifecycle_soak: 200 cycles, seed 0x50ace9a5, work [("flushes", 1323), ("compactions", 2168), ("tombstones_dropped", 4615), ("range_tombstones_dropped", 5212), ("covered_points_dropped", 14868)]
test lifecycle_soak_keeps_every_confirmed_commit_and_verifies_clean ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.57s
```

**Mutation.** Reproduced in a copy (§4.3, M-C): `close_clean` calls `checkpoint_state(..., false)`. The test fails in cycle 0 with rule REC-9; the seed it prints is `0x50ace9a5`, the one in the header. The WAL number that appears in the message (`000046.wal`) is different from the one the header cites (`000040.wal`): it depends on the history of the run, and the rule and the cycle match.

**Finding.** H-8 (comment in `group_commit.rs`, prior to this package).

## 3. Findings

Status: **resolved** (corrected and checked), **open and blocking**, or **open and not blocking** (with the reason). Evidence: *deterministic* when a command or a test shows it; *inferential* when it comes from reading the code; *unverified* when I could not check it.

| # | Package | Finding | Evidence | Class | Cause | Blocking | Status |
|---|---|---|---|---|---|---|---|
| H-1 | E9A-03 | F-E9A-03-1, coarse case: an SST filter zeroed or rewritten with a valid CRC was opened and `get` hid keys without error. Without `deep`, `verify()` did not report it | `resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` passes at HEAD; M-E and M-E2 make it fail | deterministic | pre-existing: the bug was in `src/` before E9A | no | **Resolved** in `d89c728` (opening and `verify()`) |
| H-2 | E9A-03 | Remaining limit: a filter to which only **interior** keys of a block are missing is opened, and `get` answers "does not exist" without error while `scan` returns them. Only `verify()` with `deep` reports it. The same holds for a rewrite of values or of the WAL payload, which a CRC does not distinguish | `known_limit_get_trusts_a_resealed_filter_missing_interior_keys` passes and fixes the limit; header of `open_properties.rs` | deterministic (fixed by test) | pre-existing: format without authentication (ADR-0006 pending) | **no** (see note) | **Open and not blocking**: checking it on read requires reading the data block on every "no", which cancels the filter; closing it against a deliberate rewrite requires AEAD (ADR-0006, outside these minutes, §2.3). The engine does not promise protection against that rewrite (threat model §6.6, T-04 open). The test stops the limit from being forgotten |
| H-3 | E9A-03 | Process: the correction in `src/` of H-1 was done within package E9A-03 and went into the same squash as E9A-05 (`f5fd147`, #45). The minutes (§3.1) ask that a correction in `src/` be a separate package accepted separately. I found no record of that acceptance | `git show --stat d89c728`: `src/format/sst.rs`, `src/sst.rs`, `src/verify.rs`; `git log` of the squash | deterministic (history) | introduced in the work of E9A-03 | no | **Open and not blocking**: the change has its regression test, two mutations that make it fail, and documentation. The owner still has to accept it afterwards or ask for it to be split. It is not a technical failure |
| H-4 | E9A-01 | The count clause of C-4 (ADR-0009) does not hold: `pinned_files = 3` while only `000007.sst` contains the value and the key | Output of §2.1: `pinned_files=3`, `kept_tables` with three tables; at the `compact_range` stage, the value and the key only in `000007.sst` | deterministic | pre-existing (the measurement shows it; it is not a defect of the engine) | no | **Open and not blocking**: ADR-0009 already records it in its criteria table, and the acceptance took only the deadline of C-4. I do not change ADR-0009 in this closure |
| H-5 | E9A-01 | Not measured: C-5 (blocks freed from the medium), memory and caches, point tombstones that a trivial move takes to the last level, deletion by background thread and long-lived handle | Header of `suppression_survival.rs` ("Not exercised"); ADR-0009, C-5 and K-1 to K-4 | deterministic (absence) | scope of the observation | no | **Open and not blocking**: there is no measured contract that is broken; the test is an observation; T-09 remains open |
| H-6 | E9A-02 | The detector looks for complete sentinels. A partial fragment of a key or value in a message would not be detected | `find_sentinels` looks for the full `s.bytes` in each form; no scenario produces fragments | inferential | test design | no | **Open and not blocking**: the minutes (§2.2) say that messages with interpolated data carry file names, rules and numbers; I did not check that no message carries a fragment of a key |
| H-7 | E9A-04 | The 30 minutes per target have not been rerun in this review | Assignment: do not repeat the 30-minute fuzzing | unverified | — | no | **Unverified** (the record is cited, §2.4). The corpus replay was done and comes out without failures |
| H-8 | E9A-05 | Comment in `tests/group_commit.rs:538` says "10^4 commits"; the code does 4 × 1 500 and 4 × 1 000 (line 548). The minutes pointed it out in §2.4(3) | `git grep` in `group_commit.rs` | deterministic | pre-existing | no | **Open and not blocking**: the minutes assigned it to E9A-05 only if that package touched the file, and it did not; this closure does not change tests. Correction: a comment change in a package of its own |
| H-9 | Doc (DESIGN.md) | The sentence "deletion does not overwrite either" (section "What can survive") holds for the default configuration. With `wal_preallocate` the dumped WAL is overwritten with zeros, as the section on preallocated WAL of the same guide says and P4 measures | `DESIGN.md` (section on preallocated WAL); output of P4 | deterministic (reading) | pre-existing | no | **Open and not blocking**: what DESIGN declares about the WAL did not change in this closure, so I do not rewrite it; specifying it is a later change |
| H-10 | Doc (DESIGN.md) | DESIGN.md said "ADR-0009 pending" and "left for Phase 4" since ADR-0009 was accepted (2026-11-12) | `grep -n ADR-0009` before the change | deterministic | pre-existing since E9A-00b | no | **Resolved** in this closure (DESIGN.md, two sentences) |
| H-11 | Doc (threat model) | The phrase "fuzzing of the format (Phase 6)" of T-06 and "still pending" of §6.2 were out of date | Reading of `threat-model.md` | deterministic | pre-existing since E9A-04 | no | **Resolved** in this closure (T-06, §6.2, §6.7) |
| H-12 | Doc (minutes) | The minutes cite the mutation at `sst.rs:332`; the message is at `sst.rs:335` since `d89c728` | `sed -n 335p crates/kerno-storage/src/sst.rs` shows the message "referenced by the MANIFEST but missing"; `grep -n "sst.rs:332" docs/architecture/stage9-early-record.md` gives lines 38 and 103 | deterministic | documentary (imports added by the change of H-1) | no | **Open and not blocking**: mutation M-A was reproduced at line 335, but the citation in the minutes stays at 332 and this closure does not change the approved text of the minutes. Correcting it is a later documentation change |
| H-13 | Minutes | K-E3-7 (reads of `kerno-units` that check less than `verify_units`) has not been reviewed | Minutes §2.3 and §7, point 4: outside this advance, recommended for Stage 9 after E7-06 | — | scope decision | no | **Open and not blocking**: deliberately outside E9A |
| H-14 | Minutes | I-27, I-28 and I-30 remain unverified | Minutes §2.3 and §4 | — | out of scope (Phase 4) | no | **Open and not blocking**: the advance does not include them |

**No blocking finding remains open.** H-2 is the most serious candidate: a filter rewritten with a valid CRC can make `get` fail to find keys that the table contains, without error. I do not classify it as blocking for two reasons: the engine declares no protection against deliberate rewrites (T-04 remains open and the threat model says so), and its solution depends on a decision the owner has not taken (ADR-0006). **If the owner wants to treat it as blocking**, the consequence is that Stage 8 does not write derived data until ADR-0006 is accepted. That decision is theirs.

## 4. Commands and output

Everything was run at the root of the reviewed tree (`76ff9a4`) except the mutations, which were made in an exported copy of that same commit outside the working tree.

### 4.1 Full `kerno-storage` suite

```
$ cargo test -p kerno-storage --all-features --release > suite.txt 2>&1; echo "exit=$?"
exit=0
$ grep -c "^test result: ok" suite.txt
41
$ grep -cE "FAILED|panicked" suite.txt
0
$ grep "^test result" suite.txt | awk '{p+=$4; f+=$6; i+=$8} END{print "passed=" p, "failed=" f, "ignored=" i}'
passed=672 failed=0 ignored=4
```

`suite.txt` is outside the repository (review working folder). It was run on 2026-11-12 over the tree of this closure, which changes no code or tests with respect to `76ff9a4` (`git diff --stat 76ff9a4`: only `docs/` and `crates/kerno-storage/DESIGN.md`). The count matches the recorded baseline.

### 4.2 Tests of the five packages

The literal outputs are in §2.1 to §2.5. Summary: `suppression_survival` 3/3, `redaction` 6/6, `open_properties` 13/13, `lifecycle_soak` 1/1, all green.

### 4.3 Mutations (copy of the tree, not the working tree)

The tree of `76ff9a4` (`git archive`) was exported to a separate directory, with its own build directory. Each mutation was applied by editing a file, the indicated test was run, and it was reverted. The baseline of the copy passes (`test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.65s`, `redaction`).

| # | Mutation | Test | Real result |
|---|---|---|---|
| M-A | `src/sst.rs:335`: add `(key {:?})` with `meta.smallest` to the message "referenced by the MANIFEST but missing" (minutes E9A-02) | `redaction` | `test result: FAILED. 1 passed; 5 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.93s`. The five configuration tests fail: `inline_profile_leaks_no_content`, `m0_profile_leaks_no_content`, `buffered_durability_leaks_no_content`, `default_config_leaks_no_content`, `serial_profile_leaks_no_content`. The detector passes |
| M-B | `src/format/block.rs:62`: `if false && crc_ctx(...) != le_u32(...)` (minutes E9A-03) | `open_properties` | `test result: FAILED. 10 passed; 3 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.94s`. `sst_read_only`, `sst_read_write` and `mutations_reach_both_outcomes_and_change_the_file` fail |
| M-C | `src/db.rs:1453`: `checkpoint_state(..., true)` → `false` in `close_clean` (minutes E9A-05) | `lifecycle_soak` | `seed 0x50ace9a5 cycle 0: verify() found [Issue { file: "000046.wal", offset: None, rule: "REC-9", detail: "required WAL missing (the MANIFEST says it is the oldest live log)" }]`; `test result: FAILED. 0 passed; 1 failed` |
| M-D1 | `src/compaction.rs:799`: `if false && cover.covering(...) > it.seq()` (this edit only) | `suppression_survival` | The three tests fail; in the default compaction test, the message is `after compact_range: unexpected key zz-r00-KSENT-RNG`, a read that does not match the model |
| M-D | M-D1 plus `src/compaction.rs`, `keep_fragments`: all fragments are kept (minutes E9A-01) | `suppression_survival` | `[survival] P2 (value in no live file after flush, compact_range, clean close): CONTRADICTED (delete value in [], range value in ["000016.sst"])`; `test result: FAILED. 0 passed; 3 failed` |
| M-E | `src/sst.rs:353`: `check_filter_holds_index_keys` is disabled in `Table::open` | `open_properties` | `ReadWrite: a table whose filter lost every bit opened`; `test result: FAILED. 12 passed; 1 failed` |
| M-E2 | `src/verify.rs:340`: the same check is disabled in `verify()` | `open_properties` | `assertion \`left == right\` failed: deep=true: one SST-V3 per table, at its filter block`; `test result: FAILED. 12 passed; 1 failed` |

Mutations in the header of `open_properties.rs` that I did **not** reproduce: the one that disables the filter in `Table::get`, the one that sets `tolerate_get = false` in the case of a rewritten filter, and the one that disables the check inside `SstVerifier::observe`. They remain **unverified** in this review.

### 4.4 Replay of the committed corpus (no fuzzing)

A replay runs each entry of the corpus once (`-runs=0`): it generates no new entries and does not write to the committed corpus. Order of the run: one target per line.

```
$ cargo +nightly fuzz run --fuzz-dir crates/kerno-storage/fuzz <objetivo> <directorio-vacio> crates/kerno-storage/fuzz/corpus/<objetivo> -- -runs=0 -max_len=65536 -timeout=1; echo "exit=$?"
varint exit=0 Done 4 runs in 0 second(s)
header exit=0 Done 27 runs in 0 second(s)
commit exit=0 Done 3 runs in 0 second(s)
frame exit=0 Done 5 runs in 0 second(s)
data_block exit=0 Done 3 runs in 0 second(s)
filter_block exit=0 Done 2 runs in 0 second(s)
rangedel_block exit=0 Done 2 runs in 0 second(s)
props_block exit=0 Done 3 runs in 0 second(s)
index_block exit=0 Done 2 runs in 0 second(s)
footer exit=0 Done 2 runs in 0 second(s)
record exit=0 Done 9 runs in 0 second(s)
wal exit=0 Done 7 runs in 0 second(s)
sst exit=0 Done 4 runs in 0 second(s)
manifest exit=0 Done 6 runs in 0 second(s)
open exit=0 Done 33 runs in 0 second(s)
```

Each line is the exit code of the command for one target and the last line of its log. A shell loop was run over the fifteen targets, with an empty working directory per target (`<directorio-vacio>`), outside the repository. The command names the `nightly` toolchain and the `fuzz` subcommand of `cargo-fuzz` because they are part of what has to be run. The directory `crates/kerno-storage/fuzz/target/` is generated by the build and is ignored by git.

### 4.5 Format

`cargo fmt --all --check` over the tree of this closure: no output, exit code 0 (`fmt exit=0`, §8).

## 5. Promotion of I-29 for `kerno-storage`

### 5.1 Statement

I-29 (SAD §11.4): the audit and the logs do not record the full content of documents, secrets or tokens.

### 5.2 What the E9A-02 test covers against its acceptance line

| What the E9A-02 line lists (minutes §3.2) | What `tests/redaction.rs` checks | Covered |
|---|---|---|
| Produce the seven variants of `kerno_core::Error` (`Io`, `Corruption`, `Locked`, `InvalidArgument`, `Poisoned`, `Incompatible`, `ReadOnly`); if one is missing, the test fails | `check_case` requires that `VARIANTS` is complete and that no unknown variant appears | Yes |
| No sentinel, in bytes or in ASCII form, in `Display` or `Debug` | `Seen::error` stores `to_string()` and `{:?}` / `{:#?}` of each error; `leaks()` looks for the sentinels in each text | Yes |
| `Issue` | `seen.debug(issue)` and `seen.display(issue.detail)` | Yes |
| `VerifyReport` | `seen.outcome(&report)` with its `Debug` | Yes |
| `inspect()` | `seen.outcome(inspect(...))` | Yes |
| `metrics()` | `observe` with `db.metrics()` and `db.stats()` | Yes |
| Mutation: key with `{:?}` in the message of `sst.rs`, with the table missing, the test must fail | M-A (§4.3): five of five configuration tests fail | Yes |

The five configurations are the ones in the header of the test (default, `Buffered`, `inline`, `m0`, `serial`). `verify()` is run over copies with bit damage, truncation and a missing file, in addition to the healthy version.

### 5.3 Limits

- `kerno-storage` has no logging or audit layer. What is verified are the errors, the reports and the metrics. The part of SAD §11.4 that covers logs and audit remains in Phase 4 and is repeated with each layer that emits text (R-E9A-3 of the minutes; H-6 on the detector).
- The detector looks for complete sentinels (H-6).

### 5.4 Approval

The task that triggers this closure says that the promotion was **approved by delegation of the owner on 2026-11-12 and is revocable**. The minutes (§3.2) said "pending the owner's approval". I did not find in the repository a written record of that delegation dated 2026-11-12: the decisions of that date that are on record are E9A-00b, the start of Stage 8 and the closure of Stage 7. If the delegation does not exist, I-29 remains a **proposal** and the owner confirms or withdraws it in [invariants](invariants.md).

## 6. Documents updated in this closure

| Document | Change |
|---|---|
| [`invariants.md`](invariants.md) | I-29 becomes "verified for `kerno-storage`" with the logging limit in Phase 4; header with the promotion. I-27, I-28 and I-30 unchanged |
| [`threat-model.md`](../security/threat-model.md) | T-06, T-08 and T-09 in §3, §6.2 and §6.7: what is measured or tested, without extending it. No change in §6.6 (T-04) |
| [`DESIGN.md`](../../crates/kerno-storage/DESIGN.md) | Two sentences about ADR-0009 (H-10). The rest declared did not change; H-9 stays as it is |
| [`stage9-early-record.md`](stage9-early-record.md) | E9A-06 and the rows of §4 that this closure closes, marked; I-29 in §6; entry in §10 |
| [`roadmap/README.md`](../roadmap/README.md) | Advance listed in Stage 9, with the closure |

No file of `src/`, `tests/`, `fuzz/` or any ADR was modified.

## 7. Open risks and limits

1. **H-2**: the filter rewritten with a valid CRC, with interior keys of a block missing, still goes undetected by `get`. Closing it needs ADR-0006.
2. **H-3**: the change to `src/` of H-1 was not a separate package accepted separately, as the minutes ask. It requires a decision of the owner.
3. **H-4 and H-5**: ADR-0009 remains with C-4 contradicted in its count clause and with C-5 unmeasured. T-09 remains open.
4. **Unverified:** the 30 minutes per fuzzing target (H-7). The mutations in the header of `open_properties.rs` that I did not reproduce (§4.3).
5. **Unverified:** the figures 357 keys and 307 hidden of F-E9A-03-1 (threat model §6.6 and header of `open_properties.rs`). This review did not reproduce them; it only checked the result of the test.
6. **Tool names in the text:** the minutes (§2.2, §3.2, §7) and `fuzz/README.md` name fuzzing tools and dependencies, and that approved text was not rewritten in this closure. This closure names only what its commands and files need: `proptest`, the library of the properties of `tests/open_properties.rs` (§2.3), and `cargo +nightly fuzz` (command of §4.4). It names no models, assistants or the tool that produced the work.
7. **Scope of the fuzzing:** it covers the readers of `kerno-storage` and only the inputs that coverage reached. This review does not extend that coverage.
8. **Stress measurement:** the soak and the properties use fixed seeds; the runs are repeated with the same seed. To explore other seeds, the header of each test says how (`KERNO_SOAK_SEED`, `OPEN_PROPERTIES_BASE_SEED`).

## 8. Closure checks and approval

| Check | Command | Result |
|---|---|---|
| Format | `cargo fmt --all --check; echo "fmt exit=$?"` | No output; `fmt exit=0` (run on 2026-11-12, §4.5) |
| Suite of `kerno-storage` | `cargo test -p kerno-storage --all-features --release` | `passed=672 failed=0 ignored=4` (§4.1) |
| Clippy | Not run: this closure changes no code or tests | — |

| Date | Decision | Notes |
|---|---|---|
| 2026-11-12 | **Closure of the advance (E9A-06) done** | No blocking finding open (§3). I-29 promoted for `kerno-storage` by delegation of the owner, revocable (§5.4) |
