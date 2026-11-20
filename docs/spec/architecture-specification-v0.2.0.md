# KernoDB --- Architecture specification and guide for Claude Code

**Version:** 0.2.0\
**Status:** design proposal; validate against the real repository
before implementing.\
**Purpose:** to give Claude Code the product context, the
architectural principles and an incremental work plan.

## 1. Mandatory instructions for Claude Code

Do not assume that this document describes features already
implemented. First inspect the repository, local instructions, Git
status, code, tests, dependencies and documentation. Do not overwrite
the user's changes or do a massive rewrite.

If they are available, also review these reference repositories and
reuse ideas only when there is a technical justification: -
https://github.com/Rixmerz/livespec - https://github.com/Rixmerz/cauce

Deliver a diagnosis with evidence from the repository first. Distinguish
facts, hypotheses and proposals. Then propose a plan of small tasks
with tests and acceptance criteria. Implement the first safe vertical
task if the environment allows progress; if it requires approval, wait.
Do not claim that a function exists without verifying and testing it.

Priorities, in order: 1. Data integrity and recovery. 2.
Transactional correctness and index consistency. 3. Simplicity and
testability. 4. Traceability and observability. 5. Measured performance.
6. Evidence-based adaptation. 7. Advanced AI features.

Do not depend on an LLM for basic storage operations, transactions or
failure recovery. The system must be correct even when no AI model is
available.

## 2. Product vision

KernoDB seeks to be a persistent, evolutive and adaptive data engine
inspired by functional principles of the brain. It does not intend to
replicate brain anatomy literally nor claim biological equivalences.

**Value proposition:** a data engine that keeps the evolution of the
information, represents connections between units of knowledge and
learns to run frequent queries more efficiently without sacrificing
correctness, security or traceability.

The proposal must be validated against consolidated alternatives.
Integrating history, relations, semantic search and adaptation is not
automatically better than using SQLite or another mature engine.

### Non-negotiable principles

-   Confirmed data must survive the failures covered by the documented
    guarantees.
-   An optimization may not silently change the meaning of a query.
-   Learning may not skip integrity, authorization or transactions.
-   Adaptation must be inspectable, measurable, disableable and
    reversible.
-   There must be a correct execution path even when no optimization
    has been learned.
-   Reads do not mutate data by surprise.
-   Persistent format and migrations are versioned.
-   Retention and deletion consider revisions, indexes, caches,
    vectors, logs and backups.
-   Each mechanism inspired by the brain needs a use case and a value
    test.

## 3. Brain model as inspiration

The metaphor helps to design specialization, connections, route
selection, memory and learning. It does not oblige creating one engine
per lobe nor one physical table per neuron.

  -------------------------------------------------------------------------
  Functional inspiration    Abstraction in KernoDB  Responsibility
  ------------------------- ----------------------- -----------------------
  Frontal executive         Planner/executor        Choose a valid strategy
  function                                          for a query

  Memory and association    Memory engine           Retrieve units,
                                                    episodes, revisions and
                                                    associations

  Contextual integration    Contextual engine/      Combine signals and
                            ranking                 sort results

  Perceptual processing     Ingestion adapters      Convert text, images or
                                                    other inputs into
                                                    representations

  Cerebellum/automation     Path optimizer          Reuse correct plans and
                                                    reduce repeated work

  Hippocampus/consolidation Consolidation          Group events and create
                            pipeline                derived representations
                                                    with provenance

  Action selection          Strategy selector       Choose among candidate
                                                    routes by cost and
                                                    utility

  Synaptic plasticity       Update of statistics/   Adjust priorities
                            weights                 according to evidence
  -------------------------------------------------------------------------

These correspondences are design hypotheses, not exact scientific
equivalences. Modules must be defined by technical contracts and use
cases, not by anatomical names.

## 4. Conceptual architecture

1.  **Data core:** identity, schema, state and lifecycle.
2.  **Transactional persistence:** physical storage, journal/WAL or
    alternative, recovery and durability.
3.  **Revision engine:** history, snapshots/deltas, diffs, restoration
    and provenance.
4.  **Relation engine:** typed connections between units.
5.  **Indexes and retrieval:** primary key, filters, text and vectors
    when appropriate.
6.  **Planner/executor:** selection and execution of plans.
7.  **Operational adaptation:** statistics, caches, reusable plans and
    invalidation.
8.  **Consolidation:** derived artifacts with references to their
    sources.
9.  **Security and lifecycle:** permissions, encryption, retention,
    deletion and auditing.
10. **SDK/CLI:** public API, diagnostics, import and export.

The first version may implement several responsibilities within a few
modules. Do not create many crates or services before the interfaces
are stabilised.

## 5. MVP scope

### Include

-   Embedded and local engine initially.
-   Persistent units with stable IDs.
-   Transactional CRUD operations.
-   Typed relations between units.
-   Queryable revisions, comparison and restoration.
-   Queries by ID, basic filters and relations.
-   Query plans and execution metrics.
-   Correct reference path and fallback mechanism.
-   One simple operational optimization that can be measured and
    disabled.
-   Documented export/import.
-   Recovery after unexpected shutdowns, within explicit guarantees.

### Do not include initially

-   Distributed cluster or multi-region replication.
-   Full SQL or full compatibility with PostgreSQL.
-   One physical engine per brain lobe.
-   LLM inference inside the core.
-   Training of our own neural networks.
-   Approximate vector search before having benchmarks.
-   Complex multimodal processing.
-   Autonomous schema changes or deletions without an explicit policy.
-   Large-scale columnar analytics.

## 6. Conceptual data model

### 6.1 Unit

A unit is a persistent entity: document, memory, event, fact or
resource. Conceptual fields:

-   `unit_id`: stable internal identity.
-   `unit_type`: logical type.
-   `schema_version`: schema version.
-   `current_revision_id`: current revision.
-   `created_at`, `updated_at`.
-   `lifecycle_state`.
-   `metadata`.
-   `content_ref`.

The logical representation does not oblige storing everything in a
single physical structure.

### 6.2 Revision

Suggested fields:

-   `revision_id`, `unit_id`, `parent_revision_id`.
-   `commit_sequence`.
-   `content_hash`.
-   `schema_version`, `change_type`.
-   `payload_ref`.
-   `actor_ref` or `source_ref`.
-   `change_set_id` and optional reason.
-   `created_at`.

Rules: - Restoring a revision creates a new revision; it does not
silently rewrite the past. - Use snapshots/deltas with limits to avoid
excessive chains. - Distinguish the logical history from the revisions
of the WAL/recovery journal. - Define transactional visibility and
consistency of revisions.

### 6.3 ChangeSet

Groups the changes of a logical operation. It may contain ID, commit
sequence, actor/source, reason, date, references to revisions and
correlation metadata. It makes it possible to answer what changed, why
and as part of which operation.

### 6.4 Edge / relation

Suggested fields: `edge_id`, `from_unit_id`, `to_unit_id`,
`relation_type`, optional weight, optional temporal validity,
timestamps, provenance and attributes. Do not assume that all relations
are directed or have a weight. Distinguish declared relations from
inferred ones.

### 6.5 ExecutionObservation

Record, if enabled: normalised query fingerprint, plan, duration, rows
examined/returned, result, cache hit, fallback and timestamp. Avoid
storing sensitive query values in metrics by default.

## 7. Identity, privacy and security

Separate internal IDs from public references. UUIDv7 may serve as an
internal ID, but it reveals approximate time and is not a security
token. For public IDs, use opaque tokens generated with reviewed
mechanisms; authorization must never depend on an ID being hard to
guess. Do not invent our own cryptography.

If there is encryption at rest, document key management/rotation and
recovery after loss. Consider the WAL, snapshots, indexes, embeddings,
caches, logs, exports and backups. Logical and physical deletion must
have explicit semantics. If complete deletion is promised, document how
it applies to derived data and backup copies.

## 8. Persistence and transactions

Pending decision: compare a copy-on-write B+ tree, a B+ tree with WAL,
an LSM tree and reusing a consolidated embedded engine for the
prototype. Choose by ADR and benchmarks, not by brain analogy.

Minimum requirements: - Atomicity and durability in line with the
declared guarantees. - Consistency between units, revisions, relations
and indexes. - Recovery after unexpected termination. - Explicit errors
on disk failure or lack of space. - Documented isolation/concurrency
model. - Versioned persistent format. - Recovery and property tests.

Indexes must correspond to a confirmed state. Rebuilds and invalidations
must be explicit; a query must not return misleading results because an
index is incomplete. If data and indexes cannot be updated atomically,
define a consistent recovery.

## 9. Planner and adaptive execution

### First version: deterministic

1.  Analyse and validate the query.
2.  Generate valid candidate plans.
3.  Estimate cost with the available statistics.
4.  Choose a plan.
5.  Execute it.
6.  Record configurable metrics.
7.  Validate invariants and use fallback if appropriate.

### Reference path

Keep a simple and correct execution. Learned routes are optional
optimizations. Each optimized plan must record operation/fingerprint,
preconditions, schema and index dependencies, cost estimate, observed
evidence, date/version, invalidators and fallback.

### What to learn

-   Query frequency.
-   Latency and variability.
-   Data examined.
-   Cache hits.
-   Errors, fallbacks and discrepancies.
-   Maintenance cost.
-   Additional memory and storage.

Do not reinforce a route only because of frequency; it must provide net
utility. Invalidate or revalidate plans when data, schema, indexes or
relevant statistics change. Every optimization must be disableable
globally and per query. In diagnostic mode, explain why a plan was
chosen.

## 10. Computational plasticity and automation

Separate two kinds of learning:

**Operational learning:** optimises how data is retrieved/processed
through reusable plans, justified indexes, caches, batches and composed
routes.

**Knowledge learning:** proposes relations, groups events, generates
summaries, detects redundancies or contradictions, and links embeddings
to revisions.

Do not mix them: optimising a query must not alter content;
consolidating knowledge produces derived artifacts with provenance.

A connection may gain or lose priority according to evidence, but: -
Relations declared by the user must not be treated as inferred. -
Inferences must store origin and confidence. - Do not delete original
data because of low access frequency. - Forgetting content depends on
retention, consent and explicit policies. - It must be possible to
explain why a route or relation was prioritised.

Derived artifacts record their sources/revisions, method or model,
version, date, hash, validity status and whether they can be rebuilt. A
summary never silently replaces the original if auditing is required.

## 11. Indexes and retrieval

Recommended order: 1. Primary index. 2. Basic filters and relations.
3. Secondary indexes justified by load. 4. Text search, for example
BM25, if needed. 5. Exact vector search as baseline. 6. Approximate
vector index only when scale and benchmark justify it. 7. Hybrid
retrieval combining filters, text, vectors, relations, temporality and
provenance.

### Embeddings

They are derived data, not the source of truth. Record source
unit/revision, model and version, dimensions, metric, chunking, date,
input hash and validity status. When the source content changes, mark
the embedding as obsolete or create a new version. Do not mix
incompatible models/dimensions. Inference must go behind adapters or
external jobs, not inside the transactional core.

Hybrid ranking must have a diagnostic mode and be able to explain its
signals. Do not fix a universal formula before testing it.

## 12. History and provenance

Possible policies: - No content history. - Limited auditing. - Full
versioning with snapshots/deltas.

Planned operations: list revisions, compare, view provenance, restore
as a new revision, query ChangeSets and invalidate obsolete derived
data.

Distinguish information declared by the user, imported, generated by an
algorithm, inferred by a model and internally modified. Do not present
an inference as verified fact.

## 13. API and code organisation

First respect the real language, structure and conventions of the
repository. If Rust is already used, possible future modules:

-   `kerno-core`
-   `kerno-storage`
-   `kerno-tx`
-   `kerno-data`
-   `kerno-revisions`
-   `kerno-relations`
-   `kerno-index`
-   `kerno-query`
-   `kerno-adaptation`
-   `kerno-cli`

Do not create all of them as crates from day one. Extract modules when
stable contracts exist.

The initial API should allow creating/reading/updating/archiving units;
creating and traversing relations; listing/comparing/restoring
revisions; querying by filters; inspecting the plan; reading statistics;
disabling/clearing optimizations; importing/exporting data. Do not design
a complex query language before validating use cases.

## 14. Persistent format and portability

-   Version the format.
-   Define migrations and the behaviour on unknown versions.
-   Document export in an open format.
-   Verify integrity after import/export.
-   Distinguish source data from rebuildable derived data.
-   Test migrations with fixtures from earlier versions.
-   Do not assume that a change of internal structure is safe for
    existing data.

## 15. Tests and quality

Tests required, by stage: - Unit and integration. - Consistency of
units, revisions, relations and indexes. - Recovery after simulated
failures. - Property-based tests. - Fuzzing of parsers/external inputs. -
Migrations and export/import. - Concurrency according to the supported
guarantees. - Reproducible benchmarks.

Initial invariants: 1. The current state points to a valid revision.
2. Restoring does not destroy the later history. 3. Indexes and plans do
not contradict the confirmed state. 4. Adaptive plan and reference plan
produce equivalent results. 5. Invalidating an optimization does not
alter data. 6. A failed transaction leaves no partial changes visible
outside the contract. 7. Derived artifacts preserve relevant provenance.
8. Deletion respects the documented policy.

Compare against suitable alternatives, including SQLite or other
relevant embedded engines. Measure p50/p95/p99 latencies, throughput,
disk, memory, cost of revisions, relations, filters and net adaptive
improvement. Document hardware, dataset, commands and parameters. Do not
claim superiority without reproducible results.

## 16. Work plan

### Stage 0 --- Diagnosis

Map the repository, tests, dependencies, real functions, risks and
pending decisions. Deliver a status table and a prioritised plan.

**Acceptance:** repository evidence; separate facts, hypotheses and
proposals.

### Stage 1 --- Minimum contracts

Define unit, revision, relation, transaction and query; record relevant
ADRs.

**Acceptance:** understandable contracts, examples/tests and no type
without a known consumer.

### Stage 2 --- Persistence and transactions

Implement or validate storage, recovery and the transactional model.

**Acceptance:** recovery tests pass and the documented guarantees match
those tested.

### Stage 3 --- Units, revisions and provenance

CRUD, ChangeSets, comparison and restoration.

**Acceptance:** reconstruct the evolution and restore without destroying
later revisions.

### Stage 4 --- Relations and indexes

Typed relations, filters and justified indexes.

**Acceptance:** results match the reference path.

### Stage 5 --- Planner and observability

Plans, metrics, explanation of the choice and fallback.

**Acceptance:** the plan can be inspected; logs minimise sensitive data.

### Stage 6 --- First operational plasticity

Implement one small optimization (for example, a cache or a reusable
plan) and compare it with the adaptive mode disabled.

**Acceptance:** measurable improvement, equivalence of results and
tested invalidation.

### Stage 7 --- Associative memory

Retrieval by relations, context and provenance; exact vector as a
baseline if appropriate.

**Acceptance:** retrieval quality is evaluable and sources are
traceable.

### Stage 8 --- Consolidation and embeddings

Jobs/adapters for derived artifacts, versioning and obsolescence.

**Acceptance:** each derived artifact is linked to its sources and can
be invalidated/rebuilt.

### Stage 9 --- Hardening

Security, migrations, export/import, documentation, benchmarks and
stress tests.

**Acceptance:** limits and guarantees documented; critical failures
resolved or explicitly blocking.

Claude must skip stages already implemented and tested; do not redo
work just to follow the order of the document.

## 17. Required ADRs

Create `docs/adr/` or use the existing structure. Evaluate: 1.
Language/toolchain. 2. Build or reuse a physical engine; B+ tree,
copy-on-write, WAL or LSM. 3. Transactions/concurrency. 4. Persistent
format and migrations. 5. Snapshots/deltas and retention. 6. Physical
representation of relations. 7. Public API and query language. 8.
Internal and public IDs. 9. Security/encryption. 10. Text and vector
indexes. 11. Privacy of observability. 12. Adaptation and invalidation.
13. Semantics of the neural unit. 14. Versioning and rebuilding of
derived data.

Each ADR contains context, alternatives, decision, consequences, risks
and conditions for reopening it.

## 18. Risks and mitigations

  -----------------------------------------------------------------------
  Risk                                Mitigation
  ----------------------------------- -----------------------------------
  Build all engines simultaneously    Vertical MVP and stages

  Brain metaphor without real         Use case and benchmark for each
  benefit                             mechanism

  Learning changes results            Reference path, equivalence and
                                      fallback

  Costly history                      Snapshots/deltas, retention and
                                      measurement

  Obsolete indexes or embeddings      Versioning, invalidation and
                                      rebuilding

  Excess of modules                   Modularise after stabilising
                                      contracts

  Rewriting functional code           Diagnosis and tests before changing

  Own engine inferior to mature       Early benchmark and the possibility
  solutions                           of reusing

  Destructive deletion/consolidation  Originals, provenance and explicit
                                      policy

  Sensitive data in logs              Minimisation and redaction

  Performance claims without          Reproducible benchmarks
  evidence
  -----------------------------------------------------------------------

## 19. Definition of Done

A task is not finished just because it compiles. It must include code,
relevant tests, real execution of the tests, documented errors/limits,
absence of regressions, justified dependencies and an exact summary of
what was done. Do not hide failures or reduce tests to get a green
build.

## 20. First task of Claude Code

Start with diagnosis, not with a rewrite.

1.  Inspect the repository, instructions and Git status.
2.  Identify language, modules, tests, CI and the real state of the code.
3.  Review the KernoDB documentation and the `livespec` and `cauce`
    repositories if they are accessible.
4.  Compare implementation and specification.
5.  Produce a table: area, real state, evidence, gap, risk and next
    action.
6.  Identify the five most important technical decisions that are not
    justified.
7.  Recommend the smallest next vertical task that delivers value.
8.  Implement only that task if it is allowed; otherwise, wait for
    approval.
9.  At the end report modified files, decisions, tests and real results,
    limitations and next step.

Do not blindly adjust the code to this document. If the code reveals a
better solution, explain the reason and record an ADR or update the
specification.

## 21. Glossary

-   **Unit:** persistent entity of information.
-   **Revision:** versioned state of a unit.
-   **ChangeSet:** set of changes of a logical operation.
-   **Edge:** typed relation between units.
-   **Planner:** selects an execution strategy.
-   **Reference path:** simple execution used as the basis of
    correctness.
-   **Computational plasticity:** adjustment of weights/statistics
    according to evidence.
-   **Consolidation:** creation of a derived representation with
    provenance.
-   **Derived artifact:** index, embedding, summary or cache that can be
    rebuilt.
-   **Obsolescence:** derived artifact that no longer matches its current
    sources.
-   **ADR:** architecture decision record.

## 22. Executive summary

Build KernoDB in this order: persistence and integrity; units, revisions
and relations; queries and indexes; planning and observability;
operational plasticity; hybrid retrieval; consolidation and AI adapters.

The brain metaphor is only useful if it leads to contracts, tests and
measurable improvements. The goal is not a database that looks like a
brain, but a data engine able to learn to work better without losing
integrity, traceability or explainable behaviour.
