# agent-memory

A small example of memory for an agent, written as a third party would write
it: a standalone crate that depends on the repository's crates by path and
uses only their public API. It is not a member of the repository workspace and
has its own `Cargo.lock`.

What it does:

- stores a memory as a unit (text as payload, tags as metadata);
- persists a vector the caller supplies; the example's `standin::Embedder` is a
  seeded, deterministic stand-in for a model (no model, no network);
- recalls by vector neighbourhood (cosine, exact) and by time;
- corrects a memory (a new revision): its vector is reported `Stale` and is not
  recalled until it is rebuilt;
- forgets a memory: its vector and any summary over it go in the same commit.
  The removal is logical. `reclaim` then compacts the live files, which moves
  the moment at which they stop holding what was forgotten; it promises no
  physical erasure (ADR-0009, option 3: a snapshot taken before the deletion
  keeps its files, and the engine does not overwrite the blocks it frees);
- shows the retained revisions of a memory (`history`) and prunes the old ones
  (`prune`, retention with `KeepLast(1)`, the catalog included);
- summarises a set of memories with the test summariser (`TruncatingSummarizer`);
  the originals stay readable.

`FRICTION.md` lists, with code, every place where the public API was awkward,
lacked something or was confusing. It is the main result of the example.

## Run

From this directory (`examples/agent-memory`):

```bash
cargo run --release -- demo /tmp/agent-memory-demo     # the whole scenario, narrated
cargo test --release                                   # the scenario as tests, with close and reopen
```

The demo directory must not be in use by another process (the database is
locked); delete it to start over.

Command line, one command per invocation (the database is opened and closed
each time):

```bash
cargo run --release -- /tmp/mem remember "the staging token rotates on friday" --tag ops
cargo run --release -- /tmp/mem list
cargo run --release -- /tmp/mem recall "staging token" 3
cargo run --release -- /tmp/mem recent 2
cargo run --release -- /tmp/mem between <from_microseconds> <to_microseconds>
cargo run --release -- /tmp/mem correct 2 "the staging token rotates on monday"
cargo run --release -- /tmp/mem show 2          # vector: Stale
cargo run --release -- /tmp/mem rebuild 2       # vector: Ready
cargo run --release -- /tmp/mem summarize 2 3
cargo run --release -- /tmp/mem summaries
cargo run --release -- /tmp/mem history 2
cargo run --release -- /tmp/mem forget 3
cargo run --release -- /tmp/mem prune
cargo run --release -- /tmp/mem reclaim         # compacts the live files; no erasure promise
```

Memory numbers start at 2 (number 1 is the catalog, see `FRICTION.md` items 1
and 6).

## Layout

| File | What |
|---|---|
| `src/lib.rs` | `AgentMemory`: the library |
| `src/catalog.rs` | the catalog unit: the next memory number and the summary sources |
| `src/standin.rs` | the seeded stand-in for an embedding model |
| `src/main.rs` | the command line and the demo |
| `tests/scenario.rs` | the scenario end to end, `reclaim`, the lock, history |
| `tests/legacy.rs` | a directory written by the previous version of the example opens |
| `FRICTION.md` | what the public API made hard |

## Compatibility with the previous version

The previous version wrote a `mem <n> <vec|->` line per memory in the catalog.
The catalog no longer holds them: memories are listed with `DerivedDb::query`
and their vectors found with `DerivedDb::derived_of`. `Catalog::parse` still
accepts those lines and drops them, so a directory written by the previous
version opens with no migration (`tests/legacy.rs`); the next write of the
catalog leaves them out.

## Limits of the example

- **One engine.** The two-engine rule of the stage 9 acta (tests on
  `emulated_log` and `kerno_db`, section 4.3) applies to tests inside the
  workspace crates. This example does not depend on `kerno-testkit` and its
  `Cargo.toml` and `Cargo.lock` stay untouched, so its tests run on `kerno_db`
  only. Decision taken by the owner's delegate on 2026-11-12; revocable.
- **Summaries are memory-only**, so their definitions live in the catalog.
- **`reclaim` is a mechanism, not a guarantee**: see above.
- The catalog is one unit payload (at most 1 MiB), rewritten on every write.

## Checks

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --release
```
