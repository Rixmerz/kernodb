# Unit layer v7: per-type options (keyword-index opt-out)

| Field | Value |
|---|---|
| Status | The cut version of [ADR-0023](../adr/0023-format-v2-cut-for-alpha2.md) (record introduced by [ADR-0022](../adr/0022-per-type-fulltext-opt-out.md)); frozen with 1.0.0-alpha.2 |
| Layer version | `layer_version = 7`, `envelope_version = 1` |
| Related to | [units-v6](units-v6.md) (the precedent, not edited), [engine v1 format](v1.md) (frozen, no changes) |

"Must" and "must not" are normative. As in the earlier versions, the record is an ordinary key
and value of the engine: no byte of the engine's v1 format changes.

## 1. What version 7 of the layer is

- Version 7 is the **cut version** (ADR-0023): every database is created at 7, and a reader
  refuses any mark below 7 with `LayerTooOld` (an alpha.1 database is exported with the build
  that wrote it and imported). A mark above 7 is `IncompatibleLayer` (UNIT-16).
- Version 7 includes every record family of versions 1 to 6.

## 2. Tag

`0x77` is assigned by ADR-0022. `0x7A`–`0x7F` stay reserved (UNIT-2).

## 3. The record

| Part | Bytes |
|---|---|
| key | `0x77 ‖ type_len: u8 ‖ unit_type` (`type_len` = 1..=64, UTF-8) |
| value | `envelope_version: u8 = 1 ‖ reserved_flags: u8 = 0 ‖ options: u8` |

- `options` bit 0 is `FULLTEXT`: the type's payloads are in the keyword index. A type
  without the bit (or without a record) is not indexed: the index is opt-in.
  Bits 1–7 are reserved and must be 0. `options` must not be 0 (the record is deleted instead).
- A key of another shape is UNIT-3; a type that is not UTF-8 is UNIT-12; `envelope_version` 0
  is UNIT-6 and above 1 is UNIT-7; `reserved_flags` other than 0 is UNIT-8; a value that is not
  3 bytes long is UNIT-9; a zero or unknown `options` byte is UNIT-12.

Vectors (pinned by `record_layout_is_pinned` in `crates/kerno/tests/batch_ext.rs`):

| Key (hex) | Value (hex) | Result |
|---|---|---|
| `77 04 6e 6f 74 65` | `01 00 01` | type `note`, `FULLTEXT` |
| `77 04 6e 6f 74 65` | `01 00 00` | UNIT-12 (no option set) |
| `77 04 6e 6f 74 65` | `01 00 02` | UNIT-12 (unknown bit) |
| `77 04 6e 6f 74 65` | `02 00 01` | UNIT-7 (newer envelope) |
| `77 05 6e 6f 74 65` | `01 00 01` | UNIT-3 (length) |

## 4. Rules

- **TO-1.** Changing a type's option re-plans the keyword index of every `Active` unit of the
  type in the same change set (indexing it, or removing its entries).
- **TO-2.** A writer keeps a keyword-index document (`0x75`) and postings (`0x74`) only for an
  `Active` unit whose type has `FULLTEXT`.
- **TO-3.** `verify_units` expects exactly the `Active` units of `FULLTEXT` types in the keyword
  index (UNIT-49). Any other entry is a finding.
