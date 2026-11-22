# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Python packages use the PEP 440 spelling of the same version (`1.0.0a1` is `1.0.0-alpha.1`).

## [Unreleased]

### Added

- **Docs for the facade and the bindings** (#2, section 5): [`docs/bindings-coverage.md`](docs/bindings-coverage.md) lists every public call of the `kerno` facade and whether the Python and Node bindings expose it, under which name (gaps: `drop_index`, `schema`, `search_text` in both; `find_range`, `explain` and `compare` in Node). Guides: [Kerno as a revisioned history next to another database](docs/guides/history-alongside.md) and [one unit per entity, keyed by a logical key](docs/guides/one-unit-per-entity.md). The binding READMEs now say that a read-only open takes no lock.

- **`refresh` on every public surface** (#2, a): a shared read-only open sees the commits a writer made after it opened by calling `refresh()`. `KernoDb::refresh` is now reachable through `UnitDb::refresh`, `kerno::Db::refresh`, Python `db.refresh()` and Node `db.refresh()`; each returns the commit sequence it now reads and fails on a handle that is not read-only. New `kerno_core::contract::Refresh` trait; `UnitDb::refresh` also drops the cached history policies, which the writer may have changed.

- **Retention from every public surface** (#2, b): `Db::run_retention(&RetentionPolicy)` in the `kerno` facade, `db.run_retention(default=, by_type=)` in Python and `db.runRetention({ default, byType })` in Node prune old revisions by rule per unit type: `keep_last` (the newest N), `keep_for` (younger than a number of seconds) or `keep_since` (what answers as-of reads from a commit sequence on). Afterwards `history` from a pruned revision, `read_as_of` before the oldest kept one and `compare` answer `PRUNED` and `verify` stays clean. The policy is a parameter of the call and is not stored in the database. New `UnitDb::run_retention_all`, which runs passes until one completes and adds up their reports (it resumes each pass where the previous stopped, so a `KeepFor` policy finishes on a database with more than 4,096 units to skip). `RetentionPolicy`, `RetentionRule` and `RetentionReport` are re-exported by `kerno`. The bindings refuse a bad rule (a fraction, a negative, NaN, a value of the wrong type) with `ValueError` in Python and `INVALID_ARGUMENT` in Node, and write nothing; a `None` / `null` rule is the same as no rule. Node `db.history(id, { fromRevision })` lists from a revision on and throws `PRUNED` below the oldest kept one, as Python's `from_revision` does.

- **Upsert by a unique field** (#2, natural keys): ids are random, so a client kept its own map from a logical key to the Kerno id. `ChangeSet::upsert` (`kerno-units`), `Batch::upsert` and `Db::upsert` / `Db::upsert_document` (`kerno`), Python `db.upsert(type, field, document)` and the `("upsert", type, field, document)` commit op, and Node `db.upsert(unitType, field, document)` and the `{ op: 'upsert', ... }` commit op find the active unit whose unique `field` equals the document's value and revise it, or store a new unit when none holds it. The result says which unit and whether it was `created`, `updated` or `unchanged` (an identical document writes no revision). The upsert runs in the same change set as the rest of the batch, and several upserts of one key in a batch make one unit (the last document wins). A field the schema does not declare `unique`, or a type with no schema, is refused with `InvalidArgument` (`InvalidArgumentError` in Python, `INVALID_ARGUMENT` in Node); a document without a value in the field is a `SchemaViolation`. `Committed::upserts` (and `upserts` in the Python `detailed` dict and the Node report) lists what each upsert did; an upserted unit is not in `units`, so it does not count for `Endpoint::Stored`. `kerno_units::upsert_key` gives the index encoding of a payload's key.

### Fixed

- **The Windows binaries start** (#7): `kerno-mcp.exe --db <dir>` exited with `invalid argument: durability not verified on this platform`, and so did every `kerno` command, because they opened with the default `Sync` durability, which the engine refuses off Linux unless `allow_unverified_platform` is set. `kerno-mcp`, `kerno` and `kerno-server` now open with `Sync` on a platform where it is not verified and say so once: a warning line on stderr (`kerno` warns only for a command that writes), a JSON log line for `kerno-server`. `KERNO_REQUIRE_VERIFIED_DURABILITY=1` makes them refuse to open instead. New `kerno_storage::PlatformPolicy` holds the decision (a unit test runs it for every platform on Linux), with `REQUIRE_VERIFIED_ENV` and `UNVERIFIED_WARNING`. Documented in the three READMEs, the `--help` of each binary and `docs/compatibility.md` §2.4.
- **The CLI test target compiles on Windows** (#8): the test of a non-Unicode argument built it with `std::os::unix::ffi::OsStrExt`; it now builds a byte that is not UTF-8 on Unix and an unpaired UTF-16 surrogate on Windows.
- **The backup recovery test runs on Windows** (#9): it killed the server with the Unix `kill` command; it now uses `Child::kill` (SIGKILL on Unix, TerminateProcess on Windows).
- **The readiness tests run on Windows** (#10): they renamed a database directory the server held open, which Windows refuses. The tests now serve the database through a link (a symlink on Unix, a directory junction on Windows) and remove it, which keeps what they assert: `/readyz` 503 while the directory is gone, `/livez` 200.
- **Windows backup URLs and the binaries' tests** (Windows CI run 44965725461): `open_store` takes `file://C:\dir` and `file://C:/dir` on Windows (a drive path written straight after `file://`) as well as `file:///C:/dir`; a `file://` URL that is not an absolute path is still refused. The `kerno` tests built backup URLs as `file://` + the native path, which is not a URL on Windows; they now build `file:///C:/...`. The `kerno-server` tests that read the whole stderr or counted `WARN` lines no longer trip over the durability warning that every start logs off Linux.
- **CI**: the Windows job now also runs the tests of `kerno-cli`, `kerno-mcp` and `kerno-server`, so a regression in those targets shows up there.

## [1.0.0-alpha.2] - 2026-11-21

Second public alpha. **It does not open databases written by 1.0.0-alpha.1** (export with alpha.1, import with alpha.2), and the keyword index is opt-in per type; see the upgrading notes in `docs/compatibility.md`.

### Added

- **`kerno` facade**: `Batch::relate` / `relate_unique` (an end can be a unit stored earlier in the same batch, `Endpoint::Stored`), `delete_cascade`, `update_if_changed`, `archive`, `unarchive`, `restore`; `Db::commit_with` (provenance — reason, actor, source — and a `Committed` report with relations, skipped updates and the commit sequence), `Db::last_commit_seq`, `Db::read_as_of` (a sequence past the last commit reads now instead of failing), `Db::compact`, `Db::relations` (every relation of a type in one scan). `UnitError::is_corruption`.
- **Python bindings, second round** (from the LiveSpec integration feedback, #2): every op above in `commit`, `update(if_changed=)`, `delete(cascade=)`, `relate(unique=)`, `last_commit_seq`, `relations`, `compact`, schemas, indexes, documents, `find`, `execute`, `open(read_only=)`, and typed exceptions (`NotFoundError`, `LockedError`, `CorruptionError`, ... all subclasses of `KernoError`). `history` of an unknown id returns `[]` and `read_as_of` returns `None`, like `read`.

- **Python packaging and docs**: the README (the PyPI page) is rewritten: install from PyPI, concepts, common tasks, full API, errors, performance advice, durability, concurrency and limits. It no longer says the package is unpublished. `SchemaError` is added for schema refusals. PyPI metadata gains Homepage, Documentation, Changelog and Issues links and OS classifiers. `wheels.yml` builds musllinux x86_64 and aarch64 wheels and tests them in Alpine.
- **Performance report**: `docs/testing/python-vs-sqlite.md` compares Kerno and SQLite from Python with the same durability, and profiles the write path. Full-text indexing of every UTF-8 payload is the largest single cost (about 7x on writes, 3.5x on disk); a per-type opt-out is proposed for an owner decision. The benchmark script is `bindings/python/bench/vs_sqlite.py`.

- **Format cut for 1.0.0-alpha.2** (ADR-0023). **Breaking:** an alpha.1 database is refused (`LayerTooOld`); export it with alpha.1 (`kerno <dir> export`) and import it with alpha.2. Every database is created at units layer 7.
- **Keyword index is opt-in per type** (ADR-0023, units-v7): no type is BM25-indexed unless `Db::set_fulltext(type, true)` / Python `db.set_fulltext(type, True)` says so. The setting is stored in the database; switching a type with units indexes or clears them in the same commit, so it also works after an import. `verify_units` expects index entries only for indexed types. Measured on 9,000 JSON payloads: writes 4–6x faster and the store 3.5x smaller for types that are not indexed.
- **LZ4 block compression** (ADR-0023 step 3.3, v1.md §4.4): SST data blocks are stored LZ4-compressed when that makes them smaller, marked by the `incompat` feature bit `BLOCK_LZ4` (bit 1) on each such SST; the block encapsulation is unchanged, so the engine format stays v1. `Config::block_compression`, on by default. Readers handle both kinds of table. A real store (LiveSpec's history of the kernodb repository) went from 66 MB to 35 MB with no measurable cost in write time. The spec decoder of the tests decodes it with its own LZ4 decoder, and `tests/block_lz4.rs` covers the round trip, the LZ4-1 rules and an engine reopen with a deep verify. New dependency: `lz4_flex` (MIT, pure Rust, safe features only).
- `kerno-server --fulltext <types>`: the unit types to keyword-index in every database served. The examples (`chat-history`, `document-store`) turn on the types they search.
- `Db::open_with_limits` and Python `kerno.open(path, max_batch_ops=...)` change the 10,000-operation limit of one change set (50,000 operations committed atomically in a test).

- **Python bindings, third round**: `hybrid_search` (keywords, vectors and relations fused by reciprocal rank), `explain`, `find_range`, `history` with bounds and order, `compare` of two revisions, `Unit.oldest_revision`, and `query(lifecycle=...)`. 12 new tests.
- **Node.js bindings at parity with Python** (`@rixmerz/kernodb`): writes, `commit` with all nine batch ops, history and maintenance, schemas, documents, `find`, `execute`, `hybridSearch` and open options, with stable error `code`s and a rewritten README. 46 tests.
- **Keyword index everywhere it is used**: `Db::search_text(query, limit)` (keyword-only search) and `Db::fulltext(type)` in the facade; `db.fulltext(type)` in Python and Node; `GET` and `PUT /v1/types/{type}/fulltext` in `kerno-server`; `set_fulltext` / `get_fulltext` in the Python client and `setFulltext` / `getFulltext` in the TypeScript client. The Vercel AI SDK memory turns its own type's keyword index on, once, on its first `remember` or `recall`, so `recall` works with no server configuration.
- **Shared read-only opens**: a read-only `Db` no longer takes the directory lock, so any number of readers in other processes can open a directory beside one writer; `KernoDb::refresh` picks up commits made since the open (design in `crates/kerno-storage/DESIGN.md`).
- **Damage mapped to units**: when a block is damaged, `verify` names the units it holds instead of only the block.
- **`compact` drops the payloads of deleted units**, so the store shrinks after deletes.
- **Documentation for alpha.2**: an "Upgrading from 1.0.0-alpha.1" section in the `kerno` crate (rustdoc and README), the getting-started guide, and the Python and Node READMEs. Keyword search is opt-in everywhere it is described: crate docs (a doctest), the server README, the clients, the integrations and the examples. `docs/compatibility.md` describes the alpha.2 cut. The client and integration READMEs now say what holds today: TLS works with an `https://` URL, and API tokens are not sent yet (they had said the server had neither).

### Changed

- Faster batches: a change set caches the units it already found active (invalidated by its own writes) and keeps the adjacency counts in memory, writing each count record once at commit instead of once per relation. On 9,000 relations among 300 units, 82 → 63 ms; relation batches went from 10.9x to 5.9x the time of SQLite.

### Fixed

- `Db::verify` documented that a damaged store is a finding, not an `Err`; storage-level corruption (a block CRC mismatch, a damaged MANIFEST) is returned as an `Err`. The rustdoc now says so, and the Python `verify()` reports it as `clean: False` with `storage_error` instead of raising.
- **Python bindings**: `Database.update`, `delete`, `unrelate`, `commit` (an atomic batch of store/update/delete/unrelate ops), `history` and `read_as_of` (returning the new `kerno.Revision`), and `verify` (the units-layer integrity report as a dict). Driven by the experimental LiveSpec backend, which keeps one revisioned unit per indexed symbol.

## [1.0.0-alpha.1] - 2026-11-20

First public alpha. The on-disk format v1 is frozen (ADR-0013); the API and the wire protocols may still change before 1.0. Expect breaking fixes between alpha releases until the open items below are closed.

### Added

- **Rust crates** (crates.io): `kernodb-core`, `kernodb-storage`, `kernodb-units` and the `kernodb` entry-point crate. The library names stay `kerno_core`, `kerno_storage`, `kerno_units` and `kerno`, so code imports `kerno::...` (the plain `kerno` name on crates.io belongs to an unrelated project).
- **Storage engine**: crash-safe embedded engine with deterministic format-v1 vectors, checksums, backups and verification.
- **Units, revisions and relations**: history, retention, schemas, indexes, hybrid search (vectors and BM25) and a query language.
- **Bindings** (npm and PyPI): Node.js (`@rixmerz/kernodb`; the unscoped name was blocked by npm's spam filter) and Python (`kernodb`).
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
