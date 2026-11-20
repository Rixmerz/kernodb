# Stage 5 — Normalized footprint and explainable plan (E5-01)

Contract of E5-01 ([Stage 5 minutes](stage5-record.md) §3, decisions of §8; [ADR-0015](../adr/0015-privacy-of-observability.md), accepted with the evidence of E5-04). The code is in `crates/kerno-units/src/fingerprint.rs` and `plan.rs`. This package **does not change any query**: `query` and `relations_of` behave the same; the planner that uses these types is E5-02.

## 1. Footprint

A query is reduced to its **shape**: which conditions it fixes, with each literal replaced by a typed marker.

| Query | Shape |
|---|---|
| `UnitFilter` with no conditions | `units[]` |
| `UnitFilter { unit_type, schema_version }` | `units[unit_type=?text,schema_version=?u32]` (only the conditions fixed, in that order) |
| Relations of a unit, outgoing | `relations[out:unit=?unit_id]` |
| With relation type, incoming | `relations[in:unit=?unit_id,relation_type=?text]` |

The direction is structure (two possible values), not a literal. The markers are `?text`, `?u32` and `?unit_id`.

The footprint is the **keyed** hash of the shape, with the algorithm and version recorded:

```text
BLAKE3-keyed(key, "kerno.query-fingerprint.v1" ‖ 0x00 ‖ shape), first 16 bytes
```

It is shown as `units/blake3-keyed/v1/<32 hex digits>`. Changing the algorithm, the label, the truncation or the grammar of shapes requires raising `FINGERPRINT_VERSION`; the fixed vectors of `tests/fingerprint.rs` detect it.

**Key** (minutes §8, decision 4). `FingerprintKey` is local to the process and does not leave it: it has no access to its bytes, `Debug` hides them, it is not serialised and it is not comparable with `==` (a secret does not receive a variable-time comparison; footprints are compared). `generate()` reads 32 bytes from `/dev/urandom`; `from_bytes` exists for reproducible tests. Two footprints are only comparable under the same key, which is what is wanted: a shape has few possible values and an unkeyed hash would fall to a dictionary attack.

**What is shown at each level** (ADR-0015): the footprint (`Fingerprint`) is what the `aggregate` and `fingerprint` levels carry. The text of the shape (`NormalizedQuery::text`) contains no literals, but it is the structure of the query: only the `diagnostic` level.

**Vectors.** The digests in `tests/fingerprint.rs` (key `00 01 … 1f`, eight shapes) were computed with an independent implementation of BLAKE3 written from the specification, validated against the official vector of the keyed mode (empty input, key `whats the Elvish word for friend`), and were not read from this code. Both implementations agree.

## 2. Plan and explanation

A `Plan` says how a query will be answered and why; **it is** the explanation (there is no second structure to keep in agreement). It contains the footprint of the query, never the query: neither literals nor shape text.

| Query | Candidates (tie-break order) |
|---|---|
| Units | `UnitsReference`, `IndexPrimary`, `IndexType` |
| Relations | `RelationsReference`, `Adjacency` |

These are only the paths that already exist (minutes §2.2). Each candidate carries its estimated cost and whether it is usable (`Usable` or `Unusable(IndexNotReady(status) | NoTypeFilter)`). The plan also stores the chosen candidate, the reason (`LowestCost` or `Fallback { index }`) and the freshness status of the index that the decision saw (I-23; `None` in relation queries, whose adjacency entries are written by the same change set as the relation, ADR-0014).

`Plan::new` rejects what cannot be explained truthfully:

1. the candidates are **exactly** those of the table, once and in order;
2. a units query carries index status and a relations query does not;
3. **no index that is not `Ready` is declared usable** (I-23): with the index `Stale`, `Building` or `Failed`, both index paths are `Unusable(IndexNotReady(that status))`; with `Ready`, only `IndexType` can be unusable, and only for `NoTypeFilter`;
4. the reference and adjacency paths are always usable (they are the fallback, I-32), so there is always an answer;
5. the choice is the usable candidate of lowest cost and, on a tie, the first in the table; the reason is **derived** (`Fallback` if the index was not `Ready`, `LowestCost` if it was), it is not received.

`Cost` is a relative quantity (entries the path would read), comparable only within one plan. How it is estimated, with which statistics and the generation of candidates belong to E5-02; this package adds no statistics.

## 3. Tests and scope

`tests/fingerprint.rs` (7 tests) and `tests/plan.rs` (10 tests): fixed vectors, absence of sentinel literals in shape, footprint and plan, key, complete listing of candidates, tie-break, fallback with each freshness status, each rejection of `Plan::new` and a case over a real database (`Stale` and then `Ready`).

It does not cover: the sentinel test over every observability output and error at each level (E5-04), the real choice of paths and the differential test against the reference (E5-02), nor any recording of observations (E5-03).

ponytail: no `Display` for the plan (the fields are the structured explanation; a stable text will be fixed by E5-03/E5-04 once there is a consumer); the key is not erased when dropped and `generate()` exists only on Unix. Upgrade path: `getrandom` and `zeroize` if the key comes from a single trusted process.
