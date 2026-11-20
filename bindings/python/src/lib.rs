//! Python bindings for Kerno: the native half of the `kerno` package.
//!
//! A thin layer over [`kerno::Db`]. Ids cross the boundary as 32-digit lower-case hex strings,
//! payloads as `bytes` (a `str` is stored as its UTF-8 bytes), and every engine failure becomes
//! a `kerno.KernoError`.

use std::path::PathBuf;
use std::sync::RwLock;

use kerno::units::{AsOf, CommitSeq, HistoryQuery, RevisionKind, UnitAt};
use kerno::{Batch, Db, Direction, Lifecycle, RelationId, UnitFilter, UnitId};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

create_exception!(_kerno, KernoError, PyException, "Any failure reported by the Kerno engine.");

fn engine_error(e: kerno::Error) -> PyErr {
    KernoError::new_err(e.to_string())
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
    fn relate(&self, py: Python<'_>, from_id: &str, to_id: &str, relation_type: &str) -> PyResult<String> {
        let (from, to) = (UnitId(parse_id(from_id)?), UnitId(parse_id(to_id)?));
        self.with_db(py, |db| Ok(db.relate(from, to, relation_type).map_err(engine_error)?.to_string()))
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

    /// Every active unit, in id order; `unit_type` and `schema_version` narrow it.
    #[pyo3(signature = (unit_type=None, schema_version=None))]
    fn query(&self, py: Python<'_>, unit_type: Option<&str>, schema_version: Option<u32>) -> PyResult<Vec<Unit>> {
        let filter = UnitFilter { unit_type: unit_type.map(str::to_owned), schema_version };
        self.with_db(py, |db| Ok(db.query(&filter).map_err(engine_error)?.into_iter().map(Unit::from).collect()))
    }


    /// Replaces the payload of `id`, adding a revision; the earlier ones stay in its history.
    fn update(&self, py: Python<'_>, id: &str, payload: Payload) -> PyResult<()> {
        let id = UnitId(parse_id(id)?);
        let bytes = into_bytes(payload);
        self.with_db(py, |db| db.update(id, bytes).map_err(engine_error))
    }

    /// Deletes `id` with a tombstone revision; its history is kept.
    fn delete(&self, py: Python<'_>, id: &str) -> PyResult<()> {
        let id = UnitId(parse_id(id)?);
        self.with_db(py, |db| db.delete(id).map_err(engine_error))
    }

    /// Removes the relation `relation_id`.
    fn unrelate(&self, py: Python<'_>, relation_id: &str) -> PyResult<()> {
        let edge = RelationId(parse_id(relation_id)?);
        self.with_db(py, |db| db.unrelate(edge).map_err(engine_error))
    }

    /// Commits `ops` atomically, as one change set. Each op is a tuple:
    /// `("store", unit_type, payload)`, `("update", id, payload)`, `("delete", id)` or
    /// `("unrelate", relation_id)`. Returns the ids of the stored units, in order.
    fn commit(&self, py: Python<'_>, ops: Vec<Bound<'_, pyo3::types::PyTuple>>) -> PyResult<Vec<String>> {
        let mut batch = Batch::new();
        for op in &ops {
            let name: String = op.get_item(0)?.extract()?;
            match (name.as_str(), op.len()) {
                ("store", 3) => {
                    let t: String = op.get_item(1)?.extract()?;
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.store(&t, into_bytes(p));
                }
                ("update", 3) => {
                    let id: String = op.get_item(1)?.extract()?;
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.update(UnitId(parse_id(&id)?), into_bytes(p));
                }
                ("delete", 2) => {
                    let id: String = op.get_item(1)?.extract()?;
                    batch.delete(UnitId(parse_id(&id)?));
                }
                ("unrelate", 2) => {
                    let id: String = op.get_item(1)?.extract()?;
                    batch.unrelate(RelationId(parse_id(&id)?));
                }
                _ => return Err(PyValueError::new_err(format!("unknown or malformed batch op: {op}"))),
            }
        }
        self.with_db(py, move |db| {
            Ok(db.commit(&batch).map_err(engine_error)?.into_iter().map(|id| id.to_string()).collect())
        })
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
        self.with_db(py, |db| {
            Ok(db.unit_db().history(id, q).map_err(engine_error)?.into_iter().map(Revision::from).collect())
        })
    }

    /// The revision of `id` in force once commit `commit_seq` committed, with its payload;
    /// `None` when the unit did not exist yet. A deleted unit returns its tombstone.
    fn read_as_of(&self, py: Python<'_>, id: &str, commit_seq: u64) -> PyResult<Option<Revision>> {
        let id = UnitId(parse_id(id)?);
        self.with_db(py, |db| match db.unit_db().unit_as_of(id, AsOf::Seq(CommitSeq(commit_seq))).map_err(engine_error)? {
            UnitAt::Absent => Ok(None),
            UnitAt::Present(r) | UnitAt::DeletedAt(r) => Ok(Some(Revision::from(r))),
            UnitAt::Pruned { oldest } => {
                Err(KernoError::new_err(format!("revision pruned by retention; oldest kept is {}", oldest.0)))
            }
        })
    }

    /// Checks every record of the units layer: `{"clean", "commit_seq", "units", "revisions",
    /// "relations", "findings"}`. A damaged store is reported, not raised.
    fn verify<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let report = self.with_db(py, |db| db.verify().map_err(engine_error))?;
        let d = PyDict::new(py);
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
#[pyfunction]
fn open(py: Python<'_>, path: PathBuf) -> PyResult<Database> {
    let db = py.detach(|| Db::open(path)).map_err(engine_error)?;
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
    Ok(())
}
