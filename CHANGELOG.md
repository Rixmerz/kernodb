# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Python packages use the PEP 440 spelling of the same version (`1.0.0a1` is `1.0.0-alpha.1`).

## [1.0.0-alpha.1] - 2026-11-20

First public alpha. The on-disk format v1 is frozen (ADR-0013); the API and the wire protocols may still change before 1.0. Expect breaking fixes between alpha releases until the open items below are closed.

### Added

- **Rust crates** (crates.io): `kernodb-core`, `kernodb-storage`, `kernodb-units` and the `kernodb` entry-point crate. The library names stay `kerno_core`, `kerno_storage`, `kerno_units` and `kerno`, so code imports `kerno::...` (the plain `kerno` name on crates.io belongs to an unrelated project).
- **Storage engine**: crash-safe embedded engine with deterministic format-v1 vectors, checksums, backups and verification.
- **Units, revisions and relations**: history, retention, schemas, indexes, hybrid search (vectors and BM25) and a query language.
- **Bindings** (npm and PyPI): Node.js (`kernodb`) and Python (`kernodb`).
- **Clients and integrations** (source only, not published in this alpha): HTTP/gRPC clients (`kerno-client`) for TypeScript and Python, and integrations for the Vercel AI SDK, LangChain and LlamaIndex.
- **Tooling**: `kerno-cli`, `kerno-mcp` and `kerno-server` (not published to crates.io in this alpha).
- **Release checklist**: [docs/release/1.0.0-alpha.1-checklist.md](docs/release/1.0.0-alpha.1-checklist.md).

### Known limitations

- **U6 median benchmark pending**: the pre-registered U6 measurement has no median result yet.
- **30-hour fuzz run pending**: the long fuzzing campaign (30 hours in total) has not been completed.
- **External evaluations INT-07 and INT-13 pending**: they need people outside the project.
- **CLA under legal review**: the contributor licence agreement is a draft until a lawyer has reviewed it.
- **Schemas experimental**: typed schemas and secondary indexes (ADR-0021) are experimental; their on-disk records may change before 1.0, and a database that defines a schema needs this release or newer to be opened.
- **Windows experimental**: the engine builds and its tests run on Windows, but `Durability::Sync` is verified on Linux only. Lower-level APIs refuse it elsewhere unless `allow_unverified_platform` is set; `kerno::Db` opts in.

[1.0.0-alpha.1]: https://github.com/Rixmerz/kernodb/releases/tag/v1.0.0-alpha.1
