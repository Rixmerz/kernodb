# Deploying kerno-server with Docker

This page runs [`kerno-server`](server.md) from the container image in the repository's
[`Dockerfile`](../Dockerfile), with the Compose files in [`deploy/`](../deploy). What the server's
flags, routes and metrics mean is in [server.md](server.md); backups are in [backup.md](backup.md).
This page is about putting them in a container: what the image holds, where the data lives, how
the secrets get in, and what to watch.

Nothing here is published: the repository builds the image, it does not push it to any registry.
Build it yourself (or have your CI do it) and push it where you keep images.

## The image

```sh
docker build -t kerno-server .      # from the repository root; 3 to 6 minutes
```

| | |
|---|---|
| Build | Two stages, `rust:1-bookworm` then `debian:bookworm-slim`, from `Cargo.lock` (`--locked`). |
| Contains | `kerno-server` (the entry point) and `kerno`, the command-line tool, used for restores. |
| User | `kerno`, uid and gid **10001**. Never root. |
| Ports | `8080` (HTTP) and `50051` (gRPC); gRPC is off unless you pass `--grpc-listen`. |
| Volume | `/data`, the working directory. `/backups` exists too, owned by the same user. |
| Default command | `--data-dir /data --tokens /etc/kerno/tokens --listen 0.0.0.0:8080`: tenant mode. |
| Health check | `GET /readyz` every 15 s, over plain HTTP or else TLS (it tries both). |
| Stop | SIGTERM: in-flight requests finish (10 s at most), a last backup is shipped, the databases close, exit 0. |

**The default command needs a tokens file and refuses to start without one.** That is on purpose:
`--db` serves a database to anyone who reaches the port, and a container's port is reachable by
whatever is on its network. Use `--db` in a container only for a throwaway test, with the port
published on `127.0.0.1`.

If you change the port in `--listen`, set `KERNO_HEALTH_PORT` to it, or the health check probes
the wrong port and the container turns `unhealthy`.

## Quick start with Compose

Needs Docker Compose 2.24 or later (the TLS file uses `!override`).

```sh
cd deploy
docker compose build

# 1. A token for the tenant "acme". The client gets $token; the server only ever sees its hash.
token=$(openssl rand -hex 32)
echo "$token" | docker compose run --rm --no-deps -T kerno hash-token \
  | sed 's/$/ acme/' > tokens
sudo chown 10001 tokens && chmod 400 tokens        # the image's user must be able to read it

# 2. Start it.
docker compose up -d
docker compose ps                                  # STATUS says (healthy) after a few seconds

# 3. Use it.
curl -H "Authorization: Bearer $token" -d '{"type":"note","payload":{"text":"hello"}}' \
  -H 'Content-Type: application/json' http://127.0.0.1:8080/v1/units
```

[`deploy/docker-compose.yml`](../deploy/docker-compose.yml) runs one service, `kerno`, with a
volume for the databases, a volume for the backups, the tokens file mounted read-only, and both
ports published on the host's **loopback** only. `./tokens` is in `deploy/.gitignore`; so are
`./tls/` and `.env`. Never commit any of them.

## API tokens

Tokens work as in [server.md](server.md#api-tokens): a token is at least 32 characters, the tokens
file holds its SHA-256 and a tenant per line, and every tenant has its own database under `/data`.

- **Hash a token inside the image**, as above: `kerno-server hash-token` reads the token on stdin.
  Pass `-T` to `docker compose run` so it does not allocate a terminal.
- **The file is read once at start.** Add a tenant or rotate a token by editing the file and running
  `docker compose restart kerno`. A bind-mounted file edited in place (`>>`) is seen by the
  container; one replaced by an editor that writes a new file is not, until the container is
  recreated: `docker compose up -d --force-recreate`.
- **Ownership.** The server runs as uid 10001, so a bind-mounted tokens file must be readable by
  it: `chown 10001 tokens; chmod 400 tokens`. If it is not, the container exits at once with
  `cannot read the tokens file ...` in `docker compose logs kerno`.
- **Other ways to hand over the file** are your orchestrator's secret store: a Docker or Compose
  secret, a Kubernetes `Secret`, mounted at `/etc/kerno/tokens` (or anywhere, with `--tokens`).
  The hashes are not credentials (a leaked file lets nobody in), but keep the file private anyway.

## TLS

Without TLS, a token crosses the network in clear text. The published ports are bound to the host's
loopback for that reason. There are two ways to serve beyond it:

**A. TLS in the server.** Put the certificate chain (leaf first) at `deploy/tls/fullchain.pem` and
the private key at `deploy/tls/key.pem`, both readable by uid 10001 (`chown 10001 tls/*; chmod 400
tls/key.pem`), then:

```sh
cd deploy
docker compose -f docker-compose.yml -f docker-compose.tls.yml up -d
```

[`docker-compose.tls.yml`](../deploy/docker-compose.tls.yml) mounts `./tls` read-only, adds
`--tls-cert` and `--tls-key`, moves HTTP to port 8443, sets `KERNO_HEALTH_PORT=8443` and publishes
`8443` and `50051` on every interface. HTTP and gRPC then speak TLS only (1.2 and 1.3); a plaintext
client gets no answer. The certificate is read at start: renew it, then `docker compose restart
kerno`.

The health check does not verify the certificate (it connects to `127.0.0.1`, which a certificate
is not issued for); it only asks the server whether it is ready.

**B. TLS at a reverse proxy** (Caddy, nginx, a cloud load balancer) on the same private network,
speaking plain HTTP to `kerno:8080`. Keep the container's ports unpublished (or on loopback) so the
proxy is the only way in. gRPC through a proxy needs one that speaks HTTP/2 to the backend.

## Volumes and data

| Mount | Holds | Notes |
|---|---|---|
| `/data` | One directory per tenant: its database files and write-ahead log. | **This is the data.** A named volume (`kerno-data`) is the default. |
| `/backups` | The Compose file's `file://` backup location. | Only used with `--backup=file:///backups`. |
| `/etc/kerno/tokens` | The tokens file. | Read-only bind mount. |
| `/etc/kerno/tls` | Certificate and key. | Read-only bind mount; TLS file only. |

- **A named volume takes the ownership of the directory in the image** on first use, so it works for
  uid 10001 with nothing to do. **A bind mount does not**: `chown 10001:10001` the host directory
  first, or the server fails to open its databases with a permission error.
- **One server per data directory.** The engine takes an exclusive lock on each database: a second
  container on the same volume fails to open the first tenant and exits. Do not scale `kerno` above
  one replica, and do not mount `/data` into another container to look at it while the server runs.
  This also means a rolling update must stop the old container before the new one starts
  (`docker compose up -d` does).
- **A snapshot of the volume taken while the server runs** is a crash-consistent copy, the same
  as a power cut: the engine recovers it from its write-ahead log. For anything better use the
  backups below.
- `docker compose down` keeps the volumes; `docker compose down -v` **deletes them**, data and
  backups both.

## Backups

The Compose file starts the server with `--backup=file:///backups`: it takes a base backup at
start, ships the write-ahead log every 5 seconds, and ships once more on a clean stop. Each
tenant's backups go under `/backups/<tenant>`. How it works and what it does not promise (the
recovery point is the interval, not zero) is in [backup.md](backup.md).

**A backup volume on the same disk as the data is a copy, not a safeguard.** For production put it
somewhere else: S3, R2 or MinIO through the environment (credentials are never flags, so they never
reach a process list or a log):

```yaml
    command:
      - --data-dir=/data
      - --tokens=/etc/kerno/tokens
      - --listen=0.0.0.0:8080
      - --backup=s3://my-bucket/kerno
    environment:
      KERNO_S3_REGION: us-east-1
      # R2: KERNO_S3_REGION=auto and KERNO_S3_ENDPOINT=https://<account-id>.r2.cloudflarestorage.com
      # MinIO or a private CA: KERNO_S3_ENDPOINT=https://minio:9000, KERNO_S3_CA_FILE=/etc/kerno/ca.pem
    env_file: .env       # AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY; .env is in deploy/.gitignore
```

The image carries the system root certificates, so an `https://` endpoint with a public CA works as
it is.

Other things to do:

- **Take a base now**, for instance before an upgrade: `POST /v1/backup` with the tenant's token
  answers 201 and the backup's description.
- **Watch it** with the `kerno_backup_*` metrics below. A store that is down is logged and retried at
  every tick; it never stops the server, so only the metrics and logs tell you.
- **Retention** is yours: nothing deletes old bases and segments. Use the bucket's lifecycle rules.

### Restoring

A restore builds a **new** database from the store; it never writes over one. With the server
stopped, build the tenant's database next to the old one, swap them, and move the old backup chain
aside: the restored database has a new identity, so the old chain refuses it (see
[backup.md](backup.md#restoring)) and it needs a prefix of its own.

```sh
cd deploy
docker compose stop kerno
docker compose run --rm --no-deps --entrypoint sh kerno -c '
  set -e
  kerno /data/acme.restored restore file:///backups/acme      # or: --to-seq N, --to-time MS
  mv /data/acme /data/acme.damaged
  mv /data/acme.restored /data/acme
  mv /backups/acme /backups/acme.before-restore'
docker compose up -d
```

For S3, run the same `kerno ... restore s3://my-bucket/kerno/acme` with the same environment as the
server and give the restored tenant a new prefix (change `--backup` to `s3://my-bucket/kerno-2`, or
copy the old chain aside in the bucket). Delete `acme.damaged` once you have checked the result.
`kerno` is in the image for exactly this.

## Health checks and metrics

Neither the probes nor `/metrics` need a token, and none names a tenant (see
[server.md](server.md#health-checks)).

| Probe | Use |
|---|---|
| `GET /livez` | Liveness: restart the container when it fails. |
| `GET /readyz` | Readiness: the image's own `HEALTHCHECK` uses it. It fails with `503` when a database directory is gone (a volume unmounted under a running server) while `/livez` keeps passing. |

In Kubernetes, point `livenessProbe` and `readinessProbe` at those two paths on port 8080 (`scheme:
HTTPS` with TLS).

### Scraping with Prometheus

`GET /metrics` serves the Prometheus text format. The metric list is in
[server.md](server.md#metrics); with `--backup` it adds `kerno_backup_shipped_commits_total`,
`kerno_backup_segments_total`, `kerno_backup_bases_total` and `kerno_backup_errors_total`.

Prometheus must reach the container on the Compose network, so scrape `kerno:8080` from a
Prometheus in the same Compose project (or publish the port to it). A `prometheus.yml` job:

```yaml
scrape_configs:
  - job_name: kerno
    metrics_path: /metrics
    scrape_interval: 15s
    static_configs:
      - targets: ["kerno:8080"]
    # With TLS in the server (docker-compose.tls.yml): targets "kerno:8443", and
    # scheme: https
    # tls_config: { ca_file: /etc/prometheus/ca.pem, server_name: kerno.example.com }
```

`/metrics` has no authentication. The ports in the Compose file are published on loopback, and
`/metrics` shows counts only (no tenant, token or path), but do not publish it to the internet:
firewall it, or let only the proxy that terminates TLS expose `/v1`.

Two alerts that earn their keep:

- `increase(kerno_backup_errors_total[10m]) > 0`: backups are failing (only with `--backup`).
- `rate(kerno_auth_failures_total[5m]) > 1`: something is probing or a client lost its token.

### Logs

The server writes one JSON object per line to stderr, which Docker collects: `docker compose logs
-f kerno`. Fields are in [server.md](server.md#logs); a token never reaches a log. Put a size limit
on the logging driver (`logging: { driver: json-file, options: { max-size: 10m, max-file: "5" } }`),
since Docker keeps them unbounded by default.

## Upgrading

```sh
git pull && cd deploy && docker compose build && docker compose up -d
```

Take a base backup first (`POST /v1/backup`), and read the release notes: the on-disk format is
versioned, and an engine that reads a format it does not know says so instead of guessing. The old
container is stopped (and ships its last commits) before the new one starts.

## What is not here

- **Multiple replicas or failover.** One server owns a data directory.
- **A published image.** Nothing pushes to a registry.
- **Reloading tokens or certificates without a restart** (see [server.md](server.md#not-here-yet)).
- **A Kubernetes manifest or Helm chart.** The probes, the volume, the user and the signal are all
  described above, which is what one needs.
