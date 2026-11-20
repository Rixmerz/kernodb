# kerno-server

Serves Kerno databases over an HTTP JSON API and, when asked, gRPC. Both transports are thin
layers over the same code (`Service`), so an operation does the same thing on either, and a unit
written through one is read through the other. Python and TypeScript clients are in
[`clients/`](../../clients).

**Running it in the open** (API tokens, one database per tenant, TLS, health checks, Prometheus
metrics, JSON logs) is in [docs/server.md](../../docs/server.md). In short: `--db <dir>` serves
one database with no authentication and is for loopback and development; `--data-dir <dir>
--tokens <file>` serves one database per tenant and every request needs a token of its tenant.

## Build and run

```sh
cargo build --release -p kerno-server
target/release/kerno-server --db /path/to/dir                     # HTTP on 127.0.0.1:8080
target/release/kerno-server --db /path/to/dir --grpc-listen 127.0.0.1:50051
target/release/kerno-server --data-dir /var/lib/kerno --tokens tokens \
  --tls-cert cert.pem --tls-key key.pem               # tenants, tokens, TLS: see docs/server.md
```

| Flag | Meaning |
|---|---|
| `--db <dir>` | The database directory, created if it does not exist. Or set `KERNO_SERVER_DB`. |
| `--listen <addr>` | The HTTP address. Default `127.0.0.1:8080`. Port `0` picks a free port. |
| `--grpc-listen <addr>` | Also serve gRPC on this address. Off by default. |
| `--data-dir <dir>`, `--tokens <file>` | Tenant mode instead of `--db`: see [docs/server.md](../../docs/server.md). |
| `--tls-cert <pem>`, `--tls-key <pem>` | Serve both transports over TLS. |
| `--backup <url>`, `--backup-interval <s>` | Keep a base backup and the shipped write-ahead log of every database in `file:///dir` or `s3://bucket/prefix`, every `s` seconds (default 5). See [docs/backup.md](../../docs/backup.md). |

On start the server prints `listening on http://ADDR` (`https://` with TLS, and `grpc listening
on ADDR`) to stdout, with the port it really got; its logs are JSON lines on stderr. One process holds a directory at a time: a second server on it exits
with an error. On SIGINT or SIGTERM it stops accepting, lets the requests in flight finish (the HTTP side
waits at most 10 seconds), closes the database and exits 0.

Building needs no `protoc`: `build.rs` compiles `proto/kerno/v1/kerno.proto` with `protox`.

## HTTP API

Bodies and answers are JSON. Ids are 32 lowercase hex digits. Every path below is relative to `/v1`
(`GET /health` is `GET /v1/health`). In tenant mode every one of them needs
`Authorization: Bearer <token>` and acts on the token's tenant only. `GET /livez`, `GET /readyz`
and `GET /metrics` (outside `/v1`, no token) are in [docs/server.md](../../docs/server.md).

| Route | Does |
|---|---|
| `GET /health` | `{"status": "ok", "version": ...}` when the database answers a read, else an `internal` error (500). |
| `POST /units` | Create a unit. Body `{"type", "payload"}`. 201 and the unit. |
| `GET /units?type=&limit=&after=` | List active units in id order, `limit` 1 to 1000 (default 100). `next` is the `after` of the next page, `null` on the last. |
| `GET /units/{id}` | One unit, a deleted one included (as a tombstone). |
| `PUT /units/{id}` | Replace the payload: a new revision. Body `{"payload"}`. |
| `POST /units/{id}/archive`, `/unarchive` | Change the lifecycle. A new revision. |
| `DELETE /units/{id}` | Delete logically (204). The unit stays readable with `lifecycle: "deleted"` and `payload: null`. |
| `POST /relations` | Body `{"from", "to", "type"}`. 201 and the relation. |
| `GET /units/{id}/relations?direction=&type=` | `direction` is `out`, `in` or `both` (default). Each entry says which side it was found from. |
| `DELETE /relations/{id}` | 204. |
| `POST /schemas` | Give a unit type a schema (below). A type has one schema for good. |
| `GET /schemas/{type}` | The schema; the type is percent-encoded in the path. |
| `POST /query` | Run a query of the [query language](../../docs/query-language.md). Body `{"query"}`. |
| `POST /query/explain` | How the query would run: the access paths with their costs, the one chosen, the joins. |
| `POST /search` | Hybrid retrieval (below). |
| `POST /backup` | Take a base backup now (201 and its description). Only with `--backup`, else 404. See [docs/backup.md](../../docs/backup.md). |

`PUT`, `DELETE` and `archive`/`unarchive` take `?expected_revision=N`: the change is made only if
the unit is at revision `N`, else 409 `revision_conflict` and nothing changes.

A unit that has a relation, in either direction, cannot be archived or deleted (409 `conflict`).
Delete its relations first.

### Units and payloads

```json
{
  "id": "01a1ed64ea61729aaacff59d616cd231",
  "type": "note",
  "lifecycle": "active",
  "revision": 1,
  "created_at_us": 1794984176225494,
  "updated_at_us": 1794984176225494,
  "payload": {"text": "Rust has no GC"}
}
```

A payload is exactly one of:

* `{"text": "..."}`: UTF-8 text.
* `{"hex": "00ff..."}`: raw bytes. A read answers `text` when the bytes are valid UTF-8, `hex` when
  they are not.
* `{"fields": {"name": "ada", "age": 36}}`: a typed record, only for a unit type that has a
  schema. A field is a JSON boolean, integer, float or string, or `{"hex": "..."}` for bytes. A
  JSON integer written for a `float` field is accepted and read back as a float (JavaScript cannot
  write `5.0`).

A unit of a type with a schema always holds `fields`; the schema refuses anything else (422
`schema_violation`) and nothing is stored.

### Schemas

```json
{"type": "user", "fields": [
  {"name": "email", "type": "text", "required": true, "unique": true},
  {"name": "age",   "type": "int",  "indexed": true}
]}
```

Field types: `bool`, `int`, `float`, `text`, `bytes`. `unique` implies `indexed`, and the answer
says so. Indexed fields speed up `POST /query`; `POST /query/explain` shows when.

### Hybrid search

`POST /search` ranks units over up to three legs and fuses them by weighted reciprocal rank
fusion (see `kerno_units::hybrid` for the arithmetic):

```json
{
  "text": "rust memory",
  "vector": {"query": [0.1, 0.9], "metric": "cosine", "vectors": {"<unit id>": [0.2, 0.8]}},
  "graph": {"seeds": ["<unit id>"], "direction": "both", "relation_types": ["about"],
            "max_depth": 2, "limit": 20, "visit_budget": 10000},
  "weights": {"keyword": 1, "vector": 1, "graph": 1},
  "rrf_k": 60, "candidates": 100, "limit": 10
}
```

Every member is optional but at least one leg (`text`, `vector`, `graph`) must run. A weight of 0
switches its leg off; a weight left out is 1. The defaults are the values shown. The answer is
`{"hits": [{"id", "score", "ranks": {"keyword", "vector", "graph"}, "unit"}]}`, best first, where a
rank is the 1-based position in that leg or `null`. A deleted unit is never a hit, and the seeds of
the graph leg are never hits of the other legs.

**The vector leg ranks the vectors the request carries**, `metric` being `cosine` (default) or
`dot`. The server keeps no vector index (vectors are caller-owned, derived data in Kerno), so the
caller sends the vectors of the units it wants ranked. This is exact and stateless, and it costs
the request size; a server-side index is future work.

### Errors

Every failure is `{"error": {"code": "...", "message": "..."}}`; the same `code` travels in the
`kerno-error-code` metadata of a gRPC status.

| `code` | HTTP | gRPC | When |
|---|---|---|---|
| `invalid_request` | 400 | `INVALID_ARGUMENT` | A body, id, query string or query that does not parse or fit; an operation the unit's state does not allow (archiving an archived unit). |
| `schema_violation` | 422 | `INVALID_ARGUMENT` | The unit type's schema refuses the write. |
| `unauthenticated` | 401 | `UNAUTHENTICATED` | Tenant mode: no token, or one the server does not know. HTTP adds `WWW-Authenticate: Bearer`. |
| `not_found` | 404 | `NOT_FOUND` | No such unit, relation, schema or route. |
| `method_not_allowed` | 405 | `UNIMPLEMENTED` | The route exists, the method does not (an `Allow` header lists the methods). |
| `revision_conflict` | 409 | `ABORTED` | `expected_revision` is not the unit's revision. |
| `conflict` | 409 | `FAILED_PRECONDITION` | The unit has relations, or is deleted. |
| `payload_too_large` | 413 | `RESOURCE_EXHAUSTED` | A body over 16 MiB, or a payload over the engine's limit (1 MiB). |
| `unavailable` | 503 | `UNAVAILABLE` | The database cannot answer right now. |
| `internal` | 500 | `INTERNAL` | A server failure. The message is fixed; the cause goes to the server's log. |

## gRPC

`proto/kerno/v1/kerno.proto` is the contract: one RPC per HTTP route with the same name and
meaning (`CreateUnit`, `GetUnit`, `ListUnits`, `UpdateUnit`, `ArchiveUnit`, `UnarchiveUnit`,
`DeleteUnit`, `CreateRelation`, `ListRelations`, `DeleteRelation`, `DefineSchema`, `GetSchema`,
`Query`, `Explain`, `Search`, `Health`). Generate a client for any language from the file; the
Rust client is `kerno_server::grpc::pb::kerno_client::KernoClient`. It is HTTP/2, over TLS when
the server has a certificate; in tenant mode every call carries `authorization: Bearer <token>` in
its metadata.

## Limits and what is not here

* A request body is at most 16 MiB. A client has 10 seconds to send its request headers.
* `GET /units` reads every unit of the type to cut a page, so a page costs the whole type.
* A unit's `metadata` and its history (revisions, restore, as-of reads) are not exposed yet, and
  neither are `create_index`/`drop_index` (declare `indexed` in the schema instead).
* Units are deleted logically, as everywhere in Kerno (ADR-0009): the bytes stay until the engine
  compacts.
* Backup and replication through the server are not here yet (back up a stopped tenant's directory).

## Tests

`cargo test -p kerno-server` starts the real binary on free ports for every integration test
(`tests/http.rs`, `tests/grpc.rs`, and `tests/operations.rs` for tokens, tenants, TLS, probes,
metrics and logs) and talks to it over a socket; the error table, the argument parser, the tokens
file and the JSON types have unit tests. `tests/tls/` holds a test-only certificate authority and a
`localhost` certificate it signed (valid until 2126), made once with `openssl`.
