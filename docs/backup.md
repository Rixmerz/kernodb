# Backup, WAL shipping and point-in-time restore

Kerno can keep a copy of a database in a directory or in S3-compatible object storage (AWS S3,
Cloudflare R2, MinIO) while it is being written, and rebuild the database from that copy: as of the
latest commit, as of an exact commit, or as of a moment in time. The code is the `kerno-backup`
crate; `kerno-server` runs it in the background (`--backup`) and the `kerno` command-line tool
runs it by hand (`backup`, `ship`, `restore`, `backups`).

Backups are **asynchronous**: the server ships the write-ahead log every few seconds, so a disk
that is lost loses at most the commits since the last shipment (5 seconds by default), not zero. See
[Limits](#limits) before relying on it.

## How it works

A backup in a store is two kinds of object:

| Object | Holds | Made by |
|---|---|---|
| **Base** `base/<commit>/part-*` and `base/<commit>/MANIFEST` | Every key and value of the engine at one commit | `kerno <dir> backup`, `POST /v1/backup`, and the server at start |
| **Segment** `wal/<first>-<last>-<time>` | The commits `first` to `last`, read from the write-ahead logs | `kerno <dir> ship` and the server every `--backup-interval` seconds |

**The base is taken while the database writes.** The engine hands out a snapshot, a view at one
commit that later commits do not change, and the base reads it page by page. Nothing is locked and
no writer waits. The writes that arrive meanwhile are not in the base; they are in the segments
that follow it. The copy is of the raw keys and values below the units layer, so it carries every
index, count and derived record exactly as stored, and a restore needs to know none of them.

**Segments are read from the write-ahead logs.** The engine appends every commit to
`NNNNNN.wal`, checksummed frame by frame. The shipper only *reads* those files, so it runs while
the server holds the database open, and a frame that is still being written reads as a torn tail
and is picked up by the next shipment. Segments are consecutive: each starts at the commit after
the last one in the store.

**Every object is sealed**: a magic, a version, a kind and a BLAKE3 digest of everything before it.
A flipped, truncated or swapped object is an error on restore (`... the digest does not match: the
object is damaged`), never a database with something missing.

Object names are zero-padded, so a listing is in commit order. All of them live under the location
you give; with S3 the location is `s3://bucket/prefix`.

## Restoring

```sh
kerno /var/lib/kerno/restored restore file:///backups/kerno                 # as far as the backup goes
kerno /var/lib/kerno/restored restore s3://bucket/prefix --to-seq 120345    # exactly to commit 120345
kerno /var/lib/kerno/restored restore s3://bucket/prefix --to-time 1795000000000
```

The first argument is the **destination**, which must not exist or must be empty: a restore never
writes over a database. It is built next to the destination (`<dir>.restoring`) and moved in when
whole, so a restore that fails leaves nothing behind. If a crash leaves a `<dir>.restoring`, the
next restore refuses to run until you remove it.

- **Latest**: the newest base and every segment after it.
- **`--to-seq N`**: the newest base at or before commit `N`, then the segments up to `N`
  exactly. If the store ends before `N` it fails (`the backup reaches commit M only, not N`)
  instead of restoring less.
- **`--to-time MS`** (milliseconds since the Unix epoch): the newest base taken by then and the
  segments shipped by then. **The resolution is the shipping interval**: a commit made before `MS`
  is in the restore only if a shipment had read it by `MS`. Use `--to-seq` when you know the commit.

A hole in the chain (a segment someone deleted from the middle) is an error. A missing *last*
segment cannot be told apart from a store that simply ends sooner; only `--to-seq` notices it.

**A restore takes time proportional to the number of commits the database ever had, not to its
size.** The units layer writes the commit number of every change into its records and expects the
next commit to get the next number, so the restored engine has to stand at exactly the original
commit. The restore therefore spends the commits the base does not account for as small no-op
commits (a delete of a reserved key that is never there), then writes the base in one commit per
part, then replays the segments one commit each. How long that takes is measured in
[Restore time](#restore-time).

**A restored database is a new database.** The engine gives it a new `db_id`, so the backups of the
original refuse it: a server started on a restored directory with the same `--backup` location
logs `backup failed` at every tick and keeps no backup (the shipper says the store holds backups of
another database). Give the restored database its own location or prefix.

`kerno <dir> backups <url>` lists what a store holds and how far a restore from each base can go
(`<dir>` is ignored, as it is for `import`).

### Restore time

Measured on one machine (a release build, a local disk, the restore writing with
`Durability::Buffered`), restoring from memory a database whose every commit rewrote one unit, so
the data grows with the commits as well:

| Commits | Stored | Base backup | Restore |
|---|---|---|---|
| 10 001 | 2.2 MB | 38 ms | 56 ms |
| 100 001 | 22 MB | 0.19 s | 0.6 s |
| 1 000 001 | 226 MB | 1.8 s | 6.2 s |

Your disk and your store (an S3 round trip per object) will differ; the figures say the order of
magnitude. They come from `cargo test -p kerno-backup --release --test roundtrip -- --ignored
--nocapture restore_time`.

## With kerno-server

```sh
kerno-server --db /var/lib/kerno/db \
  --backup s3://my-bucket/kerno --backup-interval 5
```

| Flag | Meaning |
|---|---|
| `--backup <url>` | Keep the backups of every database in `<url>`: `file:///dir` or `s3://bucket/prefix`. |
| `--backup-interval <s>` | Seconds between shipments, 1 or more. Default 5. Needs `--backup`. |

- **Start.** The first shipment happens at once. With no base in the store it takes one; with a
  chain from an earlier run it continues it, and if the database was recovered after a crash (the
  engine replayed and deleted the log, so commits were never shipped) it takes a new base.
- **Every interval.** It ships what is new. If the engine deleted a log before it was shipped (the
  shipper fell behind a very fast writer), it takes a new base and goes on from there. A failure
  (the store is down) is logged and retried at the next tick; it never stops the server.
- **On SIGINT or SIGTERM**, after the last request is answered and before the databases close, it
  ships once more, so a clean stop loses nothing.
- **Tenants.** In tenant mode each tenant's backups are under `<location>/<tenant>/...`; restore
  with the location `s3://bucket/prefix/<tenant>`. A tenant's token reaches only its own backups.
- **`POST /v1/backup`** (HTTP only, token of the tenant in tenant mode) takes a base backup now and
  answers 201 and `{"seq", "db_id", "created_ms", "parts", "entries", "bytes"}`. Without `--backup`
  it is a 404 that says so. There is no gRPC call for it.
- **Metrics**, only with `--backup`, summed over every database and never labelled with a tenant:
  `kerno_backup_shipped_commits_total`, `kerno_backup_segments_total`, `kerno_backup_bases_total`,
  `kerno_backup_errors_total`.
- **Logs**: `backup` at INFO when a shipment carried commits or took a base (`tenant`,
  `base_commit`, `shipped_commits`, `shipped_to`), and `backup failed; trying again at the next
  tick` at WARN, with the cause logged as an `internal error`. No credential is ever logged.

## With the command line

| Command | Does |
|---|---|
| `kerno <dir> backup <url>` | A base backup. Opens the database read-only, so the database must be closed: a running server holds the exclusive lock, and takes its own bases. |
| `kerno <dir> ship <url>` | Uploads the commits since the last shipment. It reads files only, so it **works while a server has the database open**. Needs a base in `<url>`; fails with `take a new base backup` when the engine already deleted an unshipped log. |
| `kerno <new-dir> restore <url> [--to-seq N \| --to-time MS]` | Rebuilds a database. |
| `kerno <any> backups <url>` | Lists bases and segments. |

All four take `--json`. Exit status: 0 success, 1 failure, 2 a wrong command line.

## Locations and credentials

| URL | Store |
|---|---|
| `file:///absolute/dir` | A directory: a mounted volume or an NFS share. Objects are written to a temporary name, synced and renamed. |
| `s3://bucket/prefix` | S3 or an S3-compatible store, path-style, signed with AWS Signature Version 4. |

S3 is configured by the environment only, so no secret is on a command line or in a log:

| Variable | Meaning |
|---|---|
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | Required. `AWS_SESSION_TOKEN` too, if the credentials are temporary. |
| `KERNO_S3_REGION` | The signing region. Default `us-east-1`; `auto` for R2. |
| `KERNO_S3_ENDPOINT` | `https://host[:port]` or `http://host[:port]`. Default `https://s3.<region>.amazonaws.com`. For R2: `https://<account-id>.r2.cloudflarestorage.com`. For MinIO: its address. |
| `KERNO_S3_CA_FILE` | A PEM file of certificate authorities to trust besides the system's (a private MinIO, say). |

TLS uses the system's root certificates (`SSL_CERT_FILE` and `SSL_CERT_DIR` work as usual). An error
from the store names the status and S3's error code (`403 SignatureDoesNotMatch`), never a key.

## Limits

- **Not synchronous replication.** The recovery point is the shipping interval. A crash of the
  disk between two shipments loses what was committed in between. `Durability::Buffered` databases
  additionally lose what the OS had not written.
- **The log can be deleted before it is shipped.** The engine deletes a write-ahead log once it has
  flushed it. A writer so fast that the engine fills and flushes the 8 MiB write buffer between two
  shipments outruns the shipper; the shipper notices (`no longer holds commit N`) and the server
  answers with a new base backup. A smaller `--backup-interval` shrinks the window. Nothing is ever
  silently skipped.
- **Restore time** grows with the number of commits, not the size of the data (see above).
- **No retention.** Nothing deletes old bases and segments; use the bucket's lifecycle rules. A
  restore needs one base and the segments after it, so keep any base you may want to restore from
  together with everything after it. Taking a base now and then (`POST /v1/backup`) bounds how many
  segments a restore replays.
- **A suppressed unit stays in backups until they expire.** Suppressing or erasing a unit in the
  database does not reach the bases and segments already shipped. The retention deadline for
  backups is an owner decision that is still open (SEC-15, decision 9): this document states none,
  and until the owner sets one, expiry is whatever the bucket's lifecycle rules say.
- **No encryption and no authentication of the objects.** The digest finds damage, not tampering
  (anyone who can write to the store can write a valid object), and the data is stored as the
  database holds it. Encrypt at the store (SSE, a bucket policy) if you need it; encryption in
  1.0 is [not planned](../ROADMAP.md).
- **One database per location.** A store holds the chain of one database; the server gives each
  tenant its own prefix. A shipper refuses a store whose bases are of another database.
- **A base is read through memory** in parts of at most 48 MiB, and a restore holds one part at a
  time.
- **The format of the objects is version 1 and is not frozen**; a build that reads another version
  says so instead of guessing.
- **`kerno <dir> backup` needs the database closed**, and the server's `POST /v1/backup` is the way
  to take a base from a running database.

## What is tested

- A base taken while a thread writes, then shipped and restored, gives a database whose logical
  export (which holds every commit number) is byte-identical to the original's, `verify_units` is
  clean, and a write after the restore works.
- A restore to an exact commit equals the state the database had at that commit; to a time, the
  state at the shipment before it.
- A restored object that is flipped in any byte, or missing, stops the restore and leaves no
  directory behind.
- A hole in the log is reported as a gap, and a new base mends it; a writer that rotates the log
  every few KiB is followed by the server's loop.
- A killed server takes a new base when it starts again; a stopped one ships its last commit.
- The S3 client is checked against the AWS documentation's signature example, and against a mock
  S3 server (plain and TLS) that verifies every signature with its own implementation.
