# ADR-0015 — Privacy of observability

| Field | Value |
|---|---|
| Status | **Accepted** on 2026-11-06 with the evidence of E5-04 (decision of the owner of 2026-11-05; [minutes of Stage 5](../architecture/stage5-record.md) §8) |
| Date | 2026-10-28 |
| Deciders | Project owner |
| Related to | [specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §6.5, §7, §18; SAD §11.4; [adaptive execution](../architecture/adaptive-execution.md); [threat model](../security/threat-model.md); ADR-0009, ADR-0016; invariants I-29, I-37 |

## Context

Phase 1 already exposes storage metrics (WP04: counters and histograms with no user content). Stages 5 and 6 add execution observations, plan explanations and statistics, which can leak data: query literals, keys, filter values or sizes that reveal content. The specification requires not storing sensitive values by default; SAD §11.4 prohibits recording full content, secrets and tokens.

## Options considered

1. **No persistent observations**: only aggregated metrics in memory. Safe, but it prevents evidence-based adaptation (Stage 6).
2. **Levels of detail with redaction by default** (proposal): `off` (default), `aggregate` (counters and histograms per fingerprint, without text), `fingerprint` (normalized fingerprint without literals) and `diagnostic` (fingerprint text and plan explanation, enabled only explicitly, with short retention).
3. **Full log with encryption**: everything is stored encrypted. Key complexity (ADR-0006), and the risk remains when decrypting for diagnosis.

## Decision criteria

1. With the default level, no observability output contains literals or content (test with sentinels, I-37).
2. Stage 6 adaptation works with the `fingerprint` level.
3. Retention and suppression of observations comply with ADR-0009 (a data suppression leaves no copies in observations).
4. The cost of recording an observation is measured; it must not raise the p99 of the observed query above the threshold that Stage 5 sets.

## Evidence

- **Criterion 1 (no literals or content, I-37).** `crates/kerno-units/tests/sentinels.rs` plants sentinels in the type, the payload, the metadata, the relation type, the origin and the provenance source, and the values of both filters, and searches for them (text, decimal byte list and hex) in the observer's records and counters, in the plan explanations, in the fingerprints, and in the errors of queries and writes, across the four levels and with a damaged record. None appears, not even in `diagnostic`: the shape's text carries typed markers. It also checks that each reported fingerprint is that of the same shape with other literals, so that a literal put only into the hash input does not pass. The test includes a control (the response to the caller does contain the sentinels) and dies with the mutation "copy the literal into the fingerprint or the explanation" in its three variants: in the shape's text (filter and relations), only in the hash input, and in the cost of a plan candidate. Report: [E5-04](../architecture/stage5-sentinels.md).
- **Criterion 2 (Stage 6 works with `fingerprint`).** The `fingerprint`-level record carries a keyed fingerprint, path, plan, rows, sequence, duration and fallback ([E5-03](../architecture/stage5-observation.md)); no text is needed.
- **Criterion 3 (suppression, ADR-0009).** Observations live only in memory, with retention by size and age, and store no unit content; a data suppression leaves no copies in them. Export and persistence are out of scope until a new decision.
- **Criterion 4 (cost).** Threshold fixed before measuring: ≤ 5 % of the p99 of the observed query. In [E5-03](../architecture/stage5-observation.md) §7 it meets it in 7 of 8 measurements; `diagnostic` on 20 000 units, run 1, comes in at +5.9 % (+3.6 % in run 2). It is recorded as a risk and does not block (minutes §8, point 5); `diagnostic` is only on explicit activation.
- **Local key.** The fingerprint uses BLAKE3 with a local key that is not exported (no accessor, no `Debug` of its bytes): a shape has few values, and an unkeyed hash would fall to a dictionary attack ([E5-01](../architecture/stage5-footprint-and-plan.md)).

## Decision

Option 2, accepted on 2026-11-06 with the evidence above, by decision of the owner of 2026-11-05. Literals are replaced with typed markers before the fingerprint is computed; the fingerprint uses BLAKE3 with a local key that is not exported, with the algorithm and version recorded. The default level is `off`; `diagnostic` is only enabled explicitly.

## Consequences

- The planner must work with fingerprints, not with query text.
- The `diagnostic` level needs operational documentation and an explicit warning when it is enabled.
- Observations are derived data: they are excluded from the logical export by default and are deleted without affecting the data (I-34).

## Conditions for reopening

A use case that requires observations with values (for example, debugging a specific query in production), or an applicable regulation that imposes different requirements.

## Follow-up

- Done in E5-04: sentinel test and review of the [threat model](../security/threat-model.md) (T-08, T-11).
- If a field is added to an observation, an explanation or an error, the sentinel test covers it before it is integrated.
