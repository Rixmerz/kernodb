# ADR-0006 — Encryption and key management

| Field | Value |
|---|---|
| Status | Pending (not accepted). If encryption is built, the chosen design is option 1 (per-block AEAD), decided on 2026-11-13 by express delegation of the owner (D2 of the [minutes of Stage 9](../architecture/stage9-record.md)); nothing is implemented, and 1.0 does not include encryption (D3) |
| Date | 2026-10-27 (opening) |
| Related to | SAD §11.3, D-09, invariant I-28, [threat model](../security/threat-model.md) |

## Context

Encryption at rest and key management affect the physical format (headers, page/block size, nonces), recovery and backups. It must be decided before the format is frozen (ADR-0003).

**Question:** Scope of file encryption and how keys are managed.

## Options considered

1. **Per-page/block encryption with AEAD** (e.g. AES-256-GCM-SIV or XChaCha20-Poly1305 from an audited library) — cryptographic integrity included; requires space for a nonce and tag per unit.
2. **File-system/device-level encryption** delegated to the OS — no format changes, but no protection of exported backups and no key control by Kerno.
3. **Per-record/field encryption** — allows selective cryptographic erasure; more metadata and complexity in indexes.

## Initial proposal (SAD §15)

Standard algorithms and an audited library; keys kept outside the data directory; a key-provider interface (`KeyProvider`) and a defined rotation strategy.

## Decision criteria

Data, journals, temporaries and backups covered; key loss documented; integrity and rotation tests.

## What M0 contributed

M0 uses CRC32C, which detects accidental corruption but **not** tampering (risk R-S5). Both candidates write units of known size (4 KiB pages in the B+ tree; blocks of ~4 KiB plus metadata in LSM), so reserving space for nonce + tag is viable in either. In a CoW B+ tree every rewritten page gets a new location, which makes unique nonces easier; in LSM the files are immutable and are encrypted once, when written.

## Dependencies

It constrains ADR-0003 (headers and sizes), ADR-0009 (cryptographic erasure) and ADR-0005 (keys for encrypted tokens).

## Decision

Pending. No assumption of this ADR should be coded in a way that is hard to reverse until it is accepted.
