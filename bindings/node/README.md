# kernodb

Node.js bindings for [Kerno](https://github.com/Rixmerz/kernodb), an embedded database of
**units** (typed records with a payload), the **relations** between them, and queries over both.
It runs inside your process and keeps its data in a local directory. TypeScript types ship with
the package.

> **Alpha: 1.0.0-alpha.1.** The on-disk format v1 is frozen
> ([ADR-0013](https://github.com/Rixmerz/kernodb/blob/main/docs/adr/0013-format-v1-and-engine-protocol.md)).
> There is no 1.0 release and no stability promise yet for the API. Benchmarks and long fuzzing runs are not yet certified.

## Install

You need Node 20 or later. The package includes a native addon (napi-rs).

```sh
npm install kernodb@alpha      # or: pnpm add kernodb@alpha
```

## Quickstart (five minutes)

Save this as `memory.mjs` and run `node memory.mjs`:

```js
import { open } from 'kernodb'

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

The example was run on Node 26 against the built addon (the declared engine is Node 20 or later, not run here).
Run it in a directory where `memory-db` does not exist yet, because the query count assumes one `note`.

## API

| Call | Returns |
|---|---|
| `open(path)` | a `Db`; one process holds a directory at a time |
| `db.store(unitType, payload)` | the new unit's id; `unitType` is 1 to 64 bytes, `payload` a `string` or `Buffer` |
| `db.read(id)` | the `Unit`, or `null` |
| `db.relate(from, to, relationType)` | the relation's id; both units must exist |
| `db.related(id, 'out' \| 'in', relationType?)` | the `Relation[]` that start at or end at the unit |
| `db.query(filter?)` | every active `Unit` matching `{ unitType?, schemaVersion? }`, in id order |
| `db.close()` | nothing; closing twice is fine, any other call afterwards throws |

Every call is synchronous and throws an `Error` with a message on failure (a bad id, an invalid
type, a directory already open, a closed database). The exact types are in `index.d.ts`.

## Durability

`open` asks for durable commits: a commit is on stable storage when the call returns. That
guarantee is verified on Linux (ext4, xfs) only; on macOS and Windows the database works, but the
guarantee is not verified.

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
