# Getting started with Kerno

Kerno is an embedded database of **units** (records with a type and content), the **relations** between them, and queries over both. This guide covers how to install the `kerno` crate, open a database, store and retrieve memory in five minutes, and where the three complete examples are.

Status: there is no version 1.0 and no promise of API stability (see the [README](../README.md#status)). The "published on crates.io" entry in the [roadmap](../ROADMAP.md) is still unchecked, so for now the dependency is added by path.

## Requirements

- Rust 1.89 or later (the `rust-version` of the workspace). This guide was checked with Rust 1.95.
- macOS or Linux. This guide and the examples were run on macOS. On Linux the code is the same, but it has not been run here.

## Install

From a checkout of the repository, add the dependency to your `Cargo.toml`:

```toml
[dependencies]
kerno = { path = "path/to/kernodb/crates/kerno" }
```

## Open a database

```rust
let db = Db::open("mi-base")?;
```

`Db::open` creates the directory if it does not exist and opens it if it does. A database belongs to one process at a time: a second `Db::open` on the same directory fails while the first is alive. `db.close()` closes the database; if the `Db` is dropped without closing it, it is also closed, but no errors are reported.

### Durability and macOS

`Db::open` requests `Durability::Sync`: when a commit returns, it is already on stable storage. That guarantee is **verified only on Linux** (ext4, xfs). On macOS the engine rejects `Sync` unless it is allowed to proceed, and `Db::open` allows it. So `Db::open` works on macOS, but there durability is not verified.

The case that fails is building the configuration by hand with `Db::open_with` and the default `Config`, which requests `Sync` without permission:

```text
Error: Storage(InvalidArgument("durability not verified on this platform"))
```

On macOS there are two ways out:

- `Config { durability: Durability::Buffered, ..Config::default() }`: commits do not wait for the disk. A power failure can lose the last ones.
- `Config { allow_unverified_platform: true, ..Config::default() }`: keeps `Sync`, without the Linux verification.

## Five minutes: store and retrieve

```rust
use kerno::{Db, Direction, UnitFilter};

fn main() -> Result<(), kerno::Error> {
    let db = Db::open("my-base")?;

    let note = db.store("note", "Rust has no GC")?;
    let topic = db.store("topic", "memory")?;
    db.relate(note, topic, "about")?;

    let unit = db.get(note)?.expect("just stored");
    println!("payload: {}", String::from_utf8_lossy(unit.payload.as_deref().unwrap_or_default()));

    let notes = db.query(&UnitFilter { unit_type: Some("note".into()), ..UnitFilter::default() })?;
    println!("notes: {}", notes.len());

    let edges = db.related(note, Direction::Out, Some("about"))?;
    println!("related to the topic: {}", edges[0].to == topic);

    db.close()?;
    Ok(())
}
```

Real output (first run, with an empty database):

```text
payload: Rust has no GC
notes: 1
related to the topic: true
```

What each call returns:

- `store(type, content)` stores a unit and returns its `UnitId`. The type is 1 to 64 bytes of UTF-8; the content is bytes.
- `get(id)` returns `Option<Unit>`: `None` if it does not exist.
- `relate(from, to, type)` creates a relation and returns its `RelationId`. The type is 1 to 256 bytes, without NUL. Two relations of the same type between the same ends are allowed.
- `related(unit, Direction::Out | Direction::In, Some(type))` lists the relations that leave the unit (`Out`) or arrive at it (`In`).
- `query(&UnitFilter)` lists the active units that match the filter. `UnitFilter` filters by `unit_type` and `schema_version`; there is no search by content.

Unit and relation identifiers are UUIDv7, which start with the timestamp. `query` returns the units in unit id order and `related` returns the relations of a type in relation id order, so within one process both follow creation order; the chat history and document examples depend on that order and check it.

### Change and delete

```rust
use kerno::{Batch, Db};

fn main() -> Result<(), kerno::Error> {
    let db = Db::open("my-base")?;
    let note = db.store("note", "first draft")?;
    let old = db.store("note", "obsolete")?;

    db.update(note, "second draft")?; // new revision of the content

    // One commit: either everything is applied or nothing is written.
    let mut batch = Batch::new();
    batch.update(note, "final version").delete(old).store("note", "new");
    let new_ids = db.commit(&batch)?; // ids of what was stored, in order
    println!("saved: {}", new_ids.len());
    db.close()?;
    Ok(())
}
```

- `update(id, content)` replaces the content of an active unit (`update_document` does the same with a `Document` and respects the schema). The full-text index is updated in the same commit. An unknown id gives `NotFound`.
- `delete(id)` deletes the unit: `get` returns a tombstone (`Lifecycle::Deleted`, without content), `query` and word search no longer find it, and the history is kept. If a relation still touches it, it fails with `UnitHasRelations`: remove the relation first with `unrelate(relation)`.
- `commit(batch)` applies a `Batch` (`store`, `update`, `delete`, `unrelate`) as a single commit and returns the ids of the stored units. If one operation fails, nothing is written. A unit can appear in only one operation of the batch.
- A `Batch` is data: it holds no lock, so it can be built while the same thread keeps calling `store` or `get`. A `ChangeSet` opened with `unit_db()` does hold the write turn until it ends, and a `store` from the same thread would wait forever.

What the facade does not cover is in `kerno::units` (the complete `kerno-units` layer) and in `Db::unit_db()`, which gives access to the `UnitDb` that wraps `Db`. According to the [crate documentation](../crates/kerno/src/lib.rs), that includes history, reads at a past instant, change sets with several operations, associative queries, export and import, and verification.

## Three complete examples

Each example uses only the public API of `kerno`, stores its database in the system temporary directory, checks its results with `assert!`, and deletes its database when it finishes. They are run from the root of the repository:

| Example | What it shows |
|---|---|
| [`agent_memory`](../crates/kerno/examples/agent_memory.rs) | Facts an agent learns, linked to their topics; retrieval by topic after closing and reopening the database |
| [`chat_history`](../crates/kerno/examples/chat_history.rs) | Messages as units, a conversation as a unit, answers linked to their question; the conversation is replayed in order |
| [`document_store`](../crates/kerno/examples/document_store.rs) | Documents with their paragraphs as related units; search in the paragraphs and reconstruction of the document |
| [`chat-history`](../examples/chat-history/README.md) | Workspace crate: turns per session, last turns and search over past turns, using only `Db` methods (`cargo run -p chat-history`) |
| [`document-store`](../examples/document-store/README.md) | Workspace crate: documents with metadata, query by field and search; updating and deleting go through `Db::unit_db()` (`cargo run -p document-store`) |

```text
cargo run -p kerno --example agent_memory
```

Real output of `agent_memory`:

```text
about deploy:
  - The staging deploy runs on Fridays at 17:00.
  - Rollback is done with the previous image tag.
about auth:
  - Tokens expire after 15 minutes.
memories stored: 3
```

Real output of `chat_history` (`cargo run -p kerno --example chat_history`):

```text
user_message        Does Kerno need a server?
assistant_message   No: it is an embedded library.
user_message        Can two processes open it?
assistant_message   One process holds a directory at a time.
answers linked to their question: 2
```

Real output of `document_store` (`cargo run -p kerno --example document_store`):

```text
[Backup runbook] Backups run every night at 02:00 and are kept for thirty days.
[Backup runbook] To restore, stop the service and copy the latest backup back into place.
rebuilt runbook matches the original: true
documents: 2
```
