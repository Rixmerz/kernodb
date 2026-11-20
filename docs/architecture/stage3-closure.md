# Closure of Stage 3

| Field | Value |
|---|---|
| Status | **Closed** |
| Approval date | 2026-11-04 |
| Act | Approval of the closure of Stage 3 by the project owner |
| Scope | Packages E3-00 to E3-10: units, revisions and provenance, in the crate `kerno-units` |
| Documents | [Design of Stage 3](stage3-design.md) · [minutes E3-00](stage3-record-e3-00.md) · [invariants §2.1](invariants.md#21-invariants-of-the-units-layer-stage-3) · [failure matrix §7](failure-matrix.md) · [Stage 3 report](../testing/stage3-report.md) · [roadmap](../roadmap/README.md#stage-3--units-revisions-and-provenance) |

This document describes the stage as it stands in `main` after E3-09 (`b2ed16c`, PR #13), plus the corrections from the adversarial review of E3-10 (PR #14). The owner approved it on 2026-11-04.

## 0. Owner approval (2026-11-04)

The owner approved the closure of Stage 3 with these decisions:

- **Stage 3 closed.**
- **IU-1…IU-10 promoted and approved** ([invariants.md §2.1](invariants.md#21-invariants-of-the-units-layer-stage-3)).
- **I-22 and I-26 verified** (E3-10).
- **U2 and U6 fail.** They are integrated with the documented failure as open risks **K-E3-8** and **K-E3-9** (§5). No threshold was lowered and T1–T10 have not been touched.
- **K-E3-7 accepted by design**, with the recommendation to review it in Stage 9 together with the verification CLI.

## 1. Work packages delivered

| WP | Delivery | PR | Status |
|---|---|---|---|
| E3-00/E3-01 | Entry minutes (format v1 frozen, ADR-0013 accepted, decisions 1–8). ADR-0004 and ADR-0010 accepted. `units-v1.md` with encodings, `UNIT-n` rules and vectors. Crate `kerno-units` | #4 | done |
| E3-02 | Integrity of `REV_PAYLOAD`: length and BLAKE3-256 (UNIT-26…28). Minimal ADR-0008 accepted | #5 | done |
| E3-02b | Codecs for all records, error model, `Clock`/`UnitIdSource`, `UnitModel` and a decoder independent of the specification | #6 | done |
| E3-03 | `UnitDb`, `ChangeSet`, CRUD and archiving, limits, sequence prediction with poisoning | #7 | done |
| E3-04 | `history`, `get_revision`, `unit_as_of`, `state_as_of`, `change_sets_since`, `compare`, `restore` | #8 | done |
| E3-05 | Policies `none`/`audit`/`versioned` per type, `set_policy`, `HistoryUnavailable` | #9 | done |
| E3-06 | `RetentionPolicy`, `run_retention` in batches, `compact`, `RetentionReport` | #10 | done |
| E3-07 | `verify_units`, `verify_engine`, `verify_immutable` (UNIT-29…35), damage detection | #11 | done |
| E3-08 | Layer crash sweeps (`changeset_every_point`, `model_crash`, `recovery_reentrant`, `fsync_failure`, `enospc`, `torn_write`, `wal_independence`, real `kill -9`). I-20 and I-21 verified. Rows U1–U10 of the failure matrix | #12 | done |
| E3-09 | `unit_bench`, targets U1–U6, retention pace, `docs/testing/stage3-report.md` | #13 | done (U2 and U6 fail; integrated with the documented failure, see §4) |
| E3-10 | Adversarial review, promotion of IU-1…IU-10, stage closure | #14 | done (approved 2026-11-04) |

### Roadmap acceptance criteria

| Criterion | Evidence | Status |
|---|---|---|
| The evolution of a unit is reconstructed | `history::model_as_of`, `policies::policy_model` (history, as-of and `state_as_of` compared with `UnitModel`) | met |
| Restoring does not destroy later revisions (I-22) | `history::restore_keeps_later` | met; row I-22 of `invariants.md` verified |
| The history does not depend on the WAL (I-20) | `crash::wal_independence_{deleted,recycled}_wals` | met (E3-08) |
| Content and revision on the same atomic boundary, also in the failure matrix (I-21) | sweeps of `crash.rs`, `process_kill.rs`; failure matrix U1–U10 | met (E3-08) |
| Benches and metrics U1–U6 against the reference; T1–T10 untouched | `unit_bench` (3 passes, [report §8–§9](../testing/stage3-report.md)); T1–T10 before/after in `spikes/storage/results/2026-11-04-e3/t1-t10-e3-09c/` | met with accepted failures: U1 passes; U2 and U6 fail (risks K-E3-8 and K-E3-9); T1–T10 untouched and without regression |

## 2. Adversarial review (E3-10)

Method: for each invariant IU-1…IU-10 the code that enforces it and a test that fails if it breaks were located, and a code mutation was applied to check that. The detail is in [invariants.md §2.1](invariants.md#21-invariants-of-the-units-layer-stage-3). The mutations were applied one by one on `main` and reverted; none is left in the tree.

### 2.1 Defects found

| # | Defect | Where | Test that reproduces it | Status |
|---|---|---|---|---|
| D-1 | **IU-2 incomplete on the read and write path.** `load_unit` only compared "deleted or not" between the head and its `REV_META`. An `Archived` head over a `Create`/`Update` revision, or an `Active` one over `Archive`, was accepted. `get_unit` returned a lifecycle that had not been confirmed, and `unarchive`/`update` acted on it. `verify_units` did detect it (UNIT-31). | `crates/kerno-units/src/db.rs`, `load_unit` (comparison of `lifecycle`) | `unit_db::corruption_detected`, case "lifecycle" | **fixed**: `lifecycle_after(kind)` is compared, the same rule `verify` uses |

No other defects were found in the code. The other differences are **coverage gaps** (the invariant held, but no test failed when it was broken):

| # | Gap | Correction |
|---|---|---|
| H-1 | IU-1: only checked in the fixed scenario of `verify.rs`. An `Audit` rewrite that changed a field besides `payload_present` survived the whole suite. | `policy_model` calls `verify_immutable` in every round and after every retention |
| H-2 | IU-2: no test damaged the head's metadata, and the corresponding comparison in `load_unit` could be removed without anything failing. | Cases "metadata" and "lifecycle" in `corruption_detected` |
| H-3 | IU-10: the only test was the one for a decoder's message. Putting content into an API error or into a `verify` finding was not detected by anything. The tests `sentinel_no_content`, `immutability_sweep` and `payload_hash_checked` cited by the design did not exist under that name. | `verify::sentinel_no_content` |

## 3. Decisions of the stage

- Decisions 1–8 of §12 of the design: decided on 2026-10-31 ([minutes E3-00](stage3-record-e3-00.md)). A separate crate (`kerno-units`), a layer marker without an `incompat` bit, full copy per revision, a single copy of the current payload, versioned metadata, BLAKE3-256 (ADR-0008), and `AsOf::Time` is out of scope.
- Decision 12 of §12 (thresholds U1–U6): decided on 2026-11-01 ([design, §12](stage3-design.md#12-owner-decisions)). Its results are in §4.
- ADR-0004, ADR-0008 (minimal) and ADR-0010: accepted.
- Ambiguities resolved in each WP with the most conservative option (notes on E3-03…E3-07 of the design):
  - one revision per unit and `ChangeSet`;
  - `audit` keeps the hash and the length;
  - retention makes one `Retention` `ChangeSet` per call;
  - the members of `Retention` name the first retained revision;
  - `verify_units` only asserts IU-9 after the last `Maintenance`.
- **Still open** are decisions 9, 10, 11 and 13 of §12 of the design. None blocks the closure:
  - pruning of `CHANGESET` (decision 9);
  - `unit_id` and its relation to ADR-0005 (decision 10);
  - chunking of payloads (decision 11);
  - order of ADR-0009 relative to AI (decision 13).
- Whether U2 is judged on `history_page` instead of on `history` remains the owner's decision. It is tied to K-E3-8.

## 4. Performance results (E3-09)

Source: [`docs/testing/stage3-report.md`](../testing/stage3-report.md) §8–§9 and `spikes/storage/results/2026-11-04-e3/` (`unit_bench-full-e3-09c-run{1,2,3}.txt`). Three complete passes on a quiet machine; no threshold was lowered and the methodology of `unit_bench` did not change.

| Id | Result (passes 1 / 2 / 3) | Verdict |
|---|---|---|
| U1 (≤ 1.5×) | 1.05× / 1.09× / 1.10× | **passes** 3 of 3 |
| U2 on `history` (≤ 1.5×) | 1.50× (1.503) / 1.27× / 0.95× | **fails** (pass 1 exceeds the limit) |
| `history_page` | 0.90× / 0.88× / 0.58× | informational, no verdict |
| U3 (warning > 5×) | 5.03× / 4.66× / 4.78× | informational (warning in pass 1) |
| U4 | +339 B of WAL per `update`; amplification 2.67 (1 KiB) and 3.22 (64 KiB) against 1.10 and 1.78 for put | informational: structural cost of the full copy (decision 4) |
| U5 | 1,257.7 B per revision against 1,068.0 B (1.178×, +189.7 B) | informational |
| U6 | write p99 1.05–1.18×, writer throughput 0.89–0.94× (1.06× once) | **fails** 3 of 3 |

- **U2.** `history` oscillates around 1.5× because it materialises 100 `Revision`s and because of the variation in the reference's p99; the lightweight view `history_page` no longer adds its own cost. Judging U2 on `history_page` would change the definition of decision 12 and is left to the owner.
- **U6.** After E3-09c, retention does not do its own `fsync` (21,800–27,700 rev/s). What remains is the background work that pruning generates: rotations, flushes and compactions.
- **T1–T10.** Untouched and without regression beyond the noise between repetitions (report §8.2 and §9.2).
- **Owner decision (2026-11-04).** Integrate with the documented failure of U2 and U6, without lowering thresholds and without touching T1–T10. Both remain as open risks K-E3-8 and K-E3-9 (§5).

### 4.1 Defects corrected in the review of PR #13

- **Durability of an empty `ChangeSet`.** `ChangeSet::commit` with no operations returned `durable = (durability == Sync)`, although a deferred retention (decision 12c) could leave the current sequence unsynchronised. It is now durable only if the engine's `durable_seq` covers it (engines that do not report `durable_seq` sync every commit). With a new test.
- **Offsets of `history_page`.** `history_page` and `MetaFixed` store the offsets inside the page buffer as `usize` instead of `u32`, so a page of more than 4 GiB no longer truncates them.
- **Retention and rotation test.** A retention paused in `Sync` mode fills the memtable and rotates it, and the rotation syncs the WAL (one `wal_syncs`), which makes the earlier deferred sets durable, as decision 12c allows. The test now expects exactly one sync per rotation in `Sync` and none in the other modes.

## 5. Open risks

| ID | Risk | Status at closure |
|---|---|---|
| K-E3-1 | The prediction `commit_seq = read_seq + 1` fails | Mitigated: check after the commit with poisoning (`seq_mismatch_poisons_the_handle`). Still relevant if the engine comes to consume sequence on its own |
| K-E3-2 | The write slot is single: a long `ChangeSet` blocks everyone | Open. Bounded with `max_changeset_ops` and with batched retention. Multi-writer is out of scope |
| K-E3-3 | Amplification from writing several records per revision | Measured (U4/U5): +339 B of WAL per `update` and 1.178× space per revision. Accepted as the cost of the full copy (decision 4); no defect |
| K-E3-4 | Long snapshots nullify retention | Open. The report separates "logically freed" from "reclaimed". Batched retention with deferred durability (E3-09c) goes at 21,800–27,700 rev/s, but does not reclaim space while old snapshots exist |
| K-E3-5 | A layer built on pending ADRs is costly to reverse | Mitigated by opaque bytes, reserved `alg`/`kind` and `layer_version` |
| K-E3-6 | Logical deletion is confused with suppression | Open until ADR-0009 (decision 13) |
| K-E3-7 | Reads check less than `verify_units` checks. For example, a damaged intermediate `REV_META` is only seen when a read goes through it | **Accepted by design** (approved by the owner on 2026-11-04). Reads check what they touch and `verify_units` walks everything. After D-1, the head read check reuses `lifecycle_after(kind)` and there is no other read check with its own rule; the evidence is `unit_db::corruption_detected` (cases "metadata" and "lifecycle"). **Recommendation: review it in Stage 9** together with the verification CLI |
| K-E3-8 | U2 fails: `history` reaches 1.50× (1.503) of the reference in 1 of 3 passes | **Open.** Integrated with the documented failure (owner decision, 2026-11-04). Threshold unchanged. Options: judge U2 on `history_page` (0.58–0.90×) or make the materialisation of `Revision` cheaper. Carried into Stage 4 as an inherited risk (see [Stage 4 minutes](stage4-record.md)) |
| K-E3-9 | U6 fails: with concurrent retention, write p99 1.05–1.18× and throughput 0.89–0.94× (1.06× once) | **Open.** Integrated with the documented failure (owner decision, 2026-11-04). The cause is the background rotations, flushes and compactions that pruning generates. Limiting their pace in the engine is recommended. Carried into Stage 4 as an inherited risk (see [Stage 4 minutes](stage4-record.md)) |

## 6. Closure

| # | Closure condition | Status |
|---|---|---|
| 1 | Integrate E3-09 and complete §1, §4 and risks K-E3-3 and K-E3-4 | done (PR #13) |
| 2 | Owner approval of the promotion of IU-1…IU-10 ([invariants.md §2.1](invariants.md#21-invariants-of-the-units-layer-stage-3)) and of rows I-22 and I-26 | **done on 2026-11-04** |
| 3 | Roadmap (Stage 3 with its real status) and SAD §8 updated, with K-E3-8 and K-E3-9 | done in the change that incorporates this document |

## 7. Next stage

Stage 4 (relations and indexes) starts only when the owner approves its minutes: [stage4-record.md](stage4-record.md). It inherits K-E3-8 and K-E3-9, which remain open.
