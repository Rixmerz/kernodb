# Unit layer v1: keys and values

| Field | Value |
|---|---|
| Status | Normative draft (E3-01); the owner sets it when E3-01 closes |
| Layer version | `layer_version = 1`, `envelope_version = 1` |
| Related to | [Stage 3 design](../architecture/stage3-design.md) §2, [E3-00 record](../architecture/stage3-record-e3-00.md), [ADR-0004](../adr/0004-revision-history.md), [ADR-0007](../adr/0007-index-consistency-and-rebuilding.md) (primary index, E4-03), [ADR-0008](../adr/0008-document-serialization-and-hashes.md) (accepted), [ADR-0014](../adr/0014-physical-representation-of-relations.md) (relations, E4-02), [engine v1 format](v1.md) |

This document specifies, byte by byte, the keys and the values that the unit layer stores as **ordinary keys and values** of the engine. It does not change any byte of the engine's v1 format ([v1.md](v1.md)): for the engine, everything here is opaque bytes. An implementation of the layer and the independent decoder of `crates/kerno-units/tests/spec_decoder` are written **only** from this text. The words "must" and "must not" are normative.

## 1. Conventions

| Type | Encoding |
|---|---|
| `u8` | 1 byte |
| `u32`, `u64` | 4 or 8 bytes, **big-endian** (the lexicographic order of the key is the numeric one) |
| `i64` | 8 bytes, big-endian, two's complement (µs since the Unix epoch; descriptive, no value restriction) |
| `str(max)` | `len: u32` ‖ `len` bytes of valid UTF-8; `len ≤ max` |
| `bytes(max)` | `len: u32` ‖ `len` opaque bytes; `len ≤ max`; **`len = 0` means absent** (optional fields) |
| `unit_id` | 16 opaque bytes |
| `ContentHash` | 33 bytes: `alg: u8` ‖ `digest: [u8; 32]` (§4) |

There is no alignment padding and there are no implicit fields. Every byte of a value belongs to a field of this document.

### 1.1 Common header of the values

Every value, except `REV_PAYLOAD` (§3.4), starts with:

| Offset | Field | Type | Required value |
|---|---|---|---|
| 0 | `envelope_version` | `u8` | `1`. `0` is corruption (UNIT-6); greater than `1` is `IncompatibleLayer` (UNIT-7) |
| 1 | `reserved_flags` | `u8` | `0`. Any other value is corruption (UNIT-8) |

### 1.2 Limits

| Limit | Value | Rule |
|---|---|---|
| `LAYER_META` name | 1..=64 B | UNIT-3, UNIT-4 |
| `unit_type` | 1..=64 B | UNIT-11, UNIT-12 |
| `metadata` | ≤ 4096 B | UNIT-11 |
| `actor_ref`, `source_ref` of `RevisionMeta` | ≤ 256 B each | UNIT-11 |
| `reason`, `actor_ref`, `source_ref`, `correlation` of `ChangeSetRecord` | ≤ 1024 B each | UNIT-11 |
| `payload_len` | ≤ 2^26 = 67 108 864 | UNIT-22 |
| `member_count` | ≤ 2^20 = 1 048 576 | UNIT-24 |
| `relation_type` of a relation (E4-02) | 1..=64 B, no NUL | UNIT-12 |
| `origin` of the provenance of a relation | ≤ 256 B | UNIT-11 |
| supported `layer_version` | 1 | UNIT-15, UNIT-16 |
| Key length | the one in §2 | UNIT-3 |

## 2. Keys

A key is `tag: u8` followed by the fields of the table. Lengths in bytes, tag included.

| `tag` | Name | Fields after the tag | Length |
|---|---|---|---|
| `0x01` | `LAYER_META` | `name`: UTF-8, 1..=64 B (no length prefix: it is the rest of the key) | 2..=65 |
| `0x10` | `UNIT_HEAD` | `unit_id[16]` | 17 |
| `0x20` | `REV_META` | `unit_id[16]` ‖ `rev_no: u64` | 25 |
| `0x21` | `REV_PAYLOAD` | `unit_id[16]` ‖ `rev_no: u64` | 25 |
| `0x30` | `CHANGESET` | `commit_seq: u64` | 9 |
| `0x31` | `CS_MEMBER` | `commit_seq: u64` ‖ `ordinal: u32` | 13 |
| `0x40` | `REL` (E4-02) | `edge_id[16]` | 17 |
| `0x41` | `REL_OUT` (E4-02) | `unit_id[16]` ‖ `relation_type` (1..=64 B of UTF-8 without NUL) ‖ `0x00` ‖ `edge_id[16]` | 35..=98 |
| `0x42` | `REL_IN` (E4-02) | same as `REL_OUT`, with `unit_id` = the `to` end | 35..=98 |
| `0x50` | `IDX_META` (E4-03) | none: there is a single record, that of the primary index | 1 |
| `0x51` | `IDX_PRIMARY` (E4-03) | `unit_id[16]` | 17 |
| `0x52` | `IDX_TYPE` (E4-04) | `len: u8` (1..=64) ‖ `unit_type` (`len` B of UTF-8) ‖ `schema_version: u32` ‖ `unit_id[16]` | 23..=86 (= 22 + `len`) |

Key rules, evaluated **in this order**:

- **UNIT-1** (`Corruption`): an unknown tag whose high nibble is `0x0`, `0x1`, `0x2` or `0x3` (`0x00`, `0x02..=0x0F`, `0x11..=0x1F`, `0x22..=0x2F`, `0x32..=0x3F`) is within the range of a known tag and is corruption. An empty key is too (it has no tag).
- **UNIT-2** (`IncompatibleLayer`): a tag with high nibble `0x4..=0xF`, except `0x40..=0x42` (E4-02), `0x50..=0x51` (E4-03) and `0x52` (E4-04), was written by a newer layer. A layer earlier than E4-02 rejects, by this rule, a store with relations (see ADR-0014, "Consequences"); one earlier than E4-03 rejects a store with the primary index built, and one earlier than E4-04 rejects a store built with `IDX_TYPE`. A store without any of those records opens anyway with all of them: the index only exists after a call to `rebuild_index`.
- **UNIT-3** (`Corruption`): the key length is not the one in the table (in `IDX_TYPE`, a `len` outside 1..=64 or a length other than 22 + `len`; in `REL_OUT`/`REL_IN`, a length outside 35..=98 or a byte other than `0x00` just before the 16 bytes of `edge_id`).
- **UNIT-12** also applies to the key of `REL_OUT`/`REL_IN`: the `relation_type` between `unit_id` and the separator is not UTF-8 or contains a NUL.
- **UNIT-12** also applies to the key of `IDX_TYPE`: a `unit_type` that is not UTF-8.
- **UNIT-4** (`Corruption`): the `name` of `LAYER_META` is not valid UTF-8.
- **UNIT-5** (`Corruption`): `rev_no = 0` in `REV_META`/`REV_PAYLOAD`, or `commit_seq = 0` in `CHANGESET`/`CS_MEMBER`. Revision numbers and commit sequences start at 1.

`unit_id` and `ordinal` have no value restriction.

## 3. Values

The fields are listed in byte order; the offsets of the variable-width fields depend on the preceding lengths.

### 3.1 `LAYER_META` (`0x01`)

| Field | Type |
|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` (§1.1) |
| `layer_version` | `u32` |
| `history_policy` | `u8`: `None=0`, `Audit=1`, `Versioned=2` |

Fixed size: 7 B. `layer_version = 0` is corruption (UNIT-15); `layer_version > 1` is `IncompatibleLayer` (UNIT-16). The by-name semantics of `history_policy` are fixed by E3-05; the record with the name `layer` is the layer mark (design §2.6).

### 3.2 `UNIT_HEAD` (`0x10`)

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `unit_type` | `str(64)` | not empty (UNIT-12) |
| `schema_version` | `u32` | none |
| `lifecycle` | `u8` | `Active=0`, `Archived=1`, `Deleted=2` (UNIT-13) |
| `head_rev` | `u64` | ≥ 1 (UNIT-17) |
| `oldest_rev` | `u64` | `1 ≤ oldest_rev ≤ head_rev` (UNIT-17) |
| `created_seq` | `u64` | ≥ 1 (UNIT-18) |
| `updated_seq` | `u64` | ≥ `created_seq` (UNIT-18) |
| `created_at`, `updated_at` | `i64` | none |
| `content_hash` | `ContentHash` | §4 |
| `payload_len` | `u32` | ≤ 2^26 (UNIT-22) |
| `payload_codec` | `u8` | `0` (UNIT-13) |
| `metadata` | `bytes(4096)` | |

### 3.3 `REV_META` (`0x20`)

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `kind` | `u8` | `Create=1`, `Update=2`, `Archive=3`, `Unarchive=4`, `Delete=5`, `Restore=6`; `0` and `7..=255` are corruption (UNIT-13). `7..=15` are reserved for future versions of the layer |
| `parent_rev` | `u64` | `0` if and only if `kind = Create`; otherwise, `1 ≤ parent_rev < rev_no` of the key (UNIT-19) |
| `commit_seq` | `u64` | ≥ 1 (UNIT-23) |
| `created_at` | `i64` | none |
| `schema_version` | `u32` | none |
| `content_hash` | `ContentHash` | §4 |
| `payload_len` | `u32` | ≤ 2^26 (UNIT-22) |
| `payload_codec` | `u8` | `0` (UNIT-13) |
| `payload_present` | `u8` | `0` or `1`; `Delete` requires `payload_present = 0` and `payload_len = 0` (UNIT-21) |
| `restored_from` | `u64` | `0` except in `Restore`; in `Restore`, `1 ≤ restored_from < rev_no` (UNIT-20) |
| `metadata` | `bytes(4096)` | |
| `actor_ref` | `bytes(256)` | |
| `source_ref` | `bytes(256)` | |

The values of `REV_META` are immutable: a key is written once (design §3.1).

### 3.4 `REV_PAYLOAD` (`0x21`)

The value is the payload **as is**: without `envelope_version`, without a length, without padding. Its length and its integrity are not validated when the value is decoded on its own: they are fixed by `payload_len` (≤ 2^26, UNIT-22) and by `content_hash` of the corresponding `REV_META` (design §2.7), and the layer checks both when it reads the revision (§5.1, E3-02). That is why there is no `UNIT-n` decoding rule for it: any value, of any length the engine admits, is syntactically valid as `REV_PAYLOAD`, and a value whose length does not match `payload_len` is corruption detected by that cross-check, not by this decoder.

### 3.5 `CHANGESET` (`0x30`)

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `created_at` | `i64` | none |
| `member_count` | `u32` | ≤ 2^20 (UNIT-24) |
| `purpose` | `u8` | `Application=0`, `Retention=1`, `Maintenance=2`, `Relation=3`, `Index=4` (UNIT-13). `Relation` is written by the layer for an application `ChangeSet` that only creates or deletes relations (without members, E4-02). `Index` is written for the `ChangeSet` that builds the primary index or records that its build failed (without members, E4-03) |
| `reason`, `actor_ref`, `source_ref`, `correlation` | `bytes(1024)` each | |

### 3.6 `CS_MEMBER` (`0x31`)

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `unit_id` | `[u8; 16]` | none |
| `rev_no` | `u64` | ≥ 1 (UNIT-25) |
| `kind` | `u8` | as `REV_META.kind` (UNIT-13) |

Fixed size: 27 B.

### 3.7 `REL` (`0x40`, E4-02)

A directed relation from `from` to `to` ([ADR-0014](../adr/0014-physical-representation-of-relations.md), option 2).

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `from`, `to` | `[u8; 16]` | none (the end units must be `Active`: UNIT-36) |
| `relation_type` | `str(64)` | 1..=64 B, no NUL (UNIT-12) |
| `created_seq` | `u64` | ≥ 1 (UNIT-23); the `CHANGESET` of that sequence stores actor, origin and reason |
| `provenance` | `u8` | `Declared=0`, `Imported=1`, `Algorithmic=2`, `Inferred=3` (UNIT-13). **Mandatory**: there is no default value and no way to omit it |
| `origin` | `bytes(256)` | empty with `Declared`; not empty with the others (UNIT-13) |
| `confidence` | `u16` | thousandths, `0..=1000`; `0` except with `Inferred` (UNIT-13) |

A `REL` has no revisions of its own: deleting it deletes the record and its two entries, and the `ChangeSet` that did it is the only trace.

### 3.8 `REL_OUT` and `REL_IN` (`0x41`, `0x42`, E4-02)

Entries of the adjacency index, written and deleted in the same `ChangeSet` as the `REL` (I-21). The key sorts by unit, type and `edge_id`, so that the neighbours of a unit (with or without a type) form a range. The NUL after the type prevents a type that is a prefix of another (`cite`, `cites`) from matching the entries of the second.

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `other` | `[u8; 16]` | the other end of the relation (`to` in `REL_OUT`, `from` in `REL_IN`); it allows reading the neighbour without opening the `REL` |

Fixed size: 18 B.

### 3.9 `IDX_META` (`0x50`, E4-03)

The freshness state of the primary index that is stored ([ADR-0007](../adr/0007-index-consistency-and-rebuilding.md), I-23). There is a single record. It is written by the `ChangeSet` of purpose `Index` that builds the index (`UnitDb::rebuild_index`), in the same transaction as the entries and the deletion of the previous ones (I-21): a store has either the whole previous generation or the whole new one.

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `outcome` | `u8` | `Succeeded=0`, `Failed=1` (UNIT-13): how the last build attempt ended. `Building` and `Stale` **are not stored**: the first belongs to a handle in progress and the second is computed against a snapshot |
| `watermark` | `u64` | the commit sequence that the entries reflect: that of the `ChangeSet` that wrote them (no unit changes, so the entries are a function of the state at that commit). `0` only if nothing was ever built; with `Succeeded` it is ≥ 1 (UNIT-23) |
| `entries` | `u64` | number of `IDX_PRIMARY` entries of that generation. With `watermark = 0` it is `0` (UNIT-13) |

Fixed size: 19 B. With `Failed`, `watermark` and `entries` are those of the generation that remains stored (or `0` and `0` if there is none): a failed attempt does not change any entry. The state that a query sees is a function of this record and of its snapshot: `Failed` if `outcome = Failed`; otherwise, `Ready` when `watermark` is the snapshot's sequence and `Stale` when it is earlier; without a record, `Stale` with no watermark. A `watermark` later than the snapshot is corruption.

### 3.10 `IDX_PRIMARY` (`0x51`, E4-03)

The primary-index entry of an `Active` unit: the `UNIT_HEAD` fields that a basic filter reads. There is an entry only for `Active` units (archived and deleted ones do not have it). The key sorts by `unit_id`, as `UNIT_HEAD` does, so walking the index gives the units in the same order as the reference path.

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1 |
| `unit_type` | `str(64)` | not empty (UNIT-12); that of the unit's head |
| `schema_version` | `u32` | none; that of the unit's head |

### 3.11 `IDX_TYPE` (`0x52`, E4-04)

The secondary index by `(unit_type, schema_version)`, justified by workload W1 ([stage4-workloads.md](../architecture/stage4-workloads.md)). One entry for each `IDX_PRIMARY` entry and no more: the same build writes it, in the same transaction and under the same `IDX_META` (§3.9), so it shares the watermark, the state and the behaviour under crashes, and a rebuild replaces both at once (I-21, I-23, I-24). Everything is in the key; the length prefix makes the range of one type not contain that of another type that has it as a prefix. The order is length, type, schema version and `unit_id`: a filter with type and version walks its range in id order; with type only, the result is re-sorted by id.

**Compatibility with E4-03 indexes.** `IDX_META` does not record whether the generation includes `IDX_TYPE`: a store whose last index was built by E4-03 (without `IDX_TYPE`) would be read as `Ready`, and a query by type would return empty. No such store exists: the Stage 4 format was not published, and E4-03 and E4-04 form a single format. If one day indexes of a published version without `IDX_TYPE` must be read, `IDX_META` must carry a generation version that marks them `Stale`.

| Field | Type | Constraint |
|---|---|---|
| `envelope_version`, `reserved_flags` | `u8`, `u8` | §1.1; there are no further fields (UNIT-10 if bytes are left over) |

Fixed size: 2 B.

## 4. `ContentHash`

| `alg` | Algorithm | `digest` |
|---|---|---|
| `0` | Provisional CRC-32C, **without cryptographic guarantee** | `crc: u32` big-endian ‖ 28 bytes `0x00` (UNIT-14) |
| `1` | BLAKE3-256 (ADR-0008, accepted) | 32 arbitrary bytes |

The hash covers `payload_codec ‖ payload` (a codec byte followed by the payload, without a length). Any other `alg` is corruption (UNIT-13). The writers of this layer use `alg = 1`. The structure decoder does not check the hash; the payload verification of §5.1 does (E3-02, `kerno_units::verify_payload`). The offline verifier of E3-07 reuses it.

## 5. Validation rules

Each rule carries its error class: `Corruption` (the bytes are not what a writer of this layer produces) or `IncompatibleLayer` (a newer layer wrote them; nothing is modified).

| Id | Rule | Class |
|---|---|---|
| UNIT-1 | Unknown tag within the range of a known tag, or empty key | `Corruption` |
| UNIT-2 | Tag with high nibble `0x4..=0xF`, except `0x40..=0x42` and `0x50..=0x52` | `IncompatibleLayer` |
| UNIT-3 | Key length different from the one in §2 | `Corruption` |
| UNIT-4 | `name` of `LAYER_META` is not UTF-8 | `Corruption` |
| UNIT-5 | `rev_no = 0` or `commit_seq = 0` in the key | `Corruption` |
| UNIT-6 | `envelope_version = 0` | `Corruption` |
| UNIT-7 | `envelope_version > 1` | `IncompatibleLayer` |
| UNIT-8 | `reserved_flags ≠ 0` | `Corruption` |
| UNIT-9 | A field runs past the end of the value (truncated value, empty one included) | `Corruption` |
| UNIT-10 | Bytes left over after the last field | `Corruption` |
| UNIT-11 | The length prefix of a `str`/`bytes` exceeds the maximum of the field | `Corruption` |
| UNIT-12 | `unit_type` or `relation_type` empty or not UTF-8 (also that of `IDX_PRIMARY`); `relation_type` with NUL (also in the key of `REL_OUT`/`REL_IN`) | `Corruption` |
| UNIT-13 | Enumeration value outside the table: `lifecycle`, `kind`, `purpose`, `history_policy`, `alg`, `provenance`, `outcome`; `payload_codec ≠ 0`; an inconsistent `REL` provenance (§3.7); or an `IDX_META` with `entries ≠ 0` and `watermark = 0` (§3.9) | `Corruption` |
| UNIT-14 | `alg = 0` with the last 28 bytes of the `digest` non-zero | `Corruption` |
| UNIT-15 | `layer_version = 0` | `Corruption` |
| UNIT-16 | `layer_version > 1` | `IncompatibleLayer` |
| UNIT-17 | `UNIT_HEAD`: `1 ≤ oldest_rev ≤ head_rev` is not met | `Corruption` |
| UNIT-18 | `UNIT_HEAD`: `1 ≤ created_seq ≤ updated_seq` is not met | `Corruption` |
| UNIT-19 | `REV_META`: `parent_rev` inconsistent with `kind` and `rev_no` (§3.3) | `Corruption` |
| UNIT-20 | `REV_META`: `restored_from` inconsistent with `kind` and `rev_no` (§3.3) | `Corruption` |
| UNIT-21 | `REV_META`: `payload_present ∉ {0,1}`, or `Delete` with a payload | `Corruption` |
| UNIT-22 | `payload_len > 2^26` | `Corruption` |
| UNIT-23 | `REV_META` or `REL`: `commit_seq` / `created_seq` = 0; `IDX_META` with `outcome = Succeeded` and `watermark` = 0 | `Corruption` |
| UNIT-24 | `CHANGESET`: `member_count > 2^20` | `Corruption` |
| UNIT-25 | `CS_MEMBER`: `rev_no = 0` | `Corruption` |

### 5.1 Payload integrity (E3-02)

When a revision is read, the layer compares the stored `REV_PAYLOAD` (or its absence) with the fields `payload_codec`, `payload_len`, `payload_present` and `content_hash` of its `REV_META`. The rules apply in this order, and the first one violated is rejected. The length is compared before the hash is computed, so a truncated or oversized value is never processed.

| Id | Rule | Class |
|---|---|---|
| UNIT-13 | `payload_codec ≠ 0` (repeated here because the declaration may come from another source) | `Corruption` |
| UNIT-22 | `payload_len > 2^26` (repeated) | `Corruption` |
| UNIT-26 | `payload_present = 1` without `REV_PAYLOAD`; `REV_PAYLOAD` present with `payload_present = 0`; or `payload_present = 0` with `payload_len ≠ 0` | `Corruption` |
| UNIT-27 | The length of the `REV_PAYLOAD` differs from `payload_len` (truncated or extended) | `Corruption` |
| UNIT-28 | The hash of `payload_codec ‖ payload` with the declared `alg` differs from `content_hash` | `Corruption` |

This verification does not repeat UNIT-14. With `alg = 0`, a non-zero padding in the `digest` is rejected here as UNIT-28, because the computed digest always carries that padding as zero. The structure decoder already rejects it earlier, as UNIT-14.

The verification is pure: it does not write, does not depend on anything outside its arguments, and returns the same error for the same input. A rejection does not modify the database and can be repeated. Only a verified payload (`VerifiedPayload`) reaches the rest of the layer. Known hash vectors and corruption tests: `crates/kerno-units/tests/payload_integrity.rs`.

### 5.2 Rules between records (E3-07)

The offline verifier (`verify_units`, `crates/kerno-units/src/verify.rs`) walks all the keys of a snapshot and, besides UNIT-1..UNIT-28, checks these rules between records. They change no byte: they describe which combinations of records, each valid on its own, a writer of this layer produces. All are `Corruption`. A record that does not decode is reported once (by its rule in §5) and is excluded from these checks, so as not to report it again as "missing".

| Id | Rule | Invariant |
|---|---|---|
| UNIT-29 | A store with keys has no `LAYER_META` mark `layer`, or a `LAYER_META` name is neither `layer` nor `policy/<unit_type>` (1..=57 B) | §2.6 of the design |
| UNIT-30 | Orphan record: `REV_META`/`REV_PAYLOAD` without `UNIT_HEAD`, `REV_PAYLOAD` without its `REV_META`, `CS_MEMBER` without its `CHANGESET` | IU-2, IU-4 |
| UNIT-31 | Inconsistent head: a `REV_META` is missing in `[oldest_rev, head_rev]`, there are `REV_META`/`REV_PAYLOAD` outside that range, the head differs from the `REV_META` of `head_rev` (hash, length, schema, metadata, `updated_seq = commit_seq`, lifecycle deduced from `kind`), the current revision, if it is not a tombstone, has no payload, or `created_seq` is not the `commit_seq` of revision 1 (if it is retained; if not, it is less than that of `oldest_rev`) | IU-2 |
| UNIT-32 | Impossible chain: `Create` outside revision 1 or revision 1 that is not `Create`, `parent_rev ≠ rev_no − 1`, `commit_seq` not strictly increasing or later than the snapshot, a lifecycle transition that no writer produces (`Update`/`Archive`/`Restore` over a non-`Active`, `Unarchive` over a non-`Archived`, anything after `Delete`), or `Archive`/`Unarchive`/`Restore` whose hash, length, schema or metadata differ from those of its retained origin | IU-3 |
| UNIT-33 | Provenance: a `REV_META` without a `CHANGESET` at its `commit_seq`, with a `CHANGESET` whose `purpose` is not `Application`, or without a `CS_MEMBER` of that `ChangeSet` that names it (`unit_id`, `rev_no`, `kind`); members that are not exactly the ordinals `0..member_count`; a unit named twice in one `ChangeSet`; `Application`/`Retention` without members, or `Maintenance`/`Relation`/`Index` with members; a member of `Application` that names a unit that does not exist, a revision later than the head, or a retained revision with another `commit_seq` or another `kind`. The members of `Retention` are checked only in the direction `REV_META → member` (they name the first retained revision, not one they wrote) | IU-4 |
| UNIT-34 | Policy not applied, only where the policy in force when writing is known (revisions confirmed after the last `ChangeSet` `Maintenance`): under `none`, `oldest_rev ≠ head_rev`; under `audit`, a replaced revision that keeps `payload_present = 1` | IU-9 |
| UNIT-35 | Between two snapshots, a `REV_META`/`REV_PAYLOAD` key present in both changed its value, except the step of `payload_present` from 1 to 0 that a policy makes (`verify_immutable`) | IU-1 |
| UNIT-36 | Relations (E4-02): a `REL` without its `REL_OUT` or without its `REL_IN` (with the same type and the right other end), a `REL_OUT`/`REL_IN` entry without its `REL` or that does not match it, a `REL` with `created_seq` later than the snapshot, or an end that does not exist or is not `Active` (archiving or deleting a unit with relations is rejected) | I-21 |
| UNIT-37 | Primary index (E4-03): `IDX_PRIMARY` entries without `IDX_META`; a `watermark` later than the snapshot; an `entries` that is not the number of stored entries; a unit that has not changed since the watermark (`updated_seq ≤ watermark`) and that, being `Active`, has no entry or has it with another `unit_type` or `schema_version`, or that, not being `Active`, has one; an entry of a unit that does not exist or that was created after the watermark. A unit that changed after the watermark may differ from its entry: that is the obsolescence the state declares, not damage | I-23 |
| UNIT-38 | Secondary index by type (E4-04): an `IDX_PRIMARY` entry without the `IDX_TYPE` entry with its same `unit_type`, `schema_version` and `unit_id`, or an `IDX_TYPE` entry without that `IDX_PRIMARY` entry. The units with an entry that does not decode (in either of the two) are already a structure finding and are skipped | I-23, I-24 |

For payload integrity the verifier applies §5.1 to each `REV_META`, with one exception: a non-tombstone revision with `payload_present = 0` is one that an `audit` period emptied while keeping its hash and length (note of E3-05 of the design); for it only the absence of `REV_PAYLOAD` is required (UNIT-26), and the part of UNIT-26 that reads "`payload_present = 0` with `payload_len ≠ 0`" is not applied.

## 6. Truncated or corrupt input

A decoder receives the pair `(key, value)` and returns the fields or **one** rejection `(UNIT-n, class)`. It must:

1. **Not panic, not read outside the buffer, and not reserve memory from a length without validating it**: a length prefix is compared with the maximum of the field (UNIT-11) and with the bytes that remain (UNIT-9) before the body is read. The maximum is checked first: a `0xFFFFFFFF` prefix is UNIT-11, not UNIT-9.
2. **Reject the first violated rule** in this order: key rules (§2) → header (UNIT-6, UNIT-7, UNIT-8) → fields in byte order, each with its own checks (UNIT-9, UNIT-11, UNIT-12, UNIT-13, UNIT-14, UNIT-15, UNIT-16, UNIT-22, UNIT-23, UNIT-24, UNIT-25 as soon as the field is read) → bytes left over (UNIT-10) → rules between fields (UNIT-17..UNIT-21, the one of `IDX_META` with `watermark = 0`, and the coherence of the `REL` provenance, all after UNIT-10). In `REL` the `provenance` value outside `0..=3` is rejected when the field is read (UNIT-13), but the coherence between `provenance`, `origin` and `confidence` of §3.7 (`Declared` without origin or confidence; `Imported`/`Algorithmic` with origin and confidence 0; `Inferred` with origin and confidence ≤ 1000) is checked only after the whole value has been read and UNIT-10 has been applied: an inconsistent `REL` with a leftover byte is UNIT-10, not UNIT-13.
3. **Distinguish truncation from corruption only by the rule**, not by the class: both are `Corruption`. An empty value is UNIT-9, and never a new database.
4. Not "repair", fill in or ignore anything. A rejection does not modify the database (I-18 if it is read-only).
5. A tag with high nibble `0x4..=0xF`, except `0x40..=0x42` and `0x50..=0x52` (UNIT-2), or a greater `envelope_version`/`layer_version` (UNIT-7, UNIT-16), is rejected as `IncompatibleLayer` **without interpreting the rest**.

The production codecs (E3-02) are in `crates/kerno-units/src/codec.rs` (`Key`, `Record` and a type per value) and follow this order; the errors, with the rule and its class, are in `src/error.rs`. The test `production_codec_matches_committed_vectors` of `tests/struct_vectors.rs` requires them to decode each vector of §7 as the table indicates and to reproduce the positive ones byte by byte; `spec_decoder` still does not depend on them.

## 7. Vectors

`crates/kerno-units/tests/vectors/v1/structs/<name>.bin` stores each vector as `key_len: u32` (big-endian) ‖ `key` ‖ `value`. The positive ones are called `<structure>_<case>.bin`; the negative ones, `neg_unit<n>_<case>.bin`, and there is at least one per `UNIT-n` rule. The table of expected fields (positive) and of expected rule and class (negative) lives in `tests/struct_vectors.rs`; changing a byte of an existing vector follows [v1.md §15](v1.md) in spirit: a new layer version and an ADR. To regenerate:

```text
cargo test -p kerno-units --test struct_vectors -- --ignored regenerate_struct_vectors
```
