# Entry minutes of Stage 9: hardening

| Field | Value |
|---|---|
| Date | 2026-11-12 |
| Status | **Approved by delegation of the owner on 2026-11-12, revocable by them** (§8.1). The decisions D1 to D4, D12, D15 and D16, reserved for the owner in person on 2026-11-12, are **taken on 2026-11-13 by express delegation of the owner**, revocable (§8.1) |
| Act | Scope, starting state checked in the code, packages E9-00 to E9-CL, thresholds, invariants, risks and decisions D1 to D23 |
| Documents | [Roadmap](../roadmap/README.md) · [Stage 8 minutes](stage8-record.md) · [Stage 8 closure](stage8-closure.md) · [Stage 9 advance minutes](stage9-early-record.md) · [advance closure](stage9-early-closure.md) · [SAD](SAD.md) (§11, §12, §13, §16, Phases 4 and 6) · [invariants](invariants.md) · [threat model](../security/threat-model.md) · [ADR-0005](../adr/0005-public-identifiers.md) · [ADR-0006](../adr/0006-encryption-and-key-management.md) · [ADR-0009](../adr/0009-suppression-retention-and-backups.md) · [ADR-0010](../adr/0010-public-api-and-crate-boundaries.md) · [ADR-0012](../adr/0012-benchmark-methodology-and-exit-thresholds.md) · [ADR-0019](../adr/0019-license-and-contributions.md) · [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md) · [SQLite report](../testing/sqlite-report.md) · [friction list](../../examples/agent-memory/FRICTION.md) |

This minutes document describes the repository as it stood on 2026-11-12. Every claim about the code was checked by reading it; the test counts quoted (856 in `kerno-units` and `kerno-testkit`, 672 in `kerno-storage`) come from earlier closures and **have not been rerun** here. Besides reading the code, the only thing run for these minutes is `cargo fmt --all --check` (empty output, `exit=0`). These minutes change no code or tests.

## 0. Status

These minutes are **approved by delegation of the owner on 2026-11-12, revocable**. They set the scope of Stage 9, its packages with the files each one may write, the thresholds before measuring, and the decisions of §8, each with its recommendation. Each decision is recorded in §8.1 as taken by delegation, except those that change the format, a promise to users or a legal text. Those (D1 to D4, D12, D15 and D16) were **reserved for the owner in person on 2026-11-12**; on 2026-11-13 the owner, in person, asked the delegate to take the best decision on each pending point, and they are **taken on 2026-11-13 by express delegation**, revocable until something irreversible is built on them (§8.1).

No Stage 9 code is written before E9-00. The packages that write code respect §4 and §3.2.

## 1. Starting conditions

| Condition | Status read on 2026-11-12 | What is missing |
|---|---|---|
| Stage 8 closed as engineering work | **Met**. [Closure](stage8-closure.md) §0 and §8. Q4 not demonstrated; no blocking finding | E8-10 (follow-up before 1.0), handled in E9-19 and E9-20 |
| ADR-0009 accepted and written into its file | **Met**: option 3, in `0009-suppression-retention-and-backups.md` and in the index | C-4 contradicted in its count clause; A-1 and B-1 not met (§2.3, D17 and D18) |
| Advance closure (E9A-06) | **Met** on 2026-11-12, with no blocking finding open. I-29 promoted for `kerno-storage` by delegation, revocable | Findings H-2 to H-14 of the closure, assigned in §9 |
| ADR-0018 and ADR-0020 accepted | **Met** (2026-11-12) | None for Stage 9 |
| Decision 1 of the roadmap (security order versus AI) | Resolved on 2026-11-11 in the [advance minutes](stage9-early-record.md) §10 | None |
| Decision 3 of the roadmap (thresholds against SQLite) | Pending in the roadmap. Recommendation in §8: report, not criterion (D6) | Owner's decision, taken by delegation in §8.1 |
| ADR-0019, CLA text | ADR accepted on 2026-11-11, but **the CLA text is not written** (ADR-0019, Follow-up) | D15 taken on 2026-11-13 as direction (b). The legal text **is not written** here and needs the owner's approval before accepting any external contribution |
| ADR-0005, ADR-0006, ADR-0012 | **Proposed**, **Pending** and **Proposed** in their files | ADR-0005 is accepted with its option 1 (D1, 2026-11-13). ADR-0006 remains **Pending**: its option 1 (per-block AEAD) is the chosen design if encryption is built (D2, 2026-11-13). ADR-0012 is accepted by delegation in E9-00 (D5) |

## 2. Scope

### 2.1 What Stage 9 delivers

The deliverables are those of the [roadmap](../roadmap/README.md) (Stage 9), plus the physical reclaim from the public API that the closure of Stage 8 asks for (§6, group F):

| Deliverable | What this stage delivers | Package |
|---|---|---|
| Public identifiers (ADR-0005, I-27) | **Merged on 2026-11-13** (D1, option (a), squash `2697980`): codec `k1_` + base32 of 128 random bits, mapping `token -> UnitId` and its inverse written in the unit's transaction (`ChangeSet::create_public`), and tests of tampered tokens and of stability | E9-02 (merged) |
| Encryption and key management (ADR-0006, I-28) | **Decided, not implemented** (D2, D3 and D4, taken on 2026-11-13): per-block AEAD as the design if encryption is built; 1.0 without encryption, which promises no confidentiality at rest or key management; 1.0 without authenticated integrity. I-28 is declared not applicable (D19) | None: no package implements encryption |
| Minimised audit | Verification of I-29 for `kerno-units` (errors, reports, observations) and extension of the redaction test of `kerno-storage` to fragments. No persisted audit record (D8) | E9-03 |
| Export and import (I-30) | Export format **as a draft**, versioned and unfrozen (D10); export of units, revisions, relations and policies; import into a new engine; independence from the physical format | E9-04, E9-05, E9-06 |
| Migrations with fixtures from earlier versions | v1 fixtures checked, and the ADR-0020 fixture rebuilt from its base commit or declared absent (D11). The `MIGRATION` record stays reserved with no writer | E9-01 |
| Stress and fuzzing on the storage engine | Complete 30-minute fuzzing run per target on the final code of `kerno-storage` | E9-14 |
| Stress and fuzzing above the engine | Derived-data stress and property tests on the decoders of `kerno-units` (D21: no new tool) | E9-12, E9-13 |
| Comparative benchmarks | New pass of the SQLite report, without thresholds (D6), on a quiet machine; correction of third-party names in the current report | E9-17, E9-18 |
| Public API that the friction list asks for | Reading of derived data, vector replacement, errors, engine wrapper and physical reclaim (§2.4) | E9-07 to E9-11 and E9-08a to E9-08e |
| Evaluation set E8-10 | Set frozen separately, with query vectors in every class, and a single measurement | E9-19, E9-20 |
| Physical reclaim from the public API (ADR-0009 option 3) | Public `Reclaim` and `reclaim()` on `UnitDb` and `DerivedDb`, with the text of ADR-0009. No promise of physical deletion | E9-07 |
| Closure | Adversarial review, invariants, threat model, SAD as a new version, roadmap | E9-CL |

**Already done by the advance and not repeated** ([advance closure](stage9-early-closure.md)): survival measurement after deletion (`suppression_survival.rs`); I-29 for `kerno-storage` (`redaction.rs`); opening properties with 2 000 cases per type and mode (`open_properties.rs`); the 15 fuzzing targets of `kerno-storage` with their committed corpus (replay without failures); lifecycle stress of 200 cycles (`lifecycle_soak.rs`); the correction F-E9A-03-1 of SST opening (H-1). This stage repeats none of them; E9-14 only repeats the 30-minute run on the final code.

**Explicitly out of scope** (§2.5): encryption at rest and cryptographic deletion (D3, taken on 2026-11-13: no encryption in 1.0), backups as an engine function, approximate vector index, text search, hybrid fusion, real summariser, inferred relations, changes to `Config::default`, and any promise of physical deletion.

### 2.2 Starting point (checked in the code)

| Topic | What the documents say | What the code shows | Verdict |
|---|---|---|---|
| Public identifiers | ADR-0005: default codec with random token and persisted mapping | There is no `IdentifierCodec`, no token and no mapping in `crates/`. `UnitId` is an opaque `[u8; 16]` (`crates/kerno-units/src/types.rs:9`) | **Does not exist** |
| Encryption | ADR-0006 pending; I-28 in Phase 4 | The `AEAD` bit is rejected on opening: `Incompatible("encryption not supported")` (`crates/kerno-storage/src/format/header.rs:197-198`). The 40-byte slot must be zero (`format/block.rs:66`; `format/features.rs:5`) | **Does not exist**; format reservation |
| Keys | `KeyProvider` in ADR-0006 | No key API | **Does not exist** |
| Audit | I-29 in Phase 4; SAD §11.4 | `kerno-storage` has no logging layer (advance §2.2). Errors and reports: `tests/redaction.rs` (storage) and `IU-10` with `tests/sentinels.rs` (units) | I-29 verified in storage; **not in units** |
| Export | I-30 in Phase 4; `KRNO_EXP` reserved | `MAGIC_EXPORT` is declared (`format/header.rs:23-24`) and appears only in the list of valid magics (`header.rs:32`). No code writes it | **Does not exist** |
| Migrations | `MIGRATION` record reserved | `KIND_MIGRATION_RESERVED = 5` (`format/manifest.rs:26-28`). No writer | **Reserved** |
| Fixtures from earlier versions | ADR-0020, T1, T5 and T6, cite `tests/vectors/derived-v2/` and `derived-v2/db-with-vector` | `crates/kerno-storage/tests/vectors/v1/{dirs,structs}` and `crates/kerno-units/tests/vectors/v1/structs` exist. `derived-v2` does not appear in the tree (search across the whole repository, no results). The base commit `d01f646` exists | **The ADR-0020 fixture is missing** (E9-01) |
| Fuzzing | 15 targets in `crates/kerno-storage/fuzz` (advance §2.4) | 15 files in `fuzz/fuzz_targets/`. `ci.yml` runs neither fuzzing nor `cargo audit` or `cargo deny` (`grep` over `.github/workflows`: no fuzz or audit results). **There is no target for `kerno-units`** | Storage: done according to its record. Units: does not exist |
| Stress | Three tests in storage (advance §2.2) | `lifecycle_soak.rs`, `group_commit.rs` and `process_kill.rs` exist. In `kerno-units` there is only `crash.rs` and `process_kill.rs`. **There is no derived-data concurrency test**, and the lock order of the saved commit has no test (F-09 of E8) | Partial |
| Comparison with SQLite | Report without thresholds (E8-S0) | `sqlite` only appears in `spikes/storage/spike-bench/Cargo.toml` (search across all `Cargo.toml` files). No crate in `crates/` uses it | The report exists; **without thresholds** (D6) |
| Physical reclaim | ADR-0009, point 2; FRICTION 4 | `UnitDb::compact` exists only with `E: TestHooks` (`crates/kerno-units/src/retention.rs:662-669`). `KernoDb::compact_range` is `pub` (`crates/kerno-storage/src/db.rs:2011`) inside `impl KernoDb` (`db.rs:1769`), which does not carry `cfg(test)`. `DerivedDb` keeps its `UnitDb` as `pub(crate)` (`crates/kerno-units/src/derived_stale.rs:177`), so the layer does not expose it | **Partial**: see §2.3, point a |
| Engine wrapper | Rule of the wrapper from §4 (lesson of E8) | `UnitDb::engine(&self) -> &E` is `pub` and returns the internal engine. Its documentation says: "Writing to it directly bypasses the layer" (`crates/kerno-units/src/db.rs:488-491`). **20 files** use it, with 133 calls (`grep -rln '\.engine()'`; split in E9-08b to E9-08d) | **Breaks the rule** |
| Derived layer: reading | FRICTION 1 and 2 | `DerivedDb` has `get_unit`, `close`, `begin_changeset`, `state`, `descriptor`, `value`, `build`, `rebuild`, `supersede` and `vector` (`derived_stale.rs`, `vector_store.rs:1166`). It has no `snapshot`, `query` or history. `dependents_of_unit` is in `DerivedRegistry` (`derived.rs:293`), not in `DerivedDb` | **Confirmed** |
| Vector replacement | FRICTION 3 and 8 | `replace_vector` is `pub(crate)` (`vector_store.rs:1083`). `supersede_vector` is `pub` (`vector_store.rs:1060`) | **Confirmed** |
| Summaries | In memory only (E8-06, closure §5.1) | Quoted from the E8 closure; not rechecked in these minutes | Quoted |
| Default options | `wal_preallocate` false by default | `crates/kerno-storage/src/config.rs:543` | **Confirmed** |
| Dependencies | No changes to `Cargo.toml` or `Cargo.lock` since E7-00 (I-31) | `git log 52f026f..HEAD -- Cargo.toml Cargo.lock` returns no commits. `kerno-units` depends only on `kerno-core` and two computation libraries, and has `proptest`, `kerno-testkit` and `kerno-storage` as development dependencies (`crates/kerno-units/Cargo.toml`) | **Confirmed** |
| CI | Says nothing about fuzzing or dependency auditing | `.github/workflows/ci.yml`: `cargo fmt --all --check`, `cargo clippy ... -D warnings` (line 26), `cargo tree` checks (lines 28–34), `kerno-storage` tests (line 75) and those of the other crates (line 57). No fuzz or audit | **Confirmed**; T-10 remains pending |
| ADR states in the index | `docs/adr/README.md` | 0005 Proposed; 0006 Pending; 0009 Accepted; 0010 Accepted (facade and semver "are decided when a stable public API exists"); 0012 Proposed (applied in M0); 0014 Accepted; 0015 Accepted; 0016 Accepted (2026-11-09); 0017 Proposed; 0019 Accepted; 0020 Accepted | See §2.3, points e, f, g |
| Invariants | I-27, I-28, I-30 in Phase 4; I-42 and I-43 verified in tests and pending approval | [invariants](invariants.md) lines 50–53, 162–163 | **Confirmed** |

### 2.3 Discrepancies between document and code (the code wins)

a. **`KernoDb::compact_range` is not a test method.** The [Stage 8 closure](stage8-closure.md) §6, group F, says "`KernoDb::compact_range` is a test method". The code says otherwise: `pub fn compact_range` is at `crates/kerno-storage/src/db.rs:2011`, inside `impl KernoDb` (`db.rs:1769`), which does not carry `cfg(test)`; the only `#[cfg(test)]` block of that type starts at `db.rs:1664`. What is true: `DerivedDb` does not expose it, but `UnitDb::engine()` (`crates/kerno-units/src/db.rs:489`) does give a public route with `E = KernoDb`. The friction list (point 4) says "`KernoDb::compact_range` is an inherent method", which is exact. **Consequence:** the public reclaim is already reachable, but only by going around the derived layer. E9-08 removes that route and E9-07 gives the correct one.

b. **Friction 14 is not correct.** The list says that a second opening gives "no way to distinguish 'locked' from 'corrupt' except by the text". The code distinguishes the two: the would-block becomes `Error::Locked` (`crates/kerno-storage/src/db.rs:1804`), which is its own variant (`crates/kerno-core/src/error.rs:17`), and `UnitError::Storage` keeps it (`crates/kerno-units/src/error.rs:167`). A caller can `match` on `UnitError::Storage(kerno_core::Error::Locked)`. What is missing is a readable shortcut, and E9-11 adds it.

c. **Friction 11 is almost exact.** `DerivedDb::close` returns `Result<(), UnitError>` (`derived_stale.rs:236`), not `()`. What is true is that it discards the `CloseReport` of `KernoDb::close` (`db.rs:2029`), because the layer does not depend on `kerno-storage` (D9).

d. **ADR-0020 cites a fixture that does not exist.** T1, T5 and T6 (`docs/adr/0020-persistence-of-vectors-as-derivatives.md`, lines 297–302) cite `crates/kerno-units/tests/vectors/derived-v2/` and `derived-v2/db-with-vector`. There is no file with that name. The ADR is accepted, so it is not edited (index rule): E9-01 rebuilds the fixture from `d01f646` or records that it is missing.

e. **ADR-0014 contradicts itself.** The header says "Accepted (option 2; evidence from E4-02)", and in the body, line 113, it says "Still *Proposed*". E9-00 corrects the body; it does not change the decision.

f. **ADR-0009 line.** Point C-5 cites `crates/kerno-storage/src/db.rs:2009` for `compact_range`. Today it is at line 2011. The text of ADR-0009 is not edited for this; the citation is corrected in the closure of the stage (E9-CL) as an erratum, not as a change of decision.

g. **Numbering of the adaptation.** In this repository, ADR-0012 is the benchmark methodology (`docs/adr/0012-benchmark-methodology-and-exit-thresholds.md`) and the operational adaptation and invalidation is ADR-0016 (roadmap, Stage 6). If there was at some point a different numbering for the adaptation, it is not in `docs/adr/`. ADR-0016 is listed as "Accepted (2026-11-09)", with "Deciders: Project owner" but no reference to an approval minute; the owner asked, in the session that sent this task, to review its guarantees G1 to G7 before approving it; see D16.

h. **Status of the units layer.** The [advance closure](stage9-early-closure.md) §3, H-13, says that K-E3-7 has not been reviewed. These minutes confirm it: there is no written review of the `kerno-units` reads against `verify_units` (E9-16).

### 2.4 E8-09 friction: disposition of the fifteen points

The example in [`examples/agent-memory/FRICTION.md`](../../examples/agent-memory/FRICTION.md) is the real use case that the roadmap asks for before 1.0. Each point receives a disposition: **in scope** (a package resolves it and E9-21 tests it again), **declared limit** (it is documented and does not change the API in this stage) or **retained** (it depends on a decision of §8 that the owner takes in person; none is left that way since 2026-11-13, §8.1).

| # | What the list says | What the code shows | Disposition |
|---|---|---|---|
| 1 | `DerivedDb` does not list, query or read history | Confirmed (§2.2) | **In scope**: E9-09 (`snapshot`, `query` and `history` delegate to `UnitSnapshot`) |
| 2 | There is no way to find the derived data of a unit | `dependents_of_unit` exists in `DerivedRegistry` (`derived.rs:293`) | **In scope**: E9-09 (`derived_of(unit)`) |
| 3 | The derived id changes on each rebuild; the atomic path leaves rubbish | The identity includes the source revision (I-38, by design of ADR-0020). `replace_vector` is `pub(crate)` (`vector_store.rs:1083`) | **Partial**: E9-10 makes the atomic replacement public. Changing the identity is a format change; D12 does not decide it and it is not changed in this stage |
| 4 | **(promise)** Forgetting is only logical; there is no public compaction route | §2.2 and §2.3 a | **In scope**: E9-07 (`Reclaim` and `reclaim()`), with the text of ADR-0009 (D14) |
| 5 | Persisted vectors and summaries are not opened together | Summaries are in memory only (E8 closure, §5.1) | **Declared limit** (D8). The application keeps its definitions |
| 6 | 16-byte opaque ids; source of ids before opening | `UnitId` is opaque (`types.rs:9`). ADR-0005 decides the shape of the identifier | **Merged on 2026-11-13** (E9-02, D1): `ids.rs` and `PublicId` |
| 7 | `create` returns `UnitId`; `put_vector` asks for `RevisionId` | `create` returns `UnitId` (`db.rs:1133`); about 97 calls in the `kerno-units` tests | **In scope**: E9-10 adds `RevisionId::first(unit)` (without changing `create`) |
| 8 | Replacing an obsolete vector with one computed outside goes through "recompute" | `replace_vector` is not public (`vector_store.rs:1083`) | **In scope**: E9-10 |
| 9 | Two worlds of vectors: the search copies all the vectors on each call | Confirmed in the example; the search uses an in-memory `VectorSet` | **In scope**: E9-09 (the set is built once) |
| 10 | The recipe does not tie `dim` to the vector produced | No validation in `put_vector` | **In scope**: E9-10 (rejection if `dim` does not match the length) |
| 11 | `DerivedDb::close` discards the closing report | Returns `Result<(), UnitError>` (`derived_stale.rs:236`); the report belongs to `kerno-storage` | **Declared limit** (D9). The layer does not depend on `kerno-storage` |
| 12 | Three error types without a common type; messages without ids | Three types (`UnitError`, `DerivedStaleError`, `ConsolidationError`). `DerivedStaleError` does not name the unit | **Partly in scope**: E9-11 improves the `DerivedStaleError` message (F-02 and F-03 of E8). The common type is a **declared limit** (D9) |
| 13 | Opening requires three crates and a trait object; there is no `open_dir` | `kerno-units` does not depend on `kerno-storage` (`Cargo.toml`, with the note that the layer "never depends on it") | **Declared limit** (D9) |
| 14 | Second opening: "locked" is not distinguished from "corrupt" | **False in the code** (§2.3 b): `Error::Locked` exists and is kept | **In scope**: E9-11 adds `UnitError::is_locked()`; FRICTION is corrected in E9-21 |
| 15 | The catalogue's unit type shares space with the application; the history grows | `UnitDb::set_policy(unit_type, p, meta)` exists (`db.rs:619`) and `run_retention` is in `retention.rs:391` | **Resolved by the existing API**: the application sets `KeepLast(1)` on its type. It is documented in E9-21 |

In addition, the wrapper rule (§2.2, "Engine wrapper") does not appear in the friction list: it comes from the study of Stage 8 and is handled in E9-08.

### 2.5 Out of scope

- **Encryption at rest, keys and cryptographic deletion.** Out of 1.0 by D2, D3 and D4 (taken on 2026-11-13). K-1 to K-4 of ADR-0009 are not measured, because there is no encryption to measure.
- **Public identifiers with a token.** Merged by E9-02 (D1, 2026-11-13, squash `2697980`). The logical export carries the token of each unit that has one (`format_version` 1) and the import restores it: a token is stable after export and import; an import that would give one token to two units is rejected with a typed error (D-b of 2026-11-13).
- **Backup function.** There is none and none is created (D18). The backup window is not promised.
- **Approximate vector index, text search, hybrid fusion, real summariser, inferred relations.** They belong to other stages or have no benchmark that justifies them (ADR-0011; E8 §2.3, D8 and D12 of E8).
- **Changes to `Config::default`** of `kerno-storage` and to `Config::limits`.
- **Default ranking.** It stays off and experimental (D3 of E8 and the decision of 2026-11-12).
- **A guarantee of physical deletion** in any form. ADR-0009 option 3 does not contain one.
- **The `kerno` facade crate.** ADR-0010 leaves the facade for when there is a stable API (D9).
- **Reviewing the logic of K-E3-7 with a code change.** This stage only reviews it (E9-16). If the review finds a fault, a new package is opened with its own criterion; it is not corrected inside E9-16.

## 3. Work packages

Each package with code meets §4. Each one writes **only** to the files it lists. Where two packages must add a line to the same `lib.rs`, the merge order is in §3.1. Document packages do not touch code.

| WP | Content | Depends on | Writes (exact) | Can run alongside |
|---|---|---|---|---|
| **E9-00** Entry gate | Approval of these minutes by delegation; ADR-0012 accepted in its file (D5); body of ADR-0014 aligned with its header (§2.3 e); decisions of §8 recorded in the index | These minutes | `docs/adr/0012-benchmark-methodology-and-exit-thresholds.md` (status and decision only), `docs/adr/0014-physical-representation-of-relations.md` (only line 113), `docs/adr/README.md` (rows 0012 and 0014), `docs/roadmap/README.md` (Stage 9 section; sole writer of that section until E9-CL) | None: it is the gate |
| **E9-01** Fixtures from earlier versions (ADR-0020 T1, T5, T6; F-11) | Check that the v1 fixtures of storage and units open and give the model. Rebuild `derived-v2/` from `d01f646` in a copy outside the tree and check T5 (`IncompatibleLayer`, UNIT-16, `found_layer_version = 2`); if it cannot be built, declare it | E9-00 | `crates/kerno-units/tests/layer_fixtures.rs` (new), `crates/kerno-units/tests/vectors/derived-v2/` (new, only if rebuilt), `docs/architecture/stage9-fixtures.md` (new) | E9-03, E9-04, E9-07, E9-08a, E9-10 to E9-13, E9-15 to E9-19 |
| **E9-02** Public identifiers (ADR-0005, I-27) | **Merged on 2026-11-13 (D1, option (a); squash `2697980`, PR 78).** Corrected after the independent review (decisions D-a to D-f of 2026-11-13, by express delegation of the owner): value with the common wrapper and layer 3 mark ([units-v3.md](../file-format/units-v3.md)), deletion that removes the mapping, v1 export with tokens, `public_id` that checks the inverse, error text without a token/unit pair, randomness source that rejects 16 equal bytes, calls in `DerivedDb`. `verify_units` crosses and counts the records (UNIT-39 to UNIT-44, including a pair over a deleted unit). `public_ids.rs`: 49 tests listed without the `test-support` feature (48 run and 1 is ignored, the fixture regenerator) and 79 with it (78 and 1 ignored); 22 per engine without the feature and 37 with it, plus 5 that are not per engine. New fixture: `tests/vectors/public-ids-v3/`. Mutation record: [stage9-mutations-e9-02.md](stage9-mutations-e9-02.md). Changes outside the row: `verify.rs`, `codec.rs`, `export.rs`, `import.rs`, `derived_stale.rs`, `db.rs` | E9-00 and D1 (taken on 2026-11-13) | `crates/kerno-units/src/ids.rs` (new), `crates/kerno-units/src/lib.rs` (one line, last in the merge), `crates/kerno-units/tests/public_ids.rs` (new) | — |
| **E9-03** Minimised audit (I-29 in units; H-6 of storage; H-8) | Text-sentinel test for errors, reports and `verify` reports of `kerno-units`, with the same detector shape as storage. Key and value fragments in `kerno-storage` (H-6). Correct the comment "10^4 commits" of `group_commit.rs:538` (H-8) | E9-00 | `crates/kerno-units/tests/redaction_units.rs` (new), `crates/kerno-storage/tests/redaction.rs` (only adding the fragment search), `crates/kerno-storage/tests/group_commit.rs` (only the comment), `docs/architecture/stage9-audit.md` (new) | E9-01, E9-04, E9-07, E9-08a, E9-10 to E9-13 |
| **E9-04** Logical export, draft (I-30; D10) | Export format with its own header (`KRNO_EXP` stays reserved and unused, which the draft declares). Content: units, revisions, relations and policies. It does not include derived data (ADR-0018: they are rebuilt) or tokens (waits for E9-02) | E9-00 | `crates/kerno-units/src/export.rs` (new), `crates/kerno-units/src/lib.rs` (one line, first in the merge), `docs/file-format/export-v0-draft.md` (new) | E9-01, E9-03, E9-07, E9-08a, E9-10 to E9-13, E9-15 to E9-19 |
| **E9-05** Import into a new engine | Import the draft into a new directory with `emulated_log` and with `kerno_db`; the logical state is the same. Format errors with the class of `UnitError` | E9-04 | `crates/kerno-units/src/import.rs` (new), `crates/kerno-units/src/lib.rs` (one line, second in the merge), `crates/kerno-units/tests/export_import.rs` (new) | E9-03, E9-07, E9-10 to E9-13 |
| **E9-06** Independence of the physical format (I-30) | Export before and after flush, of public `compact_range` and of close; the bytes of the draft are equal | E9-05, E9-07 | `crates/kerno-units/tests/export_physical.rs` (new) | E9-12, E9-13, E9-15 to E9-19 |
| **E9-07** Public physical reclaim (ADR-0009 option 3; FRICTION 4; D14) | Public trait `Reclaim` in `kerno-core` with `compact_range`, which `TestHooks` leaves as it was. `impl Reclaim for KernoDb` delegates to the existing method. `UnitDb<E: Reclaim>::reclaim()` and `DerivedDb<E: Reclaim>::reclaim()` with the text of ADR-0009 | E9-00 | `crates/kerno-core/src/contract.rs` (only the new trait), `crates/kerno-storage/src/db.rs` (only the `impl Reclaim for KernoDb`; the inherent method is not touched), `crates/kerno-units/src/reclaim.rs` (new), `crates/kerno-units/src/lib.rs` (one line, third in the merge), `crates/kerno-units/tests/reclaim.rs` (new) | E9-01, E9-03, E9-04, E9-08a, E9-10 to E9-13, E9-15 to E9-19 |
| **E9-08a** Wrapper: `UnitDb` accessors (lesson of E8) | Classify the uses of `engine()` by method (table in the document) and add the accessors that the uses need. `engine()` does not change in this package | E9-00 | `crates/kerno-units/src/db.rs` (only new accessors), `docs/architecture/stage9-engine-uses.md` (new) | E9-01, E9-03, E9-04, E9-07, E9-10 to E9-13, E9-15 to E9-19 |
| **E9-08b** Migrate tests, group 1 | Replace `engine()` with the accessors, in 49 uses | E9-08a | `crates/kerno-units/tests/index.rs`, `crates/kerno-units/tests/relations.rs`, `crates/kerno-units/tests/assoc_indexed.rs`, `crates/kerno-units/tests/assoc_reference.rs`, `crates/kerno-units/tests/harness/units.rs` | E9-08c |
| **E9-08c** Migrate tests, group 2 | The same, in 33 uses | E9-08a | `crates/kerno-units/tests/history.rs`, `tests/verify.rs`, `tests/unit_db.rs`, `tests/policies.rs`, `tests/observation.rs`, `tests/sentinels.rs`, `tests/cache_sentinels.rs`, `tests/retention.rs` | E9-08b |
| **E9-08d** Migrate tests, group 3 and examples | The same, in 51 uses, including the measurement examples. A measurement example does not change what it measures | E9-08a | `crates/kerno-units/tests/derived_suppression.rs`, `tests/derived_vectors.rs`, `tests/derived_measure.rs`, `crates/kerno-units/examples/index_bench.rs`, `examples/relation_bench.rs`, `examples/unit_bench.rs`, `crates/kerno-units/src/retention.rs` (internal field instead of `engine()`) | E9-08b and E9-08c |
| **E9-08e** Withdraw `engine()` | `UnitDb::engine` stops being `pub`; `grep -rn "pub fn engine" crates/kerno-units/src` with no results | E9-08b, E9-08c, E9-08d | `crates/kerno-units/src/db.rs` (only the withdrawal) | — |
| **E9-08f** Review of E9-08a: raw write and hooks under `test-support` (D23) | `raw_commit` and the hooks `hook_flush`, `hook_wait_idle` and `hook_compact_range` exist only with the `test-support` feature, off by default, and in the unit tests of the crate; the integration tests and the examples that call them declare `required-features`. The raw reads check poisoning. Stricter tests of the accessors | E9-08e | `crates/kerno-units/Cargo.toml` (the feature and the `required-features`), `crates/kerno-units/src/db.rs` (accessors and `accessor_tests`), `crates/kerno-units/src/reclaim.rs` and `crates/kerno-units/src/retention.rs` (internal access to the engine), `crates/kerno-units/examples/index_bench.rs`, `relation_bench.rs` and `unit_bench.rs` (the run command carries `--features test-support`), `docs/architecture/stage9-engine-uses.md`, `docs/architecture/stage9-record.md` (D23) | — |
| **E9-08g** Minor findings of the review of E9-08f | `UnitDb::compact` moves from `E: TestHooks` to the public `Reclaim` trait, the same as `reclaim`, with the same limits and the same poisoning check. The comments and the document name the two public reclaim calls (`reclaim`, the whole key space; `compact`, the revision ranges). In `accessor_tests`: the sequence of a multi-entry `raw_commit` advances as much as one commit; `hook_compact_range` depends on its limits; the loop over `memtable_rotations` is bounded and a missing counter fails | E9-08f | `crates/kerno-units/src/retention.rs` (only `compact`), `crates/kerno-units/src/db.rs` (comment on the hooks and `accessor_tests`), `crates/kerno-units/Cargo.toml` (only the feature comment), `docs/architecture/stage9-engine-uses.md`, `docs/architecture/stage9-record.md` (§3, §3.1 and D23) | — |
| **E9-09** Reading derived data (FRICTION 1, 2, 9) | `DerivedDb::snapshot()`, `query(&UnitFilter)`, `history(unit)`, `derived_of(unit)` and a search over persisted vectors that builds the `VectorSet` once per batch of queries | E9-08e | `crates/kerno-units/src/derived_query.rs` (new), `crates/kerno-units/src/lib.rs` (one line, fourth in the merge), `crates/kerno-units/tests/derived_query.rs` (new) | E9-10 to E9-13 |
| **E9-10** Vector replacement and recipe (FRICTION 3, 7, 8, 10; F-04 of E8) | Make the atomic replacement public (`replace_vector`) with the name the package settles on. `put_vector` rejects a `dim` other than the length of the vector. `RevisionId::first(unit)`. Test of F-04: changing only `profile` changes the identity | E9-00 | `crates/kerno-units/src/vector_store.rs`, `crates/kerno-units/src/types.rs`, `crates/kerno-units/tests/derived_replace.rs` (new) | E9-01, E9-03 to E9-07, E9-12, E9-13 |
| **E9-11** Errors (FRICTION 12, 14; F-02 and F-03 of E8) | `UnitError::is_locked()`. `DerivedStaleError` messages with the unit id. `valid_inputs` (`read_input` returning `None`) becomes an explicit error, not a silent omission | E9-00 | `crates/kerno-units/src/error.rs`, `crates/kerno-units/src/derived_stale.rs` (only `Display` and `valid_inputs`), `crates/kerno-units/tests/errors_api.rs` (new) | E9-01, E9-03 to E9-07, E9-12, E9-13 |
| **E9-12** Derived-data stress (F-09, partial) | 4 threads write derived changes and others read; invariant: no derived item `ready` with a changed source. No time threshold; the poll waits for a condition, without `sleep` | E9-00 | `crates/kerno-units/tests/derived_stress.rs` (new) | E9-01, E9-03 to E9-07, E9-10, E9-11, E9-13 |
| **E9-13** Property tests of the `kerno-units` decoders (D21) | Byte mutation of the unit, relation, descriptor and vector records, with `proptest` (already a development dependency). 2 000 cases per type and mode; no panic; errors only of the corruption or incompatibility class | E9-00 | `crates/kerno-units/tests/decoders_robust.rs` (new) | E9-01, E9-03 to E9-07, E9-10, E9-11 |
| **E9-14** Fuzzing of `kerno-storage` on the final code (H-7) | 30-minute run per target of the 15, without writing to the corpus except failing entries. Runs with the machine free (§3.2) | E9-07 | `crates/kerno-storage/fuzz/README.md` (new section with the run), `crates/kerno-storage/fuzz/corpus/` (only new failing entries) | None: exclusive use of the machine |
| **E9-15** Mutation evidence for E8-06 and E8-07 (F-10; D20) | Mutate each new test of `consolidation.rs` and `derived_measure.rs` in a copy, recording line and output | E9-00 | `docs/architecture/stage9-mutations-e8.md` (new) | E9-01, E9-03 to E9-07, E9-16, E9-18 (without compiling at the same time: §3.2) |
| **E9-16** Review of K-E3-7 (H-13; D20) | Read the `kerno-units` reads against `verify_units`, with the table of each read and what it checks. No code change; a fault found opens a new package | E9-00 | `docs/architecture/stage9-k-e3-7.md` (new) | All document packages |
| **E9-17** Comparison with SQLite, new pass (no thresholds; D6) | Use the existing binary without changes (`spike-bench`), on a quiet machine (§3.2, T-SQ1). Without naming third parties that the limit of §5 prohibits | E9-00 | `spikes/storage/results/2026-11-12-e9/` (new), `docs/testing/sqlite-report-e9.md` (new) | E9-16, E9-18 |
| **E9-18** Third-party names in the current report (restriction of §5, T-NM) | Remove from the report the third-party names other than SQLite that the limit prohibits (the CPU model, the versions of the binding crates) and, as a precaution, those that describe the environment without being necessary (file system, kernel, CPU clock, compiler version). Keep what reproduces the result: the commit and the commands | E9-00 | `docs/testing/sqlite-report.md` | E9-15 to E9-17 |
| **E9-19** E8-10 set, freezing (not measured; D22) | Corpus, queries and judgements with their own seed, with a query vector in each of the four classes (§5, T-E10). Non-saturation check with the no-signal arm, before freezing; if it exceeds 0.96 it is regenerated with another seed. `MANIFEST` SHA-256 | E9-00 | `crates/kerno-testkit/src/derived_eval2.rs` (new), `crates/kerno-testkit/src/lib.rs` (one line; sole writer in the stage), `crates/kerno-testkit/data/derived-eval-2/` (new), `crates/kerno-testkit/tests/derived_eval2.rs` (new), `crates/kerno-testkit/examples/freeze_derived_eval2.rs` (new), `docs/architecture/stage9-evaluation-set.md` (new) | E9-01, E9-03 to E9-07, E9-10 to E9-13, E9-15, E9-16, E9-18 |
| **E9-20** E8-10 measurement (once only) | Run the measures of §5 over the set of E9-19, with the machine quiet. The raw output is committed. No threshold and no file is changed after measuring | E9-19 and E9-08e (so as not to measure with an API in transition) | `crates/kerno-units/tests/derived_measure2.rs` (new), `crates/kerno-units/examples/derived_bench2.rs` (new), `docs/architecture/stage9-measurement.md` (new), `docs/architecture/stage9-measurement-output.txt` (new) | None: exclusive use of the machine |
| **E9-21** Review of the use case with the new API (FRICTION) | Rewrite the example with the API of E9-07 to E9-11 and the accessors of E9-08a; update the table of §2.4 in the friction list with the disposition of each point | E9-07, E9-08e, E9-09, E9-10, E9-11 | `examples/agent-memory/src/**`, `examples/agent-memory/tests/**`, `examples/agent-memory/FRICTION.md`, `examples/agent-memory/README.md` | E9-14, E9-16 to E9-18 |
| **E9-CL** Closure | Adversarial review of E9-01 to E9-21; check of §4 and §5; invariants (§6); threat model (T-03, T-04, T-09, T-10 and the limit of H-2); DESIGN of `kerno-storage` (H-9); errata of §2.3 c and f; SAD as version 0.4.0 with its history; roadmap | All the above that run | `docs/architecture/stage9-closure.md` (new), `docs/architecture/invariants.md`, `docs/security/threat-model.md`, `docs/architecture/SAD.md` (version 0.4.0), `docs/architecture/stage9-early-record.md` (only H-12), `crates/kerno-storage/DESIGN.md` (only H-9), `docs/roadmap/README.md` (Stage 9 section) | — |

### 3.1 Merge order of the `mod` lines in `lib.rs`

- **`crates/kerno-units/src/lib.rs`**: E9-04, then E9-05, then E9-07, then E9-09. E9-02 (merged on 2026-11-13) was the last. Each package checks the test count after its merge (§4.2).
- **`crates/kerno-testkit/src/lib.rs`**: E9-19 only.
- **`crates/kerno-core/src/contract.rs`**: E9-07 only.
- **`crates/kerno-units/src/db.rs`**: E9-08a, then E9-08e, then E9-08f, then E9-08g. In addition, the integration of E9-07 with E9-08e added the internal accessor `inner_engine()`, which `reclaim` and `compact` use. No other package.
- **`crates/kerno-units/src/vector_store.rs`**, **`types.rs`**, **`error.rs`**, **`derived_stale.rs`**: one package each (E9-10 and E9-11). No other package of the stage writes them.
- **`crates/kerno-storage/src/db.rs`**: E9-07 only, only the `impl Reclaim`.
- **`crates/kerno-storage/tests/redaction.rs`**, **`group_commit.rs`**: E9-03 only.

### 3.2 What runs at the same time, and the small-machine rule

- **Independent lanes:** (1) layer API: E9-08a → (E9-08b ∥ E9-08c) → E9-08d → E9-08e → E9-09 → E9-21. (2) Export: E9-04 → E9-05 → E9-06 (waiting for E9-07). (3) Reclaim: E9-07. (4) New tests with no file conflict: E9-01, E9-03, E9-10, E9-11, E9-12, E9-13. (5) Documents, without compiling: E9-00, E9-16, E9-18. (6) Set: E9-19 → E9-20.
- **Rule for the 4-CPU machine with little disk:** at most **two build jobs at a time**. Jobs that measure times or that last hours run **alone**: E9-14 (fuzzing), E9-17 (SQLite), E9-20 (measurement of E8-10) and E9-15 (mutations, which compile once per mutation). Before each measurement the load (`uptime`, `load1`) and the free space are noted.
- Everything is compiled and tested **only in release** (`--release`), as §4.11 requires.

### 3.3 Critical path

**E9-00 → E9-08a → (E9-08b ∥ E9-08c) → E9-08d → E9-08e → E9-09 → E9-21 → E9-CL.** This is the longest code chain. In clock time, the bottleneck is E9-14: 15 targets of 30 minutes each (about 7.5 hours of exclusive compute), which can start as soon as E9-07 is merged and needs the machine free. E9-20 waits for E9-19 and E9-08e; E9-CL waits for all the others.

E9-02 (merged on 2026-11-13) was not on the critical path: it did not block the closure of the stage. I-27 is verified (§6).

## 4. Common acceptance criteria

Each package with code meets the following. A package that does not meet them is not closed.

1. **Format and lint.** `cargo fmt --all --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` without errors. Clippy rejects complex return types (`type_complexity`): use type aliases.
2. **Test count.** `cargo test -p <crate> --all-features --release` green, with the count noted before and after. The new file must appear in the count: if the number does not move, the runner does not see it. The count does not drop with respect to the baseline (T-CNT), except for a removed test with its reason written down.
3. **The two engines.** Every new test that uses an engine runs with `emulated_log` and with `kerno_db`, as in Stages 4 to 8.
4. **Each new test is seen failing.** Mutation applied to a **copy** of the tree, reverted and checked with `git diff`. The record keeps file, line, change, command and the `FAILED` line of the output. A mutation that leaves the test green is a finding, not a step (lesson of F-10 of E8).
5. **Reference path.** Where a derived item or an export answers a query, the answer is compared with the reference path over the same snapshot (I-32).
6. **No order dependencies.** No test depends on time, on order between tests or on shared state. No `sleep` to wait: a condition is polled.
7. **No content outside `diagnostic`** (I-37) in observations and explanations; no content sentinel in errors, reports or metrics (I-29).
8. **No dependency changes.** No `Cargo.toml` or `Cargo.lock` changes unless a decision asks for it (I-31). No package of §3 asks for it.
9. **Wrapper rule** (lesson of E8 and §2.2). No public function returns the engine, a reference to it or the `UnitDb` that stores a public type. Accessors return data, not the object they store (E9-08).
10. **Independent review before merging.** Each package with code is reviewed by someone other than its author. The review leaves a record: findings, their status and the evidence. It is not merged with a blocking finding open.
11. **Small machine.** Compile and test only in release; one heavy job at a time (§3.2); free space checked before each run; load noted before measuring.
12. **Language.** Code, comments and file names follow the language the repository already uses (Spanish in `docs/`, identifiers in English as in the code). Working reports are written in English.
13. **Accepted items are not edited.** An accepted ADR is not rewritten to change its decision; an erratum in a citation is corrected in the closure with a note (§2.3 f). A fixture referenced by an accepted ADR is created or declared absent; the citation is not changed.

## 5. Thresholds by topic, set before measuring

No threshold in this section is changed after measuring. A failure is recorded as such; it is not lowered. Each threshold is checked against the bound of its metric. **There is no absolute latency threshold** (D7).

| Code | Topic | Threshold | Bound of the metric and check |
|---|---|---|---|
| **T-EX1** | Export (I-30) | Differences in the logical state between source and reimport = 0 (units, revisions, relations, policies) | Integer counter ≥ 0; the threshold is equality to 0 |
| **T-EX2** | Physical independence (I-30) | Bytes of the draft equal before and after flush, `reclaim` and close | Equality of bytes; there is no time |
| **T-RE1** | Reclaim (ADR-0009, C-1 and C-3) | Live files containing a sentinel after `reclaim` and close, with no snapshot open = 0 | Integer counter; lower bound 0 |
| **T-RE2** | Reclaim with snapshot (ADR-0009, C-1) | Live files with the sentinel ⊆ files retained by the snapshot | Equality of sets. **The count clause of C-4 is not used** (D17) |
| **T-FZ1** | Fuzzing of `kerno-storage` (E9A-04) | 30 minutes per target; failures = 0; entries over 1 s = 0 | Time fixed at 1 800 s per target; integer counters |
| **T-PR1** | Properties of `kerno-units` (E9-13) | 2 000 cases per record type and mode; panics = 0; errors only of the corruption or incompatibility class | Counters; the set of classes is finite |
| **T-ST1** | Derived-data stress (E9-12) | Invariant violations = 0 in 200 cycles with 4 threads | Counter; there is no time threshold, and the duration is reported without a bound |
| **T-FR1** | Friction (§2.4) | 15 of 15 points with a disposition (in scope with a test, declared limit, or retained with its decision) | Bound 15 |
| **T-EN1** | Wrapper (lesson of E8) | `grep -rn "pub fn engine" crates/kerno-units/src` with no results | Counter = 0 |
| **T-MU1** | Mutation (lesson of F-10) | New tests with their mutation recorded = new tests | Equality of counters |
| **T-SQ1** | SQLite report (D6) | 1-minute load (`load1`) < 4 at the start of each measurement block (the number of CPUs). No latency threshold | Machine precondition, not a result. If it is not met, the block is repeated on a quiet machine and noted |
| **T-NM** | Third-party names in the report | The report names no third parties other than SQLite, nor the CPU, nor versions of binding crates | Search for the forbidden names: no results. (The limit comes from the project's restrictions note; if the owner reads it differently, it is decided in E9-18) |
| **T-CNT** | Test count | `kerno-units` + `kerno-testkit` ≥ 856; `kerno-storage` ≥ 672 (quoted, not rerun) | Lower bound; the count rises with each package |
| **T-E10-V1** | E8-10: query validity | ≥ 25 queries with a query vector in **each** of the 4 classes | Integer count; 25 is reachable because the set is frozen with that minimum |
| **T-E10-V2** | E8-10: non-saturation | nDCG@10 **without signal**, overall ≤ 0.96. If it exceeds that in the check before freezing, the set is regenerated with another seed before freezing | nDCG ∈ [0, 1]. With 0.96, the improvement of 0.02 of T-E10-S1 is reachable (bound 0.98) |
| **T-E10-Q1** | E8-10: exact retrieval | Recall@10 of the persisted exact search against brute force = 1.000 | Recall ∈ [0, 1]; 1.000 is reachable because it is the same search |
| **T-E10-Q2** | E8-10: quality with signal | nDCG@10 with signal ≥ 0.80, overall | nDCG ∈ [0, 1] |
| **T-E10-Q3** | E8-10: retrieval with signal | Recall@10 with signal ≥ 0.90, overall | Recall ∈ [0, 1] |
| **T-E10-S1** | E8-10: superiority (replaces Q4 overall) | nDCG@10 with signal − nDCG@10 without signal ≥ 0.02, overall | The difference ∈ [−1, 1]. With T-E10-V2 the bound is 0.98: 0.02 is reachable |
| **T-E10-S2** | E8-10: non-inferiority per class (replaces Q4 per class) | nDCG@10 with signal − without signal ≥ −0.01, in **each** of the 4 classes | Lower bound −1; −0.01 is a narrow margin but reachable. With T-E10-V1 each class can be computed |
| **T-E10-Q5** | E8-10: validity of judgements | Unjudged units in the top 10 ≤ 10 % of the positions, per arm and per class | Fraction ∈ [0, 1] (as Q5 of E8) |
| **T-E10-R** | E8-10: freezing | `MANIFEST` SHA-256 matches before measuring | Equality |

**On T-E10:** E8-10 does not replace the thresholds Q1 to Q5 of E8: it keeps them, and replaces Q4 with S1 and S2, which can be computed in the four classes because E9-19 puts a query vector in all of them. That closes F-06 of E8. Q2 and Q3 keep their absolute value, but T-E10-V2 is what avoids the ceiling effect of F-07.

No absolute latency threshold (D7). ADR-0011 says that an unmet limit opens a new decision; this stage sets none.

## 6. Invariants

| ID | Status when the stage opens | What Stage 9 does | Package that verifies it |
|---|---|---|---|
| **I-27** | **Verified** (2026-11-13), in E9-02 merged that day | A tampered token is rejected and a valid token does not authorise. D1 taken on 2026-11-13 (option (a)) | E9-02: `crates/kerno-units/tests/public_ids.rs` (`tampered_text_is_refused_with_its_own_error`, `tampered_but_well_formed_tokens_name_nothing`). "Does not authorise": resolving a token only returns a `UnitId`; the units layer has no authorisation of its own, the layer that exposes it decides |
| **I-28** | Phase 4, not verified | The keys are not in the data directory. **Without encryption in 1.0 (D3) there are no keys**: the invariant does not apply. The closure checks that `verify()` finds no unknown files in the directory, which is what can be said today | E9-CL (D19); the unknown-files test already exists in `lifecycle_soak.rs` |
| **I-29** | Verified in `kerno-storage` (promoted on 2026-11-12) | Extended to `kerno-units` (errors, reports, observations) and to fragments in storage | E9-03 |
| **I-30** | Phase 4, not verified | The export is independent of the physical format and can be reimported into a new engine | E9-04, E9-05, E9-06. The **freezing** of the format is not done: D12 (taken on 2026-11-13) leaves it for after D13 and real use outside the team |
| **I-31** | Verified by review of dependencies | No network or model in the core; no manifest changes | E9-CL (`git log 52f026f..HEAD -- Cargo.toml Cargo.lock`, which still has no commits) |
| **I-42** (proposal, verified in E8) | Pending approval | Deleting a source leaves its derived data with no visibility in the same `ChangeSet`. Verified again with `reclaim` | E9-07 (test that the reclaim does not alter the `ChangeSet`) and E9-CL. Promotion by D19 |
| **I-43** (proposal, verified in E8) | Pending approval | A summary never replaces the original while it exists | E9-CL, with the E8 test and that of E9-11 (messages). Promotion by D19 |
| **I-24** | Verified in indexes and derived data | The rebuild of a replaced vector equals building it again | E9-10 |
| **I-25** and **I-38** | Verified for persisted vectors; the test that varies only the profile is missing (F-04 of E8) | Changing only `profile` changes the identity | E9-10 |
| **I-32** | Verified | The queries by derived data and by vector search match the reference path | E9-09 (differential) |
| **I-37** | Verified in observations and derived data | No content in new observations, explanations or errors | E9-03, E9-09, E9-11 |
| **I-N1** (**new**, proposal: reclaim) | Does not exist | Public reclaim promises no physical deletion and only removes live files that no snapshot retains (ADR-0009, points 2 and 3) | E9-07 |
| **I-N2** (**new**, proposal: wrapper) | Does not exist | No public function of the layer returns the engine it stores | E9-08e (T-EN1) |

The new ones (I-N1 and I-N2) are numbered at the close of the stage, in [invariants](invariants.md), with whatever numbers are free then. This table uses their provisional names.

## 7. Risks

| ID | Risk | Treatment |
|---|---|---|
| **R-E9-1** | A decision that changes the format or a promise (D1 to D4, D12, D15, D16) is taken by inertia in a package, and changes the format or a promise without the owner having decided | §8.1 recorded it as reserved on 2026-11-12 and as taken on 2026-11-13 by express delegation, revocable; no package changes the format or a promise outside the recorded option; the review of each package checks that it does not touch the format |
| **R-E9-2** | A draft export becomes a de facto promise, because someone uses it | The draft is marked as unfrozen in its header and in the documentation (E9-04); the freezing is D12 |
| **R-E9-3** | The public reclaim is read as secure deletion | The text of the API and of the documentation repeats that of ADR-0009 and of threat model §6.5; the E9-07 test measures only what ADR-0009 measures |
| **R-E9-4** | Changing `engine()` breaks many tests, and they are fixed with patches that hide the rule | §3 splits the migration into five packages; E9-08e withdraws the function only when the grep gives zero; each migration is reviewed by someone other than its author |
| **R-E9-5** | The 30-minute fuzzing on the final code takes 7.5 hours of exclusive machine time | It is scheduled with the machine free (§3.2); if it does not fit, the closure says that T-FZ1 was not repeated, as the advance did |
| **R-E9-6** | Measurement with a loaded machine gives numbers that are no use, as in Stage 8 | T-SQ1 (load precondition) and solo execution; the numbers are reported with their load |
| **R-E9-7** | The E8-10 set is fitted to the implementation, as happened with that of E7 | Seed, identifier permutation and judgements fixed before freezing; `MANIFEST`; judgements written from the story and not from the code; T-E10-V2 makes the check before freezing, not after |
| **R-E9-8** | An E8-10 arm without a query vector in one class leaves Q4 unmeasured again | T-E10-V1 requires 25 per class, and E9-19 is not frozen without them |
| **R-E9-9** | The mutation of the new tests is not recorded, as in E8 (F-10) | T-MU1 and §4.4; the package is not closed without the record |
| **R-E9-10** | K-E3-7 has a real defect, and the review finds it late | E9-16 is done at the start (without dependencies); a defect opens its own package |
| **R-E9-11** | H-2 (filter rewritten with a valid CRC) is read as protection by the engine | Threat model §6.6 says so; E9-CL repeats it in the closure; D4 retains it |
| **R-E9-12** | ADR-0016 is taken as approved without the review the owner asked for | D16 taken on 2026-11-13 (option (b): ADR-0016 pending confirmation) and **closed the same day** with the review done; no package of the stage depends on ADR-0016 |
| **R-E9-13** | The ADR-0020 fixture cannot be rebuilt from `d01f646` | E9-01 declares it absent, and the citation of ADR-0020 keeps its erratum in the closure; the ADR is not edited |
| **R-E9-14** | The SQLite report keeps third-party names that the project's restriction forbids | E9-18 and T-NM; the new pass (E9-17) is born without them |
| **R-E9-15** | A validation of `dim` (E9-10) breaks tests that put vectors of another dimension on purpose | The package reviews the tests before changing the validation; if a test depends on it, the test is corrected with its reason |
| **R-E9-16** | Extending fuzzing to `kerno-units` with a new tool adds a tool chain that is not approved | D21: only property tests with the library that is already a development dependency |
| **R-E9-17** | E9-07 changes `kerno-core` (trait `Reclaim`) and is a public surface | Pre-1.0: the change is noted in the closure; the trait has a default value that returns "not supported", as `TestHooks` does |

## 8. Decisions the owner must take

Each one has options, consequences and a recommendation with its reason. The choice for each, by delegation or reserved, is in §8.1.

| # | Decision | Options and consequences | Recommendation and reason |
|---|---|---|---|
| **D1** | ADR-0005: public identifier codec | (a) Opaque random token with a persisted mapping (option 1 of the ADR). Adds a layer record `token → UnitId` in the same transaction: a layer format change with a version mark. (b) Reversible AEAD token (option 2): needs keys (D2) and longer tokens. (c) UUIDv7 exposed: discarded by the SAD | **(a)**. It needs no keys, a token can be reissued without touching the identity, and its version is fixed from day one. **Reserved on 2026-11-12; taken on 2026-11-13 by express delegation of the owner**, option (a), revocable until something irreversible is built on it. It changes the layer format and the shape of the token that users see |
| **D2** | ADR-0006: encryption design and key management | (a) Per-block AEAD (option 1 of the ADR): uses the reserved 40-byte slot and a `KeyProvider` outside the directory. Changes the format (`AEAD` bit, today rejected in `header.rs:197`). (b) File-system encryption delegated to the system: does not change the format, but does not protect exported copies (T-03). (c) Per-record encryption: selective cryptographic deletion, plus metadata and indexes | **(a)** as the design when D3 is decided in its favour. Today nothing is implemented. Reason: it is the only option that uses the slot already reserved and gives integrity (D4). **Reserved on 2026-11-12; taken on 2026-11-13 by express delegation of the owner**, option (a), as the design to use if encryption is built; nothing is implemented now. Cryptographic design, irreversible once on disk |
| **D3** | Does encryption go into 1.0? | (a) Yes, with D2 (a): delays 1.0 for the design and for the measurements K-1 to K-4. (b) No: 1.0 does not promise confidentiality at rest or key management, and declares it. (c) No in 1.0; added in 1.x with its own format bit | **(b)**. There is no design and no accepted ADR-0006, and the measurements K-1 to K-4 cannot be made without encryption. Declaring the absence is true; promising encryption without a design would be an empty promise. **Reserved on 2026-11-12; taken on 2026-11-13 by express delegation of the owner**, option (b): 1.0 does not promise confidentiality at rest or key management, and declares it. It is a promise to users |
| **D4** | Authenticated integrity against a rewrite with a valid CRC (H-2 of the advance closure) | (a) Do not add authenticated integrity in 1.0. The limit stays in threat model §6.6, and `verify()` with `deep` is the resource. (b) Keyed authentication code in the 40-byte slot, without confidentiality, with a new bit. (c) Full AEAD (D2 a). (b) and (c) change the format and require keeping a key | **(a)**. An authentication code without key management does not protect against whoever holds the key, and changing the format without D2 is taking two irreversible decisions at once. **Reserved on 2026-11-12; taken on 2026-11-13 by express delegation of the owner**, option (a); H-2 is closed as an accepted and documented limit. It changes the format |
| **D5** | ADR-0012: benchmark methodology | (a) Accept as it stands (applied in M0): equal conditions, repetitions, equivalent durability and relative thresholds. (b) Accept with changes before measuring. (c) Keep proposed: the stage's measurements have no accepted method | **(a)**. It was already applied in M0 and in E8. It changes no format or promise. **Taken by delegation** |
| **D6** | Thresholds against SQLite: criterion or report | (a) Report without thresholds, as the owner decided on 2026-11-11. (b) Criterion with thresholds set before measuring. (b) requires a quiet measurement: that of E8-S0 was made with a load of 5 to 14 on 4 CPUs, and several ranges overlap | **(a)** for 1.0. Without a clean baseline, any threshold would be a figure chosen after seeing data, like T4 of Stage 7. **Taken by delegation** |
| **D7** | Absolute latency limit | (a) None: only ratios and comparisons between paths. (b) A limit set before measuring | **(a)**, as D9 of E8. An unmet limit opens a new decision (ADR-0011). **Taken by delegation** |
| **D8** | New on-disk format in 1.0 for summaries or for audit? | (a) No: summaries stay in memory and the application keeps its definitions; there is no persisted audit record. (b) Yes: summary descriptors or an audit record with its format bit. (b) forces measuring sentinels on disk (F-05 of E8) and an ADR | **(a)**. FRICTION 5 is resolved by documenting, not with a format. **Taken by delegation**; (b) would become reserved if it is requested |
| **D9** | Shape of the public API before 1.0 and friction points in scope | (a) Keep the current crates, no facade; changes before 1.0 are allowed and noted in the closure; the 15 points have a disposition (§2.4). (b) A single `kerno` facade now (ADR-0010 option 1). (c) Freeze the API now: prevents resolving points 1, 2, 8 and 9 | **(a)**. ADR-0010 leaves the facade for when there is a stable API, and before 1.0 there is no promise to break. **Taken by delegation** |
| **D10** | Export format: draft or frozen | (a) Versioned, unfrozen draft (`export-v0-draft.md`). (b) Frozen now: a format promise. (c) No export before 1.0: I-30 stays unverified | **(a)**. It allows verifying I-30 without promising the format. The freezing is D12. **Taken by delegation** as far as the draft goes |
| **D11** | Migration and fixture policy before 1.0 | (a) v1 fixtures in storage and units; ADR-0020 fixture rebuilt from `d01f646` or declared absent; `MIGRATION` reserved with no writer. (b) Define the migration record now: changes the format. (c) v1 fixtures only | **(a)**. It does not change the format and makes clear which versions have been tested for reading. **Taken by delegation** |
| **D12** | Declare 1.0 and its promises: stable v1 format, semver of the API, frozen export format | (a) Declare at the close of Stage 9 if D13 is met. (b) Declare after D13 and real use outside the team. (c) Do not declare 1.0 | **(b)**. [ADR-0019](../adr/0019-license-and-contributions.md) conditions charging on "a 1.0 with a stable format, migrations and real users". The E8-09 use case is internal. **Reserved on 2026-11-12; taken on 2026-11-13 by express delegation of the owner**, option (b): 1.0 is not declared now. It is the largest promise to users of the stage |
| **D13** | What 1.0 requires and what it does not | **Requires:** (1) Stage 9 closed with §4 and §5 met or declared. (2) The 15 friction points with a disposition (§2.4). (3) E8-10 closed with its thresholds. (4) v1 fixtures and the ADR-0020 one tested or declared. (5) I-30 verified for the draft and the reimport. (6) Public reclaim with the text of ADR-0009 and its test. (7) Limits declared in the threat model. **Does not require:** encryption (D3 b), approximate index, text search, real summariser, backup function (D18), threshold against SQLite, tokenised identifiers (if D1 is not taken, the API uses 16-byte identifiers) | **The list of requirements**. It is the criterion of the stage. The declaration of 1.0 is D12. **Taken by delegation** as the criterion |
| **D14** | Physical reclaim from the public API | (a) Public `Reclaim` and `reclaim()` on `UnitDb` and `DerivedDb`, with the text of ADR-0009 (mechanism, not physical deletion). (b) Do not expose it and declare `forget` only logical: the promise of option 3 is unreachable. (c) Expose it with physical-deletion text: contradicts threat model §6.5 | **(a)**. It is what ADR-0009 option 3 promises in its point 2, and nothing more. **Taken by delegation** |
| **D15** | CLA text (ADR-0019) | (a) Assignment of rights to the owner. (b) Broad non-exclusive licence with the right to relicense. (c) No CLA until the first external contribution; today none is accepted. (a) and (b) are promises to contributors | **(b)**. ADR-0019 asks to keep the option of relicensing and of offering commercial terms, and (b) does so without ceding authorship. **Reserved on 2026-11-12; taken on 2026-11-13 by express delegation of the owner as direction**, option (b). The legal text is not written and needs the owner's approval before accepting any external contribution. This stage does not draft it |
| **D16** | Confirmation of the acceptance of ADR-0016 | (a) Confirm the acceptance of 2026-11-09 as it stands. (b) Review G1 to G7 before confirming. (c) Replace it. The owner conditioned their approval, in the session that sent this task, on reviewing G1 to G7, and the file references no approval minute | **(b)**. Until the text is reviewed, the status is "pending confirmation". Nothing in Stage 9 depends on ADR-0016. **Reserved on 2026-11-12; taken on 2026-11-13 by express delegation of the owner**, option (b): ADR-0016 remains pending confirmation |
| **D17** | Count clause of C-4 (ADR-0009) | (a) Keep the clause as it is, with the contradiction recorded. (b) Rewrite the clause with a new date so that it matches the measurement | **(a)**. ADR-0009 forbids adjusting criteria in view of a measurement. **Taken by delegation** |
| **D18** | Class assignment (A-1) and backup declaration (B-1) for 1.0 | (a) All classes on option 1 (what the code does); declare that there is no backup function and that the backup window is not promised. (b) Wait for a backup function | **(a)**, as D11 of E8. It is the only declaration that is true today. **Taken by delegation** |
| **D19** | Invariants: I-28, I-42, I-43 and I-29 for `kerno-units` | (a) I-28 does not apply in 1.0 (no keys); promote I-42 and I-43 if E9-CL verifies them again; promote I-29 for `kerno-units` if E9-03 passes. (b) Leave them as proposals | **(a)**. They are states, not new promises. **Taken by delegation**, revocable |
| **D20** | Process inheritance: H-3, F-10 and K-E3-7 | (i) H-3: accept the correction of `src/` that went into the squash of E9A-03, or ask for it to be reverted and reapplied separately. (ii) F-10: require the mutation evidence of E8-06 and E8-07, or accept the gap. (iii) K-E3-7: review in this stage, or leave it as it was | (i) **Accept**, with its regression test and its two mutations. (ii) **Require** the evidence (E9-15). (iii) **Review** in E9-16. **Taken by delegation**, revocable |
| **D21** | Fuzzing above the storage engine | (a) A new fuzzing tool in `crates/kerno-units/fuzz` with a nightly toolchain: new dependency, new approval. (b) Property tests with the library that is already a development dependency of `kerno-units`. (c) Do not do it | **(b)**. It is the lowest step of the ladder (the dependency already exists) and adds no tool chain. The approval of E9A-04 was for `kerno-storage`. **Taken by delegation** |
| **D22** | E8-10 set: composition and thresholds | (a) Approve §5 (T-E10-V1, V2, Q1 to Q3, S1, S2, Q5, R). (b) Change thresholds before freezing | **(a)**. The thresholds are set before any measurement of the new set and are checked against their bounds. **Taken by delegation** |

## 8.1 Choices by delegation (2026-11-12)

On 2026-11-12 the owner delegated the pending decisions of the roadmap and of these minutes. Each decision is recorded here with the option recommended in §8. The ones **taken** are revocable by the owner. The ones **reserved** are not taken by delegation: they are irreversible or change a promise to users, and they wait for the owner in person. **On 2026-11-13** the owner, in person, asked the delegate to take the best decision on each pending point. D1, D2, D3, D4, D12, D15 and D16 are now **taken** with the option that these minutes recommended, as their row indicates; their status of 2026-11-12 is kept in the same row.

| # | Status | Option recorded | Revocable | What remains pending |
|---|---|---|---|---|
| **D1** | **Taken on 2026-11-13** by express delegation of the owner (reserved on 2026-11-12) | (a): opaque random token with a persisted mapping (option 1 of ADR-0005). Frees E9-02, which is done as its own package; E9-02 is **merged on 2026-11-13** (squash `2697980`) | Yes, until something irreversible is built on it | Nothing: E9-02 is merged |
| **D2** | **Taken on 2026-11-13** by express delegation of the owner (reserved on 2026-11-12) | (a): per-block AEAD, as the design to use if encryption is built. **Nothing is implemented now** | Yes, until something irreversible is built on it | Nothing: there is no key or encryption code; K-1 to K-4 of ADR-0009 unmeasured |
| **D3** | **Taken on 2026-11-13** by express delegation of the owner (reserved on 2026-11-12) | (b): no encryption in 1.0. 1.0 does not promise confidentiality at rest or key management, and declares it | Yes, until something irreversible is built on it | Nothing |
| **D4** | **Taken on 2026-11-13** by express delegation of the owner (reserved on 2026-11-12) | (a): no authenticated integrity in 1.0. The limit stays in threat model §6.6; `verify()` with `deep` is the resource. H-2 is closed as an accepted and documented limit | Yes, until something irreversible is built on it | Nothing |
| **D5** | Taken | (a): ADR-0012 accepted as it stands. E9-00 writes it into its file | Yes | — |
| **D6** | Taken | (a): report without thresholds against SQLite | Yes | A criterion against SQLite, which would require a quiet measurement before setting it |
| **D7** | Taken | (a): no absolute latency limit | Yes | — |
| **D8** | Taken | (a): no on-disk format for summaries or for audit in 1.0 | Yes | A format for summaries or for audit (format change) |
| **D9** | Taken | (a): current crates, no facade; dispositions of §2.4 | Yes | The single facade (ADR-0010 option 1) |
| **D10** | Taken as far as the draft goes | (a): export draft, unfrozen | Yes | The freezing of the format (D12) |
| **D11** | Taken | (a): v1 fixtures, ADR-0020 fixture rebuilt or declared absent | Yes | A written migration record |
| **D12** | **Taken on 2026-11-13** by express delegation of the owner (reserved on 2026-11-12) | (b): 1.0 is declared only after D13 and real use outside the team. **1.0 is not declared now**, and no document says or implies it | Yes, until something irreversible is built on it | Declaring 1.0, freezing the export format and promising semver of the API: not done until D13 and real use are met |
| **D13** | Taken as the criterion | (a): the requirements list of §8 | Yes | The declaration of 1.0 (D12) |
| **D14** | Taken | (a): public `Reclaim` and `reclaim()` with the text of ADR-0009 | Yes | Any text that says "secure deletion" or "physical deletion" |
| **D15** | **Taken on 2026-11-13** as direction (reserved on 2026-11-12) | (b): broad non-exclusive licence with the right to relicense. The legal text **is not written** and needs the owner's approval before accepting any external contribution | Yes | Accepting any external contribution, until the owner approves the legal text; changes to the README and to `CONTRIBUTING.md` that refer to the CLA |
| **D16** | **Taken on 2026-11-13** by express delegation of the owner (reserved on 2026-11-12) | (b): ADR-0016 remains **pending confirmation** until G1 to G7 are reviewed. The review runs as a separate task. **Closed on 2026-11-13:** the review was done and, by express delegation of the owner, ADR-0016 is **accepted and confirmed** with the amendments of its "Review of 2026-11-13" (G2, G3 and G6 as they are; text of G4, G5 and G7 corrected to what the code does; gaps listed as open work; `create`, `activate`, `CacheEntry::new` and `Dependencies::current` only under `test-support`) | Yes | Confirming ADR-0016 until G1 to G7 are reviewed; nothing in Stage 9 depends on ADR-0016 |
| **D17** | Taken | (a): clause of C-4 not rewritten; contradiction recorded | Yes | — |
| **D18** | Taken | (a): all classes on option 1; no backup function declared | Yes | A backup function and its window (B-1) |
| **D19** | Taken | (a): I-28 does not apply in 1.0; I-42 and I-43 promoted if E9-CL verifies them; I-29 for `kerno-units` if E9-03 passes | Yes | — |
| **D20** | Taken | (i) accept H-3 with its test; (ii) require evidence for F-10 (E9-15); (iii) review K-E3-7 (E9-16) | Yes | — |
| **D21** | Taken | (b): property tests with the library already present; no new tool | Yes | A new fuzzing tool in `kerno-units` |
| **D22** | Taken | (a): thresholds of §5 unchanged before freezing | Yes | — |
| **D23** | Taken on 2026-11-12 by delegation (E9-08f) | The raw write accessor of `UnitDb` (`raw_commit`) and its test hooks (`hook_flush`, `hook_wait_idle`, `hook_compact_range`) exist only under the `test-support` feature of `kerno-units`, off by default, and in the unit-test builds of the crate itself (`cfg(test)`). The read accessors (`raw_snapshot`, `raw_get`, `raw_seq`, `storage_stats`) are kept without it. Detail in [uses of `engine()`](stage9-engine-uses.md) | Yes | — |

**State after 2026-11-13.** No decision is reserved any more. What remains pending, without being a delegated decision: (1) the owner's approval of the CLA text before accepting any external contribution (D15); (2) ~~the review of G1 to G7 of ADR-0016 (D16)~~ **done and D16 closed on 2026-11-13**; (3) the declaration of 1.0, which only happens after D13 and real use outside the team (D12). E9-02 was merged on 2026-11-13, before the closure (E9-CL).

## 9. Inherited findings and the package that closes them

| Finding | Origin | Status when the stage opens | Package or decision |
|---|---|---|---|
| **H-2** Filter rewritten with a valid CRC: `get` hides interior keys | Advance, E9A-03 | **Closed as an accepted and documented limit** (2026-11-13) | **D4 taken on 2026-11-13** (reserved on 2026-11-12). E9-CL leaves it as a limit in threat model §6.6 |
| **H-3** Correction of `src/` inside the squash of E9A-03 | Advance | Open, not blocking | D20 (i): accept afterwards |
| **H-4** C-4, count clause, contradicted | Advance | Open, not blocking | D17: remains contradicted, not rewritten |
| **H-5** What is not measured: C-5, memory, caches, tombstones, deletion thread, handle | Advance | Open, not blocking | Not measured; E9-CL declares it |
| **H-6** Sentinel detector only with complete text | Advance | Open | E9-03 (fragments) |
| **H-7** 30 minutes of fuzzing not repeated | Advance | Unverified | E9-14 |
| **H-8** Comment "10^4 commits" in `group_commit.rs` | Advance | Open | E9-03 |
| **H-9** `DESIGN.md`: "deletion does not overwrite either" with `wal_preallocate` | Advance | Open | E9-CL |
| **H-12** Citation `sst.rs:332` in the advance minutes, actually `sst.rs:335` | Advance | Open | E9-CL |
| **H-13** K-E3-7 not reviewed | Advance | Open | E9-16; D20 (iii) |
| **H-14** I-27, I-28, I-30 not verified | Advance | Open | E9-04 to E9-06, E9-CL (D19). E9-02 merged on 2026-11-13: I-27 verified |
| **F-02** Unclear error when rebuilding a summary | E8 | Open | E9-11 |
| **F-03** `read_input` skips a source without error | E8 | Open, not reached | E9-11 (explicit error) |
| **F-04** No test changes only `profile` | E8 | Open | E9-10 |
| **F-05** Summaries without sentinel measurement on disk | E8 | Open | D8: there is no format on disk; the measurement becomes mandatory if D8 changes |
| **F-06**, **F-07** Q4 not demonstrated; ceiling effect | E8 | Open | E9-19 and E9-20 (T-E10-V1, S1, S2, V2) |
| **F-08** P2 does not measure the reading of persisted vectors | E8 | Open | Declared limit: E9-20 does not measure reading |
| **F-09** Lock order of the saved commit without a test | E8 | Open | E9-12 covers it partly; the exact order still has no deterministic test (declared limit) |
| **F-10** Mutation evidence for E8-06 and E8-07 | E8 | Open | D20 (ii): E9-15 |
| **F-11** T5 check of ADR-0020 not run | E8 | Open | E9-01 |
| **F-12** No record of the review rounds of E8-03 and E8-04 | E8 | Open | Record in E9-CL: what can still be rebuilt from the commits, with the source of each datum |
| **F-13** Fifteen friction points | E8 | Open | §2.4; E9-07 to E9-11 and E9-21 |

## 10. Approval

| Date | Decision | Notes |
|---|---|---|
| 2026-11-12 | **Minutes written and approved by delegation of the owner, revocable** | §8.1. The reserved decisions (D1 to D4, D12, D15, D16) are not taken by delegation and are left for the owner in person. No Stage 9 code is written before E9-00. These minutes change no code or tests |
| 2026-11-13 | **D1 to D4, D12, D15 and D16 taken by express delegation of the owner, revocable** | Reserved on 2026-11-12 (row above, not deleted). The owner, in person, asked the delegate for the best decision on each pending point. Detail in §8.1. This row changes no code or tests |
| 2026-11-13 | **Closure of Stage 9 approved by the owner in person** | [Closure](stage9-closure.md). All packages are merged. The approval covers the closure and decisions D1 to D4, D12, D15 and D16 as they were recorded. 1.0 is not declared (D12). The legal text of the CLA is not written and its approval is still pending (D15). This row changes no code or tests |
