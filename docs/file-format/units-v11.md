# Unit layer v11: a payload the head carries is stored once

| Field | Value |
|---|---|
| Status | Introduced on the branch `perf/phase4`; not frozen |
| Layer version | `layer_version = 11`, `envelope_version = 1` |
| Related to | [units-v10](units-v10.md) (the cut this one supersedes; its `UNIT_HEAD` trailer is unchanged), [units-v1](units-v1.md) §3.3, §3.4 and §5.1 (`REV_META`, `REV_PAYLOAD` and the payload rules), [engine v1 format](v1.md) (frozen, no changes) |

"Must" and "must not" are normative. No key and no value encoding changes: version 11 changes
which records exist.

## 1. What version 11 of the layer is

- A cut, like versions 7, 9 and 10: a database is created at version 11, a build opens version
  11 only, and a mark below 11 is refused at open (`LayerTooOld`). There is no migration: export
  the database with the build that wrote it and import it into a build of version 11. The mark
  is written by the first change set of a database and never changes.
- A build that reads up to 10 refuses a mark of 11 with `IncompatibleLayer` (UNIT-16).

Why: in version 10 a payload of at most 2048 bytes was written twice, in the `UNIT_HEAD`
trailer and in the `REV_PAYLOAD` of the head revision. For a code index (symbols of a few
hundred bytes, text chunks of about 1.3 KB) that is most of the payload bytes, written twice to
the log, to the memtable and to the tables.

## 2. Where the payload of a revision is

- The head revision of a unit whose `UNIT_HEAD` carries a trailer (units-v10 §2) has **no**
  `REV_PAYLOAD`: the trailer is its payload. Its `REV_META` is unchanged (`payload_present = 1`,
  `payload_len`, `content_hash` of that payload).
- Every other revision whose content is kept has its `REV_PAYLOAD`, as before: the head revision
  of a unit whose head carries no trailer (a payload over 2048 bytes), and every revision below
  the head that the history policy keeps.
- So a payload is in exactly one place. A head with a trailer **and** a `REV_PAYLOAD` of its head
  revision is UNIT-31.

## 3. How a writer keeps it

- A write of a head (create, update, archive, unarchive, restore, delete, import) sets the
  trailer as in units-v10 §3, and writes the `REV_PAYLOAD` of the new head revision only when the
  head does not carry its payload.
- When a revision is superseded and the history keeps its content (`Versioned`), the writer of
  the new revision writes the outgoing revision's `REV_PAYLOAD` in the same change set, from the
  payload the old head carried, when the old head carried it. `Audit` drops that payload and
  `None` drops the revision, as before, so nothing is written for them.
- Import writes the head revision of each unit like a writer does: its `REV_PAYLOAD` only when
  the payload is over 2048 bytes.
- A head without a trailer over a payload of at most 2048 bytes, whose head revision has its
  `REV_PAYLOAD`, is valid (a writer of this layer never makes one; readers take the
  `REV_PAYLOAD`).

## 4. How a reader uses it

- `get_unit` and every lookup that returns units: as in units-v10 §4, one point read for a head
  with a trailer.
- A read of a revision (history, `read_as_of`, compare, restore, export) reads its
  `REV_PAYLOAD`; when there is none and the revision is the head revision of a head with a
  trailer, the payload is the trailer. It is checked against the revision's `REV_META` (UNIT-26
  to UNIT-28) either way.
- A checked read (`set_checked_reads(true)`) and every write to a unit read the head revision in
  full: the head must mirror its `REV_META` (IU-2), the payload (the trailer, or else the
  `REV_PAYLOAD`) must match the `REV_META`, and a `REV_PAYLOAD` beside a trailer is refused as
  corruption.

## 5. What `verify_units` checks

- The trailer of every head against the head's `content_hash` (UNIT-28), as in units-v10.
- A head with a trailer: its head revision must declare a payload (UNIT-31) and must have no
  `REV_PAYLOAD` (UNIT-31). The trailer is then the payload the full-text, trigram and schema
  index checks compare against.
- Every other `REV_META` that declares a payload needs its `REV_PAYLOAD` (UNIT-26), as before.
