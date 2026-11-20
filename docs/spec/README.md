# Product specifications

| Document | Version | Status |
|---|---|---|
| [Architecture specification](architecture-specification-v0.2.0.md) | 0.2.0 | Design proposal, received 2026-10-28; kept **unedited** |

The v0.2.0 specification does not replace the [SAD v0.1.0](../architecture/SAD.md), which remains the technical baseline, nor the accepted ADRs. The specification describes vision, principles and stages. Where it contradicts an accepted decision, the accepted decision stands until a new ADR replaces it, under the rules in [`docs/adr/`](../adr/README.md). A change to the specification is saved as a new version in this folder, without overwriting the previous one.

## Correspondence with the repository

### Stages (§16) and SAD phases (§16)

The specification numbers the work in stages 0–9; the SAD, in phases 0–6. The [roadmap](../roadmap/README.md) uses the stages for the work after Phase 1 and keeps the phases already in progress.

| Stage (spec) | Phase (SAD) | Status |
|---|---|---|
| 0 Diagnosis | Phase 0 / M0 | Closed |
| 1 Minimum contracts | Phase 0 (ADR-0001…0013) | Partial: storage and transaction contracts defined; unit, revision and relation pending (they move to Stage 3) |
| 2 Persistence and transactions | Phase 1 | In progress |
| 3 Units, revisions and provenance | Phases 2 and 3 | Closed (2026-11-04); open risks K-E3-8 and K-E3-9 ([closure](../architecture/stage3-closure.md)) |
| 4 Relations and indexes | Phase 2 (primary index, filters) and Phase 5 (secondary indexes) | Closed, approved 2026-11-05 ([closure](../architecture/stage4-closure.md)) |
| 5 Scheduler and observability | no equivalent in the SAD | Proposed acta, pending approval ([record](../architecture/stage5-record.md)) |
| 6 First operational plasticity | no equivalent in the SAD | — |
| 7 Associative memory | Phase 5 (exact vectors, hybrid ranking) | — |
| 8 Consolidation and embeddings | Phase 5 (embedding adapter, invalidation of derived data) | — |
| 9 Hardening | Phases 4 and 6 | — |

### Required ADRs (§17) and repository ADRs

| # spec | Topic | ADR |
|---|---|---|
| 1 | Language and toolchain | Decision confirmed in SAD §2.1 (Rust); no ADR of its own |
| 2 | Physical engine | [ADR-0001](../adr/0001-storage-engine.md) |
| 3 | Transactions and concurrency | [ADR-0002](../adr/0002-transaction-model-and-durability.md) |
| 4 | Persistent format and migrations | [ADR-0003](../adr/0003-physical-format-and-versioning.md), [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) |
| 5 | Snapshots, deltas and retention | [ADR-0004](../adr/0004-revision-history.md), [ADR-0009](../adr/0009-suppression-retention-and-backups.md) |
| 6 | Physical representation of relations | [ADR-0014](../adr/0014-physical-representation-of-relations.md) |
| 7 | Public API and query language | [ADR-0010](../adr/0010-public-api-and-crate-boundaries.md) (the query language stays out until use cases are validated, spec §13) |
| 8 | Internal and public IDs | [ADR-0005](../adr/0005-public-identifiers.md) |
| 9 | Security and encryption | [ADR-0006](../adr/0006-encryption-and-key-management.md) |
| 10 | Text and vector indexes | [ADR-0007](../adr/0007-index-consistency-and-rebuilding.md), [ADR-0011](../adr/0011-vector-profile-and-hybrid-retrieval.md) |
| 11 | Privacy of observability | [ADR-0015](../adr/0015-privacy-of-observability.md) |
| 12 | Adaptation and invalidation | [ADR-0016](../adr/0016-operational-adaptation-and-invalidation.md) |
| 13 | Semantics of the neural unit | [ADR-0017](../adr/0017-semantics-of-the-neural-unit.md) |
| 14 | Versioning and rebuilding of derived data | [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md) |

## Differences from the SAD that the owner must resolve

1. **Security order versus search.** The SAD puts security and lifecycle (Phase 4) before search and AI (Phase 5); the specification leaves security in hardening (Stage 9), after associative memory and embeddings. The roadmap proposes requiring the minimum of Stage 9 that affects derived data (deletion, log redaction) before starting Stage 8.
2. **Scheduler and plasticity.** Stages 5 and 6 do not exist in SAD v0.1.0; they are documented in [adaptive execution](../architecture/adaptive-execution.md) and ADR-0015/0016.
3. **Comparison with SQLite.** The specification (§15) asks for a comparison with SQLite. M0 already included it as the reference engine ([M0 report](../testing/m0-report.md)), and the Phase 1 benchmark task measures it again; it still has to be presented as an explicit *baseline* with equivalent durability (see the roadmap).
