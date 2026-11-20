# kerno

Python bindings for [Kerno](https://github.com/Rixmerz/kernodb): an embedded database of
**units** (typed records), the **relations** between them, and queries over both. It runs
inside your process and keeps its data in a directory on disk; there is no server.

> Status: alpha. `1.0.0a1` is on PyPI (`pip install --pre kernodb`); the calls marked *new* below need a build from source.

## Five-minute quickstart

Install (from a checkout of the repository, with a Rust toolchain):

```bash
cd bindings/python
python3 -m venv .venv && . .venv/bin/activate
pip install maturin
maturin develop --release
```

Open a database, store two units, relate them, read and query:

```python
import kerno

with kerno.open("./memory-db") as db:                # creates the directory if needed
    note = db.store("note", "Rust has no GC")        # (unit_type, payload) -> id
    topic = db.store("topic", b"memory")             # str is stored as UTF-8, bytes as they are

    unit = db.read(note)                             # Unit, or None for an unknown id
    print(unit.unit_type, unit.payload)              # note b'Rust has no GC'

    db.relate(note, topic, "about")                  # (from, to, relation_type) -> relation id
    for edge in db.related(note):                    # outgoing by default
        print(edge.from_id, "->", edge.to_id, edge.relation_type)
    print(len(db.related(topic, direction="in")))    # 1: who points at the topic

    print([u.id for u in db.query("note")])          # every active unit of a type
```

Leaving the `with` block closes the database; without it, call `db.close()`. Data written
before a close is there the next time you `kerno.open` the same path.

## API

| Call | Returns |
|------|---------|
| `kerno.open(path)` | a `Database`; `path` is a `str` or `os.PathLike` |
| `db.store(unit_type, payload)` | the new unit's id (32 hex digits) |
| `db.read(id)` | `Unit` (`id`, `unit_type`, `schema_version`, `lifecycle`, `revision`, `payload`) or `None` |
| `db.relate(from_id, to_id, relation_type)` | the relation's id |
| `db.related(id, direction="out", relation_type=None)` | `list[Relation]` (`id`, `from_id`, `to_id`, `relation_type`) |
| `db.query(unit_type=None, schema_version=None)` | `list[Unit]`, every active unit that matches, in id order |
| `kerno.open(path, read_only=False)` | read-only opens write nothing, but still hold the directory's lock |
| `db.update(id, payload, if_changed=False)` | `True` when a revision was written; `if_changed` skips an identical payload |
| `db.delete(id, cascade=False)` | a tombstone revision; `cascade` removes the unit's relations in the same commit |
| `db.relate(from, to, type, unique=False)` | `unique` reuses an existing relation of that type; self-loops and parallel relations are allowed otherwise |
| `db.unrelate(relation_id)` | `None` |
| `db.commit(ops, reason=None, actor=None, source=None, detailed=False)` | one change set; see below |
| `db.history(id, limit=100, with_payload=False)` | `list[Revision]`, newest first; `[]` for an unknown id |
| `db.read_as_of(id, commit_seq)` | the `Revision` in force at that commit (`None` before creation); a sequence past the last commit reads now |
| `db.relations(type=None)` | every live relation in one scan (use it instead of `related` per unit) |
| `db.last_commit_seq()` | the sequence of the last commit |
| `db.verify()` | `dict`: `clean`, `commit_seq`, counts, `findings`; storage damage is reported as `storage_error`, not raised |
| `db.compact()` | `{"before", "after"}` stored bytes; history keeps deleted payloads until retention drops them |
| `db.define_schema(type, fields)`, `db.create_index(type, field)` | typed fields (`bool`, `int`, `float`, `text`, `bytes`) with `required` / `unique` / `indexed` |
| `db.store_document(type, dict)`, `db.update_document(id, dict)`, `db.read_document(id)` | schema-validated documents |
| `db.find(type, field, value)` | units whose indexed field equals `value` |
| `db.execute(query)` | the query language (`FIND user AS u WHERE ...`): `{"aliases", "rows"}` |
| `db.close()` | `None`; closing twice is harmless |

Batch ops for `commit`: `("store", type, payload)`, `("update", id, payload)`,
`("update_if_changed", id, payload)`, `("delete", id)`, `("delete_cascade", id)`,
`("unrelate", rel_id)`, `("relate", from, to, type)`, `("relate_unique", from, to, type)`,
`("archive", id)`, `("unarchive", id)`, `("restore", id, revision)`. A relation end is a unit id
or the 0-based index of an earlier `"store"` op of the same batch. `reason`, `actor` and `source`
are recorded with every revision. One batch is about 10x faster than the same writes one by one.

Limits: a payload is at most 1 MiB (`PayloadTooLargeError`); a type name is 1 to 64 bytes.

Errors: engine failures raise `kerno.KernoError` or one of its subclasses: `NotFoundError`,
`DeletedError`, `LockedError` (another handle holds the directory), `CorruptionError`,
`InvalidArgumentError`, `PayloadTooLargeError`, `HasRelationsError`, `PrunedError`; a malformed id or direction raises `ValueError`;
using a closed database raises `ValueError`. Type stubs ship with the package (`py.typed`).

One process holds a database directory at a time. Calls release the GIL while they touch disk,
and a `Database` can be shared between threads.

## Durability

`kerno.open` asks for synchronous commits. That guarantee is verified on Linux (ext4, xfs) only;
on other platforms the bindings still open the database, without that verification.

## Development

```bash
cd bindings/python
python3 -m venv .venv && . .venv/bin/activate
pip install maturin pytest
maturin develop
pytest
```

The crate is excluded from the cargo workspace (it carries its own `Cargo.lock` and is built by
maturin); check it on its own with `cargo clippy --all-targets -- -D warnings`.
