# Concurrency in Kerno

What happens when several threads, or several processes, use one Kerno database at the same
time, and which guarantees you can rely on. Every claim below has a test; the table at the end
says which.

## In one paragraph

Inside one process, any number of threads read while one thread writes: readers never wait for
the writer and never see half of a commit. Writers take turns, so writes are serializable.
Across processes, one process at a time writes a database directory, through an exclusive lock
file: a second writer is refused at once, it does not wait. Any number of read-only processes read
beside the writer, with no lock, and each sees a consistent committed state
([ADR-0024](adr/0024-concurrent-read-only-opens.md)).

## Threads (one process)

A [`kerno::Db`](../crates/kerno/src/lib.rs) is `Send + Sync`. Every method takes `&self`, so
share it by reference (`std::thread::scope`) or behind an `Arc`.

| Part | What it does |
|---|---|
| **Readers** | Any number, in parallel. A read takes a **snapshot** of the latest published commit and reads from it. It takes no lock the writer holds, so it never waits for a write and a write never waits for it. |
| **The writer** | One at a time. A change set (`UnitDb::begin_changeset`, or any `Db` method that writes: `store`, `relate`, `define_schema`, ...) takes the engine's **write slot** and holds it until it commits, aborts or is dropped. A second writer blocks until the slot is free. |
| **Order** | Commits are totally ordered. Their commit sequences are consecutive: an abort or an empty change set consumes none. |

### Isolation level

| What you do | What you get |
|---|---|
| Write with a change set | **Serializable.** Change sets run one at a time, so the result is the one a serial execution in commit order gives. A change set reads the state of the last commit plus its own writes (`ChangeSet::get_unit`), and nothing can commit between its reads and its commit. No lost update, no write skew. |
| Read with a snapshot (`UnitDb::snapshot`) | **Snapshot isolation.** The snapshot shows exactly the commits up to its sequence, for as long as it lives: repeatable reads, no dirty reads, no phantoms (a query over the same snapshot counts the same units every time). Because writes are serial, a snapshot is always the state after some prefix of the commit order, so it is also consistent with a serializable execution. |
| Call one `Db` read method (`get`, `find`, `query`, `execute`, `related`, ...) | One snapshot, taken when the call starts. The call is atomic: a `FIND ... JOIN ...` query reads every row from one commit. |
| Call several `Db` read methods one after the other | **Read committed** between the calls: each call sees the latest commit at that moment. For one consistent view across calls, take `db.unit_db().snapshot()` once and read through it. |

A commit is **atomic to readers**: a snapshot sees all of a change set or none of it. The
units, their revisions, the relations and the index entries of one change set are one engine
commit.

A commit is **visible as soon as `commit` returns**, to snapshots taken on any thread after
that. With `Durability::Sync` (the default of `Db::open`) it is published only after its
`fsync`, so what a reader sees is already durable. With `Durability::Buffered` it is published
after the write and may be lost in a power cut (not in a process crash); `CommitInfo::durable`
says which.

### Rules for threads

- **One change set per thread at a time.** `begin_changeset` blocks while another change set
  holds the slot, so beginning a second one on a thread that already holds one (or calling
  `Db::store` while holding one) deadlocks.
- **Keep change sets short.** A long change set blocks every other writer. `max_changeset_ops`
  bounds it, and `run_retention` writes in small, paced change sets for that reason.
- **Do not keep snapshots longer than needed.** An open snapshot keeps the versions it can see:
  retention prunes them logically, but the engine cannot reclaim the bytes while the snapshot
  lives.
- **Writers can be slowed by the engine.** Flushes and compactions run on background threads.
  They never block readers, but when they fall behind the engine slows, and then stops,
  writers until they catch up.

## Processes

One process at a time **writes** a database directory. A read-write open takes an exclusive
advisory lock (`flock`) on the file `LOCK` in the directory, and the lock is held until the `Db`
is closed or dropped. A read-only open (`Config { access: AccessMode::ReadOnly, .. }`) takes **no**
lock: any number of reader processes read beside the writer and beside each other.

| Situation | Result |
|---|---|
| A second process opens, read-write, a directory another process holds | `Error::Locked` at once (`Error::is_locked()`). It does not wait. |
| A process opens a directory another process writes, read-only | It opens at once and reads the commits the writer had written when the open read the files. It never blocks the writer. |
| A second read-write `Db` in the same process | `Error::Locked`, for the same reason. Share one `Db`. |
| `verify` or `inspect` on an open database | `Error::Locked`. They are for a store nobody writes. |
| The owner exits, crashes or is killed (`kill -9`) | The operating system releases the lock with the process. The next open recovers the database from its log: every commit that returned is there. |
| A thread starts a child process (`fork` and `exec`) while a `Db` is open in another thread | Until the child execs it holds a copy of the `LOCK` file descriptor, and the lock is held as long as any copy is. A directory closed and opened again in that window gets `Error::Locked`, at once, though no one holds it for long. A host that spawns processes from threads and reopens a directory right after closing it should retry on `is_locked()` with a short sleep, or take turns with its own spawning. |

**Readers.** A read-only open reads the `MANIFEST`, the tables it names and the live logs, and
replays the logs in memory. It writes nothing: no `LOCK`, no recovery, no deletion. The state it
reads is a prefix of the writer's commits: whole change sets only, never half of one. If the
writer flushes, compacts or checkpoints while the read runs, the read starts again (for up to 30
seconds in all); damage that is still there after a read is reported at once, never skipped or
repaired. Tables the reader holds open stay readable after the writer deletes them.

A reader sees the commits the writer has *written* to its log, durable or not
(`Durability::Buffered` commits are visible once they are in the page cache): it does not wait for
the writer's `fsync`. Its handle is a view of one moment. To move it to newer commits, call
`KernoDb::refresh()` on the engine handle (`kerno-storage`): it returns the sequence it now reads,
and snapshots taken before keep their own state. The `kerno::Db` facade and a `UnitDb` over a
read-only engine have no refresh of their own: reopen them to see newer commits.

**Writers, one at a time.** The lock file is what makes it safe to start several writer processes
against one directory: each one gets in alone, and none sees a half-finished state. To share a
database between writer processes, retry on `is_locked()` (with a short sleep and a deadline),
open, work, close:

```rust
let db = loop {
    match kerno::Db::open(&dir) {
        Ok(db) => break db,
        Err(e) if e.is_locked() && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
        Err(e) => return Err(e),
    }
};
```

For a long-lived service, give the database to one process and let the others talk to it: that
is how `kerno-mcp` works (a second server on the same directory exits with a "locked" message),
and the `kerno` CLI reports the same refusal when a server holds the database.

### What is not supported

- **A second writer process at the same time.** There is no multi-writer mode: not across
  threads, not across processes.
- **A reader that outruns a writer for 30 seconds.** A writer that changes the files faster than a
  read can catch up starves the reader: its open fails with the last error, and a file it raced
  is reported as `Io(NotFound)`, which is not damage.
- **Network file systems.** The lock is an advisory `flock`, and a reader relies on the listing
  and the sizes of the files being current. Their behaviour on NFS and similar file systems
  varies, and nothing here has been tested on them: keep the directory on a local disk.

## Tests

| Claim | Test (`crates/kerno/tests/concurrency.rs` unless noted) |
|---|---|
| A snapshot never shows half of a change set; one snapshot reads the same state every time; a commit that returned is visible to the next snapshot; versions never go back | `readers_see_whole_change_sets_only` (4 reader threads against one writer) |
| A snapshot is a fixed view: no later update, no phantom | `a_snapshot_is_a_fixed_view` |
| Writers are serial: no lost update, each state is read once, consecutive commit sequences | `writers_are_serial_and_lose_no_update` (4 threads, read-modify-write) |
| An open change set blocks other writers and does not block readers | `a_writer_waits_for_the_open_change_set_and_readers_do_not` |
| A second process is refused at once when it opens read-write, a read-only one reads beside the owner, and the next open after it closes finds what it stored | `a_second_process_gets_locked_until_the_first_closes` |
| A writer process that commits 10-key batches, flushes and compacts, and reader processes: whole batches only, sequences that never go back, a second writer refused | `crates/kerno-storage/tests/read_only_processes.rs` |
| A read-only open takes no lock, refreshes to newer commits, reads again while a writer replaces every table, and reports damage without retrying | `read_only_takes_no_lock`, `read_only_refreshes_to_newer_commits`, `read_only_open_reads_again_when_the_writer_changes_the_files`, `read_only_open_reports_damage_without_retrying` (`crates/kerno-storage/tests/read_only.rs`) |
| A killed owner releases the lock and its commits survive | `a_killed_process_releases_the_lock` |
| Processes that start together take turns through the lock file and lose no commit | `processes_take_turns_through_the_lock_file` (4 processes) |
| The lock refuses a second writer, `verify` and `inspect`, and goes away with a killed process | `crates/kerno-storage/tests/process_lock.rs` |
| The engine's write transactions exclude each other and read the latest committed state; concurrent writers form one group commit and publish in order | `crates/kerno-storage/tests/txn.rs`, `crates/kerno-storage/tests/group_commit.rs` |
| A second `Db` in the same process is refused | `a_second_handle_in_the_same_process_gets_locked` |
| `kill -9` of a writer with concurrent writer threads loses no acknowledged commit | `crates/kerno-storage/tests/process_kill.rs` |

Run the new ones with `cargo test -p kerno --test concurrency`. The process tests start this
same test binary again as a child, so there is nothing else to build.
