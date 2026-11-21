# Compatibility promise for 1.0

**Status: proposed.** Kerno is at version 1.0.0-alpha.2 (an alpha, not 1.0) and the project owner declares 1.0
(ROADMAP, Phase D). Until then nothing here binds anyone. This document is the
promise that takes effect on that declaration, so the owner can approve it, or
change it, before it does. ADR-0010 left the semver policy open until a stable
API exists; the section on the Rust API below is the proposal for it and needs
the owner's decision.

Each promise names the test that keeps it. A promise with no test says so.

## 0. Before 1.0: the alpha.2 format cut

The promises below start at 1.0. Before that, **1.0.0-alpha.2 does not open databases written by
1.0.0-alpha.1**, by design ([ADR-0023](adr/0023-format-v2-cut-for-alpha2.md)): nobody ran alpha.1
in production, so the format was decided once instead of carrying compatibility paths.

- An alpha.1 database is refused with `LayerTooOld` (Python: `KernoError`), and nothing is written.
- To move one across: `kerno <dir> export <file>` with the **alpha.1** CLI, then `kerno import <file>
  <new-dir>` with the **alpha.2** CLI. Then turn on the keyword index of the types you search
  (`set_fulltext`, see below) and redefine schemas and indexes, as for any import.
- The other way round, an alpha.1 build refuses an alpha.2 database (its units layer is 7 and
  its tables carry the `BLOCK_LZ4` bit).
- What changed in behaviour: **the keyword (BM25) index is opt-in per unit type**. A type is
  searchable by text only after `set_fulltext(type, true)` (Rust, Python, Node), `PUT
  /v1/types/{type}/fulltext` (server), or `kerno-server --fulltext <types>`.

## 1. What is promised

From 1.0, for every 1.x release:

1. **A database written by any earlier 1.x opens and works in a later 1.x**,
   with no migration step and no export.
2. **An export of version 1 is readable by every 1.x**, and every 1.x writes
   export version 1 and nothing else.
3. **The public Rust API follows semver** (section 4).
4. **Nothing is silently misread.** A file a build does not understand is
   refused with `Incompatible` and left unchanged. It is never opened as if it
   were something else.

Not promised: opening a database with an *older* build than the one that last
wrote it, an automatic downgrade, or compatibility without limit in time
(ADR-0003 §5). The way across a major version, or back to an older build, is
export and import (section 3).

## 2. File formats

Kerno has three version axes that move on their own
([ADR-0003](adr/0003-physical-format-and-versioning.md) §4). A database directory
has the first two; an export file has the third.

| Axis | Today | Spec | Where the version lives |
|---|---|---|---|
| Engine (physical) format | `format_version = 1`, **frozen** since 2026-10-31 ([ADR-0013](adr/0013-format-v1-and-engine-protocol.md)) | [file-format/v1.md](file-format/v1.md) | header of every file, plus feature bits |
| Units layer (logical) | `layer_version = 7`, the cut version of alpha.2; 1 to 6 are refused | [units-v1](file-format/units-v1.md) to [units-v7](file-format/units-v7.md) | `LAYER_META` record `layer` |
| Export | `format_version = 1`, **frozen** | [file-format/export-v1.md](file-format/export-v1.md) | export header |

Derived data (vectors, summaries, indexes) has no promise of its own: it is
rebuilt from the units, and a changed algorithm marks it stale
([ADR-0018](adr/0018-versioning-and-rebuilding-of-derived-artifacts.md)).

### 2.1 Engine format v1

Frozen under the split that ADR-0013 fixed:

- **Immutable while v1 exists** (a change needs a new `format_version`): the
  common header, integer and checksum encodings, the TLV wrapper, the WAL frame
  and SST block wrappers, the SST footer, the MANIFEST record, and the recovery
  decision table.
- **Versioned by feature bits** ([v1.md](file-format/v1.md) §4): a new capability
  a reader may ignore (`compat`), open read-only (`ro_compat`), or must refuse
  (`incompat`). Encryption (ADR-0006) arrives this way and is not part of 1.0.
  LZ4 data blocks (`incompat` bit 1, `BLOCK_LZ4`, v1.md §4.4) arrived this way in alpha.2:
  every new table has it, unless `Config::block_compression` is turned off.
- **Tunable with no format change:** API limits, block and memtable sizes,
  compaction parameters, performance targets.

A v1 reader opens every file with no unknown bit. An unknown `incompat` bit or
a `format_version` above 1 gives `Incompatible` and changes nothing; an unknown
`ro_compat` bit opens read-only; the pre-freeze draft is refused as
`Incompatible("pre-freeze draft format")`. No migration exists because no
earlier supported version does; the framework of ADR-0003 §5 arrives with the
first format v2.

Kept by: `crates/kerno-storage/tests/struct_vectors.rs`
(`struct_vectors_encode_exact`) and `dir_vectors.rs`
(`dir_vectors_are_reproducible`), which regenerate every committed vector byte
for byte, so a changed byte fails CI.

### 2.2 Units layer

`layer_version` is written into the database mark:

- **Every database is created at the cut version, 7** (alpha.2, ADR-0023), and a mark below 7 is
  refused (`LayerTooOld`). Opening and reading never write.
- **A build refuses a layer newer than it knows** (rule UNIT-16, `IncompatibleLayer`) and
  modifies nothing.
- From 1.0, a minor release may add a layer version, raised only by the write that needs it
  and never lowered. That is the one thing in a minor release that an older build cannot
  follow, and the release notes must say so. To go back, export from the newer build and import
  into the older one (section 3).

Kept by: `crates/kerno/tests/batch_ext.rs` (`a_database_from_before_the_cut_is_refused`), the
UNIT-16 checks of `crates/kerno-units/tests/unit_db.rs` and `verify.rs`, and
`crates/kerno-cli/tests/cli.rs` (the layer version `stats` reports).

### 2.3 Export v1

Frozen byte for byte; the full contract, including what an export does not
hold, is [export-v1.md](file-format/export-v1.md).

- Every 1.x writes `format_version = 1` and reads it. A newer version appears
  only with a major release, which must also read version 1 or ship a converter.
- A file of another version is `Incompatible`, a malformed one is `Corruption`,
  and in both cases nothing is written to the target.
- The export is the only format that crosses engines: the same logical state
  gives the same bytes on every engine (invariant I-30).

Kept by: `crates/kerno-units/tests/export_golden.rs` with its committed files in
`tests/golden/export-v1/` (a changed byte fails it), `export_import.rs`
(round trips and every malformed-file case) and `export_physical.rs`
(independence from the physical layout).

**Two limits that the freeze keeps, so that nobody finds them later**
(export-v1.md §5): the export of a history that a policy trimmed (`Audit`)
is valid but cannot be imported, and the reference importer refuses a commit
sequence above 1,000,000. An export holds no schemas, indexes or derived data,
so a restore is: import into a new database, turn on the keyword index of the types you search
(`set_fulltext`), redefine schemas and indexes, and rebuild derived data.

## 3. Upgrade and downgrade

| You want | Do |
|---|---|
| Move from 1.0.0-alpha.1 to alpha.2 | Export with alpha.1, import with alpha.2, then `set_fulltext` for the types you search (section 0) |
| Move to a later 1.x | Close the database, run the new build on the same directory. Nothing to migrate |
| Check before and after | `kerno <dir> verify`, and `kerno <dir> export <file>` first if you want a copy that does not depend on the format |
| Move to the next major version | Export with the old build, import into a new directory with the new one. The new major reads export v1 or ships a converter |
| Go back to an older build | If the directory is refused (`Incompatible`), export with the newer build, import with the older. Not available if the older build cannot read export v1 |
| Move between engines | Export and import. The file is the same on every engine |

Write the export to a file you keep before any upgrade you cannot undo. A
build never changes a directory it refuses.

## 4. The Rust API (proposal for the owner's decision)

The semver policy below is a proposal. Nothing enforces it yet; see the gaps.

- **Covered by semver from 1.0:** the `kerno` crate: `Db`, its methods and the
  types it re-exports at the crate root (`Unit`, `UnitId`, `Relation`,
  `Document`, `Schema`, `Query`, `Error`, and the like).
- **Not covered:** `kerno::units` and `kerno::storage` (the whole layers below,
  reachable for power use; `kerno-storage` states that its API is "explicitly
  unstable"), `Db::unit_db`, the `test-support` feature, and `kerno-core`,
  `kerno-testkit` and the spikes. Anything not in the covered list may change in
  a minor release. Moving an item out of the "not covered" list into the
  covered one is a decision, not an accident: it needs an explicit mention in
  the release notes.
- **Rules.** A breaking change to a covered item needs a major release. A new
  method, type or feature flag is a minor release. A bug fix that makes a
  documented behavior true is a patch release. The minimum supported Rust
  version (`rust-version`, 1.89 today) can be raised in a minor release and is
  announced in the release notes. A covered item is deprecated for at least one
  minor release before a major release removes it.
- **Errors.** `kerno::Error` (`UnitError`) and `kerno_core::Error` are
  `#[non_exhaustive]`, so adding a variant is a minor change. Match them with a
  wildcard arm. Error *messages* are not covered; error *classes* are.
- **The other entry points.** The command line (`kerno`), the MCP server
  (`kerno-mcp`) and the Python and Node bindings are separate surfaces. Their
  command, tool and function names and arguments are meant to follow the same
  rules, but no list of them has been frozen, and human-readable output text is
  not covered. Freezing each list is a task before 1.0.

**Gaps before this can be called a promise.** No tool checks the covered API
for breaking changes (`cargo semver-checks` or a committed public-API listing
is not set up), the covered list above has not been reviewed item by item, and
the crates are published at version 1.0.0-alpha.1 only after the owner approves the release.

## 5. When a promise is broken

If a 1.x release opens a database the previous 1.x wrote and misreads it, or
refuses a version 1 export, that is a bug of the highest priority: the fix is
a patch release, and the golden tests above get the case that missed it.
Changing a frozen format is not a fix; it needs a new `format_version`, new
vectors, and an ADR (ADR-0003, v1.md §15, export-v1.md §6).
