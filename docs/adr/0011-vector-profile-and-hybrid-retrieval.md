# ADR-0011 — Vector profile and hybrid retrieval

| Field | Value |
|---|---|
| Status | Accepted (2026-11-11, by delegation of the owner; revocable) |
| Date | 2026-10-27 (opening); 2026-11-11 (accepted in E7-00) |
| Deciders | Project owner (delegation of 2026-11-11, [minutes of Stage 7](../architecture/stage7-record.md) §7.1 and §8) |
| Related to | SAD §10, D-08, invariant I-25; [minutes of Stage 7](../architecture/stage7-record.md) §2.3, §4, §7; [evaluation set](../architecture/stage7-evaluation-set.md); ADR-0007, ADR-0004, ADR-0018, ADR-0009; invariants I-25, I-31 |

## Context

Hybrid retrieval combines text, vectors and filters. Vectors are derivatives of a source revision and of a model profile.

**Question:** Which common contracts define embeddings, metrics, filters and index freshness.

Stage 7 needs an optional vector signal for ranking (E7-04) without fixing anything that is hard to reverse. A stored vector would be a derivative (I-25): it would have to be tied to its source revision and to its model profile, and its freshness depends on ADR-0018, which is decided before Stage 8. That is why this stage does not persist vectors.

## Options considered

1. **Exact search (brute force) first** as the recall baseline.
2. **ANN (HNSW or other) from the start**.
3. **Delegate to an external library** for vector search.

## Initial proposal (SAD §15)

Exact vectors first; model and source-revision metadata; generation through adapters; ANN only with evidence.

## Decision criteria

Comparison against an exact baseline, verification of dimensions/metric, and an invalidation test after updating/deleting the document.

Criteria set in the [minutes of Stage 7](../architecture/stage7-record.md) §7 before measuring: (a) an option cannot fix persistence assumptions that Stage 7 does not need; (b) the core does not depend on a network or a model (I-31); (c) ANN only with a benchmark that justifies it.

## What M0 contributed

The `agent_memory` workload stores embeddings of 768 × f32 (3 KiB) as ordinary values and measures writing, reading and space on both candidates. Values of this size are relevant to ADR-0001 (overflow pages in the B+ tree, large blocks in the LSM).

## Evidence

- Acceptance in E7-00 with the evaluation set frozen and its baselines B0 and B1 ([evaluation set](../architecture/stage7-evaluation-set.md), §4). Exact search does not exist yet: its measurement (recall against §6.2 of the minutes) comes in E7-04 and E7-05.
- No change to the persistent format in Stage 7. The E7-00 code computes no similarities and stores no vectors.

## Decision

**Option 1: exact search first**, with this limit for Stage 7:

- **Vectors are supplied by the caller and live only in memory.** The core does not generate embeddings or call models (I-31). Nothing is written to the unit format or to the indexes.
- **Dimension contract.** A vector set has a fixed dimension, greater than zero. A vector of another dimension, or a query of another dimension, is rejected with an error; there is no padding or trimming.
- **Metric contract.** Two metrics: cosine and dot product. A query with a metric different from the set's is rejected. A vector with non-finite components is rejected. With cosine, a zero-norm vector is rejected, because its cosine is not defined.
- **Persisting vectors as derivatives tied to their source revision (I-25) is left for Stage 8**, with ADR-0018 (versioning of derivatives) and ADR-0009 (suppression). No change to the persistent format in this stage.
- **No ANN without a benchmark.** An approximate index enters only with a measurement that justifies it and with an ADR that accepts it.

## Consequences

- **Easier:** exact search is correct by construction and serves as the recall reference for any future index. The dimension and metric contracts are small and are tested in E7-04.
- **Harder:** vectors are recomputed on every query or are supplied by the caller on each use. For large sets the linear cost can be high; the measurement of §6.3 of the minutes (20 000 units) records it without a threshold.
- **Committed:** the vector query API receives the vectors as an argument and does not read them from the store. Changing that (persisted vectors, ANN) is a change of decision and requires an ADR that replaces this one; an accepted ADR is not edited to change it.
- **Review:** when ANN or vector persistence (Stage 8) is decided. If the E7-05 measurement shows that exact search does not meet an agreed latency limit, that opens a new decision, not an edit of this one.

## Follow-up

- E7-04: implement the dimension and metric contracts and exact search; rejection tests by dimension and by metric.
- E7-05: measure the exact vector signal on the frozen set against §6.2 and record the cost of §6.3.
- Stage 8: decide vector persistence with ADR-0018 and ADR-0009 before writing any derivative.

## What was kept from the opening ADR

Options 2 and 3 are discarded for Stage 7: ANN needs a benchmark that does not yet exist (§2.3 of the minutes), and an external vector-search library would add a dependency that the core does not need for exact search.
