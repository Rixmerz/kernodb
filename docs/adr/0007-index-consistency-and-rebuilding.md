# ADR-0007 — Index consistency and rebuilding

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-10-27 (opening); 2026-11-04 (accepted by the owner, E4-00) |
| Related to | SAD §9, D-07, invariants I-21, I-23, I-24 |

## Context

Secondary and derived indexes must be consistent with the source content or explicitly declare their freshness.

**Question:** Which indexes participate in transactions and which are derived.

## Options considered

1. **Transactional**: updated in the same atomic batch as the content.
2. **Asynchronous derived**: updated afterwards, with state (`ready`/`building`/`stale`/`failed`) and a sequence watermark.
3. **Mixed, per index definition** (proposal).

## Initial proposal (SAD §15)

Primary index consistent with the commit; critical secondary indexes with a clear atomic boundary; costly indexes with a freshness state and rebuilding.

## Decision criteria

Rebuilt indexes produce results equivalent to a new index over the same data.

## What M0 contributed

The spike's contract (atomic batch + monotonic, persistent commit sequence) is enough for both modes: a transactional index is additional keys in the same batch; a derived index stores its watermark as one more key and compares against `Snapshot::seq()`.

## Dependencies

It depends on ADR-0002 (commit sequence as watermark).

## Decision

Pending. No assumption of this ADR should be coded in a way that is hard to reverse until it is accepted.

## Decision (2026-11-04)

The recommended option is accepted: the primary index is consistent with the commit; the secondary indexes are asynchronous derived indexes with state (`ready`/`building`/`stale`/`failed`) and a sequence watermark. I-23 and I-24 are promoted in E4-03.

## Evidence (E4-03, 2026-11-05)

E4-03 implements the primary index of `kerno-units` ([`index.rs`](../../crates/kerno-units/src/index.rs), [units-v1.md](../file-format/units-v1.md) §3.9–§3.10) with the state `ready`/`building`/`stale`/`failed` and the watermark that this decision asks for. What was decided when implementing it:

- **The watermark is the commit sequence of the `ChangeSet` that wrote the entries.** That `ChangeSet` (purpose `Index`) changes no unit, so the entries are a function of the state in that same commit, and the layer can write its own sequence (it is predicted, as in any `ChangeSet`). Comparing with `Snapshot::seq()` is then exact: `Ready` if they are equal, `Stale` if the snapshot is later.
- **Any later commit leaves the index `Stale`**, including one that touches no unit (a policy, a relation). This is deliberately conservative: a query never declares a freshness it cannot prove. `UnitSnapshot::query` falls back to the reference path and says why; `indexed_scan` refuses.
- **`Ready` and `Failed` are stored; `Stale` is computed; `Building` belongs to the handle.** A build is a single atomic `ChangeSet`, so a stored `Building` state would never truthfully exist after a crash.
- **The build reads only the heads.** The deeper damage is covered by `verify_units` (K-E3-7), and each unit that an index query returns is read and verified as in `get_unit`.

**Open point for the owner.** This decision says the primary index is "consistent with the commit". E4-03 implements it as a derived index with explicit freshness, because the minutes (§3, row E4-03) and invariant I-23 ask for the state and the watermark for it. The consequence is that, after any write, queries use the reference path until the next `rebuild_index`. If a measured workload justifies it, the next step is to maintain it inside each `ChangeSet` (always `Ready`, with the write cost that K-E3-9 already points out) or to rebuild it in the background; the format and the state already allow it. Proposal: decide it in E4-04, with a written workload, and not before.

## Evidence (E4-04, 2026-11-05)

Workload W1 ([stage4-workloads.md](../architecture/stage4-workloads.md)), written before measuring, justified a secondary index on `(unit_type, schema_version)`: with 100 000 units and 1 % selectivity, the primary path stood at 4.8–12.3× the ideal bound with a p95 of 57–71 ms. `IDX_TYPE` ([units-v1.md](../file-format/units-v1.md) §3.11) is derived as this decision states and shares a generation with `IDX_PRIMARY`: same construction, same transaction, same `IDX_META`, same state and watermark. W1 measures reads on a `Ready` index; it does not measure interleaved writes, so it provides **no** evidence for maintaining the indexes inside each `ChangeSet`. The open point above remains open, pending a workload that measures it.

## Owner's decision on the open point (2026-11-05)

The implementation of E4-03/E4-04 is approved: the unit indexes (`IDX_PRIMARY`, `IDX_TYPE`) remain **derived** with explicit freshness, and only `rebuild_index` writes them; after a write, `query` uses the reference path until the next rebuild (I-32). Maintaining them inside each `ChangeSet` or rebuilding them in the background is **not** done without a workload written before measuring that includes interleaved writes and shows that the reference path is not enough. The adjacency indexes of relations do not change: they are transactional (ADR-0014, I-21).

**Known limit.** The construction keeps all entries in memory and writes them in a single batch, so it is limited by the engine's batch limits (1 Mi operations and 64 MiB by default); above that it fails with `ChangeSetTooLarge` and the state becomes `Failed`.

