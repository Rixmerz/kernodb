# Closure of Stage 8

| Field | Value |
|---|---|
| Status | **Closed as engineering work.** Decisions of the closure (§8) taken by delegation of the owner on 2026-11-12, revocable by the owner. **Q4 not demonstrated** (not met): it is recorded that way, not as met. No open blocking finding (§4) |
| Date | 2026-11-12 (E8-08) |
| Act | Adversarial review of E8-01 to E8-07 and E8-09; check of the criteria of §4 of the minutes, of the thresholds of §6 and of the invariants of §7; update of invariants, roadmap, SAD and minutes (minutes §3, row E8-08) |
| Reviewed base | Tree at `53b7fa1` (last commit of E8-07 before this closure). This change only touches `docs/` and adds no tests or code |
| Documents | [Stage 8 minutes](stage8-record.md) · [evaluation set](stage8-evaluation-set.md) · [measurement](stage8-measurement.md) · [raw output](stage8-measurement-output.txt) · [SQLite report (E8-S0)](../testing/sqlite-report.md) · [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) · [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md) · [ADR-0009](../adr/0009-suppression-retention-and-backups.md) · [real use (E8-09)](../../examples/agent-memory/FRICTION.md) · [invariants](invariants.md) · [roadmap, Stage 8](../roadmap/README.md#stage-8--consolidation-and-embeddings) · [SAD 0.3.0](SAD.md) · [Stage 7 closure](stage7-closure.md) |

This document describes the stage as it stands in the repository. Every figure comes from a command run for this closure (§9), from the committed raw output or from a cited file. What has not been checked is marked **not verified**.

**No threshold, no expected answer, no weight and no frozen file was changed in E8-08.** No code or tests were changed either. The defects found are recorded in §4 with their reproduction; none has been corrected.

## 0. Result in brief

### 0.1 Quality against §6.2 (new set, 100 queries; 25 with a query vector)

| Threshold (minutes §6.2) | Measured | Status |
|---|---|---|
| **Q1** recall@10 of the persisted exact search against brute force = 1.000 | 1.000; 25 of 25 lists identical | **met** |
| **Q2** nDCG@10 with signal ≥ 0.80, overall | 1.000 over the 25 queries with a vector | **met**, with a ceiling effect (§3) |
| **Q3** recall@10 with signal ≥ 0.90, overall | 1.000 over the 25 queries with a vector | **met**, with a ceiling effect (§3) |
| **Q4** nDCG@10 with signal ≥ without signal − 0.01, overall and in each class | Met where it can be computed (`vector_neighbourhood`, 1.000 against 1.000). **Three of the four classes cannot be measured**: they have no query vector | **not demonstrated. Not closed.** It is not recorded as met |
| **Q5** unjudged ≤ 10 % of the top 10, in each arm | Maximum 1.1 % (`ranked`, 8 of 708) | **met** |
| **Q6** (ranking against simple order) | Not measured: only if the owner proposes switching on the ranking. Not a closure criterion | not measured |

### 0.2 Performance against §6.3

| Threshold | Measured, twelve runs | Status |
|---|---|---|
| **P1** write with descriptor / without descriptor ≤ 1.50 | 1.165 to 1.238 | **met** |
| **P2** search with persisted vectors / in memory ≤ 1.10 | 0.921 to 1.007 | **met**, with the limit of P2 described in §3 |
| **P3** set of `stale` = derived data of the changed sources | Set equality, both engines, before and after reopening | **met** |
| **P4** rebuilding byte for byte equal to building anew | 286 of 286, both engines | **met** |

### 0.3 What this closure claims and what it does not

1. The mechanics of derived data (register, staleness, rebuilding, persistence of vectors, suppression of sources and derived data, summaries with provenance) are built and have tests that pass in both engines (§1, §9).
2. Retrieval quality is **not** demonstrated: Q4 is not closed, Q2 and Q3 give the maximum value in both arms (ceiling effect), and the vectors are deterministic with a seed (§3, §5).
3. A deleted or stale derived datum is not served; suppression does not promise physical deletion (§5).
4. No blocking defect remains open. There are non-blocking findings with their reason (§4).

## 1. Work packages

For each package: what the minutes asked (§3), what was delivered, the commits and the status. The commands and their actual output are in §9.

| WP | The minutes asked | Delivered | Commits (from `git log`) | Status |
|---|---|---|---|---|
| **E8-S0** SQLite report without thresholds | Comparison of `kerno` against SQLite, paired durabilities, without thresholds, existing binary | Report [sqlite-report.md](../testing/sqlite-report.md) and raw output in `spikes/storage/results/2026-11-12-e8-s0/`. Without thresholds. `Buffered` pair with a guarantee that is not identical, stated in the report. First `Sync` pass interrupted for lack of space; cuts declared in its §6 | `078839a`, `349d735` | done |
| **E8-00** entry gate | Approval of the minutes, ADR-0018 accepted, status of ADR-0009 in its file, D1 to D12 recorded | Done according to [minutes §0](stage8-record.md) and [§10](stage8-record.md) | `7d656ac` | done |
| **E8-01** new frozen set | Corpus, queries and judgements with their own seed; SHA-256 `MANIFEST`; generator in `kerno-testkit` | `crates/kerno-testkit/src/derived_eval.rs`, `crates/kerno-testkit/data/derived-eval/`, [set](stage8-evaluation-set.md). `MANIFEST` checked before measuring. Tests in `crates/kerno-testkit/tests/derived_eval.rs` (15 tests) | `6b01dde`, `f3ddb2c` (merge `bec6f36`) | done. See F-10 for the mutation |
| **E8-02** register in memory | `DerivedDescriptor` with exact sources, method, version, parameters; states; reverse index | `crates/kerno-units/src/derived.rs`, `tests/derived_registry.rs` (13 tests) | `3dc63f0` | done |
| **E8-03** staleness and rebuilding | Changing a source marks `stale` in the same `ChangeSet`; rebuilding the same as building; new method or version creates a derived datum and marks `superseded` | `crates/kerno-units/src/derived_stale.rs`, `tests/derived_stale.rs` (46 tests). Round history in §7.1 | `d6b8ee9` (squash), `6bbadbd`, `23af819` and others, per §7.1 | done |
| **E8-04** persistence of vectors (conditional, D2 = persist) | Vectors as derived data with a layer-version bit, new ADR, byte format of the layer, DV rules | `crates/kerno-units/src/vector_store.rs`, `tests/derived_vectors.rs` (77 tests), [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md). Deviation from D2 in §7.2 | `ea2f232`, `d0ab6cc` (ADR); `ea533ca`, `ca30c28`, `e5a6d0b` (code) | done |
| **E8-05** suppression of sources and derived data | Deleting a unit or retiring a revision retires its derived data in the same `ChangeSet`; survival test with sentinels | `crates/kerno-units/src/suppression.rs`, `tests/derived_suppression.rs` (19 tests). No sentinel for summaries: F-05 | `91b8373`, `0bb9ecb` (review), `9461d7a` | done, with F-05 |
| **E8-06** consolidation with provenance | Summary as derived datum with mandatory sources; adapter as a trait with no network; deterministic test implementation; the original always readable | `crates/kerno-units/src/consolidation.rs`, `tests/consolidation.rs` (30 tests, 15 per engine). No real summariser (D12) | `5be0a06`, `5350599`, `9461d7a` | done |
| **E8-07** measurement | Quality over the new set and performance; committed raw output | `tests/derived_measure.rs` (4 tests), `examples/derived_bench.rs`, [measurement](stage8-measurement.md), [raw output](stage8-measurement-output.txt) | `ca60831`, `53b7fa1` (merge `1124a34`) | done. See F-10 and §3 |
| **E8-09** real use case (pre-1.0) | Memory for an agent with only the public API; register of derived data if E8-05 is closed | `examples/agent-memory/` (outside the workspace), [FRICTION.md](../../examples/agent-memory/FRICTION.md) with 15 points | `5518b08`, `56971dd` | done. Friction list in §6 |
| **E8-08** closure | Adversarial review, promotion of I-25 and I-38 and the new ones (§7), roadmap, SAD as a new version | This document, [invariants](invariants.md) §2.6, [roadmap](../roadmap/README.md), [SAD 0.3.0](SAD.md) | this change | done |

No package added dependencies: `Cargo.toml` and `Cargo.lock` have not changed since the base of Stage 7 (`52f026f`), checked with `git log` (§9, command 6).

## 2. Adversarial review (part 1 of the task)

### 2.1 What was run

- **Suite of `kerno-units` and `kerno-testkit` in release**, with all functions: 856 passed, 0 failed, 8 ignored (§9, command 1). None of the eight ignored is from Stage 8: one is the regenerator of layer-1 vectors (`struct_vectors.rs:769`, «writes the committed vectors») and seven are documentation fragments of `kerno-testkit` and `kerno-units` marked as not run.
- **Each new test file appears in the output of `cargo test`** (§9, command 2). None is outside the runner's scope: there is no `autotests = false` nor `[[test]]` in the `Cargo.toml` of `kerno-units` or `kerno-testkit`.
- **Probes in a copy of the tree** (`git archive` of `53b7fa1`, outside the working tree, deleted at the end), in release, in both engines (`emulated_log` and `kerno_db`), with a test file of its own that uses the same harness as the set. Results in §2.3.
- **Code reading** of `consolidation.rs`, `suppression.rs` and of the opening and commit path of `derived_stale.rs`, which are the pieces without a prior independent review.

### 2.2 What was reviewed and with what result

| Piece | Reviewed by | Result |
|---|---|---|
| `consolidation.rs` (E8-06) | Code reading and probes P3, P5 | No defect. `summary()` serves only a `ready` summary with the type marker; `get_unit` has no path to the text of a summary. Two non-blocking observations: F-02 and F-03 |
| `suppression.rs` (E8-05, fixes of `0bb9ecb`) | Reading and probes P2, P4 | No defect. `source_gone` retires a derived datum if the unit is missing, is `Deleted` or its source revision is below `oldest`. The retention path reconciles the register even if retention fails. Relations: no derived data to delete, as the file itself says |
| Opening path of `derived_stale.rs` after the review of E8-05 | Reading and probes P3, P4 | No defect. A derived datum whose source changed or was deleted while the database was closed is registered `stale` or discarded, never `ready` without recalculating. A value is kept only for `ready` (`values` is cleared when it moves to `stale`, `failed` or `superseded`) |
| Commit of `DerivedChangeSet::commit` | Reading | The register is locked before the commit and released after the marking. The code's own note says that this order has no test: F-09 |
| Persistence of vectors (E8-04) | Reading, tests | The T5 check (reference binary) was not run: F-11 |
| Measurement (E8-07) | Reading of the output and of the document | No defect. The mutation of each test was not done: F-10 |

### 2.3 Probes in a copy of the tree, in both engines

The probe file (five tests, two engines) and its actual output are in §9, command 7. Summary:

| Probe | What it does | Result |
|---|---|---|
| P1 deleting and restoring the same unit in a single `ChangeSet` | Builds a summary, deletes and restores the source in the same `ChangeSet` | `restore` after `delete` in the same `ChangeSet` **is rejected** in both engines (`is_ok=false`). The case that could delete a summary that was to remain `stale` is not reachable through the API. Not a finding |
| P2 retention that purges the revision a summary was built from, and reopening | Summary over rev 1, edit, summary over rev 2, `KeepLast(1)`, close, reopen with both descriptors | Both engines: `removed=1`; the summary of the purged revision disappears; the one of the current revision still serves `aaaa`; the reopening reports exactly the purged id as dropped. **Passes** |
| P3 summary `stale` at close; reopening and rebuilding | Source change, close, reopen with the old descriptor, rebuild | Both engines: reopens `stale`, serves no text, the rebuild gives a new id and the old one stops existing. **Passes.** Observation: building anew over the same sources gives `Duplicate` («a derived datum with this identity is already registered»): F-02 |
| P4 source deleted with the database closed; reopening | Summary over two units, deletion of one of them with the database closed, reopening | Both engines: the id is returned as dropped, there is no text, and the other source is still read. **Passes** |
| P5 differential rule: no read returns a summary in place of a source | Two summaries and one superseded summary, edit of the source, close, read | Both engines: `summary()` is `None` for the `stale` one and for the superseded one; `get_unit` of the source equals the read of a plain database. **Passes** |

The five probes were run with `--test-threads=1`. The result is `10 passed; 0 failed`, and the three observations of P1 to P3 that appear in the output are read with `--nocapture` (§9, command 7).

### 2.4 What the review did not do

- **Mutation of the new tests** (minutes §4, point 4): not run in this review. See F-10.
- **The T5 check of ADR-0020** (building the binary of `d01f646` and opening the fixture): not run (F-11).
- **Reordering of the commit lock**: no test covers it, and the code's own note says so (F-09).

## 3. Measurement: result as obtained

This section is the result of the measurement (`stage8-measurement.md`), unchanged. The figures come from the committed raw output.

- **Q1, Q2, Q3 and Q5 and P1 to P4 are met**, in the sense of their threshold. The four performance ones are in the twelve runs of P1 and P2 and in the equality of P3 and P4 (§0.2).
- **Q4 is not closed.** Only one of the four query classes of the frozen set carries a query vector: `vector_neighbourhood`, 25 of 25. The classes `temporal`, `provenance` and `combined` have 0 of 25 vectors. As Q4 asks «overall and in each class», it cannot be computed in three of four classes. It is recorded as **not demonstrated**. No query vectors were invented, so as not to change the frozen set.
- **Q2, Q3 and Q4 give 1.000 in the arm with signal and also in the arm without it.** It is a ceiling effect: these 25 queries do not distinguish one arm from the other. The thresholds are met, but the measurement contributes little evidence of quality. The set and the threshold were not changed after measuring.
- **P2 does not measure the cost of reading persisted vectors.** It compares two searches over the same `VectorSet` already loaded. The single read of 20 000 vectors (about 10 to 11 ms) is reported apart and does not enter the ratio. The cold cost of opening the database (156 to 162 ms) does not enter either.
- **The vectors are deterministic with a seed** (D5). They measure the mechanics, not the semantic quality of a model.
- **Corpus, queries and judgements were written by the same team.** There is no independent judgement (minutes §6.1, declared limit).

## 4. Findings

States: **resolved**; **open and blocking**; **open and non-blocking**, with its reason. Evidence class: **shown by a command**, **read from the code** or **not verified**.

**Open blocking findings: none.** No finding of this review contradicts an invariant of §7 or a criterion of §4 in a way that prevents the engineering closure. That is why decision (a) of §8 stands.

| ID | Where | Finding | Status | Evidence class | Reason |
|---|---|---|---|---|---|
| F-01 | `derived_stale.rs` and probes P1 | None: `restore` after `delete` in the same `ChangeSet` is rejected. It is noted because the `deleted` branch of `commit` is not cleaned up, and it would be an over-deletion if the restoration were allowed | resolved by the API (not reachable) | shown (probe P1, both engines) | No path produces it |
| F-02 | `consolidation.rs` `build_summary`, `derived_stale.rs` `register` | After rebuilding a summary, building anew over the same sources returns `Duplicate` with the text «a derived datum with this identity is already registered». Whoever wants the summary has to keep the new id. The error says little | **open and non-blocking** | shown (probe P3) | It is consistent with the identity (I-24): the same identity is not registered twice. It is an ergonomics problem, recorded in group D of §6 |
| F-03 | `derived_stale.rs` `read_input` and `valid_inputs`, lines 354 to 372 | `read_input` returns `None` if the head has no payload, and `valid_inputs` (used by `rebuild` and `supersede`) then **skips** that source without error, whereas `build` rejects it with `MissingSource`. Only a deleted unit has no payload; deleted units are already removed from the register at commit, on opening and in retention, so the path is not reached | **open and non-blocking** | read from the code; not reached by any probe | Not reached. If it were once reached, `rebuild` could produce a `ready` derived datum with fewer sources than `build` would. A test or an explicit error is advisable before the public API |
| F-04 | `derived.rs:190` (identity) and `tests/derived_registry.rs` | The identity includes the profile (`field(self.profile.as_bytes())`), as I-25 requires. No test changes **only** the profile and checks that the identity changes: `each_identity_field_changes_the_identity` does not vary it, and `derived_vectors.rs` uses the profile as data, not as a check | **open and non-blocking** | read from the code; grep of `profile` in `tests/` | The property holds in the code. The test that shows it is missing. Pending for the follow-up of Stage 8 |
| F-05 | `tests/derived_suppression.rs`, header «Sentinels in summaries: declared gap» | Summaries have no survival measurement of sentinels on disk: they live only in memory, so there are no bytes to measure. Minutes §5.3 point 5 asked for the sentinel. The I-37 sentinel tests do cover summaries (`sentinels.rs`, `no_sentinel_in_summary_types_or_errors`, in both engines) | **open and non-blocking** | shown (header of the file and `sentinels.rs`) | The summary persists no byte (`a_summary_leaves_nothing_on_disk`). If summaries come to persist, the measurement becomes mandatory |
| F-06 | Measurement, §2 of [measurement](stage8-measurement.md) | Q4 not demonstrated: three of four classes without a query vector | **open and non-blocking** (owner's decision (a), §8) | shown (raw output) | The frozen set has no query vectors in those classes. Closing it requires a new set, which is the follow-up of §8 (c) |
| F-07 | Measurement, Q2, Q3 and Q4 | Ceiling effect: 1.000 in both arms over the 25 queries with a vector | **open and non-blocking** | shown (raw output) | A limit of the set; not changed after measuring. Covered by the follow-up of §8 (c) |
| F-08 | Measurement, P2 | P2 does not measure the cost of reading persisted vectors | **open and non-blocking** | shown (bench design, §3.1 of the measurement) | The bench compares two searches over the same loaded set. A read measurement requires another design |
| F-09 | `derived_stale.rs`, `DerivedChangeSet::commit`, lines 495 to 502 | The order «register lock, commit, mark» of the saved commit has no test. The code's own note says that showing it requires a reader competing with the commit, which cannot be made deterministic | **open and non-blocking** | read from the code | The correctness rests on the reasoning of the note. There is no test case that contradicts it |
| F-10 | Minutes §4, point 4 (mutation of each new test) | The registered mutation of each new test is not in the evidence of E8-07 (`stage8-measurement.md` §6, point 8: «not done in this package»). For E8-06 there is no mutation record in the commits. For E8-03 and E8-05 there is (ecafa8f, 0bb9ecb). This review did not run mutations | **open and non-blocking**, with the caveat below | not verified | It is a process gap, not a code defect: the tests pass and their branch coverage has been read. **Caveat:** the minutes say that a package that does not meet §4 is not closed. The owner must accept or require that evidence before the closure is final |
| F-11 | ADR-0020, T5 and the rejection criterion of a binary without the feature | The T5 check (building the binary of `d01f646` and opening `derived-v2/db-with-vector`) was not run in this review | **open and non-blocking** | not verified | It requires a reference binary that was not built here. Tests T6 and T7 (`layer1_reader_refuses_derived_fixture`, `open_derived_fixture_writes_nothing`) pass in the suite (§9) |
| F-12 | History of E8-03 and E8-04 | The repository has no record of the review rounds or of their findings. They are reconstructed in §7.1 from the commit messages | **open and non-blocking** | read from the history | For the project's memory, the rounds are recorded in §7.1 with the source of each one |
| F-13 | `examples/agent-memory/FRICTION.md` | Fifteen points of friction of the public API, of which one touches a promise (§6) | **open and non-blocking** for Stage 8; **blocks 1.0**, as prior work for 1.0 (minutes §1, E8-09 and decision (d) of §8) | shown (the example compiles and runs outside the workspace according to its README) | Outside the critical path of Stage 8 (minutes §3, E8-09) |
| F-14 | `docs/roadmap/README.md`, Stage 8 | The roadmap said that E8-S0 was still pending, although `docs/testing/sqlite-report.md` has been committed since `078839a` | **resolved** in this closure | shown (`git log`, file present) | Corrected in the update of the roadmap |
| F-15 | Suite, `struct_vectors.rs:769` | One ignored test, which regenerates layer-1 vectors. It is not from Stage 8 | **resolved by design** (not run in `cargo test`) | shown (suite output) | It stays as it is |

**Status of the blocking finding: none is open.** No finding in the table prevents closing Stage 8 as engineering work. F-10 is the only process caveat, and it is left to the owner.

## 5. What remains unproven and unbuilt

1. **Summaries are in memory only.** There is no survival measurement of sentinels for them, because there is no byte on disk to measure (F-05). A summary does not survive reopening unless the caller saves its descriptor and passes it again; the reconstruction goes through `open_summaries`, which recalculates only if the sources are still the same.
2. **Deleting a relation deletes no derived datum**, because no derived datum has a relation as its source. This is a design decision, and `deleting_a_relation_removes_no_derived_datum` proves it. It is not a guarantee that a future relation cannot be a source: if one is, the rule is new.
3. **The lock order of the saved commit has no test** (F-09).
4. **Deterministic vectors with a seed say nothing about a real model.** The quality of real embeddings has not been measured, and no model has been decided (D5 (b) remains undecided).
5. **Corpus, queries and judgements were written by the same team.** There is no independent judgement (minutes §6.1). With 25 queries per class, small differences are not significant.
6. **No approximate index, no full-text search, no hybrid fusion, no real summariser** (minutes §2.3 and D12). None is built.
7. **No backup function**, so there is no backup window to declare (ADR-0009, B-1; D11). No deadline is promised.
8. **No physical deletion and no cryptographic deletion** (ADR-0006 pending). Deletion is logical at once, and the bytes may remain in live files until the compaction that covers them.
9. **Compaction is not reachable through the public wrapper of E8-09.** See group F of §6.

## 6. Friction list of the use case (E8-09)

The fifteen points of [FRICTION.md](../../examples/agent-memory/FRICTION.md), grouped as prior work for 1.0. The point marked **(promise)** touches a promise.

### Group A: reading and querying derived data (points 1, 2, 9 and 15)

- **1.** `DerivedDb` does not list, query or read history. There is no `snapshot`, `query` or `history`; the unit catalogue the example uses is a second source of truth.
- **2.** There is no way to find the derived data of a unit: `dependents_of_unit` exists in the register, but `DerivedDb` offers only reads by `DerivedId`.
- **9.** Two worlds of vectors that do not meet: the persisted ones are read one by one; the search lives in memory and copies all the `ready` vectors on each call.
- **15.** The unit type of the catalogue shares a namespace with the application's data, and the default history grows with each write.

### Group B: identity and reconstruction (points 3 and 8)

- **3.** A derived id changes on each rebuild, and the register the application keeps cannot be updated in the same transaction. The only atomic path (`supersede_vector`) leaves `superseded` garbage in the store. `replace_vector` is `pub(crate)`.
- **8.** Replacing a stale vector with one computed outside goes through the «recalculate» API with a closure that ignores its arguments.

### Group C: summaries and opening (point 5)

- **5.** There is no way to open persisted vectors and summaries at the same time: `open_persisted` loads vectors, `open_summaries` does not. Summaries are in memory only, so the application has to store its own definition.

### Group D: creation ergonomics and identifiers (points 6, 7 and 10)

- **6.** Unit identifiers are 16 opaque bytes, and the id source is fixed before opening the database.
- **7.** `create` returns `UnitId`, but `put_vector` wants `RevisionId`. The number of the first revision is written by hand.
- **10.** A vector recipe requires many fields, and nothing links `dim` and `seed` to the vector that is produced.

### Group E: errors, closing and opening (points 11, 12, 13 and 14)

- **11.** `DerivedDb::close` returns `()` and discards the closing report.
- **12.** Three error types without a common type; the messages of `DerivedStaleError` name no unit.
- **13.** Opening the database needs three crates and a trait object; there is no `open_dir` for the default engine.
- **14.** A second opening of the same directory fails with a storage error that does not distinguish «locked» from «corrupt».

### Group F: deletion and space (point 4) **(promise)**

- **4.** **(promise)** `forget` is only logical. The text stays in the files after `forget`, after retention and after closing: `forgotten_text_stays_in_the_files_after_retention` proves it, finding it in `000003.sst`. `UnitDb::compact` needs `TestHooks` and `KernoDb::compact_range` is a test method, so both stay behind `DerivedDb`, which hides the engine. The documentation of `suppression.rs` says so («No promise of physical erasure»), but **there is no public path to obtain the compaction that would fulfil a deletion request**. An application that has to honour a deletion has no way to do it. This touches the promise of ADR-0009 option 3 (immediate logical deletion and removal at compaction), which the public API cannot fulfil by itself.

**Work order before 1.0.** Decision (d) of §8 turns this list into the entry of the public API work. Its proposed order: first group F (the promise), then B and A, which change the shape of the types; C and D after; E at the end, because it changes no public type.

## 7. Process facts

### 7.1 E8-03: review rounds

The minutes and the repository count **five rounds** for E8-03 (data from the task). The commit history touching `derived_stale.rs` or its test, in order, is the following. **The assignment of each commit to a round is not written in any document of the repository** (F-12); it is a reading of the messages:

| Round | Commit | What the message says | What it found |
|---|---|---|---|
| 1 | `b938ece` | First implementation: `DerivedDb`, marking by dependency under the register lock, rebuilding and superseded; 16 new tests | Initial implementation |
| 2 | `23af819` | E8-03 package; the session was cut by the usage limit; `Locks-Verified: no (blocked)` | No complete review: the package was not verified |
| 3 | `3528ef9` | New pass after compilation failures from an obsolete build cache; no file changed | None; verification repeated |
| 4 | `ecafa8f` | Closes **two blockers** of the review: the duplicate guard in `rebuild` and the revision term in `input_hash`; and adds mutation tests for each branch of `derived_stale.rs`. Suite from 675 to 688 tests | Two blockers resolved; mutation gaps covered |
| 5 | `3238aa4` | The commit-failure test now injects a sync failure into `SimVfs` in both engines | The commit-failure test was not a real test |
| 6 | `6bbadbd` | Final package of E8-03; the worker ran out of turns without a verdict; the verification command passed instead | No new finding; full verification in the session |

According to the task, the correct number is five rounds. The history has six commits with visible review, because the first one (`23af819`) was an interrupted attempt. This review cannot confirm the count of five without the record the task mentions: it is left as **not verified** in the exact count (F-12).

### 7.2 E8-04: two rounds

| Round | Commit | What it found |
|---|---|---|
| 1 | `ea533ca` | E8-04 package; the worker ran out of turns without a verdict; the verification command passed instead |
| 2 | `ca30c28` | One finding: `derived_follow` did not follow the sources when the `ChangeSet` had already written a derived record in a layer-1 database (`db.rs:1005`). Five tests per engine were added (persist and update, persist and delete, update and persist, delete and persist, two units) |

### 7.3 Deviation of ADR-0020 from decision D2

Decision D2 (minutes §8 and §8.1) says that the vectors are persisted with «a new feature bit» and that the bit is `incompat`, in the header format of §5.4 of the minutes. **ADR-0020 does not do that.** The bit is declared as **version 2 of the unit layer** in the `LAYER_META "layer"` marker, with the feature `DERIVED_VECTORS` (ADR-0020, Decision 2 and Options 2). The reason, according to the ADR itself, is four:

1. The files already written do not carry the bit in the header, and putting it on all of them would require rewriting the engine; the MANIFEST is rewritten at each *checkpoint*.
2. The engine does not know what a derived datum is and has no concept of a layer; the header would have to change its rules.
3. Changing `KNOWN_INCOMPAT` is a change to the rules of the frozen format (ADR-0013), with regeneration of vectors.
4. An old build would only fail if the bit were in the MANIFEST, which is replaced at each *checkpoint*; that would couple the engine to the layer.

The layer-version marker fulfils the same function as `incompat`: a layer-1 build rejects the database on opening (UNIT-16, `IncompatibleLayer`). The minutes already record it in §8.1 (D2: «The bit is declared as version 2 of the unit layer, not in the engine»). **The deviation is one of form, not of purpose:** the result that D2 asked for, that an old build does not write over derived data without marking them, is preserved.

## 8. Owner decisions in this closure

Taken **by delegation of the owner on 2026-11-12**, revocable by the owner. They are recorded as they were stated, without adding anything.

- **(a)** Stage 8 is **closed as engineering work**, with **Q4 recorded as not demonstrated**, not as met.
- **(b)** **No threshold and no frozen file is rewritten after measuring.**
- **(c)** A **follow-up package before 1.0** is opened: a second evaluation set, frozen separately, in which **each query class carries query vectors** and whose queries are hard enough not to saturate; its thresholds are fixed before measuring it, and it is measured only once.
- **(d)** The friction list (§6) becomes the entry of the public API work before 1.0.
- **(e)** The ranking stays **switched off by default and experimental**, as was decided when Stage 7 closed.

**Condition of (a).** If the review found a blocking defect, (a) would not hold and nothing would be closed. There is no blocking defect in §4. The caveat F-10 (mutation evidence) stays open for the owner to accept or require.

The follow-up package of point (c) is called **E8-10** in the roadmap. Its full content is in [roadmap, Stage 8](../roadmap/README.md#stage-8--consolidation-and-embeddings).

## 9. Verification: commands and actual output

All were run in the working tree or in the probe copy indicated.

### 9.1 Suite of `kerno-units` and `kerno-testkit` in release (command 1)

```text
cargo test -p kerno-units -p kerno-testkit --all-features --release
```

Result: exit code 0. Totals of all the crates (unit, integration and documentation tests):

```text
passed=856 failed=0 ignored=8
```

The eight ignored: `struct_vectors.rs:769` («writes the committed vectors») and seven documentation fragments not run in `kerno-testkit` and `kerno-units`.

### 9.2 The new test files appear in the runner (command 2)

Output of `cargo test`, filtered to the Stage 8 files:

```text
tests/derived_eval.rs          15 passed
tests/derived_registry.rs      13 passed
tests/derived_stale.rs         46 passed
tests/derived_suppression.rs   19 passed
tests/derived_vectors.rs       77 passed
tests/consolidation.rs         30 passed   (15 pruebas × 2 motores)
tests/derived_measure.rs        4 passed
```

And, in the same command, `tests/cache.rs` (4), `tests/codecs.rs` (21) and `tests/assoc_rank.rs` (19) with their own figures.

### 9.3 Format (command 3)

```text
cargo fmt --all --check
fmt exit=0
```

### 9.4 Clippy (not run in this closure)

The closure only changes documentation. The last output of `cargo clippy --workspace --all-targets --all-features -- -D warnings` of Stage 8 is cited in [measurement §8](stage8-measurement.md) (`Lints: 0`). **It has not been run again.**

### 9.5 Cited tests (command 4)

Each name cited in §5 and in [invariants](invariants.md) §2.6 was searched with `grep -rl` in `crates/*/tests` and `crates/*/src`. All exist. None is missing.

### 9.6 Dependencies (command 6)

```text
git log --oneline 52f026f..HEAD -- Cargo.toml Cargo.lock crates/kerno-units/Cargo.toml crates/kerno-testkit/Cargo.toml | wc -l
0
```

### 9.7 Probes in a copy of the tree (command 7)

Copy of `53b7fa1` (`git archive`) in a working directory, with the probe file, run in release with `--test-threads=1` and `--nocapture`:

```text
running 10 tests
test emulated_log::t1_delete_then_restore ... P1 restore after delete in one change set: is_ok=false
test emulated_log::t2_retention_then_reopen ... P2 removed=1 unreadable=0
P2 reopen dropped=1 (expect old id only)
test emulated_log::t3_stale_at_close ... P3 same sources again: Err: a derived datum with this identity is already registered
test emulated_log::t4_delete_while_closed ... ok
test emulated_log::t5_no_read_returns_a_summary ... ok
test kerno_db::t1_delete_then_restore ... P1 restore after delete in one change set: is_ok=false
test kerno_db::t2_retention_then_reopen ... P2 removed=1 unreadable=0
P2 reopen dropped=1 (expect old id only)
test kerno_db::t3_stale_at_close ... P3 same sources again: Err: a derived datum with this identity is already registered
test kerno_db::t4_delete_while_closed ... ok
test kerno_db::t5_no_read_returns_a_summary ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

The copy is deleted at the end of the review.

## 10. Documents this closure updates

| Document | Change |
|---|---|
| [`stage8-record.md`](stage8-record.md) | E8-08 marked done (§3); rows of §1, §7 and §10 closed with their result |
| [`invariants.md`](invariants.md) | I-25 and I-38 promoted to Stage 8 with their tests (§2.6); I-42 and I-43 added as verified proposals, pending approval |
| [`../roadmap/README.md`](../roadmap/README.md) | Stage 8 closed with the open points and package E8-10 |
| [`SAD.md`](SAD.md) | Version 0.3.0: derived data, persisted vectors, suppression and summaries as they are built |
