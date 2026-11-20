//! A document store on the `kerno` facade.
//!
//! Model: a `document` unit per document, typed by a schema (metadata: unique `slug`, `title`,
//! indexed `author`, `status`, `year`), a `document_body` unit holding the body as plain UTF-8 (so
//! it is full-text indexed), and a `body_of` relation from the body to its document.
//!
//! Reads and the schema use `Db` methods. `Db` has no update, delete or multi-operation commit
//! today, so insert, update and delete are single change sets through `Db::unit_db()`.

use std::ops::Bound;
use std::path::Path;

use kerno::units::hybrid::{MAX_HYBRID_CANDIDATES, MAX_HYBRID_LIMIT};
use kerno::units::{ChangeMeta, NewUnit, RelationProvenance, UnitUpdate};
use kerno::{Db, Direction, Document, FieldDef, FieldType, FieldValue, HybridQuery, Relation, Unit};
pub use kerno::{Error, UnitId};

pub const DOCUMENT: &str = "document";
pub const BODY: &str = "document_body";
pub const BODY_OF: &str = "body_of";

/// A document: its metadata and its body.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub slug: String,
    pub title: String,
    pub author: String,
    pub status: String,
    pub year: i64,
    pub body: String,
}

pub struct DocumentStore {
    db: Db,
}

impl DocumentStore {
    /// Opens (or creates) the store in `dir`.
    ///
    /// # Errors
    ///
    /// The database cannot be opened or its schema cannot be defined.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let db = Db::open(dir)?;
        // A type keeps one schema for good, so define it only the first time.
        if db.schema(DOCUMENT)?.is_none() {
            db.define_schema(
                DOCUMENT,
                vec![
                    FieldDef::new("slug", FieldType::Text).unique().required(),
                    FieldDef::new("title", FieldType::Text).required(),
                    FieldDef::new("author", FieldType::Text).indexed().required(),
                    FieldDef::new("status", FieldType::Text).indexed().required(),
                    FieldDef::new("year", FieldType::Int).indexed().required(),
                ],
            )?;
        }
        Ok(DocumentStore { db })
    }

    /// Inserts a document and its body in one change set.
    ///
    /// # Errors
    ///
    /// `SchemaViolation` when the slug is taken (nothing is stored), or the engine fails.
    pub fn insert(&self, doc: &Doc) -> Result<UnitId, Error> {
        let record = record(doc)?;
        // One change set: a crash cannot leave a document without its body.
        let mut changes = self.db.unit_db().begin_changeset(ChangeMeta::default())?;
        let id = changes.create(new_unit(DOCUMENT, record))?;
        let body = changes.create(new_unit(BODY, doc.body.clone().into_bytes()))?;
        changes.create_relation(body, id, BODY_OF, RelationProvenance::Declared)?;
        changes.commit()?;
        Ok(id)
    }

    /// The document with `slug`.
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn get(&self, slug: &str) -> Result<Option<Doc>, Error> {
        self.unit_by_slug(slug)?.map(|u| self.load(&u)).transpose()
    }

    /// Replaces the metadata and body of the document with `doc.slug`; the slug itself cannot
    /// change. `false` when there is no such document.
    ///
    /// # Errors
    ///
    /// The engine fails or the schema refuses the new values.
    pub fn update(&self, doc: &Doc) -> Result<bool, Error> {
        // Every `Db` read happens before the change set: it holds the one write slot.
        let Some(current) = self.unit_by_slug(&doc.slug)? else { return Ok(false) };
        let body = self.body_edge(&current)?;
        let mut changes = self.db.unit_db().begin_changeset(ChangeMeta::default())?;
        changes.update(
            current.id,
            UnitUpdate { payload: record(doc)?, schema_version: None, metadata: None },
            Some(current.head),
        )?;
        changes.update(
            body.from,
            UnitUpdate { payload: doc.body.clone().into_bytes(), schema_version: None, metadata: None },
            None,
        )?;
        changes.commit()?;
        Ok(true)
    }

    /// Deletes the document with `slug` and its body. `false` when there is no such document.
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn delete(&self, slug: &str) -> Result<bool, Error> {
        let Some(current) = self.unit_by_slug(slug)? else { return Ok(false) };
        let edge = self.body_edge(&current)?;
        let mut changes = self.db.unit_db().begin_changeset(ChangeMeta::default())?;
        // A unit with relations cannot be deleted; a change set reads its own writes.
        changes.delete_relation(edge.id)?;
        changes.delete(edge.from, None)?;
        changes.delete(current.id, Some(current.head))?;
        changes.commit()?;
        Ok(true)
    }

    /// Documents by `author`, in unit id order.
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn by_author(&self, author: &str) -> Result<Vec<Doc>, Error> {
        self.load_all(&self.db.find(DOCUMENT, "author", &FieldValue::Text(author.to_owned()))?)
    }

    /// Documents with `from <= year <= to`, ordered by year, at most `limit`.
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn by_year(&self, from: i64, to: i64, limit: usize) -> Result<Vec<Doc>, Error> {
        let (lo, hi) = (FieldValue::Int(from), FieldValue::Int(to));
        self.load_all(&self.db.find_range(DOCUMENT, "year", Bound::Included(&lo), Bound::Included(&hi), limit)?)
    }

    /// Published documents, newest year first. Query text is constant: `Db::execute` takes no
    /// bind parameters, so caller values go through `find` and `find_range`.
    ///
    /// # Errors
    ///
    /// The engine fails.
    pub fn published(&self) -> Result<Vec<Doc>, Error> {
        let result = self.db.execute("FIND document WHERE status = \"published\" ORDER BY year DESC, slug")?;
        result.rows.iter().map(|row| self.load(&row[0])).collect()
    }

    /// Documents whose body matches `words`, best first, at most `limit`.
    ///
    /// Only plain-text units are full-text indexed, and the index is global (best 1000 hits), so
    /// hits are filtered down to bodies and followed to their document.
    ///
    /// # Errors
    ///
    /// `words` is empty, or the engine fails.
    pub fn search(&self, words: &str, limit: usize) -> Result<Vec<Doc>, Error> {
        let query =
            HybridQuery { text: Some(words), candidates: MAX_HYBRID_CANDIDATES, ..HybridQuery::new(MAX_HYBRID_LIMIT) };
        let mut out = Vec::new();
        for hit in self.db.hybrid_search(&query)? {
            if out.len() == limit {
                break;
            }
            let unit = self.db.get(hit.unit)?.expect("a hit is a stored unit");
            if unit.unit_type != BODY {
                continue;
            }
            let edge = self.db.related(unit.id, Direction::Out, Some(BODY_OF))?.into_iter().next();
            let doc = self.db.get(edge.expect("a body belongs to a document").to)?;
            out.push(self.load(&doc.expect("a body relation ends at a stored document"))?);
        }
        Ok(out)
    }

    /// Closes the database.
    ///
    /// # Errors
    ///
    /// The engine fails to close cleanly.
    pub fn close(self) -> Result<(), Error> {
        self.db.close()
    }

    fn unit_by_slug(&self, slug: &str) -> Result<Option<Unit>, Error> {
        Ok(self.db.find(DOCUMENT, "slug", &FieldValue::Text(slug.to_owned()))?.into_iter().next())
    }

    fn body_edge(&self, document: &Unit) -> Result<Relation, Error> {
        let edges = self.db.related(document.id, Direction::In, Some(BODY_OF))?;
        Ok(edges.into_iter().next().expect("a document has one body"))
    }

    fn load_all(&self, units: &[Unit]) -> Result<Vec<Doc>, Error> {
        units.iter().map(|u| self.load(u)).collect()
    }

    fn load(&self, unit: &Unit) -> Result<Doc, Error> {
        let fields = Document::decode(unit.payload.as_deref().expect("an active document has a payload"))?;
        let text = |name: &str| match fields.get(name) {
            Some(FieldValue::Text(t)) => t.clone(),
            _ => panic!("the schema guarantees text in {name}"),
        };
        let Some(FieldValue::Int(year)) = fields.get("year") else { panic!("the schema guarantees an int in year") };
        let body = self.db.get(self.body_edge(unit)?.from)?.expect("a body relation starts at a stored body");
        Ok(Doc {
            slug: text("slug"),
            title: text("title"),
            author: text("author"),
            status: text("status"),
            year: *year,
            body: String::from_utf8(body.payload.unwrap_or_default()).expect("a body is stored as UTF-8"),
        })
    }
}

fn new_unit(unit_type: &str, payload: Vec<u8>) -> NewUnit {
    NewUnit { unit_type: unit_type.to_owned(), schema_version: 1, metadata: Vec::new(), payload }
}

fn record(doc: &Doc) -> Result<Vec<u8>, Error> {
    Document::new()
        .with("slug", FieldValue::Text(doc.slug.clone()))
        .with("title", FieldValue::Text(doc.title.clone()))
        .with("author", FieldValue::Text(doc.author.clone()))
        .with("status", FieldValue::Text(doc.status.clone()))
        .with("year", FieldValue::Int(doc.year))
        .encode()
}
