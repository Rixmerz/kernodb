# Unit layer v13: covering index entries

| Field | Value |
|---|---|
| Status | Introduced on the branch `perf/covering-index`; not frozen |
| Layer version | `layer_version = 13`, `envelope_version = 1` |
| Related to | [units-v12](units-v12.md) (the cut this layer builds on; unchanged), [units-v11](units-v11.md) (the inline payload rule reused here), [ADR-0021](../adr/0021-typed-schemas-and-secondary-indexes.md) (schemas and `FIELD_INDEX`) |

"Must" and "must not" are normative. Only the schema record (`0x78`) and the user index entries
(`0x79`) change, and only for a field declared `covering`.

## 1. What version 13 of the layer is

- **Not a cut.** A layer 12 database holds no covering field and no covering entry, so it is
  valid layer 13 data. A build of this layer opens a mark of 12 or 13; a database is still
  created at 12. The transaction that writes the first schema record with a covering field
  raises the mark to 13 in the same commit, never before and never lowered (the rule of layers 6
  and 8).
- A build that reads up to 12 refuses a mark of 13 with `IncompatibleLayer` (UNIT-16): it would
  write units without rewriting their covering entries.
- A mark of 11 or below is refused at open with `LayerTooOld`, as in units-v12.

## 2. Records

- `SCHEMA` `0x78`: field flags bit 3 is `covering`. It implies bit 2 (`indexed`); flags with bit 3
  and not bit 2, or above `0b1111`, are no writer's (UNIT-12).
- `FIELD_INDEX` `0x79`: the key is unchanged. The value is `envelope[2]` as before, or, for an
  entry of a covering field whose unit's head payload is at most `INLINE_PAYLOAD_MAX` (2048) bytes,
  `1 ‖ 0x01 ‖ payload`: envelope flag bit 0 announces the payload, which is the unit's head
  payload byte for byte. An entry of a covering field whose payload is longer is `envelope[2]`. An
  entry of a field that is not covering is byte for byte what layer 12 writes. A value `1 ‖ 0x01`
  followed by more than 2048 bytes, `1 ‖ 0` followed by any byte, or other flags is a structure finding
  (UNIT-8, UNIT-10).

## 3. Writes

- Entries exist only for `Active` units (ADR-0021), and that does not change.
- Every operation that writes a unit's head with a new payload (create, update, upsert, restore,
  unarchive, and the writes an import replays) rewrites every covering entry of that unit in the
  same transaction, also when the indexed value did not change. Archive and delete remove the
  entries as before.
- `define_schema` with a schema that differs from the stored one only in `covering` flags rewrites
  the entries of those fields for every `Active` unit in its change set (with the payload, or
  without it). `drop_index` clears `covering` with `indexed`. Any other difference is still
  `SchemaExists`.

## 4. Reads

`find_payloads_by_field` (`kerno::Db::find_payloads`, Python `find_documents`) answers `(unit id,
payload)` for one value, in unit id order. On a covering field, a hit whose entry carries the
payload is answered from the entry: no head is read and the entry is not checked against its unit
(that is `verify_units`). Other hits are read and checked as `find_by_field` does. When reads
verify (`verify_payload_on_read` is on, or checked reads are on, `set_checked_reads(true)`), no hit
is answered from its entry: every hit is read and checked as `find_by_field` does, so the payload
comes from the unit's head. `find` and `find_range` are unchanged.

## 5. What `verify_units` checks (UNIT-53)

An entry that matches its unit (UNIT-50) of a covering field must carry exactly the unit's head
payload when that payload is at most 2048 bytes, and nothing otherwise; an entry of a field that is
not covering must carry nothing; a schema with a covering field needs a mark of at least 13.

## 6. Export and import

The export format (export-v1, frozen) holds no schemas and no index entries, so there is nothing
of this layer to carry: as for every schema, define it again after an import, with `covering`.
