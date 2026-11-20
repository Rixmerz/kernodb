# ADR-0017 — Semantics of the "neural unit"

| Field | Value |
|---|---|
| Status | Proposed |
| Date | 2026-10-28 |
| Deciders | Project owner (pending) |
| Related to | [specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §2, §3, §6.1, §10, §21; SAD §5; ADR-0004, ADR-0005, ADR-0008, ADR-0014, ADR-0016; invariants I-22, I-39 |

## Context

The specification uses the brain as inspiration (§3) and warns that the correspondences are design hypotheses, not scientific equivalences, and that modules are defined by technical contracts rather than anatomical names. It is necessary to fix what "unit" means in the data model and what the terms inspired by the brain (connection, plasticity, consolidation, forgetting) mean, so that nobody implements them with another semantics.

## Options considered

1. **Unit = document with identity and lifecycle** (proposal). The "neural unit" is only a name from the metaphor; technically it is a `Unit` (spec §6.1): stable identity, type, schema version, current revision, lifecycle state, metadata and content reference. The "connections" are `Edge` (ADR-0014); the "plasticity", adjustments of statistics or priorities (ADR-0016); the "consolidation", derived artifacts with provenance (ADR-0018).
2. **Unit with its own dynamic state** (activation, weights that change on read). Rejected in the proposal: it would make reads mutate data (spec §2) and would mix operational learning with content (spec §10).
3. **No "unit" term**, only "document" as in the SAD. It loses the generality that the specification asks for (document, memory, event, fact, resource).

## Decision criteria

1. Each term of the metaphor is defined with a verifiable technical contract and a use case (spec §2).
2. No read changes unit data.
3. Compatibility with the SAD's document model (§5) and with the revision history (ADR-0004).

## Evidence

No measurement applies: this is a decision about vocabulary and contract. It is validated with examples and tests once the Stage 3 API exists.

## Decision

Pending. Proposal: option 1, with these rules:

- `Unit` is equivalent to the SAD's document; `unit_type` is the collection or logical type.
- `lifecycle_state` admits at least `active`, `archived` and `deleted`; the transitions are defined by ADR-0009.
- A term of the metaphor does not name crates or modules (spec §3, §13).
- "Forgetting" never deletes content for low access frequency; only explicit retention policies (spec §10).
- Every inferred "connection" stores its origin and confidence and is not presented as declared (I-39).

## Consequences

- The documentation and the API use `Unit`, `Revision`, `ChangeSet` and `Edge`; the brain terms appear only in the vision documentation.
- A mechanism inspired by the brain that has no use case and proof of value does not enter the product.

## Conditions for reopening

A validated use case that needs dynamic state per unit; in that case it would be modelled as derived data, not as part of the unit.

## Follow-up

- Apply the vocabulary in ADR-0010 (public API) when Stage 3 starts.
