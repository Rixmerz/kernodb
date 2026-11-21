# kernodb (Python)

Python bindings for [Kerno](https://github.com/Rixmerz/kernodb): an embedded, local-first
database of **units** (typed records), their **revisions** (every change is kept), the
**relations** between them, and queries over all three. It runs inside your process and keeps
its data in a directory on disk. There is no server.

The package is `kernodb`; the module you import is `kerno`.

> **Alpha (1.0.0a2).** It does not open databases written by 1.0.0a1 (see
> [Upgrading](#upgrading-from-100-alpha1)), and the keyword index is opt-in per type. The on-disk
> format is frozen, the API is not. Expect breaking changes between alpha releases. See the [changelog](https://github.com/Rixmerz/kernodb/blob/main/CHANGELOG.md).

Guides: [Kerno as a revisioned history next to another database](https://github.com/Rixmerz/kernodb/blob/main/docs/guides/history-alongside.md),
[one unit per entity, keyed by a logical key](https://github.com/Rixmerz/kernodb/blob/main/docs/guides/one-unit-per-entity.md),
and the [coverage of the `kerno` facade in the Python and Node bindings](https://github.com/Rixmerz/kernodb/blob/main/docs/bindings-coverage.md).

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

- **Keyword index**: off by default; `set_fulltext(type, True)` turns it on for one type.
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

Range lookups and query plans work on the same indexed fields:

```python
db.find_range("user", "age", 18, 65)           # both ends included; None leaves an end open
db.find_range("user", "age", low=65)           # age >= 65, ordered by age; `limit=` caps it
db.explain("FIND user AS u WHERE u.age > 30")  # how `execute` would run it, without running it
# {"candidates": [{"access": "type_scan", ...}, {"access": "field_index", "field": "age", "cost": ...}],
#  "chosen": 1, "joins": [], ...}
```

### Search by keyword, vector and relations

```python
db.set_fulltext("note", True)                   # keyword (BM25) search finds only these types
hits = db.hybrid_search(
    "garbage collector",                        # keyword signal
    vector=query_vec, vectors={uid: vec, ...},  # vector signal: your vectors, searched exactly
    seeds=[topic_id],                           # relation signal: units near these seeds
)
hits[0].id, hits[0].score                       # best first; score = sum of the signal scores
hits[0].keyword_score, hits[0].vector_score, hits[0].relation_score   # None: the signal missed it
```

Kerno does not store vectors: you pass the `{unit id: vector}` dict you keep in step with your
units, as in the Rust API. Signals are fused by weighted reciprocal rank fusion; each adds
`weight / (rrf_k + rank)`. Only active units are hits. Types you did not enable with
`set_fulltext` never match the keyword signal.

### Read the past

```python
seq = db.last_commit_seq()
db.update_document(uid, {"email": "a@example.com", "age": 32})
db.read_as_of(uid, seq).payload   # the payload before the update (raw document bytes)
db.history(uid, limit=10)         # newest first; [] for an unknown id

# Page through a long history: [from_revision, to_revision), newest first or order="asc"
page = db.history(uid, limit=50)
older = db.history(uid, limit=50, to_revision=page[-1].revision)

diff = db.compare(uid, 1, 3)      # both revisions with payloads: diff.a, diff.b, diff.same_content
db.read(uid).oldest_revision      # the oldest revision still kept; asking for an older one
                                  # raises PrunedError
```

Archived and deleted units stay out of `query()` unless you ask: `db.query("note",
lifecycle="archived")` (or `"deleted"`, or `"all"`). A deleted unit comes back with
`payload=None`; read its history to see what it held. `find` and `find_range` see only active
units.

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
| `kerno.open(path, read_only=False, max_batch_ops=None)` | a `Database`. A read-only open writes nothing and takes no lock, so it runs beside a writer (see `refresh`). `max_batch_ops` changes the 10,000-operation limit of one `commit` |
| `db.store(unit_type, payload)` | the new unit's id |
| `db.read(id)` | `Unit` (`id`, `unit_type`, `schema_version`, `lifecycle`, `revision`, `oldest_revision`, `payload`) or `None`. `oldest_revision` is the oldest revision retention still keeps |
| `db.update(id, payload, if_changed=False)` | `True` when a revision was written |
| `db.delete(id, cascade=False)` | `None`. Without `cascade`, a unit with relations raises `HasRelationsError` |
| `db.relate(from, to, type, unique=False)` | the relation id. `unique` reuses an existing one |
| `db.unrelate(relation_id)` | `None` |
| `db.related(id, direction="out", relation_type=None)` | `list[Relation]` (`id`, `from_id`, `to_id`, `relation_type`) |
| `db.relations(relation_type=None)` | every live relation, in one scan. Use it instead of `related` per unit |
| `db.query(unit_type=None, schema_version=None, lifecycle="active")` | `list[Unit]` that match, in id order. `lifecycle` is `"active"`, `"archived"`, `"deleted"` or `"all"`; the last three read every unit's head, so they cost time linear in the store |
| `db.commit(ops, reason=None, actor=None, source=None, detailed=False)` | stored ids, or a dict with `detailed=True` |
| `db.history(id, limit=100, with_payload=False, from_revision=None, to_revision=None, order="desc")` | `list[Revision]` (`revision`, `kind`, `commit_seq`, `created_at`, `lifecycle`, `schema_version`, `content_hash`, `payload`) in `[from_revision, to_revision)`, at most `limit`. A `from_revision` below `oldest_revision` raises `PrunedError` |
| `db.compare(id, a, b)` | `Comparison` (`a`, `b`: the two `Revision`s with payloads; `same_content`). Takes the unit id, as the Rust call does |
| `db.read_as_of(id, commit_seq)` | the `Revision` in force then, or `None` before creation. A sequence past the last commit reads now |
| `db.set_fulltext(type, enabled)` | `None`. Keyword-index a type (off by default); its active units follow in the same commit |
| `db.fulltext(type)` | `bool`: whether the type is keyword-indexed |
| `db.last_commit_seq()` | the sequence of the last commit |
| `db.refresh()` | on a `read_only=True` database: pick up the commits a writer made since the open; returns the sequence now read |
| `db.define_schema(type, fields)` / `db.create_index(type, field)` | `None` |
| `db.store_document(type, dict)` / `db.update_document(id, dict)` / `db.read_document(id)` | id / `None` / `dict` or `None` |
| `db.find(type, field, value)` | `list[Unit]` whose indexed field equals `value` |
| `db.find_range(type, field, low=None, high=None, limit=None)` | `list[Unit]` whose indexed field is in `[low, high]` (both included, `None` = open end), ordered by value then id. No `limit` means no cap |
| `db.execute(query)` | `{"aliases": [...], "rows": [[Unit, ...], ...]}` |
| `db.explain(query)` | the plan `execute` would follow, without running it: `root_alias`, `root_type`, `candidates` (`access` `"type_scan"` or `"field_index"`, `field`, `cost`), `chosen` (an index into `candidates`), `joins`, `planner_version` |
| `db.hybrid_search(text=None, vector=None, vectors=None, seeds=None, *, keyword_weight=1, vector_weight=1, relation_weight=1, limit=10, candidates=100, rrf_k=60, max_depth=2, metric="cosine")` | `list[HybridHit]`, best first: `id`, `score`, `keyword_score`, `vector_score`, `relation_score`, `keyword_rank`, `vector_rank`, `relation_rank` (a signal that missed the unit is `None`). `vector` and `vectors` go together; `metric` is `"cosine"` or `"dot"` |
| `db.verify()` | `dict`: `clean`, `commit_seq`, `units`, `revisions`, `relations`, `findings`, maybe `storage_error` |
| `db.compact()` | `{"before", "after"}` stored bytes |
| `db.run_retention(default=None, by_type=None)` | prune old revisions; a rule is a dict with one of `keep_last` (newest N), `keep_for` (seconds) or `keep_since` (commit sequence); returns `{"units_retained", "revisions_pruned", "payloads_dropped", "bytes_logically_freed", "commit_seq"}`. Pruned revisions raise `PrunedError` in `read_as_of`, `history(from_revision=)` and `compare`. A bad rule raises `ValueError` and writes nothing; a `None` rule is no rule. The policy is not stored: pass it each time |
| `db.close()` | `None`. Closing twice is harmless |

Type stubs ship with the package (`py.typed`).

## Retention and the oldest revision

Retention (dropping old revisions) is not configurable from Python. The Rust `Config` that
`open` fills has no retention setting: a policy is applied by calling `run_retention` on the
units layer, and that call has no Python binding. A database pruned by another tool (the Rust
API) keeps working here. Before reading an old revision, compare it with
`db.read(id).oldest_revision`, or catch `PrunedError`. A unit that was never pruned has
`oldest_revision == 1`.

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

These numbers are from one Linux machine (ext4), with every commit synced to disk, comparing Kerno 1.0.0a2 to SQLite in
WAL mode with `synchronous=FULL` storing the same data (with a revision table). The full
method is in
[docs/testing/python-vs-sqlite.md](https://github.com/Rixmerz/kernodb/blob/main/docs/testing/python-vs-sqlite.md).

| Workload | Kerno vs SQLite |
|---|---|
| one write per commit | about the same |
| point read by id, history of a unit | 0.8–1.4x |
| writes in one batch | about 3x slower |
| relations in one batch | 6–11x slower |
| disk | about 2x smaller here; LZ4 blocks halve a real store (66 → 35 MB) |

What makes the most difference:

1. **Batch your writes.** One `commit([...])` is about 10x faster than the same writes one by
   one, because each commit syncs to disk.
2. **The keyword index is opt-in.** Since 1.0.0-alpha.2 no type is indexed for keyword search
   (BM25) unless you call `db.set_fulltext("my_type", True)`. Indexing costs writes several
   times over and about 3.5x the disk, so turn it on only for the types you search by text.
   The setting is stored in the database, and switching a type that already has units
   indexes (or clears) them in the same commit.
3. **Read relations in bulk** with `relations(type)` instead of `related(id)` per unit.
4. **Mind the batch limit.** A commit holds 10,000 operations by default. Split larger work, or open with `max_batch_ops=` when it must be atomic.

## Durability

`kerno.open` asks for synchronous commits: when a commit returns, it is on stable storage.
That guarantee is verified on Linux (ext4, xfs) only. On other platforms the database opens
and works without that verification. In our tests, killing the process with `kill -9` in the
middle of a stream of commits never lost an acknowledged commit and never left half a batch.

## Concurrency

One read-write `Database` holds a database directory at a time: a second read-write open raises
`LockedError`. A read-only open takes no lock, so readers run beside the writer, in other processes
or in the same one. Inside the process, a `Database` can be shared between threads. Calls release
the GIL while they touch disk, any number of readers run while one writer commits, and a read
never sees half of a change set.

## Limits

- Payload: 1 MiB. Unit and relation type: 1–64 bytes (no NUL in a relation type).
- One change set: 10,000 operations by default (`kerno.open(path, max_batch_ops=...)` changes it, up to 2^20); the engine also limits the bytes of one commit.
- `find` and `find_range` return active units only. The Rust API has no index of archived or
  deleted units, so `query(lifecycle=...)` reads every unit's head instead of looking one up.
- Deleting a unit keeps its revisions (and their payloads) until a retention policy drops them,
  so deletes do not shrink the store. `compact()` reclaims only what retention removed.

## Upgrading from 1.0.0-alpha.1

1.0.0-alpha.2 does not open a database written by alpha.1. Opening one fails with an error whose
message says the database's `layer_version` predates the format of this build and to export and
import it. Nothing in the directory changes. To carry it over, export it with the
alpha.1 command-line tool and import it with the alpha.2 one:

```sh
kerno old-db export old.kexp        # kerno CLI of 1.0.0-alpha.1
kerno import old.kexp new-db        # kerno CLI of 1.0.0-alpha.2
```

Then turn on the keyword index of every type you search by text (it is off by default in
alpha.2), and define again the schemas and indexes: an export holds neither.

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
