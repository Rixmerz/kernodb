# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Python packages use the PEP 440 spelling of the same version (`1.0.0a1` is `1.0.0-alpha.1`).

## [1.0.0-alpha.1] - 2026-11-19

First public alpha. The on-disk format v1 is frozen (ADR-0013); the API and the wire protocols may still change before 1.0. Expect breaking fixes between alpha releases until the open items below are closed.

### Added

- **Rust crates** (crates.io): `kerno-core`, `kerno-storage`, `kerno-units` and the `kerno` entry-point crate.
- **Storage engine**: crash-safe embedded engine with deterministic format-v1 vectors, checksums, backups and verification.
- **Units, revisions and relations**: history, retention, schemas, indexes, hybrid search (vectors and BM25) and a query language.
- **Bindings and clients**: Node.js (`kernodb`), Python (`kernodb`), and HTTP/gRPC clients for TypeScript (`kerno-client`) and Python (`kerno-client`).
- **Integrations**: Vercel AI SDK (`kerno-ai-sdk`), LangChain (`kerno-langchain`) and LlamaIndex (`kerno-llamaindex`).
- **Tooling**: `kerno-cli`, `kerno-mcp` and `kerno-server` (not published to crates.io in this alpha).
- **Release checklist**: [docs/release/1.0.0-alpha.1-checklist.md](docs/release/1.0.0-alpha.1-checklist.md).

### Known limitations

- **U6 median benchmark pending**: the pre-registered U6 measurement has no median result yet.
- **30-hour fuzz run pending**: the long fuzzing campaign (30 hours in total) has not been completed.
- **External evaluations INT-07 and INT-13 pending**: they need people outside the project.
- **CLA under legal review**: the contributor licence agreement is a draft until a lawyer has reviewed it.
- Nothing is published until the owner runs the steps in the release checklist.

[1.0.0-alpha.1]: https://github.com/Rixmerz/kernodb/releases/tag/v1.0.0-alpha.1
