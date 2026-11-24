# Unit layer v9: posting blocks for keyword postings and adjacency entries

| Field | Value |
|---|---|
| Status | Introduced by [ADR-0026](../adr/0026-posting-blocks-for-adjacency-and-keyword-postings.md); not frozen |
| Layer version | `layer_version = 9`, `envelope_version = 1` |
| Related to | [units-v8](units-v8.md) (the trigram blocks, whose design this reuses, not edited), [units-v4](units-v4.md) (the count records, unchanged), [engine v1 format](v1.md) (frozen, no changes) |

"Must" and "must not" are normative. As in the earlier versions, the records are ordinary keys
and values of the engine: no byte of the engine's v1 format changes.

## 1. What version 9 of the layer is

- A cut, like [ADR-0023](../adr/0023-format-v2-cut-for-alpha2.md): a database is created at
  version 9, a build opens versions 9 only, and a mark below 9 is refused at open
  (`LayerTooOld`). There is no migration: export the database with the build that wrote it and
  import it. The mark is written by the first change set of a database and never changes, so the
  mark-raising rules of versions 2 to 8 no longer apply.
- Version 9 includes every record family of versions 1 to 8. Two families change shape:
  `0x74` (keyword postings) and `0x41` / `0x42` (adjacency). Both are posting blocks, the
  design of the trigram index ([units-v8](units-v8.md) §4).
- A build that reads up to 8 refuses a mark of 9 with `IncompatibleLayer` (UNIT-16).

## 2. The block design they share

A *list* is a run of entries, each a 16-byte id and a fixed-size value (none for the trigram
index), in ascending id order, kept under one key prefix, the *list key*. It is cut in blocks:

| Part | Bytes |
|---|---|
| key of a bounded block | `list key ‖ 0x00 ‖ bound[16]` |
| key of the tail block | `list key ‖ 0x01` |
| value | `envelope_version: u8 = 1 ‖ reserved_flags: u8 = 0 ‖ entry × n` |

- `1 <= n <= max` (128 postings, 64 adjacency entries), strictly ascending by id.
- The blocks of a list hold consecutive ranges: every id of a block is above the `bound` of the
  block before it in key order and at most its own `bound`. The tail block, the last one, has no
  bound. A list may have no tail block, and a block is never empty.
- The block of an id is the first key at or after `list key ‖ 0x00 ‖ id`.
- A change set keeps its changes per list and writes them at commit: each block it changes is
  read and written once. A block over `max` entries is cut from its low end into blocks of `max`,
  each bounded by its last id; the rest keeps the key it had. An emptied block is deleted. A list
  the writer knows to be empty (its count record, §4, is absent or says zero) is written without
  being read.
- Errors common to both families: `envelope_version` 0 is UNIT-6, above 1 is UNIT-7;
  `reserved_flags` other than 0 is UNIT-8; a value with no whole entry or a partial one is UNIT-9;
  more than `max` entries is UNIT-11; ids not strictly ascending is UNIT-13.

## 3. Keyword postings (`0x74`)

| Part | Bytes |
|---|---|
| list key | `0x74 ‖ term ‖ 0x00` (the term is 1..=64 bytes of UTF-8 with no NUL) |
| entry | `unit[16] ‖ tf: u32 BE ‖ dl: u32 BE` (24 bytes) |

- `tf >= 1` and `dl >= tf`; otherwise UNIT-12. The first NUL of the key after the tag ends the
  term: a key with none, or a marker that is neither `0x01` nor `0x00 ‖ bound[16]`, is UNIT-3; a
  term that is empty, over 64 bytes or not UTF-8 is UNIT-12.
- The `DOC` (`0x75`) and `STATS` (`0x76`) records are unchanged ([ADR-0022](../adr/0022-per-type-fulltext-opt-out.md)).
  `STATS` is written once per change set, with the sum of the changes of its documents.
- The blocks of a term hold exactly the postings of the `Active` units of the keyword-indexed
  types that have the term (UNIT-49), and each block holds only units in its range (UNIT-49).

Vectors (pinned by `verify_sees_block_values_that_do_not_decode` and
`a_batch_writes_one_record_per_block_not_one_per_posting` in
`crates/kerno-units/src/fulltext/tests.rs`):

| Key (hex) | Value (hex) | Result |
|---|---|---|
| `74 72617265 00 01` | `01 00` ‖ `01…01` ‖ `00000001` ‖ `00000001` | the tail block of the term `rare`, one posting |
| `74 72617265 00 00 01…01` (16 bytes) | the same | a bounded block |
| `74 72617265 00 07` | any | UNIT-3 |
| any posting key | `01 00` | UNIT-9 |
| any posting key | the one posting with `tf = 0` | UNIT-12 |

## 4. Adjacency (`0x41` out, `0x42` in)

| Part | Bytes |
|---|---|
| list key | `tag ‖ unit[16] ‖ relation_type ‖ 0x00` (the type is 1..=64 bytes of UTF-8 with no NUL) |
| entry | `edge[16] ‖ other[16]` (32 bytes): the relation id and the unit at the other end |

- The key of a block is 20 to 99 bytes. The first NUL after the unit ends the type: a key with
  none, or a marker that is neither `0x01` nor `0x00 ‖ bound[16]`, is UNIT-3; a type that is empty,
  over 64 bytes or not UTF-8 is UNIT-12 (a type with a NUL in it is therefore a UNIT-3 key).
- The range of a unit and direction is `[tag ‖ unit, tag ‖ unit + 1)`; the range of one type
  is the list key's range `[… ‖ 0x00, … ‖ 0x01)`. A read of a list is a scan of its few blocks.
- The `REL` record (`0x40`) of each relation is unchanged: one record per relation, written when
  it is created and removed when it is deleted. Each relation has exactly one entry in the
  out list of its `from` unit and one in the in list of its `to` unit, naming the other end, and
  the entry's list type is the relation's type (UNIT-36). A block that holds a relation outside its
  range is UNIT-36.
- The count records (`0x72`, [units-v4](units-v4.md)) are unchanged: one per `(unit, direction,
  type)` with at least one entry, equal to the number of entries of that list (UNIT-45). A list
  with no record is an empty list, or one whose record was lost: before it changes such a list
  the writer counts the entries of its blocks and records the total with the change, so it never
  writes a block over entries it did not read.

Vectors (pinned by `crates/kerno-units/tests/vectors/v1/structs`, `rel_out_*`, `rel_in_*` and
`neg_unit*_rel_*`):

| Vector | Result |
|---|---|
| `rel_out_one` (bounded block, one entry), `rel_out_tail_two`, `rel_out_block_64` (64 entries) | accepted |
| `neg_unit3_rel_out_key_19`, `neg_unit3_rel_out_key_without_nul`, `neg_unit3_rel_out_key_bad_marker`, `neg_unit3_rel_out_key_bound_15`, `neg_unit3_rel_in_key_type_with_nul`, `neg_unit3_rel_in_type_65_bounded` | UNIT-3 |
| `neg_unit12_rel_out_key_type_not_utf8`, `neg_unit12_rel_in_type_65_tail` | UNIT-12 |
| `neg_unit9_rel_out_cut`, `neg_unit9_rel_out_trailing`, `neg_unit9_rel_out_no_entry` | UNIT-9 |
| `neg_unit11_rel_out_65_entries` | UNIT-11 |
| `neg_unit13_rel_out_not_ascending`, `neg_unit13_rel_out_repeats_a_relation` | UNIT-13 |
| `neg_unit6_rel_in_version_0` | UNIT-6 |

## 5. How a writer keeps it

- Creating or deleting a relation writes or deletes its `REL` record and keeps the two entry
  changes; both lists' counts are read first, and a list whose count was zero is written without a
  read. The change set writes each changed block once at commit.
- A unit that has an entry in some list, or a change that adds one in the change set, is refused
  an archive or a delete (`UnitHasRelations`, as before).
- A write of a unit's text keeps its posting changes and the statistics delta, and writes them at
  commit; the unit's `DOC` record is written with the unit.
- Turning the keyword index off removes each of the type's units' postings through the same path.
