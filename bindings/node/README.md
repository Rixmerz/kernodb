# kernodb

Node.js bindings for [Kerno](https://github.com/Rixmerz/kernodb), an embedded database of
**units** (typed records with a payload), the **relations** between them, and queries over both.
It runs inside your process and keeps its data in a local directory. TypeScript types ship with
the package, and the API follows the [Python binding](https://github.com/Rixmerz/kernodb/tree/main/bindings/python)
(camelCase names, optional arguments in an options object). Calls that one has and the other lacks are
listed in the [coverage table](https://github.com/Rixmerz/kernodb/blob/main/docs/bindings-coverage.md).
The guides [Kerno as a revisioned history next to another database](https://github.com/Rixmerz/kernodb/blob/main/docs/guides/history-alongside.md)
and [one unit per entity, keyed by a logical key](https://github.com/Rixmerz/kernodb/blob/main/docs/guides/one-unit-per-entity.md)
have Python examples. Every call they use exists here under its camelCase name (see the coverage table).

> **Alpha: 1.0.0-alpha.3.** It does not open databases written by alpha.1 (see
> [Upgrading](#upgrading-from-100-alpha1)). The keyword index is opt-in per type. The on-disk format v1 is frozen
> ([ADR-0013](https://github.com/Rixmerz/kernodb/blob/main/docs/adr/0013-format-v1-and-engine-protocol.md)).
> There is no 1.0 release and no stability promise yet for the API. Benchmarks and long fuzzing runs are not yet certified.

## Install

You need Node 20 or later. The package includes a native addon (napi-rs).

```sh
npm install @rixmerz/kernodb@alpha      # or: pnpm add @rixmerz/kernodb@alpha
```

## Quickstart (five minutes)

Save this as `memory.mjs` and run `node memory.mjs`:

```js
import { open } from '@rixmerz/kernodb'

// 1. Open a database in a directory (created when missing).
const db = open('./memory-db')

// 2. Store units: a type and a payload (a string, or a Buffer for binary data).
const note = db.store('note', 'Rust has no GC')
const topic = db.store('topic', 'memory')

// 3. Read one back. Ids are 32-digit hex strings; `read` returns null for an unknown id.
const unit = db.read(note)
console.log(unit.unitType, unit.payload.toString()) // note Rust has no GC

// 4. Relate two units, then follow the relation from either end.
db.relate(note, topic, 'about')
console.log(db.related(note, 'out'))   // an array with one relation: { id, from, to, relationType: 'about' }
console.log(db.related(topic, 'in'))   // the same edge, seen from the topic

// 5. Query by type (or by schemaVersion; no filter returns every unit).
console.log(db.query({ unitType: 'note' }).length) // 1

// 6. Close it. Reopening the same directory later gives you the same data.
db.close()
```

Run it in a directory where `memory-db` does not exist yet, because the query count assumes one `note`.

## Conventions

* **Ids** (units, relations) are 32-digit lower-case hex strings.
* **Payloads** go in as a `string` (stored as UTF-8) or a `Buffer`, and come out as a `Buffer`.
* **Every call is synchronous** and blocks the event loop for the length of the commit.
* **Optional arguments** go in an options object: `db.update(id, payload, { ifChanged: true })`.
* **Counters** (`commitSeq`, `revision`, sizes) are JS numbers, exact up to 2^53.
* **Times** (`createdAtUs`, `updatedAtUs`) are microseconds since the Unix epoch. They describe,
  they never order: use `commitSeq` and `revision` for order.
* An optional result field that has no value is absent (`undefined`), a missing unit is `null`.

## API

### Open and close

| Call | Returns |
|---|---|
| `open(path, { readOnly?, maxBatchOps? })` | a `Db`. The directory is created when missing. `readOnly` writes nothing to it, takes no lock, and does not create it, so it can run beside a writer (see `refresh`); `maxBatchOps` sets the most operations one `commit` may hold (default 10,000). One read-write handle holds a directory at a time: a second read-write `open` throws `LOCKED`. |
| `db.close()` | nothing. Closing twice is fine; any other call afterwards throws `CLOSED`. |

### Units and relations

| Call | Returns |
|---|---|
| `db.store(unitType, payload)` | the new unit's id. `unitType` is 1 to 64 bytes of UTF-8. |
| `db.read(id)` | the `Unit`, or `null` |
| `db.update(id, payload, { ifChanged? })` | `true` when a revision was written. With `ifChanged`, a payload equal to the current one writes nothing and returns `false`. The earlier revisions stay in the history. |
| `db.delete(id, { cascade? })` | nothing. Writes a tombstone revision; the history is kept. Without `cascade` a unit that still has relations throws `HAS_RELATIONS`; with it, the relations are removed first, in the same commit. |
| `db.relate(from, to, relationType, { unique? })` | the relation's id. Both units must exist and be active; `relationType` is 1 to 256 bytes. With `unique`, an existing relation of that type between the two units is reused. |
| `db.unrelate(relationId)` | nothing |
| `db.related(id, 'out' \| 'in', relationType?)` | the `Relation[]` that start at (`out`) or end at (`in`) the unit |
| `db.relations(relationType?)` | every live `Relation[]`, in one scan: much faster than `related` per unit |
| `db.query({ unitType?, schemaVersion? }?)` | every active `Unit` that matches, in id order. No filter returns every one. |

### Atomic commits

`db.commit(ops, { reason?, actor?, source? })` applies `ops` as one change set, all of them or
none, and returns a `Committed` report:

```ts
{ units: string[]       // ids of the units the `store` ops created, in order
  relations: string[]   // ids of the relations created (or found, for a unique relate), in order
  unchanged: number     // `update` ops skipped by `ifChanged`, and `upsert` ops that found the same document
  upserts: Upserted[]   // what each `upsert` op did, in order (see below); an upserted unit is not in `units`
  commitSeq?: number }  // the commit's sequence; absent when nothing was written
```

`reason` (at most 1024 bytes), `actor` and `source` (a string or Buffer, at most 256 bytes each)
are recorded with every revision of the commit.

| `op` | Other fields |
|---|---|
| `'store'` | `unitType`, `payload` |
| `'update'` | `id`, `payload`, `ifChanged?` |
| `'delete'` | `id`, `cascade?` |
| `'delete_cascade'` | `id` |
| `'archive'`, `'unarchive'` | `id` |
| `'restore'` | `id`, `revision`: a new revision that copies that one |
| `'relate'` | `from`, `to`, `relationType`, `unique?` |
| `'unrelate'` | `relationId` |
| `'upsert'` | `unitType`, `field`, `document`: see "Upsert by a unique field" |

A relation end (`from`, `to`) is a unit id, or the 0-based position in the list of an earlier `store`
or `upsert` op of the same commit (every op counts, not only the stores; an upsert names the unit it
created, revised or found unchanged). Any other position (a later op, an update, a relation, past the
end) throws `BAD_ENDPOINT` and nothing is written:

```js
const { units, commitSeq } = db.commit(
  [
    { op: 'store', unitType: 'note', payload: 'first' },
    { op: 'store', unitType: 'note', payload: 'second' },
    { op: 'relate', from: 0, to: 1, relationType: 'next' },
  ],
  { reason: 'import', actor: 'importer' },
)
```

### Upsert by a unique field

Ids are random, so a logical key (an email, a path) needs a lookup. Declare the field `unique`
and `db.upsert(unitType, field, document)` revises the active unit that holds the same value of
it, or stores a new one. It returns `{ id, outcome }`, where `outcome` is `'created'`,
`'updated'` (a new revision) or `'unchanged'` (the unit had this document already: nothing is
written).

```js
db.defineSchema('user', [{ name: 'email', type: 'text', unique: true }, { name: 'name', type: 'text' }])
const { id } = db.upsert('user', 'email', { email: 'a@example.com', name: 'Ada' })   // created
db.upsert('user', 'email', { email: 'a@example.com', name: 'Ada L.' }).outcome       // 'updated'
db.history(id).map((r) => r.kind)                                                    // ['update', 'create']
```

In `commit`, `{ op: 'upsert', unitType, field, document }` runs in the same change set as the other
ops, and several upserts of one key in a commit make one unit (the last document wins). A field
that is not declared `unique`, or a type with no schema, throws `NOT_UNIQUE`; a document
with no value in the field throws `SCHEMA`. Only active units hold a key: an archived or
deleted one does not.

### History and maintenance

| Call | Returns |
|---|---|
| `db.history(id, { limit?, withPayload?, fromRevision? })` | the newest `limit` `Revision`s (default 100, at least 1) from `fromRevision` on (default: the oldest kept), newest first; empty for an unknown id. `withPayload` reads and verifies each revision's content. A `fromRevision` retention pruned throws `PRUNED`. |
| `db.readAsOf(id, commitSeq)` | the `Revision` in force once that commit committed (with its payload), or `null` when the unit did not exist yet. A deleted unit returns its tombstone. A sequence past the last commit reads the current state. Throws `PRUNED` when retention removed the revision. |
| `db.lastCommitSeq()` | the sequence of the last commit (0 for an empty database) |
| `db.refresh()` | on a `readOnly: true` database: pick up the commits a writer made since the open; returns the sequence now read |
| `db.verify()` | `{ clean, commitSeq, units, revisions, relations, findings, findingsOmitted, storageError? }`. A damaged store is reported (`clean: false`), not thrown. |
| `db.runRetention({ default?, byType? })` | prune old revisions; a rule is `{ keepLast }` (newest N), `{ keepFor }` (seconds) or `{ keepSince }` (commit sequence); returns `{ unitsRetained, revisionsPruned, payloadsDropped, bytesLogicallyFreed, commitSeq? }`. `readAsOf` of a pruned revision and `history` from one throw `PRUNED`; `history` lists what is kept. A bad rule throws `INVALID_ARGUMENT` and writes nothing; `null` is the same as no rule. The policy is not stored: pass it each time |
| `db.compact()` | `{ before?, after? }`, the stored bytes around the compaction: `before` is the size when the call was made (every commit so far counts), `after` the size once the payloads of deleted units are dropped and their space given back. |
| `db.setFulltext(unitType, enabled)` | nothing. Turns keyword (BM25) indexing of a type on or off (off by default); its active units are indexed or cleared in the same commit. |
| `db.fulltext(unitType)` | `boolean`: whether the type is keyword-indexed. |

### Schemas, indexes and documents

A **document** is an object of `boolean`, `number`, `bigint`, `string` and `Buffer` values, stored
as a unit's payload. A type may have a schema; once it does, every write of that type is checked
against it, and a refused write stores nothing (`SCHEMA`).

| Call | Returns |
|---|---|
| `db.defineSchema(unitType, fields)` | nothing. Each field is `{ name, type, required?, unique?, indexed? }` with `type` one of `'bool' \| 'int' \| 'float' \| 'text' \| 'bytes'`. `unique` implies `indexed`. |
| `db.createIndex(unitType, field)` | nothing. Indexes a field of the type's schema, over the units that exist already. |
| `db.storeDocument(unitType, document)` | the new unit's id |
| `db.updateDocument(id, document)` | nothing |
| `db.upsert(unitType, field, document)` | `{ id, outcome }`: store the document or revise the active unit holding the same value of the unique `field`; `outcome` is `'created'`, `'updated'` or `'unchanged'` |
| `db.readDocument(id)` | the document, or `null` for an unknown or deleted unit |
| `db.find(unitType, field, value)` | the `Unit[]` of the type whose indexed `field` equals `value` |
| `db.execute(query)` | `{ aliases: string[], rows: Unit[][] }`: one `Unit` per alias in each row. See the [query language](https://github.com/Rixmerz/kernodb/blob/main/docs/query-language.md). |

JavaScript has one number type, so a `number` is an `int` when it is a whole number within
±2^53, and a `float` otherwise, unless the type's schema declares the field: a `float` field
takes any number, an `int` field refuses one that is not whole (`INVALID_ARGUMENT`). A `bigint` is
always an `int`. Reading gives back a `number`, or a `bigint` for an int beyond ±2^53.

### Hybrid search

`db.hybridSearch(query)` answers one query over up to three legs and fuses their rankings by
weighted reciprocal rank fusion. It wraps the engine's `Db::hybrid_search`. A leg runs when its
input is set and its weight is above 0; at least one must run.

```js
db.setFulltext('note', true)
const hits = db.hybridSearch({
  text: 'garbage collector',                                   // keywords (BM25)
  vectors: { query: [0.1, 0.9], items: [{ id: noteId, vector: [0.2, 0.8] }] },
  relations: { seeds: [topicId], types: ['about'], maxDepth: 1 },
  weights: { keyword: 2 },
  limit: 5,
})
// [{ id, score, ranks: { keyword?, vector?, relations? } }], best first
```

| Field | Meaning |
|---|---|
| `text` | the keyword leg. It only searches types that have `setFulltext(type, true)`. |
| `vectors` | `{ query, items, metric? }`: the vector leg ranks the `items` (`{ id, vector }`) against `query` by `'cosine'` (default) or `'dot'`. All vectors share the dimension of `query`. |
| `relations` | `{ seeds, types?, direction?, maxDepth?, visitBudget? }`: units reached from the seed units by walking relations (`direction` `'out'`, `'in'` or `'both'`; `maxDepth` 1 to 4, default 2; `visitBudget` up to 100,000, default 10,000). Seeds must be active and are not hits. |
| `weights` | `{ keyword?, vector?, relations? }`, 0 to 1000, default 1 each. 0 switches a leg off. |
| `fusion` | `'rrf'` (default), or `'graph_gated'`: units the relations leg found rank first, nearest hop first. |
| `rrfK` | the `k` of the fusion, default 60 |
| `candidates` | units each leg contributes, 1 to 1000, default 100 |
| `limit` | hits returned, 1 to 1000, default 10 |

A hit's `score` is only for comparing hits of the same query. Units that are not active are never
hits.

## Errors

Every failure is a plain `Error` with a stable string `code` (and a message). Match on `code`, not on the message:

```js
try {
  db.delete(id)
} catch (e) {
  if (e.code === 'HAS_RELATIONS') db.delete(id, { cascade: true })
  else throw e
}
```

| `code` | Python equivalent | Thrown when |
|---|---|---|
| `NOT_FOUND` | `NotFoundError` | no unit, relation or revision with that id |
| `DELETED` | `DeletedError` | the unit is deleted (or not in the lifecycle the call needs) |
| `LOCKED` | `LockedError` | another handle or process holds the database directory |
| `CORRUPTION` | `CorruptionError` | stored bytes failed validation (a checksum or a structure rule) |
| `INVALID_ARGUMENT` | `InvalidArgumentError` | an argument was refused: a malformed id, a type name, a limit, a bad op, a value no document can hold (`null`, a bigint outside 64 bits), a `find` value over 254 bytes |
| `SCHEMA_EXISTS` | `SchemaExistsError` | `defineSchema` for a type that already has a different schema (the same one is accepted and writes nothing) |
| `NOT_INDEXED` | `NotIndexedError` | `find` on a field that is not indexed; a keyword search while no type has `setFulltext` |
| `NOT_UNIQUE` | `NotUniqueError` | `upsert` by a field the schema does not declare unique (or a type with no schema) |
| `BAD_ENDPOINT` | `BadEndpointError` | a `relate` end that is not an earlier `store` or `upsert` op of the commit |
| `PAYLOAD_TOO_LARGE` | `PayloadTooLargeError` | the payload is over 1 MiB |
| `CHANGESET_TOO_LARGE` | `ChangeSetTooLargeError` | a commit over 10,000 ops (see `maxBatchOps`) or over the engine's bytes per commit |
| `READ_ONLY` | `ReadOnlyError` | a write through a handle opened with `readOnly: true` |
| `DATABASE_NOT_FOUND` | `DatabaseNotFoundError` | `readOnly: true` on a directory with no database |
| `HAS_RELATIONS` | `HasRelationsError` | the unit still has live relations; unrelate them or use `cascade` |
| `PRUNED` | `PrunedError` | retention removed the revision asked for (`readAsOf`, or `history` from a pruned `fromRevision`) |
| `SCHEMA` | `SchemaError` | the type's schema refused the document, or the payload is not a document |
| `CLOSED` | `ValueError` | the handle is closed |
| `KERNO` | `KernoError` | any other engine failure |

## Limits

* **Payload: at most 1 MiB** (1,048,576 bytes) per unit. A larger one throws `PAYLOAD_TOO_LARGE`.
* **Unit type:** 1 to 64 bytes of UTF-8. **Relation type:** 1 to 256 bytes.
* **Commit metadata:** `reason` at most 1024 bytes, `actor` and `source` at most 256 bytes each.
* **Operations per commit:** 10,000 by default; `open(path, { maxBatchOps })` changes it. A unit
  can appear in one operation of a commit.
* **Keyword search is opt-in per type.** A type is not keyword-indexed until you call
  `setFulltext(type, true)`; until then `hybridSearch`'s keyword leg finds none of its units.
  Turning it on indexes the type's active units in the same commit, and turning it off clears them.
* **Vector search** ranks the vectors you pass in each call, by brute force. There is no stored
  vector index in this binding.
* **One handle per directory**, and one process at a time.
* **Hybrid search** returns at most 1000 hits; each leg contributes at most 1000 units.
* **Numbers** that cross as JS numbers (sequences, revisions, sizes) are exact up to 2^53.
* `readAsOf` and `history` see what retention kept. Retention is a call, `db.runRetention(...)`, and the policy is not stored: pass it each time you prune.

## Durability

`open` asks for durable commits: a commit is on stable storage when the call returns. That
guarantee is verified on Linux (ext4, xfs) only; on macOS and Windows the database works, but the
guarantee is not verified.

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

## Build from source

You need a Rust toolchain (1.89 or later) and pnpm. From `bindings/node`:

```sh
pnpm install
pnpm build   # compiles the addon and regenerates index.js and index.d.ts
pnpm test    # type-checks the types, then runs the tests against the built addon
```

The crate is outside the cargo workspace of the repository on purpose and has its own
`Cargo.lock`.

## License

Licensed under either of the Apache License, Version 2.0 or the MIT license, at your option.
