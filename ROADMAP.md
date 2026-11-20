# Kerno product roadmap

This is the product-facing roadmap: where Kerno is going after Stage 9, and why. The engineering stages that built the engine up to now are tracked in [docs/roadmap/README.md](docs/roadmap/README.md).

> **Nothing here is a date or a promise.** Items are ordered by priority. The file format and the API are not stable until 1.0, and 1.0 is declared only by the project owner (see [Status](README.md#status)).

## Where Kerno wants to be

**The embedded memory engine for AI applications.** SQLite is the default embedded database for general data. Kerno aims at the case SQLite handles poorly: long-term memory for agents and AI apps, where every piece of data keeps its history, its relations and its provenance, and where retrieval mixes exact lookups, keywords, meaning and the relation graph.

## What already exists

- [x] Transactional storage on a leveled LSM tree, with crash, I/O-failure and corruption tests ([ADR-0001](docs/adr/0001-storage-engine.md))
- [x] Units with revisions and provenance: the history of every piece of data can be reconstructed
- [x] Typed relations between units, with indexes that verify themselves
- [x] Query planner with `explain`
- [x] Associative memory and stored embeddings (ranking still experimental)
- [x] Export, import, deep verify and reclaim
- [x] `kerno-mcp`: an MCP server that gives an AI agent a Kerno database as memory ([README](crates/kerno-mcp/README.md))
- [x] `kerno`: an administration CLI ([README](crates/kerno-cli/README.md))
- [x] Fuzzing of the WAL, the manifest and opening, plus a dependency audit in CI

## Phase A: usable from the languages AI is built in

Goal: someone can `pip install` or `npm install` Kerno and store memory in five minutes.

- [ ] Python bindings, published on PyPI
- [ ] Node/TypeScript bindings, published on npm
- [ ] A facade crate `kerno` published on crates.io, with one obvious entry point
- [x] A getting-started guide and three end-to-end examples (agent memory, chat history, document store)
- [ ] Prebuilt binaries for `kerno` and `kerno-mcp` on Linux, macOS and Windows

## Phase B: retrieval that beats a plain table

Goal: the reason to pick Kerno over SQLite for AI work.

- [x] Keyword search (full-text, BM25 ranking)
- [x] Vector search with an approximate nearest-neighbour index (HNSW)
- [x] Pluggable embedding providers: any OpenAI-compatible endpoint, remote (OpenAI, Azure OpenAI) or local (Ollama, the llama.cpp server, vLLM, LM Studio), chosen by the user; no model runs inside the process
- [x] Hybrid retrieval: keywords, meaning and graph relations combined and ranked in one query
- [x] Demonstrated retrieval quality on a frozen evaluation set (criterion Q4, set E8-10), with graph-gated fusion; not yet confirmed on a held-out set
- [x] Memory hygiene for agents: importance, expiry (TTL) and deduplication; summaries of old memory as in-memory derived data rebuilt from their sources, not persisted

## Phase C: a real general-purpose embedded database

Goal: parity with what people expect from SQLite.

- [x] A declarative query language with filters, ordering and joins across types and relations
- [x] Typed schemas with required fields, uniqueness and validation on write
- [x] User-defined secondary indexes on any field
- [x] Documented concurrency: many readers and one writer across the threads of one process, with a stated isolation level; one process at a time owns a directory through an exclusive lock: a second process gets `Locked` until the owner closes the database and releases the lock (processes do not take turns automatically); `kerno-server` is the path for concurrent access from several processes ([concurrency](docs/concurrency.md))
- [ ] Closed performance gaps (U2, U6) and a published, reproducible benchmark against SQLite
- [ ] Long fuzzing runs on an idle machine, at Stage 9 execution counts or more

## Phase D: 1.0

- [ ] File format and API compatibility promise: drafted in [compatibility](docs/compatibility.md) (status: proposed); in force when the owner approves it and declares 1.0
- [x] Export format frozen at version 1 ([export-v1](docs/file-format/export-v1.md))
- [ ] Real use outside the core team, with feedback addressed
- [ ] Contributor license text approved, so external contributions can be accepted

## Phase E: Kerno in the cloud

Goal: use Kerno as the memory store of an AI service, not only inside one process.

- [x] Server mode with an HTTP/gRPC API and Python and TypeScript clients
- [x] Authentication, TLS and per-tenant isolation
- [x] Hot backup, point-in-time restore and WAL replication to object storage (S3, R2)
- [x] Docker image and documented deployment
- [x] Metrics (Prometheus), structured logs and health checks
- [x] Integrations with LangChain, LlamaIndex and the Vercel AI SDK

## Not planned

- Becoming a Postgres replacement: no multi-writer MVCC server, no SQL dialect compatibility as a goal.
- Encryption and authenticated integrity in 1.0 (decided on 2026-11-13; may be revisited after 1.0).

## How to help

Try `kerno-mcp` with your agent, open an issue with what you stored and what you could not find, and see [CONTRIBUTING.md](CONTRIBUTING.md). Feedback from real use decides the order of Phases A to C.
