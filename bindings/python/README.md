# kerno

Python bindings for [Kerno](https://github.com/Rixmerz/kernodb): an embedded database of
**units** (typed records), the **relations** between them, and queries over both. It runs
inside your process and keeps its data in a directory on disk; there is no server.

> Status: alpha, not yet published on PyPI. Build it from source as shown below.

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
| `db.close()` | `None`; closing twice is harmless |

Errors: engine failures (invalid unit type, unknown unit in `relate`, a directory already held
by another handle) raise `kerno.KernoError`; a malformed id or direction raises `ValueError`;
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
