# kerno-units

The units layer of [Kerno](https://github.com/Rixmerz/kernodb): typed units, their revisions and change sets,
relations, indexes (including HNSW vector search and BM25 keyword search), the query planner and derived data,
over the storage engine.

> **Alpha (1.0.0-alpha.1).** The on-disk format v1 is frozen
> ([ADR-0013](https://github.com/Rixmerz/kernodb/blob/main/docs/adr/0013-format-v1-and-engine-protocol.md)).
> The narrative-thread retrieval experiment is not part of this crate. Applications should use the `kerno`
> crate, which re-exports this one as `kerno::units`.

```sh
cargo add kerno-units@1.0.0-alpha.1
```

Optional features: `test-support` (test hooks and the raw write accessor; off by default) and `embed-openai`
(a client for an OpenAI-compatible embeddings endpoint; off by default).

Licensed under either of the Apache License, Version 2.0 or the MIT license, at your option.
