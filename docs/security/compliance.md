# Compliance notes

Status: draft for the owner's review (row SEC-14 of the completion plan). Facts only: each one names the file or the test that shows it. This page makes no legal claim and does not say that Kerno, or an application built on it, complies with any law or standard. Kerno provides mechanisms; the application decides how to use them and stays responsible for its obligations ([threat model](threat-model.md) §1 and §4).

Paths are relative to the repository root. `ADR-0009` is [the suppression, retention and backups decision](../adr/0009-suppression-retention-and-backups.md).

## 1. Erasure

### What Kerno does

- **Delete is logical and immediate.** The unit gets a tombstone revision and keeps its head with lifecycle `Deleted`. Queries and keyword search stop finding it, and its id is never reused. Sources: `crates/kerno/src/lib.rs:304-306`, `crates/kerno-units/src/db.rs:1407-1409`, ADR-0009 Decision point 1.
- **A snapshot taken before the delete keeps the old state until it is released** (ADR-0009 point 1).
- **Derived data goes in the same commit.** Vectors, descriptors and their records are removed with the delete (`crates/kerno-units/src/suppression.rs:6-12`).
- **The public id token is removed on delete** ([export format v1](../file-format/export-v1.md) §1).
- **`reclaim` and `compact` clear the live files.** `reclaim` is also `kerno <dir> reclaim`. Both make the live files stop holding deleted data, once no snapshot older than the deletion is open. Sources: `crates/kerno-units/src/reclaim.rs:1-16`, `crates/kerno-units/src/retention.rs:701-709`, ADR-0009 point 2. This is observed by `crates/kerno-storage/tests/suppression_survival.rs`; that test calls itself an observation, not a contract.

### What Kerno does not do

- **Delete does not remove history.** Earlier revisions, payloads included, stay readable through `history`, `get_revision` and `unit_as_of` until retention prunes them (`crates/kerno/src/lib.rs:305`).
- **Under the default policy, retention always keeps a deleted unit's last content.** The default history policy is `Versioned` (`crates/kerno-units/src/db.rs:127`). Under it, retention always keeps the revision before a tombstone, payload included (`crates/kerno-units/src/retention.rs:13-15` and `:284-285`; test `crates/kerno-units/tests/retention.rs:239`). Only a type whose policy is `None` keeps nothing below the head (`retention.rs:268-269`). Under `Audit`, retention drops the payloads below the head but keeps their content hash and length (`retention.rs:16-21`). A policy applies to a whole unit type (`set_policy`, `crates/kerno-units/src/db.rs:630`). There is no per-unit purge.
- **Change-set records are never deleted by any API in this release.** Each record keeps the actor, source, reason and correlation (`crates/kerno-units/src/db.rs:184-198`) and its member list (unit id and revision number). They outlive pruned revisions and deleted units. Evidence: the tags `CHANGESET` 0x30 and `CS_MEMBER` 0x31 (`crates/kerno-units/src/codec.rs:27-28`) appear in `crates/kerno-units/src` only as writes (`db.rs:1279, 1691, 1751`) and reads (`history.rs`).
- **A unit that relations still touch cannot be deleted.** It answers `UnitHasRelations` (`crates/kerno/src/lib.rs:310-311`).
- **There is no physical or secure erasure.** Freed blocks are not overwritten. The file system, SSD wear-levelling and OS or cloud snapshots can keep copies. Sources: ADR-0009 point 3, threat model §6.5, the `compact_range` doc at `crates/kerno-storage/src/db.rs:2011-2016`. `delete`, `compact` and `reclaim` are not secure erasure (ADR-0009 Consequences).
- **There is no crypto-erasure** (ADR-0009 point 4; there is no encryption in 1.0).
- **Backups and exports are not erased.** See Retention below.

## 2. Retention

### What Kerno does

- **A history policy per unit type:** `None`, `Audit` or `Versioned`; the default is `Versioned` (`crates/kerno-units/src/types.rs:144`, `db.rs:127`).
- **Retention rules** (`crates/kerno-units/src/retention.rs:67-80`): `KeepAll` (the default), `KeepLast(n)`, `KeepFor(duration)` and `KeepSince(commit_seq)`. `KeepFor` reads the injected clock and the descriptive `created_at`.
- **`run_retention` is logical, bounded by a budget and resumable.** `compact` and `reclaim` then reclaim the files (`retention.rs:4-50`).
- **TTL expiry archives; it does not delete.** `expire_due` archives due units: an archived unit stays readable and in history, and `unarchive` undoes it (`crates/kerno-units/src/hygiene/expiry.rs:36-41`).

### What Kerno does not do

- **Nothing runs retention or expiry on its own.** No code in `crates/kerno`, `kerno-cli`, `kerno-mcp` or `kerno-server` calls `run_retention` or `expire_due` (the search is empty). The application calls them through the `kerno-units` API (`Db::unit_db`). The CLI and the server have no retention command or route.
- **Backups never expire on their own.** `kerno-backup` deletes no base and no WAL segment: "use the bucket's lifecycle rules" ([backup](../backup.md) §Limits). A suppressed unit stays in every base backup and segment taken before its deletion or pruning, until those objects are deleted. No backup deadline is declared (ADR-0009 point 5, criterion B-1). The deadline is the owner's decision 9 (row SEC-15); this page sets none.
- **Observations are not kept on disk.** They stay in memory, with bounded retention (threat model T-11, [ADR-0015](../adr/0015-privacy-of-observability.md)).

## 3. Export (data portability)

### What Kerno does

- **A whole-database logical export at one snapshot.** Format v1 is frozen and documented byte for byte ([export-v1](../file-format/export-v1.md); [compatibility](../compatibility.md) §2.3). CLI: `kerno <dir> export <file>`. It opens the directory read-only (`crates/kerno-cli/src/cmd.rs:511`), so the owning process must have closed it first (exclusive lock, threat model §6.4). API: `kerno_units::export::export`.
- **What it holds:** units, retained revisions, relations, history policies and public tokens (export-v1 §1). The same logical state gives the same bytes on every engine (I-30).
- **Import into a new database** with `kerno import`.
- **The CLI prints its reads as JSON** with `--json` (`crates/kerno-cli/src/main.rs:78`).

### What Kerno does not do

- **No per-person export.** There is no subject filter; the application gathers one person's data with its own reads.
- **Not in the export:** derived data, typed schemas, indexes, and the reason and correlation of change sets (export-v1 §1).
- **An `Audit`-trimmed history does not import**, and the reference importer refuses a commit sequence above 1,000,000 (export-v1 §5).
- **No export route in `kerno-server` or `kerno-mcp`** (the search is empty).
- **The export file is not encrypted.**

## 4. Encryption

### What Kerno does

- **The format reserves an AEAD slot.** A file with the AEAD bit set is refused with `Incompatible("encryption not supported")` (threat model §6.3; `docs/FEATURES.md` row SEC-01).
- **`kerno-server` can serve TLS.** Without it, tokens cross the network in clear text and the server logs a warning ([server](../server.md) §TLS).

### What Kerno does not do

- **No encryption at rest and no key management in 1.0.** Decision D3, taken 2026-11-13 (`docs/architecture/stage9-record.md:294, :323`); `ROADMAP.md` "Not planned"; threat model T-03.
- **No authenticated integrity.** CRC-32C finds damage, not deliberate tampering (D4; threat model §6.1 and §6.6).
- **Backups are stored as the database holds them.** Their digest finds damage, not tampering; encrypt at the store (backup §Limits).
- **Export files are not encrypted.**
- **Volume or file-system encryption** is neither required nor detected by Kerno.

## 5. Audit

### What Kerno does

- **Each change set records who and why** (`crates/kerno-units/src/db.rs:184-198`): actor and source, at most 256 B each, copied into every revision of the change set; reason and correlation, at most 1024 B each; purpose (`Application`, `Maintenance`, `Retention`); and `created_at` from the injected clock, descriptive and never used for ordering (`crates/kerno-units/src/history.rs:121-122`).
- **Reading it:** `history`, `change_set` and `change_sets_since` (`history.rs:850-876`), and `kerno <dir> history`.
- **Retention change sets are marked** with purpose `Retention`. Expiry change sets carry the hygiene actor (`crates/kerno-units/src/hygiene/expiry.rs`).
- **No payload bytes in errors, verify reports or observations** (I-29; threat model T-08; `crates/kerno-storage/tests/redaction.rs`, `crates/kerno-units/tests/redaction_units.rs`).
- **`kerno-server` logs one JSON line per request to stderr** ([server](../server.md) §Logs): method, path (unit ids and types), status, tenant and duration; never tokens, payloads or query text. Kerno does not keep that log.

### What Kerno does not do

- **The actor is not authenticated.** It is whatever the application passes (threat model §1).
- **No record of reads.**
- **No persisted operation-level audit log** (D8, `docs/architecture/stage9-record.md:299`; `docs/FEATURES.md` row SEC-12).
- **Change-set records are not tamper-evident** (D4).

## Summary

| Topic | Does | Does not |
|---|---|---|
| Erasure | Logical delete at once; derived data and public token go with it; `reclaim` and `compact` clear the live files | Remove history; secure or crypto erasure; delete change-set records; purge one unit |
| Retention | Per-type history policy; retention rules; archive on TTL | Run retention or expiry by itself; expire backups |
| Export | Whole-database logical export (format v1) and import | Per-person export; export derived data; encrypt the file |
| Encryption | Reserve an AEAD slot; TLS in `kerno-server` | Encrypt at rest; manage keys; authenticate integrity |
| Audit | Actor, source, reason, correlation and purpose per change set; request log in the server | Authenticate the actor; log reads; keep an audit log; tamper evidence |
