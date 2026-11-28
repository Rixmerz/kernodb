# kerno

Kerno is an embedded database of **units** (typed, versioned records), the **relations**
between them, and queries over both. This crate is the one entry point: `kerno::Db`.

> **Alpha (1.0.0-alpha.3).** It does not open databases written by 1.0.0-alpha.4 or any earlier
> alpha: see [Upgrading from an earlier alpha](#upgrading-from-an-earlier-alpha). The on-disk format v1 is frozen
> ([ADR-0013](https://github.com/Rixmerz/kernodb/blob/main/docs/adr/0013-format-v1-and-engine-protocol.md));
> the test vectors fail if one of its bytes changes. Narrative search is an experiment and is not part of
> this crate. Benchmarks and long fuzzing runs are not yet certified. There is no 1.0 stability promise for the API yet.

## Install

```sh
cargo add kernodb@1.0.0-alpha.3
```

## Quick start

Run it once in a new directory: the assertions expect an empty database.

```rust
use kerno::{Db, Direction, UnitFilter};

fn main() -> Result<(), kerno::Error> {
    let db = Db::open("my-db")?;

    let note = db.store("note", "Rust has no GC")?;
    let topic = db.store("topic", "memory")?;
    db.relate(note, topic, "about")?;

    let unit = db.get(note)?.expect("just stored");
    assert_eq!(unit.payload.as_deref(), Some(&b"Rust has no GC"[..]));

    let notes = db.query(&UnitFilter { unit_type: Some("note".into()), ..UnitFilter::default() })?;
    assert_eq!(notes.len(), 1);
    let edges = db.related(note, Direction::Out, Some("about"))?;
    assert_eq!(edges[0].to, topic);

    db.close()?;
    Ok(())
}
```

The same example, with comments, is the first doctest of the crate documentation (`cargo doc -p kerno --open`).
Everything the layers below offer is still reachable: `kerno::units` is `kerno-units`, `kerno::storage` is
`kerno-storage`, and `Db::unit_db` hands out the underlying `UnitDb`.

To administer a database directory from a shell, use the `kerno` command-line tool, which is not published
to crates.io in this alpha.

## Keyword search (opt-in per type)

No unit type is keyword-indexed (BM25) until you turn it on. The setting is stored in the
database, and turning it on for a type that already has units indexes them in the same commit:

```rust
db.set_fulltext("note", true)?;
let rust = db.store("note", "Rust has no garbage collector")?;
db.store("config", "garbage collector settings")?; // not indexed, never a hit
let hits = db.search_text("garbage collector", 10, None)?; // best first, every indexed type
assert_eq!(hits[0].unit, rust);
// Only some unit types: `UnknownType` for a type nothing ever used, `NotIndexed` for one that is off.
let notes = db.search_text("garbage collector", 10, Some(&["note"]))?;
assert_eq!(notes.len(), 1);
```

Indexing costs writes several times over and about 3.5x the disk, so turn it on only for the types
you search by text. `db.fulltext(type)` says whether a type is indexed.

## Hybrid retrieval

`Db::hybrid_search` answers one query over keywords (BM25), vectors and relations, fused by weighted
reciprocal rank fusion (`k = 60`; each leg has weight 1 by default, and a weight of 0 switches that leg off).
The keyword leg reads only the types turned on with `set_fulltext`, and `HybridQuery::text_types` narrows it to some of them.
With no indexed type the leg raises `NotIndexed` instead of answering no hits.
The vector leg is an index you own and keep in step with your units: a `kerno::units::AnnIndex` (HNSW) or,
for exact search, a `kerno::units::assoc_rank::VectorSet`. Units that are not active are never hits.
A complete query is in `crates/kerno/tests/db.rs` (`hybrid_search_fuses_text_hnsw_and_graph`).

## Schemas and indexes

A unit type can have a schema: typed fields (`Bool`, `Int`, `Float`, `Text`, `Bytes`) that may be
`required`, `unique` or `indexed`. Once it does, every unit of that type holds a `kerno::Document`,
a write the schema refuses is rejected whole with `Error::SchemaViolation` (nothing is stored), and an
indexed field is looked up with `Db::find` and `Db::find_range`. The index is written in the same
transaction as the unit, so it never disagrees with it after a crash, and `verify_units` checks it.

```rust
use kerno::{Db, Document, FieldDef, FieldType, FieldValue};

fn main() -> Result<(), kerno::Error> {
    let db = Db::open("users-db")?;
    db.define_schema("user", vec![
        FieldDef::new("email", FieldType::Text).unique().required(),
        FieldDef::new("age", FieldType::Int).indexed(),
    ])?;
    let ada = Document::new()
        .with("email", FieldValue::Text("ada@example.com".into()))
        .with("age", FieldValue::Int(36));
    let id = db.store_document("user", &ada)?;
    assert_eq!(db.find("user", "age", &FieldValue::Int(36))?[0].id, id);
    db.close()?;
    Ok(())
}
```

A type has one schema for good (indexes can be added with `Db::create_index` and dropped with
`Db::drop_index`). The first schema raises the layer mark to 6, so a build that predates schemas
refuses to open the database instead of writing records it would not validate or index. The format and its limits are in
[ADR-0021](https://github.com/Rixmerz/kernodb/blob/main/docs/adr/0021-typed-schemas-and-secondary-indexes.md).

## Verifying integrity

A lost index entry or adjacency entry makes a read return fewer results, with no error (accepted
by design). `Db::verify` finds it: it checks the whole store on one snapshot and returns a
`VerifyReport` (`is_clean`, `findings`). Run it after a crash that leaves the index status
`Failed`, before an export, and on a schedule. `db.unit_db().rebuild_index()` repairs what it reports about the
built-in type index.

That index is written only by `rebuild_index`: an ordinary write leaves it `Stale`, and queries
fall back to the reference path until the next rebuild. After a crash the status survives, the
query falls back, and the owner rebuilds; nothing rebuilds it on open.

## Retrying a revision conflict

An update, archive or delete on a change set can carry the revision the caller read
(`Some(unit.head)`); if another writer committed in between it fails with `Error::Conflict`.
The core has no retry policy: read the unit again, redo the edit and commit. The loop, as a
doctest, is in the crate documentation.

## Durability

`Db::open` asks for `Durability::Sync` (every commit is on stable storage when it returns). The
guarantee is verified on Linux (ext4, xfs) only; elsewhere the engine refuses `Sync` unless told
it may, and `Db::open` tells it so. Use `Db::open_with` and a `Config` to choose otherwise.

## Queries

`Db::execute` runs a declarative query over those typed units: filters, ordering, limits, and
joins across unit types through typed relations, using an index when it is cheaper. `Db::explain`
shows the plan. The grammar and the meaning of every construct are in
[the query language reference](https://github.com/Rixmerz/kernodb/blob/main/docs/query-language.md).

A query needs its types to have schemas, because it reads typed fields:

```rust
use kerno::{Db, Document, FieldDef, FieldType, FieldValue};

fn main() -> Result<(), kerno::Error> {
    let db = Db::open("query-db")?;
    db.define_schema("user", vec![
        FieldDef::new("email", FieldType::Text).unique().required(),
        FieldDef::new("age", FieldType::Int).indexed(),
    ])?;
    db.define_schema("team", vec![FieldDef::new("name", FieldType::Text).required()])?;
    let ada = db.store_document(
        "user",
        &Document::new()
            .with("email", FieldValue::Text("ada@example.com".into()))
            .with("age", FieldValue::Int(36)),
    )?;
    let ops = db.store_document("team", &Document::new().with("name", FieldValue::Text("ops".into())))?;
    db.relate(ada, ops, "member_of")?;

    let rows = db.execute(
        "FIND user AS u JOIN team AS t VIA member_of WHERE u.age >= 30 ORDER BY u.age DESC LIMIT 10",
    )?;
    for row in &rows.rows {
        let (user, team) = (&row[0], &row[1]); // one unit per binding, in the order written
        let _ = (user, team);
    }
    println!("{:?}", db.explain("FIND user WHERE email = \"ada@example.com\"")?.chosen());
    db.close()?;
    Ok(())
}
```

`kerno <dir> query '...'` runs the same text from the command line.

## License

Licensed under either of the Apache License, Version 2.0 or the MIT license, at your option. The license texts are in the package (`LICENSE-APACHE`, `LICENSE-MIT`).

## Storage

New tables are LZ4-compressed when that makes them smaller (on by default:
`kerno::Config::block_compression`). On a real store that halved the disk. Payloads are at most
1 MiB, and a commit holds at most 10,000 operations unless the database is opened with
`Db::open_with_limits`.

## Upgrading from an earlier alpha

This build refuses a database of units layer 11 or below (`Error::LayerTooOld`), which is every
database written by 1.0.0-alpha.1 to 1.0.0-alpha.4, and changes nothing in it. To carry one over:

```sh
kerno old-db export old.kexp        # with the CLI of the build that wrote old-db
kerno import old.kexp new-db        # with this build's CLI
```

Then call `db.set_fulltext(type, true)` for every type you search by text, and redefine schemas
and indexes: an export holds neither. The details are in
[the compatibility document](https://github.com/Rixmerz/kernodb/blob/main/docs/compatibility.md).
