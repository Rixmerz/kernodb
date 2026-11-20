# Stage 8 entry minutes: consolidation and embeddings

| Field | Value |
|---|---|
| Date | 2026-11-12 |
| Status | **Approved by the owner on 2026-11-12**, with decisions D1 to D12 of §8.1 (D10 in option b). See §0 |
| Act | Scope, start conditions, decisions and work packages E8-S0 and E8-00 to E8-09 |
| Documents | [Roadmap, Stage 8](../roadmap/README.md) · [Stage 7 minutes](stage7-record.md) · [Stage 7 closure](stage7-closure.md) · [Stage 7 measurement](stage7-measurement.md) · [ranking (E7-04)](stage7-ranking.md) · [Stage 7 evaluation set](stage7-evaluation-set.md) · [Stage 9 advance minutes](stage9-early-record.md) · [ADR-0009](../adr/0009-suppression-retention-and-backups.md) · [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) · [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) · [ADR-0003](../adr/0003-physical-format-and-versioning.md) · [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) · [ADR-0006](../adr/0006-encryption-and-key-management.md) · [invariants](invariants.md) · [SAD §9, §10, §16](SAD.md) · [spec v0.2.0 §10, §11, §14, §16](../spec/architecture-specification-v0.2.0.md) |

## 0. Status

These minutes are **approved by the owner on 2026-11-12**. They describe the state of the repository as it was read on 2026-11-12 and what must be done in Stage 8. The owner approved decisions D1 to D12 with the recommendation of each, except D10, where the owner chose option (b); the choice of each decision is in §8.1.

E8-00 is done with this approval. E8-S0 (SQLite report, without thresholds) is **done** (`078839a`). E8-08 (closure) is **done** on 2026-11-12 with [Stage 8 closure](stage8-closure.md): closed as engineering work, Q4 not demonstrated, decisions of §8 taken by delegation of the owner and revocable. No Stage 8 code is written without going through E8-00.

What the owner has already decided and these minutes record (§1.1 and §8.1) is not proposed again.

## 1. Start conditions

Stage 8 depends on Stages 7 and 6 in the roadmap. Its start condition is «ADR-0018 accepted; suppression of derived data defined (ADR-0009)».

| Condition | Status as read on 2026-11-12 | What is needed |
|---|---|---|
| Stage 7 closed | **Met.** The owner approved the closure on 2026-11-12, and [Stage 7 closure](stage7-closure.md) says so in its header and in its §9 | None |
| Stage 6 closed | [Closure](stage6-closure.md) approved by delegation on 2026-11-11; cache off by default | None |
| Ranking switched off by default and marked as experimental | Met in the code: `RankMode` has `Off` as its default variant (`crates/kerno-units/src/assoc_rank.rs`, `#[default]`). The owner's decision of 2026-11-12 confirms it | That the Stage 8 documentation says so in those words (E8-08) |
| ADR-0018 (versioning and rebuilding of derived data) | **Accepted** on 2026-11-12 with option 2 (common register of derived data), in its file and in `docs/adr/README.md` | **Met** (gate E8-00) |
| ADR-0009 (suppression, retention and backups) | **Accepted** on 2026-11-12 with option 3 (see §1.1). Its file `docs/adr/0009-suppression-retention-and-backups.md` and `docs/adr/README.md` say «Accepted», with date and with option 3. That file records the counting clause of C-4 as contradicted and B-1 without a declared deadline | **Met** (gate E8-00). The follow-up of C-4 and B-1 is in the ADR itself |
| Criteria of ADR-0009 fixed before measuring | Met: the text of the criteria is in ADR-0009, dated 2026-11-11, before any execution of E9A-01 | None |
| Suppression measured (E9A-01) | `crates/kerno-storage/tests/suppression_survival.rs` and commit `2bab299` exist. Its header says that predictions P1 to P4 hold. It is an observation of current behaviour, not a contract | None for E8; E8-05 extends it to derived data |
| SQLite comparison report, without thresholds (owner's decision of 2026-11-11, point 2) | **Does not exist.** `docs/testing/f1-report.md` mentions SQLite only in its method and contains no results. `docs/testing/` has no report with that name | **Done** (E8-S0, `078839a`): [report](../testing/sqlite-report.md) without thresholds, in `docs/testing/sqlite-report.md` |
| Real use case before 1.0 (owner's decision of 2026-11-11, point 3) | Does not apply to the entry of Stage 8: it is a requirement for 1.0 | WP E8-09 (§3), outside the critical path of Stage 8 |
| Owner's decision on rewriting T4 (Stage 7 closure, §8.2) | **Decided on 2026-11-12: option (b).** T4 is left as it is in the Stage 7 minutes and is not rewritten | None (§8.1, D10) |

### 1.1 Owner's decisions these minutes record

Communicated on 2026-11-12:

1. The closure of Stage 7 is approved.
2. Stage 8 starts with the ranking of associative results **switched off by default and as experimental**. The default order remains «hops, then identifier».
3. ADR-0009 is accepted with **option 3** (combination according to the class of data): immediate logical deletion plus removal of live files at compaction when no earlier snapshot remains open. No physical deletion is promised. Cryptographic erasure is left for ADR-0006. No backup deadline is declared until a backup function exists.

Still in force from 2026-11-11: the SQLite report is brought forward before Stage 8 and has no thresholds; the real use case is built before 1.0; the decisions of the advanced Stage 9 (fuzzing with coverage and the status of ADR-0013) were approved by delegation in its own minutes.

## 2. Scope

### 2.1 Objectives

1. **Common register of derived data** (ADR-0018, option 2): each derived datum has a descriptor with its exact sources (unit and revision), its method or model, its version, its parameters, its status (`ready`, `building`, `stale`, `failed`, `superseded`) and whether it is rebuildable.
2. **Staleness and rebuilding by dependency**: changing a source marks its derived data as `stale` in the same `ChangeSet`. Rebuilding produces the same as building anew (I-24).
3. **Vectors as derived data**: persisting embeddings tied to their source revision and to their model profile (I-25). Decided by the owner (§8.1, D2): yes, conditional on E8-03 passing.
4. **Suppression of sources and derived data** according to ADR-0009 option 3 (§5.3).
5. **Consolidation with provenance**: a summary is a derived datum with its sources. It never replaces the auditable original (spec §10).
6. **Adapters outside the core** (I-31): the core stores the result and its descriptor; whoever generates the embedding or the summary is an adapter or an external job, with a contract with no network in the core.

### 2.2 Starting point (what was read in the code)

- **No vector or embedding is persisted.** No file of `crates/kerno-units/src` or `crates/kerno-storage/src` mentions `embedding`, and the persistence files (`db.rs`, `codec.rs`, `index.rs`) do not use `f32`. Vectors exist only in memory and are supplied by the caller (`VectorSet` and `VectorQuery` in `crates/kerno-units/src/assoc_rank.rs`, ADR-0011). Vector search is exact, by brute force (`VectorSet::search`). There is no approximate index.
- **There is no register of derived data.** The derived data that exist today are three (§5.1): the unit indexes (`IDX_TYPE` and the primary index, with status and watermark, I-23), the result cache (in memory only, invalidated by dependencies, off by default) and the ranking (in memory, off).
- **The result cache** (`crates/kerno-units/src/cache.rs`) stores the fingerprint of the query with its dependencies: `schema_version`, `planner_version`, index status and watermark, and `commit_seq`. `Dependency::ALL` fixes the comparison order and `first_change` says which one changed. It is not persisted.
- **There is no export code** in `crates/`, and no backup function. Backup windows cannot be measured (ADR-0009, B-2).
- **Relation adjacencies**: they are primary indexes, written in the same `ChangeSet` as the relation (I-21), not derived data.
- **Dependencies**: `Cargo.toml` and `Cargo.lock` have not changed since before E7-00 (Stage 7 closure, §4, I-31).

### 2.3 Out of scope

- **Approximate vector index (ANN).** Without a benchmark that justifies it (ADR-0011, roadmap). Stage 8 does not write it.
- **Full-text search (BM25) and hybrid fusion.** They belong to the query layer and to another phase (SAD §10.4).
- **Real embedding or summary models.** They sit behind adapters (I-31). This stage does not call any model or any provider.
- **Default ranking.** It stays switched off (§1.1). Switching it on requires a later decision with a new set (§6.2, Q6).
- **Inferred relations.** The [Stage 7 minutes](stage7-record.md) §2.3 assigned them to Stage 8, but the roadmap does not include them among Stage 8's deliverables. These minutes leave them out. The owner decided to leave them out (D8, §8.1).
- **Encryption and cryptographic erasure.** ADR-0006 remains pending. K-1 to K-4 of ADR-0009 do not apply here.
- **Export and import** (I-30) and migrations with fixtures: Stage 9.
- **Using the result cache in associative queries**: out, as in Stage 7.
- **Changes to default values** of `kerno-storage` (`Config::default`) and of `Config::limits`.
- **Modifying files of the frozen v1 format** (ADR-0013). Any persistent change follows §5.4.

## 3. Work packages

All packages with code comply with the common rules of §4. Each one writes only to the files it lists. A package that adds a new module must declare it in `crates/kerno-units/src/lib.rs` (or in `crates/kerno-testkit/src/lib.rs`): that file is shared, so **the packages that add a `mod` do not run at the same time**. New test files under `tests/` do not need to touch `lib.rs`.

| WP | Content | Depends on | Writes | Can run alongside |
|---|---|---|---|---|
| **E8-S0** SQLite report, without thresholds | Comparison of `kerno` against SQLite on the loads both can run, with the equivalent durability that [f1-performance-goals](../testing/f1-performance-goals.md) already declares: `Sync` against `journal_mode=WAL` with `synchronous=FULL`; `Buffered` against `synchronous=NORMAL`, noting that the guarantee on power loss is not identical. Without thresholds (decision of 2026-11-11). Run with the existing benchmark binary | — | `docs/testing/sqlite-report.md` (new), `spikes/storage/results/` (new outputs) | E8-00, E8-01, E8-02 |
| **E8-00** Entry gate | Approval of these minutes. Acceptance of ADR-0018 (option 2, or the one the owner decides). Status of ADR-0009 written in its file. Decisions D1 to D12 of §8 recorded. Closure of Stage 7 marked as approved | Approval of these minutes | `docs/adr/0018-*.md` (only the status and decision section), `docs/adr/README.md`, `docs/roadmap/README.md` | E8-S0 |
| **E8-01** New evaluation set, frozen | Corpus, queries and expected answers with relevance grade (§6.1), generated with its own seed, with a SHA-256 `MANIFEST`. Generator in `kerno-testkit`, which does not depend on `kerno-units` | E8-00 | `crates/kerno-testkit/src/derived_eval.rs` (new), `crates/kerno-testkit/src/lib.rs` (one `mod` line), `crates/kerno-testkit/data/derived-eval/` (new), `docs/architecture/stage8-evaluation-set.md` (new) | E8-S0, E8-02 (as long as E8-02 does not touch `lib.rs` at the same time) |
| **E8-02** Register of derived data in memory | `DerivedDescriptor` type with exact sources (unit, revision), method, version, parameters and input hash; states `ready`, `building`, `stale`, `failed`, `superseded`; reverse index source → derived data maintained by the register itself. No persistence | E8-00 | `crates/kerno-units/src/derived.rs` (new), `crates/kerno-units/src/lib.rs` (one `mod` line), `crates/kerno-units/tests/derived_registry.rs` (new) | E8-S0, E8-01 |
| **E8-03** Staleness and rebuilding | A `ChangeSet` that changes a source marks its derived data as `stale` in the same `ChangeSet`. Rebuilding is the same as building anew. Changing method or version creates a new derived datum and marks the previous one `superseded` | E8-02 | `crates/kerno-units/src/derived_stale.rs` (new), `crates/kerno-units/src/lib.rs` (one `mod` line), `crates/kerno-units/tests/derived_stale.rs` (new) | None of those that add `mod` |
| **E8-04** Persistence of vectors (**conditional**) | Only if D2 = persist and a new ADR is accepted (§5.4). Vectors as derived data tied to their revision and to their model profile, in the format with a new feature bit | E8-03; decision D2; new ADR | `crates/kerno-units/src/vector_store.rs` (new), `crates/kerno-units/tests/derived_vectors.rs` (new), `docs/file-format/` (new document for the bit), `crates/kerno-units/src/lib.rs` | — |
| **E8-05** Suppression of sources and derived data | Deleting a unit, a revision (retention) or a relation deletes in the same `ChangeSet` the derived data of that source (§5.3). Test of what survives, in the form of E9A-01, with sentinels also in vectors and summaries | E8-03; E8-04 if D2 = persist | `crates/kerno-units/src/suppression.rs` (new), `crates/kerno-units/src/lib.rs`, `crates/kerno-units/tests/derived_suppression.rs` (new) | — |
| **E8-06** Consolidation with provenance | Contract of a summary as derived datum: mandatory sources, descriptor, and the original always readable. Adapter as a trait with no network; one deterministic test implementation. No real model | E8-03, E8-05 | `crates/kerno-units/src/consolidation.rs` (new), `crates/kerno-units/src/lib.rs`, `crates/kerno-units/tests/consolidation.rs` (new) | — |
| **E8-07** Measurement | Quality over the new set (§6.2) and performance (§6.3). Committed raw output | E8-01 to E8-06 | `crates/kerno-units/tests/derived_measure.rs` (new), `crates/kerno-units/examples/derived_bench.rs` (new), `docs/architecture/stage8-measurement.md` (new), `docs/architecture/stage8-measurement-output.txt` | — |
| **E8-08** Closure (**done** on 2026-11-12, [closure](stage8-closure.md)) | Adversarial review of E8-01 to E8-07. Promotion of I-25, I-38 and the new ones (§7). Update of [invariants](invariants.md) (I-25 moves from «Phase 5» to Stage 8), roadmap and [SAD](SAD.md) as a new numbered version | E8-00 to E8-07 | `docs/architecture/stage8-closure.md` (new), `docs/architecture/invariants.md`, `docs/roadmap/README.md`, `docs/architecture/SAD.md` (new version) | — |
| **E8-09** Real use case (pre-1.0) | Memory for an agent, with only the public API and as a third party would use it. Uses the register of derived data if E8-05 is closed | E8-05 (does not block the closure of Stage 8) | `examples/agent-memory/**` (outside the workspace, with a path dependency), no changes to the workspace `Cargo.toml` | Can run after E8-05, alongside E8-07 |

### 3.1 What runs at the same time

- **Now, without waiting for the owner:** nothing in E8 writes code before the approval of these minutes. The only thing that can run at the same time after approval is **E8-S0**, which touches no code.
- **After E8-00:** E8-01 and E8-02 in parallel, because they write different files. Their `mod` lines in `lib.rs` are merged in the order they arrive; each one checks the test count after the merge.
- **Sequential:** E8-03 after E8-02; E8-05 and E8-06 after E8-03; E8-04 only if D2 asks for it.
- **Critical path:** E8-00 → E8-02 → E8-03 → E8-05 → E8-07 → E8-08.

## 4. Common acceptance criteria

Each package with code meets the following. A package that does not meet them is not closed.

1. `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` without errors. Clippy rejects complex return types (`type_complexity`): use type aliases.
2. `cargo test -p kerno-units -p kerno-testkit --all-features --release` green, with the test count noted before and after. The new file must appear in the count: if the number does not move, the runner does not see it.
3. Every new test is tested in **both engines**, `emulated_log` and `kerno_db`, as in Stages 4 to 7.
4. **Each new test is seen to fail** with a mutation applied to a **copy** of the tree, reverted and checked with `git diff`. The mutation is recorded with its line. A mutation that leaves the test green is a finding, not a step.
5. Where a derived datum answers a query, the answer is compared with the **reference path**: the same query over the same snapshot, with the derived datum unused (differential, I-32).
6. No test depends on time, on the order between tests, or on shared state.
7. No query literal, content or vector appears outside the `diagnostic` level (I-37). The Stage 5 sentinel tests are extended to the new types.
8. No change to `Cargo.toml` or `Cargo.lock` unless decision D2 or D8 asks for it. If it does, it is reviewed separately (I-31).

## 5. Derived data: contract, invalidation and suppression

### 5.1 What is derived today

| Data | Derived from | Where it lives | How it is invalidated today |
|---|---|---|---|
| Primary index of units and `IDX_TYPE` | The heads of the units | Persisted, in `kerno-units` (ADR-0007) | `status` and `watermark` (I-23). A query that is not `Ready` in its snapshot goes to the reference path |
| Result cache | State of the database in a snapshot | In memory only, off by default | By dependencies: `schema_version`, `planner_version`, index status and watermark, `commit_seq` (`cache.rs`) |
| Search vectors | Source revision and model profile | **Supplied by the caller, in memory** | Not applicable: they are not stored. Each use starts from what the caller supplies |
| Ranking | Signals of the query and the result | In memory only, off | Not persisted |

Relation **adjacencies** are not derived data: they are primary and go in the same `ChangeSet` as the relation (I-21).

With this table, the ADR-0018 register is not born empty: its first users are the index and the cache, which already have state and dependencies. The register can describe them without changing their format.

### 5.2 Invalidation by dependency

- Each derived datum stores, in its descriptor, **the exact sources**: the unit and the revision (or the `commit_seq` range), the method or model with its version, and the parameters that change the result.
- **Changing a source** (a `ChangeSet` that touches the unit or the revision) marks its derived data as `stale` **in that same `ChangeSet`**, through the reverse index. In this way the status never falls behind the database (I-21, I-23).
- **Changing the method, the model or the version** does not rewrite the derived datum: it creates a new one and marks the previous one `superseded` (ADR-0018, option 2).
- **Rebuilding** redoes the derived datum from the sources that remain valid. Its result must equal that of building anew over the same data (I-24). A derived datum is never the source of truth: if it is lost, it is redone.
- **A cache is not persisted** (the result cache does not persist), so its invalidation happens in memory and does not touch the format.

### 5.3 How a suppression reaches persisted derived data

This section is the treatment that ADR-0009 option 3 asks for. What follows is what Stage 8 will implement, not what the engine does today.

1. **Same `ChangeSet`.** Deleting a unit, a revision (retention) or a relation writes, in the same `ChangeSet`, the deletion of its derived data. They are obtained with the reverse index source → derived data. It is the atomicity rule of I-21 applied to derived data.
2. **What is removed immediately** (criterion I-1 of ADR-0009, extended to derived data): after the commit, no new read returns the vector, the summary or the descriptor of the deleted source. It covers `get`, `scan` and the E8 queries.
3. **Open snapshots** (criterion I-2): a snapshot taken before the commit still sees its derived data until it is released.
4. **What survives on disk** (criterion C-1 applied to derived data): the bytes of the vector or of the summary may remain in a WAL or in a table until a compaction covers their range. After `compact_range` and a clean close, the sentinel of that vector does not appear in any live file that any snapshot retains (C-1 and C-3). The deadline is that of ADR-0009 (C-4), not a duration.
5. **Summaries**: a summary has as sources a set of units. Deleting **any** of them deletes the summary in the same `ChangeSet`. The summary never keeps the text of a deleted source, and it never replaces the original while the original exists (spec §10).
6. **Backups**: no deadline is declared. ADR-0009, B-1, asks for a window declared in writing; that declaration is «there is no backup function in the engine; the deadline of the copies the user makes is not promised». When a backup function exists, its window is declared at that time.
7. **Cryptographic erasure**: outside Stage 8. It depends on ADR-0006, which is pending. It is not promised.
8. **No promise of physical deletion**: the engine does not guarantee that the bytes disappear from the medium (threat model §6.5).

What E8-05 measures with sentinels: the stages of E9A-01 (before the flush, after `compact_range`, after a clean close, after reopening), for `delete` of a unit with a vector and with a summary. The predictions are written before running, as in E9A-01.

### 5.4 Persistent format

Question: **does the persistent format change in Stage 8?** The answer depends on D2 and on what is persisted.

- **No persistence** (E8-02, E8-03, E8-05 and E8-06 in memory): the format does not change. The register is rebuilt from the sources when the database is opened. This is valid because a derived datum is rebuildable (ADR-0018, criterion 2).
- **With persistence** of descriptors or vectors (E8-04): **it does change**. The decision on how it is persisted, where the keys live, which bit and which atomicity is in [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md), accepted on 2026-11-12. It must follow the format versioning rule and not an edit of the frozen format:
  - ADR-0003 §1 and §3: a change is added as a **feature bit**. A `compat` bit is ignored; a `ro_compat` bit opens read-only only; an `incompat` bit is rejected.
  - Derived data require **`incompat`**. An old reader that ignored those records could write over the database without marking its derived data as stale: an invisible incorrect state. ADR-0003 §3 says that an old version never writes over a format it does not understand.
  - **ADR-0013 is not edited.** It is accepted and the v1 format was frozen on 2026-10-31. The v1 files and their vectors do not change. The new bit is described in a new format document, and the change needs a new ADR.
  - **ADR-0011 is not edited.** It says that persisting vectors requires an ADR that replaces it (ADR-0011, Consequences). The new ADR takes the next free number (0020; the last existing one is 0019); the owner assigns it when it is opened.
- **Versioning of derived data**: independent of the physical version (ADR-0003 §4). Changing the model or the algorithm marks the derived data as `superseded` and does not migrate the format.

## 6. New evaluation set and thresholds

### 6.1 New set

The Stage 7 set is not used to measure anything in this stage. That set has already seen the results (Stage 7 closure, §8.2), and measuring it again adds no evidence.

- **A different seed** from that of Stage 7, fixed in the generator before writing E8 retrieval code.
- **Identifiers assigned by seeded random permutation**, not in creation order. In this way the tie-break by identifier does not favour any structure. This corrects the structural signal that Stage 7 could not rule out (measurement §5, point 2).
- **Corpus**: about 2 000 units for quality, with relations of the four provenance classes and distractors. Performance scale: 20 000 units (as in Stage 7).
- **Queries**: at least 100, in four classes of at least 20 each: neighbourhood and vector retrieval; temporal; provenance; and combined. One same set contains the vector queries, with vectors **generated by a seeded deterministic function** (see D5).
- **Judgements**: relevance grade 0 to 3, written from the narrative with which the corpus is generated, not from the code of `kerno-units`. A new judgement requires freezing the `MANIFEST` again before measuring.
- **Corrected judgement convention**: units **not judged** in the top 10 of an arm do not count as grade 0. They are excluded from the calculation and their number is reported per arm. If more than 10 % of the top-10 positions of an arm are unjudged, the set is considered incomplete and more units are judged before measuring. This corrects the bias of R-5 of Stage 7 (§9).
- **Freezing**: `MANIFEST` with SHA-256 of each file, checked by a test before each measurement, as in Stage 7.
- **Declared limit**: the narrative, the queries and the judgements are written by the same team. No independent judgement exists. This is said in the closure.

### 6.2 Quality (cuts at 10)

The thresholds are fixed in these minutes, before any measurement of the new set. Each one is checked against the bound of its metric before being fixed (see the check column).

| Code | Threshold | Check against the metric |
|---|---|---|
| **Q1** | The exact vector search of E8 gives the same result as the reference brute-force search: recall@10 = 1.000 | Recall is in [0, 1]; 1.000 is attainable because the arm is the same search |
| **Q2** | nDCG@10 with vector signal ≥ 0.80, overall | nDCG is in [0, 1]; Stage 7 measured 0.912 with the ranking |
| **Q3** | Recall@10 with vector signal ≥ 0.90, overall | Recall in [0, 1]; Stage 7 measured 0.939 |
| **Q4** | With vector signal, nDCG@10 ≥ nDCG@10 without it − 0.01, **overall and in each class** | It is a non-inferiority: the bound lies below the value measured without it, so it demands no more than 1 |
| **Q5** | Validity of the set: unjudged units in the top 10 ≤ 10 % of positions, in each arm | It is a validity condition, not a quality one. If it fails, nothing is measured: more is judged |
| **Q6** | **Only if the owner proposes switching on the ranking by default in a later stage**: nDCG@10 ≥ simple order + 0.02 and not lower by more than 0.01 in any class | The bound demands ≤ 1: if the simple order exceeds 0.98, the threshold is unattainable and is rewritten before measuring. **It is not a closure criterion of Stage 8** |

Q6 is added because the decision of §1.1 leaves the ranking off and experimental. This stage does not measure it to switch it on.

What is **not** fixed here, and why: an absolute quality threshold for the ranking with vectors. Without a measurement of the vector signal on a set with vectors, there is no baseline to start from. The result of Q1 to Q5 is what exists at the moment E8 closes.

### 6.3 Performance

The performance thresholds are comparisons between paths or between writes, so that they do not depend on the machine. They are fixed here and measured on the same machine, with the same repetitions as in Stage 7.

| Code | Threshold | Check |
|---|---|---|
| **P1** | Writing a source with its derived-datum descriptor: p50 ≤ 1.50 × the write of the same source without a descriptor, with 20 000 units | A ratio of two p50 measured in the same session; 1.50 is a ceiling, not a target |
| **P2** | Search with persisted vectors (only if D2 = persist): p50 ≤ 1.10 × the same search with vectors supplied in memory | Same data and same query; a ceiling of 10 % read overhead |
| **P3** | Invalidation: the entries marked `stale` are exactly the derived data of the sources that changed | It is a counter, not a time: equality is checked |
| **P4** | Rebuilding: the result is equal, byte for byte, to that of building anew (I-24) | Equality, not time. The time to rebuild is reported without a threshold |

No absolute latency threshold. The [Stage 7 minutes](stage7-record.md) §6.3 did not have one either, and the [measurement](stage7-measurement.md) §4 showed that absolute time depends on the machine's load. If the owner wants an absolute limit, that is decision D9, and it is fixed before measuring.

## 7. Invariants

| ID | Current status | What Stage 8 does | Package |
|---|---|---|---|
| **I-25** | «Phase 5» in [invariants](invariants.md) | Every embedding-derived datum references the exact source revision and the model profile; changing the source marks it stale. It is verified and moves to «Stage 8» | E8-02, E8-03, E8-04 if D2. **Status after E8-08:** promoted to Stage 8 for persisted vectors ([invariants §2.6](invariants.md)); the test that varies only the profile is missing (F-04). Pending the owner's approval |
| **I-38** | «Stage 8» | Every derived datum references its exact sources, its method and its version; changing a source marks it stale or creates a new version. It is verified | E8-02, E8-03. **Status after E8-08:** verified and promoted ([invariants §2.6](invariants.md)). Pending the owner's approval |
| **I-24** | Verified in indexes | Extended to derived data: rebuilt the same as new | E8-03, E8-07 (P4) |
| **I-21** | Verified in units and relations | Extended to derived data: the descriptor, its status and its deletion go in the same `ChangeSet` as the source | E8-05 |
| **I-1, I-2** (ADR-0009) | Verified for the value | Extended to derived data: invisible after the commit; visible in the open snapshot | E8-05 |
| **I-31** | Verified by dependency review | No adapter or consolidation contract brings network or model into the core | E8-06, E8-08 |
| **I-37** | Verified in observations | The sentinels of vectors and summaries do not leave `diagnostic` | E8-05, E8-07 |
| **I-41** | Proposed for promotion (Stage 7) | No change; the switched-off ranking still meets it | — |
| **I-42** (**proposal**) | Does not exist | Suppressing a source leaves its derived data invisible in the same `ChangeSet` (§5.3) | E8-05. **Status after E8-08:** verified in tests ([invariants §2.6](invariants.md)); pending the owner's approval |
| **I-43** (**proposal**) | Does not exist | A summary never replaces the original while the original exists and is auditable (spec §10) | E8-06. **Status after E8-08:** verified in tests ([invariants §2.6](invariants.md)); pending the owner's approval |

I-42 and I-43 are proposals. Neither is promoted until E8-08 verifies it and the owner approves it. The [Stage 9 advance minutes](stage9-early-record.md) §6 says that, if ADR-0009 is accepted, Stage 8 will propose its suppression invariant. These minutes propose it (I-42) without going beyond ADR-0009.

## 8. Decisions the owner must take

Each one has its recommendation. The owner's choice for each is in §8.1, after this table.

| # | Decision | Options and consequences | Recommendation |
|---|---|---|---|
| **D1** | ADR-0018 | (a) Accept option 2 (common register). (b) Accept with changes. (c) Keep it proposed and leave E8 without entering | **(a)**. Without a common register, each derived datum carries its own suppression and freshness logic (ADR-0018, option 1) |
| **D2** | Are vectors persisted in Stage 8? | (a) **Yes**, as derived data with an `incompat` bit and a new ADR (§5.4; [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md)). It meets «invalidate or rebuild» with real data. It requires a format change. (b) **No**: the register only describes the vectors that the caller supplies. The acceptance of «derived datum linked to its sources» is met only in memory. (c) Postpone to a later stage with ANN | **(a)**, if E8-03 passes. Without persistence there is no persisted derived datum to invalidate, and the acceptance of Stage 8 is empty in its embedding part. Status: ADR-0020 accepted on 2026-11-12; E8-03 passed |
| **D3** | Default ranking in Stage 8 | Already decided on 2026-11-12: off and experimental | No action |
| **D4** | Thresholds Q1 to Q5 and P1 to P4 of §6 | (a) Approve them as they are. (b) Modify before measuring. Once the new set is measured, changing them is a breach, not a correction | **(a)** |
| **D5** | Origin of the vectors of the set | (a) **Seeded deterministic vectors**, generated by a test function. They measure the mechanics of search and of the register, not the semantic quality of a model. (b) An external model, run outside the core and outside CI, with its cost and version recorded. No model is named here. It requires a separate decision | **(a)** in this stage. The consequences of (b) are different: there is no quality measurement of real embeddings until a decision is made |
| **D6** | Real use case (E8-09): when | (a) After E8-05, before 1.0. (b) After the closure of Stage 8. (c) After 1.0 | **(a)**. It is the only point where the API is tested from outside; such a case in Stage 9 would arrive late |
| **D7** | SQLite: thresholds against SQLite (pending 3 of the roadmap) | (a) Without thresholds, report only (decision of 2026-11-11). (b) Fix thresholds before the report | **(a)**. A threshold against SQLite without prior data repeats the error of T4 |
| **D8** | Inferred relations | (a) Outside Stage 8 (§2.3). (b) Inside, with its own provenance descriptor (spec §12, I-39) | **(a)**. The roadmap does not put them among Stage 8's deliverables. If they enter, their invariants are met first in Stage 4 or 5 |
| **D9** | Absolute search latency limit | (a) None: only comparisons between paths (§6.3). (b) A limit fixed before measuring | **(a)**. ADR-0011 says that an unmet limit opens a new decision, and there is no latency baseline with persisted vectors |
| **D10** | Rewrite T4 (Stage 7 closure, §8.2) | (a) Rewrite it before any new measurement with an attainable threshold, frozen with a date. (b) Leave it as it is in the Stage 7 minutes | **(a)**, with the Stage 7 text unchanged. It does not affect the new set of §6, which has its own thresholds |
| **D11** | Backups: declared window (ADR-0009, B-1) | (a) Declare that the engine has no backup function and that the deadline of the copies is not promised. (b) Wait for a backup function before writing the declaration | **(a)**. It is the only true declaration today |
| **D12** | Consolidation: scope of E8 | (a) Only the summary contract as derived datum, with a test adapter (E8-06). (b) In addition, a real summariser | **(a)**. A real summariser needs a model, which Stage 8 does not fix (D5) |

### 8.1 Owner's choices (2026-11-12)

The owner approved these minutes on 2026-11-12 with the recommendation of each decision, except D10. D3 and D7 were already decided before this approval, and they are recorded that way.

| # | Owner's choice | Origin |
|---|---|---|
| **D1** | **(a)**: ADR-0018 accepted with option 2 (common register of derived data) | Approval of the minutes, 2026-11-12, with the recommendation |
| **D2** | **(a)**: vectors are persisted in Stage 8 as derived data, with an `incompat` bit and a new ADR (planned as ADR-0020, §5.4). Conditional on E8-03 passing. The frozen v1 format is not touched. **Condition met on 2026-11-12** (E8-03: 46 and 13 tests green). **ADR-0020 accepted on 2026-11-12 by delegation of the owner**, revocable by the owner. The bit is declared as version 2 of the unit layer, not in the engine (ADR-0020, §Options 2) | Approval of the minutes, 2026-11-12, with the recommendation; ADR-0020 by delegation |
| **D3** | Already decided: ranking off by default and marked as experimental. No further action | Owner's decision of 2026-11-12 (§1.1, point 2) |
| **D4** | **(a)**: thresholds Q1 to Q5 and P1 to P4 of §6 approved as they are | Approval of the minutes, 2026-11-12, with the recommendation |
| **D5** | **(a)**: seeded deterministic vectors, generated by a test function. No external model is used | Approval of the minutes, 2026-11-12, with the recommendation |
| **D6** | **(a)**: real use case (E8-09) after E8-05 and before 1.0 | Approval of the minutes, 2026-11-12, with the recommendation |
| **D7** | Already decided: SQLite report without thresholds (option (a)) | Owner's decision of 2026-11-11 (roadmap, decisions of 2026-11-11, point 2). Not a new decision |
| **D8** | **(a)**: inferred relations outside Stage 8 | Approval of the minutes, 2026-11-12, with the recommendation |
| **D9** | **(a)**: no absolute search latency limit; only comparisons between paths (§6.3) | Approval of the minutes, 2026-11-12, with the recommendation |
| **D10** | **(b)**: T4 of the Stage 7 minutes is left as it is and is not rewritten. **It is not the recommendation, which was (a)** | Owner's choice, 2026-11-12 |
| **D11** | **(a)**: it is declared that the engine has no backup function and that the deadline of the copies is not promised | Approval of the minutes, 2026-11-12, with the recommendation |
| **D12** | **(a)**: only the summary contract as derived datum, with a test adapter (E8-06). No real summariser | Approval of the minutes, 2026-11-12, with the recommendation |

The two conditions this paragraph asked for are met on 2026-11-12: the approval of these minutes (§10) and the status of ADR-0018 and ADR-0009 written in their files.

## 9. Risks

| ID | Risk | Proposed treatment |
|---|---|---|
| **R-E8-1** | ADR-0009 accepted by the owner but not in the file: E8 rests on a decision the repository does not show | E8-00 requires the status written in `0009` and in `docs/adr/README.md` |
| **R-E8-2** | The new set is adjusted to the implementation, as happened with the Stage 7 set | Seed and identifier permutation fixed before writing retrieval code; `MANIFEST` with SHA-256; judgements written from the narrative, not from the code |
| **R-E8-3** | The judgement convention favours one arm, like R-5 of Stage 7 | Unjudged units excluded and counted per arm; Q5 as a validity condition |
| **R-E8-4** | A synthetic set does not represent the agent's real memory | Declared in the closure. The real use case (E8-09) measures the API from outside, but not semantic quality |
| **R-E8-5** | Seeded deterministic vectors say nothing about the semantic quality of a model | Declared in D5. Q1 to Q4 measure the mechanics, not the quality of real embeddings |
| **R-E8-6** | Changing the format without the right bit lets an old reader write over derived data that were not invalidated | `incompat` bit (§5.4); test of rejection of a reader that does not know the bit, with the directory identical byte for byte |
| **R-E8-7** | A suppression leaves bytes of the vector or of the summary in live files until compaction | This is the behaviour that ADR-0009 option 3 accepts. It is measured with sentinels in E8-05 and declared as a limit. No physical deletion is promised |
| **R-E8-8** | An open snapshot retains deleted derived data | Criterion I-2 of ADR-0009. The deadline is the release of the snapshot (C-4), not a duration |
| **R-E8-9** | A derived datum is left `stale` but is served as current after reopening the database, if the register is not persisted | Without persistence, the register is rebuilt on opening; a derived datum not rebuilt is marked `failed` or not served, never `ready` (I-23). With persistence, R-E8-6 covers it |
| **R-E8-10** | The register grows in the reverse index with many derived data per source | ADR-0018, conditions for reopening: if the cost of the reverse index is disproportionate, the ADR is reopened. Measure in E8-07 |
| **R-E8-11** | A real summary could silently replace the original | I-43 (proposal). The E8-06 contract requires the original to remain readable. Adversarial review in E8-08 |
| **R-E8-12** | Index H-1 (Stage 7 closure, R-2): with 64 relations or more, an entry missing at both ends is not detected at query time | Still open. E8 does not close it. Derived data that depend on adjacency are not declared complete by the index: they are verified with `verify_units` or with the reference path |
| **R-E8-13** | Stage 8's derived data inherit the judgement-convention bias of Stage 7 if its measurement code is reused | E8-07 does not reuse `assoc_measure.rs`: it writes its own measurement, with the convention of §6.1 |
| **R-E8-14** | The SQLite report is read as a threshold even though it has none | The title and the text of E8-S0 say so: without thresholds (D7) |
| **R-E8-15** | Delaying E8-09 until 1.0 leaves the API untested from outside | D6. If the owner chooses (c), it is recorded as a risk |

## 10. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-12 | **Proposal. Pending approval by the owner** | No part of Stage 8 is approved by delegation. Gates of E8-00: these minutes, ADR-0018, ADR-0009 written in its file, Stage 7 closure marked as approved, and decisions D1 to D12. No Stage 8 code before that approval |
| 2026-11-12 | **Closure of Stage 8 (E8-08), done** | [Closure](stage8-closure.md). Closed as engineering work; Q4 recorded as not demonstrated; no open blocking finding; decisions (a) to (e) taken by delegation of the owner on 2026-11-12, revocable by the owner. No threshold or frozen file modified |
| 2026-11-12 | **Approved by the owner** | D1 to D12 with the recommendation of §8, except D10: option (b), T4 is not rewritten (§8.1). ADR-0018 accepted with option 2 and ADR-0009 accepted with option 3, both in their file. Stage 7 closure approved. E8-00 is done; E8-S0 remains pending. No threshold of §6 is modified |
