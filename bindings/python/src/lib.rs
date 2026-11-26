//! Python bindings for Kerno: the native half of the `kerno` package.
//!
//! A thin layer over [`kerno::Db`]. Ids cross the boundary as 32-digit lower-case hex strings,
//! payloads as `bytes` (a `str` is stored as its UTF-8 bytes), and every engine failure becomes
//! a `kerno.KernoError`.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::collections::HashMap;
use std::ops::Bound as Range;
use std::path::PathBuf;
use std::sync::RwLock;

use kerno::units::assoc_rank::{VectorMetric, VectorSet};
use kerno::units::hybrid::RRF_SCALE;
use kerno::units::{Access, AsOf, CommitSeq, HistoryQuery, Order, RevNo, RevisionKind, UnitAt};
use kerno::{
    AccessMode, AssocQuery, Batch, CommitMeta, Config, Db, Direction, Document, Endpoint, FieldDef, FieldType,
    FieldValue, HybridQuery, HybridWeights, Lifecycle, QueryPlan, RelationId, RetentionPolicy, RetentionRule,
    UnitFilter, UnitId, UpsertOutcome, Upserted, ValueRef, VectorLeg,
};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyValueError};
use pyo3::prelude::*;
use pyo3::pybacked::PyBackedBytes;
use pyo3::sync::PyOnceLock;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyMapping, PyString};

create_exception!(_kerno, KernoError, PyException, "Any failure reported by the Kerno engine.");
create_exception!(_kerno, NotFoundError, KernoError, "No unit, relation or revision with that id.");
create_exception!(
    _kerno,
    DeletedError,
    KernoError,
    "The unit is deleted. A wrong lifecycle for an op is an InvalidArgumentError."
);
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
    "The engine or the binding refused an argument (an id, a type name, a limit, a batch). Also a `ValueError`."
);
create_exception!(_kerno, ClosedError, KernoError, "A call on a database handle that was closed. Also a `ValueError`.");
create_exception!(
    _kerno,
    HistoryUnavailableError,
    KernoError,
    "The content of that revision is no longer stored: `compact()` dropped the payloads of a deleted unit's history."
);
create_exception!(
    _kerno,
    SchemaExistsError,
    InvalidArgumentError,
    "`define_schema` for a type that already has a different schema (the same schema again is accepted)."
);
create_exception!(
    _kerno,
    UnknownTypeError,
    InvalidArgumentError,
    "A search names a unit type that no unit, schema or type option ever used, so nothing can match."
);
create_exception!(
    _kerno,
    NotIndexedError,
    InvalidArgumentError,
    "The call needs an index that is not there: a field not declared indexed, a type with no schema, or \
     keyword search with no type switched on by `set_fulltext`."
);
create_exception!(
    _kerno,
    NotUniqueError,
    InvalidArgumentError,
    "`upsert` by a field the type's schema does not declare unique (or by a type with no schema)."
);
create_exception!(
    _kerno,
    BadEndpointError,
    InvalidArgumentError,
    "A batch `relate` end that is not an earlier `store` or `upsert` op of the batch."
);
create_exception!(_kerno, PayloadTooLargeError, KernoError, "The payload is larger than the limit (1 MiB by default).");
create_exception!(
    _kerno,
    ChangeSetTooLargeError,
    PayloadTooLargeError,
    "A batch holds more operations (10,000 by default, see `max_batch_ops`) or bytes than one commit may."
);
create_exception!(_kerno, ReadOnlyError, KernoError, "A write went through a handle opened with `read_only=True`.");
create_exception!(
    _kerno,
    DatabaseNotFoundError,
    KernoError,
    "The directory holds no database and `read_only=True` never creates one."
);
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

/// The longest unit type name, in bytes.
const MAX_TYPE_LEN: usize = 64;

const HISTORY_UNAVAILABLE: &str = "the content of that revision is no longer stored: compact() dropped the payloads \
     of a deleted unit's history (its revisions are still listed by history(), without a payload)";

/// Units `query(lifecycle=...)` reads per `state_as_of` page.
const STATE_PAGE: usize = 256;

fn engine_error(e: kerno::Error) -> PyErr {
    use kerno::Error as E;
    let msg = e.to_string();
    if e.is_locked() {
        return LockedError::new_err(msg);
    }
    if e.is_corruption() {
        return CorruptionError::new_err(msg);
    }
    if e.is_read_only() {
        return ReadOnlyError::new_err(msg);
    }
    if e.is_no_database() {
        return DatabaseNotFoundError::new_err(msg);
    }
    if e.is_engine_argument() {
        return InvalidArgumentError::new_err(msg);
    }
    match e {
        E::NotFound { .. } | E::RelationNotFound { .. } | E::RevisionNotFound { .. } => NotFoundError::new_err(msg),
        E::Deleted { .. } => DeletedError::new_err(msg),
        E::PayloadTooLarge { .. } => PayloadTooLargeError::new_err(msg),
        E::InvalidArgument(_) => InvalidArgumentError::new_err(msg),
        E::ChangeSetTooLarge { .. } => ChangeSetTooLargeError::new_err(msg),
        E::SchemaExists { .. } => SchemaExistsError::new_err(msg),
        E::NotIndexed { .. } => NotIndexedError::new_err(msg),
        E::UnknownType { .. } => UnknownTypeError::new_err(msg),
        E::NotUnique { .. } => NotUniqueError::new_err(msg),
        E::BadEndpoint { .. } => BadEndpointError::new_err(msg),
        E::UnitHasRelations { .. } => HasRelationsError::new_err(msg),
        E::Pruned { .. } => PrunedError::new_err(msg),
        E::HistoryUnavailable { .. } => HistoryUnavailableError::new_err(HISTORY_UNAVAILABLE),
        E::SchemaViolation { .. } => SchemaError::new_err(msg),
        _ => KernoError::new_err(msg),
    }
}

/// A whole number `0..2**N` (`N` the width of `T`) from Python, as the argument `what`. A
/// negative, too large, fractional, `bool` or non-number value is an `InvalidArgumentError` that
/// names `what`: the binding never lets an `OverflowError` or `TypeError` out for a number it was
/// given.
fn unsigned<T: TryFrom<u64>>(what: &str, v: &Bound<'_, PyAny>) -> PyResult<T> {
    unsigned_from(what, 0, v)
}

/// [`unsigned`] with a lowest accepted value `min` (a count that must be at least 1).
fn unsigned_from<T: TryFrom<u64>>(what: &str, min: u64, v: &Bound<'_, PyAny>) -> PyResult<T> {
    let bits = std::mem::size_of::<T>() * 8;
    let whole = v.is_instance_of::<PyInt>() && !v.is_instance_of::<PyBool>();
    whole.then(|| v.extract::<u64>().ok().filter(|n| *n >= min).and_then(|n| T::try_from(n).ok())).flatten().ok_or_else(
        || InvalidArgumentError::new_err(format!("{what} must be a whole number from {min} to 2**{bits} - 1, not {v}")),
    )
}

/// [`unsigned_from`], or `None` for Python's `None`.
fn maybe_unsigned_from<T: TryFrom<u64>>(what: &str, min: u64, v: &Bound<'_, PyAny>) -> PyResult<Option<T>> {
    if v.is_none() { Ok(None) } else { unsigned_from(what, min, v).map(Some) }
}

/// A list of names (`str`) as the argument `what`, or `None`. A bare `str` (the usual slip) and
/// any other non-list value is an `InvalidArgumentError`, not a raw extraction error.
fn names(what: &str, v: &Bound<'_, PyAny>) -> PyResult<Option<Vec<String>>> {
    if v.is_none() {
        return Ok(None);
    }
    let bad = || {
        InvalidArgumentError::new_err(format!(
            "{what} must be a list of strings (for example [\"note\"]), not {}",
            v.get_type().name().map_or_else(|_| "this value".to_owned(), |n| n.to_string())
        ))
    };
    if v.is_instance_of::<PyString>() {
        return Err(bad());
    }
    v.extract::<Vec<String>>().map(Some).map_err(|_| bad())
}

/// Defines the extractor of one whole-number parameter, for `#[pyo3(from_py_with = ...)]`: pyo3
/// hands an extractor only the value, so each parameter gets one that knows its name.
macro_rules! whole_arg {
    ($name:ident, $what:literal, $ty:ty) => {
        fn $name(v: &Bound<'_, PyAny>) -> PyResult<$ty> {
            unsigned($what, v)
        }
    };
    (maybe $name:ident, $what:literal, $ty:ty) => {
        fn $name(v: &Bound<'_, PyAny>) -> PyResult<Option<$ty>> {
            maybe_unsigned_from($what, 0, v)
        }
    };
    (at_least_1 $name:ident, $what:literal, $ty:ty) => {
        fn $name(v: &Bound<'_, PyAny>) -> PyResult<$ty> {
            unsigned_from($what, 1, v)
        }
    };
    (maybe_at_least_1 $name:ident, $what:literal, $ty:ty) => {
        fn $name(v: &Bound<'_, PyAny>) -> PyResult<Option<$ty>> {
            maybe_unsigned_from($what, 1, v)
        }
    };
    (names $name:ident, $what:literal) => {
        fn $name(v: &Bound<'_, PyAny>) -> PyResult<Option<Vec<String>>> {
            names($what, v)
        }
    };
}

whole_arg!(at_least_1 limit_arg, "limit", usize);
whole_arg!(maybe_at_least_1 max_rows_arg, "limit", usize);
whole_arg!(names types_arg, "types");
whole_arg!(names text_types_arg, "text_types");
whole_arg!(names seeds_arg, "seeds");
whole_arg!(maybe schema_version_arg, "schema_version", u32);
whole_arg!(maybe from_revision_arg, "from_revision", u64);
whole_arg!(maybe to_revision_arg, "to_revision", u64);
whole_arg!(revision_a_arg, "a", u64);
whole_arg!(revision_b_arg, "b", u64);
whole_arg!(commit_seq_arg, "commit_seq", u64);
whole_arg!(keyword_weight_arg, "keyword_weight", u32);
whole_arg!(vector_weight_arg, "vector_weight", u32);
whole_arg!(relation_weight_arg, "relation_weight", u32);
whole_arg!(candidates_arg, "candidates", usize);
whole_arg!(rrf_k_arg, "rrf_k", u32);
whole_arg!(max_depth_arg, "max_depth", u32);
/// `max_batch_ops`: `1 ..= MAX_MEMBER_COUNT` (the most members one change set can record).
fn max_batch_ops_arg(v: &Bound<'_, PyAny>) -> PyResult<Option<usize>> {
    let max = u64::from(kerno::units::codec::MAX_MEMBER_COUNT);
    let whole = v.is_instance_of::<PyInt>() && !v.is_instance_of::<PyBool>();
    if v.is_none() {
        return Ok(None);
    }
    whole
        .then(|| v.extract::<u64>().ok().filter(|n| (1..=max).contains(n)))
        .flatten()
        .map(|n| Some(n as usize))
        .ok_or_else(|| {
            InvalidArgumentError::new_err(format!("max_batch_ops must be a whole number from 1 to {max}, not {v}"))
        })
}

/// A Python value as a document field: bool, int (64 bits), float, str or bytes. Anything else
/// is an `InvalidArgumentError`, as is an int outside `-2**63 .. 2**63 - 1`.
fn to_field(v: &Bound<'_, PyAny>) -> PyResult<FieldValue> {
    if v.is_instance_of::<PyBool>() {
        Ok(FieldValue::Bool(v.extract()?))
    } else if v.is_instance_of::<PyInt>() {
        let fits = v.extract::<i64>();
        fits.map(FieldValue::Int).map_err(|_| {
            InvalidArgumentError::new_err(format!("the integer {v} does not fit 64 bits (-2**63 to 2**63 - 1)"))
        })
    } else if v.is_instance_of::<PyFloat>() {
        Ok(FieldValue::Float(v.extract()?))
    } else if v.is_instance_of::<PyString>() {
        Ok(FieldValue::Text(v.extract()?))
    } else if let Ok(Payload(b)) = v.extract::<Payload>() {
        // Only byte buffers: a list or tuple of small ints is not bytes.
        Ok(FieldValue::Bytes(b))
    } else {
        Err(InvalidArgumentError::new_err(format!(
            "unsupported field value {v}: a field is a bool, int, float, str or bytes"
        )))
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

fn value_ref_to_py<'py>(py: Python<'py>, v: ValueRef<'_>) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        ValueRef::Bool(b) => PyBool::new(py, b).to_owned().into_any(),
        ValueRef::Int(i) => i.into_pyobject(py)?.into_any(),
        ValueRef::Float(f) => f.into_pyobject(py)?.into_any(),
        ValueRef::Text(t) => PyString::new(py, t).into_any(),
        ValueRef::Bytes(b) => PyBytes::new(py, b).into_any(),
    })
}

/// A retention rule from its dict: exactly one of `keep_last` (a whole number), `keep_for`
/// (seconds) or `keep_since` (a whole number). Every bad rule is a `ValueError`, never a
/// truncated or wrapped-around number.
fn retention_rule(rule: &Bound<'_, PyAny>) -> PyResult<RetentionRule> {
    let bad = || {
        InvalidArgumentError::new_err("a retention rule is a dict with exactly one of keep_last, keep_for, keep_since")
    };
    let d = rule.cast::<PyDict>().map_err(|_| bad())?;
    let Some((name, value)) = d.iter().next().filter(|_| d.len() == 1) else {
        return Err(bad());
    };
    let name: String = name.extract().map_err(|_| bad())?;
    // `True` is an int to Python; a rule never means it.
    let number = !value.is_instance_of::<PyBool>();
    match name.as_str() {
        "keep_last" => unsigned_from("keep_last", 1, &value).map(RetentionRule::KeepLast),
        "keep_since" => unsigned_from("keep_since", 0, &value).map(|s| RetentionRule::KeepSince(CommitSeq(s))),
        "keep_for" => value
            .extract::<f64>()
            .ok()
            .filter(|_| number)
            .and_then(|secs| std::time::Duration::try_from_secs_f64(secs).ok())
            .map(RetentionRule::KeepFor)
            .ok_or_else(|| {
                InvalidArgumentError::new_err(format!("keep_for is a number of seconds, 0 or more, not {value}"))
            }),
        _ => Err(bad()),
    }
}

fn field_type(name: &str) -> PyResult<FieldType> {
    Ok(match name {
        "bool" => FieldType::Bool,
        "int" => FieldType::Int,
        "float" => FieldType::Float,
        "text" => FieldType::Text,
        "bytes" => FieldType::Bytes,
        other => return Err(InvalidArgumentError::new_err(format!("unknown field type {other:?}"))),
    })
}

/// `obj` as a `String`, converted only when it is not the very object `cache` holds (which
/// keeps it alive, so its address cannot be reused while cached). Errors are `extract`'s own.
fn cached_str<'a, 'py>(
    cache: &'a mut Option<(Bound<'py, PyAny>, String)>,
    obj: Borrowed<'_, 'py, PyAny>,
) -> PyResult<&'a str> {
    if !matches!(cache, Some((o, _)) if o.as_ptr() == obj.as_ptr()) {
        let s: String = obj.extract()?;
        *cache = Some((obj.to_owned(), s));
    }
    Ok(cache.as_ref().map_or("", |(_, s)| s.as_str()))
}

/// The ids of the `str` objects a `commit` call met, by object address: callers pass the same
/// id object op after op (a unit's id in each of its relations). Direct-mapped; each slot holds
/// a strong reference, so an address cannot be reused by another object while it is cached.
/// Only `str`s that parsed are kept, and a `str` is immutable, so a hit is what parsing gives.
struct IdCache<'py> {
    slots: Vec<Option<(Bound<'py, PyAny>, [u8; 16])>>,
}

impl<'py> IdCache<'py> {
    const BITS: u32 = 12;

    fn new() -> Self {
        IdCache { slots: Vec::new() }
    }

    /// [`parse_id_py`] of `v`, from the cache when `v` is an object met before.
    fn parse(&mut self, v: &Bound<'py, PyAny>) -> PyResult<[u8; 16]> {
        let addr = v.as_ptr() as usize;
        let at = (addr >> 4).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (usize::BITS - Self::BITS);
        if let Some(Some((o, id))) = self.slots.get(at)
            && o.as_ptr() == v.as_ptr()
        {
            return Ok(*id);
        }
        let id = parse_id_py(v)?;
        if self.slots.is_empty() {
            self.slots.resize_with(1 << Self::BITS, || None);
        }
        self.slots[at] = Some((v.clone(), id));
        Ok(id)
    }

    /// A relation end in a batch: a unit id (hex, through the cache), or the position in the
    /// op list of an earlier "store" or "upsert" op.
    fn endpoint(&mut self, v: &Bound<'py, PyAny>) -> PyResult<Endpoint> {
        if v.is_instance_of::<PyString>() {
            Ok(Endpoint::Unit(UnitId(self.parse(v)?)))
        } else {
            Ok(Endpoint::Op(unsigned("a relate end", v)?))
        }
    }
}

/// [`parse_id`] of a Python `str`, read into a stack buffer instead of a `String` when it is
/// 32 ASCII hex digits; anything else goes the [`parse_id`] way, so errors are unchanged.
fn parse_id_py(v: &Bound<'_, PyAny>) -> PyResult<[u8; 16]> {
    if let Some(id) = fast_id(v) {
        return Ok(id);
    }
    parse_id(&v.extract::<String>()?)
}

/// The id a `str` of exactly 32 ASCII hex digits spells, or `None` for any other value.
fn fast_id(v: &Bound<'_, PyAny>) -> Option<[u8; 16]> {
    if !v.is_instance_of::<PyString>() {
        return None;
    }
    let mut buf = [0u32; 33];
    // SAFETY: `v` is a `str` (checked above), and `PyUnicode_AsUCS4` writes at most `buf.len()`
    // code points (plus no terminator, `copy_null = 0`) into the buffer it is given; a longer
    // string fails without writing past it. Both calls are in the limited API since 3.3.
    let n = unsafe {
        let len = pyo3::ffi::PyUnicode_GetLength(v.as_ptr());
        if len != 32 {
            if len < 0 {
                pyo3::ffi::PyErr_Clear();
            }
            return None;
        }
        pyo3::ffi::PyUnicode_AsUCS4(v.as_ptr(), buf.as_mut_ptr().cast(), buf.len() as pyo3::ffi::Py_ssize_t, 0)
    };
    if n.is_null() {
        // SAFETY: the failed call set an exception; the slow path raises its own.
        unsafe { pyo3::ffi::PyErr_Clear() };
        return None;
    }
    // Each code point through a table: `0x10` marks a non-digit, OR-ed into `bad` so the loop
    // has no branch per digit.
    let mut id = [0u8; 16];
    let mut bad = 0u8;
    let nib = |c: u32| if c < 128 { HEX[c as usize] } else { 0x10 };
    for (byte, pair) in id.iter_mut().zip(buf[..32].chunks_exact(2)) {
        let (hi, lo) = (nib(pair[0]), nib(pair[1]));
        bad |= hi | lo;
        *byte = hi << 4 | (lo & 0x0f);
    }
    (bad & 0x10 == 0).then_some(id)
}

/// The value of an ASCII hex digit, or `0x10` for any other byte below 128.
const HEX: [u8; 128] = {
    let mut t = [0x10u8; 128];
    let mut i = 0;
    while i < 10 {
        t[b'0' as usize + i] = i as u8;
        i += 1;
    }
    let mut i = 0;
    while i < 6 {
        t[b'a' as usize + i] = 10 + i as u8;
        t[b'A' as usize + i] = 10 + i as u8;
        i += 1;
    }
    t
};

fn parse_id(hex: &str) -> PyResult<[u8; 16]> {
    let bad = || InvalidArgumentError::new_err(format!("not a unit id (expected 32 hex digits): {hex:?}"));
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
struct Payload(Vec<u8>);

impl<'a, 'py> FromPyObject<'a, 'py> for Payload {
    type Error = PyErr;

    /// A `str`, or anything that exposes a byte buffer (`bytes`, `bytearray`, `memoryview`).
    /// A list of ints is not bytes; every other value is an `InvalidArgumentError`, never a `TypeError`.
    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        if obj.is_instance_of::<PyString>() {
            return Ok(Payload(obj.extract::<String>()?.into_bytes()));
        }
        // The buffer API is not in the stable ABI, so a memoryview is copied out with `tobytes()`.
        let bytes = if obj.get_type().name()?.extract::<String>()? == "memoryview" {
            obj.call_method0("tobytes")?
        } else {
            obj.to_owned().into_any().to_owned()
        };
        bytes.extract::<PyBackedBytes>().map(|b| Payload(b.to_vec())).map_err(|_| {
            InvalidArgumentError::new_err("a payload is a str or a bytes-like object (bytes, bytearray, memoryview)")
        })
    }
}

/// `{"id": hex, "outcome": "created" | "updated" | "unchanged"}`.
fn upserted_dict<'py>(py: Python<'py>, u: &Upserted) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("id", u.id.to_string())?;
    d.set_item(
        "outcome",
        match u.outcome {
            UpsertOutcome::Created => "created",
            UpsertOutcome::Updated => "updated",
            UpsertOutcome::Unchanged => "unchanged",
        },
    )?;
    Ok(d)
}

fn lifecycle_name(l: Lifecycle) -> &'static str {
    match l {
        Lifecycle::Active => "active",
        Lifecycle::Archived => "archived",
        Lifecycle::Deleted => "deleted",
    }
}

fn into_bytes(p: Payload) -> Vec<u8> {
    p.0
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
    /// Descriptive wall-clock time, in microseconds since the Unix epoch; never used for ordering.
    #[pyo3(get)]
    created_at_us: i64,
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
            created_at_us: r.created_at.0,
            lifecycle: lifecycle_name(r.lifecycle),
            schema_version: r.schema_version,
            content_hash: encode_hex(&r.content_hash.digest),
            payload: r.payload,
        }
    }
}

/// Lowercase hex of `bytes`, byte-identical to `format!("{b:02x}")` per byte, built in one
/// allocation instead of one `String` per byte.
fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 0x0f)] as char);
    }
    out
}

/// A stored unit, as of its latest revision.
#[pyclass(frozen, module = "kerno")]
struct Unit {
    id: UnitId,
    /// The id as the Python string, made the first time it is asked for: a lookup that
    /// returns hundreds of units formats the ids of those the caller looks at.
    id_str: PyOnceLock<Py<PyString>>,
    #[pyo3(get)]
    unit_type: String,
    #[pyo3(get)]
    schema_version: u32,
    #[pyo3(get)]
    lifecycle: &'static str,
    #[pyo3(get)]
    revision: u64,
    /// The oldest revision retention still keeps: `history(from_revision=...)` and `read_as_of`
    /// fail with `PrunedError` below it. 1 until retention prunes the unit.
    #[pyo3(get)]
    oldest_revision: u64,
    /// When the unit was created, in microseconds since the Unix epoch.
    #[pyo3(get)]
    created_at_us: i64,
    /// When the unit's head revision was written, in microseconds since the Unix epoch.
    #[pyo3(get)]
    updated_at_us: i64,
    payload: Option<Vec<u8>>,
}

#[pymethods]
impl Unit {
    /// The unit's id: 32 lower-case hex digits.
    #[getter]
    fn id<'py>(&self, py: Python<'py>) -> Bound<'py, PyString> {
        self.id_str.get_or_init(py, || PyString::new(py, &self.id.to_string()).unbind()).bind(py).clone()
    }

    /// The content, as `bytes`; `None` for a deleted unit.
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
        self.payload.as_deref().map(|p| PyBytes::new(py, p))
    }

    /// The payload decoded as a document (`dict`), without reading the database again; `None`
    /// for a deleted unit. Raises `SchemaError` when the payload is not a document.
    fn document<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        payload_document(py, self.payload.as_deref())
    }

    fn __repr__(&self) -> String {
        format!("Unit(id={:?}, unit_type={:?}, lifecycle={:?})", self.id.to_string(), self.unit_type, self.lifecycle)
    }
}

/// A payload decoded as a document dict (`None` for no payload), straight from the bytes: no
/// intermediate document. Raises `SchemaError` when the payload is not a document.
fn payload_document<'py>(py: Python<'py>, payload: Option<&[u8]>) -> PyResult<Option<Bound<'py, PyDict>>> {
    DocBuilder::new(None).build(py, payload)
}

/// Builds document dicts for one call: each field name becomes a Python str once and is reused
/// as the key of every dict; with `fields`, only those names are kept.
struct DocBuilder {
    keys: Vec<(String, Py<PyString>)>,
    fields: Option<Vec<String>>,
    /// `fields` given as a mapping: each kept field name and its key in the result, interned
    /// once, in the mapping's order.
    renamed: Option<Vec<(String, Py<PyString>)>>,
}

/// What `fields=` of `find_documents` / `neighbor_documents` takes: a sequence of field names,
/// or a mapping `{field_name: key_in_result}`.
enum FieldsArg {
    Names(Vec<String>),
    Renamed(Vec<(String, Py<PyString>)>),
}

/// `fields=` of `find_documents` / `neighbor_documents`, for `#[pyo3(from_py_with = ...)]`
/// (a sequence that is not one of names fails as when the parameter was typed as one).
fn fields_arg(ob: &Bound<'_, PyAny>) -> PyResult<Option<FieldsArg>> {
    if ob.is_none() {
        return Ok(None);
    }
    if let Ok(m) = ob.cast::<PyMapping>() {
        let py = ob.py();
        let mut out = Vec::new();
        for item in m.items()?.iter() {
            let (name, key): (String, String) = item.extract()?;
            out.push((name, PyString::intern(py, &key).unbind()));
        }
        return Ok(Some(FieldsArg::Renamed(out)));
    }
    Ok(Some(FieldsArg::Names(ob.extract()?)))
}

impl DocBuilder {
    fn new(fields: Option<FieldsArg>) -> Self {
        match fields {
            Some(FieldsArg::Renamed(r)) => DocBuilder { keys: Vec::new(), fields: None, renamed: Some(r) },
            Some(FieldsArg::Names(f)) => DocBuilder { keys: Vec::new(), fields: Some(f), renamed: None },
            None => DocBuilder { keys: Vec::new(), fields: None, renamed: None },
        }
    }

    /// The key of `name`, the `at`-th field kept: documents of one type list their fields in one
    /// order, so the name seen at that position last time is tried first.
    fn key<'py>(&mut self, py: Python<'py>, at: usize, name: &str) -> Bound<'py, PyString> {
        if let Some((n, k)) = self.keys.get(at)
            && n == name
        {
            return k.bind(py).clone();
        }
        if let Some((_, k)) = self.keys.iter().find(|(n, _)| n == name) {
            return k.bind(py).clone();
        }
        let k = PyString::intern(py, name);
        self.keys.push((name.to_owned(), k.clone().unbind()));
        k
    }

    fn build<'py>(&mut self, py: Python<'py>, payload: Option<&[u8]>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(payload) = payload else { return Ok(None) };
        if let Some(renamed) = &self.renamed {
            return Self::build_renamed(py, renamed, payload).map(Some);
        }
        let d = PyDict::new(py);
        let mut failed: Option<PyErr> = None;
        let mut at = 0;
        let walked = Document::visit(payload, |name, value| {
            if failed.is_some() || self.fields.as_ref().is_some_and(|f| !f.iter().any(|n| n == name)) {
                return;
            }
            let key = self.key(py, at, name);
            at += 1;
            if let Err(e) = value_ref_to_py(py, value).and_then(|v| d.set_item(key, v)) {
                failed = Some(e);
            }
        });
        if let Err(e) = walked {
            return Err(SchemaError::new_err(format!("the payload is not a document: {e}")));
        }
        match failed {
            Some(e) => Err(e),
            None => Ok(Some(d)),
        }
    }

    /// [`build`](Self::build) with `fields` as a mapping: the kept fields under their mapped
    /// keys, in the mapping's order (a name the document lacks is absent).
    fn build_renamed<'py>(
        py: Python<'py>,
        renamed: &[(String, Py<PyString>)],
        payload: &[u8],
    ) -> PyResult<Bound<'py, PyDict>> {
        let mut values: Vec<Option<Bound<'py, PyAny>>> = (0..renamed.len()).map(|_| None).collect();
        let mut failed: Option<PyErr> = None;
        let walked = Document::visit(payload, |name, value| {
            if failed.is_some() {
                return;
            }
            if let Some((slot, _)) = values.iter_mut().zip(renamed).find(|(_, (n, _))| n == name) {
                match value_ref_to_py(py, value) {
                    Ok(v) => *slot = Some(v),
                    Err(e) => failed = Some(e),
                }
            }
        });
        if let Err(e) = walked {
            return Err(SchemaError::new_err(format!("the payload is not a document: {e}")));
        }
        if let Some(e) = failed {
            return Err(e);
        }
        let d = PyDict::new(py);
        for (v, (_, key)) in values.into_iter().zip(renamed) {
            if let Some(v) = v {
                d.set_item(key.bind(py), v)?;
            }
        }
        Ok(d)
    }
}

impl From<kerno::Unit> for Unit {
    fn from(u: kerno::Unit) -> Self {
        Unit {
            id: u.id,
            id_str: PyOnceLock::new(),
            unit_type: u.unit_type,
            schema_version: u.schema_version,
            lifecycle: lifecycle_name(u.lifecycle),
            revision: u.head.0,
            oldest_revision: u.oldest.0,
            created_at_us: u.created_at.0,
            updated_at_us: u.updated_at.0,
            payload: u.payload,
        }
    }
}

/// A directed, typed edge between two units.
///
/// The fields are Python strings made once: reading an attribute hands out a reference, it does
/// not build a new `str` (a scan of every relation reads three attributes per relation).
#[pyclass(frozen, module = "kerno")]
struct Relation {
    #[pyo3(get)]
    id: Py<PyString>,
    #[pyo3(get)]
    from_id: Py<PyString>,
    #[pyo3(get)]
    to_id: Py<PyString>,
    #[pyo3(get)]
    relation_type: Py<PyString>,
}

#[pymethods]
impl Relation {
    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "Relation(from_id={:?}, to_id={:?}, relation_type={:?})",
            self.from_id.bind(py).to_string_lossy(),
            self.to_id.bind(py).to_string_lossy(),
            self.relation_type.bind(py).to_string_lossy()
        )
    }
}

/// Python `Relation`s of engine relations. A relation type is one shared `str` per distinct
/// name: a database has few types and many relations.
struct RelationMaker<'py> {
    py: Python<'py>,
    types: HashMap<String, Py<PyString>>,
    /// One `str` per unit id: the ends of a database's relations repeat (many relations, fewer
    /// units), and a caller that keys a dict by them hashes each `str` once.
    units: HashMap<UnitId, Py<PyString>>,
}

impl<'py> RelationMaker<'py> {
    fn new(py: Python<'py>) -> Self {
        RelationMaker { py, types: HashMap::new(), units: HashMap::new() }
    }

    fn unit(&mut self, id: UnitId) -> Py<PyString> {
        let py = self.py;
        self.units.entry(id).or_insert_with(|| PyString::new(py, &id.to_string()).unbind()).clone_ref(py)
    }

    fn make(&mut self, r: kerno::Relation) -> Relation {
        let py = self.py;
        let relation_type = match self.types.get(&r.relation_type) {
            Some(t) => t.clone_ref(py),
            None => {
                let t = PyString::new(py, &r.relation_type).unbind();
                self.types.insert(r.relation_type, t.clone_ref(py));
                t
            }
        };
        Relation {
            id: PyString::new(py, &RelationId::to_string(&r.id)).unbind(),
            from_id: self.unit(r.from),
            to_id: self.unit(r.to),
            relation_type,
        }
    }
}

/// Two revisions of one unit, from `compare`.
#[pyclass(frozen, module = "kerno")]
struct Comparison {
    #[pyo3(get)]
    a: Py<Revision>,
    #[pyo3(get)]
    b: Py<Revision>,
    /// Same content hash and same payload bytes (two tombstones count as the same).
    #[pyo3(get)]
    same_content: bool,
}

/// One hit of `hybrid_search`. A score is that signal's share of `score`; it and its rank are
/// `None` when the signal did not find the unit.
#[pyclass(frozen, module = "kerno", name = "HybridHit")]
struct SearchHit {
    #[pyo3(get)]
    id: String,
    /// The sum of the signal scores.
    #[pyo3(get)]
    score: f64,
    #[pyo3(get)]
    keyword_score: Option<f64>,
    #[pyo3(get)]
    vector_score: Option<f64>,
    #[pyo3(get)]
    relation_score: Option<f64>,
    /// 1-based rank of the unit in the keyword (BM25) signal.
    #[pyo3(get)]
    keyword_rank: Option<u32>,
    #[pyo3(get)]
    vector_rank: Option<u32>,
    #[pyo3(get)]
    relation_rank: Option<u32>,
}

#[pymethods]
impl SearchHit {
    fn __repr__(&self) -> String {
        format!("HybridHit(id={:?}, score={})", self.id, self.score)
    }
}

/// One hit of `search_text`: the unit's id and its BM25 score (higher is a better match).
#[pyclass(frozen, module = "kerno", name = "TextHit")]
struct TextHit {
    #[pyo3(get)]
    id: String,
    #[pyo3(get)]
    score: f64,
}

#[pymethods]
impl TextHit {
    fn __repr__(&self) -> String {
        format!("TextHit(id={:?}, score={})", self.id, self.score)
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
                None => Err(ClosedError::new_err("operation on a closed database")),
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
    /// `relation_type`. An invalid type name raises `InvalidArgumentError`, and an id no unit ever
    /// had raises `NotFoundError` (a deleted unit is known and has none).
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
            other => {
                return Err(InvalidArgumentError::new_err(format!("direction must be 'out' or 'in', not {other:?}")));
            }
        };
        let edges = self.with_db(py, |db| db.related(id, direction, relation_type).map_err(engine_error))?;
        let mut maker = RelationMaker::new(py);
        Ok(edges.into_iter().map(|r| maker.make(r)).collect())
    }

    /// Every live relation, optionally of one `relation_type`, in one scan: much faster than
    /// calling `related` for each unit.
    #[pyo3(signature = (relation_type=None))]
    fn relations(&self, py: Python<'_>, relation_type: Option<&str>) -> PyResult<Vec<Relation>> {
        let edges = self.with_db(py, |db| db.relations(relation_type).map_err(engine_error))?;
        let mut maker = RelationMaker::new(py);
        Ok(edges.into_iter().map(|r| maker.make(r)).collect())
    }

    /// The relations of `id` like [`related`](Self::related), each with the unit at its other
    /// end, in one call: `[(Relation, Unit), ...]`. A relation whose other end no longer exists
    /// is left out. With `type_prefix`, relations whose type does not start with it are skipped
    /// before the unit at their other end is loaded.
    #[pyo3(signature = (id, direction="out", relation_type=None, type_prefix=None))]
    fn neighbors(
        &self,
        py: Python<'_>,
        id: &str,
        direction: &str,
        relation_type: Option<&str>,
        type_prefix: Option<&str>,
    ) -> PyResult<Vec<(Relation, Unit)>> {
        let id = UnitId(parse_id(id)?);
        let direction = match direction {
            "out" => Direction::Out,
            "in" => Direction::In,
            other => {
                return Err(InvalidArgumentError::new_err(format!("direction must be 'out' or 'in', not {other:?}")));
            }
        };
        let pairs = self.with_db(py, |db| {
            let edges = db.related(id, direction, relation_type).map_err(engine_error)?;
            let mut out = Vec::with_capacity(edges.len());
            for r in edges {
                if type_prefix.is_some_and(|p| !r.relation_type.starts_with(p)) {
                    continue;
                }
                let other = if matches!(direction, Direction::Out) { r.to } else { r.from };
                let Some(unit) = db.get(other).map_err(engine_error)? else { continue };
                out.push((r, unit));
            }
            Ok(out)
        })?;
        let mut maker = RelationMaker::new(py);
        Ok(pairs.into_iter().map(|(r, u)| (maker.make(r), Unit::from(u))).collect())
    }

    /// The units in id order; `unit_type` (an invalid name raises `InvalidArgumentError`, for every
    /// `lifecycle`) and
    /// `schema_version` narrow them. `lifecycle` is
    /// "active" (the default), "archived", "deleted" or "all". A deleted unit has no payload.
    #[pyo3(signature = (unit_type=None, schema_version=None, lifecycle="active"))]
    fn query(
        &self,
        py: Python<'_>,
        unit_type: Option<&str>,
        #[pyo3(from_py_with = schema_version_arg)] schema_version: Option<u32>,
        lifecycle: &str,
    ) -> PyResult<Vec<Unit>> {
        let wanted = match lifecycle {
            "active" => Some(Lifecycle::Active),
            "archived" => Some(Lifecycle::Archived),
            "deleted" => Some(Lifecycle::Deleted),
            "all" => None,
            other => {
                return Err(InvalidArgumentError::new_err(format!(
                    "lifecycle must be 'active', 'archived', 'deleted' or 'all', not {other:?}"
                )));
            }
        };
        let filter = UnitFilter { unit_type: unit_type.map(str::to_owned), schema_version };
        self.with_db(py, |db| {
            // `filter.matches` below checks nothing, so refuse a bad type name here for every
            // lifecycle, as the "active" path does.
            if wanted != Some(Lifecycle::Active)
                && let Some(t) = unit_type.filter(|t| t.is_empty() || t.len() > MAX_TYPE_LEN)
            {
                return Err(InvalidArgumentError::new_err(format!(
                    "unit_type must be 1 to {MAX_TYPE_LEN} bytes, not {} bytes",
                    t.len()
                )));
            }
            if wanted == Some(Lifecycle::Active) {
                return Ok(db.query(&filter).map_err(engine_error)?.into_iter().map(Unit::from).collect());
            }
            // ponytail: reads every unit's head through `state_as_of` (linear in the store, and
            // it verifies each payload); the engine has no index of archived or deleted units.
            let snap = db.unit_db().snapshot().map_err(engine_error)?;
            let at = AsOf::Seq(snap.seq());
            let (mut out, mut after) = (Vec::new(), None);
            loop {
                let page = snap.state_as_of(at, after, STATE_PAGE).map_err(engine_error)?;
                for (id, state) in &page {
                    let (UnitAt::Present(r) | UnitAt::DeletedAt(r)) = state else { continue };
                    if wanted.is_some_and(|l| l != r.lifecycle) {
                        continue;
                    }
                    let unit = snap.get_unit(*id).map_err(engine_error)?;
                    out.extend(unit.filter(|u| filter.matches(&u.unit_type, u.schema_version)).map(Unit::from));
                }
                match page.last() {
                    Some((id, _)) if page.len() == STATE_PAGE => after = Some(*id),
                    _ => return Ok(out),
                }
            }
        })
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

    /// Turns keyword (BM25) indexing of `unit_type` on or off. It is off by default (ADR-0023);
    /// the setting is stored in the database, and the type's active units are indexed (or
    /// cleared) in the same commit.
    fn set_fulltext(&self, py: Python<'_>, unit_type: &str, enabled: bool) -> PyResult<()> {
        self.with_db(py, |db| db.set_fulltext(unit_type, enabled).map_err(engine_error))
    }

    /// The units whose text matches `query` (BM25, any of its words), best first, at most
    /// `limit`. Only types turned on with `set_fulltext` are searched; `types` (a list of unit
    /// type names) restricts the search to those types, `None` searches every indexed type.
    /// A search that could never match raises instead of returning `[]`: `NotIndexedError` when
    /// no type is keyword-indexed or a type in `types` is not, `UnknownTypeError` for a type
    /// nothing ever used, `InvalidArgumentError` for `limit < 1`, a query with no
    /// word, or an empty `types`. A unit scores the same with or without `types`.
    #[pyo3(signature = (query, limit=10, types=None))]
    fn search_text(
        &self,
        py: Python<'_>,
        query: &str,
        #[pyo3(from_py_with = limit_arg)] limit: usize,
        #[pyo3(from_py_with = types_arg)] types: Option<Vec<String>>,
    ) -> PyResult<Vec<TextHit>> {
        self.with_db(py, |db| {
            let names: Option<Vec<&str>> = types.as_ref().map(|t| t.iter().map(String::as_str).collect());
            let hits = db.search_text(query, limit, names.as_deref()).map_err(engine_error)?;
            Ok(hits.into_iter().map(|h| TextHit { id: h.unit.to_string(), score: h.score }).collect())
        })
    }

    /// Whether `unit_type` is keyword-indexed (off by default).
    fn fulltext(&self, py: Python<'_>, unit_type: &str) -> PyResult<bool> {
        self.with_db(py, |db| db.fulltext(unit_type).map_err(engine_error))
    }

    /// Turns the substring (trigram) index of `unit_type` on or off. It is off by default
    /// (ADR-0025); the setting is stored in the database, and the type's active units are
    /// indexed (or cleared) in the same commit.
    fn set_substring(&self, py: Python<'_>, unit_type: &str, enabled: bool) -> PyResult<()> {
        self.with_db(py, |db| db.set_substring(unit_type, enabled).map_err(engine_error))
    }

    /// Whether `unit_type` is substring-indexed (off by default).
    fn substring(&self, py: Python<'_>, unit_type: &str) -> PyResult<bool> {
        self.with_db(py, |db| db.substring(unit_type).map_err(engine_error))
    }

    /// The ids of the units whose payload holds `needle`, in id order, at most `limit` (`None`:
    /// all of them). ASCII letters match in either case, every other byte must be equal, like
    /// SQL's `LIKE '%needle%'`. Only types turned on with `set_substring` are searched; `types`
    /// restricts the search to those types. A search that could never match raises instead of
    /// returning `[]`, with the errors of `search_text`.
    #[pyo3(signature = (needle, limit=None, types=None))]
    fn search_substring(
        &self,
        py: Python<'_>,
        needle: Payload,
        #[pyo3(from_py_with = max_rows_arg)] limit: Option<usize>,
        #[pyo3(from_py_with = types_arg)] types: Option<Vec<String>>,
    ) -> PyResult<Vec<String>> {
        let needle = into_bytes(needle);
        self.with_db(py, |db| {
            let names: Option<Vec<&str>> = types.as_ref().map(|t| t.iter().map(String::as_str).collect());
            let found = db.search_substring(&needle, limit, names.as_deref()).map_err(engine_error)?;
            Ok(found.iter().map(ToString::to_string).collect())
        })
    }

    /// The sequence of the last commit (0 for an empty database): the point to pass to
    /// `read_as_of` for "now".
    fn last_commit_seq(&self, py: Python<'_>) -> PyResult<u64> {
        self.with_db(py, |db| db.last_commit_seq().map_err(engine_error))
    }

    /// Sets how many bytes of units already read and checked the handle keeps in memory to
    /// answer the same reads again until the next commit (32 MiB by default); 0 turns that
    /// off, so every read goes to the files. The cache starts empty either way.
    fn set_read_cache_bytes(&self, py: Python<'_>, bytes: usize) -> PyResult<()> {
        self.with_db(py, |db| {
            db.unit_db().set_read_cache_bytes(bytes);
            Ok(())
        })
    }

    /// Moves a database opened with `read_only=True` to the commits the writer made since it
    /// opened (or since the last refresh); returns the commit sequence it now reads. Raises
    /// `InvalidArgumentError` on a handle that is not read-only (a read-write handle is always
    /// current).
    fn refresh(&self, py: Python<'_>) -> PyResult<u64> {
        self.with_db(py, |db| {
            db.refresh().map_err(|e| {
                if e.is_engine_argument() {
                    InvalidArgumentError::new_err(
                        "refresh() is for a database opened with read_only=True; a read-write handle is always current",
                    )
                } else {
                    engine_error(e)
                }
            })
        })
    }

    /// Drops the payloads of deleted units, compacts the revision ranges and returns the stored
    /// bytes as `{"before": bytes, "after": bytes}`. `before` is the size when the call was made,
    /// counting every commit so far (the write buffer is written out first); `after` is the size
    /// once the space is given back. A deleted unit keeps its history entries but not their
    /// payloads, so an as-of read of one of its old revisions is no longer possible. `after` can
    /// be larger than `before` when there is little to reclaim.
    fn compact<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let r = self.with_db(py, |db| db.compact().map_err(engine_error))?;
        let d = PyDict::new(py);
        d.set_item("before", r.stored_bytes_before)?;
        d.set_item("after", r.stored_bytes_after)?;
        Ok(d)
    }

    /// Prunes old revisions and returns what it removed: `{"units_retained", "revisions_pruned",
    /// "payloads_dropped", "bytes_logically_freed", "commit_seq"}` (`commit_seq` is the last
    /// retention commit, `None` when there was nothing to prune). `by_type` maps a unit type to
    /// its rule and `default` is the rule of every other type; no rule keeps everything. A rule
    /// is a dict with exactly one of `keep_last` (the newest N revisions, N >= 1), `keep_for`
    /// (the revisions younger than that many seconds) or `keep_since` (what answers an as-of
    /// read at that commit sequence or later). The head is always kept. Afterwards `history` from
    /// a pruned revision and `read_as_of` before the oldest kept one raise `PrunedError`.
    /// The policy is not stored: pass it each time. Idempotent.
    #[pyo3(signature = (default=None, by_type=None))]
    fn run_retention<'py>(
        &self,
        py: Python<'py>,
        default: Option<Bound<'py, PyAny>>,
        by_type: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mut policy = RetentionPolicy::all(default.map(|r| retention_rule(&r)).transpose()?.unwrap_or_default());
        if let Some(by_type) = by_type {
            let by_type = by_type
                .cast::<PyDict>()
                .map_err(|_| InvalidArgumentError::new_err("by_type is a dict of unit type to rule"))?;
            for (unit_type, rule) in by_type.iter() {
                let unit_type: String = unit_type
                    .extract()
                    .map_err(|_| InvalidArgumentError::new_err(format!("a unit type is a str, not {unit_type}")))?;
                // `None` is no rule, as `default=None` is.
                if !rule.is_none() {
                    policy = policy.with_type(&unit_type, retention_rule(&rule)?);
                }
            }
        }
        let r = self.with_db(py, |db| db.run_retention(&policy).map_err(engine_error))?;
        let d = PyDict::new(py);
        d.set_item("units_retained", r.units_retained)?;
        d.set_item("revisions_pruned", r.revisions_pruned)?;
        d.set_item("payloads_dropped", r.payloads_dropped)?;
        d.set_item("bytes_logically_freed", r.bytes_logically_freed)?;
        d.set_item("commit_seq", r.change_set.map(|c| c.commit_seq.0))?;
        Ok(d)
    }

    /// Gives `unit_type` a schema. `fields` is a list of dicts: `name`, `type` ("bool", "int",
    /// "float", "text", "bytes"), and optional `required`, `unique`, `indexed`, `covering` booleans
    /// (`unique` implies `indexed`; `covering`, valid only with `indexed` or `unique`, stores each
    /// unit's document in its index entries so `find_documents` reads no unit; defining the schema
    /// again with only `covering` changed rewrites those entries). Defining the schema the type has already (field order aside) is
    /// accepted and writes nothing, so a program can define on every start; a different one
    /// raises `SchemaExistsError`. A field added to the index later with `create_index` makes the
    /// first field list a different schema. An indexed text or bytes value holds at most 254
    /// bytes (a 0x00 byte counts twice); a longer one raises `SchemaError` on write.
    fn define_schema(&self, py: Python<'_>, unit_type: &str, fields: Vec<Bound<'_, PyDict>>) -> PyResult<()> {
        let mut defs = Vec::new();
        for f in &fields {
            let name: String =
                f.get_item("name")?.ok_or_else(|| InvalidArgumentError::new_err("field without name"))?.extract()?;
            let ty: String =
                f.get_item("type")?.ok_or_else(|| InvalidArgumentError::new_err("field without type"))?.extract()?;
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
            if flag("covering")? {
                def = def.covering();
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

    /// Stores `document` as a `unit_type` unit, or revises the active unit that already holds the
    /// same value of `field`, a field the type's schema declares `unique`. Returns `{"id",
    /// "outcome"}`, the outcome being "created", "updated" or "unchanged" (the unit had this
    /// document already: no revision is written). Raises `NotUniqueError` when `field` is
    /// not declared unique (or the type has no schema) and `SchemaError` when the schema refuses
    /// the document or it has no value in `field`.
    fn upsert<'py>(
        &self,
        py: Python<'py>,
        unit_type: &str,
        field: &str,
        document: Bound<'py, PyDict>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let doc = to_document(&document)?;
        let done = self.with_db(py, |db| db.upsert_document(unit_type, field, &doc).map_err(engine_error))?;
        upserted_dict(py, &done)
    }

    /// Replaces the payload of `id` with a document.
    fn update_document(&self, py: Python<'_>, id: &str, document: Bound<'_, PyDict>) -> PyResult<()> {
        let id = UnitId(parse_id(id)?);
        let doc = to_document(&document)?;
        self.with_db(py, |db| db.update_document(id, &doc).map_err(engine_error))
    }

    /// The payload of `id` decoded as a document dict, or `None` for an unknown or deleted unit.
    /// A payload that is not a document raises `SchemaError`.
    fn read_document<'py>(&self, py: Python<'py>, id: &str) -> PyResult<Option<Bound<'py, PyDict>>> {
        let id = UnitId(parse_id(id)?);
        let unit = self.with_db(py, |db| db.get(id).map_err(engine_error))?;
        let Some(payload) = unit.and_then(|u| u.payload) else { return Ok(None) };
        let doc = Document::decode(&payload)
            .map_err(|e| SchemaError::new_err(format!("the payload is not a document: {e}")))?;
        let d = PyDict::new(py);
        for (k, v) in doc.iter() {
            d.set_item(k, field_to_py(py, v)?)?;
        }
        Ok(Some(d))
    }

    /// The active units of `unit_type` whose indexed `field` equals `value`. Raises
    /// `NotIndexedError` when `field` is not indexed and `InvalidArgumentError` for a text or
    /// bytes `value` over 254 bytes (no unit can hold it; the answer is not an empty list).
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

    /// `find`, but each hit as its document dict: no `Unit` objects, no per-unit `document()`
    /// call. A hit whose payload is not a document raises `SchemaError`. `fields`, a sequence of
    /// field names, keeps only those in each dict (a name the document lacks is absent); as a
    /// mapping `{field_name: key_in_result}` it also renames them, each dict built with the
    /// mapped keys in the mapping's order.
    #[pyo3(signature = (unit_type, field, value, fields=None))]
    fn find_documents<'py>(
        &self,
        py: Python<'py>,
        unit_type: &str,
        field: &str,
        value: Bound<'py, PyAny>,
        #[pyo3(from_py_with = fields_arg)] fields: Option<FieldsArg>,
    ) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let v = to_field(&value)?;
        let hits = self.with_db(py, |db| db.find_payloads(unit_type, field, &v).map_err(engine_error))?;
        let mut out = Vec::with_capacity(hits.len());
        let mut docs = DocBuilder::new(fields);
        for (_, payload) in hits {
            if let Some(d) = docs.build(py, Some(&payload))? {
                out.push(d);
            }
        }
        Ok(out)
    }

    /// `find(unit_type, field, value)` then `neighbors(unit, direction, type_prefix=...)` of every
    /// unit found then `.document()` of each other end, in one call:
    /// `[(relation_type, document), ...]`. An other end that is deleted or whose payload is not a
    /// document is left out. `fields` keeps only those field names in each dict, or renames them
    /// when it is a mapping `{field_name: key_in_result}`, as in `find_documents`.
    #[pyo3(signature = (unit_type, field, value, direction="out", type_prefix=None, fields=None))]
    #[allow(clippy::too_many_arguments)] // the Python signature's keywords
    fn neighbor_documents<'py>(
        &self,
        py: Python<'py>,
        unit_type: &str,
        field: &str,
        value: Bound<'py, PyAny>,
        direction: &str,
        type_prefix: Option<&str>,
        #[pyo3(from_py_with = fields_arg)] fields: Option<FieldsArg>,
    ) -> PyResult<Vec<(String, Bound<'py, PyDict>)>> {
        let v = to_field(&value)?;
        let dir = match direction {
            "out" => Direction::Out,
            "in" => Direction::In,
            other => {
                return Err(InvalidArgumentError::new_err(format!("direction must be 'out' or 'in', not {other:?}")));
            }
        };
        let pairs = self.with_db(py, |db| {
            let mut out = Vec::new();
            for unit in db.find_shared(unit_type, field, &v).map_err(engine_error)? {
                for r in db.related(unit.id, dir, None).map_err(engine_error)? {
                    if type_prefix.is_some_and(|p| !r.relation_type.starts_with(p)) {
                        continue;
                    }
                    let other = if matches!(dir, Direction::Out) { r.to } else { r.from };
                    let Some(u) = db.get_shared(other).map_err(engine_error)? else { continue };
                    out.push((r.relation_type, u));
                }
            }
            Ok(out)
        })?;
        let mut out = Vec::with_capacity(pairs.len());
        let mut docs = DocBuilder::new(fields);
        for (t, u) in pairs {
            if let Some(d) = docs.build(py, u.payload.as_deref())? {
                out.push((t, d));
            }
        }
        Ok(out)
    }

    /// The active units of `unit_type` whose indexed `field` lies between `low` and `high`, both
    /// inclusive; `None` leaves that end open. Ordered by value, then id. `limit` caps the
    /// result (default: no cap; 0 is an `InvalidArgumentError`). Floats order as numbers, text and
    /// bytes as bytes. A range no value can be in (`low` above `high`) raises
    /// `InvalidArgumentError` instead of returning `[]`.
    #[pyo3(signature = (unit_type, field, low=None, high=None, limit=None))]
    fn find_range(
        &self,
        py: Python<'_>,
        unit_type: &str,
        field: &str,
        low: Option<Bound<'_, PyAny>>,
        high: Option<Bound<'_, PyAny>>,
        #[pyo3(from_py_with = max_rows_arg)] limit: Option<usize>,
    ) -> PyResult<Vec<Unit>> {
        let end = |v: &Option<Bound<'_, PyAny>>| -> PyResult<Range<FieldValue>> {
            Ok(match v {
                Some(v) => Range::Included(to_field(v)?),
                None => Range::Unbounded,
            })
        };
        let (lo, hi) = (end(&low)?, end(&high)?);
        self.with_db(py, |db| {
            let units = db
                .find_range(unit_type, field, lo.as_ref(), hi.as_ref(), limit.unwrap_or(usize::MAX))
                .map_err(engine_error)?;
            Ok(units.into_iter().map(Unit::from).collect())
        })
    }

    /// How `execute` would answer `query`, without running it: `{"root_alias", "root_type",
    /// "candidates": [{"access": "type_scan" | "field_index", "field", "cost"}], "chosen" (an
    /// index into `candidates`), "joins": [...], "planner_version"}`.
    fn explain<'py>(&self, py: Python<'py>, query: &str) -> PyResult<Bound<'py, PyDict>> {
        let plan: QueryPlan = self.with_db(py, |db| db.explain(query).map_err(engine_error))?;
        let d = PyDict::new(py);
        d.set_item("root_alias", plan.root_alias)?;
        d.set_item("root_type", plan.root_type)?;
        let candidates = PyList::empty(py);
        for c in plan.candidates {
            let item = PyDict::new(py);
            match c.access {
                Access::TypeScan => {
                    item.set_item("access", "type_scan")?;
                    item.set_item("field", py.None())?;
                }
                Access::FieldIndex { field } => {
                    item.set_item("access", "field_index")?;
                    item.set_item("field", field)?;
                }
            }
            item.set_item("cost", c.cost.0)?;
            candidates.append(item)?;
        }
        d.set_item("candidates", candidates)?;
        d.set_item("chosen", plan.chosen)?;
        let joins = PyList::empty(py);
        for j in plan.joins {
            let item = PyDict::new(py);
            item.set_item("alias", j.alias)?;
            item.set_item("unit_type", j.unit_type)?;
            item.set_item("relation_type", j.relation_type)?;
            item.set_item("direction", if j.direction == Direction::Out { "out" } else { "in" })?;
            item.set_item("from", j.from)?;
            joins.append(item)?;
        }
        d.set_item("joins", joins)?;
        d.set_item("planner_version", plan.planner_version)?;
        Ok(d)
    }

    /// Fuses up to three signals by weighted reciprocal rank fusion, best first:
    /// - keyword (BM25): `text`, over the types enabled with `set_fulltext`;
    /// - vector: `vector` (the query) looked up in `vectors`, a `{unit id: vector}` dict you keep
    ///   in step with your units (an exact, in-memory search; `metric` is "cosine" or "dot");
    /// - relation: units reached from the `seeds` over at most `max_depth` hops.
    ///
    /// A signal runs when its input is given and its weight is above 0; at least one must run.
    /// A keyword signal with no type switched on by `set_fulltext` raises `NotIndexedError`
    /// (a switched-on type with no match gives an empty list). `text_types` restricts the keyword
    /// signal to those unit types, with the errors of `search_text`; it is an error without
    /// `text`. The other signals are not scoped to a type.
    /// Each signal adds `weight / (rrf_k + rank)` to a unit's score. Only active units are hits.
    #[pyo3(signature = (
        text=None, vector=None, vectors=None, seeds=None, *, text_types=None,
        keyword_weight=1, vector_weight=1, relation_weight=1,
        limit=10, candidates=100, rrf_k=60, max_depth=2, metric="cosine"
    ))]
    #[allow(clippy::too_many_arguments)]
    fn hybrid_search(
        &self,
        py: Python<'_>,
        text: Option<&str>,
        vector: Option<Vec<f32>>,
        vectors: Option<HashMap<String, Vec<f32>>>,
        #[pyo3(from_py_with = seeds_arg)] seeds: Option<Vec<String>>,
        #[pyo3(from_py_with = text_types_arg)] text_types: Option<Vec<String>>,
        #[pyo3(from_py_with = keyword_weight_arg)] keyword_weight: u32,
        #[pyo3(from_py_with = vector_weight_arg)] vector_weight: u32,
        #[pyo3(from_py_with = relation_weight_arg)] relation_weight: u32,
        #[pyo3(from_py_with = limit_arg)] limit: usize,
        #[pyo3(from_py_with = candidates_arg)] candidates: usize,
        #[pyo3(from_py_with = rrf_k_arg)] rrf_k: u32,
        #[pyo3(from_py_with = max_depth_arg)] max_depth: u32,
        metric: &str,
    ) -> PyResult<Vec<SearchHit>> {
        let metric = match metric {
            "cosine" => VectorMetric::Cosine,
            "dot" => VectorMetric::Dot,
            other => {
                return Err(InvalidArgumentError::new_err(format!("metric must be 'cosine' or 'dot', not {other:?}")));
            }
        };
        let set = match (&vector, vectors) {
            (Some(q), Some(vs)) => {
                let bad = |e: kerno::units::assoc_rank::VectorError| InvalidArgumentError::new_err(e.to_string());
                let mut set = VectorSet::new(q.len(), metric).map_err(bad)?;
                for (id, v) in vs {
                    set.insert(UnitId(parse_id(&id)?), v).map_err(bad)?;
                }
                Some(set)
            }
            (None, None) => None,
            _ => return Err(InvalidArgumentError::new_err("vector and vectors must be given together")),
        };
        let graph = seeds
            .map(|s| {
                let ids = s.iter().map(|id| parse_id(id).map(UnitId)).collect::<PyResult<Vec<_>>>()?;
                Ok::<_, PyErr>(AssocQuery::new(ids, max_depth, candidates, kerno::units::assoc::MAX_ASSOC_VISITS))
            })
            .transpose()?;
        let weights = HybridWeights { keyword: keyword_weight, vector: vector_weight, graph: relation_weight };
        self.with_db(py, |db| {
            let mut q = HybridQuery::new(limit);
            q.text = text;
            let names: Option<Vec<&str>> = text_types.as_ref().map(|t| t.iter().map(String::as_str).collect());
            q.text_types = names.as_deref();
            q.vector = set.as_ref().zip(vector.as_deref()).map(|(index, query)| VectorLeg { index, query });
            q.graph = graph;
            q.weights = weights;
            q.rrf_k = rrf_k;
            q.candidates = candidates;
            let share = |weight: u32, rank: Option<u32>| {
                rank.map(|r| {
                    (u64::from(weight) * RRF_SCALE / (u64::from(rrf_k) + u64::from(r))) as f64 / RRF_SCALE as f64
                })
            };
            Ok(db
                .hybrid_search(&q)
                .map_err(engine_error)?
                .into_iter()
                .map(|h| SearchHit {
                    id: h.unit.to_string(),
                    score: h.score as f64 / RRF_SCALE as f64,
                    keyword_score: share(keyword_weight, h.ranks.keyword),
                    vector_score: share(vector_weight, h.ranks.vector),
                    relation_score: share(relation_weight, h.ranks.graph),
                    keyword_rank: h.ranks.keyword,
                    vector_rank: h.ranks.vector,
                    relation_rank: h.ranks.graph,
                })
                .collect())
        })
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
    /// `("unarchive", id)`, `("restore", id, revision)` or `("upsert", unit_type, field,
    /// document)` (see `upsert`; several upserts of one key in a batch make one unit and the last
    /// document wins, and each op of the key reports that unit's id and one outcome: `created`
    /// when the first made it; an upserted unit is not in the stored ids, so read it from
    /// `upserts`). A relation end is a unit id or the position (0-based, counting every op in
    /// the list) of an earlier "store" or "upsert" op, which names the unit it created, revised
    /// or found unchanged. Any other position (a later op, an update, a relation, past the end)
    /// raises `BadEndpointError` and nothing is written. `reason`, `actor` and `source` are
    /// recorded with every revision. Returns the ids of the stored units, or with
    /// `detailed=True` a dict: `units`, `relations`, `unchanged`, `upserts` (one `{"id",
    /// "outcome"}` per upsert op), `commit_seq`.
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
        let mut ids = IdCache::new();
        // Op names as Python interns them: a literal name in the caller's code is this very
        // object, so the name is matched by identity without reading it.
        // The most frequent first: they are tried in this order.
        const NAMES: [&str; 12] = [
            "relate",
            "store",
            "upsert",
            "update",
            "update_if_changed",
            "delete",
            "delete_cascade",
            "unrelate",
            "relate_unique",
            "archive",
            "unarchive",
            "restore",
        ];
        let interned: Vec<Bound<'py, PyString>> = NAMES.iter().map(|n| PyString::intern(py, n)).collect();
        // Callers pass the same `str` object for a relation's type, and for its `from` end,
        // op after op: the last one seen and what it converted to. The cache holds a strong
        // reference to the object, so its address cannot be reused by another while cached.
        let mut last_type: Option<(Bound<'py, PyAny>, String)> = None;
        let mut last_from: Option<(Bound<'py, PyAny>, Endpoint)> = None;
        // The same for the unit type of "store" and "upsert" ops and the key field of "upsert".
        let mut last_unit_type: Option<(Bound<'py, PyAny>, String)> = None;
        let mut last_field: Option<(Bound<'py, PyAny>, String)> = None;
        for op in &ops {
            if op.is_empty() {
                return Err(InvalidArgumentError::new_err("unknown or malformed batch op: ()"));
            }
            let first = op.get_borrowed_item(0)?;
            let owned_name: String;
            let name: &str = match interned.iter().position(|n| n.as_ptr() == first.as_ptr()) {
                Some(at) => NAMES[at],
                None => {
                    owned_name = first.extract()?;
                    &owned_name
                }
            };
            match (name, op.len()) {
                ("store", 3) => {
                    let t = cached_str(&mut last_unit_type, op.get_borrowed_item(1)?)?;
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.store(t, into_bytes(p));
                }
                ("update", 3) => {
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.update(UnitId(ids.parse(&op.get_item(1)?)?), into_bytes(p));
                }
                ("update_if_changed", 3) => {
                    let p: Payload = op.get_item(2)?.extract()?;
                    batch.update_if_changed(UnitId(ids.parse(&op.get_item(1)?)?), into_bytes(p));
                }
                ("delete", 2) => {
                    batch.delete(UnitId(ids.parse(&op.get_item(1)?)?));
                }
                ("delete_cascade", 2) => {
                    batch.delete_cascade(UnitId(ids.parse(&op.get_item(1)?)?));
                }
                ("unrelate", 2) => {
                    let id: String = op.get_item(1)?.extract()?;
                    batch.unrelate(RelationId(parse_id(&id)?));
                }
                ("relate", 4) | ("relate_unique", 4) => {
                    let from = op.get_borrowed_item(1)?;
                    let f = match &last_from {
                        Some((o, e)) if o.as_ptr() == from.as_ptr() => *e,
                        _ => {
                            let e = ids.endpoint(&from)?;
                            last_from = Some((from.to_owned(), e));
                            e
                        }
                    };
                    let t = ids.endpoint(&op.get_item(2)?)?;
                    let ty_obj = op.get_borrowed_item(3)?;
                    let ty = cached_str(&mut last_type, ty_obj)?;
                    if name == "relate" {
                        batch.relate(f, t, ty);
                    } else {
                        batch.relate_unique(f, t, ty);
                    }
                }
                ("archive", 2) => {
                    batch.archive(UnitId(ids.parse(&op.get_item(1)?)?));
                }
                ("unarchive", 2) => {
                    batch.unarchive(UnitId(ids.parse(&op.get_item(1)?)?));
                }
                ("restore", 3) => {
                    let rev: u64 = unsigned("a restore op's revision", &op.get_item(2)?)?;
                    batch.restore(UnitId(ids.parse(&op.get_item(1)?)?), rev);
                }
                ("upsert", 4) => {
                    let t = cached_str(&mut last_unit_type, op.get_borrowed_item(1)?)?;
                    let field = cached_str(&mut last_field, op.get_borrowed_item(2)?)?;
                    let doc = to_document(op.get_item(3)?.cast::<PyDict>().map_err(|_| {
                        InvalidArgumentError::new_err("an upsert op's document is a dict of bool/int/float/str/bytes")
                    })?)?;
                    batch.upsert(t, field, doc.encode().map_err(engine_error)?);
                }
                _ => return Err(InvalidArgumentError::new_err(format!("unknown or malformed batch op: {op}"))),
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
        let upserts = done.upserts.iter().map(|u| upserted_dict(py, u)).collect::<PyResult<Vec<_>>>()?;
        d.set_item("upserts", upserts)?;
        d.set_item("commit_seq", done.commit_seq)?;
        Ok(d.into_any())
    }

    /// Up to `limit` revisions of `id`, newest first (`order="desc"`) or oldest first
    /// (`order="asc"`), among those in `[from_revision, to_revision)`; a missing bound is the
    /// oldest kept or the newest. To page, pass the last revision returned as the next
    /// `to_revision` (desc) or its successor as the next `from_revision` (asc). A
    /// `from_revision` below `Unit.oldest_revision` raises `PrunedError`, and an id no unit ever had
    /// raises `NotFoundError` (not `[]`); `from_revision` above `to_revision` raises
    /// `InvalidArgumentError` (equal bounds are an empty page). `with_payload` reads
    /// and verifies each revision's content; for a deleted unit whose payloads `compact()` dropped,
    /// the revisions are listed with `payload` `None` instead of raising.
    #[pyo3(signature = (id, limit=100, with_payload=false, from_revision=None, to_revision=None, order="desc"))]
    #[allow(clippy::too_many_arguments)]
    fn history(
        &self,
        py: Python<'_>,
        id: &str,
        #[pyo3(from_py_with = limit_arg)] limit: usize,
        with_payload: bool,
        #[pyo3(from_py_with = from_revision_arg)] from_revision: Option<u64>,
        #[pyo3(from_py_with = to_revision_arg)] to_revision: Option<u64>,
        order: &str,
    ) -> PyResult<Vec<Revision>> {
        let id = UnitId(parse_id(id)?);
        let order = match order {
            "desc" => Order::Desc,
            "asc" => Order::Asc,
            other => {
                return Err(InvalidArgumentError::new_err(format!("order must be 'desc' or 'asc', not {other:?}")));
            }
        };
        let query = |with_payload| HistoryQuery {
            from: from_revision.map(RevNo),
            to: to_revision.map(RevNo),
            order,
            limit,
            with_payload,
        };
        self.with_db(py, |db| {
            let revisions = match db.unit_db().history(id, query(with_payload)) {
                // `compact()` dropped the content of a deleted unit's revisions: list them
                // without payloads rather than fail the whole call.
                Err(kerno::Error::HistoryUnavailable { .. }) if with_payload => db.unit_db().history(id, query(false)),
                other => other,
            };
            Ok(revisions.map_err(engine_error)?.into_iter().map(Revision::from).collect())
        })
    }

    /// Revisions `a` and `b` of unit `id`, with their verified payloads, and whether their
    /// content is the same. Raises `NotFoundError` for an unknown unit or revision and
    /// `PrunedError` for a revision retention removed, and `HistoryUnavailableError` for a deleted
    /// unit after `compact()`.
    fn compare(
        &self,
        py: Python<'_>,
        id: &str,
        #[pyo3(from_py_with = revision_a_arg)] a: u64,
        #[pyo3(from_py_with = revision_b_arg)] b: u64,
    ) -> PyResult<Comparison> {
        let id = UnitId(parse_id(id)?);
        let c = self.with_db(py, |db| db.unit_db().compare(id, RevNo(a), RevNo(b)).map_err(engine_error))?;
        Ok(Comparison {
            a: Py::new(py, Revision::from(c.a))?,
            b: Py::new(py, Revision::from(c.b))?,
            same_content: c.same_content,
        })
    }

    /// The revision of `id` in force once commit `commit_seq` committed, with its payload;
    /// `None` when the unit did not exist yet (or never did). A deleted unit returns its
    /// tombstone. A sequence after the last commit reads the current state. Raises `PrunedError`
    /// below the oldest kept revision and `HistoryUnavailableError` for a deleted unit after
    /// `compact()`.
    fn read_as_of(
        &self,
        py: Python<'_>,
        id: &str,
        #[pyo3(from_py_with = commit_seq_arg)] commit_seq: u64,
    ) -> PyResult<Option<Revision>> {
        let id = UnitId(parse_id(id)?);
        self.with_db(py, |db| match db.read_as_of(id, commit_seq).map_err(engine_error)? {
            UnitAt::Absent => Ok(None),
            UnitAt::Present(r) | UnitAt::DeletedAt(r) => Ok(Some(Revision::from(r))),
            UnitAt::Pruned { oldest } => Err(PrunedError::new_err(format!(
                "commit {commit_seq} predates the oldest kept revision (revision {}): retention pruned what it \
                     read. `read_as_of` takes a commit sequence; `history` and `compare` take revision numbers",
                oldest.0
            ))),
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
/// `read_only=True` writes nothing to the directory and takes no lock, so it opens beside a live
/// writer (one read-write handle at a time holds the lock; a second raises `LockedError`). A
/// read-only handle reads the commits made up to the moment it opened; call `refresh()` to see
/// later ones. It raises `ReadOnlyError` on a write and `DatabaseNotFoundError` when the
/// directory holds no database (it never creates one). `max_batch_ops` raises (or lowers) the
/// most operations one `commit` may hold (default 10,000). `verify_payload_on_read=True` makes
/// every read (get, find, query, neighbors, search hits) also check each payload against its
/// content hash; off by default, since the bytes come from blocks whose checksum is always
/// checked. `verify()`, history reads, export and import always check it. `durability="sync"`
/// (the default) returns from a commit only once it is on stable storage; `"buffered"` does not
/// wait for it (no `fsync` per commit, like SQLite's `synchronous=NORMAL` in WAL mode): after a
/// power cut the last commits may be missing, never torn, and `close()` still makes everything
/// durable. `close_compaction=False` makes `close()` only flush: it skips the compaction a close
/// otherwise does to tidy the newest files, which a bulk load that closes right after its last
/// commit pays in full; the next session that writes compacts in the background as usual.
#[pyfunction]
#[pyo3(signature = (path, read_only=false, max_batch_ops=None, verify_payload_on_read=false, durability="sync", close_compaction=true))]
fn open(
    py: Python<'_>,
    path: PathBuf,
    read_only: bool,
    #[pyo3(from_py_with = max_batch_ops_arg)] max_batch_ops: Option<usize>,
    verify_payload_on_read: bool,
    durability: &str,
    close_compaction: bool,
) -> PyResult<Database> {
    let durability = match durability {
        "sync" => kerno::Durability::Sync,
        "buffered" => kerno::Durability::Buffered,
        other => {
            return Err(InvalidArgumentError::new_err(format!(
                "durability must be \"sync\" or \"buffered\", not {other:?}"
            )))
        }
    };
    let db = py
        .detach(|| {
            let config = if read_only {
                Config {
                    access: AccessMode::ReadOnly,
                    create_if_missing: false,
                    allow_unverified_platform: true,
                    // A reader opens beside a writer and often for one question: load each
                    // table when a read first needs it, not all of them up front.
                    lazy_tables: true,
                    verify_payload_on_read,
                    ..Config::default()
                }
            } else {
                let mut config =
                    Config { allow_unverified_platform: true, verify_payload_on_read, durability, ..Config::default() };
                if !close_compaction {
                    config.close_budget = kerno::storage::CloseBudget::Deferred;
                }
                config
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
    m.add_class::<Comparison>()?;
    m.add_class::<SearchHit>()?;
    m.add_class::<TextHit>()?;
    m.add("KernoError", m.py().get_type::<KernoError>())?;
    m.add("SchemaExistsError", m.py().get_type::<SchemaExistsError>())?;
    m.add("NotIndexedError", m.py().get_type::<NotIndexedError>())?;
    m.add("UnknownTypeError", m.py().get_type::<UnknownTypeError>())?;
    m.add("NotUniqueError", m.py().get_type::<NotUniqueError>())?;
    m.add("BadEndpointError", m.py().get_type::<BadEndpointError>())?;
    m.add("ChangeSetTooLargeError", m.py().get_type::<ChangeSetTooLargeError>())?;
    m.add("ReadOnlyError", m.py().get_type::<ReadOnlyError>())?;
    m.add("DatabaseNotFoundError", m.py().get_type::<DatabaseNotFoundError>())?;
    m.add("NotFoundError", m.py().get_type::<NotFoundError>())?;
    m.add("DeletedError", m.py().get_type::<DeletedError>())?;
    m.add("LockedError", m.py().get_type::<LockedError>())?;
    m.add("CorruptionError", m.py().get_type::<CorruptionError>())?;
    let invalid = m.py().get_type::<InvalidArgumentError>();
    let closed = m.py().get_type::<ClosedError>();
    // Both contracts hold: `except KernoError` and `except ValueError` each catch these.
    let bases = (m.py().get_type::<KernoError>(), m.py().get_type::<PyValueError>());
    invalid.setattr("__bases__", bases.clone())?;
    closed.setattr("__bases__", bases)?;
    m.add("InvalidArgumentError", invalid)?;
    m.add("ClosedError", closed)?;
    m.add("HistoryUnavailableError", m.py().get_type::<HistoryUnavailableError>())?;
    m.add("PayloadTooLargeError", m.py().get_type::<PayloadTooLargeError>())?;
    m.add("HasRelationsError", m.py().get_type::<HasRelationsError>())?;
    m.add("PrunedError", m.py().get_type::<PrunedError>())?;
    m.add("SchemaError", m.py().get_type::<SchemaError>())?;
    Ok(())
}

#[cfg(test)]
mod hex_tests {
    use super::encode_hex;

    fn old_encoding(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn empty_input_encodes_to_an_empty_string() {
        assert_eq!(encode_hex(&[]), "");
    }

    #[test]
    fn every_byte_value_matches_the_old_format_output() {
        for b in 0..=255u8 {
            assert_eq!(encode_hex(&[b]), format!("{b:02x}"), "byte {b}");
        }
        let all: Vec<u8> = (0..=255u8).collect();
        let hex = encode_hex(&all);
        assert_eq!(hex.len(), 512);
        assert_eq!(hex, old_encoding(&all));
    }

    #[test]
    fn a_32_byte_digest_matches_the_old_format_output() {
        let digest: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
        assert_eq!(encode_hex(&digest), old_encoding(&digest));
        assert_eq!(encode_hex(&[0x00, 0x0a, 0xf0, 0xff]), "000af0ff");
    }
}
