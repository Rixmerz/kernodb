# Unit layer v8: the trigram index of substring search

| Field | Value |
|---|---|
| Status | Introduced by [ADR-0025](../adr/0025-substring-index-and-sqlite-parity.md); not frozen |
| Layer version | `layer_version = 8`, `envelope_version = 1` |
| Related to | [units-v7](units-v7.md) (the precedent, not edited), [engine v1 format](v1.md) (frozen, no changes) |

> **Since ADR-0026 the layer is cut at 9** ([units-v9](units-v9.md)): a database starts at 9 and the
> mark is not raised by `set_substring`. §1 describes the mark of the build that introduced the
> trigram index; the record format of §2 to §5 is unchanged.

"Must" and "must not" are normative. As in the earlier versions, the records are ordinary keys
and values of the engine: no byte of the engine's v1 format changes.

## 1. What version 8 of the layer is

- A database is created at version 7, the cut version (ADR-0023). The transaction that first
  turns the `SUBSTRING` option on for a type writes the mark at 8; no writer lowers it, even
  when the option is turned off again.
- Version 8 includes every record family of versions 1 to 7. A build that reads up to 7 refuses
  a mark of 8 with `IncompatibleLayer` (UNIT-16) instead of misreading the index.
- A `0x7A` record, or a `SUBSTRING` bit, under a mark below 8 is a UNIT-52 finding.

## 2. Tags and option bit

- `0x7A` is the trigram block. `0x7B`–`0x7F` stay reserved (UNIT-2).
- In the type-options record (`0x77`, [units-v7](units-v7.md) §3), bit 1 is `SUBSTRING`: the
  type's payloads are in the trigram index. Bits 2–7 are reserved and must be 0. Bit 0
  (`FULLTEXT`) and bit 1 are independent.

## 3. What is indexed

The head payload of every `Active` unit of a type with `SUBSTRING`, text or not: its first
256 KiB (the keyword index's limit). ASCII letters are folded to lower case; every other byte is
kept. The grams of a payload are the three bytes that start at each of its positions, the payload
followed by two `0x00` bytes, so a payload of `n` bytes has `n` grams (not all distinct). A unit
with an empty payload has none.

## 4. The block record

One posting list per type and gram, cut in blocks:

| Part | Bytes |
|---|---|
| key of a bounded block | `0x7A ‖ type_len: u8 ‖ unit_type ‖ gram[3] ‖ 0x00 ‖ bound[16]` |
| key of the tail block | `0x7A ‖ type_len: u8 ‖ unit_type ‖ gram[3] ‖ 0x01` |
| value | `envelope_version: u8 = 1 ‖ reserved_flags: u8 = 0 ‖ unit[16] × n` |

- `1 <= n <= 128`, and the units are strictly ascending.
- The blocks of a list hold consecutive ranges: every unit of a block is above the `bound` of
  the block before it in key order, and at most its own `bound`. The tail block, the last one,
  has no bound. A list may have no tail block.
- The units of a type's blocks are exactly that type's `Active` units, each in the list of each
  of its grams and in no other (UNIT-52).
- A key of another shape is UNIT-3; a type that is not UTF-8 is UNIT-12; `envelope_version` 0
  is UNIT-6 and above 1 is UNIT-7; `reserved_flags` other than 0 is UNIT-8; a value with no unit
  or a partial one is UNIT-9; more than 128 units is UNIT-11; units not strictly ascending is
  UNIT-13; a block that holds a unit outside its range is UNIT-52.

Vectors (pinned by `verify_reports_an_entry_that_does_not_decode` and
`blocks_are_cut_at_their_limit_and_deleted_when_emptied` in
`crates/kerno-units/src/substring/tests.rs`):

| Key (hex) | Value (hex) | Result |
|---|---|---|
| `7a 01 73 616263 01` | `01 00 01…01` (one unit) | the tail block of gram `abc` of type `s` |
| `7a 01 73 616263 00 01…01` (16 bytes) | `01 00 01…01` | a bounded block |
| `7a 01 73 616263 00 01…01` (15 bytes) | any | UNIT-3 |
| `7a 01 73 616263 07` | any | UNIT-3 |
| any block key | `01 00` | UNIT-9 |
| any block key | `01 00 02…02 01…01` | UNIT-13 |

## 5. How a writer keeps it

- The block of a unit is the first key at or after `list ‖ 0x00 ‖ unit`.
- A change set keeps the gram changes of its operations and writes them at commit, each changed
  block once. A block over 128 units is cut from its low end into blocks of 128, each bounded by
  its last unit; the rest keeps the key it had. An emptied block is deleted.
- Turning the option off deletes the type's range `0x7A ‖ type_len ‖ unit_type`.
- Unit ids are UUIDv7, so a new unit goes to the tail block of each of its grams.
