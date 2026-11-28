# 1.0.0-alpha.5 pre-release validation

- Commit validated: 2bf5fef (main), plus the release commit that follows it
- Date: 2026-11-28
- Gate items: 2 (pre-release suite), 3 (outside-style boundary and conformance passes), 4 (independent review) of [RELEASING.md](../../RELEASING.md)

## Test suites

| Suite | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test -p kernodb-storage -p kernodb-units -p kernodb --all-features --release` | 1994 passed, 0 failed, 5 ignored |
| other crates (kerno-server, kerno-testkit, kerno-cli, kerno-mcp, kerno-backup, kernodb-core) | 474 passed |
| Python binding (`pytest`) | 237 passed |
| Node binding (`pnpm test`) | 146 passed |
| `clients/typescript` | 47 passed |
| `clients/python` | passes against a real server |

## Independent review of 6e0c689..870e7c2

A reviewer who did not write the code read every change between the alpha.4 release (6e0c689) and 870e7c2. No data-loss path was found. Findings, all fixed:

- backpressure for log-less bulk commits (a665df1) and for large commits that write the WAL (af9d943);
- covering lookups verify when reads verify (b5195b3);
- only a batch limit maps to `ChangeSetTooLarge` (cb25d61, 296ed26);
- argument checks for `set_read_cache_bytes` and `bulk_without_wal` (4067413);
- documentation corrections (be5ccec).

## Boundary inputs

About 2,600 hostile calls across both bindings: negative numbers, 0, 2**63, 2**64, NaN and infinity, empty values, NUL bytes, lone surrogates, 10 MB values, over-long names, wrong types, bad paths and closed handles. No crash, hang or panic. Fixed:

- Node `runRetention` silently ignored a `byType` or `default` that is not an object (df4ef95);
- a NUL byte in the `open` path (fe539a7).

## Conformance

About 190 documented examples were run and about 150 claims were checked against HEAD. Eight documentation mismatches were fixed (5fa4a6d, 2bf5fef).

## Tag workflow finding

The tag workflow found both musl Python wheels (x86_64 and aarch64) broken before anything was published: they failed to import on Alpine with `initial-exec TLS resolves to dynamic definition` (mimalloc v3 uses initial-exec TLS, which musl refuses in a dlopen'ed shared object). Fixed by enabling mimalloc's `local_dynamic_tls` on musl targets only; the rebuilt x86_64 musl wheel passes the full suite (237) in Alpine.

## Performance note

The Python binding keeps its mimalloc arena off transparent huge pages (870e7c2). On a host with THP set to `always`, the `requests` ingest on btrfs takes 27.1 ms against SQLite's 34.7 ms (median, 16 of 16 runs); it was 45.2 ms before.

## Known limitations

- Python wrong-type and lone-surrogate arguments raise the right class (`InvalidArgumentError`) but the message does not name the argument.
- An I/O error while opening a path surfaces as the generic `KernoError` / `KERNO`.
- Node `new Db()` throws an error without a code.
- Importing an export produced by an alpha.4 build into alpha.5 was not tested.
- L0 can overshoot the stop limit by a bounded amount during a large frame write.
- On tmpfs (no sync cost) SQLite is still faster on the small `requests` ingest.
