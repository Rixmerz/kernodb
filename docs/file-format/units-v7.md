# Unit layer v7: per-type options (keyword-index opt-out)

| Field | Value |
|---|---|
| Status | **Proposed** with [ADR-0022](../adr/0022-per-type-fulltext-opt-out.md): experimental, the record may change before 1.0 |
| Layer version | `layer_version = 7`, `envelope_version = 1` |
| Related to | [units-v6](units-v6.md) (the precedent, not edited), [engine v1 format](v1.md) (frozen, no changes) |

"Must" and "must not" are normative. As in the earlier versions, the record is an ordinary key
and value of the engine: no byte of the engine's v1 format changes.

## 1. What version 7 of the layer is

The precedent of [units-v6.md](units-v6.md) §1 is followed:

- The `LAYER_META "layer"` mark carries `layer_version = 7` **if and only if** the database has
  or had a `0x77` record. A database with none keeps the mark it had (1 to 6), byte for byte.
- Version 7 is written **in the same transaction** as the first `0x77` record, and **is never
  lowered**. Version 7 includes everything in 6.
- A layer 1 to 6 reader rejects the whole database: `IncompatibleLayer` with rule UNIT-16 and
  `found_layer_version = 7`. A reader of this version rejects `layer_version > 7`.
- A database written before this layer is not migrated. Opening writes nothing (I-18).

## 2. Tag

`0x77` is assigned by ADR-0022. `0x7A`–`0x7F` stay reserved (UNIT-2).

## 3. The record

| Part | Bytes |
|---|---|
| key | `0x77 ‖ type_len: u8 ‖ unit_type` (`type_len` = 1..=64, UTF-8) |
| value | `envelope_version: u8 = 1 ‖ reserved_flags: u8 = 0 ‖ options: u8` |

- `options` bit 0 is `NO_FULLTEXT`: the type's payloads are not in the keyword index.
  Bits 1–7 are reserved and must be 0. `options` must not be 0 (the record is deleted instead).
- A key of another shape is UNIT-3; a type that is not UTF-8 is UNIT-12; `envelope_version` 0
  is UNIT-6 and above 1 is UNIT-7; `reserved_flags` other than 0 is UNIT-8; a value that is not
  3 bytes long is UNIT-9; a zero or unknown `options` byte is UNIT-12.

Vectors (pinned by `record_layout_is_pinned` in `crates/kerno/tests/batch_ext.rs`):

| Key (hex) | Value (hex) | Result |
|---|---|---|
| `77 03 73 79 6d` | `01 00 01` | type `sym`, `NO_FULLTEXT` |
| `77 03 73 79 6d` | `01 00 00` | UNIT-12 (no option set) |
| `77 03 73 79 6d` | `01 00 02` | UNIT-12 (unknown bit) |
| `77 03 73 79 6d` | `02 00 01` | UNIT-7 (newer envelope) |
| `77 04 73 79 6d` | `01 00 01` | UNIT-3 (length) |

## 4. Rules

- **TO-1.** A writer changes a type's options only while the type has no `Active` unit, as its
  change set sees it.
- **TO-2.** A writer keeps no keyword-index document (`0x75`) and no posting (`0x74`) for a unit
  whose type has `NO_FULLTEXT`.
- **TO-3.** `verify_units` does not expect units of a `NO_FULLTEXT` type in the keyword index
  (UNIT-49). An entry for one of them is a finding. So is a `0x77` record under a mark below 7.
