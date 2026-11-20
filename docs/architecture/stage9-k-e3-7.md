# Review of K-E3-7: `kerno-units` reads against `verify_units`

**Package:** E9-16 of the [Stage 9 minutes](stage9-record.md) (§3, row E9-16; H-13 of §9; D20 (iii) of §8.1).
**Base of the review:** commit `a2248ab` (HEAD when the package started).
**Scope:** code reading only. There is no change to code, tests or dependencies. No test was run: the cited tests were read, not run. The claims marked *inferential* are deduced from the code and are pending a test that confirms them (§5).

## 1. What K-E3-7 is

[Stage 3 closure](stage3-closure.md), limits table, row K-E3-7: "The reads check less than `verify_units` checks. For example, a damaged intermediate `REV_META` is only seen when a read passes through it." It was **accepted by design** on 2026-11-04, with the recommendation to review it in Stage 9. The design that is accepted is: reads check what they touch, and `verify_units` goes through everything.

This review answers a single question: for each public or internal read of `kerno-units` that returns data from a snapshot, what does it check, what does `verify_units` check that it does not, and whether the difference is the one the design accepts or is a defect.

**What `verify_units` is:** `verify_snapshot` (`crates/kerno-units/src/verify.rs:1141`), which goes through each key of a snapshot and applies the rules UNIT-1 to UNIT-38 (header of `verify.rs:1-60`). It is read-only (I-18).

## 2. Method

Each read function cited in §3 was read to its body, and the rules of `verify.rs` that cover the same thing. The lines were obtained with `grep -n` over the tree of `a2248ab`. The two damage tests that appear in §5 were read to see what they assert about the read, not about `verify_units`.

Not reviewed, and outside this review: the decision logic of the planner (`planner.rs` except `query`, `query_relations`, `check_units`, `check_relations`, `check_adjacent`, `execute_relations`), the result cache (`cache.rs`), retention (`retention.rs`), the writing of vectors (`vector_store.rs` except `vector` and the loading), and the final part of `unit_at_head` (`history.rs:487` onwards).

## 3. Table of reads

Legend for the last column: **by design** = what the design accepts and documents; **K** = finding with a number in §4.

| # | Read (file:line) | What it checks | What `verify_units` checks and the read does not | Verdict |
|---|---|---|---|---|
| R1 | `UnitDb::get_unit` (`db.rs:585`), `UnitSnapshot::get_unit` (`db.rs:721`), `load_unit` (`db.rs:403-429`) | `UNIT_HEAD` decodes (UNIT-8). Its `REV_META` exists (if not, corruption). Head and `REV_META` of its revision match in hash, length, schema, metadata, `updated_seq` and life cycle (`db.rs:415-420`, `lifecycle_after`). The head's `REV_PAYLOAD` is verified by hash and length (`db.rs:425`). | It does not look at the intermediate `REV_META` (UNIT-31 in the range, UNIT-32 chain, UNIT-33 provenance, UNIT-34 policy). It does not validate `oldest_rev` in `1..=head_rev` (that is done by `read_head`, see R2; see K3). It does not look at `created_seq` (UNIT-31). | **By design** for the intermediates. K3 for `oldest_rev`. |
| R2 | `get_revision` (`history.rs:552`), `history` (`history.rs:562`), `history_page` (`history.rs:699`); `read_head` (`history.rs:355-367`), `check_bounds` (`history.rs:371`), `read_meta` (`history.rs:385`), `read_payload` (`history.rs:401`) | Header with `oldest_rev` in `1..=head_rev`. Bounds: `RevisionNotFound`, `Pruned`, `HistoryUnavailable`. Each `REV_META` of the requested span: exact key, decoding, `commit_seq` strictly increasing up to `updated_seq` (`history.rs:590-612`). Payload verified for each requested revision. | The `REV_META` outside the requested span. UNIT-32 (`parent_rev`, `Create` in revision 1) and UNIT-33 (provenance) are not checked in the read. | **By design**: a damaged intermediate `REV_META` appears only when the read goes through it. It is exactly the case of K-E3-7. |
| R3 | `unit_as_of` (`history.rs:789`), `state_as_of` (`history.rs:799`), `unit_at` (`history.rs:439`), `unit_at_head` (`history.rs:447-486`) | `created_seq` against the requested `s`. `updated_seq` against the head's `REV_META` (`history.rs:457-464`). With several revisions, search over `commit_seq`. With `oldest_rev = 1`, `created_seq` against revision 1 (`history.rs:481-486`). The policy is read with `policy_of`. | The same as R2 for the revisions the search does not touch. UNIT-32 and UNIT-33. | **By design**. Same limit as R2. |
| R4 | `get_relation` (`db.rs:726`) | The `REL` decodes (`RelationRecord::decode`). | `created_seq` not later than the snapshot and ends `Active` (UNIT-36). Presence of the two adjacency entries (UNIT-36). | **K5** (minor). |
| R5 | `relations_of` (`db.rs:741-773`) | Each entry of the adjacency index on the requested side has its `REL` (`db.rs:763`), and matches in ends and type (`db.rs:767`). Damage in an entry → corruption. | The **entry of the opposite end** (`REL_IN` for `Out`, `REL_OUT` for `In`). A `REL` with only one of its two entries is invisible from the side that has lost it, without error (K1). UNIT-36 does report it. | **K1**. |
| R6 | `reference_relations` (`reference.rs:94`), `reference_relations_of` (`reference.rs:113`) | Decodes each `REL` of the snapshot and filters. | It does not go through the adjacency: it is the reference path that I-32 takes as truth. That is why it is the comparison for K1: in a store with a missing entry, R5 and R6 give different answers without anyone signalling it. | Reference. |
| R7 | `query` (`planner.rs:434`), `indexed_scan` (`index.rs:337-342`), `scan_type` (`index.rs:362-387`), `scan_entries` (`index.rs:389-413`), `check_units` (`planner.rs:283-297`) | Per unit returned: `get_unit` (R1) and the entry matches the head (active cycle, type, schema; `index.rs:372-380`, `index.rs:400-410`). `check_units`: only active ones, which meet the filter, and in strictly increasing order of what was **returned**. | **Completeness**: that every active unit meeting the filter has its `IDX_PRIMARY` or `IDX_TYPE` entry. That the number of `IDX_META` entries matches the walk: `index.rs:328` returns it, and the planner uses it only for costs (`planner.rs:236`, `planner.rs:338`). UNIT-37 and UNIT-38 check it. | **K2**. |
| R8 | `indexed_assoc_query` (`assoc.rs:344`), `indexed_traverse` (`assoc.rs:374-392`), `check_adjacent` (`planner.rs:761-774`), `is_mirrored` (`planner.rs:776-788`) | Per hop: `check_relations`, `created_seq` not later than the snapshot, and mirror entry of the other end (`planner.rs:761-774`). | Relations that do not reach the traversal, for example a missing `REL_IN` when the traversal goes outward from the other end. `planner.rs:758-760` declares it as a limit ("not a completeness check"). | **By design** as declared. See K1 for what is not declared. |
| R9 | `query_relations` (`planner.rs:609`), `execute_relations` (`planner.rs:791-812`) | Through the adjacency path, only `check_relations` (`planner.rs:800`) and R5. | It does **not** call `check_adjacent` or `is_mirrored`. The mirror check that the associative path does (R8) is not applied to relation queries (K1). | **K1**. |
| R10 | `read_input` (`derived_stale.rs:354-358`), `valid_inputs` (`derived_stale.rs:370-372`) | Reads the source with `get_unit` (R1). | An archived or deleted source returns `payload = None`, and `and_then` **skips it without error** (`derived_stale.rs:355-358`). | **Already assigned**: F-03 of E8, in E9-11 (minutes §9). It is not reopened here. |
| R11 | `open_persisted` (`vector_store.rs:1128`), `load_persisted` (`vector_store.rs:1134`), `vector` (`vector_store.rs:1166`) | On opening, `analyze` (`vector_store.rs:720`), the same analysis used by `verify_findings` (`vector_store.rs:903-906`). A vector is served only if its descriptor, source and output hash match. `vector` returns what remained in memory. | Damage on disk after opening does not appear in `vector` until reopening. | **By design**: `vector_store.rs:1124-1127` describes it ("Every `Ready` descriptor … is `Ready` and served"). Inferential as to there being no re-reading. |
| R12 | `policy_of` (`db.rs:346-360`), `UnitSnapshot::policy` (`db.rs:685`), `layer_version` (`db.rs:709`), `read_mark` (`db.rs:388`) | The `LAYER_META` of the policy or of the mark decodes with `decode_upto`. Without a mark, it falls back to the default value. | Nothing relevant: UNIT-29 covers the names and the mark (`verify.rs:763`). | No finding. |
| R13 | `change_set` (`history.rs:851`), `change_sets_since` (`history.rs:863`), `change_set_info` (`history.rs:879-915`) | The record decodes. `member_count` matches the number of `CS_MEMBER` (`history.rs:888`). Ordinals in order. The reason is UTF-8. | That each member matches its `REV_META` (UNIT-33, `verify.rs:1078-1140`). | **K4** (minor). |

## 4. Findings

Each finding has its evidence and its class. *Inferential* means it comes from reading the code; no test has reproduced it. The status of all of them is **open**: none is corrected in E9-16 (minutes §3, row E9-16).

**K1. A relation with only one adjacency entry is invisible from the side that loses it.** *Inferential.*
- Where: `db.rs:741-773` (R5), `planner.rs:791-812` (R9).
- What happens: if `REL_IN` of a relation is missing, `relations_of(to, In)` does not return it (there is no entry to walk), but `reference_relations_of(to, In)` does (R6). `query_relations` through the adjacency path does not compare with the reference, because `execute_relations` only applies `check_relations`. The associative path does compare the mirror (`planner.rs:776-788`), but only for the relations its traversal reaches.
- Existing damage test: `crates/kerno-units/tests/relations.rs:534-572`, in the `damages` list includes "REL_IN missing". For that case only `UNIT-36` is asserted (`relations.rs:562`). The result block of the read has `_ => {}` (`relations.rs:568`): **no test asserts what the read returns** with `REL_IN` missing.
- What the design does cover: `planner.rs:758-760` declares that the mirror check is not complete. It does not say that `query_relations` does not apply it.
- Defect class: divergence of I-32 in a damaged store, without error or `Fallback`.
- Cause: pre-existing in the base code (no change in this package introduces it).

**K2. An index query can return fewer units than the reference when an entry is missing.** *Inferential.*
- Where: `index.rs:389-413` (`scan_entries`), `index.rs:362-387` (`scan_type`), `planner.rs:283-297` (`check_units`), `index.rs:328` (use of the count).
- What happens: an active unit without its `IDX_PRIMARY` is not walked, and no check on the query path notices it. `check_units` only looks at what is returned. The `IDX_META` count is not compared with the walk.
- Existing damage test: `crates/kerno-units/tests/index.rs:783-820`. The two lists (`type_damages`, `damages`) assert only `UNIT-37` or `UNIT-38`. There is no assertion about what `indexed_scan` returns.
- What the code does declare: `index.rs:52-55` says "on a damaged store the index path can answer where the reference path … fails". That describes the case in which the index path answers and the reference path fails. It says nothing about the opposite case, in which the index path answers less and without error.
- Defect class: same as K1.
- Cause: pre-existing.

**K3. `get_unit` does not validate the range of `oldest_rev`, which `read_head` does.** *Inferential.*
- Where: `db.rs:403-429` against `history.rs:355-367`.
- What happens: `load_unit` does not look at `oldest_rev`; `read_head` requires `1 <= oldest_rev <= head_rev`. A `UNIT_HEAD` with `oldest_rev = 0` is read by `get_unit` without error. UNIT-31 reports it (`verify.rs:809-830`).
- Consequence checked: none in the read. It is a difference between two reads of the same header.
- Class: minor.

**K4. `change_set_info` does not check that the members match their `REV_META`.** *Inferential.*
- Where: `history.rs:879-915`, against `verify.rs:1078-1140` (UNIT-33).
- What happens: `change_set` and `change_sets_since` return the members with their revision and type without cross-checking them with `REV_META`.
- Class: minor. It is the limit the design accepts ("what they touch"), and the read does not hide it: it only does not check it.

**K5. `get_relation` does not check `created_seq` or the ends.** *Inferential.*
- Where: `db.rs:726-733`, against UNIT-36 (`verify.rs:694-761`).
- Class: minor. It combines with K1 if the relation also loses an entry.

## 5. Tests missing to close K1 and K2

This review runs nothing, so K1 and K2 are not demonstrated. The package that handles them must:

1. For K1: assert, with `REL_IN` missing, that `relations_of(to, In)` and `query_relations` return the same result as `reference_relations_of` **or** an error. Today `relations.rs:568` asserts nothing.
2. For K2: with `IDX_PRIMARY` missing from an active unit, assert that `indexed_scan` and `reference_filter` agree or that `indexed_scan` gives an error.
3. Each test, in both engines (`emulated_log` and `kerno_db`, criterion §4.3 of the minutes), seen failing with a mutation in a copy.

## 6. Proposal, not decided

K1 and K2 are what the minutes call a "defect found" (§3, row E9-16: "a defect found opens a new package"). Opening that package changes the stage plan, so the owner decides it (D20 (iii), revocable). The proposal is a new package with this criterion: the relation and index reads, with an adjacency or index entry missing, return the reference result or an error; and the test asserts the result of the read, not only the rule of `verify_units`. K3, K4 and K5 are minor: they can be noted in the closure (E9-CL) or in the same package.

## 7. Review record (minutes §4.10)

| Finding | Status | Evidence |
|---|---|---|
| K1 | Open. Proposed as a new package | Code reading; test pending (§5) |
| K2 | Open. Proposed as a new package | Code reading; test pending (§5) |
| K3 | Open, minor | Code reading |
| K4 | Open, minor | Code reading |
| K5 | Open, minor | Code reading |
| F-03 (R10) | Assigned to E9-11 | Minutes §9 |

**Independent review:** pending. The author of this document cannot be its reviewer (§4.10). This package is not merged with K1 or K2 open without a decision of the owner.

## 8. Result of E9-22 (2026-11-12)

Package opened by the owner, under delegation, after this review. The tests were run in release mode, in both engines (`emulated_log` and `kerno`, the latter over `KernoDb`). Each new test was seen failing with a mutation in a copy of the tree, deleted afterwards. No format on disk changes.

**K1. Reproduced, in both engines.** Test: `one_adjacency_entry_missing_{small,large}_store` in `crates/kerno-units/tests/relations.rs`. It deletes by hand `REL_IN` and then `REL_OUT` of a relation, with the same damage medium that the crate's tests already use (`put_all` over the key), and compares the read with `reference_relations_of` and with `verify_units` (UNIT-36).
- From the end that keeps its entry, `query_relations` returned the adjacency answer as clean. Correction: `execute_relations` (`planner.rs`) now checks the mirror entry of each relation returned (`all_mirrored`, which uses the same `is_mirrored` as the associative path). If it is missing, it abandons the adjacency with `Fallback` `Invalid(Unmirrored)` and answers through the reference path; an unreadable mirror gives `Fallback` `Corrupt`. Cost in a healthy store: one point read per relation returned and no more. The crate has no counter of point reads (the engine's counters measure bloom and block cache, not `get`), so the cost is fixed by construction in `is_mirrored`, not by a counter.
- Mutation: `all_mirrored` always returns `true`. `one_adjacency_entry_missing_large_store` fails in both engines.
- **Still open:** from the end that lost its entry there is nothing to walk. `relations_of` and `query_relations` return the short answer, through the adjacency path and without `Fallback`. Detecting it requires comparing the adjacency range with the `REL` records, which costs more than one read per relation returned. The test fixes this limit so that a change to it is deliberate. `verify_units` reports it (UNIT-36).

**K2. Reproduced, in both engines.** Test: `index_path_with_a_missing_entry` in `crates/kerno-units/tests/index.rs`. It deletes the `IDX_PRIMARY` of an active unit and compares `indexed_scan_via` with `reference_filter` and with `verify_units`.
- With the `IDX_META` count intact, the `IDX_PRIMARY` path returned fewer units without error. Correction: `scan_entries` (`index.rs`) counts the entries it walks (it walks all of them, whatever the filter) and gives `Corruption` if they do not match the `IDX_META` count. The count comes from the `IDX_META` read that the caller already did to know the index is `Ready`: no extra read. `query` then answers through the reference path with `Fallback`.
- Mutation: the count check disabled. `index_path_with_a_missing_entry` fails in both engines.
- **Still open:** a count lowered to match the lost entry is not seen in the read (only the headers would show it), and the `IDX_TYPE` path has no count per type, so a lost `IDX_TYPE` entry still gives a short answer. The test fixes the two cases. `verify_units` reports them (UNIT-37, UNIT-38).

**K3. Not reproduced.** Test: `get_unit_checks_oldest_rev` in `crates/kerno-units/tests/relations.rs`, in both engines. With a `UNIT_HEAD` forged with `oldest_rev = 0`, `get_unit` already gives `Corruption`: `UnitHead::decode` requires `oldest_rev` in `1..=head_rev` (UNIT-17) before `load_unit` reads anything else. The reading in §4 was incorrect. No code was changed for K3.

**K4 and K5. Still open, minor.** There is no one-line check that closes them without further reads. K4 requires reading the `REV_META` of each member of the change set. K5 requires reading the two headers of the ends and the two adjacency entries of the relation. They are left for the closure of the stage (E9-CL), as §6 proposes.

| Finding | Status after E9-22 | Evidence |
|---|---|---|
| K1 | Corrected from the end that keeps its entry; open from the end that lost it | `one_adjacency_entry_missing_{small,large}_store`, two engines |
| K2 | Corrected for `IDX_PRIMARY` with the count intact; open with a lowered count and for `IDX_TYPE` | `index_path_with_a_missing_entry`, two engines |
| K3 | Not reproduced | `get_unit_checks_oldest_rev`, two engines |
| K4 | Open, minor | No change |
| K5 | Open, minor | No change |

## 9. Result of P1 after Stage 9 (2026-11-13): K1 from the end that lost its entry

P1 package, opened by the owner under delegation after the closure of the stage ([stage9-closure.md](stage9-closure.md) §5, point 2). §8 left K1 corrected only from the end that keeps its entry. This package closes it from the end that lost it. **It does change the format on disk** (unlike E9-22): it is documented in [units-v4.md](../file-format/units-v4.md).

**Design chosen and why.** From the end that lost its entry there is nothing to walk, and nothing points from the unit to its relations, so no check without a prior mark can see it; walking the `REL` records would cost more than one read per relation returned and would break the normal path. A mark that the reader compares is chosen: a record `0x72` per `(unit, direction, relation type)` with the number of adjacency entries, written in the same transaction as the entries. A total per unit was discarded: a given type would require walking all the types to check one.

**Cost on the normal path** (documented in `crates/kerno-units/src/adj_count.rs` and in units-v4 §5):

| Operation | Added cost |
|---|---|
| `relations_of` with a type | one point read (the record of that type) |
| `relations_of` without a type | a range walk over the records of that unit and direction (one per distinct type) |
| create or delete a relation | one read and one record write per adjacency entry, in the transaction that already exists; a start-up walk only once per `(unit, direction, type)` in a database from before layer 4 |

**Result.** With a count that disagrees, `relations_of` returns `Storage(Corruption)` and `query_relations` answers through the reference path with `Fallback` `Corrupt` (the answer is the reference one, complete). `verify_units` adds UNIT-45 (the count is not the number of entries) and UNIT-46 (counts with a mark lower than 4). Tests in `crates/kerno-units/tests/rel_counts.rs` and `relations.rs` (`one_adjacency_entry_missing_{small,large}_store`, which before fixed the short answer as a limit), in both engines. Mutations in [post9-mutations.md](post9-mutations.md).

**Compatibility.** The mark rises to 4 with the first count; a layer 3 or lower build rejects that whole database (UNIT-16). A directory written before (fixture [stage9-fixtures.md](stage9-fixtures.md) §6) opens, reads and verifies clean without writing anything.

**Limits that remain open, declared.**
- A database written before layer 4 has no counts until a relation of that `(unit, direction, type)` is created or deleted: until then **K1 remains as in §8** for those relations. Closing it fully requires a migration that walks all the relations; it is not done because opening does not write (I-18).
- A lost entry **together with** its count record (double damage) leaves a read that agrees with itself; `verify_units` sees the missing entry (UNIT-36).

| Finding | Status after P1 | Evidence |
|---|---|---|
| K1 | **Closed from the end that lost its entry** for relations with a count (layer 4); open for relations from before layer 4 without a count and for the double damage | `rel_counts.rs`, `one_adjacency_entry_missing_{small,large}_store`, two engines |
| K2 | No change (§8) | — |
| K3 | Not reproduced (§8) | — |
| K4 | **Open, minor, no change**: P1 does not close it | — |
| K5 | **Open, minor, no change**: P1 does not close it (that `get_relation` does not check its entries remains the same) | — |
