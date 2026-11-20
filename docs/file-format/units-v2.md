# Unit layer v2: persisted derived data

| Field | Value |
|---|---|
| Status | Normative since E8-04. It copies sections A–D of [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md), which prevails if there is a discrepancy |
| Layer version | `layer_version = 2` (feature `DERIVED_VECTORS`), `envelope_version = 1`. [Version 3](units-v3.md) (public identifiers) includes it and raises the maximum known |
| Related to | [unit layer v1](units-v1.md) (not edited; this document only adds), [ADR-0020](../adr/0020-persistence-of-vectors-as-derivatives.md), [ADR-0018](../adr/0018-versioning-and-rebuilding-of-derived-artifacts.md), [ADR-0008](../adr/0008-document-serialization-and-hashes.md) (BLAKE3-256), [ADR-0013](../adr/0013-format-v1-and-engine-protocol.md) (engine v1 format, frozen), [engine v1 format](v1.md) |

This document specifies, byte by byte, what the unit layer adds in version 2: persisted derived data (today, vectors). As in version 1, they are **ordinary engine keys and values**: no byte of the engine's v1 format ([v1.md](v1.md)) changes, nor its list of feature bits (`KNOWN_INCOMPAT` does not change). The compatibility boundary is the layer mark, not the engine. An implementation and the independent decoder `spec_decoder_agrees_derived_v2` (`crates/kerno-units/tests/derived_vectors.rs`) are written **only** from this text and from that of version 1. The words "must" and "must not" are normative.

## 1. Conventions

Those of [units-v1.md](units-v1.md) §1, unchanged: **big-endian** integers, `str(max)` = `len: u32` ‖ UTF-8 bytes, the two-byte common header (`envelope_version = 1`, `reserved_flags = 0`; UNIT-6, UNIT-7, UNIT-8), `unit_id` of 16 bytes. There is **one exception**, stated explicitly because it is what the code of E8-02 and E8-03 does: the **hash preimages** (§5) use **little-endian** integers.

## 2. What version 2 of the layer is

- The `LAYER_META "layer"` mark (units-v1 §3.1) carries `layer_version = 2` **if and only if** the database has or had a persisted derived datum. A database with none stays at `layer_version = 1` and is opened by any build of layer 1 or 2.
- Version 2 is written **in the same transaction** as the first `0x60`–`0x62` key. **It is never lowered**: not by `set_policy`, not by any later `ChangeSet`, not by deleting all the derived data. Whoever rewrites the mark keeps its `history_policy`.
- A layer 1 reader reads the mark before any other key and rejects it: `IncompatibleLayer` with rule UNIT-16 and `found_layer_version = 2`. It writes nothing. A reader of this version rejects `layer_version > 2` with the same rule. (Since E9-02, [version 3](units-v3.md) raises the maximum to 3: the row `mark_unit16_v3` of §8 became `mark_unit16_v4`, and layer 2 readers reject a mark 3.)
- A mark with `layer_version = 1` and `0x60`–`0x62` keys is corruption (DV-11).

## 3. Keys

| `tag` | Name | Fields after the tag | Length |
|---|---|---|---|
| `0x60` | `DERIVED_DESC` | `derived_id[32]` | 33 |
| `0x61` | `DERIVED_VEC` | `derived_id[32]` | 33 |
| `0x62` | `DERIVED_SRC` | `unit_id[16]` ‖ `rev_no: u64` ‖ `derived_id[32]` | 57 |

- `derived_id` is the identifier of §5.1. Each derived datum is a descriptor, at most one vector and exactly one `DERIVED_SRC` entry.
- `DERIVED_SRC` sorts by unit and then by revision: "all the derived data of a unit" is the range with prefix `0x62 ‖ unit_id`, and "all those of a revision" the one with `0x62 ‖ unit_id ‖ rev_no`.
- `rev_no = 0` in a `0x62` key is UNIT-5. A different length is DV-3.
- Tags `0x63`–`0x6F`: reserved. A layer 2 that finds them rejects them with UNIT-2 (`IncompatibleLayer`). In a layer 1, the whole `0x6` nibble falls under UNIT-2.

## 4. Values

### 4.1 `DERIVED_DESC` (`0x60`)

| # | Field | Type | Constraint |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` |
| 3 | `kind` | `u8` | `1` = `Vector`. `0` and `2..=255`: DV-1 |
| 4 | `state` | `u8` | `1` = `Ready`, `2` = `Stale`, `3` = `Superseded`. The others: DV-2 |
| 5 | `source_unit` | `[u8; 16]` | none |
| 6 | `source_rev` | `u64` | ≥ 1 (DV-4) |
| 7 | `method` | `str(64)` | 1..=64 B, UTF-8 (DV-4) |
| 8 | `method_version` | `u32` | none |
| 9 | `profile` | `str(64)` | 1..=64 B, UTF-8 (DV-4). Model profile, opaque text |
| 10 | `param_count` | `u8` | 0..=16 (DV-5) |
| 11 | `params` | `param_count` × (`key: str(64)` ‖ `value: str(256)`) | `key` 1..=64 B, `value` 0..=256 B, keys in **increasing byte order**, no repeats, UTF-8 (DV-5) |
| 12 | `input_hash` | `[u8; 32]` | §5.2 |
| 13 | `output_hash` | `[u8; 32]` | §5.3 |

No spare bytes (UNIT-10). **Precedence of lengths:** a text length outside its maximum (or zero where it is not allowed) is the DV rule of its field (DV-4, DV-5), not UNIT-11; if the length is valid and bytes are missing, UNIT-9. Inside a parameter, the key is evaluated, then the value, then the order with respect to the previous key. The descriptor **carries no date**: that of a derived datum is the `created_at` of the `CHANGESET` of its commit.

### 4.2 `DERIVED_VEC` (`0x61`)

| # | Field | Type | Constraint |
|---|---|---|---|
| 1 | `envelope_version` | `u8` | `1` |
| 2 | `reserved_flags` | `u8` | `0` |
| 3 | `dim` | `u32` | 1 ≤ `dim` ≤ 4096 (DV-6) |
| 4 | `elem_type` | `u8` | `1` = `f32` IEEE 754 binary32. Another value: DV-7. A new type requires a new layer version |
| 5 | `data` | `dim` × 4 B | each element with its bits in **big-endian**; finite (DV-8) |

Total length: `7 + 4·dim` bytes; another length is DV-6. `NaN`, `+∞`, `−∞` and `−0.0` (bits `0x80000000`) are DV-8: the writer writes `−0.0` as `+0.0`, so that the output hash does not depend on the sign of zero.

### 4.3 `DERIVED_SRC` (`0x62`)

Two bytes: `envelope_version = 1`, `reserved_flags = 0`. No further fields (UNIT-10). The key carries the source; the value repeats nothing.

### 4.4 Layer mark

The value of units-v1 §3.1 with `layer_version = 2` (`u32` big-endian). Same record, no change in shape. UNIT-15 and UNIT-16 of version 1 are read this way: `0` is UNIT-15, and a value greater than the maximum the reader knows is UNIT-16 with the value found.

### 4.5 `CHANGESET`, purpose `5` (`Derived`)

`purpose = 5` is written by the layer for a `ChangeSet` that writes only `0x60`–`0x62` keys, without members, as `Relation` (3) and `Index` (4) do. A layer 1 reader rejects it as outside the table (UNIT-13). With `member_count ≠ 0` it is corruption (DV-13). A `ChangeSet` that writes units and derived data is of purpose `Application`.

## 5. Identity and hashes

All the functions use BLAKE3-256 (ADR-0008). `F(x)` is the `u64` length field **little-endian** (8 bytes) followed by `x`.

### 5.1 `derived_id` (key of `0x60`, `0x61` and `0x62`)

```
derived_id = BLAKE3-256(
    "kerno.derived.identity.v2"                      -- ASCII, without F(·)
    ‖ F(u64le(n))                                    -- n = number of sources (1 in vectors)
    ‖ por cada fuente, en orden (unit, rev):
        F(unit[16]) ‖ F(u64le(rev))
    ‖ F(method) ‖ F(u32le(method_version))
    ‖ F(profile)
    ‖ F(u64le(m))                                    -- m = number of parameters
    ‖ por cada parámetro, en orden creciente de clave:
        F(key) ‖ F(value)
)
```

Outside the identifier: `input_hash`, `output_hash` and the date. The `…v1` domain of E8-02 (without `profile`) was never persisted.

### 5.2 `input_hash` (without `F(·)`, little-endian)

```
input_hash = BLAKE3-256(
    "kerno.derived.input.v1"                          -- ASCII
    ‖ u64le(n)                                        -- n = number of sources
    ‖ por cada fuente, en orden (unit, rev):
        unit[16] ‖ u64le(rev)
)
```

### 5.3 `output_hash`

`BLAKE3-256` of all the bytes of the `DERIVED_VEC` value (`7 + 4·dim` bytes, header included). It is set by the writer; the reader checks it before serving the vector.

## 6. Validation rules

Evaluation order, as in units-v1 §6: key, envelope, fields in order, rules between fields. The rules UNIT-6 to UNIT-10 apply to the new values.

| Id | Rule | Class |
|---|---|---|
| DV-1 | `kind ∉ {1}` | `Corruption` |
| DV-2 | `state ∉ {1, 2, 3}` | `Corruption` |
| DV-3 | Key length ≠ 33 (`0x60`, `0x61`) or ≠ 57 (`0x62`) | `Corruption` |
| DV-4 | `source_rev = 0`; `method` or `profile` empty, longer than 64 B or not UTF-8 | `Corruption` |
| DV-5 | `param_count > 16`; empty key or longer than 64 B; value longer than 256 B; keys not increasing or repeated; not UTF-8 | `Corruption` |
| DV-6 | `dim = 0`, `dim > 4096`, or value length ≠ `7 + 4·dim` | `Corruption` |
| DV-7 | `elem_type ≠ 1` | `Corruption` |
| DV-8 | Element `NaN`, `±∞` or `−0.0` | `Corruption` |
| DV-9 | Vector or `0x62` entry without a descriptor; descriptor without a `0x62` entry; or a `0x62` entry whose `unit`/`rev` are not those of the descriptor | `Corruption` |
| DV-10 | The recomputed `derived_id` (§5.1) ≠ the key | `Corruption` |
| DV-11 | `0x60`–`0x62` keys in a database whose mark has `layer_version = 1` | `Corruption` |
| DV-12 | `Ready` without a vector, or with an `output_hash` different from the hash of the value; `Stale` or `Superseded` with a vector | `Corruption` |
| DV-13 | `purpose = 5` with `member_count ≠ 0`. The other half of ADR-0020 (a `ChangeSet` of purpose `Derived` that touches keys of units or relations) cannot be seen from the data, because the keys do not carry the commit that wrote them: the layer does not write that form | `Corruption` |
| DV-14 | A descriptor whose source unit does not exist, or whose head is in `Deleted` | `Corruption` |
| UNIT-2 (extended) | Nibble tags `0x4`–`0xF` outside `0x40`–`0x42`, `0x50`–`0x52` and `0x60`–`0x62` | `IncompatibleLayer` |
| UNIT-16 (extended) | `layer_version` greater than the one the reader knows (2 in this version, 1 in a layer 1) | `IncompatibleLayer` |

A descriptor that fails DV-10 (or any rule of its value) is not loaded: its vector is not served and it does not occupy the record. Its child keys are left orphaned and are not reported separately.

## 7. Atomicity, reopening and suppression (normative)

1. The descriptor, the vector, the `0x62` entry and, if needed, the layer mark are written in **one** transaction of the engine. Writing a derived datum is only valid if its source revision is the head of its unit in that transaction; otherwise it is rejected (`InvalidArgument`) and nothing is written.
2. A source change (any revision that is not `Delete`) walks `0x62 ‖ unit_id` and, for each `Ready` descriptor, writes `state = Stale` and deletes its vector, in the same transaction. `Stale` and `Superseded` are not touched.
3. A unit deletion, and a retention that purges a revision (with the range `0x62 ‖ unit_id ‖ rev_no` of the purged revisions), delete the descriptors, vectors and `0x62` entries of those derived data in the same transaction. They do not leave them as obsolete.
4. Opening does not write. The record comes out of the descriptors: a `Ready` whose source is no longer the head (or whose `input_hash` does not match), or whose vector is missing, does not decode, or does not match its `output_hash`, is served as `Stale` and its vector is not served. Repairing is an explicit call that writes a `ChangeSet` of purpose `Derived`.
5. What ADR-0009 (option 3) declares survives on disk until compaction; there is no physical or cryptographic deletion. The vectors are in clear.

## 8. Vectors

`crates/kerno-units/tests/derived_vectors.rs` (`t01_vectors_of_the_document_match_the_codec_byte_for_byte`) builds each row with the production codec and compares it **byte by byte** with this table, and decodes the hex of this table and checks the result. `ok` is a value that is decoded and encoded again to the same bytes; any other value is the rule that rejects it. The types `mark1` and `changeset1` are read with the layer 1 reader (that of a build prior to E8-04); `mark` and `changeset` with that of this version. Common inputs: unit `0102…10`, revision 2, method `embed` version 3, profile `model-a`, parameters `dim=4` and `norm=l2`, vector `[1.0, −2.5, 0.0, 3.75]`.

Constants of those vectors:

| Constant | Value |
|---|---|
| `derived_id` | `bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` |
| `input_hash` | `0c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5d` |
| `output_hash` | `a23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` |
| `vector_value` | `010000000004013f800000c02000000000000040700000` |

They are listed as `` `derived_id` ``, `` `input_hash` ``, `` `output_hash` `` and `` `vector_value` ``; a test (`t01_hashes_of_the_document`) recomputes them.

| Nombre | Tipo | Bytes (hex) | Result |
|---|---|---|---|
| `key_desc` | key | `60bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | ok |
| `key_vec` | key | `61bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | ok |
| `key_src` | key | `620102030405060708090a0b0c0d0e0f100000000000000002bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | ok |
| `desc_ready` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `desc_stale` | desc | `010001020102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `desc_superseded` | desc | `010001030102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `desc_no_params` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61000c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | ok |
| `vec_dim4` | vec | `010000000004013f800000c02000000000000040700000` | ok |
| `vec_dim1` | vec | `010000000001013f000000` | ok |
| `src_entry` | src | `0100` | ok |
| `mark_v2` | mark | `01000000000202` | ok |
| `changeset_derived` | changeset | `010000060a24181e4000000000000500000000000000000000000000000000` | ok |
| `key_dv3_short` | key | `60bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0f` | DV-3 |
| `key_dv3_src_short` | key | `620102030405060708090a0b0c0d0e0f100000000000000002bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0f` | DV-3 |
| `key_unit5_rev0` | key | `620102030405060708090a0b0c0d0e0f100000000000000000bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | UNIT-5 |
| `key_unit2_reserved` | key | `63bef60da8227e0e422fbb7ea34f4fe33d3ed462c39b8b4aafa822aaa3011f0fae` | UNIT-2 |
| `desc_unit6_env0` | desc | `000001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | UNIT-6 |
| `desc_unit7_env2` | desc | `020001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | UNIT-7 |
| `desc_unit8_flags` | desc | `010101010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | UNIT-8 |
| `desc_dv1_kind0` | desc | `010000010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-1 |
| `desc_dv1_kind2` | desc | `010002010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-1 |
| `desc_dv2_state0` | desc | `010001000102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-2 |
| `desc_dv2_state4` | desc | `010001040102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-2 |
| `desc_dv4_rev0` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000000000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_method_empty` | desc | `010001010102030405060708090a0b0c0d0e0f1000000000000000020000000000000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_method_65` | desc | `010001010102030405060708090a0b0c0d0e0f100000000000000002000000416d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d6d00000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_method_not_utf8` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005ff6d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv4_profile_empty` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d6265640000000300000000020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-4 |
| `desc_dv5_count17` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61110000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_key_empty` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000064696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_not_increasing` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d6102000000037a696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_repeated_key` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d00000001340000000364696d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_value_257` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d000001017676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676767676000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_dv5_count_255` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61ff0000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab4` | DV-5 |
| `desc_unit9_truncated` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6ca` | UNIT-9 |
| `desc_unit10_trailing` | desc | `010001010102030405060708090a0b0c0d0e0f10000000000000000200000005656d62656400000003000000076d6f64656c2d61020000000364696d0000000134000000046e6f726d000000026c320c32fa6504e19c611390f36dbed265860bb9e5a2533efd04691b6822383a7c5da23aca012016dcfbb9393cf0db33178f31d45dd0429582917f10bb8819e6cab400` | UNIT-10 |
| `vec_dv6_dim0` | vec | `010000000000013f800000c02000000000000040700000` | DV-6 |
| `vec_dv6_dim4097` | vec | `010000001001013f800000c02000000000000040700000` | DV-6 |
| `vec_dv6_length` | vec | `010000000004013f800000c020000000000000407000` | DV-6 |
| `vec_dv7_elem2` | vec | `010000000004023f800000c02000000000000040700000` | DV-7 |
| `vec_dv8_nan` | vec | `010000000004017fc00000c02000000000000040700000` | DV-8 |
| `vec_dv8_inf` | vec | `010000000004017f800000c02000000000000040700000` | DV-8 |
| `vec_dv8_negative_zero` | vec | `010000000004013f800000c02000008000000040700000` | DV-8 |
| `vec_unit9_short` | vec | `0100000000` | UNIT-9 |
| `vec_unit7_env2` | vec | `020000000004013f800000c02000000000000040700000` | UNIT-7 |
| `src_unit10_trailing` | src | `010000` | UNIT-10 |
| `src_unit8_flags` | src | `0101` | UNIT-8 |
| `src_unit9_empty` | src | `` | UNIT-9 |
| `mark_unit16_v4` | mark | `01000000000402` | UNIT-16 |
| `mark_v1_reader_unit16` | mark1 | `01000000000202` | UNIT-16 |
| `mark_unit15_v0` | mark | `01000000000002` | UNIT-15 |
| `changeset_v1_reader_unit13` | changeset1 | `010000060a24181e4000000000000500000000000000000000000000000000` | UNIT-13 |

## 9. History

| Date | Change |
|---|---|
| 2026-11-12 | E8-04: first version, from ADR-0020 §A–§D. |
