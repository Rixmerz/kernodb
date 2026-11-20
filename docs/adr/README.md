# Architecture Decision Records

Each ADR records a lasting decision, its context, the options evaluated, the evidence and its consequences ([template](template.md)). The numbering and initial questions of 0001–0012 come from [SAD §15](../architecture/SAD.md#15-architecture-decision-records-adr); 0013 from the design of Phase 1; 0014–0018 from the [specification v0.2.0](../spec/README.md); 0019 from a decision of the owner of 2026-11-11; 0020 from the decision of 2026-11-12 on vector persistence (minutes of Stage 8, D2).

Rules:

- An **Accepted** ADR is not edited to change the decision: another one is created that replaces it, and the previous one is marked as *Replaced by ADR-XXXX*.
- Decision criteria are written before the results are examined.
- A decision that depends on benchmarks links the code, the command and the raw data.
- The confirmed decisions D-01…D-11 of SAD §2.1 do not need their own ADR; if any of them is reopened, it is documented here.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-storage-engine.md) | Physical storage engine | **Accepted**: leveled LSM tree (2026-10-28) |
| [0002](0002-transaction-model-and-durability.md) | Transaction model and durability | **Accepted** (2026-10-28) |
| [0003](0003-physical-format-and-versioning.md) | Physical format and versioning | **Accepted** (2026-10-28; the concrete format is frozen when Phase 1 closes) |
| [0004](0004-revision-history.md) | Revision history | **Accepted** (2026-10-31; [minutes E3-00](../architecture/stage3-record-e3-00.md)) |
| [0005](0005-public-identifiers.md) | Public identifiers | **Accepted** (2026-11-13, option 1; D1 of the [minutes of Stage 9](../architecture/stage9-record.md)). Implementation in E9-02, merged 2026-11-13 (corrected after independent review) |
| [0006](0006-encryption-and-key-management.md) | Encryption and key management | Pending (not accepted). Design chosen if encryption is built: option 1, per-block AEAD (D2, 2026-11-13). Nothing is implemented |
| [0007](0007-index-consistency-and-rebuilding.md) | Index consistency and rebuilding | Pending |
| [0008](0008-document-serialization-and-hashes.md) | Document serialization and hashes | Accepted (minimum: BLAKE3-256 and opaque bytes) |
| [0009](0009-suppression-retention-and-backups.md) | Suppression, retention and backups | **Accepted** (2026-11-12, option 3 under the terms of the ADR; no physical deletion guaranteed; no backup deadline declared) |
| [0010](0010-public-api-and-crate-boundaries.md) | Public API and crate boundaries | **Accepted** (2026-10-31; [minutes E3-00](../architecture/stage3-record-e3-00.md)) |
| [0011](0011-vector-profile-and-hybrid-retrieval.md) | Vector profile and hybrid retrieval | **Accepted** (2026-11-11, E7-00: option 1; [evaluation set](../architecture/stage7-evaluation-set.md)) |
| [0012](0012-benchmark-methodology-and-exit-thresholds.md) | Benchmark methodology and exit thresholds | **Accepted** (2026-11-12, by delegation of the owner; D5 of the [minutes of Stage 9](../architecture/stage9-record.md)). It was proposed and applied in M0 |
| [0013](0013-format-v1-and-engine-protocol.md) | Format v1 and engine protocol | **Accepted** (2026-10-31; the v1 format was frozen that day, [v1.md](../file-format/v1.md)). Status as of 2026-11-11: this row said «Proposed», an outdated earlier text |
| [0014](0014-physical-representation-of-relations.md) | Physical representation of relations | Accepted (2026-11-05, E4-02: option 2). The body of the E4-00 note was aligned with this row on 2026-11-12 (E9-00) |
| [0015](0015-privacy-of-observability.md) | Observability privacy | Accepted on 2026-11-06 with the evidence of E5-04 (spec v0.2.0 §17.11) |
| [0016](0016-operational-adaptation-and-invalidation.md) | Operational adaptation and invalidation | Accepted (2026-11-09; guarantees G1–G7); **confirmed 2026-11-13** with amendments (D16, closed) |
| [0017](0017-semantics-of-the-neural-unit.md) | Semantics of the "neural unit" | Proposed (spec v0.2.0 §17.13; decided when Stage 3 starts) |
| [0018](0018-versioning-and-rebuilding-of-derived-artifacts.md) | Versioning and rebuilding of derived artifacts | **Accepted** (2026-11-12, by the owner: option 2, common derivatives registry; [minutes of Stage 8](../architecture/stage8-record.md)) |
| [0019](0019-license-and-contributions.md) | License and contributions | **Accepted** (2026-11-11, by the project owner) |
| [0020](0020-persistence-of-vectors-as-derivatives.md) | Vector persistence as derivatives | **Accepted** (2026-11-12, by delegation of the owner, revocable by them; condition of D2 met: E8-03 passed; [minutes of Stage 8](../architecture/stage8-record.md) §5.4 and D2). Replaces the persistence clause of ADR-0011, which is not edited |
| [0021](0021-typed-schemas-and-secondary-indexes.md) | Typed schemas per unit type and user-defined secondary indexes | **Proposed** (2026-11-18, Phase C; implemented, new record families `0x78`/`0x79` with layer 6, pending the owner's acceptance) |

ADRs 0014–0018 come from the list in the [specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §17; the full correspondence is in [`docs/spec/`](../spec/README.md).
