# document-store

Documents with metadata, on the public API of the `kerno` crate. A workspace member, so `cargo test --workspace` runs it.

## What it does

- stores a document as a `document` unit typed by a schema (unique `slug`, `title`, indexed `author`, `status` and `year`) and its body as a plain-text `document_body` unit related to it;
- inserts, updates and deletes a document and its body as one change set each;
- queries by field (`by_author`, `by_year` with `Db::find` and `Db::find_range`), with the query language (`published`, `Db::execute`), and by keyword over the bodies (`search`).

## Where the facade stops

- **Update and delete go through `Db::unit_db()` today.** `Db` has no update, delete or multi-operation commit, so those writes are change sets of the units layer (`begin_changeset`, then `create`, `update`, `delete`, `delete_relation`, `commit`), the route the crate documentation points to. Every read and the schema still use `Db` methods. A change set holds the single write slot, so the example reads first and never calls a `Db` write while one is open.
- A unit with relations cannot be deleted, so delete removes the body's relation, the body and the document in one change set.
- Full-text search reads only plain UTF-8 payloads, not typed fields, so bodies are separate units and search results are filtered to bodies.
- Keyword search is global (the best 1000 hits), and has no stemming.
- `Db::execute` takes no bind parameters, so the example runs constant query text only; caller values go through `find` and `find_range`.
- `Db::store` cannot set a unit's raw `metadata`; the metadata here are the schema fields.

## Run

```bash
cargo run -p document-store            # a narrated demo
cargo test -p document-store --release # the scenarios, with close and reopen, and the demo
```

Real output of the demo:

```text
get: Backup runbook by ops (2024)
by author ops: ["backup-runbook", "upgrade-notes"]
by year 2024..=2025: ["backup-runbook", "upgrade-notes"]
published: ["backup-runbook", "retention-policy"]
search "backup": ["upgrade-notes", "backup-runbook"]
after update, search "backup": ["backup-runbook"]
documents left: 2
```

## Layout

| File | Holds |
|---|---|
| `src/lib.rs` | `DocumentStore`: insert, get, update, delete, by_author, by_year, published, search |
| `src/main.rs` | the demo |
| `tests/scenario.rs` | the scenarios and a test that runs the demo binary |
