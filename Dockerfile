# kerno-server as a container image. Build from the repository root:
#   docker build -t kerno-server .
# How to run it: docs/deployment.md; a ready compose file: deploy/docker-compose.yml.

# ---- build ------------------------------------------------------------------------------------
# Debian 12 on both stages, so the binary links the glibc it runs on. `rust:1` is always new
# enough for the workspace's rust-version.
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
# --locked: the image builds exactly the dependencies of Cargo.lock, or fails.
# ponytail: no dependency-only layer, so any source change rebuilds everything (about 3-6 minutes);
# add cargo-chef or BuildKit cache mounts if image builds become frequent.
# `kerno`, the command-line tool, comes along for restores: the server ships backups and
# `kerno restore` rebuilds a database from them.
RUN cargo build --release --locked -p kerno-server -p kerno-cli \
    && install -D target/release/kerno-server target/release/kerno -t /out

# ---- runtime ----------------------------------------------------------------------------------
FROM debian:bookworm-slim
# ca-certificates: the S3 backup client trusts the system roots. curl: the HEALTHCHECK.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --user-group --no-create-home --shell /usr/sbin/nologin kerno \
    && mkdir /data /backups \
    && chown kerno:kerno /data /backups
COPY --from=build /out/kerno-server /out/kerno /usr/local/bin/

# Never root. A named volume mounted on /data or /backups takes this ownership on first use; a
# bind mount must be owned by uid 10001 (docs/deployment.md).
USER 10001:10001
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8080 50051

# Readiness, not just liveness: /readyz also fails when a database directory is gone. It needs no
# token. The server speaks plain HTTP or TLS, never both on one port, so try the one and then the
# other; -k because the certificate is not made for 127.0.0.1. Set KERNO_HEALTH_PORT when
# --listen uses another port.
HEALTHCHECK --interval=15s --timeout=10s --start-period=20s --retries=3 \
    CMD curl -fsS --max-time 4 "http://127.0.0.1:${KERNO_HEALTH_PORT:-8080}/readyz" >/dev/null \
     || curl -fskS --max-time 4 "https://127.0.0.1:${KERNO_HEALTH_PORT:-8080}/readyz" >/dev/null

# SIGTERM (the default stop signal) drains requests and closes the databases cleanly.
ENTRYPOINT ["kerno-server"]
# Tenant mode on every interface, because inside a container loopback is unreachable. It
# needs a tokens file mounted at /etc/kerno/tokens: without one the server refuses to start rather
# than serve a database open to anyone. Publish the port on loopback or behind TLS (see the docs).
CMD ["--data-dir", "/data", "--tokens", "/etc/kerno/tokens", "--listen", "0.0.0.0:8080"]
