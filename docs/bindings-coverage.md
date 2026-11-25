# Coverage of the `kerno` facade in the bindings

This table lists every public method of the `kerno` facade (`kerno::Db`, `crates/kerno/src/lib.rs`)
and the `Batch` operations, and says whether the Python binding (`bindings/python`, package
`kernodb`, module `kerno`) and the Node binding (`bindings/node`, package `@rixmerz/kernodb`) expose each one,
and under which name. It is built from the source: the Rust names come from `crates/kerno/src/lib.rs`,
the Python names from the `#[pymethods]` of `bindings/python/src/lib.rs` and its stub
`bindings/python/python/kerno/__init__.pyi`, and the Node names from the `#[napi]` methods of
`bindings/node/src/lib.rs`, checked against `bindings/node/index.js` (the package entry, which checks
the arguments) and `bindings/node/index.d.ts`.

Both bindings were built and run against these rows in the alpha.4 gate
(`docs/release/alpha.4-validation-rerun.md`). The tests in `bindings/python/tests` and
`bindings/node/__test__` are the checks the code is held to.

Status: **Full** means the binding exposes the call with the facade's semantics. **Partial** means it
exposes only part of it, or behaves differently in a way named in the row. **Gap** means there is no
way to call it from that binding.

## Lifecycle and open

| Rust (`kerno::Db`) | Python | Node | Status |
|---|---|---|---|
| `Db::open(path)` | `kerno.open(path)` | `open(path)` | Full |
| `Db::open_with(path, Config)` | `kerno.open(path, read_only=)` | `open(path, { readOnly })` | Partial: the bindings set the read-only mode and nothing else of `Config` |
| `Db::open_with_limits(path, Config, max)` | `kerno.open(path, max_batch_ops=)` | `open(path, { maxBatchOps })` | Partial: same as `open_with` |
| `Db::close` | `db.close()` (also `with kerno.open(...)`) | `db.close()` | Full. Any later call on a closed handle raises `ClosedError` in Python and throws `CLOSED` in Node; closing twice is fine in both |

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
| `commit_with(batch, CommitMeta)` | `db.commit(ops, reason=, actor=, source=, detailed=True)` | `db.commit(ops, { reason, actor, source })` | Full. Python returns a list of ids unless `detailed=True`, then a dict; Node always returns the `Committed` report |
| `last_commit_seq()` | `db.last_commit_seq()` | `db.lastCommitSeq()` | Full |
| `read_as_of(id, seq)` | `db.read_as_of(id, seq)` | `db.readAsOf(id, seq)` | Full. Both raise `PrunedError` / `PRUNED` for a pruned revision and `HistoryUnavailableError` / `HISTORY_UNAVAILABLE` for a deleted unit whose payloads `compact()` dropped |
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
| `substring(type)` | `db.substring(type)` | `db.substring(type)` | Full |
| `set_substring(type, enabled)` | `db.set_substring(type, enabled)` | `db.setSubstring(type, enabled)` | Full |
| `search_substring(needle, limit, types)` | `db.search_substring(needle, limit=, types=)` | `db.searchSubstring(needle, { limit, types })` | Full. Returns the ids of the units whose payload holds `needle` |
| `search_text(query, limit, types)` | `db.search_text(query, limit=, types=)` | `db.searchText(query, { limit, types })` | Full. `types` restricts the search to unit types; `hybrid_search` takes it as `text_types` / `textTypes` |
| `query(filter)` | `db.query(unit_type=, schema_version=, lifecycle=)` | `db.query({ unitType, schemaVersion, lifecycle })` | Full. `lifecycle` is `'active'` (the default), `'archived'`, `'deleted'` or `'all'` in both. Any other value is refused with `InvalidArgumentError` / `INVALID_ARGUMENT` |
| `execute(query)` | `db.execute(query)` | `db.execute(query)` | Full |
| `explain(query)` | `db.explain(query)` | none | **Gap** (Node) |
| `hybrid_search(query)` | `db.hybrid_search(text=, vector=, vectors=, seeds=, text_types=, keyword_weight=, vector_weight=, relation_weight=, limit=, candidates=, rrf_k=, max_depth=, metric=)` | `db.hybridSearch({ text, textTypes, vectors, relations: { seeds, types, direction, maxDepth, visitBudget }, weights, fusion, rrfK, candidates, limit })` | Partial. Python has only reciprocal rank fusion (no `fusion`, no `graph_gated`), no relation `direction`, no relation-type filter, and no `visit_budget`: its relation leg walks both directions over every relation type with the engine's maximum budget of 100,000 relation entries (`MAX_ASSOC_VISITS`). Node takes all of them, and defaults to `both`, every type, and 10,000 entries (up to 100,000). Both return a float `score` (the sum of the leg scores) and each leg's score and rank: Python `keyword_score`, `vector_score`, `relation_score` and `keyword_rank`, `vector_rank`, `relation_rank`; Node `keywordScore`, `vectorScore`, `relationScore` and `ranks.keyword`, `ranks.vector`, `ranks.relations` |

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
| `UnitDb::history` | `db.history(id, limit=, with_payload=, from_revision=, to_revision=, order=)`. For a deleted unit whose payloads `compact()` dropped, `with_payload=True` lists its revisions with `payload` `None` | `db.history(id, { limit, withPayload, fromRevision, toRevision, order })`. Partial: for the same unit, `withPayload: true` throws `HISTORY_UNAVAILABLE` instead of listing the revisions without payloads. Pass `withPayload: false` to list them |
| `UnitDb::compare` | `db.compare(id, a, b)` | **Gap**: no `compare` |
| read a document back (`Document::decode`) | `db.read_document(id)` | `db.readDocument(id)` |
| the `Unit` record that `read`, `query` and `find` return | fields `id, unit_type, schema_version, lifecycle, revision, oldest_revision, created_at_us, updated_at_us, payload` | fields `id, unitType, schemaVersion, lifecycle, revision, oldestRevision, createdAtUs, updatedAtUs, payload` (Full: same fields) |
| the `Revision` record that `history` and `read_as_of` return | fields `revision, kind, commit_seq, created_at_us, lifecycle, schema_version, content_hash, payload` | fields `revision, kind, commitSeq, createdAtUs, lifecycle, schemaVersion, contentHash, payload` (Full: same fields) |

Timestamps (`created_at_us`, `updated_at_us`, `createdAtUs`, `updatedAtUs`) are microseconds since the
Unix epoch, descriptive only; the ordering of revisions is by revision number, never by time.

## Errors

Every error from the engine reaches the caller in its binding's form. Python raises the named
exception, and Node throws an error whose `code` is the stable string in the last column. In Python,
`InvalidArgumentError` and `ClosedError` are also `ValueError`s.

| Python exception | Node `code` | Note |
|---|---|---|
| `NotFoundError` | `NOT_FOUND` | |
| `DeletedError` | `DELETED` | |
| `LockedError` | `LOCKED` | |
| `CorruptionError` | `CORRUPTION` | |
| `InvalidArgumentError` (and the subclasses below) | `INVALID_ARGUMENT` | |
| `ClosedError` | `CLOSED` | A call on a closed handle. Same condition in both bindings |
| `HistoryUnavailableError` | `HISTORY_UNAVAILABLE` | The payload of a revision is gone: the unit was deleted and `compact()` dropped its payloads. Python's `history(with_payload=True)` swallows this and lists the revisions without payloads; Node's throws (see the `history` row) |
| `PayloadTooLargeError`, `ChangeSetTooLargeError` | `PAYLOAD_TOO_LARGE`, `CHANGESET_TOO_LARGE` | |
| `SchemaExistsError`, `NotIndexedError`, `UnknownTypeError`, `NotUniqueError`, `BadEndpointError` | `SCHEMA_EXISTS`, `NOT_INDEXED`, `UNKNOWN_TYPE`, `NOT_UNIQUE`, `BAD_ENDPOINT` | Subclasses of `InvalidArgumentError` in Python |
| `ReadOnlyError`, `DatabaseNotFoundError` | `READ_ONLY`, `DATABASE_NOT_FOUND` | |
| `HasRelationsError`, `PrunedError`, `SchemaError` | `HAS_RELATIONS`, `PRUNED`, `SCHEMA` | |
| `KernoError` | `KERNO` | Any other engine failure |

## Numeric ranges

Each binding accepts a different range of whole numbers, and refuses a value outside its own range with
its own invalid-argument error (**difference, not a gap**):

- Python: counts and revisions take 0 to 2**64 - 1; `limit` and `keep_last` take 1 to 2**64 - 1
  (the engine's own limits still apply, for example `max_batch_ops`).
- Node: a number argument must be a whole number up to 2^53 (`limit` and `keepLast` from 1).
  `limit`, `schemaVersion` and `maxBatchOps` are also capped at 2^32 - 1.

## Unknown options

Both bindings refuse an option the call does not know, with the binding's invalid-argument error, and
change nothing:

- Python: an unknown keyword argument raises `InvalidArgumentError` (the call's own `TypeError` is
  converted by the package's wrapper in `kerno/__init__.py`).
- Node: every options object (`open`, `update`, `delete`, `relate`, `query`, `commit`, `history`,
  `runRetention`, `searchText`, `searchSubstring`, `defineSchema`, `hybridSearch`, and the nested
  objects of `hybridSearch`) throws `INVALID_ARGUMENT` with `unknown option "<key>"` naming the
  known keys. The check is in `bindings/node/index.js`, so it applies to the package entry, not to the
  native addon loaded directly.

## Rust only, by design

| Rust (`kerno::Db`) | Why it has no binding |
|---|---|
| `unit_db()` | It returns the `UnitDb` handle, a Rust type. The bindings expose the calls on `UnitDb` that they need, one by one |

## Gaps to close

- `drop_index` and `schema`: missing from both bindings.
- `find_range`, `explain` and `compare`: missing from Node. (`find_range` and `explain` are in Python.)
- Hybrid search, Python: no `fusion` (only reciprocal rank fusion), no relation `direction`, no
  relation-type filter, no `visit_budget`. Node has all four.
- `history(with_payload=True)` for a deleted, compacted unit: Python lists the revisions without
  payloads, Node throws `HISTORY_UNAVAILABLE`. This is a difference, and the Node `history` row names it.
- `open_with` and `open_with_limits` take a `Config` in Rust; the bindings expose only the read-only
  mode and the batch limit.
