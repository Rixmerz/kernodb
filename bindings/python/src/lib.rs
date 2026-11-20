//! Python bindings for Kerno: the native half of the `kerno` package.
//!
//! A thin layer over [`kerno::Db`]. Ids cross the boundary as 32-digit lower-case hex strings,
//! payloads as `bytes` (a `str` is stored as its UTF-8 bytes), and every engine failure becomes
//! a `kerno.KernoError`.

use std::path::PathBuf;
use std::sync::RwLock;

use kerno::units::{HistoryQuery, RevisionKind, UnitAt};
use kerno::{
    AccessMode, Batch, CommitMeta, Config, Db, Direction, Document, Endpoint, FieldDef, FieldType, FieldValue,
    Lifecycle, RelationId, UnitFilter, UnitId,
};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString};

create_exception!(_kerno, KernoError, PyException, "Any failure reported by the Kerno engine.");
create_exception!(_kerno, NotFoundError, KernoError, "No unit, relation or revision with that id.");
create_exception!(_kerno, DeletedError, KernoError, "The unit is deleted (or not in the lifecycle the call needs).");
create_exception!(_kerno, LockedError, KernoError, "Another handle or process holds the database directory.");
create_exception!(
    _kerno,
    CorruptionError,
    KernoError,
    "Stored bytes failed validation (a checksum or a structure rule)."
);
create_exception!(
    _kerno,
    InvalidArgumentError,
    KernoError,
    "The engine refused an argument (a type name, a limit, a batch)."
);
create_exception!(_kerno, PayloadTooLargeError, KernoError, "The payload is larger than the limit (1 MiB by default).");
create_exception!(
    _kerno,
    HasRelationsError,
    KernoError,
    "The unit still has live relations; unrelate them or use cascade."
);
create_exception!(_kerno, PrunedError, KernoError, "Retention removed the revision asked for.");
create_exception!(
    _kerno,
    SchemaError,
    KernoError,
    "The type's schema refused the document (or the payload is not a document)."
);

fn engine_error(e: kerno::Error) -> PyErr {
    use kerno::Error as E;
    let msg = e.to_string();
    if e.is_locked() {
        return LockedError::new_err(msg);
    }
    if e.is_corruption() {
        return CorruptionError::new_err(msg);
    }
    match e {
        E::NotFound { .. } | E::RelationNotFound { .. } | E::RevisionNotFound { .. } => NotFoundError::new_err(msg),
        E::Deleted { .. } => DeletedError::new_err(msg),
        E::PayloadTooLarge { .. } => PayloadTooLargeError::new_err(msg),
        E::InvalidArgument(_) | E::ChangeSetTooLarge { .. } => InvalidArgumentError::new_err(msg),
        E::UnitHasRelations { .. } => HasRelationsError::new_err(msg),
        E::Pruned { .. } => PrunedError::new_err(msg),
        E::SchemaViolation { .. } => SchemaError::new_err(msg),
        _ => KernoError::new_err(msg),
    }
}

fn is_not_found(e: &kerno::Error) -> bool {
    matches!(e, kerno::Error::NotFound { .. })
}

/// A Python value as a document field: bool, int, float, str or bytes.
fn to_field(v: &Bound<'_, PyAny>) -> PyResult<FieldValue> {
    if v.is_instance_of::<PyBool>() {
        Ok(FieldValue::Bool(v.extract()?))
    } else if v.is_instance_of::<PyInt>() {
        Ok(FieldValue::Int(v.extract()?))
    } else if v.is_instance_of::<PyFloat>() {
        Ok(FieldValue::Float(v.extract()?))
    } else if v.is_instance_of::<PyString>() {
        Ok(FieldValue::Text(v.extract()?))
    } else if let Ok(b) = v.extract::<Vec<u8>>() {
        Ok(FieldValue::Bytes(b))
    } else {
        Err(PyValueError::new_err(format!("unsupported field value: {v}")))
    }
}

fn to_document(d: &Bound<'_, PyDict>) -> PyResult<Document> {
    let mut doc = Document::new();
    for (k, v) in d.iter() {
        doc = doc.with(k.extract::<String>()?, to_field(&v)?);
    }
    Ok(doc)
}

fn field_to_py<'py>(py: Python<'py>, v: &FieldValue) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        FieldValue::Bool(b) => PyBool::new(py, *b).to_owned().into_any(),
        FieldValue::Int(i) => i.into_pyobject(py)?.into_any(),
        FieldValue::Float(f) => f.into_pyobject(py)?.into_any(),
        FieldValue::Text(t) => t.into_pyobject(py)?.into_any(),
        FieldValue::Bytes(b) => PyBytes::new(py, b).into_any(),
    })
}

fn field_type(name: &str) -> PyResult<FieldType> {
    Ok(match name {
        "bool" => FieldType::Bool,
        "int" => FieldType::Int,
        "float" => FieldType::Float,
        "text" | "str" => FieldType::Text,
        "bytes" => FieldType::Bytes,
        other => return Err(PyValueError::new_err(format!("unknown field type {other:?}"))),
    })
}

/// A relation end in a batch: a unit id (hex), or the index of an earlier "store" op.
fn endpoint(v: &Bound<'_, PyAny>) -> PyResult<Endpoint> {
    if let Ok(n) = v.extract::<usize>() {
        Ok(Endpoint::Stored(n))
    } else {
        Ok(Endpoint::Unit(UnitId(parse_id(&v.extract::<String>()?)?)))
    }
}

fn parse_id(hex: &str) -> PyResult<[u8; 16]> {
    let bad = || PyValueError::new_err(format!("not a unit id (expected 32 hex digits): {hex:?}"));
    if hex.len() != 32 || !hex.is_ascii() {
        return Err(bad());
    }
    let mut id = [0u8; 16];
    for (byte, pair) in id.iter_mut().zip(hex.as_bytes().chunks(2)) {
        // Both bytes are ASCII, so the chunk is valid UTF-8.
        *byte = u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| bad())?, 16).map_err(|_| bad())?;
    }
    Ok(id)
}

/// What `store` accepts as content: text is stored as its UTF-8 bytes.
#[derive(FromPyObject)]
enum Payload {
    Text(String),
    Bytes(Vec<u8>),
}

fn lifecycle_name(l: Lifecycle) -> &'static str {
    match l {
        Lifecycle::Active => "active",
        Lifecycle::Archived => "archived",
        Lifecycle::Deleted => "deleted",
    }
}

fn into_bytes(p: Payload) -> Vec<u8> {
    match p {
        Payload::Text(s) => s.into_bytes(),
        Payload::Bytes(b) => b,
    }
}

/// One revision of a unit, from `history` or `read_as_of`.
#[pyclass(frozen, module = "kerno")]
struct Revision {
    #[pyo3(get)]
    revision: u64,
    /// "create", "update", "archive", "unarchive", "delete" or "restore".
    #[pyo3(get)]
    kind: &'static str,
    #[pyo3(get)]
    commit_seq: u64,
    /// Descriptive wall-clock time (milliseconds); never used for ordering.
    #[pyo3(get)]
    created_at: i64,
    #[pyo3(get)]
    lifecycle: &'static str,
    #[pyo3(get)]
    schema_version: u32,
    /// Hex digest of the verified content.
    #[pyo3(get)]
    content_hash: String,
    payload: Option<Vec<u8>>,
}

#[pymethods]
impl Revision {
    /// The verified content, when it was asked for and the revision has one.
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.payload.as_deref().map(|p| PyBytes::new(py, p))
    }

    fn __repr__(&self) -> String {
        format!("Revision(revision={}, kind={:?}, commit_seq={})", self.revision, self.kind, self.commit_seq)
    }
}

impl From<kerno::units::Revision> for Revision {
    fn from(r: kerno::units::Revision) -> Self {
        Revision {
            revision: r.id.rev.0,
            kind: match r.kind {
                RevisionKind::Create => "create",
                RevisionKind::Update => "update",
                RevisionKind::Archive => "archive",
                RevisionKind::Unarchive => "unarchive",
                RevisionKind::Delete => "delete",
                RevisionKind::Restore => "restore",
            },
            commit_seq: r.commit_seq.0,
            created_at: r.created_at.0,
            lifecycle: lifecycle_name(r.lifecycle),
            schema_version: r.schema_version,
            content_hash: r.content_hash.digest.iter().map(|b| format!("{b:02x}")).collect(),
            payload: r.payload,
        }
    }
}

/// A stored unit, as of its latest revision.
#[pyclass(frozen, module = "kerno")]
struct Unit {
    #[pyo3(get)]
    id: String,
    #[pyo3(get)]
    unit_type: String,
    #[pyo3(get)]
    schema_version: u32,
    #[pyo3(get)]
    lifecycle: &'static str,
    #[pyo3(get)]
    revision: u64,
    payload: Option<Vec<u8>>,
}

#[pymethods]
impl Unit {
    /// The content, as `bytes`; `None` for a deleted unit.
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.payload.as_deref().map(|p| PyBytes::new(py, p))
    }

    fn __repr__(&self) -> String {
        format!("Unit(id={:?}, unit_type={:?}, lifecycle={:?})", self.id, self.unit_type, self.lifecycle)
    }
}

impl From<kerno::Unit> for Unit {
    fn from(u: kerno::Unit) -> Self {
        Unit {
            id: u.id.to_string(),
            unit_type: u.unit_type,
            schema_version: u.schema_version,
            lifecycle: lifecycle_name(u.lifecycle),
            revision: u.head.0,
            payload: u.payload,
        }
    }
}

/// A directed, typed edge between two units.
#[pyclass(frozen, module = "kerno")]
struct Relation {
    #[pyo3(get)]
    id: String,
    #[pyo3(get)]
    from_id: String,
    #[pyo3(get)]
    to_id: String,
    #[pyo3(get)]
    relation_type: String,
}

#[pymethods]
impl Relation {
    fn __repr__(&self) -> String {
        format!("Relation(from_id={:?}, to_id={:?}, relation_type={:?})", self.from_id, self.to_id, self.relation_type)
    }
}

/// An open Kerno database. Use it as a context manager, or call `close()`.
#[pyclass(frozen, module = "kerno")]
struct Database {
    // Calls share the read lock; `close` takes the write lock, so it waits for calls in flight
    // and every later call sees a closed database.
    db: RwLock<Option<Db>>,
}

impl Database {
    /// Runs `op` on the open database with the GIL released (opens and commits do disk I/O).
    fn with_db<T: Send>(&self, py: Python<'_>, op: impl FnOnce(&Db) -> PyResult<T> + Send) -> PyResult<T> {
        py.detach(|| {
            let guard = self.db.read().map_err(|_| KernoError::new_err("database lock poisoned"))?;
            match guard.as_ref() {
                Some(db) => op(db),
                None => Err(PyValueError::new_err("operation on a closed database")),
            }
        })
    }
}

#[pymethods]
impl Database {
    /// Stores a new unit of `unit_type` (1 to 64 bytes of UTF-8) and returns its id.
    fn store(&self, py: Python<'_>, unit_type: &str, payload: Payload) -> PyResult<String> {
        let bytes = into_bytes(payload);
        self.with_db(py, |db| Ok(db.store(unit_type, bytes).map_err(engine_error)?.to_string()))
    }

    /// The unit `id`, or `None` when no such unit exists.
    fn read(&self, py: Python<'_>, id: &str) -> PyResult<Option<Unit>> {
        let id = UnitId(parse_id(id)?);
        self.with_db(py, |db| Ok(db.get(id).map_err(engine_error)?.map(Unit::from)))
    }

    /// Relates unit `from_id` to unit `to_id` with a relation of `relation_type`; returns the
    /// relation's id.
    /// With `unique=True`, an existing relation of that type between the two units is reused.
    #[pyo3(signature = (from_id, to_id, relation_type, unique=false))]
    fn relate(
        &self,
        py: Python<'_>,
        from_id: &str,
        to_id: &str,
        relation_type: &str,
        unique: bool,
    ) -> PyResult<String> {
        let (from, to) = (UnitId(parse_id(from_id)?), UnitId(parse_id(to_id)?));
        self.with_db(py, |db| {
            let mut b = Batch::new();
            if unique {
                b.relate_unique(from, to, relation_type);
            } else {
                b.relate(from, to, relation_type);
            }
            let done = db.commit_with(&b, &CommitMeta::default()).map_err(engine_error)?;
            Ok(RelationId::to_string(&done.relations[0]))
        })
    }

    /// The relations that start at (`direction="out"`) or end at (`"in"`) `id`, optionally of one
    /// `relation_type`.
    #[pyo3(signature = (id, direction="out", relation_type=None))]
    fn related(
        &self,
        py: Python<'_>,
        id: &str,
        direction: &str,
        relation_type: Option<&str>,
    ) -> PyResult<Vec<Relation>> {
        let id = UnitId(parse_id(id)?);
        let direction = match direction {
            "out" => Direction::Out,
            "in" => Direction::In,
            other => return Err(PyValueError::new_err(format!("direction must be 'out' or 'in', not {other:?}"))),
        };
        self.with_db(py, |db| {
            let edges = db.related(id, direction, relation_type).map_err(engine_error)?;
            Ok(edges
                .into_iter()
                .map(|r| Relation {
                    id: RelationId::to_string(&r.id),
                    from_id: r.from.to_string(),
                    to_id: r.to.to_string(),
                    relation_type: r.relation_type,
                })
                .collect())
        })
    }

    /// Every live relation, optionally of one `relation_type`, in one scan: much faster than
    /// calling `related` for each unit.
    #[pyo3(signature = (relation_type=None))]
    fn relations(&self, py: Python<'_>, relation_type: Option<&str>) -> PyResult<Vec<Relation>> {
        self.with_db(py, |db| {
            Ok(db
                .relations(relation_type)
                .map_err(engine_error)?
                .into_iter()
                .map(|r| Relation {
                    id: RelationId::to_string(&r.id),
                    from_id: r.from.to_string(),
                    to_id: r.to.to_string(),
                    relation_type: r.relation_type,
                })
                .collect())
        })
    }

    /// Every active unit, in id order; `unit_type` and `schema_version` narrow it.
    #[pyo3(signature = (unit_type=None, schema_version=None))]
    fn query(&self, py: Python<'_>, unit_type: Option<&str>, schema_version: Option<u32>) -> PyResult<Vec<Unit>> {
        let filter = UnitFilter { unit_type: unit_type.map(str::to_owned), schema_version };
        self.with_db(py, |db| Ok(db.query(&filter).map_err(engine_error)?.into_iter().map(Unit::from).collect()))
    }

    /// Replaces the payload of `id`, adding a revision; the earlier ones stay in its history.
    /// With `if_changed=True`, an identical payload writes nothing. Returns whether a revision
    /// was written.
    #[pyo3(signature = (id, payload, if_changed=false))]
    fn update(&self, py: Python<'_>, id: &str, payload: Payload, if_changed: bool) -> PyResult<bool> {
        let id = UnitId(parse_id(id)?);
        let bytes = into_bytes(payload);
        self.with_db(py, |db| {
            let mut b = Batch::new();
            if if_changed {
                b.update_if_changed(id, bytes);
            } else {
                b.update(id, bytes);
            }
            Ok(db.commit_with(&b, &CommitMeta::default()).map_err(engine_error)?.unchanged == 0)
        })
    }

    /// Deletes `id` with a tombstone revision; its history is kept. With `cascade=True` its
    /// relations are removed first, in the same commit.
    #[pyo3(signature = (id, cascade=false))]
    fn delete(&self, py: Python<'_>, id: &str, cascade: bool) -> PyResult<()> {
        let id = UnitId(parse_id(id)?);
        self.with_db(py, |db| {
            let mut b = Batch::new();
            if cascade {
                b.delete_cascade(id);
            } else {
                b.delete(id);
            }
            db.commit(&b).map(|_| ()).map_err(engine_error)
        })
    }

    /// Turns keyword (BM25) indexing of `unit_type` off or back on. Only while the type has no
    /// active unit; the setting is stored in the database. Use it for types whose payloads are
    /// text nobody searches by keyword (JSON, code): their writes get several times faster.
    fn set_fulltext(&self, py: Python<'_>, unit_type: &str, enabled: bool) -> PyResult<()> {
        self.with_db(py, |db| db.set_fulltext(unit_type, enabled).map_err(engine_error))
    }

    /// The sequence of the last commit (0 for an empty database): the point to pass to
    /// `read_as_of` for "now".
    fn last_commit_seq(&self, py: Python<'_>) -> PyResult<u64> {
        self.with_db(py, |db| db.last_commit_seq().map_err(engine_error))
    }

    /// Compacts the revision ranges; returns `{"before": bytes, "after": bytes}`. History keeps
    /// deleted units' payloads, so this frees only what a retention policy dropped.
    fn compact<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let r = self.with_db(py, |db| db.compact().map_err(engine_error))?;
        let d = PyDict::new(py);
        d.set_item("before", r.stored_bytes_before)?;
        d.set_item("after", r.stored_bytes_after)?;
        Ok(d)
    }

    /// Gives `unit_type` a schema. `fields` is a list of dicts: `name`, `type` ("bool", "int",
    /// "float", "text", "bytes"), and optional `required`, `unique`, `indexed` booleans.
    fn define_schema(&self, py: Python<'_>, unit_type: &str, fields: Vec<Bound<'_, PyDict>>) -> PyResult<()> {
        let mut defs = Vec::new();
        for f in &fields {
            let name: String =
                f.get_item("name")?.ok_or_else(|| PyValueError::new_err("field without name"))?.extract()?;
            let ty: String =
                f.get_item("type")?.ok_or_else(|| PyValueError::new_err("field without type"))?.extract()?;
            let flag =
                |k: &str| -> PyResult<bool> { Ok(f.get_item(k)?.map(|v| v.is_truthy()).transpose()?.unwrap_or(false)) };
            let mut def = FieldDef::new(name, field_type(&ty)?);
            if flag("required")? {
                def = def.required();
            }
            if flag("unique")? {
                def = def.unique();
            }
            if flag("indexed")? {
                def = def.indexed();
            }
            defs.push(def);
        }
        self.with_db(py, |db| db.define_schema(unit_type, defs).map_err(engine_error))
    }

    /// Indexes `field` of `unit_type` (it must be in the type's schema).
    fn create_index(&self, py: Python<'_>, unit_type: &str, field: &str) -> PyResult<()> {
        self.with_db(py, |db| db.create_index(unit_type, field).map_err(engine_error))
    }

    /// Stores a document (a dict of bool/int/float/str/bytes); a schema, if any, validates it.
    fn store_document(&self, py: Python<'_>, unit_type: &str, document: Bound<'_, PyDict>) -> PyResult<String> {
        let doc = to_document(&document)?;
        self.with_db(py, |db| Ok(db.store_document(unit_type, &doc).map_err(engine_error)?.to_string()))
    }

    /// Replaces the payload of `id` with a document.
    fn update_document(&self, py: Python<'_>, id: &str, document: Bound<'_, PyDict>) -> PyResult<()> {
        let id = UnitId(parse_id(id)?);
        let doc = to_document(&document)?;
        self.with_db(py, |db| db.update_document(id, &doc).map_err(engine_error))
    }

    /// The payload of `id` decoded as a document dict, or `None` for an unknown or deleted unit.
    fn read_document<'py>(&self, py: Python<'py>, id: &str) -> PyResult<Option<Bound<'py, PyDict>>> {
        let id = UnitId(parse_id(id)?);
        let unit = self.with_db(py, |db| db.get(id).map_err(engine_error))?;
        let Some(payload) = unit.and_then(|u| u.payload) else { return Ok(None) };
        let doc = Document::decode(&payload).map_err(engine_error)?;
        let d = PyDict::new(py);
        for (k, v) in doc.iter() {
            d.set_item(k, field_to_py(py, v)?)?;
        }
        Ok(Some(d))
    }

    /// The active units of `unit_type` whose indexed `field` equals `value`.
    fn find(&self, py: Python<'_>, unit_type: &str, field: &str, value: Bound<'_, PyAny>) -> PyResult<Vec<Unit>> {
        let v = to_field(&value)?;
        self.with_db(py, |db| {
            Ok(db.find(unit_type, field, &v).map_err(engine_error)?.into_iter().map(Unit::from).collect())
        })
    }

    /// Runs a query of the Kerno query language; returns `{"aliases": [...], "rows": [[Unit]]}`.
    fn execute<'py>(&self, py: Python<'py>, query: &str) -> PyResult<Bound<'py, PyDict>> {
        let r = self.with_db(py, |db| db.execute(query).map_err(engine_error))?;
        let d = PyDict::new(py);
        d.set_item("aliases", r.aliases)?;
        let rows = PyList::empty(py);
        for row in r.rows {
            rows.append(row.into_iter().map(Unit::from).collect::<Vec<_>>())?;
        }
        d.set_item("rows", rows)?;
        Ok(d)
    }

    /// Removes the relation `relation_id`.
    fn unrelate(&self, py: Python<'_>, relation_id: &str) -> PyResult<()> {
        let edge = RelationId(parse_id(relation_id)?);
        self.with_db(py, |db| db.unrelate(edge).map_err(engine_error))
    }

    /// Commits `ops` atomically, as one change set. Each op is a tuple:
    /// `("store", unit_type, payload)`, `("update", id, payload)`, `("update_if_changed", id,
    /// payload)`, `("delete", id)`, `("delete_cascade", id)`, `("unrelate", relation_id)`,
    /// `("relate", from, to, type)`, `("relate_unique", from, to, type)`, `("archive", id)`,
    /// `("unarchive", id)` or `("restore", id, revision)`. A relation end is a unit id or the
    /// index (0-based) of an earlier "store" op of the batch. `reason`, `actor` and `source` are
    /// recorded with every revision. Returns the ids of the stored units, or with
    /// `detailed=True` a dict: `units`, `relations`, `unchanged`, `commit_seq`.
    #[pyo3(signature = (ops, reason=None, actor=None, source=None, detailed=false))]
    fn commit<'py>(
        &self,
        py: Python<'py>,
        ops: Vec<Bound<'py, pyo3::types::PyTuple>>,
        reason: Option<String>,
        actor: Option<Payload>,
        source: Option<Payload>,
        detailed: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let mut batch = Batch::new();
        let id_at = |op: &Bound<'py, pyo3::types::PyTuple>, i: usize| -> PyResult<UnitId> {
            Ok(UnitId(parse_id(&op.get_item(i)?.extract::<String>()?)?))
        };
        for op in &ops {
            let name: String = op.get_item(0)?.extract()?;
            match (name.as_str(), op.len()) {
                ("store", 3) => {
                    let t: String = op.get_item(1)?.extract()?;
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.store(&t, into_bytes(p));
                }
                ("update", 3) => {
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.update(id_at(op, 1)?, into_bytes(p));
                }
                ("update_if_changed", 3) => {
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.update_if_changed(id_at(op, 1)?, into_bytes(p));
                }
                ("delete", 2) => {
                    batch.delete(id_at(op, 1)?);
                }
                ("delete_cascade", 2) => {
                    batch.delete_cascade(id_at(op, 1)?);
                }
                ("unrelate", 2) => {
                    let id: String = op.get_item(1)?.extract()?;
                    batch.unrelate(RelationId(parse_id(&id)?));
                }
                ("relate", 4) | ("relate_unique", 4) => {
                    let (f, t) = (endpoint(&op.get_item(1)?)?, endpoint(&op.get_item(2)?)?);
                    let ty: String = op.get_item(3)?.extract()?;
                    if name == "relate" {
                        batch.relate(f, t, &ty);
                    } else {
                        batch.relate_unique(f, t, &ty);
                    }
                }
                ("archive", 2) => {
                    batch.archive(id_at(op, 1)?);
                }
                ("unarchive", 2) => {
                    batch.unarchive(id_at(op, 1)?);
                }
                ("restore", 3) => {
                    let rev: u64 = op.get_item(2)?.extract()?;
                    batch.restore(id_at(op, 1)?, rev);
                }
                _ => return Err(PyValueError::new_err(format!("unknown or malformed batch op: {op}"))),
            }
        }
        let meta = CommitMeta { reason, actor: actor.map(into_bytes), source: source.map(into_bytes) };
        let done = self.with_db(py, move |db| db.commit_with(&batch, &meta).map_err(engine_error))?;
        let units: Vec<String> = done.units.iter().map(ToString::to_string).collect();
        if !detailed {
            return Ok(units.into_pyobject(py)?.into_any());
        }
        let d = PyDict::new(py);
        d.set_item("units", units)?;
        d.set_item("relations", done.relations.iter().map(RelationId::to_string).collect::<Vec<_>>())?;
        d.set_item("unchanged", done.unchanged)?;
        d.set_item("commit_seq", done.commit_seq)?;
        Ok(d.into_any())
    }

    /// The newest `limit` revisions of `id`, newest first. `with_payload` reads and verifies
    /// each revision's content.
    #[pyo3(signature = (id, limit=100, with_payload=false))]
    fn history(&self, py: Python<'_>, id: &str, limit: usize, with_payload: bool) -> PyResult<Vec<Revision>> {
        let id = UnitId(parse_id(id)?);
        if limit == 0 {
            return Err(PyValueError::new_err("limit must be at least 1"));
        }
        let q = HistoryQuery { with_payload, ..HistoryQuery::latest(limit) };
        self.with_db(py, |db| match db.unit_db().history(id, q) {
            Ok(revs) => Ok(revs.into_iter().map(Revision::from).collect()),
            Err(e) if is_not_found(&e) => Ok(Vec::new()),
            Err(e) => Err(engine_error(e)),
        })
    }

    /// The revision of `id` in force once commit `commit_seq` committed, with its payload;
    /// `None` when the unit did not exist yet (or never did). A deleted unit returns its
    /// tombstone. A sequence after the last commit reads the current state.
    fn read_as_of(&self, py: Python<'_>, id: &str, commit_seq: u64) -> PyResult<Option<Revision>> {
        let id = UnitId(parse_id(id)?);
        self.with_db(py, |db| match db.read_as_of(id, commit_seq).map_err(engine_error)? {
            UnitAt::Absent => Ok(None),
            UnitAt::Present(r) | UnitAt::DeletedAt(r) => Ok(Some(Revision::from(r))),
            UnitAt::Pruned { oldest } => {
                Err(PrunedError::new_err(format!("revision pruned by retention; oldest kept is {}", oldest.0)))
            }
        })
    }

    /// Checks every record of the units layer: `{"clean", "commit_seq", "units", "revisions",
    /// "relations", "findings"}`. A damaged store is reported, not raised.
    fn verify<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let report = match self.with_db(py, |db| Ok(db.verify()))? {
            Ok(r) => r,
            Err(e) if e.is_corruption() => {
                // Storage-level damage stops the scan: report it, as the doc promises.
                d.set_item("clean", false)?;
                d.set_item("storage_error", e.to_string())?;
                d.set_item("findings", vec![e.to_string()])?;
                d.set_item("findings_omitted", 0)?;
                return Ok(d);
            }
            Err(e) => return Err(engine_error(e)),
        };
        d.set_item("clean", report.is_clean())?;
        d.set_item("commit_seq", report.seq.0)?;
        d.set_item("units", report.counts.units)?;
        d.set_item("revisions", report.counts.revisions)?;
        d.set_item("relations", report.counts.relations)?;
        d.set_item("findings", report.findings.iter().map(ToString::to_string).collect::<Vec<_>>())?;
        d.set_item("findings_omitted", report.findings_omitted)?;
        Ok(d)
    }

    /// Closes the database and releases its directory. Closing twice is harmless.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| {
            let taken = self.db.write().map_err(|_| KernoError::new_err("database lock poisoned"))?.take();
            taken.map_or(Ok(()), |db| db.close().map_err(engine_error))
        })
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    #[pyo3(signature = (*_args))]
    fn __exit__(&self, py: Python<'_>, _args: &Bound<'_, pyo3::types::PyTuple>) -> PyResult<bool> {
        self.close(py)?;
        Ok(false)
    }
}

/// Opens the database in `path`, creating it when `path` holds none.
///
/// `read_only=True` opens without writing anything to the directory (it still holds the
/// directory's lock: one handle at a time). `max_batch_ops` raises (or lowers) the most
/// operations one `commit` may hold (default 10,000).
#[pyfunction]
#[pyo3(signature = (path, read_only=false, max_batch_ops=None))]
fn open(py: Python<'_>, path: PathBuf, read_only: bool, max_batch_ops: Option<usize>) -> PyResult<Database> {
    let db = py
        .detach(|| {
            let config = if read_only {
                Config {
                    access: AccessMode::ReadOnly,
                    create_if_missing: false,
                    allow_unverified_platform: true,
                    ..Config::default()
                }
            } else {
                Config { allow_unverified_platform: true, ..Config::default() }
            };
            Db::open_with_limits(path, config, max_batch_ops.unwrap_or(kerno::units::DEFAULT_MAX_CHANGESET_OPS))
        })
        .map_err(engine_error)?;
    Ok(Database { db: RwLock::new(Some(db)) })
}

#[pymodule]
fn _kerno(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(open, m)?)?;
    m.add_class::<Database>()?;
    m.add_class::<Unit>()?;
    m.add_class::<Relation>()?;
    m.add_class::<Revision>()?;
    m.add("KernoError", m.py().get_type::<KernoError>())?;
    m.add("NotFoundError", m.py().get_type::<NotFoundError>())?;
    m.add("DeletedError", m.py().get_type::<DeletedError>())?;
    m.add("LockedError", m.py().get_type::<LockedError>())?;
    m.add("CorruptionError", m.py().get_type::<CorruptionError>())?;
    m.add("InvalidArgumentError", m.py().get_type::<InvalidArgumentError>())?;
    m.add("PayloadTooLargeError", m.py().get_type::<PayloadTooLargeError>())?;
    m.add("HasRelationsError", m.py().get_type::<HasRelationsError>())?;
    m.add("PrunedError", m.py().get_type::<PrunedError>())?;
    m.add("SchemaError", m.py().get_type::<SchemaError>())?;
    Ok(())
}
