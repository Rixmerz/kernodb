# General report on Kerno

| Field | Value |
|---|---|
| Date | 2026-11-13 |
| Status of the document | Reflects the tree after the four packages that follow Stage 9 (P1 to P4). Date 2026-11-13. **No test and no build was run to write it**: the figures of each package are those of its documents, with their date |
| Reference tree | commit `758b5d4`. The four packages are in the history of this commit |
| What it is | A synthesis of what the repository's documents say. It adds no new measurements or tests. Where the repository has no information, it says so in those words |
| How it was read | In this update the documents of P1 to P4 were read in full ([post9-pending](post9-pending.md), [post9-mutations](post9-mutations.md), [requirements-1.0](requirements-1.0.md), [cla-draft](../legal/cla-draft.md), [units-v4](../file-format/units-v4.md), [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md), [ADR-0005](../adr/0005-public-identifiers.md) and [ADR-0019](../adr/0019-license-and-contributions.md)). In addition, [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md) was read for its criterion T5, the [Stage 9 minutes](stage9-record.md) in §2.4 and §8.1, and [stage9-fixtures](stage9-fixtures.md) §3. The claims about the code were checked with text searches and by reading the files cited. Of the rest of the previous version (closures, minutes, measurements, roadmap, ADR index, CI) the citations are kept, without rereading them in full. `docs/research/` and `experiments/` were not read; they are outside this update |

> **Status of the packages after Stage 9.** This report reflects the tree after the four packages: P1 (K1 and unit format v4), P2 (missing tests, an error message and an intermittent test), P3 (the three gaps of ADR-0016) and P4 (audit of the 1.0 requirements, a CLA draft without force and a record of the randomness of the identifiers). It is written as of 2026-11-13. The figures of each package are those its document gives, with its date. **No test was run to write this report**, because the task forbade building. The two documentary inconsistencies that P4 found and that nobody corrected are in section 5.9.

## Conventions of this document

**Stages and phases.** The specification numbers the work in stages 0 to 9. The SAD numbers the phases 0 to 6. The correspondence table is in [`docs/spec/README.md`](../spec/README.md). Here the stage numbering is used. "Stage 0" is the M0 phase of the SAD. "Stage 2" is Phase 1 of the SAD, the persistent core.

**Labels that appear in the documents.** Each document uses its own labels. These are the ones that appear here:

| Label | What it is | Where it is defined |
|---|---|---|
| ADR-00NN | Architecture decision record: a durable decision, with context, options and evidence | [ADR index](../adr/README.md) |
| I-NN (I-01 to I-45) | Invariant: something no change may break, and the test that verifies it | [`invariants.md`](invariants.md) |
| UNIT-NN | Verification rule of the units layer, applied by `verify_units` | [`units-v1.md`](../file-format/units-v1.md) and following |
| K1 to K5 | Findings of the review of the `kerno-units` reads against `verify_units` (K-E3-7). K1 is that a relation with only one adjacency entry is invisible from the end that loses it | [K-E3-7 review](stage9-k-e3-7.md) |
| K-E3-1 to K-E3-9 | Open risks of Stage 3, inherited by the following stages | [Stage 3 closure](stage3-closure.md) |
| H-1 to H-14, F-01 to F-15, R-1 to R-12 | Review findings of each stage. Each closure uses its own letter | Each stage closure, in its review section |
| D1 to D23 | Decisions of the Stage 9 minutes. Their table is in [minutes §8.1](stage9-record.md) | [Stage 9 minutes](stage9-record.md) |
| E9-02, E8-10, E7-05… | Work packages of each stage | Each minutes document |
| T1 to T10 | Performance targets of Phase 1 | [`f1-performance-goals.md`](../testing/f1-performance-goals.md) |
| U1 to U6 | Performance targets of Stage 3 | [`stage3-report.md`](../testing/stage3-report.md) |
| Q1 to Q6, P1 to P4 of Stage 8 | Quality (Q) and time (P) criteria of Stage 8. **P1 and P2 here are time ratios of Stage 8, not the packages that follow** | [Stage 8 minutes](stage8-record.md) |
| P1 to P4 following | Packages after Stage 9, already in the tree. See section 2 | This document |

**Terms of the engine.** The storage engine is a tree of levelled merges, called LSM. Its write-ahead log is the WAL. Its register of live files is the MANIFEST. Its sorted tables on disk are the SSTs. A commit has two durability modes: `Sync` (synchronises on each commit) and `Buffered` (does not synchronise on each commit). The definitions are in [ADR-0002](../adr/0002-transaction-model-and-durability.md) and in [v1.md](../file-format/v1.md).

**Terms of the units layer.** A unit (`Unit`) has identity, type and revisions. A `ChangeSet` groups the writes of a commit. A declared relation joins two units with a type. An adjacency index allows walking the relations of a unit. A derived item is data computed from units, with its sources registered. A vector is a derived item with a fixed number of components. A summary is a derived item of text with mandatory sources.

**Two engines in the tests.** The tests of `kerno-units` run twice: over `emulated_log` (the `LogEngine` of `kerno-testkit`, [`reference.rs`](../../crates/kerno-testkit/src/reference.rs)), and over `kerno` (`KernoDb`, the real engine). Its documentation says that `LogEngine` keeps all the state in memory and a log of batches with checksums on disk, and that it exists to validate the test harness: if it fails a shared test, the test is wrong. The tests that cite "in both engines" do that.

---

## 1. Summary on one page

**What Kerno is.** Kerno is an embedded, local-first data engine. It combines transactional persistence, versioned evolution of data and verifiable integrity. It has an extension path towards hybrid retrieval for AI applications. It is written in Rust. Its code is split into four crates: `kerno-core` (the shared contract), `kerno-storage` (the engine), `kerno-testkit` (the test infrastructure and the reference engine) and `kerno-units` (the units layer: revisions, relations, indexes, planner and derived data). Source: [README](../../README.md) and [SAD §14](SAD.md#14-proposed-repository-organisation).

**Where it stands today.**

- **There is no version 1.0.** The README says so in its first status section. There is no promise of stability of the format or of the API. Source: [README](../../README.md).
- **The v1 on-disk format has been frozen** since 2026-10-31, with [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md). That is not a compatibility promise for users.
- **Stages 3 to 9 are closed**, with nuances. Stage 8 is closed "as engineering work". Its criterion Q4, retrieval quality with and without vector signal, **is not demonstrated**. Stage 9 has its closure approved by the owner on 2026-11-13. Sources: [roadmap](../roadmap/README.md), [Stage 8 closure](stage8-closure.md), [Stage 9 closure](stage9-closure.md).
- **The packages after Stage 9 are in the tree.** P1 closes K1 for relations with a count record. P2 adds tests and corrects a message and an intermittent test. P3 closes the three gaps of ADR-0016. P4 writes the audit of the 1.0 requirements, a CLA draft without force and the record of the randomness of the identifiers. Detail in section 2. Sources: [post9-pending](post9-pending.md), [post9-mutations](post9-mutations.md), [requirements-1.0](requirements-1.0.md).
- **1.0 has not been declared.** That depends on D13 and on real use outside the team (D12). Source: [Stage 9 minutes §8.1](stage9-record.md).

**What matters most, in four points.**

1. **What has been tested is a lot, but it has been tested on one type of machine.** There are crash tests at each I/O point, corruption tests, real `kill -9`, a sweep of 1 000 seeds, fifteen fuzzing targets and 856 tests in `kerno-units` and `kerno-testkit` (figure of 2026-11-12). After P3, the list of tests of `kerno-units` has 870 (2026-11-13; it is a count of the list, not a full run). All of it comes from a four-core VM with Linux and ext4. No measurement was made on the target machine, which the repository does not name (C-09). Sources: [Phase 1 report §4](../testing/f1-report.md), [Stage 9 closure §5](stage9-closure.md).
2. **Several performance targets are not met, and that is accepted and written down.** In Phase 1, T1 (write of `agent_memory`, 2.78 times the LSM reference against a maximum of 2.0) and T7 (write amplification, 4.29 against 1.20) are not met. In Stage 3, U2 and U6 are not met; the owner accepted it as an open risk, without lowering thresholds. Sources: [Phase 1 report §5.3](../testing/f1-report.md), [Stage 3 closure §4](stage3-closure.md).
3. **There are integrity and security limits that are not built.** There is no encryption at rest (D3) and no authenticated integrity (D4). A filter of an SST rewritten with a valid CRC can pass opening and answer "does not exist" without error for keys that do exist. Opening rejects the coarse case, but only `verify()` with `deep` reports the case of interior keys. There is no guaranteed physical deletion: deletion is logical and the public reclaim is a mechanism, not a secure deletion. Sources: [threat model §3 and §6](../security/threat-model.md), [ADR-0009](../adr/0009-suppression-retention-and-backups.md).
4. **Some capabilities are switched off or left unproven on purpose.** The ranking of associative results is switched off by default and is experimental. The result cache is switched off by default, and its net improvement is not claimed. With the cache switched on, it stores nothing while the observer is below `Fingerprint` (P3, section 4.9). Retrieval quality with vectors is not demonstrated, and the vectors in the tests are deterministic with a seed. Sources: [Stage 7 closure §8](stage7-closure.md), [Stage 6 closure §0](stage6-closure.md), [Stage 8 closure §0.3](stage8-closure.md).

**What is best to do first, in this order.**

1. **Review P1 before considering it closed.** The defect that the independent review found (orphaned counters in a rejected relation creation) is corrected and covered by mutation M20 (section 4.7). But [post9-pending](post9-pending.md) still lists the independent review of P1 as pending, and P1 touches the on-disk format.
2. **Clone the repository on a machine of your own and run the battery in section 9** (format, clippy, the tests of the two crates, dependency guards). The documents do not give complete durations. Section 9 says what is known.
3. **Repeat in a quiet state the time measurements of Stage 8 (P1 and P2, ratios)**, and the longer run of the fuzzing of `wal`, `manifest` and `open`. Both were deliberately left for the owner's machine, because the disk of the build machine was almost full and is shared. Section 9 gives the commands.
4. **Decide the text of the contributor agreement (CLA) before accepting any external contribution.** There is a draft without force ([cla-draft](../legal/cla-draft.md)), which needs a lawyer's review and the owner's approval (D15). Source: [ADR-0019](../adr/0019-license-and-contributions.md).
5. **Do not declare 1.0** until D13 is met and there is real use outside the team (D12). The list of requirements and their status are in section 8.
6. **Correct or decide the two inconsistencies of section 5.9.** One is in the Stage 9 minutes, closed, and the other in ADR-0020, accepted.

---

## 2. What has been achieved

The table summarises each stage. The dates are those of each closure. When a closure was approved by delegation of the owner, it is said.

| Stage (SAD) | Closure date | What it built | Where it is documented |
|---|---|---|---|
| **0 / M0** (Phase 0) | 2026-10-28 | Two storage prototypes, a copy-on-write B+ tree and a levelled LSM, with the same tests and the same benchmarks. The LSM was chosen (ADR-0001). ADR-0002 (transactions and durability) and ADR-0003 (format and versioning) accepted the same day. Failure matrix. Benchmarks of 410 runs | [m0-report](../testing/m0-report.md), [ADR-0001](../adr/0001-storage-engine.md), [ADR-0002](../adr/0002-transaction-model-and-durability.md), [ADR-0003](../adr/0003-physical-format-and-versioning.md), [failure matrix](failure-matrix.md), [spikes/storage](../../spikes/storage/README.md) |
| **1** (minimal contracts) | Partial | Storage and transaction contracts defined. Unit, revision and relation moved to Stage 3. The specification's own table marks it as "Partial" | [docs/spec/README.md](../spec/README.md) |
| **2 / Phase 1** (persistent core) | Closed according to the roadmap, dated 2026-11-11 | `kerno-core` and `kerno-storage`: format v1 (frozen with ADR-0013, accepted on 2026-10-31), LSM engine, WAL, MANIFEST as an edit log, range deletions, read-only opening, group commit, read-modify-write transactions. Adversarial review with defects corrected (section 4) | [roadmap](../roadmap/README.md), [Phase 1 report](../testing/f1-report.md), [adversarial review](../testing/f1-adversarial-review.md), [v1.md](../file-format/v1.md), [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) |
| **3** Units, revisions and provenance | 2026-11-04 (approved by the owner) | `kerno-units`: units with life cycle, `ChangeSet`, revisions, history, `restore`, policies `none`, `audit` and `versioned`, retention by batches, `verify_units`. Two performance targets (U2 and U6) are not met and are accepted as a risk | [closure](stage3-closure.md), [stage3-report](../testing/stage3-report.md), [units-v1](../file-format/units-v1.md), [design](stage3-design.md), [ADR-0004](../adr/0004-revision-history.md), [ADR-0010](../adr/0010-public-api-and-crate-boundaries.md) |
| **4** Relations and indexes | 2026-11-05 (approved by the owner) | Declared relations (`REL` and two adjacency indexes, ADR-0014), primary index with state and watermark, type index `IDX_TYPE` justified by the W1 load, reference path without indexes for all queries | [closure](stage4-closure.md), [minutes](stage4-record.md), [workloads](stage4-workloads.md), [ADR-0007](../adr/0007-index-consistency-and-rebuilding.md), [ADR-0014](../adr/0014-physical-representation-of-relations.md) |
| **5** Planner and observability | 2026-11-06; approved by delegation on 2026-11-09 | Deterministic seven-step planner, with plan explanation. `ExecutionObservation` with four levels and bounded retention. Sentinels that prove there are no literals outside the `diagnostic` level | [closure](stage5-closure.md), [planner](stage5-planner.md), [observation](stage5-observation.md), [sentinels](stage5-sentinels.md), [ADR-0015](../adr/0015-privacy-of-observability.md), [adaptive execution](adaptive-execution.md) |
| **6** First operational plasticity | 2026-11-11; approved by delegation | A single optimisation: a result cache tied to `commit_seq`, used by the planner with a recorded fallback. **Switched off by default.** Net improvement not claimed | [closure](stage6-closure.md), [measurement](stage6-measurement.md), [reversibility](stage6-reversibility.md), [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) |
| **7** Associative memory | 2026-11-12; approved by the owner | Associative query with hard bounds (depth 4, 1 000 results, 100 000 visits), execution by index with a check of each answer, verifiable trail of each result (I-40), ranking with diagnostic mode (I-41). **The ranking is switched off by default and is experimental.** The evaluation set meets T1 to T3 and T5, but not T4 | [closure](stage7-closure.md), [query](stage7-associative-query.md), [indexed execution](stage7-indexed-execution.md), [context and provenance](stage7-context-and-provenance.md), [ranking](stage7-ranking.md), [measurement](stage7-measurement.md), [set](stage7-evaluation-set.md), [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) |
| **8** Consolidation and embeddings | 2026-11-12, closed as engineering work | Common registry of derived data with states `ready`, `building`, `stale`, `failed` and `superseded` (ADR-0018). Obsolescence and rebuilding in the same commit as the source change. Persisted vectors as derived data (ADR-0020). Suppression of sources and derived data (ADR-0009). Consolidation with summaries of mandatory provenance and a deterministic test summariser. **Q4 not demonstrated.** A follow-up package (E8-10) is needed before 1.0 | [closure](stage8-closure.md), [minutes](stage8-record.md), [set](stage8-evaluation-set.md), [measurement](stage8-measurement.md), [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md), [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md), [ADR-0009](../adr/0009-suppression-retention-and-backups.md) |
| **9** Hardening | 2026-11-13; closure approved by the owner in person | Public identifiers with an opaque token (E9-02, ADR-0005, merged on 2026-11-13). Minimised audit (E9-03). Export and import with a draft format, unfrozen (E9-04 to E9-06). Public `Reclaim` (E9-07). Withdrawal of `UnitDb::engine()` (E9-08). Derived-data reading, stress, robust decoders, fuzzing of fifteen targets, mutations of Stage 8, review of K-E3-7, new SQLite report, set and measurement of E8-10, real-use example `agent-memory` with fifteen friction points | [closure](stage9-closure.md), [minutes](stage9-record.md), [advance](stage9-early-closure.md), [audit](stage9-audit.md), [fixtures](stage9-fixtures.md), [K-E3-7 review](stage9-k-e3-7.md), [uses of engine()](stage9-engine-uses.md), [SQLite E9 report](../testing/sqlite-report-e9.md), [E9 measurement](stage9-measurement.md), [FRICTION](../../examples/agent-memory/FRICTION.md), [ADR-0005](../adr/0005-public-identifiers.md), [ADR-0006](../adr/0006-encryption-and-key-management.md), [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md) |

**Packages after Stage 9** (status according to the tree and the documents of each package; the commits are in the history of `758b5d4`):

| Package | Status | What it is | Where it is documented |
|---|---|---|---|
| **P1** | **Delivered** (`0c2c3d5`, with the correction `4b65c51` and `0310bb3`) | Closes K1 for relations with a count record: unit format v4, with a record per `(unit, direction, type)` that the reader compares with the adjacency entries and that `verify_units` checks (UNIT-45 and UNIT-46). An independent review found a defect in rejected creation, corrected and covered by mutation M20 (section 4.7). 20 mutations (M1 to M20), all red | [units-v4](../file-format/units-v4.md), [post9-mutations](post9-mutations.md), [post9-pending](post9-pending.md) |
| **P2** | **Delivered** (`2356ff3`) | Missing tests (Stage 9 closure, points 4, 5, 6 and 10), the message of `NoValidSources`, and the intermittent test `space_is_reclaimed_after_range_deletes`, whose cause was a timing assumption of the test itself (section 4.8). 18 mutation records: Q1 to Q16, plus Q13b (discarded candidate, green) and Q13c | [post9-mutations](post9-mutations.md) (P2 record), [post9-pending](post9-pending.md) (points 3 to 6, 10 and 11) |
| **P3** | **Delivered** (`fc7e233`) | The three gaps of ADR-0016 ("open work" 1 to 3): the plan explanation says whether the cache was consulted, the creation evidence is that of the observer and the entry carries a measured cost, and `set_key` leaves a record. Behaviour change: with the observer below `Fingerprint`, the cache stores nothing (section 4.9). 17 mutations (M1 to M17), all red | [ADR-0016, "Review of 2026-11-13"](../adr/0016-operational-adaptation-and-invalidation.md), [post9-mutations](post9-mutations.md) (P3 record) |
| **P4** | **Delivered** (`f2aa157`, documents only) | Audit of the seven requirements of D13: six with declared limits and one met; 1.0 is not declared. CLA draft, **without force**. Record of the randomness of the public identifiers (`/dev/urandom`, without a new dependency). Two documentary inconsistencies that were not corrected (section 5.9) | [requirements-1.0](requirements-1.0.md), [cla-draft](../legal/cla-draft.md), [ADR-0005, "Notes"](../adr/0005-public-identifiers.md), [post9-pending](post9-pending.md) |

---

## 3. What has been tested, and how

This section separates each kind of evidence. For each one it says what it covers and what it does not. The figures come with the date of the document that gives them. **The repository has no total count of tests after Stage 9.** The Stage 9 closure says it did not rerun the count (T-CNT).

### 3.1 Test counts the documents give

| Scope | Figure | Date | Source |
|---|---|---|---|
| `kerno-units` and `kerno-testkit`, release, all features | 856 passed, 0 failed, 8 ignored | 2026-11-12 | [Stage 8 closure §9.1](stage8-closure.md) |
| `kerno-units` and `kerno-testkit`, with the Stage 7 closure | 645 passed, 0 failed, 8 ignored | 2026-11-12 | [Stage 7 closure §5](stage7-closure.md) |
| `kerno-units` only, release | 320 passed, 1 ignored | 2026-11-11 | [Stage 6 closure §5](stage6-closure.md) |
| `kerno-storage`, release | 649 passed, 4 ignored | 2026-11-06 | [Stage 5 closure §4](stage5-closure.md) |
| Workspace without `kerno-storage`, release | 615 passed, 8 ignored | 2026-11-06 | [Stage 5 closure §4](stage5-closure.md) |
| Full workspace | 1 212 before E4-05 and 1 214 after | 2026-11-05 | [Stage 4 closure §2](stage4-closure.md) |
| `kerno-units`, test list (`--list`) | 822 before P1 and 853 after (+31 in `rel_counts.rs`) | 2026-11-13 | [post9-mutations](post9-mutations.md) |
| `kerno-core`, `kerno-storage`, `kerno-testkit` and `kerno-units`, test list | 1 798 in the base tree of P2 and 1 804 with P2 (+6: `reclaim.rs`, `db.rs` and `derived_lock_order.rs`) | 2026-11-13 | [post9-mutations](post9-mutations.md), P2 record |
| `kerno-units`, test list (`--list`) | 859 before P3 and 870 after (+3 in `cache.rs`, +8 in `cache_planner.rs`) | 2026-11-13 | [post9-mutations](post9-mutations.md), P3 record |
| `kerno-units`, E9-02 | 49 tests without `test-support` and 79 with it, in both engines | 2026-11-13 | [Stage 9 closure §1](stage9-closure.md) |
| Storage of M0, B+ tree prototype | 32 own unit tests | 2026-10-27 | [m0-report §2.1](../testing/m0-report.md) |
| Storage of M0, LSM prototype | 24 unit and 7 integration | 2026-10-27 | [m0-report §2.1](../testing/m0-report.md) |

The figures are not comparable with each other. Each one counts a different scope. A test that no longer exists stops counting. For example, `completeness_is_not_a_query_time_check` was renamed `a_relation_lost_with_both_entries_is_caught_by_its_counts` in P1, and the Stage 7 closure still cites the old name ([post9-pending](post9-pending.md)).

### 3.2 Unit and integration tests of the two engines

- **What they are.** Tests in `crates/kerno-storage/tests/` (42 entries, with subfolders), `crates/kerno-units/tests/` (54 entries, with subfolders) and `crates/kerno-testkit/tests/` (5 files). Each test of `kerno-units` that concerns data runs in `emulated_log` and in `kerno`.
- **What they cover.** The rules `UNIT-n`, the codes of each record, the reading and writing of units, revisions, relations and their count records (P1, `rel_counts.rs`), indexes, the cache, the planner, derived data, export and import, public reclaim and the decoders. The byte vectors (`tests/vectors/`) fix the encoding of each version of the format.
- **What they do not cover.** Databases without a count record (from before layer 4) with more than 64 relations and an entry missing at both ends (H-1 of Stage 7, section 5). For relations with a count record, P1 adds tests in both engines (`rel_counts.rs`). No concurrency test between readers of the cache (H-1 of Stage 6, section 3.4 of its closure). No test on platforms other than Linux with ext4 (see section 6).

### 3.3 Differential test against the reference engine

- **What it is.** The same history of operations is run on the real engine and on the in-memory reference engine (`emulated_log`), or on the reference path without indexes, and the answers are compared. Examples: `differential_seeded` in `tests/index.rs` (six seeds, two engines, forty steps; since E4-05 it includes `restore` and `run_retention`, [Stage 4 closure §2](stage4-closure.md)), `differential_across_scan_pages`, and the `differential_*` tests of `tests/relations.rs`.
- **What the planner's one tests.** `cache_on_off_and_invalidated_give_identical_results` compares the answer with the cache switched on, off and invalidated, with writes between rounds ([Stage 6 closure §2](stage6-closure.md)).
- **Frozen corpus.** The 64 queries of the Stage 7 set give the same answer by the reference, by the index and by the planner: 64 of 64, without failures ([Stage 7 closure §4.1](stage7-closure.md)). The set has a `MANIFEST` with SHA-256 that is checked before measuring.
- **What it does not cover.** That the index path is complete in stores of 64 relations or more with an entry missing at both ends, in databases without a count record. For relations with a count record, P1 makes it detected at query time (`Fallback` `Corrupt`, [units-v4 §5](../file-format/units-v4.md)). Without a count, only `verify_units` sees it (Stage 7 closure, R-2). Nor does it cover damaged stores, except in the tests that damage them on purpose.

### 3.4 Crashes and fault injection

- **Simulator.** `SimVfs` in [`crates/kerno-testkit/src/vfs/sim.rs`](../../crates/kerno-testkit/src/vfs/sim.rs) simulates power cuts at each I/O point, process deaths, torn writes, truncations and `fsync` errors. The failure matrix ([failure-matrix](failure-matrix.md)) says what must remain after reopening at each point.
- **Sweeps.** A crash before each I/O operation, an isolated EIO in each operation, and the failed `fsync` mode with Linux semantics, in `Sync` and in `Buffered` (M0 §2.1, [Phase 1 report §2](../testing/f1-report.md)).
- **A thousand seeds (2026-10-29).** In the default variant of the product engine: 1 000 truncations detected on opening, 0 harmless, 0 wrong data. Bit flips: 256 detected on opening, 744 on reading, 0 harmless, 0 wrong data ([Phase 1 report §2](../testing/f1-report.md)).
- **Real `kill -9` (2026-10-29).** 200 deaths of a writer in `Sync` and 200 in `Buffered`, both tests green. Two more tests with four concurrent writers, added after that report, with their output in [`process-kill-200.log`](../../spikes/storage/results/2026-10-29-f1/process-kill-200.log).
- **What it does not cover.** A real device that loses pages. The LazyFS record is **pending**: the Phase 1 report says that no run is recorded ([§4](../testing/f1-report.md)). The simulator modelled the failure of `wait_writeback` badly (G1), and that was corrected (section 4). The `kill -9` model does not lose the operating system's cache, and the document itself says so ([failure-matrix §1](failure-matrix.md)). The `loom` models run only in the weekly CI, and the documents do not give a record of their result.

### 3.5 Fuzzing

Fifteen targets over the decoders of `kerno-storage` and the opening of a database. They are in [`crates/kerno-storage/fuzz/`](../../crates/kerno-storage/fuzz/README.md), with their own workspace. They do not run in the CI.

Three runs are documented:

| Run | Date | Duration per target | Result |
|---|---|---|---|
| Short, with the committed corpus | 2026-11-10 | 60 s | No failures, no entries over 1 s |
| E9A-04, first thirty-minute run | 2026-11-12 | 1 801 s | No failures in the fifteen |
| E9-14, on the final code of Stage 9 | 2026-11-12 | 1 801 s, four targets at a time | No failures in the fifteen, no new entries in the corpus |

Results of the E9-14 run, with the machine load around 4 on 4 CPUs ([Stage 9 closure §5, point 7](stage9-closure.md)):

| Target | Executions in 30 min | Failures |
|---|---:|---:|
| varint | 427 959 903 | 0 |
| header | 296 364 723 | 0 |
| commit | 207 625 874 | 0 |
| frame | 313 543 961 | 0 |
| data_block | 198 919 045 | 0 |
| filter_block | 262 139 947 | 0 |
| rangedel_block | 175 649 868 | 0 |
| props_block | 242 636 008 | 0 |
| index_block | 183 972 923 | 0 |
| footer | 499 398 250 | 0 |
| record | 310 873 073 | 0 |
| **wal** | **4 877 579** | 0 |
| sst | 198 623 700 | 0 |
| **manifest** | **6 676 172** | 0 |
| **open** | **4 546 088** | 0 |

Source: [`fuzz/README.md`](../../crates/kerno-storage/fuzz/README.md), section "Stage 9 run on the final code". The first thirty-minute run (E9A-04) gave figures of the same order for `wal` (5 698 047), `manifest` (7 420 267) and `open` (4 959 653). The `wal`, `manifest` and `open` targets do file work per entry. Their number of executions is two orders of magnitude less than that of the pure decoders. The closure says that thirty minutes is little for them. **The fuzzing of `kerno-units` does not exist**: D21 decided to use only property tests with the library that is already a dependency.

What it does not cover: longer runs of `wal`, `manifest` and `open` on a free machine (pending, left for the owner's machine; section 9.3), and any input that is not a database file. The CI fuzzing does not exist: the threat model says that T-10 remains open because the CI does not run fuzzing or dependency auditing ([threat-model §3](../security/threat-model.md)).

### 3.6 Mutations

A mutation changes one line of production or test code, and it is checked that some test turns red. Each package has its record.

| Package | Mutations | Result the document gives | Source |
|---|---|---|---|
| M0 harness (validation) | 8 mutations by hand | Each detected by a test. Two were real errors of one engine | [test-plan §2.1](../testing/test-plan.md) |
| E3-10 (Stage 3 review) | One per invariant IU-1 to IU-10 | Each invariant with its test that fails when it is broken; findings H-1 to H-3 | [Stage 3 closure §2](stage3-closure.md) |
| E4-05 | One per invariant of the stage | Gaps H-1 to H-5 corrected | [Stage 4 closure §3](stage4-closure.md) |
| E5-05 | 3 production mutations | All three detected. One of them is not seen by the sentinel search: it is detected by the structural test `levels_below_diagnostic_carry_no_shape_text` | [Stage 5 closure §3.1](stage5-closure.md) |
| E6-05 | 3 production mutations | Two detected; **one survives** (H-7: with the cache switched off, the code still takes its path, and no test sees it) | [Stage 6 closure §3.3](stage6-closure.md) |
| E7-02 | 29 mutations | All detected | [Stage 7 closure §0](stage7-closure.md) |
| E7-03 | 16 mutations | All detected | [Stage 7 closure §0](stage7-closure.md) |
| E7-04 | 42 mutations (M01 to M42) | All detected | [Stage 7 closure §6.2](stage7-closure.md) |
| E7-06 (replica) | 4 mutations in a copy of `HEAD` | All four detected by some test. The `Contributions::sum` +1 is seen by 8 tests of the `assoc_rank` oracle, and **not** by the `assoc_measure` measurement (R-3) | [Stage 7 closure §3.3](stage7-closure.md) |
| E8 (probes) | 5 probes in a copy of the tree, two engines | `10 passed; 0 failed`. Three observations recorded | [Stage 8 closure §2.3](stage8-closure.md) |
| E9-02 | 28 mutations (M01 to M25, M27 to M29) | All detected | [stage9-mutations-e9-02](stage9-mutations-e9-02.md) |
| E9-03 (audit) | 5 mutations (M1 to M5) | All detected by the target test | [stage9-audit §3](stage9-audit.md) |
| E9-15 (mutations of Stage 8) | M1 to M15 over `consolidation.rs` and D1 to D4 over `derived_measure.rs` | The 19 tests have a mutation. Two mutations (D3 and D4) make only one test fail | [stage9-mutations-e8](stage9-mutations-e8.md) |
| E9-20 (measurement of E8-10) | 7 mutations over `derived_measure2.rs` | All detected | [stage9-measurement §4](stage9-measurement.md) |
| E9-22 (K1 and K2) | 1 mutation per finding | Detected in both engines | [stage9-k-e3-7 §8](stage9-k-e3-7.md) |
| P1 (K1, format v4) | 20 mutations (M1 to M20), all red (`exit=101`). M20 is the correction of the independent review | None survived. M15 mutates a test, not `src/` | [post9-mutations](post9-mutations.md) |
| P2 (tests and text) | 18 records: Q1 to Q16, Q13b and Q13c. All of the new or changed tests, in red. **Q13b**, in green, is a discarded candidate | Q13 mutates the test, not `src/`. Q5 moves the taking of the lock, and the lock during marking has no mutation that shows it | [post9-mutations](post9-mutations.md) (P2 record) |
| P3 (ADR-0016) | 17 mutations (M1 to M17), all red | None survived. M9 was repeated after hardening its test | [post9-mutations](post9-mutations.md) (P3 record) |

**Limits of the mutations.** Many were done on a copy of the tree, one at a time, and restored. Several are "broad": a mutation that breaks everything also makes the target test fail, and that proves little (the E9-15 document itself says so). The Stage 7 closure found that a mutation of `sum` was not seen by the measurement (R-3). The mutation of each new test of E8-06 and E8-07 had no record until E9-15 (F-10 of Stage 8).

### 3.7 Benchmarks and measurements

All the figures come from the documents cited. **None was measured on the target machine**, which the repository does not name (C-09). The conditions of each one are in the table.

| Measurement (date) | Conditions | Main figures | What it does not cover | Source |
|---|---|---|---|---|
| **M0** (2026-10-27) | VM of 4 cores and 15.7 GiB, ext4, kernel 6.18. `fdatasync` of 4 KiB: p50 113 µs. Release build. 410 runs, 0 failures. Scales small, medium and large. Three interleaved repetitions | Pipeline in `Sync`: LSM 3 087 steps/s, B+ tree 1 111. Simulated history: LSM 66.6 thousand revisions/s, B+ tree 8.3 thousand | Target machine; load from other processes not controlled | [m0-report §3](../testing/m0-report.md) |
| **Phase 1, T1 to T10** (2026-10-29) | Same class of VM. 1-minute load between 0.97 and 1.90. `medium`, `Sync`, median of 3 repetitions. 552 runs of the main sweep | T1: `agent_memory` p99 of write 3 938 µs, 2.78 times the LSM reference; T7: amplification of `small_commits` 4.29 against 1.20 (range 1.17 to 5.01 between repetitions); T4: close after `bulk_load` 251 ms against 250 | Scale `large` incomplete (`range_scans` and `small_commits` were not run in `large`). LazyFS, not run. C-09 | [f1-report §5](../testing/f1-report.md) |
| **Stage 3, U1 to U6** (2026-11-04) | KVM VM of 4 vCPU, virtual ext4 disk with 90 % occupied. Three complete passes on a quiet machine, according to the closure | U1: 1.05 to 1.10 (met). U2 `history`: 1.50, 1.27, 0.95 (not met in one pass). U6 p99 of write with concurrent retention: 1.05 to 1.18; throughput 0.89 to 0.94 (not met) | No indexes or relations in U6 (E5-00 corrects that) | [Stage 3 closure §4](stage3-closure.md), [stage3-report](../testing/stage3-report.md) |
| **U6 with indexes and relations** (E5-00, 2026-11-06) | Raw output committed | p99 of write 1.05 to 1.18; throughput 0.89 to 0.94. Still not met. Does not block | No new variation | [Stage 5 closure §5](stage5-closure.md), [u6 remeasurement](stage5-u6-remeasurement.md) |
| **Planner** (E5-02/E5-03, 2026-11-06) | Shared machine with other processes ("shared-machine noise", according to the closure) | p99 +8.5 % at 100 000 units; +14.1 % at 20 000 in one run and +0.3 % in another. `diagnostic` observation at 20 000: +5.9 % in one run. Minutes' limits: 10 % and 5 % | Machine without load: pending | [Stage 5 closure §5](stage5-closure.md), [planner §7](stage5-planner.md) |
| **Result cache** (E6-04b, 2026-11-11) | 4-CPU machine, load ≈ 1, 20 000 units, four runs | p50 of W1 with cache: −97.8 % to −97.9 %. p99 of W1: +0.5, +7.8, +4.7 and +7.5 %. Paired measurement of the real cost: +3.4 to +4.9 % on the first read after a write | No repetitions, only 20 000 and 100 000. No concurrency. Cache size not varied | [Stage 6 closure §4](stage6-closure.md), [measurement §8](stage6-measurement.md) |
| **Associative quality and performance** (E7-05, 2026-11-11) | Set of 2 000 units and 64 queries. Performance at depth 2, twelve runs, 4-CPU machine with mean load from 1 to 9 | Quality: simple order nDCG@10 0.942; ranking 0.912; baseline 0.500. Index p50 0.147 to 0.159 ms against reference 103.9 to 110.5 ms | Other scales; absolute figures are not comparable between machines | [Stage 7 closure §4](stage7-closure.md), [measurement](stage7-measurement.md) |
| **Derived data, E8-07** (2026-11-12) | New set of 100 queries; 25 with a query vector, all of one same class. P1 and P2 in twelve runs | Q1 = 1.000; Q2 = Q3 = 1.000 in the arm with signal and without it (ceiling effect). Q5 = 1.1 %. P1 (write with descriptor between without descriptor) = 1.165 to 1.238. P2 (persisted search between in memory) = 0.921 to 1.007. P3 and P4 met (286 of 286) | Q4 not demonstrated: three classes without a vector. P2 does not measure the reading of persisted vectors (F-08) | [Stage 8 closure §0](stage8-closure.md), [E8 measurement](stage8-measurement.md) |
| **Derived data, E8-10 in E9-20** (2026-11-13) | A single measurement, with other load active. Four runs of 12 rounds of 64 pairs for P1 and P2. A single synthetic set of 2 000 units | Quality: nDCG@10 with signal 0.9738; without signal 0.4216; S1 +0.5522; Q5 1.6 %. P1 1.140 to 1.173; P2 0.997 to 1.001 | P1 and P2 **not** at rest. Thresholds printed, not asserted (section 5) | [E9 measurement](stage9-measurement.md), [raw output](stage9-measurement-output.txt) |
| **SQLite, E9-17** (2026-11-12) | Scale tiny, 5 000 records. 12 loads, 5 repetitions, durabilities in pairs. 240 runs. 1-minute load between 1.3 and 1.9 | No thresholds. Kerno is slower at opening, recovery and closing. Kerno is faster at point reads, scans and writes, with the warning that the `Buffered` mode has no equivalent guarantees | Not extrapolated to other machines or other scales | [SQLite E9 report](../testing/sqlite-report-e9.md) |

The documents of P1 to P3 do not record a new performance measurement. One change of P3 does affect an existing measure: the cached arm of the `index_bench` bench (`crates/kerno-units/examples/index_bench.rs:335`) raises the observer to `Fingerprint`, so its figure includes the cost of observing and is not comparable with that of Stage 6 ([post9-mutations](post9-mutations.md), P3).

---

## 4. Errors found and corrected

This list collects the defects that the documents record as corrected. Each entry says how it was found, how it was corrected and which test keeps it. When the document does not name the test, it says so.

### 4.1 Storage engine (M0, 2026-10-27)

The harness, the own tests of each prototype and the adversarial review of M0 found these defects. They belong to the prototypes, not to the current engine, but the current engine was born from them ([m0-report §2.2](../testing/m0-report.md)).

| # | Engine | Defect | How it was found | Correction | What keeps it |
|---|---|---|---|---|---|
| M0-1 | B+ tree | The meta slot was chosen by the parity of the sequence. After losing one slot, the next commit overwrote the only valid meta | Own unit test | The report records it as a defect found during implementation; it does not say when it was corrected | Own unit test |
| M0-2 | B+ tree | The directory was synchronised only if the file "had just been created" | `single_io_failure_at_every_point` | Corrected | The sweep of failures of one EIO |
| M0-3 | LSM | If the `rename` of the MANIFEST succeeded and the `fsync` of the directory failed, outputs that the MANIFEST could reference were deleted | Own review | Corrected | The document does not name it |
| M0-4 | Both | The database directory was never synchronised in its parent: a cut could make a new database disappear | Adversarial review | Corrected in both engines | Regressions (not named) |
| M0-5 | B+ tree (high severity) | Opening took as durable a meta that was only visible. Its freed pages were reused, and a later cut could destroy the database (~30 % of the simulations) | Adversarial review | `fsync` on opening, with regression | Regression (not named) |
| M0-6 | B+ tree (medium) | A damaged bit in the most recent meta silently rolled back to the previous commit | Adversarial review | Two copies of the meta per slot, with regression | Regression (not named) |
| M0-7 | B+ tree (medium) | A panic inside the commit did not poison the handle | Adversarial review | Corrected | Not named |
| M0-8 | B+ tree (performance) | Values over ~1 KiB were not cached: one `pread` per page | Adversarial review | Unified cache | Not named |
| M0-9 | LSM (medium-high) | Damage in the middle of the most recent WAL after a dirty close was taken as a broken tail. Commits in `Sync` were silently lost | Adversarial review | `SYNCED` flag per record, with regression | Regression (not named) |
| M0-10 | LSM (low, medium performance) | `close()` abandoned the compaction in progress | Adversarial review | Compacts on close, with regression | Regression (not named) |
| M0-11 | B+ tree and LSM (low) | Panic with a valid-CRC file; overflow of the sequence at `u64::MAX`; batches over 4 GiB unreadable; a snapshot deleted an orphan SST; non-canonical names; unchecked arithmetic | Adversarial review | Corrected, with regressions | Not named |
| M0-12 | Harness | Two races with background work and two coverage gaps (uniform damage that almost never reached targets; tear only at 512 B) | The candidates themselves | Corrected in the harness | Validated with mutations ([test-plan §2.1](../testing/test-plan.md)) |

### 4.2 Adversarial review of Phase 1 (2026-10-31)

Source: [f1-adversarial-review §7](../testing/f1-adversarial-review.md). All of these were corrected before the merge, except those marked.

| # | Severity | Defect | Correction | What keeps it |
|---|---|---|---|---|
| **B1** | **Blocking** | A failed deferred write of an SST was taken as durable: the error of `wait_writeback` was discarded, and the final `sync_data` returned `Ok`. A later cut could lose confirmed data. Review by reading the code and the Linux semantics; not reproduced on a real device | `Sink::write_pending` propagates the error of `wait_writeback`, except `Unsupported` and `InvalidInput`. Commit `555565a`. Checked at [`sst.rs:182`](../../crates/kerno-storage/src/sst.rs) | `write_amp_levers::a_failed_wait_for_writeback_never_loses_acknowledged_commits`. Verified in this note: the name exists in [`write_amp_levers.rs`](../../crates/kerno-storage/tests/write_amp_levers.rs) |
| **G1** | Major (test gap) | The simulator modelled the failure of `WaitWriteback` as "no effect", the opposite of Linux. That is why no sweep could see B1 | `SimVfs` with `LinuxDropPages` discards covered writes for ever and marks the error as seen. Same commit `555565a` | The test of B1 (fails without the fix) |
| **M1** | Minor | A damaged WAL header with a single frame was taken as "header never synchronised" and discarded commits | The writer synchronises the header before the first frame. WAL-R6 treats every frame with a valid CRC under a broken header as `Corruption`. Commit `772f3fa` | Vectors `dirty-wal-one-frame` (positive) and `wal-one-frame-broken-header` (negative) |
| **M2** | Minor | `AccessMode::ReadOnly` opened `LOCK` with write permission: it could not open a database in a read-only directory | `StdVfs::lock` falls back to opening `LOCK` for reading only, or the directory. **Limit:** without `LOCK`, the locking falls on the directory | `lock_works_in_a_directory_without_write_permission` in [`std_fs.rs`](../../crates/kerno-core/src/vfs/std_fs.rs) |
| **G3** | Minor (test gap) | The real `kill -9` had only one sequential writer | Two new tests with four writer threads (`process_kill.rs`), in `Sync` with `wal_preallocate` and in `Buffered`. They **do not cover** `begin_write` or `compact_range` | The two new tests (their names do not appear in the document) |
| **D1** | Minor (documentation) | Inaccurate provenance of the record of 1 000 seeds and a broken table | Corrected | Not applicable |
| **G2** | Minor (test gap) | The closing with parallel subcompactions (default value) has no crash sweep | **Partial:** the sentence of the matrix was rewritten; the sweep was not added. **Open** | Not applicable |

### 4.3 Stage 3 (E3-10, 2026-11-04)

| # | Defect or gap | How it was found | Correction | What keeps it |
|---|---|---|---|---|
| D-1 | IU-2 incomplete in the reading of the head. A head `Archived` over a `Create` revision was accepted. `get_unit` returned an unconfirmed life cycle | Adversarial review of E3-10 | `load_unit` compares `lifecycle_after(kind)`, the rule that `verify` uses | `unit_db::corruption_detected`, "lifecycle" and "metadata" cases |
| H-1 (gaps) | IU-1 was only checked in a fixed scenario | Mutation from the review | `policy_model` calls `verify_immutable` in each round | `policy_model` |
| H-3 (gaps) | No control that content does not enter an API error or a `verify` finding | Mutation from the review | New test | `verify::sentinel_no_content` |
| PR #13 | An empty `ChangeSet` reported durability without covering it. `history_page` truncated offsets to `u32`. A retention test did not wait for its synchronisation | Review of the PR | Corrected in three points | New retention test (not named) |

### 4.4 Stage 4 (E4-05, 2026-11-05)

| # | Gap | Correction | What keeps it |
|---|---|---|---|
| H-1 | The independent decoder had no vectors of `REL`, `REL_OUT` or `REL_IN` | 10 positive and 25 negative vectors were added (`struct_vectors.rs`) | The `rel_*` vectors |
| H-2 | `units-v1.md` did not say the order of checking of provenance | The document was corrected | Not applicable |
| H-3 | No test damaged provenance already stored | New test | `relations::stored_provenance_corruption_is_detected` |
| H-4 | The differential index workload had no `restore` or `run_retention` | They were added | `random_step` of `tests/index.rs` |
| H-5 | Formal promotion of invariants without cited tests | `invariants.md` was updated | Not applicable |

### 4.5 Stages 6, 7 and 8

| Stage | Defect | Correction | What keeps it |
|---|---|---|---|
| 6 (E6-05, H-4) | The header comment of `cache.rs` said the cache was not connected to the planner | The comment was corrected | Not applicable (comment only) |
| 6 (E6-05, H-5) | The comment said "no entry older than `max_age`", which is not true at rest | The comment was corrected. **The code did not change** | Not applicable |
| 7 (E7-00) | The frozen set had a single-field correction before any recovery code (`6b2a3cb`). The expected answers went from 527 to 533 | It is documented in [set §9](stage7-evaluation-set.md#9-correction-before-use-single-field-leaks). No threshold changed | The `MANIFEST` of the set |
| 7 (R-11) | Mutation 1 of the measurement had no recorded message | The message was noted | Not applicable |
| 8 (F-14) | The roadmap said E8-S0 was still pending, although its report was committed | The roadmap was corrected | Not applicable |

### 4.6 Stage 9

| Package | Defect | How it was found | Correction | What keeps it |
|---|---|---|---|---|
| **E9A-03, H-1** (Stage 9 advance) | An SST filter block rewritten and resealed with a valid CRC passed opening. `get` answered "does not exist" for keys that the table contains, without error | Opening property test, E9A-03 | Opening and `verify()` without `deep` check that the filter contains the last key of each block. Commit `d89c728` | `open_properties::resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` (verified: exists). Residual limit in H-2 |
| **E9-02** (ADR-0005) | A first design of public identifiers had faults that the independent review found: record value without the common header, missing layer 3 mark, deletion and export | Independent review | Corrected on 2026-11-13 by delegation. See [units-v3](../file-format/units-v3.md) | `tests/public_ids.rs`, including `tampered_text_is_refused_with_its_own_error` and `tampered_but_well_formed_tokens_name_nothing` |
| **E9-03, H-6 and H-8** | The sentinel detector did not search 8-byte fragments. A comment said "10^4 commits", and the code does 4 000 or 6 000 | Advance closure | Detector with fragments; comment corrected | `redaction_units.rs` and `redaction.rs`; detector with its own test |
| **E9-22, K1 (partial)** | From the end that keeps its adjacency entry, `query_relations` returned the adjacency answer as clean when the mirror entry was missing | Reading of the K-E3-7 review and a damage test | `execute_relations` checks the mirror entry of each relation (`all_mirrored`). If it is missing, `Fallback` `Invalid(Unmirrored)` | `one_adjacency_entry_missing_{small,large}_store` in `relations.rs`, two engines |
| **E9-22, K2 (partial)** | With the `IDX_META` count intact, the `IDX_PRIMARY` path returned fewer units without error | Damage test | `scan_entries` counts what it walks and gives `Corruption` if it does not match the count | `index_path_with_a_missing_entry` in `index.rs`, two engines |
| **E9-22, K3** | Suspicion that `get_unit` did not validate `oldest_rev` | Reading of the review | **Not reproduced**: `UnitHead::decode` already requires it | `get_unit_checks_oldest_rev` in `relations.rs`, two engines (kept as a guard) |
| **H-8 to H-12** (advance and closure) | Outdated comments, documents and a line citation | Review of the advance | H-8 was closed in E9-03; H-9 and H-12 in the Stage 9 closure (§8); H-10 and H-11 were marked resolved in the advance closure | Not applicable |
| **Errata of the E9 closure (§7)** | Type of `DerivedDb::close` (returns `Result`, not `()`); line of `compact_range` that cites ADR-0009 (`db.rs:2009`; when the minutes were written it was `db.rs:2011`); citation of the ADR-0020 fixture | Review of the closure | Corrected in the closure, without editing the accepted ADRs | Not applicable |

The findings F-02, F-03 and F-04 of Stage 8 do not appear here as corrected. The Stage 9 closure only assigns them to E9-10 and E9-11 and does not say whether they were resolved. They are in section 5.4.

### 4.7 After Stage 9: P1, K1 and the independent review

| Finding | How it was found | Correction | What keeps it |
|---|---|---|---|
| **K1**, relations with a count record. A lost adjacency entry left no trace: from its end, `relations_of` returned a short answer without error, and `query_relations` did not give `Fallback` (section 5.1) | Review of K-E3-7, in Stage 9 ([stage9-k-e3-7](stage9-k-e3-7.md) §9) | Unit format v4 (`0c2c3d5`): one count record per `(unit, direction, type)`. The writer adds and subtracts it. The reader compares it with what it walks. `verify_units` checks it with UNIT-45 and UNIT-46 ([units-v4](../file-format/units-v4.md)) | `rel_counts.rs`, in both engines: 30 tests that run and 1 `#[ignore]` that regenerates the fixture. 19 mutations of P1, all detected. Limits: databases from before layer 4 and double damage (section 5.1) |
| **Defect of the independent review of P1.** A rejected relation creation left an orphaned counter. `create_relation` adjusted the two counters before `record.encode()` validated the provenance. If `encode` rejected the relation, the counter stayed in the open change set and a later `commit` wrote it. Then `relations_of` gave `Corruption` and `verify_units` UNIT-45, on a store without damage | Independent review of P1 (mutation record, "Later correction of K1") | Build and encode all the writes before touching the counters (`crates/kerno-units/src/db.rs:1473-1487`). `delete_relation` does the same with its keys (`db.rs:1503-1509`). Import was reordered in the same way (`import.rs:213`, commit `0310bb3`). Commit of the creation: `4b65c51` | `a_refused_create_leaves_no_count` in `rel_counts.rs`, in both engines. Before the correction it gave `Corruption` in `relations_of` in both engines. Mutation **M20** (the two `adjust_count` moved before the encoding): red in both engines (`exit=101`). The import case admits no test, because its change set is private and is abandoned without `commit` on an error |

The independent review of P1 is still listed as pending in [post9-pending](post9-pending.md) (point 2, breakdown). This report records the defect it found and its correction, according to [post9-mutations](post9-mutations.md).

### 4.8 After Stage 9: P2, tests, message and intermittent test

| Finding | Correction | What keeps it |
|---|---|---|
| **Missing tests** (Stage 9 closure, points 4, 5, 6 and 10). Missing were: the "registry lock, then commit" order of the derived data (F-09); the test that errors and observations carry no content; the test that `compact()` refuses a poisoned handle and does not widen its range (E9-08g); and the mutation record of Stage 8 (E9-15) | New tests in `derived_lock_order.rs` (two engines, with the hook `hook_registry_locked`, only with `test-support`), in `reclaim.rs` and `db.rs` for `compact()`, and coverage assertions in `redaction_units.rs` | Records Q1 to Q13 of [post9-mutations](post9-mutations.md) (P2 record). Limits: it is not tested that the lock stays taken during marking; fragments under 8 bytes are still not searched for (Stage 9 closure, point 5) |
| **Message of `NoValidSources`** (point 3). The text did not say which datum was missing | It now says: "no source of derived datum <id> is valid any more" (`crates/kerno-units/src/derived_stale.rs:124`). The two tests that fixed the text changed (`consolidation.rs`, `derived_stale.rs`) | Mutation Q7. Commit `2356ff3` |
| **Intermittent test** `m0_profile::shared_range::space_is_reclaimed_after_range_deletes` (point 11). Its cause was a timing assumption of the test itself, not a defect of the engine. It measured the size of the directory just after the last write of each refill, without waiting for the background compaction to catch up with the writer. With the machine loaded, the single compaction thread could be one cycle behind. The four failures were all of the size assertion after refilling, never of the keys assertion | Function `settled_size` (`crates/kerno-testkit/src/harness/mod.rs:72`): if the size exceeds the bound, it measures again every 5 ms, up to 30 s. The bound, the number of cycles and the checks do not change. Under load, 4 failures in 600 runs before the change and 0 after. Alone, 0 in 200 before and after. The sibling test `functional::space_is_reclaimed` had the same assumption: 1 failure in 300 before and 0 after, and it was corrected in the same place | Mutations Q14 to Q16. Limits: the first fill does not wait; an engine that does not reclaim fails 30 s later; 600 runs without failure do not prove that the test cannot fail |

### 4.9 After Stage 9: P3, ADR-0016 and the review of D16

| Gap | State before | Correction | What keeps it |
|---|---|---|---|
| **Open work 1.** The explanation of the plan did not say whether the cache was consulted, nor whether it answered with learned state | `explain_*` did not say it | `Plan::cache()` (`crates/kerno-units/src/plan.rs:328`) returns a `CacheView`: `Off`, `Miss`, `Invalidated`, `Discrepancy`, `Hit { evidence, cost_ns }` or `NotApplicable`. It is computed by `ResultCache::peek`, which leaves no record, counts nothing and removes nothing. `Plan::answering_path()` (`plan.rs:335`) says whether the learned state answers | `the_plan_says_whether_the_cache_was_consulted_and_whether_learned_state_answers` and `the_plan_does_not_announce_an_entry_that_fails_its_check` (`cache_planner.rs`, two engines). Mutations M9, M13 to M16 |
| **Open work 2 (H-3 of Stage 6).** The creation evidence was constant: `Evidence { observations: 1, window: Duration::ZERO }` | Fixed evidence | The evidence is the observer's (`Observer::evidence_of`, `crates/kerno-units/src/observation.rs:424`). `CacheEntry::build` rejects an entry without observations (`NoEvidence`) or without cost (`NoCost`) (`cache.rs:351-354`). **Behaviour change:** the observer is switched off by default. Below `Fingerprint` it keeps no per-execution records, so with the cache switched on the planner creates no entries and no read is served from the cache. To use it, the observer must be raised to `Fingerprint` or more | `relation_evidence_comes_from_the_observer` and `observations_never_act_as_authority` (`cache_planner.rs`). Mutations M1 to M6 and M17 |
| **Open work 2, cost.** The entries did not carry a cost | No cost | `CacheEntry::cost_ns` (`cache.rs:376`): nanoseconds of the execution that the entry replaces. `Transition::Created { evidence, cost_ns }` and `CacheView::Hit { cost_ns }`. Limit: a single measurement per entry, not a cost model | `the_created_record_carries_the_evidence_and_the_measured_cost` (`cache.rs`). Mutations M6 and M8 |
| **Open work 3.** `set_key` deleted entries, record and counters without leaving a trace | No record | `Transition::KeyReplaced { entries }` in the new record (`cache.rs:559`). `clear` still leaves `Cleared` | `replacing_the_key_leaves_a_record_of_the_wipe` (`cache_planner.rs`, two engines) and `set_key_leaves_one_record_of_the_wipe_in_the_new_log` (`cache.rs`). Mutations M11 and M12 |
| **Review of D16, point 1 (G1 and G5), before P3.** `ResultCache::create` and `activate` were public, and so were `CacheEntry::new` and `Dependencies::current`: a caller could plant rows that were then served | The planner uses `pub(crate)` versions (`store`, `arm`, `build`, `in_force`). `create` and `activate` have `cfg(any(test, feature = "test-support"))` (`cache.rs:907-914`), and ADR-0016 says the same of the other two | Four `compile_fail` doctests, one per function, according to ADR-0016, point 1 | The description is in [ADR-0016, "Review of 2026-11-13", point 1](../adr/0016-operational-adaptation-and-invalidation.md) |

**Outside the packages.** In Stage 4, `kerno::retention_every_point_paced_power_loss_sync` failed once in six runs (M-11). It was not corrected. It was not reproduced in E5-05 ([Stage 5 closure §5](stage5-closure.md)).

---

## 5. Known and open errors

This section collects each open defect or limit that the documents record. For each one it says the impact, how to reproduce or observe it when the document says so, and what solutions the documents investigated, with their trade-offs. Where a document recommends a solution, it is marked. **Where there is no recommendation, it says so.**

### 5.1 Relations and indexes

| Id | What happens | Impact | How to reproduce or observe | Solutions investigated (trade-offs) | Recommended |
|---|---|---|---|---|---|
| **K1**, remaining limit (closed for relations with a count record in P1; the defect is taken as corrected in section 4.7) | A relation of a database **from before layer 4** (without count) has a lost adjacency entry: from that end `relations_of` returns a short answer, without `Fallback`. Only `verify_units` (UNIT-36) sees it | Incomplete answer without error in old databases | Test `one_adjacency_entry_missing_{small,large}_store` in `relations.rs`: deletes `REL_IN` or `REL_OUT` by hand and compares. Old database: fixture `db-legacy-relations` ([units-v4 §8](../file-format/units-v4.md)) | (a) Walk all the relations in a migration: costly, and opening does not write (I-18). (b) A count record that is maintained on write, which is what P1 does: no migration; old databases have no count until each `(unit, direction, type)` is touched. (c) Walk `REL` on each query: more than one read per relation returned | **No migration recommended.** P1 stands as a declared limit ([units-v4 §7](../file-format/units-v4.md)) |
| **K1**, double damage (limit of P1) | A lost entry **together with** its count record leaves a read that is consistent with itself | As K1, without detection at query time | Described in [units-v4 §7](../file-format/units-v4.md). `verify_units` reports it via UNIT-36 | None investigated | Not applicable |
| **K2** (remaining) | With the `IDX_META` count lowered to match the lost entry, the index path answers with fewer without error. The `IDX_TYPE` path has no count per type, so a lost `IDX_TYPE` entry gives a short answer | Incomplete answer without error | Test `index_path_with_a_missing_entry` in `index.rs`. What it does not see: a lowered count | Not investigated in the documents. P1 applies the same count-record principle to relations | No recommendation |
| **K4** | `change_set_info` does not cross the members of a `ChangeSet` with their `REV_META` | Minor: a damaged history may not be seen in that read | Inferential: read from the code. Test: UNIT-33 in `verify_units` | Check each member: read the `REV_META` of each | No recommendation. The Stage 9 closure leaves it open and unchanged |
| **K5** | `get_relation` does not check `created_seq` or the ends | Minor | Inferential. UNIT-36 reports it | Read the two headers of the ends and the two entries (two more reads per query) | No recommendation |
| **H-1 of Stage 7** (R-2) | With 64 relations or more and a relation without an entry at either of its ends, the index answer differs from the reference, without fallback. P1 covers it only if the relation has a count | Incomplete answer without error for databases without a count | `tests/assoc_indexed.rs` (test that fixes the limit, formerly `completeness_is_not_a_query_time_check`) | Count per `(unit, direction, type)`: P1 implements it for relations with a count. Check at query time: costly | Count: implemented in P1 for relations with a count record. Databases without a count remain open |
| **H-1 of Stage 5** (completeness) | `check_units` and `check_relations` check that the answer of an index path is a correct subset, not that it is complete | An incomplete index answer is not seen at query time | Reading of `planner.rs`. Seen by the differential test and by `verify_units` (UNIT-37) | None investigated | Accepted as a limitation |
| **H-2 of Stage 5** | The fallback with index in `Building` is not exercised through `query`, because a snapshot does not see that state | Partial coverage of a transient state | `candidates_carry_the_usability_the_state_gives` covers the plan, not the query | Not investigated | Accepted as a caveat |
| **M-10 of Stage 4** | A failing concurrent rebuild can write `Failed` over a `Ready` that another has just written | Conservative: the index is declared unusable when it was usable | Inferential | None. A rebuild repairs it | Accepted |
| **M-7 of Stage 4** | The type-only query has p95 of 17 ms with 100 000 units, against a criterion of 10 ms | W1 performance outside the criterion | Measured in [stage4-workloads](stage4-workloads.md) | The ideal bound (opening the 999 results) already costs 12 ms: no index gets below that | Accepted |

### 5.2 Performance

| Id | What happens | Impact | How to observe | Solutions investigated (trade-offs) | Recommended |
|---|---|---|---|---|---|
| **K-E3-8 (U2)** | `history` reaches 1.50 times the reference in a pass of three | U2 target not met | [stage3-report §8–§9](../testing/stage3-report.md) | (a) Judge U2 over `history_page`, which gives 0.58 to 0.90 times: changes the definition of the criterion. (b) Make the materialisation of `Revision` cheaper | No explicit recommendation. The owner accepted it as a risk (2026-11-04) |
| **K-E3-9 (U6)** | With concurrent retention, p99 of write 1.05 to 1.18 times and throughput 0.89 to 0.94 | U6 target not met. Cause: rotations, flushes and background compactions | [stage3-report](../testing/stage3-report.md) and [u6 remeasurement](stage5-u6-remeasurement.md) | Limit the rate of that background work in the engine | **Yes:** the Stage 3 closure recommends it |
| **T1 (Phase 1)** | p99 of write of `agent_memory`: 2.78 times the LSM reference, against a maximum of 2.0. The improvement against M0 is real | Target not met | [f1-report §5.2](../testing/f1-report.md) | Contingency review of ADR-0001, which is the owner's decision | No recommendation in the document. The decision is open |
| **T2 and T3 (Phase 1)** | p99 of commit of Cauce 1.95 times the reference (target 1.2); p99 of LiveSpec 1.79 times the B+ tree (target 1.5) | Targets not met | [f1-report §5.3](../testing/f1-report.md) | The review of condition 5 | No recommendation in the document |
| **T4 (Phase 1)** | Close after `bulk_load` `medium`: 251 ms against 250 ms. Range from 235 to 296 ms | A 1 ms failure in the noise: it is recorded as a failure | [f1-report §5.2](../testing/f1-report.md) | None | Not applicable |
| **T5 (Phase 1)** | Below 0.9 times the LSM reference in `bulk_load` (0.85), `point_reads` (0.80), `range_scans` (0.71) and `cauce` (0.87) | Target not met in five loads | [f1-report §5.3](../testing/f1-report.md) | None | Not applicable |
| **T7 (Phase 1)** | Write amplification of `small_commits`: 4.29, with range from 1.17 to 5.01 | Target not met, and unstable | [f1-report §5.3](../testing/f1-report.md) | The document says it depends on whether the compaction work falls inside the measured window. No solution | No recommendation |
| **Planner (R-E5-1)** | p99 cost +8.5 % at 100 000 units; +14.1 % at 20 000 in one run (the second gives +0.3 %) | Performance risk, not blocking | [Stage 5 closure §5](stage5-closure.md) | Repeat on a machine without load | **Yes:** the closure proposes it, and it has not been done |
| **`diagnostic` observation** | +5.9 % at 20 000 units in one run; minutes' limit, 5 % | Performance risk, not blocking | [Stage 5 closure §5](stage5-closure.md) | Repeat on a machine without load | **Yes**, the same |
| **Cache, p99 of W1** (E6) | p99 of W1 between +0.5 and +7.8 % in four runs. The first read after a write cannot be served from the cache. Net improvement **not claimed** | The cache is switched off by default | [Stage 6 closure §4](stage6-closure.md) | (a) Repeat with more rounds. (b) Avoid copying the result when storing it: changes the public type `CachedResult` and `QueryOutcome` | No recommendation. The owner chose to keep it switched off |

### 5.3 Retrieval quality and ranking

| Id | What happens | Impact | How to observe | Solutions investigated (trade-offs) | Recommended |
|---|---|---|---|---|---|
| **Q4 (Stage 8)** | Only one of the four query classes has query vectors in the E8-07 set (25 of 25). The other three have 0 | The criterion of quality with and without signal could not be computed in three classes | [Stage 8 closure §3](stage8-closure.md) | A second evaluation set (E8-10), with vectors in each class, frozen before measuring. It was measured once in E9-19 and E9-20, under load. S1 and S2 replace Q4 ([minutes §5](stage9-record.md)) | **Done with limits:** S1 and S2 replace Q4 ([requirements-1.0](requirements-1.0.md), point 3). The quality thresholds are printed and not asserted |
| **Ceiling effect (Q2, Q3, Q4)** | 1.000 in both arms on the 25 queries with vector | The 25 queries do not distinguish the arms | [Stage 8 closure §3](stage8-closure.md) | The same set of E8-10 | As Q4 |
| **T4 (Stage 7)** | Requires 1.036 with a simple order of 0.942. An nDCG cannot exceed 1 | An unreachable criterion as written | [Stage 7 closure §3.2 R-7](stage7-closure.md) | (a) Rewrite T4 before measuring again. (b) Leave it. The owner decided not to rewrite it now | Owner's decision: do not rewrite (2026-11-12) |
| **Ranking (R-5, R-6)** | Unnamed units score 0, which penalises the ranking. The per-class result changes with the tie-break of the simple order | Class conclusions are fragile | [Stage 7 closure §3.4](stage7-closure.md), exploratory controls | Measure with another set | Not applicable |
| **R-8 and R-9 (Stage 7)** | Configuration measured without type signal or vectors. The weights were justified before measuring, not measured separately | The ranking remains unevaluated in its full configuration | [Stage 7 closure §3.2](stage7-closure.md) | Another measurement with another set | Not applicable |
| **R-10 (Stage 7)** | The baseline B1 does not receive the query's constraints. The comparison with B1 favours the associative queries by design | Comparison with B1 is biased | [Stage 7 closure §3.2](stage7-closure.md) | None investigated | Not applicable |
| **R-3 (Stage 7)** | The check of I-41 in the measurement is not independent of `Contributions::sum`. Only the `assoc_rank` oracle sees it | The measurement is not independent evidence of I-41 | [Stage 7 closure §3.3](stage7-closure.md), mutation M-R4 | None investigated | Not applicable |
| **R-4 (Stage 7)** | The counter of C1 is tautological: it adds the same number twice | The figure 757 of 757 is not an independent count | [Stage 7 closure §3.2](stage7-closure.md) | None investigated | Not applicable |
| **Ranking on the default path** | The ranking is not the default order. It is switched off (`RankMode::Off`) | The order the query serves is "hops, then identifier" | [Stage 7 closure §8](stage7-closure.md) | Owner's decision, 2026-11-12 | Not applicable |
| **F-08 (Stage 8)** | P2 does not measure the cost of reading persisted vectors: it compares two searches over the same already-loaded set | A real cost not measured | [Stage 8 closure §3](stage8-closure.md) | A different bench design. A single read of 20 000 vectors took 9.8 to 11 ms and is reported separately | No recommendation |

### 5.4 Derived data, summaries and suppression

| Id | What happens | Impact | How to observe | Solutions investigated (trade-offs) | Recommended |
|---|---|---|---|---|---|
| **F-02** | After rebuilding a summary, building again over the same sources returns `Duplicate` with an unclear message | Ergonomics | P3 probe in [section 2.3 of the Stage 8 closure](stage8-closure.md) | Whoever wants the summary must keep the new id, which the code already allows. The Stage 8 closure records it as ergonomics within group D of the friction list | No recommendation |
| **F-03** | `read_input` returns `None` if the head has no payload, and `valid_inputs` skips the source without error | A missing source is silently skipped in `rebuild` and in `supersede`. Only a deleted unit produces it, and the path is not reached | Reading of `derived_stale.rs`: `read_input` and `valid_inputs` (lines 385 and 406 in the current tree; the E8 closure cites them at 354 to 372). Assigned to E9-11; the Stage 9 closure does not say whether it was corrected | None investigated | Not applicable |
| **F-04** | The identity of a derived item includes the profile, but no test changes only the profile and checks that the identity changes | The property holds in the code and has no test that shows it | Search for `profile` in `tests/` | Add the test | No recommendation |
| **F-05** | Summaries live only in memory. There is no sentinel measurement over them | No bytes on disk to measure | Header of [`derived_suppression.rs`](../../crates/kerno-units/tests/derived_suppression.rs) | None: summaries have no on-disk format (D8) | Not applicable |
| **F-09** | The "registry lock, commit, mark" order of the saved commit has no test | A late lock after the commit stays green in the stress test (Stage 9 closure, point 4) | The note in the code of `derived_stale.rs` itself (the commit is at line 549 of the current tree; the E8 closure cites lines 495 to 502) | Showing it requires a reader that competes with the commit, and it cannot be made deterministic | No recommendation |
| **F-10** | The recorded mutation of each new test of E8-06 and E8-07 is missing | Process gap | Stage 8 closure, point 4 of the minutes | Closed in E9-15, with two mutations that make only one test fail | Covered, with caveats |
| **F-11** | The T5 check of ADR-0020 (building the binary of `d01f646` and opening the fixture) was not run | The ADR-0020 fixture has no executed check | Stage 8 closure §2.4 | A reference binary that was not built | No recommendation |
| **F-12** | There is no record of the review rounds of E8-03 and E8-04. They are rebuilt from the commit messages | The count of rounds is not verified: the closure cites five, and the history shows six commits with visible review | [Stage 8 closure §7.1](stage8-closure.md) | None | Not applicable |
| **F-13** | Fifteen friction points in the public API; one touches a promise (`forget` is only logical) | Blocks 1.0 as prior work | [FRICTION](../../examples/agent-memory/FRICTION.md). The test `forgotten_text_stays_in_the_files_after_retention` finds the text in `000003.sst` | Proposed order: first group F (the promise), then B and A, C and D after, E at the end | Order of the Stage 8 closure §6 |
| **Unreachable compaction** | `UnitDb::compact` needs `TestHooks`, and `KernoDb::compact_range` is a test method: the public wrapper does not expose them | Physical deletion is not reachable from the public API | [Stage 8 closure §5, point 9](stage8-closure.md), group F | Expose compaction in the public API | Not decided |
| **Limitation of E9-21, `set_policy`** | `set_policy` does not accept `KeepLast(1)`. The example prunes explicitly | The example does not use the policy its minutes ask for | [Stage 9 closure, point 8](stage9-closure.md) | None investigated | Not applicable |
| **E9-12 and F-09, partial** | P2 tests the order "registry lock, commit" (`derived_lock_order.rs`, two engines). It does not test that the lock stays taken during marking: no external observation sees it without a competing reader | As F-09 | [Stage 9 closure, point 4](stage9-closure.md), [post9-pending](post9-pending.md) (point 4) | None | Not applicable |
| **Deterministic vectors** | The vectors are deterministic with a seed. They say nothing about the quality of a real model. No model was decided (D5 (b) of Stage 8) | Semantic quality has not been measured | [Stage 8 closure §5, point 4](stage8-closure.md) | A set with vectors from a real model. None exists | No recommendation |

### 5.5 Cache, planner and observability

| Id | What happens | Impact | How to observe | Solutions investigated (trade-offs) | Recommended |
|---|---|---|---|---|---|
| **H-5 of Stage 6** | An entry older than `max_age` is never served, but stays in memory while the cache does not create a new entry | The age bound is not met at rest. Memory is bounded by size | `grep -n prune_entries crates/kerno-units/src/cache.rs` | Call `prune_entries` also in `stats()` and `records()`: one line per function, with a test | No recommendation |
| **H-6 of Stage 6** | The causes of the cache (`precondition_failed`, `invalidated`, `disabled`, `discrepancy`) do not reach `ExecutionObservation` | A cache fallback does not appear in the observation | `observation.rs` and `planner.rs` | Carry the cause to `ExecutionObservation` (a change of `FallbackCause`), or leave it in the transition record | Accepted by delegation as a recorded risk ([Stage 6 closure §7](stage6-closure.md)) |
| **H-7 of Stage 6** | The mutation "with the cache switched off, it still takes the cache's path" survives the whole cache suite | I-36 is verified in results and counters, not in cost | Mutation of `if !self.cache.enabled()` (the Stage 6 closure cites it at `planner.rs:354`; in the current tree it is at lines 470 and 636 of `planner.rs`; section 3.3 of the Stage 6 closure) | An injection point to observe the index read | Accepted by delegation as a risk |
| **H-1 of Stage 6** | `check_units` and `check_relations` check subset, not completeness | As H-1 of Stage 5 | Reading | Differential test | Accepted as a limitation |
| **H-4 of Stage 5** | Age retention checks only the front of the queue. With a clock that goes back, an old record can stay behind | An old record may be retained for too long | `observation.rs`, `ponytail:` note | Walk the queue on read | Accepted, with its path documented |
| **Loss of observations (H-3 of Stage 5)** | A record is lost and counted when the lock is busy | Incomplete observation by design | [observation §3](stage5-observation.md) | Do not wait | Accepted design |

### 5.6 Storage, durability and tests

| Id | What happens | Impact | How to observe | Solutions investigated (trade-offs) | Recommended |
|---|---|---|---|---|---|
| **LazyFS not run** | The scenario A on a device that loses pages has no recorded run | The behaviour on page loss in a real device is not demonstrated | `run-lazyfs.sh` returns 2 and does not go green without FUSE. Manual and weekly workflow | Run `lazyfs.yml` in the CI, or locally with FUSE | **Yes:** it is the WP14 pending item |
| **Limit of `LOCK` (M2)** | Without `LOCK`, the locking falls on the directory and does not exclude a writer that creates `LOCK` later | Two writers could coexist if one creates `LOCK` late | `ponytail:` mark in the code of `std_fs.rs` | None investigated | Accepted |
| **G2 (partial)** | The closing with parallel subcompactions (default value) has no crash sweep | A crash during a closing with subcompactions is not tested | Failure matrix, row SC | Add a sweep with `Background::Threads` | Open, not blocking |
| **G3 (partial)** | The `kill -9` does not cover `begin_write` or `compact_range` | Two paths without real process death | [f1-adversarial-review §7](../testing/f1-adversarial-review.md) | Add the tests | Open |
| **Unverified M0 performance findings** | Contention of global counters in concurrent reads (LSM); `fdatasync` of a WAL that grows without preallocation (LSM); copies of pages in very large batches (B+ tree). They were not verified for lack of agent quota | Unmeasured performance risk | [m0-report §2.2](../testing/m0-report.md) | Measure | No recommendation |
| **C-4 of ADR-0009 (H-4 of the advance)** | The count clause of C-4 does not hold: `pinned_files = 3` while only `000007.sst` contains the value and the key | The ADR's clause does not match the measurement | Output of §2.1 of [stage9-early-closure](stage9-early-closure.md) | None. The ADR is accepted and is not edited. D17 left it recorded | Contradiction recorded (D17) |
| **H-5 of the Stage 9 advance (not measured)** | Blocks freed from the medium (C-5), memory, caches, point tombstones, deletion by background thread and long-lived handle are not measured | Deletion and memory are not verified | [suppression_survival.rs](../../crates/kerno-storage/tests/suppression_survival.rs), header "Not exercised" | Measure each one | Not applicable |
| **Intermittent test of Stage 4 (M-11)** | `retention_every_point_paced_power_loss_sync` failed 1 time in 6 runs | As the one above | [Stage 4 closure §4](stage4-closure.md) | None. The closure says there is no small, clear correction | Accepted as recorded |

### 5.7 Security and privacy

| Id | What happens | Impact | How to observe | Solutions investigated (trade-offs) | Recommended |
|---|---|---|---|---|---|
| **T-03** | Read access to the disk or to backups: no encryption at rest | A stolen disk gives the data | [threat model T-03](../security/threat-model.md) | (a) Per-block AEAD (ADR-0006, option 1), with a `KeyProvider` outside the directory: changes the format and uses the reserved 40-byte slot. (b) Encryption at file-system or device level, delegated to the operating system (ADR-0006, option 2): no format change, but no protection of exported copies and no key control by Kerno. (c) Per-record or per-field encryption (ADR-0006, option 3): allows selective cryptographic deletion, with more metadata | Option (a) as a **design**, not implemented (D2). No encryption in 1.0 (D3) |
| **T-04 and H-2** | An attacker who rewrites a block and recalculates the CRC: the engine does not detect it on read | Silent manipulation | [threat model §6.6](../security/threat-model.md). `resealed_zeroed_filter_is_refused_at_open_and_reported_by_verify` covers the coarse case | (a) Do not add authenticated integrity; `verify()` with `deep` as the resource. (b) Keyed authentication code in the 40-byte slot, without confidentiality, with a new bit. (c) Full AEAD. Options (b) and (c) change the format and require keeping a key (D4) | Option (a) of D4, taken: no authenticated integrity in 1.0 |
| **T-09** | Involuntary retention: deletion is logical, and the bytes stay in live files until compaction | The suppressed data may remain on disk | [ADR-0009](../adr/0009-suppression-retention-and-backups.md), [threat model §6.5](../security/threat-model.md) | Option 3 of ADR-0009: no guaranteed physical deletion | Accepted (ADR-0009, 2026-11-12) |
| **T-10** | The CI runs neither fuzzing nor dependency auditing (`cargo audit`/`cargo deny`) | Supply chain without automatic audit | [threat model §3](../security/threat-model.md) | Add to the CI. No document decides it | Not decided |
| **Backups** | There is no backup function in the engine, and no backup window is declared | No declared loss window | [ADR-0009](../adr/0009-suppression-retention-and-backups.md), D11 and B-1 | Backup function (D18), outside 1.0 | Not decided |

### 5.8 Documentation and coherence

| What | Where | Status |
|---|---|---|
| The specification's table says that Stage 2 is "In progress", while the roadmap marks it as closed | [docs/spec/README.md](../spec/README.md) and [roadmap](../roadmap/README.md) | Not corrected. The roadmap fixes the date 2026-11-11 |
| The Phase 1 report (2026-10-29) says that ADR-0013 is still "Proposed" and that the freezing is pending | [f1-report §1 and §7](../testing/f1-report.md) | Superseded by ADR-0013 (accepted on 2026-10-31). The report is not rewritten |
| The Stage 7 closure cites the old name of a test | [Stage 7 closure §3.1](stage7-closure.md) | Renamed in P1 |
| The `test-plan` has an unfilled placeholder (`TIEMPO_SUITE`) where the suite's duration should go | [test-plan §7.5](../testing/test-plan.md) | Not corrected |
| `T-CNT` was not rerun in the Stage 9 closure | [Stage 9 closure §3](stage9-closure.md) | No global count after Stage 8 |
| ADR-0017 remains "Proposed", although its text says it is decided at the start of Stage 3 | [ADR-0017](../adr/0017-semantics-of-the-neural-unit.md) | Not decided (see section 8) |
| The ADR index marks ADR-0007 as "Pending"; its body says "Accepted" and has no date | [ADR index](../adr/README.md), [ADR-0007](../adr/0007-index-consistency-and-rebuilding.md) | Contradiction not corrected |
| Some `main` commits carry authorship trailers that the project's rules prohibit | [Stage 9 closure §5, point 12](stage9-closure.md) | Known and accepted stain. The history is not rewritten |

### 5.9 Document inconsistencies that P4 found and nobody corrected

P4 found them when auditing the 1.0 requirements ([requirements-1.0](requirements-1.0.md), rows 2 and 4). This report checked them against the files cited on 2026-11-13. None is corrected: the texts affected belong to a closed minutes document and an accepted ADR.

| Id | What the document says | What the files say | Where the text is | Why it is not corrected alone |
|---|---|---|---|---|
| **(a)** Row 15 of the minutes, §2.4 | That `set_policy` sets `KeepLast(1)` per type: "Resolved by the existing API: the application sets `KeepLast(1)` on its type". It cites `UnitDb::set_policy` at `db.rs:619` | `set_policy` is at `crates/kerno-units/src/db.rs:625` and receives a `HistoryPolicy`. `HistoryPolicy` only has `None`, `Audit` and `Versioned` (`crates/kerno-units/src/types.rs:144`). `KeepLast` is a variant of `RetentionRule` (`crates/kerno-units/src/retention.rs:67-72`), which is received by `run_retention`. The example prunes with `prune_history` (`examples/agent-memory/src/lib.rs:576`) | [stage9-record.md](stage9-record.md), line 119 | The minutes are closed. The Stage 9 closure (§5, point 8) already notes the difference, and the minutes are not corrected |
| **(b)** Criterion T5 of ADR-0020 | That a build without the feature rejects, on opening, a database with derived data, and that "the file tree (names and hash of each one) is identical before and after" | A binary of `d01f646` returns `IncompatibleLayer { found: 2, supported: 1 }`, but the tree is not identical: `LOCK` is created (0 bytes) and `MANIFEST` changes its crc, with the same size (254 bytes). `000003.sst` does stay identical. The opening of the engine creates `LOCK` (`crates/kerno-core/src/vfs/std_fs.rs:136`, `create(true)`) and adds a session record to `MANIFEST`, before the layer reads the mark ([stage9-fixtures §3](stage9-fixtures.md)) | [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md), line 301 | ADR-0020 is accepted. The index rule of ADRs (section 7) is not to edit an accepted ADR to change a decision, and the repository does not say how a criterion is corrected. There is precedent for recorded errata in another document (Stage 9 closure §7). The fixture records the difference as a caveat, but the criterion was not corrected |

---

## 6. Risks and areas without information

Everything that follows has not been tested or measured, or there is no information. Each point says what test or measurement would turn it into knowledge.

| Area | State in the repository | What would turn it into knowledge |
|---|---|---|
| **Platforms** | Only Linux with ext4, in a 4-core VM (kernel 6.18). No document tests macOS, Windows or another file system. The target machine (C-09) is not named | Run the battery of section 9 on the target machine, and on a second file system. Repeat T1 to T10 there |
| **Real device with page loss** | LazyFS pending, no recorded runs | Run `lazyfs.yml` with FUSE, or scenario A on a mount of your own |
| **Sizes** | Associative quality: 2 000 units. Associative performance: 20 000 units. Derived data: 2 000 units (quality) and 20 000 (performance). Phase 1, scale `large` (1 000 000 records): only `bulk_load` and `point_reads`; `range_scans` and `small_commits` were not run | Run `derived_bench2`, `index_bench` and `run-benchmarks.sh` with `LARGE=1` at 100 000 and 1 000 000 units |
| **Concurrency between readers** | Not measured with several readers on the cache (Stage 6 closure, §3.4 and §4.2: "no concurrency") | A bench with several simultaneous readers and writers |
| **Relations of databases before layer 4** | Without a count record, a lost adjacency entry remains invisible from its end, and there is no block migration (I-18). It is also true for double damage | Write a migration that counts the relations, or accept the limit. None is decided ([units-v4 §7](../file-format/units-v4.md)) |
| **Multi-writer** | Outside the engine's scope (K-E3-2: the write slot is single). Group commit has tests with four writers | A bench and a load test with more than one writer, if it is to be supported |
| **Long runs** | 30-minute fuzzing per target. `kill -9` of 200 iterations in the weekly CI. Longer runs of `wal`, `manifest` and `open` pending, left for the owner's machine (section 9.3). The intermittent space test had a timing cause, corrected in P2 (section 4.8) | Longer runs of `wal`, `manifest` and `open` (section 9.3). A soak of days. Repeat the suite over several weeks |
| **Real use outside the team** | No document records use outside the team. The `agent-memory` example is used only by the team, with the public API | An external user who builds something with the public API. D12 asks for it for 1.0 |
| **Encryption and keys** | No encryption, no key management, and the slot is not used. K-1 to K-4 of ADR-0009 unmeasured | Build the design of ADR-0006 and measure K-1 to K-4 |
| **Authenticated integrity** | None. H-2 stays as an accepted limit | Build an authentication code or AEAD, or accept the limit in 1.0 (D4, already taken) |
| **Physical deletion** | No guarantee. C-5 (blocks freed from the medium) is not measured | Measure C-5 on the device (ADR-0009, K-1 to K-4) |
| **Semantic quality of retrieval** | Deterministic vectors with a seed. Synthetic set by a single author, with judgements by that author. Q4 not demonstrated | The E8-10 set: query vectors in each class, independent judgements and thresholds before measuring |
| **Embedding model** | No model was decided (D5 (b) of Stage 8). There is no adapter to a real model in the core | Decide the model and measure with real vectors |
| **Memory and caches** | Not measured (H-5 of Stage 9). Summaries live in memory and there is no measurement of their use | Measure memory per load, with the cache size of the configuration |
| **Long history** | K-E3-4: long snapshots prevent reclaiming space. Not measured with years of history | A retention bench with old snapshots for hours |
| **Format compatibility between versions** | v1 format frozen; the vector tests fail if a byte changes. Unit format v4 (P1): a database with a relation created by this build does not open with layer 1 to 3 readers ([units-v4 §2](../file-format/units-v4.md)). There is no migration test between versions, because there are no migrations (D11 leaves a migration record with no writer) | Write the migration and its fixtures before 1.0 |
| **Export** | Format frozen at `format_version = 1` ([export-v1.md](../file-format/export-v1.md)), pinned by golden files. I-30 verified for it | 1.0 is still not declared (D12) |
| **Backups** | No backup function. No declared window | Decide a backup function (D18) and measure its window |
| **Cost of the cache and the planner on an idle machine** | Measured under load. Repetition pending | Repeat the E5 and E6 benches without load |
| **Stability of vectors across platforms** | Guaranteed by design, not tested on another platform (R-12 of Stage 7) | Run the vector test on another platform |
| **Results against SQLite with thresholds** | No thresholds (D6). The report is descriptive | Set thresholds before measuring, on a quiet machine |
| **Machine load in the measurements** | Almost all the measurements were made with other processes active. P1 and P2 of Stage 8 were not measured at rest, and that run is pending (section 9.4) | Repeat without load, with `uptime` recorded in each run |
| **Independent review** | Pending for E9-16 and for P1, according to [post9-pending](post9-pending.md) (point 2 and its breakdown). The P1 mutation record cites an independent review that found a defect, already corrected (section 4.7) | A review done by someone other than the author of P1 (minutes §4.10) |
| **Verification of this report** | No test was run or built to write it. The figures of P1 to P3 are those of their documents, with their date | Run the battery of section 9 on your own machine |
| **Research documents** | `docs/research/` and `experiments/` were not read for this note | A note of its own |

---

## 7. Decisions taken and revocable

The decisions in this section are those that the documents record as taken by delegation of the owner, or approved by them, with the mark of whether they are revocable. The text of each is in the document cited. For each one it says what would be needed to revoke it, according to what the document says.

**General rule for ADRs.** An accepted ADR is not edited to change the decision: another is created that replaces it and the previous one is marked ([ADR index](../adr/README.md)). To revoke an ADR, then, a new ADR is needed and, if the decision touched the format or the code, the work that applies it.

### 7.1 Decisions of the Stage 9 minutes (2026-11-13)

Taken on 2026-11-13 by express delegation of the owner, revocable by them, D1, D2, D3, D4, D12, D15 and D16. The others, by the owner's delegate, on 2026-11-12. [Minutes §8.1](stage9-record.md).

| Decision | What it decides | What would be needed to revoke it |
|---|---|---|
| **D1** | Public identifiers: opaque random token with a persisted mapping (option 1 of ADR-0005). E9-02 merged | Revocable as long as nothing irreversible is built on them (according to §8.1). Today the layer carries mark 3 and the draft export carries tokens (SAD 0.4.1, §13). Revoking it would change that format; the document does not describe the procedure |
| **D2** | Per-block AEAD as the **design** if encryption is built. Nothing is implemented | Revocable as long as nothing irreversible is built on it. Today there is no encryption code (§8.1); revoking it is a new ADR, according to the index rule |
| **D3** | No encryption in 1.0. 1.0 does not promise confidentiality at rest | Revoking it requires building the encryption before 1.0, with D2 and K-1 to K-4 measured. It delays 1.0 |
| **D4** | No authenticated integrity in 1.0. `verify()` with `deep` is the resource | Revoking it requires building authenticated integrity, with its format |
| **D5** | ADR-0012 accepted as it stands (benchmark methodology) | A new ADR that changes the methodology. The measurements already made stay with the previous methodology |
| **D6** | Report against SQLite without thresholds | Set thresholds before measuring, on a quiet machine |
| **D7** | No absolute latency limit | Set a limit before measuring. An unmet limit would open a new decision |
| **D8** | No on-disk format for summaries or for audit in 1.0 | A new format is a format change: an ADR and a layer mark |
| **D9** | Current crates, no single facade | The facade (ADR-0010 option 1) is an API change, which can be noted before 1.0 |
| **D10** | Export as an unfrozen draft | Freezing the format, which is D12 |
| **D11** | v1 fixtures; ADR-0020 fixture rebuilt or declared absent | Write a migration record |
| **D12** | 1.0 is declared only after D13 and real use outside the team. **1.0 is not declared now** | Revoking it would mean declaring 1.0 before D13 is met. What changes is the promise of format and of API |
| **D13** | The list of requirements of §8 as the 1.0 criterion | Change the list |
| **D14** | Public `Reclaim`, with the text of ADR-0009 | Change the text of the API (any text that says "secure deletion" or "physical deletion" would be a false promise) |
| **D15** | Direction: broad non-exclusive licence with the right to relicense. **The text is not in force**: there is a draft without force ([cla-draft](../legal/cla-draft.md), P4) | Changing the direction requires another legal text. No external contribution is accepted until the owner approves the text |
| **D16** | ADR-0016 accepted and confirmed with amendments after reviewing G1 to G7 | Revoking it is a new ADR. The three gaps are closed in P3 (section 4.9). With the observer below `Fingerprint`, the cache stores nothing |
| **D17** | Clause C-4 of ADR-0009 not rewritten; contradiction recorded | Rewrite the clause with a new ADR |
| **D18** | All deletion classes on option 1; no backup function | A backup function and its window (B-1) |
| **D19** | I-28 does not apply in 1.0; I-42 and I-43 promoted; I-29 for `kerno-units` | Review the promotions if the tests are not maintained |
| **D20** | Accept H-3 with its test; require evidence for F-10; review K-E3-7 | Reopen any of the three points |
| **D21** | Property tests with the library already present. No new fuzzing tool in `kerno-units` | Add a new fuzzing tool |
| **D22** | Thresholds of §5 of the minutes unchanged before freezing | Change thresholds (it should not be done after measuring) |
| **D23** | Raw write and test hooks of `UnitDb` only under `test-support` (off by default) | Exposing the raw write outside `test-support`: a change of the public API |

### 7.2 Other decisions with delegation or approval

| Decision | Who and when | What makes it revocable |
|---|---|---|
| Stage 5 closure approved | By delegation, 2026-11-09 | A change to the closure; the limitations H-1 and H-2 remain recorded |
| Stage 6 closure approved, with the cache **switched off by default** | By delegation, 2026-11-11 | Switching the cache on by default would require a decision on a criterion that the measurement does not allow to claim |
| ADR-0011 (exact vectors, hybrid retrieval), accepted | By delegation, 2026-11-11 | New ADR |
| Stage 7: ranking **switched off by default** and experimental; T4 not rewritten | The owner, 2026-11-12 | Measure the ranking with a new set and a threshold set before measuring |
| Stage 8 closed as engineering work, with Q4 not demonstrated | By delegation, 2026-11-12 | Q4 is closed with package E8-10 |
| ADR-0012 accepted (benchmark methodology) | By delegation, 2026-11-12 | New ADR |
| ADR-0018 (common registry of derived data), option 2 | The owner, 2026-11-12 | New ADR |
| ADR-0009 (suppression), option 3 | The owner, 2026-11-12 | New ADR. The ADR says that no backup window is declared |
| ADR-0020 (persistence of vectors as derived data), revocable by the owner | By delegation, 2026-11-12 | New ADR that replaces its clause. ADR-0011 already marks it as replaced in persistence |
| K-E3-7 accepted by design | The owner, 2026-11-04 | Reviewed in Stage 9, with K1 to K5 (see section 5) |
| Integrate Stage 3 with U2 and U6 not met | The owner, 2026-11-04 | Make the materialisation cheaper, or judge U2 over `history_page`. Changes the criterion |
| Threshold against SQLite: report only | The owner, 2026-11-11 | Set thresholds before measuring |
| Comparison with SQLite, brought forward before Stage 8 | The owner, 2026-11-11 | As the one above |
| Real use case before 1.0 (memory for an agent) | The owner, 2026-11-11 | Done in E8-09 and E9-21; use outside the team does not exist (section 6) |
| Randomness of the public identifiers: the 128-bit token is read from `/dev/urandom` with `std`, without a new dependency. On a platform without that source, `create_public` fails with a typed error and creates nothing (`crates/kerno-units/src/ids.rs:283`) | By express delegation of the owner, 2026-11-13 (P4) | A randomness API in the standard library for the platforms the project supports, or accepting a dependency, which goes through I-31 ([ADR-0005, "Notes"](../adr/0005-public-identifiers.md)) |
| Unit format v4 (adjacency counts), design decision | Taken by delegation, 2026-11-13 (P1; [units-v4](../file-format/units-v4.md), "Status") | Changing it is a format change, with its layer 4 mark ([units-v4 §2 and §7](../file-format/units-v4.md)) |

---

## 8. What remains the owner's

These decisions have not been delegated, or the documents reserve them for the owner in person.

### 8.1 Reserved decisions

1. **Approve the text of the contributor agreement (CLA) before accepting any external contribution.** ADR-0019 requires it, and D15 sets the direction. **There is a draft, without force** ([cla-draft](../legal/cla-draft.md), P4), which needs a lawyer's review. It is not legal advice. Meanwhile, the only rule about contributions is the "Licence" section of the README. Sources: [ADR-0019](../adr/0019-license-and-contributions.md), [minutes §8.1, D15](stage9-record.md).

2. **Declare 1.0.** Only after meeting D13 and having real use outside the team (D12). The D13 list has seven points. Status of each one according to the documents:

   | Point of D13 | Status according to the documents |
   |---|---|
   | (1) Stage 9 closed, with §4 and §5 met or declared | **Met with declared limits** ([requirements-1.0](requirements-1.0.md), point 1). Closure approved on 2026-11-13. The independent review of E9-16 and of P1 remain pending, and the test count (T-CNT), which was not rerun. The measurement of E9-20 and the fuzzing of E9-14 ran with high load |
   | (2) The 15 friction points with a disposition | **Met with declared limits** (requirements-1.0, point 2). Row 15 does not match the minutes (section 5.9, a) |
   | (3) E8-10 closed with its thresholds | **Met with declared limits** (requirements-1.0, point 3). E8-10 was measured once, with load (E9-20). The quality thresholds are printed and not asserted. P1 and P2 were not measured at rest |
   | (4) v1 fixtures and the ADR-0020 one tested or declared | **Met with declared limits** (requirements-1.0, point 4). The ADR-0020 fixture is rebuilt. Its criterion T5 is not met as written (section 5.9, b) |
   | (5) I-30 verified for the v1 format and the reimport | **Done**: the format is frozen (export-v1.md). 1.0 is still not declared (D12) |
   | (6) Public reclaim with the text of ADR-0009 and its test | **Met with declared limits** (requirements-1.0, point 6). E9-07 and I-44. No physical or cryptographic deletion, and no backup window |
   | (7) Limits declared in the threat model | **Met** (requirements-1.0, point 7). T-03, T-04, T-09 and T-10 remain open, and that is declared |

   In addition, D12 asks for real use outside the team, which does not exist in the documents. The declaration of 1.0 is made by the owner.

3. **Review and approve the requirements audit for 1.0** ([requirements-1.0](requirements-1.0.md)), **the CLA draft**, which has no force, and **the decision on the randomness of the identifiers** (section 7.2). The audit does not declare 1.0.

4. **Decide on the contingency review of ADR-0001**, because of the non-compliance of T1 in Phase 1. The report leaves it as an open decision of the owner ([f1-report §8](../testing/f1-report.md)).

5. **Approve the targets T1 to T10.** They remain "pending" in their document ([f1-performance-goals](../testing/f1-performance-goals.md)). The Phase 1 report lists them as an open decision.

6. **Name the target machine (C-09)** and repeat the measurement there. The Phase 1 report lists it as pending ([§8, point 4](../testing/f1-report.md)).

7. **Decide whether the indexes are kept inside each `ChangeSet`** (ADR-0007). Today they are derived data that only `rebuild_index` writes. The Stage 4 closure says that the decision requires a mixed read and write load, which does not exist ([Stage 4 closure §6](stage4-closure.md)).

8. **Decide on H-5, H-6 and H-7 of Stage 6** and on the improvement of the p99 of W1. The Stage 6 closure accepted H-5, H-6 and H-7 as recorded risks ([Stage 6 closure §7](stage6-closure.md)). H-3 was closed in P3 (section 4.9), so there is no decision left on it.

9. **Decide on K-E3-8 (U2).** Judging U2 over `history_page` remains the owner's decision and changes the definition of decision 12 of Stage 3. Making the materialisation of `Revision` cheaper is engineering work ([Stage 3 closure §3 and §5](stage3-closure.md)).

10. **Decide on ADR-0006, ADR-0017 and ADR-0007 in their status.** ADR-0006 is pending, although the chosen design is already decided (D2). ADR-0017 is "Proposed", although its text says it is decided at the start of Stage 3, which is already closed. ADR-0007 says "Accepted" in its body, but the ADR index marks it "Pending".

11. **Repeat the Stage 8 time measurements (P1 and P2, ratios) at rest**, if you want to confirm they are reliable. The Stage 9 closure leaves it pending. It is left for the owner's machine (section 9.4).
12. **Decide how the two inconsistencies of section 5.9 are corrected.** One is in the Stage 9 minutes, closed, and the other in ADR-0020, accepted. The repository does not say how a criterion of an accepted ADR is corrected. There is precedent for recorded errata in another document (Stage 9 closure §7).
13. **Decide who does the independent review of P1 and that of E9-16.** [post9-pending](post9-pending.md) lists them as pending, and the author of P1 cannot be its reviewer (minutes §4.10).
14. **Run on your machine what was left for it:** the long run of `wal`, `manifest` and `open` (section 9.3) and the re-measurement at rest of P1 and P2 of Stage 8 (section 9.4). Neither has been run.

### 8.2 Work that the documents require before 1.0 (not a decision of the owner, but their follow-up responsibility)

- E8-10, measured once in E9-20 with query vectors in each class. Its quality thresholds are printed and not asserted ([requirements-1.0](requirements-1.0.md), point 3; [Stage 8 closure §8, (c)](stage8-closure.md)).
- The fifteen friction points of `agent-memory` have a disposition ([requirements-1.0](requirements-1.0.md), point 2). The proposed work order is in the [Stage 8 closure §6](stage8-closure.md). Row 15 does not match the minutes (section 5.9, a).
- A real use of the API outside the team (D12).

---

## 9. How to continue on your own machine

**Prior warning about the disk.** A full release build needs **several GB of free disk per copy** of the repository. No document measures how much `target/` takes: this figure is a planning warning, not a measured datum. The Stage 3 report describes the disk of the reference VM at 90 % occupied ([stage3-report](../testing/stage3-report.md)), and the E9 SQLite measurement notes 7.1 GiB free before measuring ([sqlite-report-e9 §2](../testing/sqlite-report-e9.md)). Before compiling, check the free space with `df -h`. The disk of the machine where this report was written was almost full and is shared with other processes. That is why the long fuzzing run and the re-measurement at rest were left for the owner's machine (sections 9.3 and 9.4).

**Requirements.** Linux. Rust 1.89 or later, which is the `rust-version` of the workspace. The tests were run with Rust 1.97. A C compiler, because the benchmark crates compile SQLite. Source: [README](../../README.md), section "Development". For fuzzing, a nightly toolchain and `cargo-fuzz` (see below).

**Clone and enter:**

```bash
git clone <repository-url> kerno
cd kerno
```

The URL is not in the repository's documents. It has to be provided.

### 9.1 CI checks (copied from `ci.yml` and the README)

None of these commands has been run in this task. The task forbids building. They are copied as they are from [`ci.yml`](../../.github/workflows/ci.yml) and the README.

```bash
# formatting and lints
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# dependency and file-access guards (each must exit with code 0)
test "$(cargo tree -p kerno-core -e normal --prefix none | wc -l)" -eq 1
tree=$(cargo tree -p kerno-storage -e normal --prefix none) && grep -q '^kerno-storage ' <<<"$tree" && ! grep -E '^(spike-|kerno-testkit|proptest|loom)' <<<"$tree"
if grep -rn 'std::fs' crates/kerno-storage/src; then false; else test $? -eq 1; fi

# tests: the whole workspace except kerno-storage
cargo test --workspace --exclude kerno-storage --all-features --release -- --nocapture

# tests: kerno-storage
cargo test -p kerno-storage --all-features --release -- --nocapture
```

**Durations.** The documents give these figures:

- `cargo clippy` finished in 20.43 s in E6-05 and in 30.52 s in E7-06, on the reference VM. They are times of an incremental build, not of a clean one ([Stage 6 closure §5](stage6-closure.md), [Stage 7 closure §5](stage7-closure.md)).
- The CI budget is 15 minutes for `test-spikes` and 25 for `test-kerno` ([test-plan §7.5](../testing/test-plan.md)). The document itself has no measured duration: the marker `TIEMPO_SUITE` is not filled in.
- **There is no documented duration** for the complete suite, nor for a clean build.

**Long and weekly tests (copied from the CI):**

```bash
# long seed: 1 000 seeds in the randomized suites (weekly CI)
SPIKE_SEEDS=1000 cargo test --workspace --all-features --release -- --nocapture

# kill -9: 200 iterations (weekly CI)
KERNO_KILL_ITERS=200 cargo test --release -p kerno-storage --test process_kill -- --nocapture

# loom models of the commit channel (weekly CI, no documented duration)
RUSTFLAGS="--cfg kerno_loom" cargo test -p kerno-storage --release --lib commit_queue
```

**Benchmark smoke (copied from the CI):**

```bash
cargo build --release -p spike-bench --features candidates,kerno
target/release/spike-bench suite --engines btree,lsm,kerno,redb,fjall,sqlite --workloads all --scales tiny --reps 1 --dir bench-data --out smoke.jsonl
target/release/spike-bench report --input smoke.jsonl
! grep -q '"error"' smoke.jsonl
cargo run --release -p kerno-units --features test-support --example unit_bench -- --quick
```

None of these commands has been run in this task.

### 9.2 Full M0 benchmarks

```bash
spikes/storage/run-benchmarks.sh
SCALES=small,medium,large REPS=5 spikes/storage/run-benchmarks.sh
```

Variables documented in the script itself ([`run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh)): `SCALES` (default `small,medium`), `REPS` (default 3), `ENGINES`, `COLD` (default 1, **needs root** to empty the system cache), `LARGE` and `LARGE_WORKLOADS`. The duration is not documented. M0 made 410 runs.

### 9.3 Fuzzing

Installation, according to [`fuzz/README.md`](../../crates/kerno-storage/fuzz/README.md):

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

Run from `crates/kerno-storage`. The committed corpus (`fuzz/corpus/<target>`) **is only read**. New entries go to a separate working directory, which the owner chooses:

```bash
mkdir -p <working-directory>/varint
cargo +nightly fuzz run varint <working-directory>/varint fuzz/corpus/varint -- -max_total_time=60 -timeout=1
# open needs a larger input:
cargo +nightly fuzz run open <working-directory>/open fuzz/corpus/open -- -max_total_time=60 -max_len=65536 -timeout=1
```

Duration per target: 60 seconds in the short test. The documents use `-max_total_time=1800` for the thirty-minute run, and the recorded runs lasted 1 801 seconds each.

**Left for the owner's machine, on purpose.** The documents do not give a duration for this run. What they say is that thirty minutes is little and that a longer run of those three is the natural step ([Stage 9 closure §5, point 7](stage9-closure.md)). It was not done here because the disk of the build machine was almost full and is shared. The command is that of the thirty-minute run, from `crates/kerno-storage`, like the 60-second order of [`fuzz/README.md`](../../crates/kerno-storage/fuzz/README.md) ("Run"). That README writes `corpus/<target>` in its 1 800-second order, which only resolves from `fuzz/`; here `fuzz/corpus/<target>` is used. The time is chosen by the owner. Here `1800` appears, the value of the last recorded run:

```bash
cd crates/kerno-storage
mkdir -p <working-directory>/wal
cargo +nightly fuzz run wal <working-directory>/wal fuzz/corpus/wal -- -max_total_time=1800 -max_len=65536 -timeout=1
mkdir -p <working-directory>/manifest
cargo +nightly fuzz run manifest <working-directory>/manifest fuzz/corpus/manifest -- -max_total_time=1800 -max_len=65536 -timeout=1
mkdir -p <working-directory>/open
cargo +nightly fuzz run open <working-directory>/open fuzz/corpus/open -- -max_total_time=1800 -max_len=65536 -timeout=1
```

For a longer run, replace `1800` with the number of seconds chosen. A failure leaves a file in `fuzz/artifacts/<target>/`. It is reproduced with `cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>`, according to the README.

**Reproducing an already saved failure.** See the README, section "Reproduce an artifact".

### 9.4 Re-measurement at rest, pending, left for the owner's machine

The P1 and P2 ratios of Stage 8 were measured again in E9-20, on the E8-10 set, with another process active on the machine. The Stage 9 closure asks for them to be repeated with the machine at rest. That run was not done here, for the same reason as the one in section 9.3. The commands are those of the raw output ([stage9-measurement-output.txt](stage9-measurement-output.txt)), lines 4 and 180:

```bash
uptime
# quality of the E8-10 set (test; checks the MANIFEST and the figures)
cargo test -p kerno-units --release --test derived_measure2 -- --nocapture --exact quality_over_the_second_frozen_set

# P1 and P2 timings (example; cargo test does not run it)
cargo run --release -p kerno-units --example derived_bench2 -- --runs 4 --reps 12 --dir <temporary-directory>
```

Before each run, note `uptime` (1-minute load). The report itself gives the criterion: T-SQ1 asks for a 1-minute load below 4. The Stage 9 closure asks for an idle machine, and that is the stricter criterion.

Other pending measurements:

- **Scale `large` of `range_scans` and `small_commits`** (Phase 1). The report says: "repeat with `LARGE=1` on a machine at rest" ([f1-report §8, point 6](../testing/f1-report.md)). The script `run-benchmarks.sh` accepts `LARGE_WORKLOADS`, so one way is `SCALES=medium LARGE=1 LARGE_WORKLOADS=range_scans,small_commits spikes/storage/run-benchmarks.sh`. **This combination is not in any document, and it has not been run.**
- **LazyFS**, on a machine with FUSE: `./spikes/storage/run-lazyfs.sh`. The script returns 2 if FUSE or the tools are missing. The `lazyfs.yml` workflow can be launched by hand.
- **Target machine (C-09)**: repeat T1 to T10 there.

### 9.5 Intermittent test

`space_is_reclaimed_after_range_deletes` had a timing assumption in the test itself, corrected in P2 (section 4.8). If it fails again, repeat the `kerno-storage` suite and check that `settled_size` still waits for the size to fall below the bound. The intermittence of Stage 4 (M-11) has no correction (section 5.6).

### 9.6 Rules when measuring

- Measure with the machine at rest and note `uptime` before each run.
- Do not compile in parallel with the measurement. The documents show that load changes the figures (E9-20, [measurement §1](stage9-measurement.md)).
- Save the raw output in the repository, as the documents do.

---

## 10. Document map

One line per document.

| Document | Question it answers |
|---|---|
| [README](../../README.md) | What is Kerno and what state is it in? |
| [SAD](SAD.md) | What is the reference architecture and which decisions change that document? (version 0.4.1) |
| [Roadmap](../roadmap/README.md) | Which stage is closed, what remains and which decisions are still open? |
| [Specification v0.2.0](../spec/architecture-specification-v0.2.0.md) | What does the specification ask, without editing? |
| [Specification–SAD correspondence](../spec/README.md) | Which stage of the specification is which phase of the SAD? |
| [ADR index](../adr/README.md) | Which decisions are accepted, pending or replaced? |
| [ADR-0001](../adr/0001-storage-engine.md) | Why is the engine a levelled LSM? |
| [ADR-0002](../adr/0002-transaction-model-and-durability.md) | What does "commit" mean and what does each durability mode guarantee? |
| [ADR-0003](../adr/0003-physical-format-and-versioning.md) | What versioning and integrity rules does the format have? |
| [ADR-0005](../adr/0005-public-identifiers.md) | What are the public identifiers like? (opaque token, D1; randomness limit of 2026-11-13) |
| [ADR-0006](../adr/0006-encryption-and-key-management.md) | Which encryption design was chosen and why is it not implemented? (pending) |
| [ADR-0009](../adr/0009-suppression-retention-and-backups.md) | What does "delete" mean in Kerno? (no guaranteed physical deletion) |
| [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) | What vector search is there? (exact, no approximate index) |
| [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md) | How is it measured and with which thresholds? |
| [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) | Which v1 format is frozen? |
| [ADR-0014](../adr/0014-physical-representation-of-relations.md) | How are relations stored? |
| [ADR-0015](../adr/0015-privacy-of-observability.md) | What can an observation contain? (no content outside `diagnostic`) |
| [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) | What can the engine learn and how is it invalidated? Confirmed on 2026-11-13; open work 1 to 3 is closed in P3 |
| [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) | How are derived data registered and when are they rebuilt? |
| [ADR-0019](../adr/0019-license-and-contributions.md) | Under which licence and with which contract is external code accepted? (there is a CLA draft without force) |
| [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md) | How are vectors persisted as derived data? |
| [Invariants](invariants.md) | What may no change break, and which test verifies it? (I-01 to I-45) |
| [Failure matrix](failure-matrix.md) | What remains after reopening at each point of a crash? |
| [Stage 3 closure](stage3-closure.md) | What did the units layer build and what risks did it inherit? |
| [Stage 4 closure](stage4-closure.md) | How do relations and indexes work, and what remains open? |
| [Stage 5 closure](stage5-closure.md) | How does the planner explain its plans and what does observing cost? |
| [Stage 6 closure](stage6-closure.md) | Why is the cache switched off and what is not claimed? |
| [Stage 7 closure](stage7-closure.md) | What quality and performance does associative memory have, and why is the ranking switched off? |
| [Stage 8 closure](stage8-closure.md) | Which derived data exist, what is not demonstrated (Q4) and what follows before 1.0? |
| [Stage 9 closure](stage9-closure.md) | What is closed, which decisions were taken by delegation and which limits remain? |
| [Stage 9 minutes](stage9-record.md) | Which packages and decisions did the last stage have, and what does 1.0 require (D13)? |
| [K-E3-7 review](stage9-k-e3-7.md) | Which reads check less than `verify_units`, and what are K1 to K5? |
| [Pending items after Stage 9](post9-pending.md) | Which point of the closure remains open and which one changed? |
| [Mutations after Stage 9](post9-mutations.md) | Which mutations did P1 (M1 to M20), P2 (Q1 to Q16) and P3 (M1 to M17) make, and which did they detect? |
| [1.0 requirements](requirements-1.0.md) | Which D13 requirements does the repository meet, and with which limits? (P4 audit) |
| [CLA draft](../legal/cla-draft.md) | Which contributor agreement is proposed? (draft without force, not approved) |
| [Unit format v4](../file-format/units-v4.md) | What does format v4 add (adjacency counts) and what limits does it have? (P1) |
| [Engine format v1](../file-format/v1.md) | How are the engine's bytes read? (frozen) |
| [Unit format v1](../file-format/units-v1.md) | How are units, revisions and relations encoded? |
| [Unit formats v2 and v3](../file-format/units-v2.md) | What changed in versions 2 and 3 of the layer? (v2: persisted vectors; v3: public identifiers) |
| [Export v1](../file-format/export-v1.md) | How is a database exported? (frozen) |
| [Test plan](../testing/test-plan.md) | Which tests are there, where do they run and which gates does the CI require? |
| [M0 report](../testing/m0-report.md) | What was measured when the engine was chosen? |
| [Phase 1 report](../testing/f1-report.md) | What came out of the tests and measurements of Phase 1, and what remained pending? |
| [Phase 1 adversarial review](../testing/f1-adversarial-review.md) | Which defects blocked the merge and how were they corrected? |
| [SQLite E9 report](../testing/sqlite-report-e9.md) | How does Kerno compare with SQLite, without thresholds? |
| [E8-10 measurement (E9-20)](stage9-measurement.md) | What do the P1 and P2 ratios of Stage 8 measure, and under which conditions? |
| [Threat model](../security/threat-model.md) | Which threats are considered, which remain open and what is left outside? |
| [Fuzzing](../../crates/kerno-storage/fuzz/README.md) | How is fuzzing run and what results does it have? |
| [Storage spike](../../spikes/storage/README.md) | How are the M0 benchmarks reproduced? |
| [`agent-memory` example](../../examples/agent-memory/FRICTION.md) | What friction did the team's real use find with the public API? (fifteen points) |
| [CI](../../.github/workflows/ci.yml) | What runs on each change and what runs each week? |
| [Contributing](../../CONTRIBUTING.md) | Which code and documentation rules apply to contributions? |

---

## How this note was checked

**What was done.** The documents of P1 to P4 and those cited in the header were read. The claims about the code were checked with text searches, without building, with the following commands. The outputs are those of this update:

```text
$ git rev-parse --short HEAD
758b5d4

$ git cat-file -t 0c2c3d5; git cat-file -t 2356ff3; git cat-file -t fc7e233; git cat-file -t f2aa157
commit
commit
commit
commit

$ grep -n 'HistoryPolicy {' crates/kerno-units/src/types.rs
144:    HistoryPolicy { None = 0, Audit = 1, Versioned = 2 }

$ grep -rn 'KeepLast' crates/kerno-units/src/types.rs; echo "exit=$?"
exit=1

$ grep -n 'adjust_count' crates/kerno-units/src/db.rs
1486:        self.adjust_count(from, Direction::Out, relation_type, 1)?;
1487:        self.adjust_count(to, Direction::In, relation_type, 1)?;
1508:        self.adjust_count(r.from, Direction::Out, &r.relation_type, -1)?;
1509:        self.adjust_count(r.to, Direction::In, &r.relation_type, -1)?;

$ grep -n 'create(true)' crates/kerno-core/src/vfs/std_fs.rs
136:        let f = match fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path) {

$ grep -rn 'no source of derived datum' crates/kerno-units/src
crates/kerno-units/src/derived_stale.rs:124:                write!(f, "no source of derived datum {} is valid any more", IdHex(id))

$ grep -n 'T5 |' docs/adr/0020-persistence-of-vectors-as-derivatives.md | cut -c1-120
301:| T5 | **Una build sin la característica rechaza al abrir una base con derivados.** Un binario construido desde `d0

$ for n in a_refused_create_leaves_no_count the_created_record_carries_the_evidence_and_the_measured_cost replacing_the_key_leaves_a_record_of_the_wipe space_is_reclaimed_after_range_deletes; do grep -rl "$n" crates examples; done
crates/kerno-units/tests/rel_counts.rs
crates/kerno-units/src/cache.rs
crates/kerno-units/tests/cache_planner.rs
crates/kerno-testkit/src/harness/range.rs
crates/kerno-testkit/src/harness/mod.rs

```

The test names cited in this update exist in the tree (the output above and the others checked with the same method). The report was searched for names of models, assistants and their makers, and none appears. `completeness_is_not_a_query_time_check` no longer exists: it was renamed in P1. The lines of code cited in this update were checked with text searches; those in sections 3 to 5 that do not appear in the block are taken from the source document.

**What was not done.** No test was built or run: the task forbids it, and the disk of the build machine was almost full and is shared. That is why the commands of section 9 have not been run: they are copied from the CI, from the READMEs and from the raw output of E9-20. The figures in sections 3 and 5 are those the documents give, with their date, and have not been repeated. Packages P1 to P4 are described according to their documents and according to the code they cite. The only file modified is this report. `experiments/narrative-threads/.venv` was already untracked and was not touched.
