# ADR-0024 — Concurrent read-only opens

| Field | Value |
|---|---|
| Status | **Proposed**, implemented with GitHub issue #2, section 3. It changes no on-disk format, but it changes what a read-only open promises, so it waits for the owner's acceptance |
| Date | 2026-11-21 |
| Deciders | Project owner (pending) |
| Related to | v1.md §9.5 (RO-1, RO-4), invariant I-18, ADR-0013 |

## Context

An indexer writes a store while a UI or a CLI reads it. Until now every read-only open took the exclusive `LOCK`, so one reader excluded the writer and every other reader. The scenarios this must satisfy:

- (a) A reader opens while a writer commits 10-key batches and flushes and compacts continuously, and sees whole batches only.
- (b) A reader that keeps its handle moves to newer commits without reopening.
- (c) A second writer is still refused (`Locked`).
- (d) Damage is still reported, never skipped or repaired.

The on-disk format must not change.

## Options considered

1. **A shared lock on `LOCK`.** The `Vfs` has only an exclusive `try_lock`, and a shared `flock` cannot be taken while the writer holds the exclusive one, so it would let readers in beside each other but not beside the writer.
2. **No lock for readers, one consistent read.** Everything a reader needs is immutable or append-only: a table never changes after it is written, a WAL only grows, the MANIFEST only grows until a checkpoint replaces it by an atomic rename. A reader that reads the MANIFEST, then the tables and the WALs it names, sees a consistent state or an error that says a file moved.
3. **Copy the directory.** Cost proportional to the database, and a copy of a directory being written has the same problem.

## Decision criteria

Scenarios (a)–(d) above, no format change, and a reader that never blocks or delays the writer.

## Decision

Option 2. `AccessMode::ReadOnly` takes **no lock**. (An unknown `ro_compat` bit, which asked to write, still takes the exclusive `LOCK` like any read-write open.) A read-only open runs one *read* (`read_view`): list the directory, read the MANIFEST, scan and classify (REC-5..13), open the tables, replay the live WALs in memory.

A flush, a compaction, a WAL rotation or a checkpoint by the writer between two of those steps shows up as an error (a listed table or log is gone, a rule of the table does not hold). So an error is judged against the directory itself:

- The *fingerprint* is the listing of the files plus the size of the MANIFEST. Every flush, compaction, rotation and checkpoint changes it; a commit does not (it grows a WAL, which the read tolerates as a torn tail, REC-10).
- If the read fails and the fingerprint is the same after as before, nothing moved under the reader: the error is **real damage** and is returned at once.
- If the fingerprint changed, the read starts again, for up to 30 seconds in all; then the last error is returned. Tables opened by an attempt that failed stay open for the next one (immutable files, numbers never reused), so a retry only opens what the writer made since, and a retry gets cheaper as the writer's progress per retry shrinks. A bound on the *number* of attempts was tried first (64) and was not enough: against a writer that flushed every few milliseconds, one read in about forty used all of them and failed with `NotFound`.

Damage is therefore never skipped or repaired, and costs one read (three listings), not 64.

A successful read is consistent: the tables are those of one MANIFEST (immutable files held open, so a deleted file stays readable through its descriptor), and the WALs are replayed from that MANIFEST's `log_number` on, so the state is a prefix of the commit sequence.

**Refresh.** `KernoDb::refresh()` runs the same read on a read-only handle (`InvalidArgument` on any other), reuses the tables it already has (by file number) and swaps the published version and sequence. Snapshots taken before keep their own version. The sequence can go *down* only if a writer crashed and lost commits it never made durable.

**What a reader sees.** The commits the writer has *written* to the WAL, durable or not (`Durability::Buffered` commits and the unsynced tail of a group are visible once in the page cache): a reader does not wait for the writer's `fsync`. A reader never sees part of a batch (a frame is one batch and a torn frame is the end of the log). Orphans and a `MANIFEST.tmp` of a writer at work are reported like those of a crashed one and never deleted.

`verify()` and `inspect()` keep the exclusive `LOCK`: they are for a store nobody writes.

## Consequences

- Easy: UIs, CLIs and exports read a live store; readers never block each other or the writer.
- Hard: a reader's file descriptors keep the writer's deleted tables alive on disk until the reader's snapshot or handle goes (on Windows the name of a deleted open file stays until the reader closes it, and the `Deleter` retries a deletion that fails). A reader that is never refreshed keeps its old tables.
- Risks: the 30 s bound is arbitrary; a writer that changes the files faster than a read can catch up for that long starves it, and the open fails with the last error (a vanished file is then reported as `Io(NotFound)`, which is the file the reader raced, not damage).
- Format: unchanged. Only the wording of RO-1 in v1.md §9.5 changes (it no longer says a read-only open takes the exclusive `LOCK`).
- A read-only `UnitDb` (and so `kerno::Db`) has no refresh of its own and keeps caches built from what it read (its result cache among them): it moves to newer commits by reopening. Calling the engine's `refresh()` under it is not supported.
- Related, same change: `verify()` reports damaged blocks as key ranges (`VerifyReport::damaged`; `kerno_units::damaged_spans` turns a range into record kinds and unit ids) and `UnitDb::compact` drops the payloads of deleted units that no retention rule keeps (rule in `crates/kerno-units/src/retention.rs`).

## Evidence

- `crates/kerno-storage/tests/read_only.rs`: no lock (and the `LOCK` kept by a handle that is read-only for an unknown `ro_compat` bit), refresh (and its refusal of another database in the directory), a writer replacing every table while the open runs, damage reported without a retry.
- `crates/kerno-storage/tests/read_only_processes.rs`: a writer process and reader processes (whole batches, sequences that never go back, a second writer `Locked`).
- `crates/kerno-storage/tests/process_lock.rs`, `crates/kerno/tests/concurrency.rs`, `crates/kerno-cli/tests/cli.rs`.

## Follow-up

- **Texts that state the lock behaviour** were updated with this decision: `docs/concurrency.md` (§Processes and the test table), `docs/FEATURES.md` (rows ENG-10, FUT-01, STO-12, REV-10 and the counts) and the doc comments of `Db`, `Db::delete`, `Db::read_as_of` and `Db::compact` in `crates/kerno/src/lib.rs`. These texts outside what this change could touch still state the old behaviour and need the same correction: `docs/COMPLETION-PLAN.md` (item T1, rows ENG-10 and FUT-01: a read-only open gets `Locked`), `ROADMAP.md` line 51 (one process owns a directory), `docs/backup.md` and `crates/kerno-cli/README.md` (`kerno backup` needs the database closed; it now opens read-only beside a holder), `docs/testing/test-plan.md` and `docs/architecture/phase1-design.md` (`process_lock.rs` checks `Locked` on a read-only open, and constraint C-15; a read-only open now takes no lock), and `bindings/python/README.md` (`read_only=True` still raises `LockedError`).
- Expose `refresh` through the `kerno` facade and the bindings if callers need it (not done here).
- Revisit the retry bound if a real writer ever starves a reader.
