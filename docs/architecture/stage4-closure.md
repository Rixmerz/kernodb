# Closure of Stage 4

| Field | Value |
|---|---|
| Status | **Approved by the owner (2026-11-05)** |
| Date | 2026-11-05 (draft of E4-05) |
| Act | Approval of the closure of Stage 4 by the project owner |
| Scope | Packages E4-00 to E4-05: declared relations, reference path, primary index and the secondary index `IDX_TYPE`, in the crate `kerno-units` |
| Documents | [Minutes of Stage 4](stage4-record.md) · [workloads](stage4-workloads.md) · [invariants §2, §2.2, §2.3](invariants.md) · [failure matrix](failure-matrix.md) · [`units-v1.md`](../file-format/units-v1.md) · [ADR-0007](../adr/0007-index-consistency-and-rebuilding.md) · [ADR-0014](../adr/0014-physical-representation-of-relations.md) · [roadmap](../roadmap/README.md#stage-4--relations-and-indexes) · [closure of Stage 3](stage3-closure.md) |

This document describes the stage as it stands in `main` after E4-04 (`e2d16f6`, PR #20), plus the corrections from the adversarial review of E4-05. **The closure is not effective until the owner approves it** (§7).

## 1. Work packages delivered

| WP | Delivery | PR | Status |
|---|---|---|---|
| E4-00 | Minutes approved; ADR-0007 accepted; I-32, I-23 and I-24 assigned to E4-03; rejection of archiving or deleting units with relations | #16 | done |
| E4-01 | Reference path without indexes over `UnitSnapshot` (`reference_filter`, `reference_relations*`) | #17 | done |
| E4-02 | Declared relations (ADR-0014, option 2: `REL` and adjacencies `REL_OUT`/`REL_IN` in the same `ChangeSet`), mandatory provenance, UNIT-36, crash sweeps, U2 and U6 re-measured | #18 | done (ADR-0014 accepted, option 2) |
| E4-03 | Primary index `IDX_PRIMARY` with `IDX_META` (status and watermark), `rebuild_index`, `indexed_scan`, `query`, UNIT-37 | #19 | done |
| E4-04 | Workload W1 written before measuring; justifies `IDX_TYPE` (UNIT-38) | #20 | done |
| E4-05 | Adversarial review, relation vectors, promotion of invariants, this document | #21 | done, approved |

## 2. Acceptance criteria (minutes §4)

| Criterion | Evidence | Status |
|---|---|---|
| Results per index match the reference path (I-32 applied to indexes) | `tests/index.rs`: `differential_seeded` (6 seeds × 2 engines × 40 steps; since E4-05 the workload includes `restore` and `run_retention`), `differential_across_scan_pages`; `check` compares `IndexPath::Primary` and `IndexPath::ByType` with `reference_filter` and with an independent model; `tests/relations.rs`: `differential_*` (adjacency against reference). Mutations in [invariants §2.2](invariants.md) | **met** |
| A rebuilt index is equivalent to a new one (I-24) | `tests/index.rs`: `rebuilt_equals_fresh` (entries byte by byte, `IDX_PRIMARY` and `IDX_TYPE`) | **met** |
| The freshness status is exposed (I-23) | `tests/index.rs`: `states_and_watermark`, `building_is_reported_while_a_build_waits`, `snapshots_keep_their_own_status`, `failed_build_is_recorded_and_repaired`, `status_survives_reopen`, `index_every_point_{power_loss,process_kill}`; UNIT-37/38; [failure matrix](failure-matrix.md) §9 | **met** |
| Each secondary index has a documented workload and equivalence with the reference path; no index without a workload | [stage4-workloads.md](stage4-workloads.md) (W1 → `IDX_TYPE`); differential tests per `IndexPath::ByType` | **met** |
| A declared relation is never confused with an inferred one (I-39) | `tests/relations.rs`: `provenance_is_stored_and_checked`, `declared_is_never_confused_with_inferred`, `stored_provenance_corruption_is_detected` (new in E4-05); `rel_*` vectors in `tests/struct_vectors.rs`; mutations in [invariants §2.3](invariants.md) | **met** (declared part; the inferred one belongs to Stage 7) |
| Relations and indexes on the atomic boundary (I-21, E4-02 minutes) | `tests/crash.rs`: `relations_every_point_{power_loss,process_kill}`; `tests/relations.rs`: `abort_leaves_no_relation`, `relation_only_changeset_commits`; relation rows of the [failure matrix](failure-matrix.md) | **met** |
| U2 and U6 re-measured without changing thresholds (E4-02 minutes) | ADR-0014, "Evidence", criterion 3 | **met with inherited failures** (K-E3-8 and K-E3-9 stay open; see §5) |

Correction gate: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` green at the closure of E4-05 (1,212 tests before, 1,214 after).

## 3. Adversarial review (E4-05)

Method: the same as in Stage 3. For each invariant of the stage the code that enforces it and a test that fails if it breaks were located, and a mutation was applied to check it (tables in [invariants §2.2 and §2.3](invariants.md)). The review **found no blockers**. It found these gaps, corrected in E4-05:

| # | Gap | Correction |
|---|---|---|
| H-1 | The independent decoder (`tests/spec_decoder`) treated `REL` (`0x40`) and `REL_OUT`/`REL_IN` (`0x41`/`0x42`) as UNIT-2, and there were no committed vectors for relations: the format of E4-02 did not have the double implementation that the other records have | `spec_decoder` decodes the three according to `units-v1.md` §2, §3.7 and §3.8; 10 positive and 25 negative `rel_*` vectors in `struct_vectors.rs` (UNIT-3, 6, 9, 10, 11, 12, 13, 23), checked against the production decoder, also by truncation and by inverted bytes |
| H-2 | `units-v1.md` §6 did not say that the consistency of the provenance of `REL` (UNIT-13) is checked after UNIT-10, as `codec.rs` does; nor that a `REL_OUT`/`REL_IN` without the NUL separator is UNIT-3 | §6, point 2, and key rules of §2 |
| H-3 | I-39: no test damaged the provenance already stored and read it through the API | `relations::stored_provenance_corruption_is_detected` (`get_relation`, `relations_of`, `reference_relations` and `verify_units` reject with UNIT-13) |
| H-4 | The differential workload of indexes had neither `restore` (which changes the head's `schema_version`) nor `run_retention` (a commit that does not change heads) | `random_step` of `tests/index.rs` includes them, with the model of retained revisions |
| H-5 | I-21 (relations), I-23, I-24, I-32 and I-39 without the formal promotion or the tests of E4-02/E4-04 cited | Rows updated in `invariants.md` §2; mutation table §2.3 |

## 4. Known minor defects accepted

None affects the correctness of the delivered results; they are recorded so they are not lost.

| # | Matter | Why it is accepted |
|---|---|---|
| M-1 | `created_seq` of a `REL` is not checked against the `CHANGESET` of that sequence (UNIT-36 only requires that it is not later than the snapshot) | The relation has no revisions; a false `created_seq` changes no answer. Review with the verification CLI (Stage 9) |
| M-2 | An `IDX_META` with a watermark later than the snapshot also triggers the other findings of UNIT-37 in cascade | It is corruption and is reported; only noise is added to the report |
| M-3 | `active_entries` (index construction) reads only the heads | By design (invariants §2.2, "Known limitation") |
| M-4 | The indexes built by E4-03 do not carry `IDX_TYPE`, and `IDX_META` does not record the generation | The format of Stage 4 has not been published; E4-03 and E4-04 are a single format (`units-v1.md` §3.11). If it is published, `IDX_META` will need a generation version |
| M-5 | A missing `IDX_TYPE` entry is not seen when querying (only `verify_units`, UNIT-38) | Same as M-3: a query verifies what it returns, not what it discards |
| M-6 | The `Ord` of `Key` reserves memory when comparing | Cost, not correctness |
| M-7 | The query by type alone has a p95 of 17 ms with 100,000 units (W1), above the 10 ms of the W1 criterion | The ideal bound (opening the 999 results) already costs 12 ms; no index goes below that ([stage4-workloads.md](stage4-workloads.md)) |
| M-8 | The W1 workload document was integrated in the same change as the implementation of `IDX_TYPE` | The W1 thresholds were fixed before measuring and the "before" measurements are committed separately; it is noted here for traceability |
| M-9 | **No raw output of the E4-02 bench is committed** (`relation_bench`, `unit_bench --relations`): the figures appear only in ADR-0014, "Evidence". **U6 (K-E3-9) was not re-measured with active indexes**: E4-03 and E4-04 did not run `unit_bench`; the index is derived and no application `ChangeSet` writes it, but the measurement that the minutes asked for (§5) does not exist | Accepted as a gap in evidence, not in correctness. Re-measuring U6 with indexes and relations and committing the output before or at the start of Stage 5 is recommended |
| M-10 | A concurrent `rebuild_index`: a failing attempt can write `Failed` over a `Ready` that another attempt had just written | Conservative: the index is declared unusable when it was usable, never the other way round (I-23 holds); a rebuild repairs it |
| M-11 | Intermittent test `kerno::retention_every_point_paced_power_loss_sync` (predates Stage 4) | Reproduced in E4-05: 1 failure in 6 runs of `cargo test -p kerno-units --test retention`, at the final assertion (`tests/retention.rs:570`, "no power loss took back a deferred retention; the sweep proved nothing"). It is the check that the sweep is meaningful, not a data loss: in that run no power cut took back a deferred retention, which depends on the timing of the background rotations and flushes of `KernoDb`. There is no small, clearly correct fix; it is recorded |

## 5. Risks

| ID | Risk | Status at closure |
|---|---|---|
| K-E3-8 | U2 fails on `history` | **Open**, unchanged: re-measured in E4-02 in the same range (ADR-0014, "Evidence"); relations are not on the path of `history` |
| K-E3-9 | U6 fails with concurrent retention | **Open**: re-measured in E4-02 with relations (1 pass per mode), within the documented range; **not re-measured with indexes** (M-9) |
| K-E3-2 | Single write slot | Open; each relation adds three writes inside the slot |

## 6. Open decision for the owner

- **ADR-0007: whether the indexes are maintained inside each `ChangeSet`.** Today the primary index and `IDX_TYPE` are derived: only `rebuild_index` writes them and any later commit leaves them `Stale`, with `query` falling back to the reference path. W1 measured queries over a freshly built index; it **provides no evidence** about the cost of maintaining them in each `ChangeSet` nor about how long they stay `Stale` under a mixed load. The decision requires a mixed read and write workload, which does not exist yet. It does not block the closure.

## 7. Closure

| # | Closure condition | Status |
|---|---|---|
| 1 | Criteria of the minutes §4 met, with evidence (§2) | done |
| 2 | Adversarial review with no open blockers (§3) | done |
| 3 | Promotion of I-21 (relations), I-23, I-24, I-32 (indexes) and I-39 (declared) in [invariants.md](invariants.md) | done, **approved by the owner on 2026-11-05** |
| 4 | Roadmap, SAD §9 and minutes updated | done in the change that incorporates this document |
| 5 | Owner approval of the closure, of the promotion and of the accepted minor items (§4) | **pending** |

## 8. Next stage

Stage 5 (planner and observability) starts only when the owner approves this closure. It inherits K-E3-8, K-E3-9, the open decision of §6 and the recommendation of M-9.
