# Running kerno-server

`kerno-server` serves Kerno databases over an HTTP JSON API and, when asked, gRPC. This page is
about running it: the two modes, API tokens, tenants, TLS, health checks, metrics and logs. The
routes, payloads and error codes are in the [crate README](../crates/kerno-server/README.md);
`crates/kerno-server/proto/kerno/v1/kerno.proto` is the gRPC contract.

Kerno is an embedded engine first (like SQLite). The server is for when several processes or
machines need the same memory; it does not change what the engine guarantees.

## Two modes

| Mode | Command | Who may call |
|---|---|---|
| One database | `kerno-server --db <dir>` | Anyone who reaches the port. No token. |
| Tenants | `kerno-server --data-dir <dir> --tokens <file>` | Only a request with a listed token, and only on its own tenant's database. |

**`--db` has no authentication.** It is for development and for a server only its own machine can
reach: it listens on `127.0.0.1:8080` unless told otherwise, and it logs a warning when it listens
anywhere else. Use tenant mode for anything on a network, even with one tenant.

The two modes do not mix: `--db` with `--data-dir` is refused, and so is `--tokens` without
`--data-dir`. `KERNO_SERVER_DB` stands for `--db` and is ignored in tenant mode.

`KERNO_SERVER_HEADER_TIMEOUT_MS` (default 10000) is how long a client may take to send its request
line and headers or to finish a TLS handshake. It exists so that the tests do not wait ten seconds.

### All flags

| Flag | Meaning |
|---|---|
| `--db <dir>` | One database, no authentication. Created if missing. |
| `--data-dir <dir>` | One database per tenant, in `<dir>/<tenant>`. Created if missing. Needs `--tokens`. |
| `--tokens <file>` | The API tokens (below). Needs `--data-dir`. |
| `--listen <addr>` | The HTTP address. Default `127.0.0.1:8080`; port `0` picks a free one. |
| `--grpc-listen <addr>` | Also serve gRPC here. Off by default. |
| `--tls-cert <pem>`, `--tls-key <pem>` | Serve both transports over TLS (below). Both or neither. |
| `--fulltext <types>` | Unit types to keyword-index (BM25) in every database served, comma-separated. Off by default. |
| `--backup <url>` | Keep a base backup and ship the write-ahead log of every database to `file:///dir` or `s3://bucket/prefix` (a tenant's backups go under its name). Off by default; see [backup.md](backup.md). |
| `--backup-interval <s>` | Seconds between shipments, a whole number of 1 or more. Default 5. Needs `--backup`. |

On start the server prints `listening on http://ADDR` (`https://` with TLS) and, with gRPC,
`grpc listening on ADDR` to **stdout**, with the real port. Everything else goes to stderr as JSON
logs. A bad flag, an unreadable tokens file or certificate, or a database that will not open
stops it before it listens, with one plain-text line on stderr that says why, and exit code 1.

On SIGINT or SIGTERM it stops accepting, lets the requests in flight finish (HTTP waits at most
10 seconds), closes every database and exits 0.

## API tokens

A token is an opaque string of at least 32 characters. The server never stores it: the tokens
file holds the token's SHA-256, so a leaked file lets nobody in.

Make a token and its line:

```sh
token=$(openssl rand -hex 32)                    # give this to the client, keep it secret
echo "$token" | kerno-server hash-token          # prints the SHA-256 as 64 hex digits
echo "$(echo "$token" | kerno-server hash-token) acme" >> tokens
```

The tokens file has one `<sha256 hex> <tenant>` per line. Blank lines and lines starting with `#`
are skipped:

```text
# acme: the indexer and the chat app
3f4c...e1a0 acme
9b02...77cd acme
# globex
c1d8...0b4e globex
```

A tenant is 1 to 64 of `a-z`, `0-9`, `_` and `-`; it is the name of its directory under
`--data-dir`, so nothing else is accepted (no `/`, no `.`, no capitals). A tenant may have several
tokens, which is how a token is rotated: add the new line, restart, move the clients, remove the
old line, restart. The file is read once at start; a malformed line, a hash listed twice or a
file with no tokens stops the server and names the line.

### Sending the token

HTTP: `Authorization: Bearer <token>` (the scheme is case-insensitive). gRPC: the same value in
the `authorization` metadata. Every `/v1` route and every RPC needs it, including `Health`.

A request with no token, another scheme, an empty token or an unknown one gets one answer, so a
caller learns nothing by probing:

* HTTP `401` with `WWW-Authenticate: Bearer` and
  `{"error": {"code": "unauthenticated", "message": "a valid API token is required: ..."}}`;
* gRPC `UNAUTHENTICATED` with `kerno-error-code: unauthenticated`.

The token is checked first, before the route, the method or the body, so an unauthenticated
caller cannot tell which routes exist either.

## Tenant isolation

Every tenant named in the tokens file has its own Kerno database in `<data-dir>/<tenant>`, opened
at start: its own files, its own write-ahead log, its own lock. A request reaches the database of
its token's tenant and no other. No route, query parameter or message field names a tenant, so
there is no way to ask for another one: a unit id of another tenant is simply `not_found`, a list
or a search sees only the caller's units, and a relation cannot point into another tenant.

Isolation is by directory, not by row filter: a bug in a query cannot leak across tenants, and a
tenant is backed up, moved or deleted as a directory (with the server stopped).

A new tenant needs a restart. One server holds a data directory at a time: a second one on the
same directory fails to open the first tenant and exits.

## TLS

`--tls-cert` takes the certificate chain (leaf first) and `--tls-key` the private key, both PEM;
the key may be PKCS#8, PKCS#1 (RSA) or SEC1 (EC). Both transports then speak TLS only (TLS 1.2 and
1.3, rustls with the `ring` provider): a plaintext client gets no answer. There is no client
certificate authentication; tokens do that job.

Without TLS, a token crosses the network in clear text. The server logs a warning when it holds
tokens and listens beyond loopback without TLS. Terminating TLS at a reverse proxy in front of a
loopback-only server is fine too.

The certificate is read at start; renewing it needs a restart.

## Health checks

Neither probe needs a token, and neither names a tenant or a directory.

| Route | Answers | Use it for |
|---|---|---|
| `GET /livez` | Always `200 {"status": "alive"}` while the process serves HTTP. | Liveness: restart the process when it fails. |
| `GET /readyz` | `200 {"status": "ready"}` when every database's directory is still there and the database answers a read; else `503` with code `unavailable`. | Readiness: send traffic only when it passes. |
| `GET /v1/health` | `{"status": "ok", "version"}` for the caller's own database. Needs a token in tenant mode. | A client checking its own connection. |

A volume unmounted or a directory removed under a running server makes `/readyz` fail while
`/livez` keeps passing; the log line names the directory. gRPC has `Health`, which is the
`/v1/health` of the transport and needs a token; probes should use HTTP.

## Metrics

`GET /metrics` serves the Prometheus text format (`text/plain; version=0.0.4`) without a token.
No label names a tenant, a token or a path.

| Metric | Type | Meaning |
|---|---|---|
| `kerno_http_responses_total{status}` | counter | HTTP answers by status code, the probes and `/metrics` included. |
| `kerno_grpc_calls_total{code}` | counter | gRPC calls by error code (`ok`, `not_found`, `unauthenticated`, ...; the codes of the README's error table). |
| `kerno_auth_failures_total` | counter | Requests refused for a missing or unknown token, on either transport. |
| `kerno_request_duration_seconds{transport}` | histogram | Time to answer, `transport` being `http` or `grpc`. Buckets from 1 ms to 5 s. |
| `kerno_databases_open` | gauge | Databases the server holds open (the number of tenants, or 1). |

A scrape is counted after it is answered, so it shows on the next one. Counters start at zero
with each process.

## Logs

The server logs one JSON object per line to stderr, through `tracing`:

```json
{"timestamp":"2026-11-18T18:12:09.984458Z","level":"INFO","fields":{"message":"request","transport":"http","method":"POST","path":"/v1/units","status":201,"tenant":"acme","duration_us":694},"target":"kerno_server::http"}
```

| `message` | Level | Fields |
|---|---|---|
| `listening` | INFO | `http`, `grpc`, `tls`, `authentication`, `databases` |
| `tenant database open` | INFO | `tenant` |
| `request` | INFO | HTTP: `transport`, `method`, `path`, `status`, `tenant` (tenant mode, once the token is accepted), `duration_us`. gRPC: `transport`, `method` (the RPC), `code`, `duration_us`. |
| `request refused` | WARN | `reason`: `no bearer token` or `unknown token` |
| `the database directory is gone` | WARN | `dir` |
| `internal error` | ERROR | `detail`: the cause the client was not sent |
| `stopping` | INFO | |

**A token never reaches a log, not even a prefix.** Paths hold unit ids and unit types; payloads
and query text are never logged. Only the startup lines on stdout and a fatal error line are not
JSON.

## Deploying

A typical setup: the server on loopback or a private network, tenant mode, TLS, a process
manager that sends SIGTERM, `/readyz` as the readiness probe, `/livez` as the liveness probe, and
`/metrics` scraped by Prometheus.

```sh
kerno-server --data-dir /var/lib/kerno --tokens /etc/kerno/tokens \
  --listen 0.0.0.0:8443 --grpc-listen 0.0.0.0:50051 \
  --tls-cert /etc/kerno/tls/fullchain.pem --tls-key /etc/kerno/tls/key.pem
```

Keep the tokens file and the key readable only by the server's user.

## Not here yet

* Reloading tokens or certificates without a restart.
* Per-tenant quotas and rate limits.
* Backup, restore and replication through the server (back up a stopped tenant's directory).
* Read-only or per-route token scopes: a token can do everything on its tenant.
* A timeout on a request body once its headers are in: put a reverse proxy in front if slow
  clients are a concern.
