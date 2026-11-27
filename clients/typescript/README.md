# kerno-client (TypeScript)

A fetch-based, typed HTTP client for [`kerno-server`](../../crates/kerno-server), the Kerno
database server. No runtime dependencies: it uses the global `fetch` (Node 18 or newer, browsers,
Deno, Bun). The package is private and not published anywhere yet: use it from a checkout.

```ts
import { ApiError, KernoClient } from 'kerno-client';

const kerno = new KernoClient('http://127.0.0.1:8080');

// Units: a payload is a string (text), a Uint8Array, or a record of fields for a type that has a schema.
const note = await kerno.createUnit('note', 'Rust has no GC');
const topic = await kerno.createUnit('topic', 'memory');
await kerno.relate(note.id, topic.id, 'about');
await kerno.updateUnit(note.id, 'Rust has no tracing GC', { expectedRevision: 1 });

// Typed records, and the query language.
await kerno.defineSchema('user', [
  { name: 'email', type: 'text', unique: true },
  { name: 'age', type: 'int', indexed: true },
]);
await kerno.createUnit('user', { email: 'ada@example.com', age: 36 });
const { rows } = await kerno.query('FIND user AS u WHERE u.age >= 30 ORDER BY u.age');
console.log(rows[0]?.[0]?.payload);
console.log((await kerno.explain('FIND user AS u WHERE u.age = 36')).chosen);

// The keyword index is off by default (Kerno 1.0.0-alpha.2): turn it on for the types you search.
await kerno.setFulltext('note', true);
// Hybrid retrieval: keywords, relations and (caller-supplied) vectors, fused by reciprocal rank.
for (const hit of await kerno.search({ text: 'rust', graph: { seeds: [topic.id] } })) {
  console.log(hit.id, hit.score, hit.ranks);
}

try {
  await kerno.getUnit('00'.repeat(16));
} catch (e) {
  if (e instanceof ApiError) console.log(e.status, e.code, e.message); // 404 not_found 404 not_found: unit 0000... not found
}
```

Every method throws an `ApiError` when the server refuses the call (`.status`, `.code`; `.message` starts with the status and code; the codes
are in the server README) and a `TransportError` when it cannot be reached or does not answer
within `timeoutMs` (30 s by default); both are `KernoError`s. Names are camelCase here and
snake_case on the wire (`createdAtUs`, `expectedRevision`, `rrfK`).

A few things to know:

* Bytes that are valid UTF-8 come back as a `string`: the server cannot tell them apart.
* JavaScript numbers are doubles: an `int` field beyond 2^53 loses precision. A whole number
  written for a `float` field is accepted and read back as a number.
* `search`'s vector leg ranks the vectors you pass in: the server keeps no vector index.
* The server can serve TLS (`--tls-cert`, `--tls-key`): pass an `https://` URL. API tokens
  (`--data-dir` with `--tokens`) are not sent by this client yet, so use it with a server started
  with `--db` (one database, no authentication), or behind a proxy that adds the token.

## Develop

```sh
npm install
npm test          # tsc --noEmit, then node --test on test/*.test.ts
npm run build     # emits dist/
```

Node 22.18 or newer runs the TypeScript tests directly (type stripping), so the sources use only
erasable syntax and `.ts` import extensions. The tests start the real server: `$KERNO_SERVER_BIN`
when set, else the binary `cargo build -p kerno-server` makes. A missing binary fails the run, it
does not skip it.
