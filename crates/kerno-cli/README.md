# kerno-cli

`kerno` is an administration command-line tool for any Kerno database directory: check it,
look inside it, query it, export it, import an export, and compact it. It uses only the
public API of `kerno-storage` and `kerno-units`, and `kerno-backup` for the backup commands;
its one other dependency is `serde_json`, for `--json`.

## Build

```sh
cargo build --release -p kerno-cli
# the binary is target/release/kerno
```

## Usage

```text
kerno <dir> <command> [args]
kerno import <file> <new-dir>
kerno <dir> backup|ship|backups <url>
kerno <new-dir> restore <url>
kerno --help            every command
kerno <dir> --help      the same
kerno <dir> <command> --help
```

`<dir>` is the database directory and always comes first. `--help` opens nothing.

**Exit status:** `0` success; `1` failure (no database, a unit that does not exist, a database
another process holds, a file that cannot be written) and also a `verify` that found
something; `2` the command line is wrong. A failure is one line on stderr, `kerno: ...`;
nothing panics.

**Read-only.** `verify`, `stats`, `get`, `history`, `relations`, `query` and `export` open the
database read-only (`AccessMode::ReadOnly`): they create, sync, delete and change no file
(I-18), and a directory that holds no database is an error, never a new database. Only
`reclaim` opens for writing; `import` writes the new directory it is given.
One process holds a directory at a time: while another handle is open, every command fails with
"database is locked by another handle".

**Output.** The read commands print text; with `--json` they print one JSON document with the
same facts. A unit is named by its 32-digit hex id (what every command prints) or by its public
id (`k1_...`). A payload (or metadata) is shown as text when it is valid UTF-8, otherwise as hex
with its length.

## Commands

### `verify [--deep] [--json]`

Runs two checks and prints every finding; exits `1` if there is any.

- The engine's file check (`kerno_storage::verify`). Without `--deep` it runs the open-time
  checks; `--deep` also reads every block of every table.
- The units layer's `verify_units` (records, revisions, relations and their adjacency entries,
  the primary index, public ids).

If the layer cannot even open or read the store, that is reported as a finding, not a crash.
Orphan files are listed as a note: they are not findings.

### `backup <url>`, `ship <url>`, `restore <url>`, `backups <url>`

Hot backup, WAL shipping and point-in-time restore to `file:///dir` or `s3://bucket/prefix`; all
four take `--json`. `backup` copies the whole database as a base (it opens it read-only, so the
database must be closed); `ship` uploads the commits since, read from the write-ahead logs, and
works while a server has the database open; `restore <url> [--to-seq N | --to-time MS]` rebuilds
the database into `<dir>`, which must be new or empty; `backups` lists what a store holds (`<dir>`
is ignored). Read [docs/backup.md](../../docs/backup.md) first.

### `stats [--json]`

Units by type (active, archived, deleted), relations by type, the state and watermark of the
primary index, the layer version and the format version of the engine files, the last commit
and every file in the directory with its size. It reads every unit, so it is linear in the size
of the database.

### `get <unit-id> [--json]`

One unit at its current revision: type, lifecycle, revision, schema version, commits and
times, public id, metadata and payload.

### `history <unit-id> [--json]`

The retained revisions of a unit, oldest first, each with its kind, commit, actor, source and
payload. A revision whose payload the unit type's history policy does not keep says so.

### `relations <unit-id> [--dir in|out] [--type T] [--json]`

The relations of a unit, answered by the planner. Without `--dir`, both directions (`out`
first). `--type` keeps one relation type. Each row has the relation id, both ends and its
provenance.

### `query <query> [--explain] [--json]` and `query --type T [--limit N] [--explain] [--json]`

`kerno <dir> query 'FIND user AS u JOIN team AS t VIA member_of WHERE u.age >= 30 ORDER BY u.age
DESC LIMIT 10'` runs a query of the language in `docs/query-language.md`: filters, `ORDER BY`,
`LIMIT`/`OFFSET`, and joins across unit types through typed relations, over the units of types
that have a schema. One line per row, one unit per binding; the header says how the first binding
was read (`TypeScan`, or `FieldIndex(field)` when a user-defined index was cheaper). `--json`
prints `columns`, `total`, `access` and the `rows` (each unit with its `fields`, or its payload
when its type has no schema). `--explain` adds the plan: the access paths of the first binding
with their costs and the one chosen, every join, and the planner's own plan for a scan of that
type. A query that does not parse is a usage error (exit 2) naming the byte; one that does not fit
the schemas (a field a type does not declare, a literal of the wrong type) is a failure (exit 1).
A query text and `--type` (or `--limit`) cannot be given together.

`query --type T` lists the active units of type `T`, in unit id order, answered by the planner:
the output says which path answered (`IndexType`, `IndexPrimary` or `UnitsReference`) and whether
it fell back. `--limit N` shows the first `N` (the total is still printed). `--explain` also
prints the plan: its fingerprint, the index state, every candidate path with its cost and
usability, the one chosen and why, and what the result cache would do.

### `export <file>`

Writes the database to `<file>` in the export format. **The format is frozen at
`format_version` 1** (`docs/file-format/export-v1.md`): a later Kerno reads this file. The same logical state gives the same bytes. It never
replaces a file that exists.

### `import <file> <new-dir>`

Creates a database in `<new-dir>` from an export file. `<new-dir>` must not exist or must be an
empty directory; the engine creates it. There is no source directory, so the command is
written `kerno import <file> <new-dir>`; the form `kerno <dir> import ...` is accepted and
`<dir>` is ignored. A failed import removes what it made. Reads the frozen export format v1 (`docs/file-format/export-v1.md`). An export whose layer-wide default history policy is not `Versioned` is refused by
`import` (see Limits).

### `reclaim`

Compacts the live files so that they stop holding what was deleted logically. **It is not a
secure erase** (ADR-0009): a file that an open snapshot retains stays on disk, and the engine
does not overwrite the blocks it frees, so nothing is promised about the storage medium. It is
the only command that writes to a database.

## Limits

- `stats` reads every unit and every adjacency list; the public API has no counts by type.
- It shows the primary index only; the persisted derived data (vectors, summaries) has no
  public listing without knowing its descriptors.
- `import` opens the new database with the default configuration, so it refuses an export made
  with another layer-wide default history policy; the public API gives no way to read that
  policy from the file.
- `query --explain` draws a fresh fingerprint key (`/dev/urandom`, Unix only), so the
  fingerprint differs on every run.

## Tests

```sh
cargo test -p kerno-cli
```

Each test runs the real binary on a temporary database built through the public API.
