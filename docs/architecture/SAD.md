# KERNO — Initial architecture document and implementation plan

| Field | Value |
|---|---|
| Version | 0.4.1 |
| Date | 13 November 2026 (0.4.1: 13 November 2026; 0.3.0: 12 November 2026; 0.2.0: 29 October 2026; 0.1.0: 27 October 2026) |
| Status | Baseline 0.1.0 with the updates of Phase 1 (0.2.0), of Stage 8 (0.3.0), of Stage 9 (0.4.0) and of the decisions of 2026-11-13 (0.4.1), which list "Changes in version 0.2.0", "Changes in version 0.3.0", "Changes in version 0.4.0" and "Changes in version 0.4.1"; the rest of the text has not been rewritten |
| Type | Software architecture document (SAD) |
| Implementation language | Rust |
| Initial delivery model | Embedded library and local CLI |

> **Executive summary.** Kerno will be an embedded, persistent and extensible data engine. Its core will be a transactional key-value store; on top of it, structured documents, revision history, indexes and, later, hybrid retrieval for AI will be built. The architecture prioritises correctness, recovery, portability and explicit evolution of the data. The physical storage algorithm —copy-on-write B+ tree or LSM— remains pending an ADR based on a reproducible technical test.

> **Maintenance note.** Version 0.1.0 was the baseline approved for review. Later decisions are recorded in [`docs/adr/`](../adr/README.md); when an ADR changes something in this document, a new version of the SAD is published instead of silently editing this one. Version 0.2.0 (closure of Phase 1) lists below each change; everything not listed there keeps the text of 0.1.0, even where Phase 1 has gone beyond it in detail (for example, the split of crates in §14 is the proposed one, not the built one).

## Changes in version 0.4.1

Decisions of 2026-11-13. The owner, in person, asked the delegate to take the best decision on each pending point. D1, D2, D3, D4, D12, D15 and D16, retained on 2026-11-12 (see version 0.4.0), are **taken by express delegation of the owner, revocable**. This change does not rewrite the body of the document or version 0.4.0.

| Section | Change |
|---|---|
| §11 | Public identifiers: D1 taken (option 1 of ADR-0005). E9-02 is **merged** (2026-11-13); I-27 verified by `crates/kerno-units/tests/public_ids.rs`, with the mutation log in [stage9-mutations-e9-02.md](stage9-mutations-e9-02.md); the units layer moves to mark 3 ([units-v3.md](../file-format/units-v3.md)). No encryption or keys in 1.0 (D2 and D3) and no authenticated integrity in 1.0 (D4) |
| §13 | No change to the v1 engine format; the units layer moves to mark 3 (public identifiers) and the draft logical export to `format_version` 1 (it carries tokens). The freezing of the export format is not done: D12 leaves 1.0 for after D13 and real use outside the team |
| §15 | ADR-0005 accepted with option 1 (D1). ADR-0006 remains pending; its option 1, per-block AEAD, is the chosen design if encryption is built (D2). ADR-0016 appears as accepted in its file and remains pending the owner's confirmation (D16) |
| §16 | Status of Stage 9: closed as engineering work, with all packages merged (E9-02 on 2026-11-13); the owner approved the closure in person on 2026-11-13. 1.0 is not declared (D12) |

## Changes in version 0.4.0

Closure of Stage 9 ([closure](stage9-closure.md)). This version records what the stage built and what it left **undecided**. It does not rewrite the body of the document: each row refers to the section affected and to the document that details it. Decisions taken by delegation of the owner are revocable; D1, D2, D3, D4, D12, D15 and D16 remain retained.

| Section | Change |
|---|---|
| §9, §10 | The units layer has reads of derived data (`snapshot`, `query`, `history`, `derived_of`, search over persisted vectors), public replacement of vectors and `UnitError::is_locked`. `UnitDb::engine` is no longer public: the wrapper does not return the engine (I-45) |
| §11 | Public physical reclaim: trait `Reclaim` in `kerno-core` and `reclaim()` in `UnitDb` and `DerivedDb`, with the text of ADR-0009 (option 3): mechanism, not physical deletion (I-44). Minimised audit extended to `kerno-units` (I-29). **No encryption, no keys and no public identifiers with token** in this version (D1 to D4 retained; I-27 not verified; I-28 does not apply) |
| §13 | Logical export as a **draft, not frozen** (I-30 for the draft; the freezing is D12) and reimport into a new engine. The v1 on-disk format does not change |
| §15 | ADR-0012 accepted (D5) and ADR-0014 aligned. ADR-0005 and ADR-0006 remain unaccepted; ADR-0016 appears as accepted in its file and remains pending the owner's confirmation (D16) |
| §16 | Status of Stage 9: closed as engineering work with E9-02 retained; limits and pending items in the [closure](stage9-closure.md) §5. 1.0 is not declared (D12) |

## Changes in version 0.3.0

Closure of Stage 8 ([closure](stage8-closure.md)). This version describes the derived data as it is built, not as 0.1.0 proposed it nor as the [minutes of Stage 8](stage8-record.md) proposed it before measuring.

| Section | Change |
|---|---|
| §9.2 | The index derivatives (primary and `IDX_TYPE`) keep their status and I-23. The common derivative registry of [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) (option 2) is added, in memory or persisted depending on the data |
| §10.2 | A persisted vector stores its source revision, its method, its version and its model profile (opaque text). Identity includes them |
| §10.3 | Life cycle: `ready`, `building`, `stale`, `failed`, `superseded`. A source change marks `stale` in the same commit. Rebuilding gives the same as building again. A new method or version creates a new derivative and marks the old one `superseded` |
| §10.5 (new) | Derived data as built: persisted vectors as layer 2 (ADR-0020), suppression of sources and of derivatives, in-memory summaries with provenance |
| §13 | The units layer moves to version 2 only in the databases that have persisted vectors (`LAYER_META` mark). A build of version 1 of the layer rejects those databases. The engine and the frozen v1 format (ADR-0013) do not change |
| §15 | ADR-0018 accepted (option 2); ADR-0020 accepted by delegation of the owner on 2026-11-12, which replaces in its persistence part the clause of ADR-0011. ADR-0009 accepted with option 3 |
| §16 | Status of Stage 8: closed as engineering work; Q4 not demonstrated; follow-up package E8-10 before 1.0 |

## Changes in version 0.2.0

| Section | Change |
|---|---|
| §2.2 | Status note: the physical engine, the commit protocol and the binary representation are resolved by ADR-0001, ADR-0002 and ADR-0003 (Accepted on 2026-10-28) and by format v1 ([ADR-0013](../adr/0013-format-v1-and-engine-protocol.md), Proposed). The other decisions of the list remain open. |
| §14 | Note with the crates that exist today: `kerno-core`, `kerno-testkit` and `kerno-storage`, plus the frozen M0 spikes. |
| §15 | Current status of each ADR (0001–0012) and reference to ADR-0013…0018, which were not in the original list. |
| §16 | Status of Phase 0 (closed) and of Phase 1 (complete implementation; open exit items and their reason). |

## Contents

| No. | Section |
|---|---|
| 1 | [Purpose and scope](#1-purpose-and-scope) |
| 2 | [Confirmed and open decisions](#2-confirmed-and-open-decisions) |
| 3 | [Architecture principles](#3-architecture-principles) |
| 4 | [Deployment and concurrency](#4-deployment-and-concurrency-model) |
| 5 | [Logical data model](#5-logical-data-model) |
| 6 | [Module architecture](#6-logical-module-architecture) |
| 7 | [Storage, transactions and recovery](#7-physical-storage-transactions-and-recovery) |
| 8 | [Revision Engine](#8-revision-engine-history-differences-and-provenance) |
| 9 | [Indexes and query engine](#9-indexes-and-query-engine) |
| 10 | [Embeddings and hybrid retrieval for AI](#10-embeddings-and-hybrid-retrieval-for-ai) |
| 11 | [Security, privacy and life cycle](#11-security-privacy-and-life-cycle) |
| 12 | [Backups, diagnostics and local operation](#12-backups-diagnostics-and-local-operation) |
| 13 | [Schemas, format and compatibility](#13-schemas-format-and-compatibility) |
| 14 | [Proposed repository organisation](#14-proposed-repository-organisation) |
| 15 | [Proposed ADRs](#15-architecture-decision-records-adr) |
| 16 | [Implementation plan](#16-implementation-plan) |
| 17 | [First milestone: M0](#17-first-milestone-m0--evidence-based-storage-decision) |
| 18 | [Quality, tests and definition of done](#18-quality-tests-and-definition-of-done) |
| 19 | [Risks and mitigations](#19-risks-and-mitigations) |
| 20 | [Immediate decisions and tasks](#20-immediate-decisions-and-tasks) |
| 21 | [Technical references](#21-technical-references) |
| A | [Glossary](#appendix-a-glossary) |

## 1. Purpose and scope

This document turns the initial decisions of Kerno into a technical basis to implement, test and review the project. It defines the purpose, the limits of the first version, the modules, the guarantees the engine must offer, the decisions still open and the execution plan.

Kerno will not be considered ready for adding features, but for demonstrating guarantees through reproducible tests. The first priority is that confirmed data can be recovered correctly, that transactions have a clear semantics and that formats can evolve without losing data.

### 1.1 Vision

Kerno is an embedded, local-first data engine, with transactional persistence, versioned evolution and a path of extension towards hybrid retrieval for AI applications.

The value proposition is not to implement every database model, but to coordinate consistently:

- storage and queries by key;
- structured documents and their indexes;
- revisions, differences and provenance of changes;
- textual and vector search as extensions;
- access controls, encryption, retention and export;
- failure recovery and format migration.

### 1.2 Product objectives

1. Offer an embedded engine usable from a Rust application and through a CLI.
2. Persist data on disk and define verifiable commit and recovery guarantees.
3. Keep a small, extensible key-value core; documents will be a logical layer above it.
4. Separate the recovery journal from the permanent revision history and from auditing.
5. Allow data revisions to be queried and compared when the history policy requires it.
6. Facilitate secondary indexes and hybrid retrieval without coupling the engine to an AI provider.
7. Facilitate export, migrations, diagnostics, backups and integrity tests.
8. Design security and privacy as explicit capabilities, without promising automatic compliance with any law.

### 1.3 Reference use cases

| Case | Representative load | What it evaluates |
|---|---|---|
| LiveSpec | symbols, dependencies, hashes, changes and relations | queries by key, indexes, traceability and evolution |
| Cauce | tasks, attempts, results, events and memories | transactional writes, history and recovery |
| Agent memory | documents, memories, provenance, validity and embeddings | history, filters, textual/vector search and deletion of derived data |

These cases will be evaluation workloads, not dependencies of the engine nor models that must be imposed on all users.

### 1.4 Non-objectives of the first version

- Network server, managed service or remote protocol.
- Replication, consensus, sharding or distributed deployment.
- Full SQL and its parser; the first API will be typed and programmatic.
- Columnar engine for massive analytics.
- Own query language for graphs.
- Execution of AI models inside the engine.
- Autonomous agent that decides what to remember; that logic belongs to the application.
- Branches and merges of history in the Git style.
- Guarantee that Kerno, by itself, makes any application legally compliant.

## 2. Confirmed and open decisions

### 2.1 Confirmed decisions

| ID | Decision |
|---|---|
| D-01 | Implementation in Rust. |
| D-02 | Initial delivery as a local library and CLI; no network server. |
| D-03 | One writer and several concurrent readers within the same process. |
| D-04 | Key-value core; structured documents above it. |
| D-05 | UUIDv7 for internal identity; public representation configurable. |
| D-06 | The revision history is independent of the recovery journal. |
| D-07 | Primary index first; secondary indexes, text and vectors afterwards. |
| D-08 | Embeddings through external adapters; their generation is not part of the core. |
| D-09 | Standard encryption, key management and life-cycle policies as product capabilities. |
| D-10 | Versioned persistent format, documented migrations and export independent of the physical format. |
| D-11 | Compare copy-on-write B+ tree and LSM through an ADR before freezing the physical storage. |

### <a id="22-decisions-that-must-not-yet-be-assumed-as-resolved"></a>2.2 Decisions that must not yet be assumed as resolved

- Definitive choice of the physical engine: copy-on-write B+ tree or LSM.
- Exact commit protocol, disk synchronisation and recovery.
- Binary representation of pages/records and compatibility between versions.
- Concrete model of MVCC, read snapshots and locking between processes.
- Revision format, snapshot frequency and delta algorithm.
- Default strategy for the public identifier: random token with mapping or reversible encrypted token.
- Algorithms, limits and key management for encryption at rest.
- Document serialisation, normalisation and canonical hashing.
- Semantics of derived indexes and how to declare their freshness.
- Semantics of deletion and retention against revisions, backups and derived data.
- Stable public API, semver and deprecation policy.

Each topic must be resolved in an Architecture Decision Record (ADR) before its assumptions are encoded in a way that is hard to reverse.

> **Status at version 0.2.0.** Resolved: the choice of the physical engine (**leveled LSM**, ADR-0001), the commit protocol, synchronisation and recovery (ADR-0002) and the rules of versioning and integrity (ADR-0003), all Accepted on 2026-10-28; the concrete binary representation is the [v1 format](../file-format/v1.md), which remains a **candidate for freezing** until the owner accepts [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md). The rest of the list remains open.

## 3. Architecture principles

1. **Correctness before speed.** A high benchmark does not compensate for data loss, ambiguous commits or incoherent indexes.
2. **Small core.** Advanced features are implemented in separate layers.
3. **One source of truth.** Indexes, embeddings and other derivatives never replace the source content.
4. **Explicit recovery.** Each persistent state must have recovery, validation and diagnostic rules.
5. **History with semantics.** WAL, revisions and auditing are distinct mechanisms, with different life cycles.
6. **Reproducible construction.** Formats, hashes, migrations and tests must produce deterministic results where appropriate.
7. **Security without home-made cryptography.** Established algorithms and libraries are used; Kerno defines the integration and the handling of keys.
8. **Measure before specialising.** HNSW, LSM, B+ tree, graph or other structures are justified by workloads and metrics.
9. **Portable and exportable.** The user must not be captive to the internal physical format.
10. **Explicit policies.** Retention, auditing, eventual consistency and exposure of identifiers have documented configuration and limits.

## 4. Deployment and concurrency model

### 4.1 Initial deployment

Kerno will run embedded in the consuming process and will expose a Rust library. A local CLI will allow creating, inspecting, validating, exporting and diagnosing databases. There will be no server nor network authentication in the MVP.

The library must offer clear APIs to open/close the database, start transactions, read/write, query state and export data. The CLI will use the same public library, not a second implementation of the engine.

### 4.2 Agreed concurrency

- One active writer per database.
- Several concurrent readers within the same process.
- Reads over a coherent snapshot according to the isolation model to be decided in an ADR.
- Detection of incompatible double opening of the same directory/file.
- No initial promise of multi-process writing, remote server or replication.

The internal model may use MVCC, copy-on-write, read locks or a combination; it must be resolved consistently with the chosen physical engine. It must not be inferred that "one writer" eliminates the need for atomicity, isolation or handling of unexpected shutdowns.

## 5. Logical data model

### 5.1 Main entities

- **Database:** set of files, metadata and configuration of one instance.
- **Namespace/Collection:** logical grouping for keys, documents and policies.
- **Record:** key/value pair with identity and revision metadata.
- **Document:** structured representation on top of the value; JSON will be the first convenient format, but it must not be confused with the physical format.
- **Revision:** logical version of a record.
- **ChangeSet:** grouping of changes that belong to one logical operation.
- **IndexDefinition:** versioned definition of an index.
- **DerivedArtifact:** embedding, fragment, search representation or other computed data derived from a source revision.

### 5.2 Identity

Each record will have an `internal_id` based on UUIDv7. UUIDv7 offers approximate temporal order, but it will not be Kerno's transactional order: the order of commits will be represented by a monotonic sequence assigned by the database.

The public representation will be interchangeable through an `IdentifierCodec` abstraction. The initial options to evaluate are:

- persisted opaque random token mapped to `internal_id`;
- authenticated and versioned encrypted token that allows recovering the internal ID.

The first option adds a stored correspondence; the second adds complexity of cryptographic management and key rotation. The token will be stable for each record. Bits of the UUIDv7 will not be selectively encrypted, nor will an own transformation be invented. An opaque identifier does not replace authorisation.

### 5.3 Schemas

The first version will allow structured documents without requiring a rigid global schema. Later, validation per collection, field types, declarative indexes and schema versions may be defined. Schema migrations will be separate from migrations of the physical format.

## 6. Logical module architecture

| Module | Responsibility | Prohibited dependencies or limits |
|---|---|---|
| `kerno-core` | fundamental types, errors, configuration and contracts | must not depend on an AI provider nor on the CLI |
| `kerno-storage` | pages/records, files, physical format, reading/writing | must not decide business policies |
| `kerno-transactions` | commit, isolation, writer coordination and recovery | must not depend on embeddings |
| `kerno-documents` | document serialisation and queries | must not be the only physical storage format |
| `kerno-revisions` | history, deltas, snapshots and restoration | independent of the recovery WAL |
| `kerno-indexes` | primary index, secondary indexes and rebuild state | indexes must be rebuildable from source data |
| `kerno-security` | interfaces for keys, encryption, public identifiers and policies | must not implement own cryptographic algorithms |
| `kerno-embeddings` | adapter contracts, versions and derived jobs | no mandatory network/model dependencies in the core |
| `kerno-cli` | administration, diagnostics, export and inspection | reuse the library API |

This structure is logical, not an obligation to create nine crates from day one. It is recommended to start with few crates and split them when their contracts are stable.

### 6.1 Query flow

1. The application calls the API with an internal key or a public identifier.
2. The public layer validates and resolves the identifier, where appropriate.
3. The transaction establishes the read snapshot.
4. The engine queries the primary index and retrieves the record.
5. The document or search layers apply the required filters and transformations.
6. The output applies the representation policy and avoids exposing unnecessary internal metadata.

### 6.2 Write flow

1. Validate the operation, limits and applicable policy.
2. Serialise the value and compute the integrity metadata.
3. Record the modification in the transactional mechanism defined by the storage ADR.
4. Maintain atomicity between content, revision and critical transactional structures.
5. Confirm according to the configured durability policy.
6. Mark asynchronous derived indexes as pending, where applicable, and expose their watermark/status.

No write is confirmed before meeting the durability guarantee promised by the configuration. The documentation will distinguish, for example, confirmation with forced synchronisation and less strict modes, if they end up being offered.

## 7. Physical storage, transactions and recovery

### 7.1 Main alternatives

**Alternative A: copy-on-write B+ tree.** Potentially attractive for a general-purpose embedded database, ranges, snapshots and state queries. Fragmentation, write amplification, page management and recovery of roots/commits must be studied.

**Alternative B: LSM tree.** Potentially attractive for sustained ingestion and writes. It introduces management of memtables, sorted files, tombstones, compaction and read/write amplification.

The choice remains pending ADR-0001 and must not be based only on popularity or on results published by third parties.

### 7.2 Transactional recovery

The design will have to unambiguously identify the last valid commit, detect truncated records and recover a coherent state. The implementation may use a WAL or a transactional protocol appropriate to the chosen engine; the concept of "WAL" will not be imposed if the architecture ends up using a different mechanism.

The following will be defined explicitly:

- what it means for a transaction to be confirmed;
- when the file is synchronised and what the API returns if synchronisation fails;
- how partial writes are detected;
- how shutdowns before, during and after commit are resolved;
- how an interrupted format update is recovered;
- which tool allows verifying or exporting a damaged database without overwriting it automatically.

### 7.3 Journal, history and auditing

| System | Purpose | Retention |
|---|---|---|
| Recovery journal | atomicity and recovery after failures | only the time required by the protocol, unless another need is documented |
| Revision Engine | rebuild and compare historical states | configurable according to policy |
| Auditing | record operations and security context | configurable and minimised |

These systems may share commit sequences or metadata, but they will not implicitly share the same retention policy.

## 8. Revision Engine: history, differences and provenance

> **Status (2026-11-04):** implemented in Stage 3, in the crate `kerno-units` and not in `kerno-storage`. The stage is closed ([closure of Stage 3](stage3-closure.md)). What was built: full-copy revisions (§8.2 and §8.3; deltas are not implemented, decision 4 of Stage 3), `restore` that adds a new revision (§8.3, I-22), policies `none`/`audit`/`versioned` per type (§8.4) and batched retention within the policy. Physical suppression (ADR-0009, K-E3-6) and structural difference (ADR-0008) remain out of scope. Open risks: **K-E3-8** (U2: `history` reaches 1.50× the reference in one pass of three) and **K-E3-9** (U6: write p99 and throughput with concurrent pruning). Both are documented failures that the owner accepted on 2026-11-04, without lowering thresholds.

### 8.1 Objective

Allow reconstructing earlier versions of the data, comparing two revisions and recording the context of changes without turning the recovery journal into a permanent history.

### 8.2 Revision model

A revision may include:

- `revision_id` and `record_id`;
- `parent_revision` or equivalent reference;
- `commit_sequence`;
- hash of the canonical content, where applicable;
- schema version;
- reference to snapshot or delta;
- type of change: creation, modification, logical deletion or restoration;
- actor, source, reason or reference to a task, if available and allowed by the policy.

The absence of actor or reason must be valid; context will not be invented.

### 8.3 Revision storage

A hybrid model of full snapshots and structural deltas will be evaluated. Depth and size limits will be fixed to avoid a chain of deltas that is too costly. Snapshots may be rebuilt/compacted without altering the semantics of the history.

Restoring a revision will produce a new revision equivalent to the selected historical content; it will not silently delete later revisions.

### 8.4 History policies

- **none:** keeps no logical versions; the guarantees necessary for transactional recovery are maintained.
- **audit:** keeps defined events/metadata, not necessarily the full previous content.
- **versioned:** keeps revisions that can be queried and compared within the configured retention.

The life cycle of the history must consider personal data, backups, indexes, embeddings, caches and snapshots. Unlimited immutability is not promised, nor that every deletion can be reflected instantly in every secondary medium.

## 9. Indexes and query engine

> **Status (2026-11-05):** Stage 4 implemented, in `kerno-units`, the reference path without indexes, the primary index (order 1 of §9.1) and a single ordered secondary index, `IDX_TYPE` by `(unit_type, schema_version)`, justified by a measured workload (W1, [stage4-workloads.md](stage4-workloads.md)); no index is created without a workload. Declared relations use two adjacency indexes that are written in the same atomic boundary as the relation (§9.2, I-21; ADR-0014). Unit indexes are derived (ADR-0007): `rebuild_index` writes them in its own `ChangeSet`, they expose `ready`/`building`/`stale`/`failed` and their watermark (I-23), a rebuilt index equals a new one (I-24), and `query` answers through the reference path when the index is not ready, without pretending freshness (I-32). Derivatives are maintained until a workload with interleaved writes justifies keeping them inside each `ChangeSet` (decision of the owner, 2026-11-05, ADR-0007). The closure of the stage was approved by the owner on 2026-11-05 ([stage4-closure.md](stage4-closure.md)).
>
> **Status (2026-11-06, Stage 5, pending owner approval):** `query` and `query_relations` go through a deterministic planner of the seven steps of §9 over the paths that already existed (reference, primary index, `IDX_TYPE` and adjacency). It chooses the usable one of lowest cost with the statistics the layer already has, validates the answer and, if the index fails or answers wrongly, answers with the reference path and records it. `explain_filter` and `explain_relations` show the plan without literals (I-37). The observations (`ExecutionObservation`) are disabled by default, live in memory with bounded retention, do not write to the engine and do not change the commit sequence (observations part of I-34). The learned optimisations and the cache belong to Stage 6. Closure: [stage5-closure.md](stage5-closure.md).

> **Status (2026-11-11, Stage 6, closed pending owner approval):** the only learned optimisation is a result cache by query footprint and `commit_seq` (G4), **disabled by default**. `query` and `query_relations` consult it before the planner: a hit goes through `check_units`/`check_relations`; any failure, invalidation, deactivation or discrepancy executes the planner or the reference path and records it in the cache's transition log (G7). The observations do not decide whether an entry is used (G3). The net improvement is not claimed: the p50 of W1 meets it (−97.8%), the p99 of W1 is not certified. Closure: [stage6-closure.md](stage6-closure.md).

> **Status (2026-11-12, Stage 7, closed and approved by the owner on 2026-11-12):** the associative queries (`assoc_query`, `indexed_assoc_query` and `reference_assoc_query`) walk typed relations over several hops with hard limits (depth 4, 1,000 results and 100,000 visits). The planner chooses between the adjacency index and the reference path, checks each answer of the index and records the fall-back; the result carries its verifiable trail (I-40). The ranking (`assoc_rank`) is off by default: with its weights it stays below the simple order "hops, then identifier" in nDCG@10 (0.912 against 0.942) on the evaluation set. The vectors are supplied by the caller, in memory; their quality has not been measured. There is no approximate vector index and no derivative is persisted. The index limit H-1 (an entry absent at both ends of a relation, with 64 relations or more) remains open. Closure: [stage7-closure.md](stage7-closure.md).

### 9.1 Implementation sequence

| Order | Structure | Main case | Entry condition |
|---|---|---|---|
| 1 | primary index | search by key/ID | MVP |
| 2 | ordered secondary indexes | equality, ranges, ordering and filters | after validating transactions and storage |
| 3 | composite indexes | combinations of frequent predicates | backed by benchmarks |
| 4 | inverted index with textual ranking | term queries and relevance | search phase |
| 5 | exact vector search | similarity reference and recall tests | first AI extension |
| 6 | HNSW or other ANN | low latency at scale | only after measuring the cost of exact search |
| 7 | specialised relations/graph | relation traversals | only if workloads justify it |
| — | columnar storage | massive analytical aggregations | outside the initial direction |

Not all indexes will be created automatically. Each index consumes disk, memory and maintenance work; the definition will be explicit and, in the long term, may inform the query optimiser.

### 9.2 Index consistency

The indexes needed for a transaction to be observed coherently will be updated in the same atomic boundary. Costly and rebuildable indexes may be derived and asynchronous, but they will have a visible state (`ready`, `building`, `stale`, `failed`) and a synchronisation sequence.

A query must declare or inherit the freshness policy. If the index is behind, Kerno must apply a documented strategy, such as waiting, querying an alternative exact path or returning an unavailability status for that query; it must not pretend freshness.

### 9.3 Query API

The MVP will have a typed API for keys and document operations. SQL is not in the initial scope. Query limits, cursor pagination, cancellation and typed errors will be designed from the start. A future `EXPLAIN` will show the indexes used, filters, observed costs and whether a derived structure took part.

## 10. Embeddings and hybrid retrieval for AI

### 10.1 Separation of responsibilities

Kerno will store vectors and offer search contracts; it will not download models, will not require internet access and will not generate embeddings in the core. An optional adapter will allow local or external providers.

### 10.2 Metadata of an embedding

At a minimum: document ID, exact source revision, embedding profile, provider/model and version, dimensions, numeric type, distance metric, chunking profile, status and synchronisation mark.

### 10.3 Life cycle

- **Document change:** mark the previous embedding as obsolete.
- **Model or chunking change:** create a new profile and allow gradual rebuilding.
- **Provider failure:** keep the source, mark the failed job and allow idempotent retries.
- **Deletion:** apply the policy also to vectors, chunks, indexes and pending queues.
- **Index change:** be able to rebuild it from source embeddings that remain valid.

### 10.4 Hybrid search

The planned evolution is textual search (e.g. BM25) + vector search + structured filters. Result fusion (e.g. Reciprocal Rank Fusion) will be an option of the query layer, not an obligation of the physical storage. Exact vector search will be implemented first as a baseline; approximate indexes must be measured against it in recall, latency, memory and disk.

### 10.5 Derived data as built (Stage 8, version 0.3.0)

This subsection describes the code of Stage 8. What is not built is said at the end.

**Common registry (ADR-0018, option 2).** Each derivative has a descriptor with its exact sources (unit and revision), its method, its version, its parameters, its model profile (opaque text, 1 to 64 bytes) and the hash of its input. The identity of the derivative is a hash of all that (`DerivedDescriptor::id`); the hash of the output and the date are not part of the identity. A change of method, version, parameters or profile gives a new identity. The registry lives in memory in `DerivedRegistry` and carries an inverse index source → derivatives.

**Obsolescence.** A `ChangeSet` that writes a unit marks `stale` in the same commit every derivative that has that unit as a source with a revision different from the head. The marking is done inside the commit for persisted data, and after the commit, under the same lock of the registry, for data in memory. A `stale` derivative is not served. Rebuilding (`rebuild`) produces the same as building again over the same sources; if the sources changed, the identity is another one.

**Persisted vectors (ADR-0020).** Search vectors are stored as derivatives in the units layer, under three keys: descriptor (`0x60`), vector (`0x61`) and inverse index source → derivative (`0x62`). A vector, its descriptor and its index entry are confirmed in the same transaction, and only if the source revision is the head in that transaction. The database declares layer 2 in the `LAYER_META "layer"` mark in the first `ChangeSet` that persists a derivative, and never lowers it. A layer-1 build rejects that database on opening (tested with the layer-1 reader in `layer1_reader_refuses_derived_fixture`; the check with an earlier binary is not verified, F-11 of the closure). Databases without vectors remain at layer 1 and are opened by any build of layer 1 or 2. The engine and the frozen v1 format do not change.

**Opening.** `DerivedDb::open_persisted` rebuilds the registry from the saved descriptors, without writing anything. It recalculates the identity of each descriptor, crosses the inverse index with the descriptors and checks that the head of each source has the revision and the input hash that the descriptor declares; if not, the derivative becomes `stale`. A vector whose output hash does not match is not served. The repair is an explicit call (`repair_derived`) that writes a `ChangeSet` with purpose `Derived`; opening never repairs.

**Suppression (ADR-0009, option 3).** Deleting a unit, or purging a revision by retention, withdraws in the same `ChangeSet` the descriptors, vectors and index entries of the derivatives that have it as a source, even when they have other sources. After the commit, no new read returns that data. A snapshot opened earlier still sees it until it is released. The bytes may remain in live files until a compaction that covers their range; there is no promise of physical deletion. Deleting a relation deletes no derivative, because no derivative has a relation as a source. On opening or after a retention, a derivative whose source no longer exists is discarded and reported.

**Summaries (E8-06).** A summary is a derivative with a reserved parameter (`kerno.kind = summary`) and at least one source. It is produced by a `Summarizer`, a trait with no network or model; the crate includes only a deterministic one, for tests. Summaries live **in memory only** and write no byte. No unit read returns a summary instead of a source: `get_unit` has no path to the text of a summary, and `summary()` serves only one in state `ready`. A summary is reopened with `open_summaries`, which recalculates only if its sources are still the same.

**What is not built.** Approximate index (ANN); textual search and hybrid fusion; real summariser; inferred relations as a source; physical deletion; cryptographic erasure (ADR-0006, pending); backup function; persistence of summaries; search over persisted vectors without copying them into memory on each call (friction of the use case, point 9). The measurement and test limits are in [closure of Stage 8](stage8-closure.md) §3 and §5.

## <a id="11-security-privacy-and-life-cycle"></a>11. Security, privacy and life cycle

### 11.1 Limits of responsibility

The embedded library offers mechanisms and contracts; the host application remains responsible for the identity of its users, business authorisation and the legal basis for processing data. A future Kerno server will need its own authentication controls, TLS, tenant isolation and administration.

### 11.2 Public identifiers

`IdentifierCodec` will be interchangeable. The public token will be stable, versioned and cryptographically validated if a reversible encrypted format is chosen. Key management and rotation will be defined before that format is published. It will not be assumed that the difficulty of guessing an ID grants authorisation.

### 11.3 Encryption at rest

Established cryptographic libraries and authenticated encryption will be used where appropriate. Master keys will not be stored next to the encrypted data. It will be defined which files are protected: data, journal, snapshots, temporaries and backups. Cryptographic erasure will be considered a strategy dependent on the key design, not a universal promise.

### 11.4 Auditing

Events may record actor/reference, operation, resource, result, timestamp, sequence and correlation. The full content of documents, secrets, tokens or personal payloads will not be recorded automatically. Auditing will have its own limits, redaction and retention policy.

### 11.5 Privacy by design

Mechanisms for export, rectification, suppression, classification and retention will be designed. The engine will make it easy to apply policies, but it will not automatically decide the legal basis, the purpose or the exceptions to retention. Normative profiles will map requirements to controls and tests; they will not amount to certification or automatic legal compliance.

## 12. Backups, diagnostics and local operation

Kerno must offer utilities to:

- create consistent backups without copying files in an ambiguous state;
- verify integrity and version compatibility;
- restore to a separate directory and validate the result;
- export data to a documented logical format;
- inspect format versions and pending migrations;
- show statistics on files, indexes, revisions and space;
- inspect the state of derived indexes and rebuild them;
- diagnose damage without running destructive repair automatically.

A backup restoration will be a tested operation, not a promise based only on the existence of a copy file.

## 13. Schemas, format and compatibility

Kerno will have three separate version axes:

1. **Physical format:** pages, records, headers and commits.
2. **Logical schema:** structure and constraints of the documents.
3. **Indexes/derived data:** algorithms, versions and build parameters.

Every migration will have preconditions, execution steps, subsequent validation and a documented restoration path from a backup. It will be defined which versions each release can open and when it requires an explicit migration.

Content hashes need a canonical serialisation when they must be comparable between runs or versions. For JSON, the use of RFC 8785 (JCS) will be evaluated, taking into account its restrictions on numeric types and Unicode; no silent ad hoc normalisation will be applied.

The logical export will have a versioned format independent of the internal pages. The user will be able to migrate data without having to understand the physical tree, the LSM segments or the private encoding of the engine.

## 14. Proposed repository organisation

```text
kerno/
├── Cargo.toml
├── crates/
│   ├── kerno-core/
│   ├── kerno-storage/
│   ├── kerno-transactions/
│   ├── kerno-documents/
│   ├── kerno-revisions/
│   ├── kerno-indexes/
│   ├── kerno-security/
│   ├── kerno-embeddings/     # optional in the initial stage
│   └── kerno-cli/
├── docs/
│   ├── architecture/
│   ├── adr/
│   ├── file-format/
│   ├── security/
│   ├── testing/
│   └── roadmap/
├── tests/
│   ├── recovery/
│   ├── transactions/
│   ├── revisions/
│   ├── corruption/
│   └── workloads/
└── benchmarks/
```

The final structure of crates will be reviewed after the contracts have stabilised. The first prototype must prefer simplicity and few dependencies, not premature fragmentation.

> **Status at version 0.2.0.** There are three product crates: `kerno-core` (contract, errors, `Vfs`; no external dependencies), `kerno-testkit` (harness, simulators and reference engine; `publish = false`, never a runtime dependency of the engine) and `kerno-storage` (the LSM v1 engine; explicitly unstable API). The other crates in the list are the proposal of 0.1.0 and do not exist. The M0 prototypes remain in `spikes/storage/` (`spike-btree` and `spike-lsm` frozen as the performance baseline, `spike-bench` as the test bench). The CI guards that protect these limits are in [CONTRIBUTING](../../CONTRIBUTING.md#local-checks).

## 15. Architecture Decision Records (ADR)

The ADRs describe durable decisions and their consequences. The following are proposed; their initial status is *Proposed / pending approval*, except the points explicitly confirmed in section 2. The current status of each one is kept in [`docs/adr/`](../adr/README.md).

**ADR-0001 — Physical storage engine.** Status: **Accepted** (2026-10-28): leveled LSM.
Question: copy-on-write B+ tree or LSM?
Options: own implementation of a copy-on-write B+ tree; own implementation of an LSM; evaluate a third option only if the evidence justifies it.
Proposal: compare a prototype of each approach with equal workloads, including point, range, update, history and recovery.
Criteria: correctness, latency p50/p95/p99, throughput, recovery time, maximum RAM, bytes written/data size and operational complexity. Do not fix a winner by external benchmarks.
Consequence: this decision must be closed before the physical format is frozen.

**ADR-0002 — Transaction and durability model.** Status: **Accepted** (2026-10-28).
Question: which protocol defines commit, synchronisation, snapshots and recovery?
Options: WAL with a commit protocol; copy-on-write with a commit root/manifest; a variant appropriate to the selected engine.
Approval criterion: a failure matrix that defines each interruption point and the expected state after reopening. The API must document which guarantees each durability mode offers.

**ADR-0003 — Physical format and versioning.** Status: **Accepted** (2026-10-28); the concrete format is frozen with the acceptance of ADR-0013.
Question: how are headers, pages, records, checksums and commits versioned?
Proposal: explicit versioning from the first database; safe rejection of incompatible versions; independent logical export.
Criteria: detection of truncation/corruption, proven compatibility and migration with simulated interruption.

**ADR-0004 — Revision history.** Status: Pending (preliminary design available).
Question: what does a revision represent and when to use snapshot/delta?
Proposal: Revision Engine separate from the journal; optional ChangeSet; snapshot + delta with a depth limit; restoration that creates a new revision.
Criteria: exact reconstruction, deterministic diff and configurable retention.

**ADR-0005 — Public identifiers.** Status: Proposed (draft).
Question: random token with mapping or reversible encrypted token.
Proposal: `IdentifierCodec` abstraction; compare the cost of persisting a correspondence against key management, rotation, length and validation.
Criteria: stability of the token, rejection of tampering, support for versioning and clarity of the separation between identification and authorisation.

**ADR-0006 — Encryption and key management.** Status: Pending.
Question: scope of file encryption and how keys are administered.
Proposal: standard algorithms and an audited library; keys outside the data directory; key-provider interface and a defined rotation strategy.
Criteria: data, journals, temporaries and backups covered; loss of key documented; integrity and rotation tests.

**ADR-0007 — Index consistency and rebuilding.** Status: Pending.
Question: which indexes participate in transactions and which are derived.
Proposal: primary index consistent with the commit; critical secondary indexes with a clear atomic boundary; costly indexes with a freshness status and rebuilding.
Criteria: rebuilt indexes produce results equivalent to a new index over the same data.

**ADR-0008 — Document serialisation and hashes.** Status: Pending.
Question: initial JSON, binary support and canonicalisation for hashing.
Proposal: separate logical and physical format; evaluate RFC 8785 for JSON when its restrictions are acceptable; version the hash algorithm.
Criteria: reproducible results in the supported cases and compatibility tests.

**ADR-0009 — Suppression, retention and backups.** Status: Accepted on 2026-11-12 (option 3, under the terms of [the ADR](../adr/0009-suppression-retention-and-backups.md)).
Question: how to coordinate logical/physical deletion, revisions, indexes, embeddings and backups.
Proposal: explicit policies per collection; suppression operations aware of derived data; documentation of backup expiry windows.
Criteria: an end-to-end test shows what is removed immediately, what remains subject to compaction and what deadline backups follow.

**ADR-0010 — Public API and crate limits.** Status: Pending.
Question: which types and errors are public and which modules are internal.
Proposal: small, typed and stable API; do not make page or physical-engine details public; separate modules only when their contracts are clear.
Criteria: an external application example that compiles; error guide; deprecation and semver policy.

**ADR-0011 — Vector profile and hybrid retrieval.** Status: Pending.
Question: which common contracts define embeddings, metrics, filters and index freshness.
Proposal: exact vectors first; model and source revision metadata; generation through adapters; ANN only with evidence.
Criteria: comparison with an exact baseline, verification of dimensions/metric and an invalidation test after updating/deleting the document.

**ADR-0012 — Benchmark methodology and output thresholds.** Status: Proposed (applied in M0 and in the Phase 1 output measurement).
Question: how to measure without declaring success from a single synthetic test.
Proposal: reproducible datasets and scripts; same machine, same data and equivalent durability; report p50/p95/p99, throughput, RAM, disk and recovery.
Criteria: repeatable results and publication of environment, version, configuration and limitations.

**ADR-0013 to ADR-0018** were not in the list of 0.1.0. ADR-0013 (format v1 and engine protocol) arises in Phase 1 and is a requirement for freezing; ADR-0014…0018 (relations, observability privacy, adaptation, neural unit, derivatives) come from the [specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §17. The current status of all of them is in [`docs/adr/`](../adr/README.md).

## <a id="16-implementation-plan"></a>16. Implementation plan

### Phase 0 — Architectural spike and decisions (1–2 weeks)

Objective: resolve the storage uncertainty before freezing the format.

Deliverables:

- ADR-0001 with a documented decision or an explicit recommendation to change the approach;
- small, comparable prototypes for copy-on-write B+ tree and LSM, without pretending they are production engines;
- benchmark harness and synthetic/representative datasets;
- proposed failure matrix and commit guarantees;
- initial ADRs for persistence, transactions, format and identity;
- list of invariants and risks.

Output: decision revised by evidence. If the prototype does not justify its own complexity, record a temporary no-go and reconsider the scope before writing the full engine.

### Phase 1 — Persistent core (3–5 weeks, initial estimate)

- opening/closing and locking compatible with one process;
- key/value storage;
- commits and recovery;
- integrity check;
- corruption and failure tests.

Output: experimental engine that demonstrates persistence invariants and passes the initial recovery matrix.

### Phase 2 — Transactions and documents (3–5 weeks)

- typed API and batch operations;
- structured document over key-value;
- isolation of concurrent reads;
- primary index and basic filters;
- initial CLI.

Output: local library that an external test project can use without accessing internal details.

### Phase 3 — Versioning (3–5 weeks)

- Revision Engine, ChangeSet, snapshots and deltas;
- history/diff/restore;
- none/audit/versioned policies;
- rebuilding and retention limits.

Output: reproducible history with atomicity documented with respect to the current record.

### Phase 4 — Security and life cycle (3–5 weeks)

- `IdentifierCodec` and initial policy for public IDs;
- encryption and key provider;
- minimised auditing;
- export, suppression and backups;
- migrations and diagnostics.

Output: security review and life-cycle tests completed for the embedded scope.

### Phase 5 — Search and AI (4–6 weeks)

- secondary indexes;
- textual search;
- exact vectors and embedding adapter;
- update/invalidity of derivatives;
- initial hybrid ranking.

Output: hybrid retrieval benchmark and freshness/suppression tests.

### Phase 6 — Hardened beta (4–8 weeks)

- fuzzing and extended failure tests;
- format/migration compatibility;
- operation and recovery documentation;
- comparative benchmarks with SQLite and reference engines;
- tests with LiveSpec, Cauce and agent-memory workloads.

Output: documented beta with limitations, compatibility matrix and explicit criteria for recommended use.

The estimates are initial ranges for one person with regular development. They are not calendar guarantees. A basic prototype may appear earlier; a safe and reliable beta may require six to twelve months or more, and preparation for critical systems requires additional validation.

## 17. First milestone: M0 — evidence-based storage decision

### 17.1 Objective

Do not yet write the complete engine. Choose the storage approach and record the guarantees that the implementation must satisfy.

### 17.2 Scope

- Minimal prototype of key access and persistent writing for each candidate alternative.
- Benchmarks of reads/writes by key, updates, range scans and mixed pattern.
- Estimate of RAM, disk size and reopening/recovery time.
- Prototype of versioning of changes at design level, without yet building the complete Revision Engine.
- Tests of simulated interruptions at key points of the candidate protocol.
- Comparison with a relevant existing alternative as a reference, recording that figures will not be compared if the durability guarantees are different.

### 17.3 Minimum workloads

1. **Point:** insert, get, update and delete with random keys.
2. **Append-heavy:** sequential insertion and successive updates.
3. **Mixed:** predominance of reads and a fraction of writes defined in the benchmark.
4. **Range:** ordered reading of key intervals.
5. **Reopening:** time to reopen and rebuild the index/state.
6. **Simulated history:** several revisions per record to estimate the future cost of versioning.

The tests must include datasets that fit in memory and larger datasets if the environment allows it. The exact size is recorded as a parameter, not as a universal condition.

### 17.4 Measurement method

- Same machine, operating system, build and configuration for each candidate.
- Separate warm-up from measurements and run several repetitions.
- Report throughput, latencies p50/p95/p99, maximum RAM, disk size and recovery time.
- Record versions, flags, datasets, sync configuration and known limits.
- Do not compare strong durability of one option with relaxed durability of another as if they were equivalent.
- Do not set a speed threshold before measuring the baseline; define objectives after knowing the results.

### 17.5 Acceptance criteria

M0 is considered finished only if:

1. ADR-0001 selects an approach or documents why no alternative should yet be implemented.
2. A reproducible report is published with code, commands, configuration and results.
3. A matrix of failure points and expected state after recovery exists.
4. The exact meaning of the proposed commit and durability is documented.
5. The options are also compared by complexity, recovery, compaction, ranges and future compatibility with the Revision Engine.
6. The decisions that depend on the choice of storage and which remain decoupled are identified.
7. There is no mandatory functional dependency on AI, network or external provider.

### 17.6 Minimum M0 tests

- The same sequence of operations produces the same logical state in each candidate.
- Insert/update/delete/reopen recovers the expected state in normal runs.
- The prototype detects or rejects truncated entries/files according to the design tested.
- Failure tests run at predefined points and are compared with the expected matrix.
- The benchmarks can be repeated by a single documented command.
- The results include at least one range measurement, one recovery measurement and one memory/disk measurement.

M0 does not certify the correctness of the future engine; it reduces the risk of an early architectural decision and leaves the first contracts ready to be implemented.

## <a id="18-quality-tests-and-definition-of-done"></a>18. Quality, tests and definition of done

### 18.1 Test pyramid

- Unit tests for serialisation, keys, indexes and policies.
- Property tests over random sequences of operations.
- Integration tests of transactions and recovery.
- Crash testing: interruption of processes at write and commit points.
- Fuzzing of inputs, document parser and binary format.
- Compatibility/migration tests from earlier versions.
- Benchmarks with defined workloads and versioned results.

### 18.2 Definition of Done

A feature is not considered finished until:

- the API and error contract is documented;
- the affected invariants have been defined;
- the relevant normal and failure tests pass;
- the effects on format, migration, history, indexes and backups have been evaluated;
- the consequences for security and privacy have been evaluated;
- performance metrics are available where appropriate;
- the related ADRs are updated;
- the CLI/documentation show the real behaviour, without untested guarantees.

### 18.3 Release gates

Do not publish a beta if there is a reproducible loss of a confirmed operation under the promised durability configuration, an untested recovery path, a destructive migration without a validated backup, or a known discrepancy between indexes and source information that is hidden from the consumer.

## 19. Risks and mitigations

| Risk | Consequence | Mitigation |
|---|---|---|
| Excessive scope | The core is never finished | Bounded MVP and extensions by evidence |
| Choosing B+ tree/LSM badly | Redoing format and recovery | M0 with ADR and comparable prototypes |
| History too costly | High disk use or slow reads | snapshots, delta limits, retention and benchmarks |
| Stale derived indexes | Incorrect results | explicit status, watermark and rebuilding |
| Badly designed encryption | Data loss or false security | consolidated libraries, key management and specific review |
| Incomplete deletion | Unintended retention | policy covering revisions, indexes and backups |
| Format without versioning | Dangerous migrations | version from the first database and logical export |
| Unrepresentative benchmarks | Wrong decisions | workloads of LiveSpec, Cauce and agent memory |
| Unstable API | Maintenance cost | small public surface, semver and ADR |
| Excessive legal promises | Contractual and reputational risk | map controls to requirements; legal review of the use case |

## 20. Immediate decisions and tasks

The next order of work will be:

1. Create the repository, the chosen licence and the contribution conventions; do not copy implementation from other projects without reviewing licences.
2. Add this document as the architecture baseline and create the `docs/adr/` folder.
3. Write ADR-0001 and ADR-0002 as drafts with hypotheses and decision criteria.
4. Build the M0 test and benchmark harness.
5. Run the B+ tree versus LSM spike and record reproducible results.
6. Approve the format and the commit semantics before creating user databases.
7. Begin the implementation of the persistent core only after closing M0.

**Outside the first batch of commits.** Do not start with HNSW, embeddings, SQL, a graph language, a network server or an agent system. Nor commit format compatibility before there are tests of opening, recovery and migration.

## 21. Technical references

The following references are study bases; they do not replace the ADRs nor oblige copying their architectures.

1. RFC 9562 — Universally Unique IDentifiers (UUIDs). Defines UUIDv7 and its time field. <https://www.rfc-editor.org/rfc/rfc9562.html>
2. RFC 8785 — JSON Canonicalization Scheme (JCS). Reference for a deterministic JSON representation in hashing/signing uses, subject to the limitations of the standard. <https://www.rfc-editor.org/rfc/rfc8785.html>
3. redb — Rust embedded database. Architecture reference for embedded databases, copy-on-write B+ trees and transactions. <https://www.redb.org/>
4. RocksDB Wiki — Overview / Architecture. Reference for the LSM approach, SSTables, memtables and compaction. <https://github.com/facebook/rocksdb/wiki/RocksDB-Overview>
5. NIST SP 800-88 Rev. 2 — Guidelines for Media Sanitization. Reference for sanitisation and cryptographic erasure policies, depending on applicability. <https://csrc.nist.gov/pubs/sp/800/88/r2/final>
6. OWASP Application Security Verification Standard (ASVS). Catalogue of verifiable requirements for applications and services that may be built on Kerno. <https://asvs.owasp.org/>

## Appendix A. Glossary

- **ADR:** brief record of an architecture decision, its context and consequences.
- **ANN:** approximate nearest-neighbour search.
- **BM25:** ranking function for textual retrieval.
- **ChangeSet:** set of changes that form a logical operation.
- **Delta:** representation of differences with respect to an earlier revision.
- **Embedding:** vector generated by a model from a content.
- **LSM tree:** structure that organises writes in memory and sorted segments on disk, with compaction.
- **MVCC:** concurrency control that keeps versions visible to different read snapshots.
- **Snapshot:** stable representation of a state at a given point.
- **WAL:** journal that records operations before applying/confirming changes, according to a recovery protocol.
- **Watermark:** sequence that identifies up to which commit or revision a derived structure is up to date.
