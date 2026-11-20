# Minutes E3-00: entry gates of Stage 3

| Field | Value |
|---|---|
| Date | 2026-10-31 |
| Act | Approval of package E3-00 by the project owner |
| Document | [Stage 3 design](stage3-design.md), §11 (E3-00) and §12 |
| Result | **Approved** |

## Start condition

The roadmap requires, before Stage 3, Phase 1 closed, format v1 frozen and ADR-0013 accepted. Status checked on this date:

- **Phase 1 closed** (PR #2, merged).
- **Format v1 frozen** and **ADR-0013 accepted** (PR #3): the `incompat` bit `DRAFT` is withdrawn and `docs/file-format/v1.md` no longer lists it as active.

## Decisions of §12

| # | Decision | Result | Owner condition (verbatim) |
|---|---|---|---|
| 1 | Start condition | **Satisfied** (freeze done) | — |
| 2 | Crate and API (ADR-0010) | Accepted as the design recommends | New crate `kerno-units`, strict separation from `kerno-storage`, no regressions in T1–T10. |
| 3 | Feature bit | Accepted as the design recommends | On-disk layer marker; no `incompat` bit for UNITS while `kerno-storage` is internal. |
| 4 | Snapshots versus deltas | Accepted as the design recommends | Full-copy revisions; no deltas until benchmarks justify them. |
| 5 | Current payload at the head | Accepted as the design recommends | A single copy; the cost of 2 `get` calls is accepted. |
| 6 | Versioned metadata | Accepted as the design recommends | Metadata and classification versioned and audited in the revision. |
| 7 | Hash | Accepted as the design recommends | BLAKE3-256 with its ADR (ADR-0008); CRC-32C provisional until that decision is complete. |
| 8 | `AsOf::Time` | Accepted as the design recommends | Out of scope for Stage 3. |

Decisions 9–13 of §12 are **not** part of this approval and remain open.

## Scope of the approval

The approval **does not authorise** features of later stages nor unnecessary changes to the storage engine. The only change allowed outside `kerno-units` is `Txn::read_seq` (E3-01).

## Effects

- ADR-0004 and ADR-0010 move to **Accepted** (2026-10-31, these minutes).
- ADR-0008 is proposed in its minimal version (hash BLAKE3-256 and opaque bytes) as **Proposed**.
- E3-01 can start; E3-02 and later depend on it.
