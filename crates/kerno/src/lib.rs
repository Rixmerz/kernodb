//! Kerno: an embedded database of **units** (typed, versioned records), the **relations**
//! between them, and queries over both. [`Db`] is the one entry point.
//!
//! ```
//! use kerno::{Db, Direction, UnitFilter};
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//!
//! // Store units: a type and an opaque payload each.
//! let note = db.store("note", "Rust has no GC")?;
//! let topic = db.store("topic", "memory")?;
//!
//! // Read one back.
//! let unit = db.get(note)?.expect("just stored");
//! assert_eq!(unit.unit_type, "note");
//! assert_eq!(unit.payload.as_deref(), Some(&b"Rust has no GC"[..]));
//!
//! // Relate two units, then follow the relation.
//! db.relate(note, topic, "about")?;
//! let edges = db.related(note, Direction::Out, Some("about"))?;
//! assert_eq!(edges[0].to, topic);
//!
//! // Query by type.
//! let notes = db.query(&UnitFilter { unit_type: Some("note".into()), ..UnitFilter::default() })?;
//! assert_eq!(notes.len(), 1);
//!
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # Changing and removing units
//!
//! [`Db::update`] replaces a unit's payload and [`Db::delete`] removes it (a tombstone stays, and
//! the keyword index drops it). [`Db::commit`] applies a [`Batch`] of stores, updates, deletes and
//! relation removals as one commit: a reader sees all of it or none, and a failing operation
//! writes nothing. A [`Batch`] holds no lock, so these methods never deadlock on one thread;
//! a [`units::ChangeSet`] from [`Db::unit_db`] does hold the write slot until it ends.
//!
//! ```
//! use kerno::{Batch, Db, Lifecycle};
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-batch-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//! let draft = db.store("note", "first draft")?;
//! let stale = db.store("note", "outdated")?;
//!
//! db.update(draft, "second draft")?;
//!
//! // One commit: edit one unit, delete another, add a third.
//! let mut batch = Batch::new();
//! batch.update(draft, "final").delete(stale).store("note", "new");
//! let stored = db.commit(&batch)?;
//! assert_eq!(stored.len(), 1);
//! assert_eq!(db.get(stale)?.expect("tombstone").lifecycle, Lifecycle::Deleted);
//!
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # What is here
//!
//! The methods of [`Db`] cover the common path. Everything the layers below offer stays
//! reachable: [`units`] is the whole of `kerno-units` (history, as-of reads, change sets with
//! several operations, associative queries, export and import, verification), [`storage`] is the
//! whole of `kerno-storage` (the engine and its [`Config`]), and [`Db::unit_db`] is the
//! [`UnitDb`] a [`Db`] wraps.
//!
//! [`Db::hybrid_search`] answers one query over keywords (BM25), vectors (an index you own) and
//! relations, fused by weighted reciprocal rank fusion.
//!
//! # Keyword search is opt-in per type
//!
//! No unit type is in the keyword (BM25) index until [`Db::set_fulltext`] turns it on: indexing
//! costs writes several times over and most of the disk for data nobody searches by text. The
//! setting is stored in the database. Turning it on for a type that already has units indexes them
//! in the same commit. [`Db::search_text`] and the keyword leg of [`Db::hybrid_search`] find only
//! the indexed types.
//!
//! ```
//! use kerno::Db;
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-fulltext-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//! db.set_fulltext("note", true)?;
//! let rust = db.store("note", "Rust has no garbage collector")?;
//! db.store("config", "garbage collector settings")?; // not indexed, never a hit
//!
//! let hits = db.search_text("garbage collector", 10, None)?;
//! assert_eq!(hits.len(), 1);
//! assert_eq!(hits[0].unit, rust);
//! assert!(db.fulltext("note")? && !db.fulltext("config")?);
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # Upgrading from 1.0.0-alpha.1
//!
//! 1.0.0-alpha.2 does not open a database written by alpha.1 ([`Error::LayerTooOld`]). Export
//! it with the alpha.1 CLI (`kerno <dir> export <file>`), import it with alpha.2 (`kerno import
//! <file> <new-dir>`), then call [`Db::set_fulltext`] for the types you search. New tables are
//! LZ4-compressed ([`Config::block_compression`]).
//!
//! # Schemas and indexes
//!
//! A unit type can have a [`Schema`]: typed fields that may be required, unique or indexed. Once it
//! does, every unit of that type holds a [`Document`], a write the schema refuses is rejected whole
//! ([`Error::SchemaViolation`]), and an indexed field is looked up with [`Db::find`] and
//! [`Db::find_range`]. The indexes are kept in step with the units in the same transaction, and
//! `verify_units` checks them.
//!
//! ```
//! use kerno::{Db, Document, Error, FieldDef, FieldType, FieldValue};
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-schema-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//! db.define_schema("user", vec![
//!     FieldDef::new("email", FieldType::Text).unique().required(),
//!     FieldDef::new("age", FieldType::Int).indexed(),
//! ])?;
//!
//! let ada = Document::new()
//!     .with("email", FieldValue::Text("ada@example.com".into()))
//!     .with("age", FieldValue::Int(36));
//! let id = db.store_document("user", &ada)?;
//! assert_eq!(db.find("user", "age", &FieldValue::Int(36))?[0].id, id);
//!
//! // The same email again is refused, and nothing is written.
//! assert!(matches!(db.store_document("user", &ada), Err(Error::SchemaViolation { .. })));
//!
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # Natural keys: upsert
//!
//! Ids are random, so a logical key (an email, a path) needs a lookup. Declare the field `unique`
//! and [`Db::upsert_document`] (or [`Batch::upsert`], in the same commit as other writes) revises
//! the active unit that holds the same value of it, or stores a new one. The result says which
//! unit, and whether it was created, updated or already had the document.
//!
//! ```
//! use kerno::{Db, Document, FieldDef, FieldType, FieldValue, UpsertOutcome};
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-upsert-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//! db.define_schema("user", vec![FieldDef::new("email", FieldType::Text).unique(), FieldDef::new("name", FieldType::Text)])?;
//! let user = |name: &str| {
//!     Document::new().with("email", FieldValue::Text("ada@example.com".into())).with("name", FieldValue::Text(name.into()))
//! };
//!
//! let made = db.upsert_document("user", "email", &user("Ada"))?;
//! assert_eq!(made.outcome, UpsertOutcome::Created);
//! assert_eq!(db.upsert_document("user", "email", &user("Ada"))?.outcome, UpsertOutcome::Unchanged);
//! let newer = db.upsert_document("user", "email", &user("Ada L."))?;
//! assert_eq!((newer.id, newer.outcome), (made.id, UpsertOutcome::Updated));
//!
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # The query language
//!
//! [`Db::execute`] runs a declarative query over the typed units of a schema: filters, ordering,
//! limits, and joins across unit types through typed relations, using the indexes above when
//! they are cheaper. [`Db::explain`] shows how. The grammar is in `docs/query-language.md`.
//!
//! ```
//! use kerno::{Db, Document, FieldDef, FieldType, FieldValue};
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-query-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//! db.define_schema("user", vec![FieldDef::new("name", FieldType::Text), FieldDef::new("age", FieldType::Int).indexed()])?;
//! db.define_schema("team", vec![FieldDef::new("name", FieldType::Text).unique()])?;
//! let team = db.store_document("team", &Document::new().with("name", FieldValue::Text("core".into())))?;
//! for (name, age) in [("ada", 36), ("bob", 29), ("cy", 41)] {
//!     let doc = Document::new().with("name", FieldValue::Text(name.into())).with("age", FieldValue::Int(age));
//!     let user = db.store_document("user", &doc)?;
//!     db.relate(user, team, "member_of")?;
//! }
//!
//! let rows = db.execute(
//!     "FIND user AS u JOIN team AS t VIA member_of WHERE u.age >= 30 AND t.name = \"core\" ORDER BY u.age DESC",
//! )?;
//! assert_eq!(rows.aliases, ["u", "t"]);
//! assert_eq!(rows.rows.len(), 2);
//! assert!(rows.rows.iter().all(|row| row[1].id == team));
//!
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # Verifying integrity
//!
//! A lost index or adjacency entry makes a read return fewer results, without an error; that is
//! accepted by design. [`Db::verify`] finds it: it checks every record and index on one snapshot
//! and returns a [`VerifyReport`]. Run it after a crash that leaves the index status `Failed`,
//! before an export, and on a schedule. `rebuild_index` repairs the type index it reports.
//!
//! ```
//! use kerno::Db;
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-verify-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//! db.store("note", "checked")?;
//! assert!(db.verify()?.is_clean());
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # Retrying a revision conflict
//!
//! An update, archive or delete through a [`units::ChangeSet`] can carry the revision the caller
//! last read; when another writer committed in between it fails with `Error::Conflict`. The core
//! has no retry policy: read the unit again, redo the edit on what is there now, and commit.
//!
//! ```
//! use kerno::{Db, Error};
//! use kerno::units::{ChangeMeta, UnitUpdate};
//!
//! # let dir = std::env::temp_dir().join(format!("kerno-doc-retry-{}", std::process::id()));
//! # let _ = std::fs::remove_dir_all(&dir);
//! let db = Db::open(&dir)?;
//! let id = db.store("counter", "0")?;
//!
//! // A concurrent writer's commit between our read and our write is simulated by `rival`.
//! let mut rival = Some(|| db.update(id, "10"));
//! let mut attempts = 0;
//! loop {
//!     attempts += 1;
//!     let unit = db.get(id)?.expect("stored");
//!     let n: i64 = std::str::from_utf8(unit.payload.as_deref().unwrap_or_default()).unwrap().parse().unwrap();
//!     if let Some(write) = rival.take() {
//!         write()?; // someone else commits here, on the first attempt only
//!     }
//!     let mut changes = db.unit_db().begin_changeset(ChangeMeta::default())?;
//!     let upd = UnitUpdate { payload: (n + 1).to_string().into_bytes(), schema_version: None, metadata: None };
//!     // Pass the revision we read: a newer one makes this fail with `Conflict`.
//!     match changes.update(id, upd, Some(unit.head)) {
//!         Ok(_) => {
//!             changes.commit()?;
//!             break;
//!         }
//!         // Nothing was written: drop the change set, read again and go round.
//!         Err(Error::Conflict { .. }) if attempts < 5 => continue,
//!         Err(e) => return Err(e),
//!     }
//! }
//! assert_eq!(attempts, 2);
//! assert_eq!(db.get(id)?.unwrap().payload.as_deref(), Some(&b"11"[..]));
//! db.close()?;
//! # std::fs::remove_dir_all(&dir).expect("remove the doc database");
//! # Ok::<(), kerno::Error>(())
//! ```
//!
//! # Durability
//!
//! [`Db::open`] asks for [`Durability::Sync`]: a commit is on stable storage when it returns.
//! That guarantee is verified on Linux (ext4, xfs) only. Elsewhere the engine refuses `Sync`
//! unless told it may, and [`Db::open`] tells it so; use [`Db::open_with`] and a [`Config`] to
//! choose otherwise.

use std::borrow::Borrow;
use std::collections::HashSet;
use std::ops::Bound;
use std::path::Path;

use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_units::{
    AsOf, ChangeMeta, CommitSeq, NewUnit, RelationProvenance, RevNo, UnitAt, UnitConfig, UnitDb, UnitUpdate, upsert_key,
};

pub use kerno_core::Durability;
pub use kerno_storage as storage;
pub use kerno_storage::{AccessMode, Config};
pub use kerno_units as units;
pub use kerno_units::{
    AssocDirection, AssocQuery, Direction, Document, FieldDef, FieldType, FieldValue, HybridHit, HybridQuery,
    HybridWeights, LegRanks, Lifecycle, NearestUnits, Query, QueryPlan, QueryResult, Relation, RelationId, Schema,
    Unit, UnitError as Error, UnitFilter, UnitId, UnitVerifyReport as VerifyReport, VectorLeg,
};
pub use kerno_units::{
    CompactReport, RetentionPolicy, RetentionReport, RetentionRule, TextHit, UpsertOutcome, Upserted,
};

/// The result of every [`Db`] method.
pub type Result<T> = std::result::Result<T, Error>;

/// Schema version [`Db::store`] gives a unit; the layer keeps it with the unit and does not
/// interpret it.
const SCHEMA_VERSION: u32 = 1;

/// One end of a relation added by [`Batch::relate`]: a unit that exists already, or the unit
/// an earlier operation of the same batch creates.
///
/// `Op(n)` names the operation at position `n` of the batch (counting every operation added,
/// from 0, not only the stores). It must come before the relation and be a [`Batch::store`] or a
/// [`Batch::upsert`]; anything else is `BadEndpoint` and the batch writes nothing. An upsert
/// names the unit it created, revised or found unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Unit(UnitId),
    Op(usize),
}

impl From<UnitId> for Endpoint {
    fn from(id: UnitId) -> Self {
        Endpoint::Unit(id)
    }
}

/// Who and why, recorded with every revision of a commit ([`Db::commit_with`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitMeta {
    /// At most 1024 bytes.
    pub reason: Option<String>,
    /// At most 256 bytes.
    pub actor: Option<Vec<u8>>,
    /// At most 256 bytes.
    pub source: Option<Vec<u8>>,
}

/// What [`Db::commit_with`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Committed {
    /// The units the batch stored, in the order it stored them.
    pub units: Vec<UnitId>,
    /// The relations the batch created (or, for a unique relate, found), in order.
    pub relations: Vec<RelationId>,
    /// Updates and upserts skipped because the payload was already the current one.
    pub unchanged: usize,
    /// What each [`Batch::upsert`] did, in the order the batch added them. Upserts of one key
    /// share an entry's id and outcome.
    pub upserts: Vec<Upserted>,
    /// The sequence of the commit; `None` when nothing was written.
    pub commit_seq: Option<u64>,
}

/// One step of a [`Batch`].
enum Op {
    Store { unit_type: String, payload: Vec<u8> },
    Update { id: UnitId, payload: Vec<u8>, if_changed: bool },
    Delete { id: UnitId, cascade: bool },
    Unrelate { edge: RelationId },
    Relate { from: Endpoint, to: Endpoint, relation_type: String, unique: bool },
    Archive { id: UnitId },
    Unarchive { id: UnitId },
    Restore { id: UnitId, from: u64 },
    Upsert { unit_type: String, field: String, payload: Vec<u8> },
}

/// Several writes that [`Db::commit`] applies as one commit: all of them or none.
///
/// A `Batch` is plain data. It holds no lock and no open change set, so building one while
/// the same thread keeps calling [`Db::store`], [`Db::get`] or any other method cannot
/// deadlock; the write slot is taken only inside [`Db::commit`]. A unit can appear in one
/// operation of a batch (a second one is `InvalidArgument`), and the operations run in the
/// order they were added.
#[derive(Default)]
pub struct Batch {
    ops: Vec<Op>,
}

impl Batch {
    /// An empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a new unit, as [`Db::store`] would create it. [`Db::commit`] returns its id.
    pub fn store(&mut self, unit_type: &str, payload: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Store { unit_type: unit_type.to_owned(), payload: payload.into() });
        self
    }

    /// Adds a replacement of the payload of `id`, as [`Db::update`] would.
    pub fn update(&mut self, id: UnitId, payload: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Update { id, payload: payload.into(), if_changed: false });
        self
    }

    /// Like [`Batch::update`], but writes no revision when `payload` is already the unit's
    /// current payload ([`Committed::unchanged`] counts it).
    pub fn update_if_changed(&mut self, id: UnitId, payload: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Update { id, payload: payload.into(), if_changed: true });
        self
    }

    /// Adds the deletion of `id`, as [`Db::delete`] would.
    pub fn delete(&mut self, id: UnitId) -> &mut Self {
        self.ops.push(Op::Delete { id, cascade: false });
        self
    }

    /// Adds the deletion of `id` and, first, of every relation that starts or ends at it, in
    /// the same commit. A relation reached from two deleted units is removed once.
    pub fn delete_cascade(&mut self, id: UnitId) -> &mut Self {
        self.ops.push(Op::Delete { id, cascade: true });
        self
    }

    /// Adds a declared relation from `from` to `to`. Either end can be a unit an earlier
    /// operation of this batch stored or upserted ([`Endpoint::Op`], the operation's position).
    pub fn relate(&mut self, from: impl Into<Endpoint>, to: impl Into<Endpoint>, relation_type: &str) -> &mut Self {
        let (from, to) = (from.into(), to.into());
        self.ops.push(Op::Relate { from, to, relation_type: relation_type.to_owned(), unique: false });
        self
    }

    /// Like [`Batch::relate`], but reuses a relation of the same type between the same units
    /// when one exists (in the database or earlier in the batch) instead of adding a parallel one.
    pub fn relate_unique(
        &mut self,
        from: impl Into<Endpoint>,
        to: impl Into<Endpoint>,
        relation_type: &str,
    ) -> &mut Self {
        let (from, to) = (from.into(), to.into());
        self.ops.push(Op::Relate { from, to, relation_type: relation_type.to_owned(), unique: true });
        self
    }

    /// Adds the archiving of an active unit (a revision that keeps its payload).
    pub fn archive(&mut self, id: UnitId) -> &mut Self {
        self.ops.push(Op::Archive { id });
        self
    }

    /// Adds the unarchiving of an archived unit.
    pub fn unarchive(&mut self, id: UnitId) -> &mut Self {
        self.ops.push(Op::Unarchive { id });
        self
    }

    /// Adds a revision of `id` that copies revision `from` (later revisions stay in the history).
    pub fn restore(&mut self, id: UnitId, from: u64) -> &mut Self {
        self.ops.push(Op::Restore { id, from });
        self
    }

    /// Adds an upsert keyed by `field`, a field of `unit_type`'s schema declared `unique`: the
    /// active unit whose `field` equals the value `payload` (an encoded [`Document`]) holds in it
    /// gets `payload` as a new revision, or nothing is written when it has that payload already
    /// (counted in [`Committed::unchanged`]); when no active unit holds the value a new unit is
    /// stored. [`Committed::upserts`] says which happened and the id. Several upserts of one key
    /// in a batch resolve to one unit and the last payload wins, and each of them reports that
    /// unit's id and one outcome (`Created` when the first made it). The unit an upsert stores is
    /// not in [`Committed::units`], but it can be a relation end: [`Endpoint::Op`] names it by
    /// the position of the upsert in the batch.
    ///
    /// [`Db::commit`] fails with `InvalidArgument` when `field` is not declared unique (or the
    /// type has no schema), and with `SchemaViolation` when `payload` has no value in it.
    pub fn upsert(&mut self, unit_type: &str, field: &str, payload: impl Into<Vec<u8>>) -> &mut Self {
        self.ops.push(Op::Upsert { unit_type: unit_type.to_owned(), field: field.to_owned(), payload: payload.into() });
        self
    }

    /// Adds the removal of a relation, as [`Db::unrelate`] would. Put it before the
    /// [`Batch::delete`] of a unit the relation touches.
    pub fn unrelate(&mut self, edge: RelationId) -> &mut Self {
        self.ops.push(Op::Unrelate { edge });
        self
    }
}

/// An open Kerno database: units, the relations between them, and queries over both.
///
/// One process writes a database directory at a time; a read-write open of it while another `Db`
/// holds it fails at once (`Error::is_locked`). A read-only open (`Config::access` set to
/// `AccessMode::ReadOnly`) takes no lock: any number of reader processes read beside the writer
/// and see a consistent committed state, and to see newer commits a reader reopens. Every method
/// takes `&self`, and a `Db` can be shared between threads: any number of readers run while one
/// writer commits, and a read never sees half of a change set. The guarantees, the isolation
/// level and the limits are in `docs/concurrency.md`.
pub struct Db {
    units: UnitDb<KernoDb>,
}

impl Db {
    /// Opens the database in `path`, creating it when `path` holds none.
    ///
    /// Commits are durable ([`Durability::Sync`]); see the [crate documentation](crate#durability)
    /// for the platforms where that is verified.
    ///
    /// # Errors
    ///
    /// The directory cannot be created or read, another handle holds it, or it holds a damaged
    /// or newer-format database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, Config { allow_unverified_platform: true, ..Config::default() })
    }

    /// Opens the database in `path` with an explicit engine [`Config`]: read-only access,
    /// no creation of a missing database, buffered durability, cache sizes.
    ///
    /// # Errors
    ///
    /// As [`Db::open`], and `InvalidArgument` for a `Config` the engine refuses (for example
    /// `Durability::Sync` off Linux without `allow_unverified_platform`).
    pub fn open_with(path: impl AsRef<Path>, config: Config) -> Result<Self> {
        Self::open_with_limits(path, config, kerno_units::DEFAULT_MAX_CHANGESET_OPS)
    }

    /// [`Db::open_with`], with the most operations one change set (one [`Db::commit`]) may hold
    /// instead of the default 10,000. The engine's own limits on the bytes of one write still
    /// apply: a commit over them fails with `ChangeSetTooLarge` and writes nothing.
    ///
    /// # Errors
    ///
    /// As [`Db::open_with`]; `InvalidArgument` when `path` is empty, or `max_changeset_ops` is 0 or
    /// above the layer's maximum (2^20).
    pub fn open_with_limits(path: impl AsRef<Path>, config: Config, max_changeset_ops: usize) -> Result<Self> {
        // An empty path names the current directory to the file system calls below: refuse it before
        // they create a database (a LOCK and a MANIFEST) in the working directory.
        if path.as_ref().as_os_str().is_empty() {
            return Err(Error::InvalidArgument("the database path is empty".into()));
        }
        let mut layer = UnitConfig { max_changeset_ops, ..UnitConfig::default() };
        // The layer reports `CommitInfo::durable` from this, so it must match the engine's.
        layer.engine.durability = config.durability;
        let engine = KernoDb::open(StdVfs::shared(), path.as_ref(), &config)?;
        Ok(Db { units: UnitDb::from_engine(engine, &layer)? })
    }

    /// Stores a new unit of type `unit_type` (1 to 64 bytes of UTF-8) with `payload` as its
    /// content, in one committed change set, and returns its id.
    ///
    /// # Errors
    ///
    /// `unit_type` is empty or too long, the payload is larger than the limit, or the engine
    /// fails.
    pub fn store(&self, unit_type: &str, payload: impl Into<Vec<u8>>) -> Result<UnitId> {
        let new = NewUnit {
            unit_type: unit_type.to_owned(),
            schema_version: SCHEMA_VERSION,
            metadata: Vec::new(),
            payload: payload.into(),
        };
        Ok(self.units.create_unit(new, ChangeMeta::default())?.0)
    }

    /// Replaces the payload of the active unit `id` with `payload`: a new revision, with the unit's
    /// type, schema version and metadata kept. The full-text index follows in the same commit.
    ///
    /// # Errors
    ///
    /// The unit does not exist, is archived or deleted, the payload is larger than the limit, or
    /// the engine fails. An unknown id is `NotFound`.
    pub fn update(&self, id: UnitId, payload: impl Into<Vec<u8>>) -> Result<()> {
        self.commit(Batch::new().update(id, payload)).map(|_| ())
    }

    /// Stores `document` as a unit of `unit_type`, or revises the active unit that already holds
    /// the same value of the unique field `field` (see [`Batch::upsert`]), in one commit, and
    /// says which unit and whether it was created, updated or already had this payload.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when the type has no schema or `field` is not declared `unique` in it;
    /// `SchemaViolation` when the schema refuses the document or it has no value in `field`.
    pub fn upsert_document(&self, unit_type: &str, field: &str, document: &Document) -> Result<Upserted> {
        self.upsert(unit_type, field, document.encode()?)
    }

    /// [`Db::upsert_document`] with an already encoded payload.
    ///
    /// # Errors
    ///
    /// As [`Db::upsert_document`].
    pub fn upsert(&self, unit_type: &str, field: &str, payload: impl Into<Vec<u8>>) -> Result<Upserted> {
        let done = self.commit_with(Batch::new().upsert(unit_type, field, payload), &CommitMeta::default())?;
        Ok(done.upserts[0])
    }

    /// Replaces the payload of `id` with `document`, like [`Db::update`]. When the type has a
    /// [`Schema`] the document must satisfy it, and a refused write changes nothing.
    ///
    /// # Errors
    ///
    /// As [`Db::update`], and `SchemaViolation` when the schema refuses the document.
    pub fn update_document(&self, id: UnitId, document: &Document) -> Result<()> {
        self.update(id, document.encode()?)
    }

    /// Deletes the unit `id`. The unit stays readable through [`Db::get`] as a tombstone
    /// (`Lifecycle::Deleted`, no payload), its history stays (until [`Db::compact`] drops the
    /// payloads of its revisions), [`Db::query`] and the keyword search no longer find it, and its
    /// id is never reused.
    ///
    /// # Errors
    ///
    /// The unit does not exist or is deleted already, a relation still touches it
    /// (`UnitHasRelations`; remove it with [`Db::unrelate`] first), or the engine fails.
    pub fn delete(&self, id: UnitId) -> Result<()> {
        self.commit(Batch::new().delete(id)).map(|_| ())
    }

    /// Removes the relation `edge`.
    ///
    /// # Errors
    ///
    /// The relation does not exist or the engine fails.
    pub fn unrelate(&self, edge: RelationId) -> Result<()> {
        self.commit(Batch::new().unrelate(edge)).map(|_| ())
    }

    /// Applies every operation of `batch` in one commit, so a reader sees all of them or none, and
    /// returns the ids of the units the batch stored, in the order it stored them. The first
    /// operation that fails abandons the whole batch and nothing is written. An empty batch
    /// writes nothing.
    ///
    /// # Errors
    ///
    /// As the single operations ([`Db::store`], [`Db::update`], [`Db::delete`],
    /// [`Db::unrelate`]), and `InvalidArgument` when two operations name the same unit or the
    /// batch is larger than the layer's change-set limit.
    pub fn commit(&self, batch: impl Borrow<Batch>) -> Result<Vec<UnitId>> {
        Ok(self.commit_with(batch, &CommitMeta::default())?.units)
    }

    /// [`Db::commit`] with provenance recorded on every revision it writes, and a full report:
    /// stored units, created relations, skipped unchanged updates and the commit sequence.
    ///
    /// # Errors
    ///
    /// As [`Db::commit`]; `BadEndpoint` for an [`Endpoint::Op`] that is not an earlier store or
    /// upsert of the batch, and `InvalidArgument` for provenance fields over their limits.
    pub fn commit_with(&self, batch: impl Borrow<Batch>, meta: &CommitMeta) -> Result<Committed> {
        let ops = &batch.borrow().ops;
        if ops.is_empty() {
            return Ok(Committed::default());
        }
        // Upserts of one key become one: the first one's place, the last one's payload.
        let mut upsert_at: Vec<(String, String, Vec<u8>)> = Vec::new();
        let mut upsert_last: Vec<&[u8]> = Vec::new();
        let mut upsert_slot = vec![usize::MAX; ops.len()];
        for (i, op) in ops.iter().enumerate() {
            if let Op::Upsert { unit_type, field, payload } = op {
                // A payload without a key is left to `ChangeSet::upsert`, which names the first
                // problem (a field that is not unique comes before a missing value).
                let Ok(key) = upsert_key(payload, field) else { continue };
                let key = (unit_type.clone(), field.clone(), key);
                let slot = upsert_at.iter().position(|k| *k == key).unwrap_or_else(|| {
                    upsert_at.push(key);
                    upsert_last.push(payload);
                    upsert_at.len() - 1
                });
                upsert_last[slot] = payload;
                upsert_slot[i] = slot;
            }
        }
        let mut upserted: Vec<Option<Upserted>> = vec![None; upsert_at.len()];
        let snap = self.units.snapshot()?;
        let change_meta = ChangeMeta {
            reason: meta.reason.clone(),
            actor: meta.actor.clone(),
            source: meta.source.clone(),
            ..ChangeMeta::default()
        };
        let mut changes = self.units.begin_changeset(change_meta)?;
        let mut out = Committed::default();
        let mut unrelated: HashSet<RelationId> = HashSet::new();
        let mut related: Vec<(UnitId, UnitId, String, RelationId)> = Vec::new();
        let mut wrote = false;
        // The unit each operation created (a store, or an upsert whatever its outcome), by the
        // operation's position: what `Endpoint::Op` indexes.
        let mut made: Vec<Option<UnitId>> = vec![None; ops.len()];
        let resolve = |e: Endpoint, at: usize, made: &[Option<UnitId>]| -> Result<UnitId> {
            let bad = |why: &str| Error::BadEndpoint { detail: format!("operation {at} relates operation {why}") };
            match e {
                Endpoint::Unit(id) => Ok(id),
                Endpoint::Op(n) if n >= at => Err(bad(&format!("{n}, which is not an earlier operation"))),
                Endpoint::Op(n) => {
                    made[n].ok_or_else(|| bad(&format!("{n}, which creates no unit (only a store or an upsert does)")))
                }
            }
        };
        for (i, op) in ops.iter().enumerate() {
            match op {
                Op::Store { unit_type, payload } => {
                    let id = changes.create(NewUnit {
                        unit_type: unit_type.clone(),
                        schema_version: SCHEMA_VERSION,
                        metadata: Vec::new(),
                        payload: payload.clone(),
                    })?;
                    out.units.push(id);
                    made[i] = Some(id);
                    wrote = true;
                }
                Op::Update { id, payload, if_changed } => {
                    if *if_changed {
                        let current = changes.get_unit(*id)?;
                        if current.as_ref().and_then(|u| u.payload.as_deref()) == Some(payload.as_slice()) {
                            out.unchanged += 1;
                            continue;
                        }
                    }
                    let upd = UnitUpdate { payload: payload.clone(), schema_version: None, metadata: None };
                    changes.update(*id, upd, None)?;
                    wrote = true;
                }
                Op::Delete { id, cascade } => {
                    if *cascade {
                        let mut edges = snap.relations_of(*id, Direction::Out, None)?;
                        edges.extend(snap.relations_of(*id, Direction::In, None)?);
                        for r in edges {
                            if unrelated.insert(r.id) {
                                changes.delete_relation(r.id)?;
                            }
                        }
                        for (from, to, _, edge) in &related {
                            if (from == id || to == id) && unrelated.insert(*edge) {
                                changes.delete_relation(*edge)?;
                            }
                        }
                    }
                    changes.delete(*id, None)?;
                    wrote = true;
                }
                Op::Unrelate { edge } => {
                    if unrelated.insert(*edge) {
                        changes.delete_relation(*edge)?;
                        wrote = true;
                    }
                }
                Op::Relate { from, to, relation_type, unique } => {
                    let (from, to) = (resolve(*from, i, &made)?, resolve(*to, i, &made)?);
                    if *unique {
                        let in_batch = related
                            .iter()
                            .find(|(f, t, ty, e)| {
                                *f == from && *t == to && ty == relation_type && !unrelated.contains(e)
                            })
                            .map(|r| r.3);
                        let existing = match in_batch {
                            Some(e) => Some(e),
                            None => snap
                                .relations_of(from, Direction::Out, Some(relation_type))
                                .unwrap_or_default()
                                .into_iter()
                                .find(|r| r.to == to && !unrelated.contains(&r.id))
                                .map(|r| r.id),
                        };
                        if let Some(edge) = existing {
                            out.relations.push(edge);
                            continue;
                        }
                    }
                    let edge = changes.create_relation(from, to, relation_type, RelationProvenance::Declared)?;
                    related.push((from, to, relation_type.clone(), edge));
                    out.relations.push(edge);
                    wrote = true;
                }
                Op::Archive { id } => {
                    changes.archive(*id, None)?;
                    wrote = true;
                }
                Op::Unarchive { id } => {
                    changes.unarchive(*id, None)?;
                    wrote = true;
                }
                Op::Restore { id, from } => {
                    changes.restore(*id, RevNo(*from), None)?;
                    wrote = true;
                }
                Op::Upsert { unit_type, field, payload } => {
                    let slot = upsert_slot[i];
                    let done = match upserted.get(slot).copied().flatten() {
                        Some(done) => done,
                        None => {
                            let payload = upsert_last.get(slot).map_or(payload.as_slice(), |p| *p);
                            let new = NewUnit {
                                unit_type: unit_type.clone(),
                                schema_version: SCHEMA_VERSION,
                                metadata: Vec::new(),
                                payload: payload.to_vec(),
                            };
                            let done = changes.upsert(new, field)?;
                            match done.outcome {
                                UpsertOutcome::Unchanged => out.unchanged += 1,
                                UpsertOutcome::Created | UpsertOutcome::Updated => wrote = true,
                            }
                            if let Some(held) = upserted.get_mut(slot) {
                                *held = Some(done);
                            }
                            done
                        }
                    };
                    made[i] = Some(done.id);
                    out.upserts.push(done);
                }
            }
        }
        if wrote {
            out.commit_seq = Some(changes.commit_seq().0);
            changes.commit()?;
        }
        Ok(out)
    }

    /// The sequence of the last commit: the point an as-of read of "now" uses. 0 for an empty
    /// database.
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn last_commit_seq(&self) -> Result<u64> {
        Ok(self.units.snapshot()?.seq().0)
    }

    /// Moves a database opened read-only to the commits the writer made since it opened (or
    /// since the last refresh) and returns the commit sequence it now reads. Reads in flight keep
    /// their state. A shared read-only open never takes the directory lock, so this is how a
    /// long-lived reader sees a writer's later commits.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` unless the database was opened with `AccessMode::ReadOnly` (a read-write
    /// handle is always current); `Corruption` if the directory now holds another database.
    pub fn refresh(&self) -> Result<u64> {
        self.units.refresh()
    }

    /// The state of `id` once commit `commit_seq` committed. A sequence after the last commit
    /// reads the current state (it is clamped, not refused).
    ///
    /// # Errors
    ///
    /// The engine fails or finds the unit corrupt, or `HistoryUnavailable` when the revision's
    /// payload is gone (a deleted unit after [`Db::compact`], or retention).
    pub fn read_as_of(&self, id: UnitId, commit_seq: u64) -> Result<UnitAt> {
        let snap = self.units.snapshot()?;
        let at = commit_seq.min(snap.seq().0);
        snap.unit_as_of(id, AsOf::Seq(CommitSeq(at)))
    }

    /// Gives back the space of deleted units, compacts the revision ranges so that what was
    /// dropped leaves the live files, and returns the stored bytes before and after.
    ///
    /// A deleted unit keeps its payloads until this call: it then drops the payload of every
    /// revision of every deleted unit, whatever its history policy, and keeps the tombstone and
    /// each revision's metadata (hash, length, kind, times), so its history still lists the
    /// revisions but an as-of read of one answers `HistoryUnavailable`. Live units are not
    /// touched. To keep the payloads a retention policy keeps, call
    /// `units::UnitDb::compact_keeping` through [`Db::unit_db`]. What an open snapshot still
    /// sees stays until it is dropped.
    ///
    /// # Errors
    ///
    /// The engine fails or finds a deleted unit's history corrupt.
    pub fn compact(&self) -> Result<kerno_units::CompactReport> {
        self.units.compact()
    }

    /// Prunes old revisions as `policy` says (a rule per unit type: keep the newest `n`, keep
    /// the ones younger than a duration, or keep from a commit sequence on) and returns what it
    /// removed. It runs until every unit is done, in change sets of at most 16 units that pause
    /// between them so writers keep the write slot; it is idempotent. The head is never pruned.
    ///
    /// After it, a unit's `oldest` revision moves up, [`UnitDb::history`](units::UnitDb::history)
    /// below it and [`Db::read_as_of`] before it answer `Pruned`, and [`Db::verify`] stays clean.
    /// The policy is not stored: pass it each time. The bytes stay on disk until the engine
    /// compacts them (see [`Db::compact`]) and no snapshot needs them.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for `KeepLast(0)`; the engine fails or finds a unit's history corrupt.
    pub fn run_retention(&self, policy: &RetentionPolicy) -> Result<RetentionReport> {
        self.units.run_retention_all(policy)
    }

    /// Stores a new unit of type `unit_type` whose payload is `document`, like
    /// [`Db::store`]. When the type has a [`Schema`] the document must satisfy it.
    ///
    /// # Errors
    ///
    /// As [`Db::store`], and `SchemaViolation` when the schema refuses the document
    /// (a missing required field, a wrong type, a value another unit holds under a
    /// unique field). A refused write stores nothing.
    pub fn store_document(&self, unit_type: &str, document: &Document) -> Result<UnitId> {
        self.store(unit_type, document.encode()?)
    }

    /// Gives `unit_type` a schema: validates the units it already has against it and
    /// indexes the `indexed` and `unique` fields, in one committed change set. A type
    /// has one schema for good; indexes can be added and dropped later.
    ///
    /// # Errors
    ///
    /// The type already has a schema, the fields are invalid, an existing unit does not
    /// satisfy the schema (`SchemaViolation`, and nothing is stored), or the engine fails.
    pub fn define_schema(&self, unit_type: &str, fields: Vec<FieldDef>) -> Result<()> {
        self.units.define_schema(unit_type, fields, ChangeMeta::default()).map(|_| ())
    }

    /// Indexes `field` of `unit_type` from now on, and for the units it already has.
    ///
    /// # Errors
    ///
    /// The type has no schema or no such field, the field is indexed already, or the
    /// engine fails.
    pub fn create_index(&self, unit_type: &str, field: &str) -> Result<()> {
        self.units.create_index(unit_type, field, ChangeMeta::default()).map(|_| ())
    }

    /// Stops indexing `field` of `unit_type` and removes its entries. A unique field
    /// keeps its index.
    ///
    /// # Errors
    ///
    /// The type has no schema or no such field, the field is not indexed or is unique, or
    /// the engine fails.
    pub fn drop_index(&self, unit_type: &str, field: &str) -> Result<()> {
        self.units.drop_index(unit_type, field, ChangeMeta::default()).map(|_| ())
    }

    /// The schema of `unit_type`, if it has one.
    ///
    /// # Errors
    ///
    /// The engine fails or the record is corrupt.
    pub fn schema(&self, unit_type: &str) -> Result<Option<Schema>> {
        self.units.snapshot()?.schema(unit_type)
    }

    /// The active units of `unit_type` whose indexed `field` equals `value`, in unit id
    /// order.
    ///
    /// # Errors
    ///
    /// The type has no schema, the field is not an indexed field of it, `value` is not of
    /// the field's type, or the engine fails or finds an index entry that disagrees with
    /// its unit.
    pub fn find(&self, unit_type: &str, field: &str, value: &FieldValue) -> Result<Vec<Unit>> {
        self.units.snapshot()?.find_by_field(unit_type, field, value)
    }

    /// The active units of `unit_type` whose indexed `field` lies between `lo` and `hi`,
    /// ordered by value then unit id, at most `limit`. Floats order as numbers, text and
    /// bytes as bytes.
    ///
    /// # Errors
    ///
    /// As [`Db::find`].
    pub fn find_range(
        &self,
        unit_type: &str,
        field: &str,
        lo: Bound<&FieldValue>,
        hi: Bound<&FieldValue>,
        limit: usize,
    ) -> Result<Vec<Unit>> {
        self.units.snapshot()?.find_range(unit_type, field, lo, hi, limit)
    }

    /// The unit `id` as of its latest revision, or `None` when no such unit exists.
    ///
    /// # Errors
    ///
    /// The engine fails or finds the unit corrupt.
    pub fn get(&self, id: UnitId) -> Result<Option<Unit>> {
        self.units.get_unit(id)
    }

    /// Relates `from` to `to` with a declared relation of type `relation_type` (1 to 256 bytes,
    /// no NUL), in one committed change set. Parallel relations of one type are allowed.
    ///
    /// # Errors
    ///
    /// Either unit does not exist or is not active, the type is invalid, or the engine fails.
    pub fn relate(&self, from: UnitId, to: UnitId, relation_type: &str) -> Result<RelationId> {
        let mut changes = self.units.begin_changeset(ChangeMeta::default())?;
        let edge = changes.create_relation(from, to, relation_type, RelationProvenance::Declared)?;
        changes.commit()?;
        Ok(edge)
    }

    /// The relations that start at (`Direction::Out`) or end at (`Direction::In`) `unit`,
    /// optionally of one `relation_type`, in relation type then relation id order.
    ///
    /// # Errors
    ///
    /// `NotFound` when no unit with this id ever existed (a deleted unit is known and has no
    /// relations); `InvalidArgument` for an invalid `relation_type` name; the engine fails or the
    /// relation index disagrees with the relations it indexes.
    pub fn related(&self, unit: UnitId, direction: Direction, relation_type: Option<&str>) -> Result<Vec<Relation>> {
        let snap = self.units.snapshot()?;
        if snap.get_unit(unit)?.is_none() {
            return Err(Error::NotFound { unit });
        }
        snap.relations_of(unit, direction, relation_type)
    }

    /// Every live relation, optionally of one `relation_type`, in one scan of the relation
    /// records: the bulk read for a caller that would otherwise call [`Db::related`] once per
    /// unit. Order: relation id.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an invalid `relation_type` name; the engine fails or finds a
    /// relation record corrupt.
    pub fn relations(&self, relation_type: Option<&str>) -> Result<Vec<Relation>> {
        if relation_type.is_some_and(|t| !kerno_units::codec::valid_relation_type(t)) {
            return Err(Error::InvalidArgument("relation_type is not a valid relation type name".into()));
        }
        self.units.snapshot()?.reference_relations(|r| relation_type.is_none_or(|t| t == r.relation_type))
    }

    /// Turns keyword (BM25) indexing of `unit_type` on (`enabled = true`) or back off.
    ///
    /// Keyword indexing is off by default: the index costs write time and disk for data nobody
    /// searches by keyword. Turning it on indexes the type's active units in the same commit, and
    /// turning it off removes their entries, so the index never disagrees with the units. The
    /// setting is stored in the database (layer version 7, ADR-0022), so it holds across opens,
    /// and a build that predates it refuses the database instead of misreading it.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when the type name is invalid.
    pub fn set_fulltext(&self, unit_type: &str, enabled: bool) -> Result<()> {
        self.units.set_fulltext(unit_type, enabled, ChangeMeta::default()).map(|_| ())
    }

    /// The units whose text matches `query` by keyword (BM25, any of the words), best first, at
    /// most `limit`. Only the types turned on with [`Db::set_fulltext`] are searched; `types`
    /// restricts the search further to those unit types (`None`: every keyword-indexed type).
    /// For keywords fused with vectors and relations, see [`Db::hybrid_search`].
    ///
    /// A search that could never match raises instead of answering an empty list.
    ///
    /// # Errors
    ///
    /// `NotIndexed` when no type is keyword-indexed or a type in `types` is not;
    /// `UnknownType` for a type in `types` that no unit, schema or option ever named;
    /// `InvalidArgument` for `limit == 0`, a query with no word, an empty `types`, or an invalid
    /// type name; the engine fails or finds the index damaged.
    pub fn search_text(&self, query: &str, limit: usize, types: Option<&[&str]>) -> Result<Vec<TextHit>> {
        self.units.snapshot()?.search_text(query, limit, types)
    }

    /// Whether `unit_type` is keyword-indexed (see [`Db::set_fulltext`]; off by default).
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn fulltext(&self, unit_type: &str) -> Result<bool> {
        self.units.snapshot()?.fulltext_enabled(unit_type)
    }

    /// Every active unit that matches `filter`, in unit id order.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` for an invalid `unit_type` name (empty or over 64 bytes); the engine fails.
    pub fn query(&self, filter: &UnitFilter) -> Result<Vec<Unit>> {
        Ok(self.units.snapshot()?.query(filter)?.units)
    }

    /// Runs a query of the language in `docs/query-language.md` on one snapshot: the units of a
    /// type, joined to units of other types through typed relations, filtered by the fields of
    /// their schemas, ordered and paged. Each row holds one unit per binding of the query, in the
    /// order written, and `aliases` names the columns.
    ///
    /// # Errors
    ///
    /// `InvalidArgument` when `query` does not parse (the message names the byte) or does not fit
    /// the schemas (a type without a schema that a field is read from, a field a type does not
    /// declare, a literal of another type than its field); the engine fails or finds a record
    /// corrupt.
    pub fn execute(&self, query: &str) -> Result<QueryResult> {
        self.units.snapshot()?.execute_query(&Query::parse(query)?)
    }

    /// How [`Db::execute`] would answer `query`: the access paths of its first binding with their
    /// costs, the one chosen, what the planner does for a scan of that type, and its joins.
    ///
    /// # Errors
    ///
    /// As [`Db::execute`], and `Io` when the fingerprint key cannot be drawn from the system.
    pub fn explain(&self, query: &str) -> Result<QueryPlan> {
        let key = units::FingerprintKey::generate().map_err(|e| Error::from(kerno_core::Error::Io(e)))?;
        self.units.snapshot()?.explain_query(&key, &Query::parse(query)?)
    }

    /// One query over up to three legs, fused by weighted reciprocal rank fusion: keywords
    /// (BM25 over the units' text), vectors and relations (an associative query). A leg runs when
    /// its input is set on `query` and its weight is above 0. The vector leg is an index you own
    /// and keep in step with the units: an [`units::AnnIndex`] (HNSW) or an
    /// [`units::assoc_rank::VectorSet`] (exact), through [`VectorLeg`]. Units that are not active
    /// are never hits, even when a stale index still holds their vector. Best first, at most
    /// `query.limit`; see [`units::hybrid`] for the arithmetic.
    ///
    /// # Errors
    ///
    /// What [`Db::search_text`] raises when the keyword leg runs and could never match:
    /// `NotIndexed` (no type, or no type of `text_types`, is keyword-indexed, see
    /// [`Db::set_fulltext`]) or `UnknownType`. `InvalidArgument` for a query outside its bounds or with no
    /// leg to run, a graph leg the
    /// model rejects or whose seed is not active, or a vector the index refuses; the engine
    /// fails or finds a record corrupt.
    pub fn hybrid_search(&self, query: &HybridQuery<'_>) -> Result<Vec<HybridHit>> {
        self.units.hybrid_search(query)
    }

    /// Checks the whole store on one snapshot and reports every inconsistency it finds, among
    /// them a unit's index entry or a relation's adjacency entry that is missing. Reads never
    /// report such a loss: they return fewer results, so this is the only way to see it. Read-only.
    /// Run it after a crash that leaves the index status `Failed`, before an export, and on a
    /// schedule; [`units::UnitDb::rebuild_index`] repairs what it reports about the type index.
    /// See the [crate documentation](crate#verifying-integrity).
    ///
    /// A damaged units layer is a finding in the report, not an `Err`; call
    /// [`VerifyReport::is_clean`] or [`VerifyReport::into_result`].
    ///
    /// Damage below the units layer is different: a storage block that fails its checksum (a
    /// torn or altered `.sst`, a damaged `MANIFEST`) stops the scan, and this returns that
    /// `Err` (`Error::Storage` holding `Corruption`). [`Error::is_corruption`] tells the two
    /// apart; `kerno-cli verify` checks the storage files themselves.
    ///
    /// # Errors
    ///
    /// The engine fails, the database is poisoned, or a storage block is corrupt.
    pub fn verify(&self) -> Result<VerifyReport> {
        self.units.verify_units()
    }

    /// The units layer this `Db` wraps, for what the methods above do not cover.
    pub fn unit_db(&self) -> &UnitDb<KernoDb> {
        &self.units
    }

    /// Closes the database and releases its directory. Dropping a `Db` closes it too, but
    /// without reporting a failure.
    ///
    /// # Errors
    ///
    /// The engine fails to close cleanly.
    pub fn close(self) -> Result<()> {
        self.units.close()
    }
}
