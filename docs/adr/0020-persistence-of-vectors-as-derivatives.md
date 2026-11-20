# ADR-0020 — Persistence of vectors as derivatives

| Field | Value |
|---|---|
| Status | **Accepted on 2026-11-12, by delegation of the owner** on the pending decisions of the roadmap that day (minutes of Stage 8, D2, with the condition of E8-03). **Revocable by the owner**: until they revoke it, it stands. The condition of D2 is met: E8-03 passed (see [Evidence](#evidence)) |
| Date | 2026-11-12 |
| Deciders | Project owner, by delegation (2026-11-12) |
| Related to | [ADR-0018](0018-versioning-and-rebuilding-of-derived-artifacts.md) (common derivatives registry, option 2); [ADR-0011](0011-vector-profile-and-hybrid-retrieval.md) (in its persistence clause, replaced by this ADR; ADR-0011 is not edited); [ADR-0003](0003-physical-format-and-versioning.md) §1, §3, §4; [ADR-0007](0007-index-consistency-and-rebuilding.md); [ADR-0008](0008-document-serialization-and-hashes.md) (BLAKE3-256); [ADR-0009](0009-suppression-retention-and-backups.md) (option 3); [ADR-0013](0013-format-v1-and-engine-protocol.md) (format v1 frozen, not edited); [ADR-0014](0014-physical-representation-of-relations.md); [units layer format v1](../file-format/units-v1.md) (not edited); [minutes of Stage 8](../architecture/stage8-record.md) §5.4, §7 and §8.1 (D2); invariants I-1, I-2, I-21, I-23, I-24, I-25, I-31, I-37, I-38, I-42 (proposed) |

## Context

Today no vector is persisted. Vectors are supplied by the caller and live in memory (`VectorSet`, ADR-0011). The derivatives registry of E8-02 and the staleness logic of E8-03 also live only in memory: on reopening, the caller hands back the list of descriptors (`DerivedDb::open`). The minutes of Stage 8 (D2, §8.1) chose to persist vectors as derivatives, with an `incompat` feature bit and a new ADR, conditional on E8-03 passing. ADR-0011 also asks that persisting vectors have an ADR that replaces it.

Persisting changes the format. The engine's format v1 is frozen (ADR-0013) and cannot change: neither its bytes, nor its list of known bits, nor its vectors. This ADR fixes where vectors and their descriptors live, how compatibility is declared, how writing and staleness are made atomic, what happens on reopening, what survives a suppression, what stays out, and what the E8-04 package must meet.

This ADR fixes the bytes of the layer, it does not write them. The code is E8-04's.

## D2 condition: E8-03 passed

The condition of D2 ("persist only if E8-03 passes") is met. It was run in this tree (commit `d01f646`, merge of E8-03):

```
cargo test -p kerno-units --test derived_registry --test derived_stale
  Running tests/derived_registry.rs
  test result: ok. 13 passed; 0 failed; 0 ignored
  Running tests/derived_stale.rs
  test result: ok. 46 passed; 0 failed; 0 ignored
```

## Options considered

Each point indicates the chosen option (**in bold**) and the discarded ones.

### 1. Where a vector and its descriptor live

1. **Key space of the units layer, with tags `0x60`, `0x61` and `0x62`** (high nibble `0x6`, unassigned today).
   - **The keys go through the same transactional engine as the units, with their own tag prefix, and the layer reads and writes them inside its `ChangeSet`s.** That way atomicity with the source (I-21) is the engine's, with no new protocol.
   - In layer 1, nibble `0x6` falls within "a newer layer" of UNIT-2 (`units-v1.md` §5): a layer 1 that encounters these keys rejects them, instead of reading them wrongly.
2. Keys directly in the engine's space, without a layer. Rejected: the engine does not know about derivatives or the layer mark, and the layer would lose control of what counts as a valid descriptor.
3. Vectors in a side file with its own format. Rejected: it breaks I-21 (the vector and its source are not committed in the same commit), and it forces a second durability and recovery protocol.
4. Tags in `0x43`–`0x4F`. Rejected: nibble `0x4` already belongs to relations (`0x40`–`0x42`), and mixing these in complicates reading the adjacency ranges.

### 2. The feature bit: name, layer and timing

1. **Version 2 of the layer in the `LAYER_META "layer"` mark, with the feature `DERIVED_VECTORS`.** Layer 1 rejects on opening any mark with `layer_version > 1` (UNIT-16, `IncompatibleLayer`). This is the form of "incompat" that already exists in the code: opening decodes the mark before any other key (`db.rs`, `read_mark`).
2. A bit in the `incompat` word of the engine's common header (`ADR-0003` §1, `v1.md` §4). Rejected for four reasons:
   - Files already written do not carry the bit, and setting it on all of them means rewriting the engine. Only the MANIFEST is rewritten on each *checkpoint*, so the bit would have to live in it and in every file an old build could read.
   - The engine would have to know what a derivative is. Today it does not, and it has no concept of a layer.
   - It changes `KNOWN_INCOMPAT` (`crates/kerno-storage/src/format/features.rs`), the list that HDR-6 applies to every file. That is a change to the rules of the frozen format, with an edit of `v1.md` §4 and regeneration of vectors (`v1.md` §15).
   - The old build only fails on opening if the bit is in the MANIFEST, which is replaced on each *checkpoint*. It must be written atomically with the first write of a vector, which couples the engine to the layer.
3. A named record inside `LAYER_META`, with a feature word (like `policy/<type>`). Rejected: opening only reads the `layer` mark (`read_mark`). An old build would ignore the new record and open the database, which is exactly what the bit must prevent.
4. A feature word inside the mark, changing its value of 7 bytes. Rejected: it changes the bytes of a layer 1 value. UNIT-10 rejects leftover bytes, so an old build would classify it as `Corruption`. It must be said that this is incompatibility, not damage (criterion of ADR-0013, point 8).
5. Mark always at 2 for every new database. Rejected: layer 1 builds would stop opening databases that have no vectors at all. The minutes ask that the bit be activated only with a persisted vector.

The bit is activated **only when a database has a persisted vector**, and it is **never lowered**. A database that once had a vector and then lost them all stays at version 2: lowering it would require knowing that no live file and no snapshot retains a `0x6x` key, and the engine does not guarantee that without compacting (ADR-0009, option 3).

### 3. Atomicity

1. **The vector, its descriptor, its inverse-index entry and, if needed, the layer mark are written in the same engine transaction (`begin_write` … `commit`).** The staleness marking caused by a source change goes in the transaction of that change.
2. Marking after the commit, in memory, as E8-03 does (`DerivedChangeSet::commit`). Rejected for persisted data: between the commit and the marking, a crash leaves on disk a `ready` descriptor whose source has already changed. It is acceptable while the registry is in memory, and not valid for persisted data.

### 4. Reconstruction and loss

1. **The database is the authority; a persisted vector is a verified copy of a computation, which can be thrown away and redone.** On opening, the registry is rebuilt from the persisted descriptors, each vector is checked against its output hash, and whatever does not match is not served.
2. Rebuilding everything on opening with the caller's computation function. Rejected: the computation is expensive and is supplied by the caller; opening must not compute anything.
3. Trusting the inverse index without checking it against the descriptors. Rejected: a wrong inverse index would cause a staleness that does not happen, or the reverse.
4. Repairing on opening, by writing. Rejected: `UnitDb::open` writes nothing (`db.rs`, documentation of `open`). Repair is an explicit call that writes a `ChangeSet` with purpose `Derived`.

### 5. Suppression

1. **Deletion of the source in the same `ChangeSet`, and the engine removes the files on compaction** (ADR-0009, option 3, with the class of vectors assigned to option 1 and the deletion of the source as suppression). This is what section 5.3 of the minutes asks for.
2. Marking `stale` and keeping the descriptor, as E8-03 does when deleting a unit. Rejected: a derivative of a deleted source cannot be rebuilt, and ADR-0009 asks that suppression reach the derivatives, not leave them as obsolete.
3. Cryptographic erasure. Not available: it depends on ADR-0006, pending (ADR-0009, point 4).
4. Promising physical deletion. Rejected: ADR-0009, point 3, does not promise it.

### 6. Out of scope

- Approximate index on disk.
- Model in the core, and network (I-31).
- Encryption (ADR-0006).
- Summaries with several sources (E8-06) and inferred relations (minutes D8).
- Physical deletion.
- Dimension policy per profile beyond what `VectorSet` already does (one dimension per set, `DimensionMismatch`).

## Decision criteria

Set before implementing E8-04:

- No byte of the engine's format v1 changes: `KNOWN_INCOMPAT`, `WRITE_INCOMPAT`, `KNOWN_RO_COMPAT`, `v1.md` and the vectors of `kerno-storage` stay as they are.
- A build that does not know the feature **rejects**, it never reads wrongly. The rejection happens before reading any other key, and writes nothing.
- A vector is never the source of truth: whatever is missing is redone from the units (I-24), and nothing is served that does not have its verified descriptor.
- The vector, its descriptor and its inverse index are committed together or not at all (I-21).
- The staleness of a vector happens in the same transaction as the change to its source (I-23, I-25, I-38).
- A source deletion leaves no vector visible after the commit (I-1, I-42).

## Decision

**Vectors are persisted as derivatives of the units layer, with the following rules.**

1. **Key space:** tags `0x60` (descriptor), `0x61` (vector) and `0x62` (inverse index source → derivative), in the units layer, never in the engine's format.
2. **Bit:** feature `DERIVED_VECTORS`, declared as **version 2 of the layer** in the `LAYER_META "layer"` mark. It is `incompat` in the sense of ADR-0003: a layer 1 build rejects it on opening. Version 2 is written in the first `ChangeSet` that persists a derivative, is never lowered, and is not written to databases without vectors.
3. **Atomicity:** descriptor, vector, inverse-index entry and layer mark are committed in one transaction. A source change marks its derivatives `stale` in the same transaction, and a source deletion removes them in the same transaction.
4. **Reconstruction:** the database is the source of truth. On opening, the registry is rebuilt from the persisted descriptors; the vectors are checked and are not served if they do not match.
5. **Suppression:** a unit deletion or a revision retention removes, in the same transaction, the descriptors, the vectors and the inverse-index entries of their derivatives. What survives on disk until compaction is declared as a limit (ADR-0009, option 3), without promising physical deletion.
6. **This ADR replaces, as far as persistence is concerned, the clause of ADR-0011** ("Committed" and "Review" in its Consequences). That ADR is not edited, and its status does not change: the rest of ADR-0011 (exact search, model profile, single dimension per set) remains in force.

The byte rules, atomicity, reconstruction and suppression are specified in the following sections. The acceptance criteria of E8-04 are in the section [E8-04 acceptance criteria](#e8-04-acceptance-criteria).

## Byte format (key and value space)

Conventions of `units-v1.md` §1, which are kept: integers **big-endian**, `str(max)` = `len: u32` (big-endian) ‖ UTF-8 bytes, `bytes(max)` with `len` before, `unit_id` of 16 bytes, `ContentHash` does not appear here. There is **one exception**, stated explicitly because it is a fact of the E8-02/E8-03 code: the **hash preimages** use **little-endian** integers (see §C).

### A. Keys (layer 2)

| `tag` | Name | Fields after the tag | Length |
|---|---|---|---|
| `0x60` | `DERIVED_DESC` | `derived_id[32]` | 33 |
| `0x61` | `DERIVED_VEC` | `derived_id[32]` | 33 |
| `0x62` | `DERIVED_SRC` | `unit_id[16]` ‖ `rev_no: u64` ‖ `derived_id[32]` | 57 |

- `derived_id` is the identifier of section C. Each derivative is one descriptor, at most one vector and exactly one `DERIVED_SRC` entry.
- `DERIVED_SRC` sorts by unit, then by revision: "all derivatives of a unit" is a range with prefix `0x62 ‖ unit_id`, and "all those of a revision" is a range with prefix `0x62 ‖ unit_id ‖ rev_no`.
- Tags `0x63`–`0x6F`: reserved. A layer 2 that encounters them rejects them with UNIT-2 (`IncompatibleLayer`).

### B. Values

**B.1 `DERIVED_DESC` (`0x60`).** Fields in this order:

| # | Field | Type | Constraint |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` (UNIT-6, UNIT-7) |
| 2 | `reserved_flags` | `u8` | `0` (UNIT-8) |
| 3 | `kind` | `u8` | `1` = `Vector`. `0` and `2..=255` are corruption (DV-1) |
| 4 | `state` | `u8` | `1` = `Ready`, `2` = `Stale`, `3` = `Superseded`. Others are corruption (DV-2) |
| 5 | `source_unit` | `[u8; 16]` | none |
| 6 | `source_rev` | `u64` | ≥ 1 (DV-4) |
| 7 | `method` | `str(64)` | 1..=64 B, UTF-8 (DV-4) |
| 8 | `method_version` | `u32` | none |
| 9 | `profile` | `str(64)` | 1..=64 B, UTF-8 (DV-4). It is the model profile of ADR-0018 and I-25; the core stores it as opaque text and does not interpret it |
| 10 | `param_count` | `u8` | 0..=16 (DV-5) |
| 11 | `params` | `param_count` × (`key: str(64)` ‖ `value: str(256)`) | `key` 1..=64 B, `value` 0..=256 B; keys in **increasing byte order**, no repeats (DV-5) |
| 12 | `input_hash` | `[u8; 32]` | §C.2 |
| 13 | `output_hash` | `[u8; 32]` | §C.3 |

No leftover bytes (UNIT-10). **The descriptor carries no date**: ADR-0018 asks for one, but a wall-clock date would make reconstruction not equal to construction (I-24). The date of a derivative is the `created_at` of the `CHANGESET` of its commit.

**B.2 `DERIVED_VEC` (`0x61`).**

| # | Field | Type | Constraint |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` |
| 3 | `dim` | `u32` | 1 ≤ `dim` ≤ 4096 (DV-6) |
| 4 | `elem_type` | `u8` | `1` = `f32`, IEEE 754 binary32. Another value is corruption in layer 2 (DV-7); a new type requires a new layer version |
| 5 | `data` | `dim` × 4 B | each element, its bits in **big-endian**; finite (DV-8) |

Total length: `7 + 4·dim` bytes (DV-6). Every element is finite: `NaN`, `+∞` and `−∞` are corruption. `−0.0` (bits `0x80000000`) is corruption: the writer writes it as `+0.0`, so that the output hash does not depend on the sign of zero (DV-8).

**B.3 `DERIVED_SRC` (`0x62`).** A 2-byte value: `envelope_version = 1`, `reserved_flags = 0`. No further fields (UNIT-10). The key carries the source; the value repeats nothing.

**B.4 Layer mark, version 2.** Same value as `units-v1.md` §3.1, with `layer_version = 2` (`u32` big-endian). `LAYER_META` record named `layer`, with no change of shape. A mark with `layer_version > 2` is `IncompatibleLayer` (UNIT-16 extended).

**B.5 `CHANGESET`, new purpose.** `purpose = 5` = `Derived`: written by the layer for a `ChangeSet` that writes only keys `0x60`–`0x62`, without members, like `Relation` (3) and `Index` (4). A `Derived` purpose `ChangeSet` with `member_count ≠ 0`, or that touches unit or relation keys, is corruption (DV-13).

### C. Identity and hashes

All functions use BLAKE3-256 (ADR-0008). `F(x)` is the length field `u64` **little-endian** (8 bytes) followed by `x`.

**C.1 `derived_id`** (32 bytes; it is the key of `0x60`, `0x61` and `0x62`):

```
derived_id = BLAKE3-256(
    "kerno.derived.identity.v2"                      -- ASCII, without F(·)
    ‖ F(u64le(n))                                    -- n = number of sources (1 in vectors)
    ‖ for each source, in order (unit, rev):
        F(unit[16]) ‖ F(u64le(rev))
    ‖ F(method) ‖ F(u32le(method_version))
    ‖ F(profile)
    ‖ F(u64le(m))                                    -- m = number of parameters
    ‖ for each parameter, in increasing key order:
        F(key) ‖ F(value)
)
```

Outside the `id`: `input_hash`, `output_hash` and the date. Thus, reconstructing the same data over equal inputs gives the same `id`.

Change with respect to E8-02: the v1 `id` (`IDENTITY_DOMAIN = "kerno.derived.identity.v1"`, `derived.rs`) has no `profile`. E8-04 adds the field and changes the domain to `…v2`. The v1 `id` was never persisted, so there is no data to migrate.

**C.2 `input_hash`** (32 bytes; the one of `derived_stale.rs`, unchanged). Here there is **no** `F(·)` and the integers are little-endian:

```
input_hash = BLAKE3-256(
    "kerno.derived.input.v1"                          -- ASCII
    ‖ u64le(n)                                        -- n = number of sources
    ‖ for each source, in order (unit, rev):
        unit[16] ‖ u64le(rev)
)
```

**C.3 `output_hash`** (32 bytes): `BLAKE3-256` of all the bytes of the `DERIVED_VEC` value (`7 + 4·dim` bytes, including the header). It is set by the writer, and the reader checks it before serving the vector (DV-12).

### D. Validation rules

Evaluation order, as in `units-v1.md` §6: key rules, then envelope, then fields in order, then cross-field rules. The layer 1 rules (UNIT-6, UNIT-7, UNIT-8, UNIT-9, UNIT-10, UNIT-11, UNIT-12) apply the same way to the new values.

| Id | Rule | Class |
|---|---|---|
| DV-1 | `kind ∉ {1}` | `Corruption` |
| DV-2 | `state ∉ {1, 2, 3}` | `Corruption` |
| DV-3 | Key length ≠ 33 (`0x60`, `0x61`) or ≠ 57 (`0x62`) | `Corruption` |
| DV-4 | `source_rev = 0`; `method` or `profile` empty, larger than 64 B or not UTF-8 | `Corruption` |
| DV-5 | `param_count > 16`; empty key or larger than 64 B; value larger than 256 B; keys not increasing or repeated; not UTF-8 | `Corruption` |
| DV-6 | `dim = 0`, `dim > 4096`, or value length ≠ `7 + 4·dim` | `Corruption` |
| DV-7 | `elem_type ≠ 1` | `Corruption` |
| DV-8 | Element `NaN`, `±∞` or `−0.0` | `Corruption` |
| DV-9 | Vector or `0x62` entry without a descriptor; descriptor without a `0x62` entry; or `0x62` entry whose `unit`/`rev` are not those of the descriptor | `Corruption` |
| DV-10 | The recalculated `derived_id` (§C.1) ≠ the key | `Corruption` |
| DV-11 | Keys `0x60`–`0x62` in a database whose mark has `layer_version = 1` | `Corruption` |
| DV-12 | `Ready` without a vector, or with an `output_hash` different from the value's hash; `Stale` or `Superseded` with a vector | `Corruption` |
| DV-13 | `purpose = 5` with `member_count ≠ 0`; or a `Derived` purpose `ChangeSet` that touches unit or relation keys | `Corruption` |
| DV-14 | A descriptor whose source unit does not exist, or whose head is `Deleted`: the suppression of §S5 should have removed it | `Corruption` |
| UNIT-2 (extended) | Nibble tags `0x4`–`0xF` outside `0x40`–`0x42`, `0x50`–`0x52` and `0x60`–`0x62`: written by a newer layer | `IncompatibleLayer` |
| UNIT-16 (extended) | `layer_version > 2` | `IncompatibleLayer` |

A descriptor that fails DV-10 is not loaded: its vector is not served and its entry does not take up space in the registry. Its key and its children are left as orphans (see §S4).

## Atomicity (S3)

A `ChangeSet` in layer 2 is a single engine transaction. Inside it:

1. **Writing a derivative** (`Ready` descriptor, vector and `0x62` entry) is only valid if the source revision **is the head of its unit in that same transaction**. If it is not, the write is rejected with `InvalidArgument` and nothing is written. That way a `Ready` over an old revision is never committed.
2. **A source change** (create, update, archive, unarchive or restore a unit: any revision that is not `Delete`) walks, with the prefix `0x62 ‖ unit_id`, the entries of the unit. For each descriptor in `Ready`, it writes `state = Stale` and deletes its vector (DV-12). In the same transaction. A derivative in `Stale` or `Superseded` is not touched.
3. **A unit deletion** (`Delete` revision) deletes, with the same walk, the descriptors, vectors and entries of its derivatives. It does not leave them as obsolete (§S5).
4. **The layer mark** is written in the same transaction as the first `0x60`–`0x62` key. If the database is at version 1, the transaction writes the mark with `layer_version = 2`, and if it is already at 2, it does not touch it.
5. **No writer lowers the mark.** `UnitDb::set_policy` and the first `ChangeSet` of a new database write `LAYER_VERSION` today with the constant 1 (`db.rs`, lines 625 and 1373). In layer 2 they write `max(current mark, 2 if the database already has derivatives)`.

**What changes with respect to E8-03.** The marking of E8-03 happens **after** `self.cs.commit()?`, in memory. For persisted data it happens **inside** the transaction. The rule of I-23 is met because the persisted state and the source change in the same commit, not because a reader waits for a lock in memory.

A commit failure (for example, the data sync) leaves nothing written: neither the source, nor the mark, nor the derivative. The engine guarantees that (ADR-0002).

## Reconstruction and loss (S4)

**Principle.** A persisted vector is never the source of truth. The source is the unit. If a vector is lost, corrupted or does not match, it is redone with `rebuild` from the source revision, and the result equals that of building again (I-24): same `derived_id`, same `input_hash`, same `output_hash`, same vector bytes.

**Reopening.** `DerivedDb::open` stops receiving the list of descriptors when there is persistence: the registry comes from the database. Without writing anything, in this order:

1. Read the layer mark. Version 1 with keys `0x60`–`0x62`: `Corruption` (DV-11). Version greater than 2: `IncompatibleLayer`.
2. Walk `0x60`. Decode each descriptor, recalculate its `derived_id` (DV-10) and record in the registry those that pass, with the persisted state.
3. Walk `0x62` and cross it with the descriptors (DV-9). The in-memory registry is built from the **descriptors**, not from the inverse index: the persisted index is checked against them.
4. For each `Ready` that passes, check that the head of its source has the revision and the `input_hash` of the descriptor. If it does not match, it becomes `Stale`. If the head matches, load its vector and check the `output_hash` (DV-12). If it fails, it becomes `Stale` and is reported.
5. Vectors without a `Ready` descriptor are not loaded.

**What happens in each case** (all tested in E8-04):

| Case | What the read serves | What `verify_units` reports | Who fixes it |
|---|---|---|---|
| Vector `0x61` without descriptor `0x60` | Nothing: not loaded | DV-9 | Explicit repair (`Derived` `ChangeSet` that deletes it) |
| Descriptor without `0x62` entry, or a different entry | The registry uses the descriptor; the state does not change | DV-9 | Explicit repair, which rewrites `0x62` |
| Descriptor whose `derived_id` does not match its key | Nothing: not loaded; its child keys are orphans | DV-10 | Explicit repair, or reconstruction |
| `Ready` without a vector, or with a different `output_hash` | `Stale`; not served | DV-12 | `rebuild` |
| `Ready` whose source is no longer the head | `Stale` (valid state, no finding) | none | `rebuild` |
| Descriptor of a deleted unit | None (its deletion removed it) | DV-14 if it exists | Explicit repair |

**Repair.** Repair is an explicit call of E8-04 that writes a `ChangeSet` with purpose `Derived`. Opening never repairs. A database with no derivatives never needs repair.

**Declared limit.** Checking the `output_hash` of each `Ready` on opening reads all persisted vectors: the opening cost is proportional to the volume of vectors. It is the accepted cost; it is measured in E8-07. If it becomes unacceptable, the check becomes lazy, and until then that change is a new decision, not an adjustment.

## Suppression (S5)

ADR-0009, option 3, applied to the class "derived vector" (the assignment that ADR-0009 A-1 asks for by class; the other classes remain unassigned). The treatment follows section 5.3 of the minutes.

1. **Same `ChangeSet`.** Deleting a unit (`Delete` revision) or purging a revision by retention deletes, in the same transaction, the descriptors, vectors and `0x62` entries of the derivatives of that source. They are found with the range `0x62 ‖ unit_id` (and, in retention, `0x62 ‖ unit_id ‖ rev_no`).
2. **What is deleted at once** (I-1, extended to derivatives): after the commit, no `get` or `scan` of `0x60`–`0x62` returns the deleted keys.
3. **Open snapshots** (I-2): a snapshot taken before the commit keeps reading descriptors and vectors until it is released.
4. **What survives on disk** until compaction: the bytes of a vector or of a descriptor may remain in a WAL or in a table until a compaction covers their range. That includes the `unit_id` of the source, which is in the key of `0x60` and of `0x62`. After `compact_range` covering the keys and a clean shutdown, the sentinel of a vector does not appear in any live file that does not retain a snapshot (C-1, C-3). The deadline is that of ADR-0009 (C-4): the compaction, or the release of the snapshot, and not a duration.
5. **No physical deletion.** The engine does not overwrite freed blocks (ADR-0009, point 3; `threat-model.md` §6.5).
6. **No cryptographic erasure.** It depends on ADR-0006, pending (ADR-0009, point 4). Persisted vectors are stored in plaintext.
7. **No declared backup window.** Today there is no copy function in the engine (ADR-0009, point 5; minutes D11).
8. **Relations.** Deleting a relation does not touch these derivatives: their source is a unit revision, not a relation. Derivatives with several sources (E8-06) will have their own rule.

**What changes with respect to E8-03.** Today E8-03 marks `stale` the derivatives of a deleted unit, because `DerivedChangeSet::commit` treats all revisions of the commit the same, and the `delete` of `ChangeSet` adds a deletion revision (`db.rs`, `delete` → `members.push`). The rule of this ADR is different: deletion, not staleness. E8-05 changes the code; this ADR fixes the persisted behaviour.

## Out of scope

- **Approximate index on disk.** Search remains exact (ADR-0011), over the vectors that the layer supplies. A persisted approximate index needs its own ADR.
- **Model in the core, and network.** No step of this layer calls a model or opens a connection (I-31). The profile is opaque text; the caller produces the vector.
- **Encryption.** ADR-0006, pending. The engine's `AEAD` bit remains rejected (`Incompatible("encryption not supported")`).
- **Summaries and derivatives with several sources**, from E8-06.
- **Physical deletion**, and any promise about the medium.

## E8-04 acceptance criteria

E8-04 is accepted if it meets each of these statements, with the test indicated. None is considered met by reading the code.

| # | Checkable statement | Test |
|---|---|---|
| T1 | Every value of §B (descriptor, vector, `0x62`, mark 2) encodes and decodes back byte by byte. There are committed vectors for every record type, positive and negative with their DV or UNIT rule | Vectors in `crates/kerno-units/tests/vectors/derived-v2/`, tested by the production codec |
| T2 | An independent decoder, written only from this ADR (§A–§D), agrees with the codec in rule, class and content, on the vectors and on byte edits (with CRC recalculated where applicable) | `spec_decoder_agrees_derived_v2`, same pattern as ADR-0013 |
| T3 | For every descriptor, the recalculated `derived_id` (§C.1) equals its key. Changing one byte of `source_rev` or `profile` makes DV-10 fail and the registry does not load it | Vector test and single-byte mutation |
| T4 | An `output_hash` different from the value's hash produces DV-12 and the vector is not served | `vector_hash_mismatch_is_not_served` |
| T5 | **A build without the feature rejects opening a database with derivatives.** A binary built from `d01f646` (last commit before E8-04) opens the fixture directory `derived-v2/db-with-vector` and returns `IncompatibleLayer` with rule UNIT-16 and `found_layer_version = 2`. The file tree (names and hash of each file) is identical before and after | Command over the reference binary; the result is recorded in the evidence of E8-04 |
| T6 | In the E8-04 suite, the same fixture, read with the layer 1 reader (`LayerMeta::decode` with maximum 1), gives UNIT-16 | `layer1_reader_refuses_derived_fixture` |
| T7 | `UnitDb::open` of that fixture with the E8-04 build opens and does not write: the file tree does not change | `open_derived_fixture_writes_nothing` |
| T8 | The mark becomes 2 in the same transaction as the first `0x60`–`0x62` key. A crash at each point of that transaction leaves, after recovery, either zero derived keys with mark 1, or keys with mark 2 | `crash.rs`: `derived_mark_every_point_{power_loss,process_kill}` |
| T9 | A database with derivatives does not go back to 1, neither with `set_policy` nor with any later `ChangeSet` | `mark_never_lowers` |
| T10 | A crash at each point of a `ChangeSet` that updates a unit with a `Ready` derivative leaves, after recovery, a state in which no `Ready` has a source different from the head (I-23, I-25, I-38) | `stale_mark_every_point_{power_loss,process_kill}` |
| T11 | A sync failure during that commit leaves no derivative marked and no vector written | `a_commit_that_fails_marks_nothing`, in its persisted version |
| T12 | Mutation: removing the marking from a `ChangeSet` that updates a unit turns T10 red | Mutation over a copy of the tree, never over the working one |
| T13 | Rebuilding after a source change gives the same `derived_id`, `input_hash`, `output_hash` and vector bytes as building again over the same sources (I-24) | Byte-by-byte comparison, like `rebuilt_equals_fresh` |
| T14 | After deleting all keys `0x60`–`0x62` and reopening, each unit and each read equals those from before (the truth does not depend on the derivatives) | `derived_loss_leaves_units_unchanged` |
| T15 | Each case of the §S4 table has its test: vector without descriptor, descriptor without `0x62`, `derived_id` that does not match, `Ready` without a vector, different `output_hash`, `Ready` with an old source. On opening nothing is served as `Ready` without a check, and `verify_units` reports the rule | `reopen_refuses_or_stales_each_damage_case` |
| T16 | Reopening a `Ready` whose source changed leaves it `Stale` before serving anything (I-23) | `reopen_after_source_change_is_stale` |
| T17 | After a `commit` that deletes a unit with dependants, `get` and `scan` of `0x60`–`0x62` do not return its keys (I-1, I-42) | `delete_hides_derived_at_once` |
| T18 | A snapshot taken before that `commit` keeps reading descriptors and vectors until it is released (I-2) | `snapshot_keeps_derived_until_released` |
| T19 | After `compact_range` covering the keys, a clean shutdown and no retained snapshot, the sentinel of a vector does not appear in any live file (C-1, C-3). The predictions are written before running | `derived_sentinel_survival`, E9A-01 form |
| T20 | Archiving or unarchiving the source marks `Stale` (does not delete). Deleting the unit deletes without marking. A retention that purges a revision deletes its derivatives in the same transaction | `archive_is_stale_delete_is_removed` |
| T21 | `supersede` writes in one transaction the new descriptor and its vector, moves the old one to `Superseded`, deletes its vector and keeps its `0x62` entry, like every descriptor | `supersede_is_one_changeset` |
| T22 | Writing a vector whose source is not the head in the transaction is rejected and writes nothing | `stale_source_write_is_refused` |
| T23 | A `Derived` purpose `ChangeSet` contains no unit or relation keys (DV-13) | Verification test with mutation |
| T24 | The sentinels of I-37 (`tests/sentinels.rs`) cover vector values and unit contents: none appears in errors, observations or `verify_units` | Extended sentinels |
| T25 | The vector guards of the engine format (`crates/kerno-storage/tests/vectors/v1/`) pass without a one-byte change, and `crates/kerno-storage/src/format/features.rs` does not change | `git diff` empty on those paths, and the CI guard |
| T26 | `Cargo.toml` and `Cargo.lock` add no dependencies. The code opens no connection (I-31) | Dependency review |

## Evidence

- **E8-03 passed** (see [D2 condition](#d2-condition-e8-03-passed)): 13 and 46 tests green.
- **Code reading** (commit `d01f646`):
  - `crates/kerno-units/src/codec.rs`: keys `0x40`–`0x42` and `0x50`–`0x52` are the only ones known above `0x3F`; UNIT-2 rejects the rest of nibble `0x4`–`0xF` with `IncompatibleLayer`.
  - `crates/kerno-units/src/db.rs`: `from_engine` calls `read_mark`, which decodes only the `layer` mark and, if it does not exist but there are keys, returns `NotAUnitDatabase`. No other key is read on opening. `begin_changeset_at` writes the mark in the transaction if it is missing.
  - `crates/kerno-units/src/derived.rs`: `DerivedDescriptor::id` hashes sources, method, version and parameters, with `F(·)` little-endian. There is no model-profile field nor date.
  - `crates/kerno-units/src/derived_stale.rs`: `input_hash` hashes `unit ‖ rev` in little-endian without a length prefix. `DerivedChangeSet::commit` marks after `self.cs.commit()?`.
  - `crates/kerno-units/src/types.rs`: `Purpose` is `Application=0, Retention=1, Maintenance=2, Relation=3, Index=4`. `RevisionId` sorts by `(unit, rev)`.
  - `crates/kerno-storage/src/format/features.rs`: `KNOWN_INCOMPAT = AEAD`, `WRITE_INCOMPAT = 0`, `KNOWN_RO_COMPAT = 0`, `KNOWN_COMPAT = 0`. They do not change.
  - `crates/kerno-units/src/assoc_rank.rs`: `VectorSet` has one dimension and returns `DimensionMismatch` for a vector of another.
- **No code was changed for this ADR.** The E8-04 tests do not exist yet.

## Discrepancies with the code and with the minutes

The code prevails, and these are the discrepancies that the ADR takes into account:

1. **The model profile is missing.** The minutes (§5.4, §7 I-25) and ADR-0018 ask for a descriptor with a profile; `DerivedDescriptor` has none and the `id` does not hash it. This ADR adds it (§B.1, §C.1). E8-04 changes the domain of the `id` to `…v2`.
2. **The descriptor date is missing.** ADR-0018 lists it; the code does not have it, and rightly so (I-24). This ADR leaves it in the `CHANGESET`.
3. **The marking is after the commit.** `DerivedChangeSet::commit` marks after the commit, in memory. Valid for memory, not for persisted data (§Atomicity).
4. **A deletion marks `stale` instead of deleting.** The current E8-03 does it (`db.rs` `delete` adds a revision; `derived_stale.rs` marks all). ADR-0009 and minutes §5.3 ask for deletion. E8-05 corrects it; this ADR fixes the persisted behaviour.
5. **Two endiannesses.** Keys and values are big-endian (`units-v1.md` §1); the identity hashes and `input_hash` are little-endian. Both are kept, and this ADR writes them down.
6. **Opening only looks at the mark.** `read_mark` reads only `layer`. That is why the feature has to be in the version mark, and not in a separate record or key.
7. **`set_policy` and the first `ChangeSet` write `LAYER_VERSION = 1` as a constant** (`db.rs` 625 and 1373). They must change so that they never lower the version (T9).
8. **`DerivedDb::open` receives the descriptor list from the caller.** With persistence, the source of truth is the database. The list stops being the source.
9. **`units-v1.md` says "Normative draft (E3-01)" in its status**, although the closing minutes of Stage 3 record its vectors. It is not changed here.
10. **ADR-0009 A-1** asks to assign each data class to an option, and its own table says "Not met". This ADR assigns the vector class. The other classes remain open.

## Consequences

- **Format.** Layer 2 is new; layer 1 and the engine do not change. A database without vectors remains layer 1 and opens with any layer 1 or 2 build. A database with vectors opens only with layer 2 or later builds. It is the rejection that the minutes ask for.
- **Engine.** No line of the engine's format changes. A tool that opens the engine directly, without the layer, reads the keys as bytes and rejects nothing. The compatibility boundary is the layer, not the engine.
- **Cost.** One descriptor per derivative, one `0x62` entry per descriptor and one vector per `Ready`. Opening reads all vectors to check their hash (§Reconstruction, declared limit).
- **Review when.** E8-07 measures the cost of opening and of the inverse index. If the cost of the inverse index is disproportionate, ADR-0018 requires reopening it. If search needs an approximate index, it is a new ADR.
- **Commitments.** The tags `0x60`–`0x62`, the fields of §B and the rules DV-1 to DV-13 are fixed when E8-04 is accepted. Changing them requires a new ADR and a new layer version, not an edit.

## Follow-up

- **E8-04**: implement §A–§D, meet T1–T26, and create `docs/file-format/units-v2.md`, which copies §A–§D as they are (the independent decoder is written from that document). Update `docs/file-format/README.md`.
- **E8-04, first step**: set the `layer_version` of layer 2 in `LAYER_VERSION` only when the database has derivatives. The constant 1 in `db.rs` can no longer be written into databases with derivatives (T9).
- **E8-05**: replace the deletion marking with the deletion of §S5 (discrepancy 4).
- **Outside this ADR**: ADR-0006 (encryption), ADR-0009 A-1 (assignment of the other classes), and the backup window of D11 when a copy function exists.
- **Review of ADR-0011**: the owner decides whether its row moves to "Replaced by ADR-0020" in its persistence part. This ADR does not edit ADR-0011.
</content>
