# ADR-0005 — Public identifiers

| Field | Value |
|---|---|
| Status | **Accepted** (2026-11-13), with option 1: opaque random token with a persisted mapping. Taken by express delegation of the owner, revocable until something irreversible is built on it (D1 of the [minutes of Stage 9](../architecture/stage9-record.md)). Its implementation is E9-02, **merged on 2026-11-13** (squash `2697980`, PR 78); corrected on 2026-11-13 after an independent review, by express delegation of the owner (registry value with the common header, layer mark 3, deletion and export; see [`units-v3.md`](../file-format/units-v3.md)) |
| Date | 2026-10-27 |
| Related to | SAD §5.2, §11.2, D-05, invariants I-26, I-27, [threat model](../security/threat-model.md), ADR-0006 |

## Context

Each record has an `internal_id` UUIDv7 (D-05). UUIDv7 exposes its creation instant with millisecond precision and, according to RFC 9562, must not be treated as a secret or as a security capability. Exposing it directly reveals when each record was created and makes it easy to enumerate records that are contiguous in time.

Kerno needs a public representation that is **opaque, stable and validatable**, behind an interchangeable `IdentifierCodec` abstraction, without opacity being confused with authorization.

## Options considered

1. **Opaque random token with a persisted mapping.** Generated once per record with a system CSPRNG (≥ 128 bits), stored alongside the record and in a `token → internal_id` index within the same transaction.
   - ✔ There are no keys to manage, rotate or lose; a leaked token reveals nothing about the internal ID.
   - ✔ The token of a record can be revoked or reissued without touching its internal identity.
   - ✘ One extra index entry per record (≈ 40–60 B) and a point lookup to resolve each token.
   - ✘ Exporting/importing requires keeping the mapping (it is part of the data, not of an external key).
2. **Authenticated, reversible encrypted token.** `version ‖ key_id ‖ AEAD(internal_id)` with a consolidated algorithm and library (deterministic cipher resistant to nonce misuse, e.g. AES-SIV, RFC 5297), generated and stored once per record.
   - ✔ Resolution without an index; valid tokens verifiable without access to the database.
   - ✘ Requires key management, rotation and a policy for key loss (ADR-0006) before publishing the format.
   - ✘ Longer tokens (≥ 40 characters in base32) and coupled to the version of the encryption scheme.
   - ✘ A compromised key de-anonymizes all tokens issued with it.
3. **Expose the UUIDv7 or a homemade transformation of its bits.** Discarded by the SAD: it reveals the timestamp, and homemade "obfuscation" is not security.

## Decision criteria (SAD §15)

Token stability, rejection of tampering, support for versioning, and clarity of the separation between identification and authorization.

## M0 evidence

- Resolving a token with option 1 costs one additional point read. In M0, a point read with a warm cache costs between 0.7 and 8.7 µs (p50, depending on engine and size, from 20 000 to 1 M records; [M0 report](../testing/m0-report.md), `point_reads`), negligible compared with a durable commit (hundreds of µs).
- The extra index entry is written in the same atomic batch as the record (I-01), so token and record cannot fall out of sync.

## Proposal

- **Default codec: option 1** (random token + mapping), with format `k{version}_{base32 of 128 random bits}` and, optionally, a type prefix defined by the application. Validation checks prefix, version, alphabet and length before searching.
- **Option 2 as an alternative codec**, implementable once ADR-0006 defines the key provider and rotation. The token is generated and stored once per record; key rotation does not change tokens already issued (old keys are kept to decrypt, or the stored token is used).
- In both cases, **a valid token authorizes nothing** (I-27): the layer that exposes Kerno checks permissions on every operation.

## Consequences

- The `token → internal_id` index is transactional (ADR-0007) and is part of the logical export (I-30). Deleting a unit removes its mapping in the same transaction (D-a of 2026-11-13); the export carries the token of each unit and the import restores it (D-b).
- There is no cryptographic material on the default critical path; ADR-0006 can be closed later without blocking phases 1–3.
- The token format carries a version from day one, so changing codec does not break existing tokens.

## Follow-up

- Phase 4: implement `IdentifierCodec`, tests of tampered tokens (format, version, length, alphabet, nonexistent tokens) and of stability after export/import.
- Review this proposal when ADR-0006 is accepted.

## Notes

### Accepted limit: source of randomness (2026-11-13)

Decision taken on 2026-11-13 by express delegation of the owner, revocable. The 128-bit token is read from `/dev/urandom` with `std`, with no new dependency. On a platform without that file (Windows, WASI) creating a public identifier fails with `UnitError::Storage` and `Io`, without creating anything. Resolving tokens already issued keeps working. A read with an error or a short read also fails, and a token whose 16 bytes are all equal is rejected. Code: `crates/kerno-units/src/ids.rs`.

This is lifted in one of two ways: a randomness API in the standard library for the platforms the project supports, or the decision to accept a dependency (`getrandom`), which goes through the dependency rule (I-31). Full record in [post9-pending](../architecture/post9-pending.md), section "Decision by delegation of 2026-11-13".
