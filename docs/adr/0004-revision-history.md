# ADR-0004 — Revision history

| Field | Value |
|---|---|
| Status | **Accepted** (2026-10-31, by the owner; [minutes E3-00](../architecture/stage3-record-e3-00.md)) |
| Date | 2026-10-27 (opening) · 2026-10-31 (acceptance) |
| Related to | SAD §8, D-06, invariants I-20…I-22 |

## Context

Kerno must reconstruct and compare earlier versions of records without turning the recovery journal into a permanent history (D-06). It is necessary to define what a revision represents and when it is stored as a snapshot or as a delta.

**Question:** What does a revision represent, and when should snapshot/delta be used?

## Options considered

1. **Snapshot per revision** — O(1) reconstruction, space cost proportional to the size of the document per change.
2. **Deltas only** — minimal space, O(depth) reconstruction; unacceptable without a limit.
3. **Hybrid snapshot + delta with a depth/size limit** — the SAD's proposal.

## Initial proposal (SAD §15)

Revision Engine separate from the journal; optional `ChangeSet`; snapshot + delta with a depth limit; restore creates a new revision.

## Decision criteria

Exact reconstruction, deterministic diff and configurable retention.

## What M0 contributed

M0 left a [preliminary design](../architecture/revision-model-design.md) (key layout `c/`, `r/`, `x/`, policies `none`/`audit`/`versioned`) and measured its cost on both candidates with the `history_sim` workload ([M0 report](../testing/m0-report.md)). The design requires nothing from the engine beyond atomic batches and range scans.

## Dependencies

It depends on ADR-0001 (cost of writing/reading ranges and physical deletion) and on ADR-0008 (delta format and canonical hash).

## Decision

The revision model of the [Stage 3 design](../architecture/stage3-design.md) (§2.4, §3) is accepted, with these conditions from the owner:

1. **Full copy per revision** (option 1): each revision stores its entire payload; there are no deltas until benchmarks justify them. `Delta` is reserved in the `kind` byte of `RevisionMeta`.
2. **A single copy of the current payload**: it lives in `REV_PAYLOAD(unit, head_rev)`, not in the head; reading it costs 2 `get`s, an accepted cost.
3. **Metadata and classification versioned and audited** in the revision (`REV_META`).
4. History lives in the layer's own keys, not in the WAL (D-06, I-20); restoring creates a new revision (I-22).
5. The content hash and the format of the payload as opaque bytes depend on ADR-0008 (accepted 2026-10-31: BLAKE3-256).

Option 3 (hybrid with a depth limit) from the SAD remains a future evolution, subject to measurement and a new ADR.
