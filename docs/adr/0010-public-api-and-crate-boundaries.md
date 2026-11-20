# ADR-0010 — Public API and crate boundaries

| Field | Value |
|---|---|
| Status | **Accepted** (2026-10-31, by the owner; [minutes E3-00](../architecture/stage3-record-e3-00.md)) |
| Date | 2026-10-27 (opening) · 2026-10-31 (acceptance) |
| Related to | SAD §6, §9.3, §14 |

## Context

The public API must be small, typed and stable; the details of the physical engine must not leak out.

**Question:** Which types and errors are public, and which modules are internal.

## Options considered

1. **One public crate `kerno`** that re-exports a facade; internal crates not published.
2. **Several public crates** per layer.
3. **A single crate** with private modules until Phase 2.

## Initial proposal (SAD §15)

Small, typed and stable API; do not make page or physical-engine details public; separate modules only when their contracts are clear.

## Decision criteria

An external application that compiles; an error guide; a deprecation and semver policy.

## What M0 contributed

The `Engine` trait of the spike (today in `crates/kerno-core/src/contract.rs`) is an **internal** comparison contract, not the public API: it lacks read-modify-write transactions, cursors, cancellation and errors by category. It does contribute an initial error taxonomy (`Io`, `Corruption`, `Locked`, `InvalidArgument`, `Poisoned`) that should be kept.

## Dependencies

It depends on ADR-0002 (shape of the transactions).

## Decision

For Stage 3, the separation into crates of the [Stage 3 design](../architecture/stage3-design.md) (§6) is accepted, with these conditions from the owner:

1. **New crate `kerno-units`**, generic over `TxnEngine` of `kerno-core`, with strict separation from `kerno-storage`: it does not depend on its internal modules and introduces no regressions against the goals T1–T10.
2. **On-disk layer mark** (`LAYER_META`), with no `incompat` bit for UNITS while `kerno-storage` is internal.
3. The only change allowed outside `kerno-units` is `Txn::read_seq` in `kerno-core`, with its `impl`s in `WriteTxn` and `EmulatedTxn`.
4. `AsOf::Time` is left out of Stage 3.

The single public facade (`kerno`, option 1) and the semver policy are decided when a stable public API exists; this acceptance does not fix them.
