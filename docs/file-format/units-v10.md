# Unit layer v10: the head carries the payload of its revision

| Field | Value |
|---|---|
| Status | Introduced on the branch `perf/cold-reads`; not frozen |
| Layer version | `layer_version = 10`, `envelope_version = 1` |
| Related to | [units-v1](units-v1.md) §3.2 (`UNIT_HEAD`, extended here), §3.3, §3.4 and §5.1 (`REV_META`, `REV_PAYLOAD` and the payload rules, unchanged), [units-v9](units-v9.md) (the cut this one supersedes), [engine v1 format](v1.md) (frozen, no changes) |

"Must" and "must not" are normative. The records are ordinary keys and values of the engine: no
byte of the engine's v1 format changes.

## 1. What version 10 of the layer is

- A cut, like versions 7 and 9: a database is created at version 10, a build opens version 10
  only, and a mark below 10 is refused at open (`LayerTooOld`). There is no migration: export the
  database with the build that wrote it and import it. The mark is written by the first change
  set of a database and never changes.
- Version 10 includes every record family of versions 1 to 9. One record changes: a `UNIT_HEAD`
  (`0x10`) may end with a copy of the payload of its head revision. No key changes, and no other
  value does.
- A build that reads up to 9 refuses a mark of 10 with `IncompatibleLayer` (UNIT-16).

Why: reading a unit was three point reads (`UNIT_HEAD`, then the `REV_META` and the `REV_PAYLOAD`
of the revision the head names), in three different places of the key space, so three blocks of
a table for a unit that is not in memory. With the payload in the head it is one.

## 2. `UNIT_HEAD` (`0x10`)

The key is unchanged: `0x10 ‖ unit[16]`. The value is the value of [units-v1](units-v1.md) §3.2,
optionally followed by a trailer:

| Part | Bytes |
|---|---|
| head | the fields of units-v1 §3.2, `envelope_version` to `metadata`, unchanged |
| trailer (optional) | `0x01 ‖ payload[payload_len]` |

- A value that ends after `metadata` has no trailer. It is exactly a layer 1 `UNIT_HEAD`, and it
  says nothing about the head revision: its payload, if it has one, is in its `REV_PAYLOAD`.
- A trailer is the byte `0x01` and then exactly `payload_len` bytes (the head's own field): the
  payload of the head revision, byte for byte what its `REV_PAYLOAD` holds.
- With a trailer, `payload_len` must be at most 2048 (`INLINE_PAYLOAD_MAX`), otherwise UNIT-11;
  the `lifecycle` must not be `Deleted` (a delete revision has no payload), otherwise UNIT-12.
- Fewer than `payload_len` bytes after the `0x01` is UNIT-9. Bytes after the payload, or bytes
  after `metadata` that do not start with `0x01`, are UNIT-10, as trailing bytes always were.
- The hash of `payload_codec ‖ payload` must be the head's `content_hash` (UNIT-28, the rule of
  units-v1 §5.1 applied to the copy). The length cannot differ: it is `payload_len` by
  construction.

The `REV_META` and `REV_PAYLOAD` records of every retained revision, the head revision included,
are written as before. The trailer is a copy, never the only place a payload is stored: history,
`read_as_of`, restore, retention and export read the revision records and do not look at it.

Vectors (pinned by `the_head_codec_roundtrips_and_bounds_the_inline_payload` in
`crates/kerno-units/tests/inline_payload.rs`), for a head `H` of an `Active` unit with
`payload_len = 3`:

| Value | Result |
|---|---|
| `H` | accepted, no payload in the head |
| `H ‖ 01 ‖ 61 62 63` | accepted, payload `abc` |
| `H ‖ 01 ‖ 61 62` | UNIT-9 |
| `H ‖ 01 ‖ 61 62 63 ‖ 00` | UNIT-10 |
| `H ‖ 02 ‖ 61 62 63` | UNIT-10 |
| `H ‖ 01 ‖ …` with `payload_len = 2049` | UNIT-11 |
| `H ‖ 01` with `lifecycle = Deleted`, `payload_len = 0` | UNIT-12 |

## 3. How a writer keeps it

- Every write of a head (create, update, archive, unarchive, restore, delete, import) sets the
  trailer from the payload of the revision the head now names: present when that revision has a
  payload of at most 2048 bytes, absent otherwise. A head never carries the payload of an older
  revision.
- Retention rewrites a head only to move `oldest_rev`; it keeps the trailer as it found it. It
  drops payloads of revisions below the head only, so it never has to remove one.
- A writer must not rely on the trailer being there: a head without one is valid whatever the
  size of the payload.

## 4. How a reader uses it

- A read of a unit (`get_unit`, and every lookup that returns units) reads the head. With a
  trailer, the payload is the trailer's, checked against the head's `content_hash` (UNIT-28), and
  the read is complete: one point read. Without one, the read is the read of version 9: the
  `REV_META` of the head revision, which the head must mirror (IU-2), and its `REV_PAYLOAD`,
  checked against that `REV_META` (UNIT-26 to UNIT-28).
- What the one-read path does not check is what it does not read: that the head mirrors the
  `REV_META` of its head revision (IU-2, UNIT-31), and that the `REV_PAYLOAD` of that revision
  exists and hashes to the same value. Both are still checked:
  - by `verify_units`, for every unit (§5);
  - by every write to the unit (update, archive, unarchive, restore, delete), which reads the head
    revision in full before it builds on it, and also requires the trailer to be the bytes of the
    `REV_PAYLOAD`;
  - by `history`, `read_as_of` and restore, which read the revision records themselves.
- Length and presence are checked on every read; the content hash is checked when
  `verify_payload_on_read` is on, and always by `verify()`, history, export/import and the reads a
  write builds on; a damaged block is always caught by the block CRC. The block checksum
  of the engine covers the bytes of a block against damage at rest; the content hash also ties
  the payload to the head that declares it, whatever wrote the two, and it is what `verify`
  compares across the head, the `REV_META` and the `REV_PAYLOAD`. For a payload that fits in a
  head it costs a fraction of the read.

## 5. What `verify_units` checks

- The trailer of every head against the head's own `content_hash` (UNIT-28), reported at the
  key of the head.
- IU-2 as before (UNIT-31): the head against the `REV_META` of its head revision, field by
  field, and the `REV_PAYLOAD` of that revision against the `REV_META` (UNIT-26 to UNIT-28). The
  three hashes being equal, the trailer and the `REV_PAYLOAD` are the same bytes.
- A head with a trailer whose head revision declares no payload, or has no `REV_PAYLOAD`, is
  UNIT-31.
- A head without a trailer is never a finding.
