# kerno-ai-sdk

Long-term memory for [Vercel AI SDK](https://ai-sdk.dev) agents, kept in a
[`kerno-server`](../../crates/kerno-server) through the TypeScript client in
[`clients/typescript`](../../clients/typescript). It gives the model three tools, `remember`,
`recall` and `forget`, and the same three operations as plain functions. Not published anywhere
yet: use it from a checkout.

```ts
import { generateText, isStepCount } from 'ai';
import { KernoClient } from 'kerno-client';
import { kernoMemory } from 'kerno-ai-sdk';

const memory = kernoMemory(new KernoClient('http://127.0.0.1:8080')); // { type: 'memory' } by default

// As tools: the model decides what to save and what to look up.
const { text } = await generateText({
  model,
  tools: memory.tools(), // remember, recall, forget
  stopWhen: isStepCount(5), // let the model call a tool and then answer
  prompt: 'I prefer green tea. What do you remember about my drinks?',
});

// As functions: for code that has the answer already.
const saved = await memory.remember('The user likes green tea'); // { id, text }
const found = await memory.recall('tea', 3); // [{ id, text }], best first
await memory.forget(saved.id); // true; false when there is no such memory
```

## How it works

* A memory is one **text unit** of the type `memory` (change it with `{ type }`). Nothing else is
  stored.
* `recall` is a **keyword search** (BM25, any of the words, ranked) through the server's hybrid
  search. There are no embeddings, so nothing here needs a model or a network: the agent asks for
  what it needs in words. For semantic search by embeddings use the
  [LangChain](../langchain) or [LlamaIndex](../llamaindex) stores.
* The tools validate what the model sends with a schema (`zod`); input that does not fit is
  reported back to the model as a tool error and never reaches the server.
* `forget` deletes only a unit of the memory's own type, so a model that guesses an id cannot delete
  anything else in the database. It answers `false` for an unknown, malformed or already deleted id.
* Units are deleted logically, as everywhere in Kerno: the bytes stay until the engine compacts.
* A server that cannot be reached throws a `TransportError`; it is never read as an empty memory.

## Limits

* The server's keyword search cannot be limited to one unit type, so `recall` reads the best 200
  hits of the whole database and keeps the memories among them. Give the memory a database of its
  own if it shares one with much other text.
* A memory is text of at most 1 MiB, and the server has no authentication or TLS yet.

## Tests

```sh
npm ci
npm test          # from integrations/vercel-ai
```

`npm test` first installs and builds the client in `clients/typescript` (it is a `file:`
dependency), then type-checks and runs the tests. They start the real server: `$KERNO_SERVER_BIN`
when set, else the binary `cargo build -p kerno-server` makes. A missing binary fails the run, it
does not skip it. The model is the AI SDK's own mock model, so no network and no API key.
