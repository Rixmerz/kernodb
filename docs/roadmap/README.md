# Roadmap

The full plan, with objectives and deliverables per phase, is in [SAD §16](../architecture/SAD.md#16-implementation-plan). This file records **where we are**.

| Phase | Content | Status |
|---|---|---|
| 0 / M0 | Storage spike, harness, initial ADRs, invariants, failure matrix | **Closed** (2026-10-28): ADR-0001/0002/0003 accepted — see checklist |
| 1 | Persistent core | **Closed** (2026-11-11, according to the text of the section "Phase 1: work plan"; [E3-00 record](../architecture/stage3-record-e3-00.md)) |
| 2 | Transactions and documents | — |
| 3 | Versioning (Revision Engine) | **Closed** (Stage 3, 2026-11-04): open risks K-E3-8 and K-E3-9 ([closure](../architecture/stage3-closure.md)) |
| 4 | Security and life cycle | — |
| 5 | Search and AI | — |
| 6 | Hardened beta | — |

## M0 checklist (SAD §17.5)

| # | Criterion | Status | Where |
|---|---|---|---|
| 1 | ADR-0001 selects an approach or documents why none should be implemented yet | Done: **levelled LSM**, accepted on 2026-10-28 | [ADR-0001](../adr/0001-storage-engine.md) |
| 2 | Reproducible report with code, commands, configuration and results | Done | [M0 report](../testing/m0-report.md), [`run-benchmarks.sh`](../../spikes/storage/run-benchmarks.sh) |
| 3 | Matrix of failure points and expected state after recovery | Done | [failure matrix](../architecture/failure-matrix.md) |
| 4 | Exact meaning of the proposed commit and durability | Done: accepted on 2026-10-28 | [ADR-0002](../adr/0002-transaction-model-and-durability.md) |
| 5 | Comparison by complexity, recovery, compaction, ranges and compatibility with Revision Engine | Done | ADR-0001 §Evidence (3 and 4), [revision design](../architecture/revision-model-design.md) |
| 6 | Decisions that depend on the storage versus those that are decoupled | Done | ADR-0001 §Consequences |
| 7 | No mandatory functional dependency on AI, network or external provider | Done | the prototypes depend only on `crc32c`, `parking_lot`, `quick_cache` and `crossbeam-skiplist`; the reference engines are used only in the benchmark |

## Immediate tasks of SAD §20

| # | Task | Status |
|---|---|---|
| 1 | Repository, licence and contribution conventions | Done: `MIT OR Apache-2.0` ([LICENSE-MIT](../../LICENSE-MIT), [LICENSE-APACHE](../../LICENSE-APACHE)) and [`CONTRIBUTING.md`](../../CONTRIBUTING.md) |
| 2 | SAD as baseline and `docs/adr/` folder | Done |
| 3 | ADR-0001 and ADR-0002 as drafts | Done (with evidence) |
| 4 | Test harness and M0 benchmark | Done (+ CI in `.github/workflows/ci.yml`) |
| 5 | B+ tree versus LSM spike with reproducible results | Done |
| 6 | Approve the format and commit semantics before creating user databases | Commit semantics (ADR-0002) and format rules (ADR-0003) accepted; the concrete format v1 is specified and frozen in Phase 1 |
| 7 | Persistent core only after closing M0 | M0 closed; Phase 1 closed (2026-11-11) |

## Phase 1: work plan

1. Specify the format v1 of the LSM engine according to ADR-0003 (versioned headers, feature bits, space reserved for AEAD) with test vectors.
2. Create the first product crates (`kerno-core`, `kerno-storage`) from `spike-lsm`, keeping the M0 harness (now in `kerno-testkit`) as a mandatory suite.
3. ADR-0001 conditions: bounded latency queues (compaction and close with a budget), *range tombstones* and targeted compaction, MANIFEST as an edit log, group commit.
4. Read-modify-write transactions, explicit `sync()` and snapshot metrics (ADR-0002).
5. Repeat the M0 benchmarks on the target hardware.

### Status of the work packages (WP01–WP14)

Status as of 2026-10-29. WP01–WP13b are integrated and tested in `kerno-storage`, `kerno-testkit` and `kerno-core`; the status of each pending decision is in the [Phase 1 report](../testing/f1-report.md).

| Package | Status |
|---|---|
| WP01–WP13b | Integrated (format v1, LSM engine, MANIFEST as an edit log, *range tombstones*, read-only opening, group commit, read-modify-write transactions) |
| WP14 | Integrated **partially**: real `kill -9` with 200 iterations and a sweep of 1000 seeds in green; benchmarks measured on the M0 VM class with **T1 not met** and other non-compliances documented. **Pending**: LazyFS record, benchmarks on the target machine (C-09), full `large` scale. The **freezing of format v1** was carried out on 2026-10-31 with ADR-0013 accepted that day: this cell said it was still pending and that ADR-0013 was "Proposed", an outdated earlier text |

Phase 1 was **not closed** as of 2026-10-29: the starting condition of stage 3 (format v1 frozen with ADR-0013 accepted) did not hold then, and the contingency review of ADR-0001 due to T1 is an open decision of the owner. Status as of 2026-11-11: format v1 was frozen and ADR-0013 was accepted on 2026-10-31, and Phase 1 was closed ([E3-00 record](../architecture/stage3-record-e3-00.md)).

### Closing benchmarks of Phase 1 and SQLite as baseline

The closing task of Phase 1 (adversarial review, benchmarks, documentation) measures `kerno` with `ENGINES=btree,lsm,kerno,redb,fjall,sqlite` ([T1–T10 objectives](../testing/f1-performance-goals.md)). At the request of the [specification v0.2.0](../spec/README.md) §15, the report will also present **SQLite as an explicit baseline**: a `kerno` / SQLite table per workload, with declared equivalent durability (`Sync` versus `journal_mode=WAL` + `synchronous=FULL`; `Buffered` versus `synchronous=NORMAL`, whose guarantee on power loss is not identical and is noted). This does **not** add new thresholds to T1–T10 nor replace the M0 results (B+ tree, LSM, redb, fjall and SQLite of 2026-10-27), which are kept in [`spikes/storage/results/`](../../spikes/storage/results/) and in the [M0 report](../testing/m0-report.md). The thresholds against SQLite, if wanted, are set by the owner before measuring (ADR-0012).

## Stages 3–9 (specification v0.2.0)

After Phase 1 the work is organised by the stages of the [specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §16; the [mapping to the SAD phases](../spec/README.md) is in `docs/spec/`. **Stage 3 is closed** (approved by the owner on 2026-11-04). **Stage 4 is closed** (approved by the owner on 2026-11-05, [closure](../architecture/stage4-closure.md)); **Stage 5 is closed** (approved by delegation of the owner on 2026-11-09, [closure](../architecture/stage5-closure.md)); **Stage 6 is closed** (approved by delegation on 2026-11-11, [closure](../architecture/stage6-closure.md)), with the cache disabled by default; **Stage 7 is closed and approved by the owner on 2026-11-12** ([closure](../architecture/stage7-closure.md); record approved on 2026-11-11, [record](../architecture/stage7-record.md)), with **E7-00 to E7-06 done** (E7-00 to E7-05 on 2026-11-11: frozen evaluation set and ADR-0011 accepted; associative query model and reference path; execution by index; verifiable temporal context and provenance; ranking with diagnostic mode; measurement, in which the ranking is not enabled by default). A stage already implemented and tested is skipped (spec §16).

| Stage | Depends on | Starting condition | ADRs decided |
|---|---|---|---|
| 3 Units, revisions and provenance | Stage 2 (Phase 1) | Phase 1 closed: WP14 integrated, format v1 frozen with ADR-0013 accepted (freezing carried out on 2026-10-31; as of 2026-10-29 it was **pending**: see the WP14 status above) | 0004, 0008, 0010, 0017 |
| 4 Relations and indexes | 3 | Stable units API (ADR-0010 accepted) | 0007, 0014 |
| 5 Planner and observability | 4 | Filter and relation queries with a proven reference path | 0015 |
| 6 First operational plasticity | 5 | Deterministic planner with differential test in CI | 0016 |
| 7 Associative memory | 4, 5 | Relation traversals and plan explanation available | 0011 |
| 8 Consolidation and embeddings | 7, 6 | ADR-0018 accepted on 2026-11-12; suppression of derivatives defined (ADR-0009, **met on 2026-11-12**) | 0009, 0018, 0020 (persistence of vectors, accepted on 2026-11-12 by delegation of the owner) |
| 9 Hardening | all the above (the security parts may be brought forward) | See pending decision on the order | 0005, 0006, 0012 |

### Stage 3 — Units, revisions and provenance

- **Deliverables:** `Unit`, `Revision` and `ChangeSet` types (spec §6.1–6.3) over `kerno-storage`; transactional CRUD and archiving; list, compare and restore revisions; history policies `none`/`audit`/`versioned` (SAD §8); serialisation and hashes (ADR-0008); first typed public API (ADR-0010).
- **Acceptance:** the evolution of a unit is reconstructed and restored without destroying later revisions (I-22); the history does not depend on the WAL (I-20); content and revision are confirmed at the same atomic boundary (I-21), also in the failure matrix.
- **Status: closed** on 2026-11-04 ([closure of Stage 3](../architecture/stage3-closure.md)). The acceptance criteria are met; IU-1…IU-10 are promoted. Two metrics miss their thresholds and the owner accepted them as open risks, without lowering any threshold: **K-E3-8** (U2: `history` reaches 1.50× the reference in a pass of three) and **K-E3-9** (U6: with concurrent retention, write p99 1.05–1.18× and throughput 0.89–0.94×, 1.06× in one pass). **K-E3-7** (reads check less than `verify_units`) is accepted by design, with a review recommended in Stage 9.

### Stage 4 — Relations and indexes

- **Status: closed, approved by the owner on 2026-11-05** ([stage4-closure.md](../architecture/stage4-closure.md), 2026-11-05; [record](../architecture/stage4-record.md) approved on 2026-11-04). Declared relations (ADR-0014, option 2), reference path, primary index with state and watermark (ADR-0007) and `IDX_TYPE` justified by workload W1. The five criteria are met and the adversarial review left no blockers. I-21 (relations), I-23, I-24, I-32 (indexes) and I-39 (declared) are promoted and approved. K-E3-8 and K-E3-9 remain open (U6 not re-measured with indexes). ADR-0007: unit indexes remain derived (only `rebuild_index`) until a workload with interleaved writes asks to maintain them in every `ChangeSet`.
- **Deliverables:** declared typed relations (ADR-0014); primary index and basic filters; secondary indexes only with a workload that justifies them; reference path for all queries.
- **Acceptance:** index results match the reference path (I-32 applied to indexes); rebuilt indexes equivalent to new ones (I-24); freshness state exposed (I-23); a declared relation is never confused with an inferred one (I-39).

### Stage 5 — Planner and observability

- **Deliverables:** deterministic planner (spec §9: validate, generate candidates, estimate cost, choose, execute, record, validate invariants); plan explanation; `ExecutionObservation` with detail levels ([adaptive execution](../architecture/adaptive-execution.md), ADR-0015).
- **Acceptance:** the plan can be inspected; differential test planner against reference path in CI (I-32); sentinel test with no literals or content in observations (I-37).
- **Status: closed on 2026-11-06, approved by delegation of the owner on 2026-11-09** ([closure of Stage 5](../architecture/stage5-closure.md)). Planner for the existing paths with plan explanation (E5-01, E5-02), `ExecutionObservation` with levels and in-memory retention (E5-03) and sentinels (E5-04). The criteria of the minutes §4 are met; the fallback criterion with `Building` state is met only at plan level (a snapshot does not see that state). Open risks: K-E3-8, K-E3-9 (U6 still misses with indexes and relations, E5-00), and planner overhead in W1 outside the threshold in one measurement of six (R-E5-1). I-32 (planner) and I-37 are promoted.
- **Minutes:** [stage5-record.md](../architecture/stage5-record.md) (approved by the owner on 2026-11-05).

### Stage 6 — First operational plasticity

- **Deliverables:** a single reversible optimisation (reusable plan or cache invalidated by sequence), with the metadata of spec §9 and global and per-query deactivation (ADR-0016).
- **Acceptance:** net improvement measured against the disabled mode with thresholds set before measuring; equivalence of results (I-32, I-33); invalidation proven by dependency (I-35); invalidating does not alter data (I-34); deactivation verified (I-36).
- **Status: closed on 2026-11-11, approved by delegation of the owner on 2026-11-11** ([closure of Stage 6](../architecture/stage6-closure.md)). Single optimisation: result cache tied to `commit_seq` (E6-01 to E6-03), used by the planner with its registered fallback (E6-02); measurement and sentinels (E6-04, E6-04b). **The cache is disabled by default.** Criteria: equivalence, fallback, dependency invalidation, reversibility, deactivation and traceability without literals met; **net improvement not claimed**: the p50 of W1 meets (−97.8 %), the p99 of W1 is not certified (+0.5 % to +7.8 % in four runs), and the worst case without repetitions meets in the paired measurement (real cost −0.6 % to +0.5 %) but not in all runs. Thresholds unchanged. Open risks: H-3, H-5, H-6 and H-7 of the closure. I-33 to I-36 are promoted.

### Stage 7 — Associative memory

- **Deliverables:** recovery by relations, temporal context and provenance; **exact** vector search as a baseline if there is a use case (ADR-0011); ranking with a diagnostic mode that explains its signals.
- **Acceptance:** retrieval quality evaluable with a set of queries and expected answers fixed before measuring; every answer traceable to its sources; no ANN before a benchmark that justifies it.
- **Minutes:** [stage7-record.md](../architecture/stage7-record.md) (approved by delegation of the owner on 2026-11-11). Packages E7-00 to E7-06.
- **Status:** E7-00 **done** on 2026-11-11: evaluation set frozen with SHA-256 ([evaluation set](../architecture/stage7-evaluation-set.md)), metrics proven by mutation and baselines B0 and B1 measured; ADR-0011 accepted. E7-01 **done** on 2026-11-11: `AssocQuery` with hard bounds (depth 4, 1 000 results, 100 000 visits), breadth-first reference path over `reference_relations_of` with deterministic order and trace per result ([associative query](../architecture/stage7-associative-query.md)). E7-02 **done** on 2026-11-11: the traversal takes the neighbours from the adjacency index as the planner's path (`assoc_query`, `explain_assoc`), each answer checked and with fallback to the reference path recorded on failure; differential test on both engines, injection of false answers, I-37 sentinels and 29 mutations detected ([indexed execution](../architecture/stage7-indexed-execution.md)). E7-03 **done** on 2026-11-11: the `commit_seq` window and the class, actor and source filters are checked against a brute-force oracle in the reference path, the index path and the planner path, and every relation and revision citing a trace is found in the snapshot (I-40), with 16 mutations detected and no changes to production code ([context and provenance](../architecture/stage7-context-and-provenance.md)). E7-04 **done** on 2026-11-11: ranking in `assoc_rank` with deterministic integer score, diagnostic mode whose contributions add up to the score (I-41) and optional exact vector signal (ADR-0011); it stays off by default until measurement ([ranking](../architecture/stage7-ranking.md)). E7-05 **done** on 2026-11-11: the ranking does not become the default order, because with its default weights it falls below the simple order "hops, then identifier" (nDCG@10 0.912 against 0.942) and the threshold of 1.10 times that order is not reachable; the index path meets §6.3 (p50 0.15 ms against 105 ms at depth 2, in the twelve runs) ([measurement](../architecture/stage7-measurement.md)). E7-06 **done** on 2026-11-12: adversarial review without blockers, criteria of §4 of the minutes met (I-32 with the limitation H-1 of the index, not claimed for stores of 64 relations or more with an absent entry at both ends), T4 not met as written, and I-40 and I-41 promoted and approved by the owner on 2026-11-12 ([closure](../architecture/stage7-closure.md)). **Status of the stage: closed and approved by the owner on 2026-11-12.** Owner decisions (closure §8): Stage 8 starts, with the associative-results ranking disabled by default and marked as experimental; T4 is not rewritten now, and if the ranking is measured again it will be with a new set and a threshold defined before measuring.

### Stage 8 — Consolidation and embeddings

- **Deliverables:** common registry of derivatives (ADR-0018); persistence of vectors as derivatives of the units layer, with its layer-2 version bit and its acceptance criteria (ADR-0020); external adapters or jobs for embeddings and summaries, outside the core (I-31); obsolescence and rebuilding.
- **Acceptance:** each derivative is linked to its sources and is invalidated or rebuilt when they change (I-25, I-38); suppressing a source affects its derivatives according to ADR-0009; a summary never replaces the auditable original.
- **Status: closed as engineering work on 2026-11-12** ([closure of Stage 8](../architecture/stage8-closure.md)), with decisions taken by delegation of the owner on 2026-11-12 and revocable by them. **Q4 not demonstrated** (not met). Open points: Q2, Q3 and Q4 give the maximum value in both arms (ceiling effect); P2 does not measure reading persisted vectors; the summaries are in memory only, with no sentinel measurement; the lock order of the saved commit has no proof; the vectors are deterministic with a seed and say nothing about a real model; corpus, queries and judgements were written by the same team; the friction list of E8-09 has fifteen points to resolve before 1.0, and point 4 touches a promise (forgetting is only logical and compaction is not reachable through the public wrapper); the mutation of the tests of E8-06 and E8-07 has no evidence (F-10 of the closure). No open blocking finding. **Follow-up package before 1.0: E8-10**, a second evaluation set frozen separately, in which each query class carries query vectors, with queries hard enough not to saturate, thresholds fixed before measuring and a single measurement. History: the start was decided by the owner ([closure of Stage 7, §8.1](../architecture/stage7-closure.md)). The owner approved the [record of Stage 8](../architecture/stage8-record.md) on 2026-11-12, with decisions D1 to D12 (D10 in option b, §8.1 of the minutes). **E8-00 done** on 2026-11-12: [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) accepted with option 2, [ADR-0009](../adr/0009-suppression-retention-and-backups.md) accepted with option 3 (E9A-00b) and written into its file, and closure of Stage 7 approved. **E8-S0 (SQLite report, no thresholds) done** in `078839a` ([report](../testing/sqlite-report.md)). The [early record of Stage 9](../architecture/stage9-early-record.md) also proposes that E9A-02 be closed before Stage 8 writes any derivative; this entry does not decide that proposal.

### Stage 9 — Hardening

- **Entry minutes:** [stage9-record.md](../architecture/stage9-record.md). **Approved by delegation of the owner on 2026-11-12, revocable.** It sets packages E9-00 to E9-CL, the thresholds before measuring and decisions D1 to D23. The decisions that change the format or a promise to users (D1 to D4, D12, D15 and D16) were **withheld for the owner in person on 2026-11-12** and are **taken on 2026-11-13 by express delegation of the owner, revocable** (minutes §8.1). Still pending, not decisions by delegation: the owner's approval of the CLA text before accepting any external contribution, and the declaration of 1.0, which only happens after D13 and real use outside the team.

- **E9-00 (entry gate) done on 2026-11-12:** ADR-0012 accepted in its file and in the [ADR index](../adr/README.md) (D5); body of ADR-0014 aligned with its header ([record §2.3](../architecture/stage9-record.md), e). The decisions of [record §8.1](../architecture/stage9-record.md) are recorded here as the minutes set them. Those taken by delegation are revocable by the owner; those withheld are left for the owner in person.

| Decision | Status | Option recorded |
|---|---|---|
| D1 | Taken on 2026-11-13 (withheld on 2026-11-12) | (a): opaque random token with persisted mapping (option 1 of ADR-0005). E9-02 merged on 2026-11-13 (squash `2697980`) |
| D2 | Taken on 2026-11-13 (withheld on 2026-11-12) | (a): AEAD per block, as the design if encryption is built. Nothing is implemented |
| D3 | Taken on 2026-11-13 (withheld on 2026-11-12) | (b): no encryption in 1.0; 1.0 does not promise confidentiality at rest or key management, and declares it |
| D4 | Taken on 2026-11-13 (withheld on 2026-11-12) | (a): no authenticated integrity in 1.0; the limit stays in the threat model and `verify()` with `deep` is the resource. H-2 closed as an accepted and documented limit |
| D5 | Taken | (a): ADR-0012 accepted as it is. E9-00 writes it into its file |
| D6 | Taken | (a): report without thresholds against SQLite |
| D7 | Taken | (a): no absolute latency limit |
| D8 | Taken | (a): no on-disk format for summaries nor for audit in 1.0 |
| D9 | Taken | (a): current crates, no facade; arrangements of §2.4 |
| D10 | Taken in what the draft is | (a): export draft, not frozen |
| D11 | Taken | (a): v1 fixtures, ADR-0020 fixture rebuilt or declared absent |
| D12 | Taken on 2026-11-13 (withheld on 2026-11-12) | (b): 1.0 is declared only after D13 and real use outside the team. **1.0 is not declared now** |
| D13 | Taken as criterion | (a): the requirements list of §8 |
| D14 | Taken | (a): public `Reclaim` and `reclaim()` with the text of ADR-0009 |
| D15 | Taken on 2026-11-13 as direction (withheld on 2026-11-12) | (b): broad non-exclusive licence with the right to relicense. The legal text is not written and needs the owner's approval before accepting any external contribution |
| D16 | **Closed on 2026-11-13** (withheld on 2026-11-12) | (b): ADR-0016 was pending confirmation until G1 to G7 were reviewed. The review was done and ADR-0016 is **accepted and confirmed on 2026-11-13** with amendments (open work listed in the ADR) |
| D17 | Taken | (a): clause of C-4 not rewritten; contradiction recorded |
| D18 | Taken | (a): all classes in option 1; no declared copy function |
| D19 | Taken | (a): I-28 does not apply in 1.0; I-42 and I-43 promoted if E9-CL verifies them; I-29 for `kerno-units` if E9-03 passes |
| D20 | Taken | (i) accept H-3 with its test; (ii) require evidence of F-10 (E9-15); (iii) review K-E3-7 (E9-16) |
| D21 | Taken | (b): property tests with the library already present; no new tool |
| D22 | Taken | (a): thresholds of §5 unchanged before freezing |
| D23 | Taken by delegation (E9-08f) | Raw writing and test hooks of `UnitDb` only under the `test-support` feature |

- **Closure (E9-CL) drafted on 2026-11-13:** [closure of Stage 9](../architecture/stage9-closure.md). All the packages of the stage are merged: E9-02 (public identifiers) was merged on 2026-11-13 (D1, squash `2697980`). I-27 is now **verified** (`crates/kerno-units/tests/public_ids.rs`; mutation record in `docs/architecture/stage9-mutations-e9-02.md`). D1, D2, D3, D4, D12, D15 and D16, withheld on 2026-11-12, are **taken on 2026-11-13 by express delegation of the owner, revocable**: 1.0 is not declared (D12), there is no encryption (D2, D3), H-2 is closed as an accepted limit (D4), and ADR-0016 is accepted and confirmed with amendments after the review of G1 to G7 (D16, closed). The other decisions (D5 to D11, D13, D14, D17 to D23) were taken by the owner's delegate and are **revocable**. I-29 (for `kerno-units`), I-42 and I-43 promoted by D19, and I-44 and I-45 new, also by delegation and revocable ([invariants](../architecture/invariants.md) §2.7). SAD at version 0.4.1. Pending items and known limits, not closed: K1 (from the end that lost its entry), K4 and K5 open; fuzzing of `wal`, `manifest` and `open` with few runs and the machine not free; P1 and P2 measured without the machine at rest; a storage test possibly flaky; and the phase table of this file, which already says "Closed" for Phase 1 with the date 2026-11-11, the one in the text ([closure](../architecture/stage9-closure.md) §5, point 1). **The owner approved the closure in person on 2026-11-13; 1.0 is not declared (D12) and the CLA text is still not written (D15).**
- **Deliverables:** public IDs (ADR-0005), encryption and keys (ADR-0006), minimised audit, export/import and migrations with fixtures of earlier versions, fuzzing of external inputs, stress tests, comparative benchmarks with SQLite and reference engines (SAD Phases 4 and 6).
- **Acceptance:** limits and guarantees documented; critical failures resolved or marked as blocking; I-27…I-30 verified.
- **Items after Stage 9:** [post9-pending.md](../architecture/post9-pending.md) has the twelve points of the closure with their status; two remain open by nature (fuzzing of `wal`, `manifest` and `open`, and P1 and P2 at rest). 1.0 requirements against the repository (D13): [requirements-1.0.md](../architecture/requirements-1.0.md). CLA draft, not in force: [cla-draft.md](../legal/cla-draft.md). 1.0 is not declared.
- **Early start (in parallel with Stage 7):** [record](../architecture/stage9-early-record.md), approved by delegation of the owner on 2026-11-11. Packages E9A-00a to E9A-06. **Closure of the early start (E9A-06) done on 2026-11-12:** [early closure](../architecture/stage9-early-closure.md). No open blocking finding; I-29 promoted for `kerno-storage`, approved by delegation of the owner on 2026-11-12 and revocable. I-27 was verified later, with E9-02 (merged on 2026-11-13); I-28 and I-30 remain pending (Phase 4).

### Pending owner decisions for these stages

1. **Security order versus AI (resolved on 2026-11-11 by delegation of the owner, in the [early record of Stage 9](../architecture/stage9-early-record.md), §10):** the SAD puts security (Phase 4) before search and AI (Phase 5); the specification leaves it in Stage 9. Proposal: bring forward to before Stage 8 what affects derivatives (suppression, ADR-0009; log redaction, I-29) and leave the rest in Stage 9.
2. Accept or modify ADR-0014…0018 in the indicated stages. ADR-0018 was accepted on 2026-11-12 (option 2).
3. Benchmark thresholds against SQLite, if wanted as a criterion and not only as a report.

### Owner decisions of 2026-11-11

1. **Cut-off point in E7-05.** If the ranking does not meet the quality thresholds of §6.2 of the [record of Stage 7](../architecture/stage7-record.md), Stage 8 does not start until the owner decides how to proceed. **Resolved on 2026-11-12:** Stage 8 starts with the ranking disabled by default and marked as experimental, and T4 is not rewritten now ([closure of Stage 7, §8](../architecture/stage7-closure.md)).
2. **Comparison with SQLite before Stage 8, as a report without thresholds.** The comparison with SQLite, a deliverable of Stage 9, is brought forward before Stage 8 as a report without thresholds, on the workloads both engines can run. Pending item 3 (thresholds against SQLite) remains open.
3. **Real use case before 1.0.** Before a 1.0, a small real use case (memory for an agent) is built that uses only the public API, as a third party would use it.
