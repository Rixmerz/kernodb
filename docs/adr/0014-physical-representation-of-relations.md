# ADR-0014 — Physical representation of relations

| Field | Value |
|---|---|
| Status | Accepted (option 2; evidence from E4-02) |
| Date | 2026-10-28 (proposal); 2026-11-05 (accepted with the evidence of E4-02) |
| Deciders | Project owner (approved the semantics of criterion 4 in E4-00); ratification of this decision in the minutes of E4-02 |
| Related to | [specification v0.2.0](../spec/architecture-specification-v0.2.0.md) §6.4, §10, §12; SAD §9 (table of structures, row 7); ADR-0001, ADR-0004, ADR-0007, ADR-0010; invariants I-21, I-39 |

## Context

Stage 4 introduces typed relations (`Edge`) between units: `edge_id`, `from_unit_id`, `to_unit_id`, `relation_type`, optional weight and time validity, provenance and attributes. The specification asks not to assume that all relations are directed or weighted, and to separate declared relations from inferred ones. The SAD (§1 and §20, "Outside the first batch") excludes a graph language and reserves a specialized graph engine for when a workload justifies it.

The physical engine is a leveled LSM (ADR-0001) with atomic batches, snapshot isolation and range scans. The question is how to encode relations on that engine.

## Options considered

1. **Relations as units** with a reserved type, without an adjacency index. Minimal surface; walking neighbours requires a filter over all relations.
2. **Relation record + transactional adjacency indexes** (proposal). One record per `edge_id` and two index entries with ordered keys `out/{from}/{type}/{edge_id}` and `in/{to}/{type}/{edge_id}`, written in the same batch as the record. Neighbours by range; cost: three writes per relation.
3. **Adjacency lists per unit** (one value with all neighbours). Neighbour reads in a single key; full rewrite of the list on each change and write conflicts on nodes with many relations.
4. **Dedicated graph engine**. Out of scope until a benchmark justifies it (SAD, spec §5).

## Decision criteria

Set on 2026-11-05, **before** the full measurement of "Evidence". The five criteria are those of the proposal; E4-02 makes them measurable with a threshold or a test that decides each one. The bench was run earlier in `--quick` mode only to debug it; its results were not used to choose the thresholds. An option that fails a correctness criterion (1, 2, 4, 5) is not accepted, whatever its performance; cost criteria (3) are judged between options.

1. **Atomicity.** The record of a relation and its index entries are committed on the same boundary as the rest of the change (I-21). It is met if, in a sweep of crashes before each mutating operation (power loss and process death, 4 seeds, `Emulated<LogEngine>` and `KernoDb`), the recovered store is always that of a whole prefix of `ChangeSet` (byte by byte against a fault-free oracle) and `verify_units` reports no findings: **zero** states with the record without its entries or the reverse.
2. **Equivalence.** Walking neighbours through the index returns exactly what the reference path returns (filter over all records, no index) over the same snapshot (I-32). It is met with **zero** differences, for every unit, direction and type, after each step of seeded random loads (inserts, deletes and aborts), with more relations per unit than a read page (256) and on old snapshots; and in the bench, 20 nodes × 1 and 2 hops between the two options.
3. **Cost.** On two synthetic loads with the shape of the roadmap workloads, both with 5 000 units: `livespec` (symbols and specifications; out-degree 5; half of the relations point to 5 % of the units) and `memory` (agent memory; chain plus links to recent elements; degree 3, without hubs):
   - (a) Write per relation: keys, WAL bytes and bytes dumped of option 2 **≤ 1.5×** those of option 1.
   - (b) Read: p99 of walking 1 and 2 hops of option 2 **≤** that of option 1 on the two loads, and p99 of 1 hop of option 2 **≤ 1 ms**.
   - (c) Effect on what is already approved: with relations active, U2 and U6 are re-measured **with the thresholds unchanged** (U2: p99 of `history` ≤ 1.5× that of the reference; U6: p99 and throughput of writes during pruning no worse than before, with the same load) and do not come out worse than the range already documented in the Stage 3 report (U2 0.95×–1.67×; U6 write p99 ≤ 1.59× and throughput ≥ 0.76×). A previous failure (K-E3-8, K-E3-9) is not attributed to relations if it stays within that range.
4. **Archive or delete with relations.** Explicit semantics, approved by the owner in E4-00: **rejection**. It is met if archiving and deleting a unit with a live relation, through either of its two ends, fails with an error that names the unit and a relation, with no effect on the store and without breaking the `ChangeSet`, in `Emulated<LogEngine>` and `KernoDb`; and if deleting the relations first (even in the same `ChangeSet`) allows archiving or deleting.
5. **Provenance (I-39).** Every relation carries a mandatory provenance field (`declared`, `imported`, `algorithmic`, `inferred`) that cannot be omitted (no default value), with origin for all except `declared` and confidence only for `inferred`. It is met if the writer and the decoder reject every incoherent combination, and if an inferred relation is never returned as declared by either read path, not even with the same ends and type as a declared one.

## Evidence

Prototype and measurements of E4-02 (2026-11-05), in `kerno-units` over `kerno-storage` (`KernoDb`, default options: `Sync`, 64 MiB cache, 8 MiB write buffer, real file system), a single machine, with no other deliberate loads (average load of 1.0 at the start). Code: relations in `crates/kerno-units/src/db.rs`, `codec.rs` and `reference.rs`; benches `examples/relation_bench.rs` (option 1 against option 2) and `examples/unit_bench.rs --relations` (U2 and U6).

**What was built.** Option 2: `create_relation` writes `REL` and its entries `REL_OUT` and `REL_IN` in the same `ChangeSet`; `delete_relation` deletes them together; `relations_of` reads the adjacency by range and checks each entry against its `REL`; the reference path (`reference_relations`, `reference_relations_of`) walks all `REL` records and never an index entry. Option 1: a relation is a unit of a reserved type with the payload `from ‖ to ‖ provenance ‖ type`; neighbours are obtained by filtering all of them (`reference_by_type`); in 2 hops it reads all of them once and filters the copy twice, which is its best case. Format: [units-v1.md](../file-format/units-v1.md) §3.7–§3.8 (tags `0x40..=0x42`, rule UNIT-36).

**Criterion 3 (a) and (b), complete `relation_bench`** (5 000 units; `livespec` 25 000 relations, `memory` 15 000; queries: 2 000 per hop and load in option 2, 200 in option 1, after a 10 % warm-up):

| | Option 1 (unit per relation) | Option 2 (record + 2 indexes) | Option 2 / option 1 |
|---|---|---|---|
| Keys written per relation | 4.00 | 3.00 | 0.75× |
| WAL per relation | 351.2 B | 198.2 B | 0.56× |
| Dump to tables per relation | 382.9 B | 221.3 B | 0.58× |
| `livespec` 1 hop: p50 / p99 | 87.10 / 130.64 ms | 0.0074 / 0.0244 ms | ≈ 1/5 000 (p99) |
| `livespec` 2 hops: p50 / p99 | 86.34 / 123.00 ms | 0.0416 / 0.1024 ms | ≈ 1/1 200 (p99) |
| `memory` 1 hop: p50 / p99 | 55.90 / 73.96 ms | 0.0049 / 0.0119 ms | ≈ 1/6 200 (p99) |
| `memory` 2 hops: p50 / p99 | 57.31 / 89.77 ms | 0.0162 / 0.0494 ms | ≈ 1/1 800 (p99) |

Option 1 writes *more* because each relation inherits the unit's machinery (head, `REV_META`, `REV_PAYLOAD` and membership in the `ChangeSet`); option 2 writes three small keys. Compaction: 0 bytes per relation in both at this size (everything fits in the dump). Both sides returned the same neighbours in 20 nodes × 1 and 2 hops in each load.

**Criteria 1, 2, 4 and 5, tests** (all pass in `Emulated<LogEngine>` and `KernoDb`; run: `cargo test -p kerno-units --test relations --test crash`):

- Criterion 1: `crash::{emulated_log,kerno}::relations_every_point_{power_loss,process_kill}`, 49 points per seed × 4 seeds; store equal to the oracle in all, `verify_units` clean, and the recovered database accepts a relation and its deletion. The test was seen to fail when compared against the wrong prefix. Rows R1–R5 of the [failure matrix](../architecture/failure-matrix.md) §8.
- Criterion 2: `relations::differential_seeded` (4 seeds × 40 steps, 12 units, 3 types where one is a prefix of another, with aborts), `differential_across_scan_pages` (600 relations over 3 units) and the bench comparison: 0 differences.
- Criterion 4: `relations::archive_and_delete_are_rejected_with_live_relations`: rejection through both ends, error `UnitHasRelations` that names the unit and the relation, the `ChangeSet` remains usable, and archiving or deleting after deleting the relations in the same `ChangeSet` works. The test was seen to fail when its assertion was inverted.
- Criterion 5: `relations::provenance_is_stored_and_checked` and `declared_is_never_confused_with_inferred` (also at byte level: changing the provenance type of an inferred record to declared or imported makes it undecodable).

**Criterion 3 (c): U2 and U6 re-measured, thresholds unchanged** (complete `unit_bench`: same sizes and method as the Stage 3 report; `--relations` adds 3 outgoing relations per unit before measuring and, in U6, one relation per `update` of the writers, in the same `ChangeSet`). Full scale, **not reduced**; U2 in 3 runs per mode, U6 in **1** run per mode (≈ 5 min each; not repeated).

| U2: p99 `history` / reference (threshold ≤ 1.5×) | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| No relations | 0.0780 / 0.0517 ms = **1.51×** (fails by 0.01) | 0.0771 / 0.0586 = 1.31× | 0.0663 / 0.0447 = 1.48× |
| With relations | 0.0781 / 0.0626 = 1.25× | 0.0755 / 0.0518 = 1.46× | 0.0800 / 0.0543 = 1.47× |

U2 stays within the range of the Stage 3 report (0.95×–1.67×), with the same oscillation around 1.5× (K-E3-8); `history_page` 0.70×–1.00× of the reference, informative. Relations are not on the path of `history`.

| U6 (retention `KeepLast(1)` with 2 writers and 1 reader) | p99 write during / worst "before" | Write throughput during / lowest "before" | p99 read during / worst "before" | Retention | Verdict |
|---|---|---|---|---|---|
| Default budget, no relations | 2.802 / 2.908 ms = 0.96× | 2 002.5 / 2 283.0 = 0.88× | 0.0225 / 0.0081 = 2.77× | 16 888 rev/s | fails |
| Default budget, with relations | 2.399 / 2.237 ms = 1.07× | 2 316.8 / 2 405.2 = 0.96× | 0.0097 / 0.0074 = 1.30× | 21 633 rev/s | fails |
| 25 units, no relations | 2.769 / 2.567 ms = 1.08× | 2 188.6 / 2 369.9 = 0.92× | 0.0317 / 0.0124 = 2.55× | 12 946 rev/s | fails |
| 25 units, with relations | 2.640 / 2.182 ms = 1.21× | 2 210.2 / 2 362.7 = 0.94× | 0.0260 / 0.0244 = 1.07× | 12 796 rev/s | fails |

**U6 still fails (K-E3-9), with and without relations, as at the close of Stage 3.** With relations active, the write p99 (1.07× and 1.21×) and the throughput (0.96× and 0.94×) stay within the already documented range (p99 ≤ 1.59×, throughput ≥ 0.76×); no worsening is attributed to relations, but with one run per mode and "before" windows that vary 2–3× among themselves as in the Stage 3 report, the difference between "with" and "without" (0.96× against 1.07×, etc.) is within noise and does not show that relations are free in U6. No threshold has been lowered and none is declared met.

**Limits of this evidence.** Synthetic loads of 5 000 units and 15 000–25 000 relations, not the real workloads of LiveSpec or agent memory (they do not exist yet); the highest input degree is about 50 per hub, far from the 10⁵ relations of "Conditions for reopening", which have **not** been measured. Option 1's latencies depend on reading the whole set of relations; an option 1 with an added index is, in practice, option 2. Option 3 (adjacency lists per unit) and option 4 were not prototyped: they are discarded on the argument of the proposal (full rewrite of the list on each change and conflicts on nodes with many relations), which this evidence neither contradicts nor confirms.

## Decision

**Option 2: relation record plus transactional adjacency indexes**, accepted on 2026-11-05 with the evidence of E4-02. It meets the five criteria; option 1 meets 1, 2, 4 and 5 by construction but loses on 3 (b) by four orders of magnitude and writes more per relation (3 (a)). What is decided:

- A relation is directed, with `edge_id`, `from`, `to`, `relation_type` (1..=64 B), `created_seq` and mandatory provenance (`declared`, `imported`, `algorithmic`, `inferred`; origin except in `declared`; confidence in thousandths only in `inferred`). The actor, the origin and the reason are those of the `ChangeSet` that created it.
- The record and the two index entries are always written and deleted in the same `ChangeSet`.
- Archiving or deleting a unit with live relations, through either of its ends, is **rejected** (`UnitError::UnitHasRelations`); creating a relation requires two `Active` units in the `ChangeSet`. It can be relaxed to cascade or to a marked dangling relation without changing the format (decision of the owner, E4-00).
- A relation **has no revisions of its own**: creating or deleting it is a change of a `ChangeSet`, and that `ChangeSet` is the only footprint (it resolves the question of ADR-0004 for this stage). Optional weight and time validity are **not** included in E4-02; adding them is a new field of the record and requires deciding the interaction with history.
- Relations count against the operation limit of a `ChangeSet`, and a `ChangeSet` that only creates or deletes relations is a commit (it is not empty).

## Consequences

- Adjacency keys become part of the logical key schema; their encoding is versioned together with the rest of the logical format (ADR-0003, ADR-0008). E4-02 defines them in [units-v1.md](../file-format/units-v1.md) §2, §3.7 and §3.8 **without raising `layer_version`** (it stays at 1): they are new tags (`0x40..=0x42`) that an earlier layer rejects as `IncompatibleLayer` (UNIT-2) when verifying, but an earlier layer that opened the store without walking it would not know about the rejection of archiving with relations. While there are no v1 stores in use outside development this is acceptable; before the layer's format is frozen, the owner must decide whether that is enough or whether `layer_version` must be raised to 2.
- Relations with time validity interact with history (ADR-0004): for now the relation has no revisions of its own and no time validity (see "Decision").
- A relation to a suppressed unit is subject to ADR-0009. With the rejection of E4-02 there cannot be a relation to an archived or deleted unit, so physical suppression has no dangling relations to handle while that rejection is kept.
- Deleting a unit requires two range reads of an entry (one per index) inside the `Txn`; the cost of relations on the write path of a `ChangeSet` without relations has not been measured separately from U1/U2/U6.
- The adjacency indexes are **primary** (they are updated in the same `ChangeSet`, ADR-0007), not secondary indexes with freshness state: E4-03 needs no watermark for them. `UnitSnapshot::relations_of` checks each entry against its `REL`.

## Conditions for reopening

Workloads where walks of more than two hops dominate, or nodes with more than 10⁵ relations where the cost of option 2 is unacceptable.

## Follow-up

- ~~Prototype and benchmark at the start of Stage 4.~~ Done in E4-02 ("Evidence").
- ~~Property test: adjacency indexes consistent with the records after failures (failure matrix).~~ Done: [failure matrix](../architecture/failure-matrix.md) §8, rows R1–R5.
- Repeat U6 (several runs, with and without relations) when K-E3-9 is addressed; measure a node with 10⁵ relations before the "Conditions for reopening" are considered discarded.

## Note on E4-00 (2026-11-04)

It was *Proposed* on that date. The final decision comes from the E4-02 prototype, which accepted it on 2026-11-05 (option 2). Semantics approved by the owner for archived or deleted units with live relations: **rejection** of the operation (relaxable later to cascade or to a marked dangling relation).
