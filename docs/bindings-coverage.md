# Coverage of the `kerno` facade in the bindings

This table lists every public method of the `kerno` facade (`kerno::Db`, `crates/kerno/src/lib.rs`)
and the `Batch` operations, and says whether the Python binding (`bindings/python`, package
`kernodb`, module `kerno`) and the Node binding (`bindings/node`, package `kernodb`) expose each one,
and under which name. It is built from the source: the Rust names come from `crates/kerno/src/lib.rs`,
the Python names from the `#[pymethods]` of `bindings/python/src/lib.rs`, and the Node names from the
`#[napi]` methods of `bindings/node/src/lib.rs` (napi camelCases them; `bindings/node/index.d.ts` agrees).

The Python column was checked against an installed build (every name present or missing as shown). The
Node column comes from the source and `index.d.ts`; the Node addon was not built for this page.

Status: **Full** means the binding exposes the call with the facade's semantics. **Partial** means it
exposes only part of it. **Gap** means there is no way to call it from that binding.

## Lifecycle and open

| Rust (`kerno::Db`) | Python | Node | Status |
|---|---|---|---|
| `Db::open(path)` | `kerno.open(path)` | `open(path)` | Full |
| `Db::open_with(path, Config)` | `kerno.open(path, read_only=)` | `open(path, { readOnly })` | Partial: the bindings set the read-only mode and nothing else of `Config` |
| `Db::open_with_limits(path, Config, max)` | `kerno.open(path, max_batch_ops=)` | `open(path, { maxBatchOps })` | Partial: same as `open_with` |
| `Db::close` | `db.close()` (also `with kerno.open(...)`) | `db.close()` | Full |

## Units

| Rust (`kerno::Db`) | Python | Node | Status |
|---|---|---|---|
| `store(type, payload)` | `db.store(type, payload)` | `db.store(type, payload)` | Full |
| `get(id)` | `db.read(id)` | `db.read(id)` | Full (renamed) |
| `update(id, payload)` | `db.update(id, payload, if_changed=)` | `db.update(id, payload, { ifChanged })` | Full |
| `update_document(id, doc)` | `db.update_document(id, dict)` | `db.updateDocument(id, doc)` | Full |
| `store_document(type, doc)` | `db.store_document(type, dict)` | `db.storeDocument(type, doc)` | Full |
| `upsert_document(type, field, doc)`, `upsert(type, field, payload)` | `db.upsert(type, field, dict)` | `db.upsert(type, field, doc)` | Full (the bindings take a document; the encoded-payload `upsert` is Rust only) |
| `delete(id)` | `db.delete(id, cascade=)` | `db.delete(id, { cascade })` | Full |
| `unrelate(edge)` | `db.unrelate(relation_id)` | `db.unrelate(relationId)` | Full |

## Relations

| Rust (`kerno::Db`) | Python | Node | Status |
|---|---|---|---|
| `relate(from, to, type)` | `db.relate(from, to, type, unique=)` | `db.relate(from, to, type, { unique })` | Full |
| `related(unit, direction, type)` | `db.related(id, direction=, relation_type=)` | `db.related(id, 'out' \| 'in', type)` | Full |
| `relations(type)` | `db.relations(relation_type=)` | `db.relations(type)` | Full |

## Commits and revisions

| Rust (`kerno::Db`) | Python | Node | Status |
|---|---|---|---|
| `commit(batch)` | `db.commit(ops)` | `db.commit(ops)` | Full |
| `commit_with(batch, CommitMeta)` | `db.commit(ops, reason=, actor=, source=, detailed=True)` | `db.commit(ops, { reason, actor, source })` | Full. Python returns a list of ids unless `detailed=True`; Node always returns the `Committed` report |
| `last_commit_seq()` | `db.last_commit_seq()` | `db.lastCommitSeq()` | Full |
| `read_as_of(id, seq)` | `db.read_as_of(id, seq)` | `db.readAsOf(id, seq)` | Full |
| `refresh()` | `db.refresh()` | `db.refresh()` | Full |
| `compact()` | `db.compact()` | `db.compact()` | Full |
| `run_retention(&policy)` | `db.run_retention(default=, by_type=)` | `db.runRetention({ default, byType })` | Full |
| `verify()` | `db.verify()` | `db.verify()` | Full |

## Schemas and indexes

| Rust (`kerno::Db`) | Python | Node | Status |
|---|---|---|---|
| `define_schema(type, fields)` | `db.define_schema(type, fields)` | `db.defineSchema(type, fields)` | Full |
| `create_index(type, field)` | `db.create_index(type, field)` | `db.createIndex(type, field)` | Full |
| `drop_index(type, field)` | none | none | **Gap** (both) |
| `schema(type)` | none | none | **Gap** (both) |
| `find(type, field, value)` | `db.find(type, field, value)` | `db.find(type, field, value)` | Full |
| `find_range(type, field, low, high, limit)` | `db.find_range(type, field, low=, high=, limit=)` | none | **Gap** (Node) |

## Search and queries

| Rust (`kerno::Db`) | Python | Node | Status |
|---|---|---|---|
| `set_fulltext(type, enabled)` | `db.set_fulltext(type, enabled)` | `db.setFulltext(type, enabled)` | Full |
| `fulltext(type)` | `db.fulltext(type)` | `db.fulltext(type)` | Full |
| `search_text(query, limit)` | none | none | **Gap** (both). The keyword leg is reached through `hybrid_search` |
| `query(filter)` | `db.query(unit_type=, schema_version=, lifecycle=)` | `db.query({ unitType, schemaVersion })` | Full. Node returns active units only, like the facade; Python also takes `lifecycle` |
| `execute(query)` | `db.execute(query)` | `db.execute(query)` | Full |
| `explain(query)` | `db.explain(query)` | none | **Gap** (Node) |
| `hybrid_search(query)` | `db.hybrid_search(text=, vector=, vectors=, seeds=, ...)` | `db.hybridSearch({ text, vectors, relations, weights, fusion, ... })` | Full. The option names differ; see each binding's README |

## Batch operations

`Batch` (`kerno::Batch`) is passed to `commit`. In Python an op is a tuple whose first item is its
name; in Node it is an object with an `op` field.

| Rust (`Batch::…`) | Python op | Node op | Status |
|---|---|---|---|
| `store(type, payload)` | `("store", type, payload)` | `{ op: 'store', unitType, payload }` | Full |
| `update(id, payload)` | `("update", id, payload)` | `{ op: 'update', id, payload }` | Full |
| `update_if_changed(id, payload)` | `("update_if_changed", id, payload)` | `{ op: 'update', id, payload, ifChanged: true }` | Full (Node: a flag, not an op name) |
| `delete(id)` | `("delete", id)` | `{ op: 'delete', id }` | Full |
| `delete_cascade(id)` | `("delete_cascade", id)` | `{ op: 'delete_cascade', id }` | Full |
| `relate(from, to, type)` | `("relate", from, to, type)` | `{ op: 'relate', from, to, relationType }` | Full |
| `relate_unique(from, to, type)` | `("relate_unique", from, to, type)` | `{ op: 'relate', from, to, relationType, unique: true }` | Full (Node: a flag) |
| `unrelate(edge)` | `("unrelate", relation_id)` | `{ op: 'unrelate', relationId }` | Full |
| `archive(id)` | `("archive", id)` | `{ op: 'archive', id }` | Full |
| `unarchive(id)` | `("unarchive", id)` | `{ op: 'unarchive', id }` | Full |
| `restore(id, revision)` | `("restore", id, revision)` | `{ op: 'restore', id, revision }` | Full |
| `upsert(type, field, payload)` | `("upsert", type, field, document)` | `{ op: 'upsert', unitType, field, document }` | Full (a document, not encoded bytes) |

## Not on the facade, but in a binding

These are on `UnitDb` (reached from Rust through `Db::unit_db()`) or are binding helpers. They are
listed so that nothing a binding offers is missing from this page.

| Call | Python | Node |
|---|---|---|
| `UnitDb::history` | `db.history(id, ...)` | `db.history(id, { fromRevision, ... })` |
| `UnitDb::compare` | `db.compare(id, a, b)` | **Gap**: no `compare` |
| read a document back (`Document::decode`) | `db.read_document(id)` | `db.readDocument(id)` |

## Rust only, by design

| Rust (`kerno::Db`) | Why it has no binding |
|---|---|
| `unit_db()` | It returns the `UnitDb` handle, a Rust type. The bindings expose the calls on `UnitDb` that they need, one by one |

## Gaps to close

- `drop_index` and `schema`: missing from both bindings.
- `search_text`: missing from both. `hybrid_search` covers the keyword search.
- `find_range`, `explain` and `compare`: missing from Node.
- `open_with` and `open_with_limits` take a `Config` in Rust; the bindings expose only the read-only
  mode and the batch limit.
