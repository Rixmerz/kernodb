# chat-history

Conversation turns per session, on the public API of the `kerno` crate and through `Db` methods only. A workspace member, so `cargo test --workspace` runs it.

## What it does

- keeps one `session` unit per conversation (schema: unique `name`) and one `user_turn` / `assistant_turn` unit per turn, with the text as a plain UTF-8 payload;
- appends a turn to a session (`append`);
- lists the most recent turns, a page at a time, with a cursor to go back (`recent`);
- searches the past turns of one session by keyword (`search`, BM25).

## Where the facade stops

- Keyword search is global: `Db::hybrid_search` has no type or relation filter and returns at most the best 1000 hits, so hits are filtered to the session afterwards. A match of the session ranked below 1000 globally is missed.
- `append` is two commits (the turn, then its `in_session` relation). A crash between them leaves an orphan turn, which is never listed or found, because both reads go through the relation.
- Turns are ordered by the relation's `created_seq`, which persists and only grows.
- No stemming: "backups" does not match "backup".

## Run

```bash
cargo run -p chat-history            # a narrated demo
cargo test -p chat-history --release # the scenario, with close and reopen, and the demo
```

Real output of the demo:

```text
page 1:
  user      How do I delay it?
  assistant Move the backup to 03:00 in the schedule.
  user      Done, thanks.
page 2:
  assistant Which time zone is the server in?
  user      UTC. The backup is set for 02:00.
  assistant Then the job ran before the disk was mounted.
page 3:
  user      My nightly backup did not run.
search "backup" in support-42:
  user      My nightly backup did not run.
  user      UTC. The backup is set for 02:00.
  assistant Move the backup to 03:00 in the schedule.
sessions keep their turns apart: true
```

## Layout

| File | Holds |
|---|---|
| `src/lib.rs` | `ChatHistory`: open, session, append, recent, search |
| `src/main.rs` | the demo |
| `tests/scenario.rs` | the scenarios and a test that runs the demo binary |
