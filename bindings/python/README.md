# kernodb (Python)

Python bindings for [Kerno](https://github.com/Rixmerz/kernodb): an embedded, local-first
database of **units** (typed records), their **revisions** (every change is kept), the
**relations** between them, and queries over all three. It runs inside your process and keeps
its data in a directory on disk. There is no server.

The package is `kernodb`; the module you import is `kerno`.

> **Alpha.** The on-disk format is frozen, the API is not. Expect breaking changes between
> alpha releases. See the [changelog](https://github.com/Rixmerz/kernodb/blob/main/CHANGELOG.md).

## Install

```bash
pip install --pre kernodb        # --pre: only alpha versions exist so far
```

Wheels are published for CPython 3.9+ (one abi3 wheel per platform) on Linux x86_64 and
aarch64 (glibc; musl from the next release), macOS x86_64 and arm64, and Windows x86_64.
On any other platform pip builds from the source distribution, which needs a Rust toolchain.

Durable commits are verified on Linux (ext4, xfs) only; elsewhere the database works but that
guarantee is not verified (see [Durability](#durability)).

## Quickstart

```python
import kerno

with kerno.open("./memory-db") as db:                 # creates the directory if needed
    note = db.store("note", "Rust has no GC")         # (unit_type, payload) -> id
    topic = db.store("topic", b"memory")              # str is stored as UTF-8, bytes as they are

    db.relate(note, topic, "about")                   # (from, to, relation_type) -> relation id
    print([r.to_id for r in db.related(note)])        # outgoing relations

    db.update(note, "Rust has no garbage collector")  # a new revision; the old one is kept
    for rev in db.history(note, with_payload=True):   # newest first
        print(rev.revision, rev.kind, rev.payload)

    print([u.id for u in db.query("note")])           # every active unit of a type
```

Leaving the `with` block closes the database; without it, call `db.close()`.

## Concepts

- **Unit**: a record with an id (32 hex digits), a `unit_type` (1–64 bytes), a payload (up to
  1 MiB of bytes) and a lifecycle: `active`, `archived` or `deleted`.
- **Revision**: every write to a unit (create, update, archive, unarchive, delete, restore) adds
  one. Nothing is overwritten. A delete leaves a tombstone, and the history stays readable.
- **Relation**: a directed, typed edge between two units. Parallel relations and self-loops
  are allowed unless you ask for `unique=True`.
- **Change set**: one atomic commit. `commit(ops)` puts many writes in one change set (at most
  10,000 operations). A reader sees all of them or none, and a crash keeps all or none.
- **Commit sequence**: every change set has a number that only grows. `read_as_of(id, seq)`
  reads a unit as it was right after commit `seq`. `last_commit_seq()` is "now".

## Common tasks

### Write many things at once (and fast)

```python
out = db.commit(
    [
        ("store", "symbol", payload_a),        # op 0
        ("store", "symbol", payload_b),        # op 1
        ("relate", 0, 1, "calls"),             # an end can be an earlier "store" of this batch
        ("relate", existing_id, 0, "calls"),   # ... or an existing unit id
        ("update_if_changed", other_id, new),  # writes nothing if the payload is the same
        ("delete_cascade", old_id),            # removes its relations too
    ],
    reason="index run 42", actor="indexer",    # recorded with every revision
    detailed=True,
)
out["units"], out["relations"], out["unchanged"], out["commit_seq"]
```

All batch ops: `("store", type, payload)`, `("update", id, payload)`,
`("update_if_changed", id, payload)`, `("delete", id)`, `("delete_cascade", id)`,
`("unrelate", relation_id)`, `("relate", from, to, type)`, `("relate_unique", from, to, type)`,
`("archive", id)`, `("unarchive", id)`, `("restore", id, revision)`.

### Look things up by a field

```python
db.define_schema("user", [
    {"name": "email", "type": "text", "required": True, "unique": True},  # unique implies indexed
    {"name": "age", "type": "int", "indexed": True},
])
uid = db.store_document("user", {"email": "a@example.com", "age": 31})
db.find("user", "email", "a@example.com")      # -> [Unit]
db.read_document(uid)                          # -> {"email": ..., "age": 31}
db.execute("FIND user AS u WHERE u.age > 30")  # the query language: {"aliases", "rows"}
```

Field types: `bool`, `int`, `float`, `text`, `bytes`. A type has one schema, defined once.
Indexes can be added later with `create_index`. The query language is described in
[docs/query-language.md](https://github.com/Rixmerz/kernodb/blob/main/docs/query-language.md).

### Read the past

```python
seq = db.last_commit_seq()
db.update_document(uid, {"email": "a@example.com", "age": 32})
db.read_as_of(uid, seq).payload   # the payload before the update (raw document bytes)
db.history(uid, limit=10)         # newest first; [] for an unknown id
```

### Check integrity

```python
report = db.verify()
report["clean"]       # False if anything is wrong
report["findings"]    # what is wrong (ids and counts, never content)
```

Damage in the storage files (a block that fails its checksum) is reported as
`report["storage_error"]`. Reads that touch a damaged block raise `kerno.CorruptionError` and
never return wrong data.

## API reference

| Call | Returns |
|------|---------|
| `kerno.open(path, read_only=False)` | a `Database`. A read-only open writes nothing but still takes the directory's lock |
| `db.store(unit_type, payload)` | the new unit's id |
| `db.read(id)` | `Unit` (`id`, `unit_type`, `schema_version`, `lifecycle`, `revision`, `payload`) or `None` |
| `db.update(id, payload, if_changed=False)` | `True` when a revision was written |
| `db.delete(id, cascade=False)` | `None`. Without `cascade`, a unit with relations raises `HasRelationsError` |
| `db.relate(from, to, type, unique=False)` | the relation id. `unique` reuses an existing one |
| `db.unrelate(relation_id)` | `None` |
| `db.related(id, direction="out", relation_type=None)` | `list[Relation]` (`id`, `from_id`, `to_id`, `relation_type`) |
| `db.relations(relation_type=None)` | every live relation, in one scan. Use it instead of `related` per unit |
| `db.query(unit_type=None, schema_version=None)` | `list[Unit]`, every active unit that matches, in id order |
| `db.commit(ops, reason=None, actor=None, source=None, detailed=False)` | stored ids, or a dict with `detailed=True` |
| `db.history(id, limit=100, with_payload=False)` | `list[Revision]` (`revision`, `kind`, `commit_seq`, `created_at`, `lifecycle`, `content_hash`, `payload`) |
| `db.read_as_of(id, commit_seq)` | the `Revision` in force then, or `None` before creation. A sequence past the last commit reads now |
| `db.last_commit_seq()` | the sequence of the last commit |
| `db.define_schema(type, fields)` / `db.create_index(type, field)` | `None` |
| `db.store_document(type, dict)` / `db.update_document(id, dict)` / `db.read_document(id)` | id / `None` / `dict` or `None` |
| `db.find(type, field, value)` | `list[Unit]` whose indexed field equals `value` |
| `db.execute(query)` | `{"aliases": [...], "rows": [[Unit, ...], ...]}` |
| `db.verify()` | `dict`: `clean`, `commit_seq`, `units`, `revisions`, `relations`, `findings`, maybe `storage_error` |
| `db.compact()` | `{"before", "after"}` stored bytes |
| `db.close()` | `None`. Closing twice is harmless |

Type stubs ship with the package (`py.typed`).

## Errors

Engine failures raise `kerno.KernoError` or one of its subclasses:

| Exception | When |
|---|---|
| `NotFoundError` | no unit, relation or revision with that id |
| `DeletedError` | the unit is deleted (or not in the lifecycle the call needs) |
| `LockedError` | another handle or process holds the directory |
| `CorruptionError` | stored bytes failed a checksum or a structure rule |
| `InvalidArgumentError` | a bad type name, a batch over 10,000 ops, an op that names no earlier store, ... |
| `PayloadTooLargeError` | a payload over 1 MiB |
| `HasRelationsError` | deleting a unit that still has relations, without `cascade` |
| `PrunedError` | retention removed the revision asked for |
| `SchemaError` | the type's schema refused a document, or a typed unit got a payload that is not a document |

A malformed id or direction, or a call on a closed database, raises `ValueError`.

## Performance

These numbers are from one Linux machine (ext4), with every commit synced to disk, comparing Kerno 1.0.0a1 to SQLite in
WAL mode with `synchronous=FULL` storing the same data (with a revision table). The full
method is in
[docs/testing/python-vs-sqlite.md](https://github.com/Rixmerz/kernodb/blob/main/docs/testing/python-vs-sqlite.md).

| Workload | Kerno vs SQLite |
|---|---|
| one write per commit | 1.1–1.3x slower |
| point read by id, history of a unit | 1.2–1.5x slower |
| writes in one batch | 2.8–6x slower |
| relations in one batch | 6–11x slower |
| disk | 1.7–2.4x larger |

What makes the most difference:

1. **Batch your writes.** One `commit([...])` is about 10x faster than the same writes one by
   one, because each commit syncs to disk.
2. **Store payloads that are not text as bytes that are not UTF-8.** Every payload that is valid UTF-8
   text is indexed for keyword search (BM25), automatically. For structured data nobody
   searches by keyword (JSON you only read back), that index made writes about 7x slower and
   the store about 3.5x larger in our tests. Compressing the payload (`zlib.compress`, which
   never produces valid UTF-8) or any binary encoding skips it.
3. **Read relations in bulk** with `relations(type)` instead of `related(id)` per unit.
4. **Keep a batch under 10,000 operations.** Split larger work into several commits.

## Durability

`kerno.open` asks for synchronous commits: when a commit returns, it is on stable storage.
That guarantee is verified on Linux (ext4, xfs) only. On other platforms the database opens
and works without that verification. In our tests, killing the process with `kill -9` in the
middle of a stream of commits never lost an acknowledged commit and never left half a batch.

## Concurrency

One process holds a database directory at a time (a second open raises `LockedError`, also for
`read_only=True`). Inside the process, a `Database` can be shared between threads. Calls release
the GIL while they touch disk, any number of readers run while one writer commits, and a read
never sees half of a change set.

## Limits

- Payload: 1 MiB. Unit and relation type: 1–64 bytes (no NUL in a relation type).
- One change set: 10,000 operations.
- Deleting a unit keeps its revisions (and their payloads) until a retention policy drops them,
  so deletes do not shrink the store. `compact()` reclaims only what retention removed.

## Development

```bash
cd bindings/python
python3 -m venv .venv && . .venv/bin/activate
pip install maturin pytest
maturin develop --release
pytest
```

The crate is excluded from the cargo workspace (it carries its own `Cargo.lock` and is built by
maturin). Check it on its own with `cargo clippy --all-targets -- -D warnings`.
