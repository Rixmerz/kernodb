# Image for the long fuzzing campaign of kerno-storage (OPS-13; docs/testing/fuzz-long-run.md).
# Build from the repository root:
#   docker build -f docker/fuzz/Dockerfile -t kerno-fuzz .
# docker/fuzz/run.sh builds it when it is missing.
FROM debian:bookworm-slim

# Pinned: the nightly date and the cargo-fuzz version. Change both on purpose, never by accident.
ARG NIGHTLY=nightly-2026-11-01
ARG CARGO_FUZZ=0.13.2

# build-essential and clang: libfuzzer-sys compiles libFuzzer's C++. procps gives `uptime`,
# util-linux `taskset`.
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential clang ca-certificates curl procps util-linux \
 && rm -rf /var/lib/apt/lists/*

# World-writable on purpose: run.sh starts the container as the host user, so the files in
# the results directory belong to that user. The default user below is for a plain `docker run`.
ENV RUSTUP_HOME=/opt/rust/rustup \
    CARGO_HOME=/opt/rust/cargo \
    PATH=/opt/rust/cargo/bin:$PATH \
    HOME=/tmp \
    TOOLCHAIN=
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "$NIGHTLY" \
 && cargo install --locked cargo-fuzz --version "=$CARGO_FUZZ" \
 && chmod -R a+rwX /opt/rust

WORKDIR /work
COPY . /work

# Build every fuzz target now, so a run starts with a warm target directory.
RUN cd crates/kerno-storage && cargo fuzz build \
 && chmod -R a+rwX /work

RUN useradd --create-home --uid 10001 fuzz
USER fuzz
WORKDIR /work/crates/kerno-storage
CMD ["cargo", "fuzz", "list"]
