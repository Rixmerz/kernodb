# ADR-0008 — Document serialization and hashes

| Field | Value |
|---|---|
| Status | **Accepted** (2026-10-31, by the owner when closing E3-02; minimum version for Stage 3) |
| Date | 2026-10-27 (opening) |
| Related to | SAD §5.3, §13, RFC 8785 |

## Context

The content hashes of revisions must be reproducible across runs and versions; that requires a canonical serialization.

**Question:** Initial JSON, binary support and canonicalization for hashing.

## Options considered

1. **JSON + RFC 8785 (JCS)** for hashing; storage in a proprietary or standard binary encoding (deterministic CBOR, RFC 8949 §4.2).
2. **Single canonical binary encoding** for storage and hash.
3. **Opaque bytes** (the application decides) with the hash computed over the bytes as they are.

## Initial proposal (SAD §15)

Separate the logical and physical format; evaluate RFC 8785 for JSON when its constraints are acceptable; version the hash algorithm.

## Decision criteria

Reproducible results for the supported cases and compatibility tests.

## What M0 contributed

No work in M0 beyond reserving the `content_hash` field and the algorithm identifier in the revision design.

## Dependencies

It constrains ADR-0004 (deltas and hashes) and ADR-0003 (versioning of the algorithm).

## Decision

Minimal proposal, limited to what Stage 3 needs (option 3 above); structural canonicalization (JCS, CBOR) and diff are left out and will be decided in a later ADR:

1. **Opaque bytes.** The payload of a revision is bytes that the layer does not interpret; `payload_codec = 0`. The hash is computed over `payload_codec ‖ payload` as stored.
2. **BLAKE3-256 hash** (32 bytes), identified by an `alg` byte in the `content_hash` field (`alg = 1`), so that the algorithm is not fixed in the layer's format. Adding the dependency requires justifying that neither the standard library nor the current manifest covers it, and it enters with the acceptance of this ADR, not before.
3. **Provisional CRC-32C** (`alg = 0`), already available in the project and **without cryptographic guarantee**, until this decision is complete; it serves only to detect end-to-end mismatch.
4. A payload's length and presence are checked on every read; the content hash is checked when `verify_payload_on_read` is on, and always by `verify()`, history, export/import and the reads a write builds on; a damaged block is always caught by the block CRC. A hash mismatch is `Corruption` (I-08).

**Acceptance (2026-10-31).** The owner accepted this ADR with these conditions:

- BLAKE3-256 (`alg = 1`) is the integrity hash of `REV_PAYLOAD`. Neither the standard library nor the manifest had a cryptographic hash, and that is why the `blake3` dependency (no default features, only `std`) entered `kerno-units` with E3-02.
- The version is pinned exactly in the workspace manifest (`blake3 = "=1.8.7"`) and in `Cargo.lock`. Changing it is a reviewed change.
- The official BLAKE3 vectors in `crates/kerno-units/tests/payload_integrity.rs` are compatibility tests and are not modified.
- CRC-32C (`alg = 0`) remains provisional, without cryptographic guarantee, and does not replace BLAKE3-256.

Encoding of the field: [`units-v1.md`](../file-format/units-v1.md) §4.
