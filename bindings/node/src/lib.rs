//! Node.js bindings for Kerno: a thin napi-rs layer over the [`kerno`] facade crate.
//!
//! Ids cross the boundary as 32-digit lower-case hex strings, payloads as `Buffer` (a `string`
//! is accepted on the way in and stored as UTF-8). Every method is synchronous, like an
//! embedded database driver: a call returns when the commit is done.
//!
//! Every failure is a JS `Error` whose `code` is one of the stable strings of `ErrorCode`.
//! Sequences, revision numbers and sizes cross as JS numbers (`i64`), exact up to 2^53.
// ponytail: sync calls block the event loop for the length of a commit; move to AsyncTask
// if a caller needs to keep the loop free during writes.

use std::collections::{BTreeSet, HashMap};
use std::sync::{PoisonError, RwLock};

use kerno::units::assoc_rank::{VectorMetric, VectorSet};
use kerno::units::hybrid::Fusion;
use kerno::units::{HistoryQuery, RevNo, RevisionKind, UnitAt};
use kerno::{
    AccessMode, AssocDirection, AssocQuery, Batch, CommitMeta, Config, Direction, Document, Endpoint, FieldDef,
    FieldType, FieldValue, HybridQuery, HybridWeights, Lifecycle, RelationId, RetentionPolicy, RetentionRule as Rule,
    UnitId, VectorLeg,
};
use napi::JsValue;
use napi::bindgen_prelude::{
    BigInt, Buffer, Either, Either5, FromNapiValue, Null, ToNapiValue, TypeName, Unknown, ValidateNapiValue,
};
use napi_derive::napi;

/// The stable `code` of every error this package throws (listed in the README).
enum ErrorCode {
    /// No unit, relation or revision with that id (`NotFoundError` in Python).
    NotFound,
    /// The unit is deleted, or not in the lifecycle the call needs.
    Deleted,
    /// Another handle or process holds the database directory.
    Locked,
    /// Stored bytes failed validation.
    Corruption,
    /// An argument was refused: a bad id, a type name, a limit, a value no document can hold.
    InvalidArgument,
    /// The payload is larger than 1 MiB.
    PayloadTooLarge,
    /// One commit holds more operations (10,000 by default, see `maxBatchOps`) or bytes than it may.
    ChangesetTooLarge,
    /// `defineSchema` for a type that already has a different schema (the same one is accepted).
    SchemaExists,
    /// The call needs an index that is not there: a field not declared indexed, a type with no
    /// schema, or keyword search with no type switched on by `setFulltext`.
    NotIndexed,
    /// `upsert` by a field the schema does not declare unique (or by a type with no schema).
    NotUnique,
    /// A batch `relate` end that is not an earlier `store` or `upsert` op of the batch.
    BadEndpoint,
    /// A write went through a handle opened with `readOnly: true`.
    ReadOnly,
    /// The directory holds no database and `readOnly: true` never creates one.
    DatabaseNotFound,
    /// The unit still has live relations; unrelate them or use `cascade`.
    HasRelations,
    /// Retention removed the revision asked for.
    Pruned,
    /// The type's schema refused the document.
    Schema,
    /// The database handle is closed.
    Closed,
    /// Any other engine failure.
    Kerno,
}

impl ErrorCode {
    fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Deleted => "DELETED",
            ErrorCode::Locked => "LOCKED",
            ErrorCode::Corruption => "CORRUPTION",
            ErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ErrorCode::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
            ErrorCode::ChangesetTooLarge => "CHANGESET_TOO_LARGE",
            ErrorCode::SchemaExists => "SCHEMA_EXISTS",
            ErrorCode::NotIndexed => "NOT_INDEXED",
            ErrorCode::NotUnique => "NOT_UNIQUE",
            ErrorCode::BadEndpoint => "BAD_ENDPOINT",
            ErrorCode::ReadOnly => "READ_ONLY",
            ErrorCode::DatabaseNotFound => "DATABASE_NOT_FOUND",
            ErrorCode::HasRelations => "HAS_RELATIONS",
            ErrorCode::Pruned => "PRUNED",
            ErrorCode::Schema => "SCHEMA",
            ErrorCode::Closed => "CLOSED",
            ErrorCode::Kerno => "KERNO",
        }
    }
}

type JsError = napi::Error<String>;
type Result<T> = std::result::Result<T, JsError>;

fn coded(code: ErrorCode, msg: impl std::fmt::Display) -> JsError {
    napi::Error::new(code.as_str().to_owned(), msg.to_string())
}

fn invalid(msg: impl std::fmt::Display) -> JsError {
    coded(ErrorCode::InvalidArgument, msg)
}

/// The same mapping as the Python binding's `engine_error`.
fn engine_error(e: kerno::Error) -> JsError {
    use kerno::Error as E;
    let msg = e.to_string();
    let code = if e.is_locked() {
        ErrorCode::Locked
    } else if e.is_corruption() {
        ErrorCode::Corruption
    } else if e.is_read_only() {
        ErrorCode::ReadOnly
    } else if e.is_no_database() {
        ErrorCode::DatabaseNotFound
    } else if e.is_engine_argument() {
        ErrorCode::InvalidArgument
    } else {
        match e {
            E::NotFound { .. } | E::RelationNotFound { .. } | E::RevisionNotFound { .. } => ErrorCode::NotFound,
            E::Deleted { .. } => ErrorCode::Deleted,
            E::PayloadTooLarge { .. } => ErrorCode::PayloadTooLarge,
            E::InvalidArgument(_) => ErrorCode::InvalidArgument,
            E::ChangeSetTooLarge { .. } => ErrorCode::ChangesetTooLarge,
            E::SchemaExists { .. } => ErrorCode::SchemaExists,
            E::NotIndexed { .. } => ErrorCode::NotIndexed,
            E::NotUnique { .. } => ErrorCode::NotUnique,
            E::BadEndpoint { .. } => ErrorCode::BadEndpoint,
            E::UnitHasRelations { .. } => ErrorCode::HasRelations,
            E::Pruned { .. } => ErrorCode::Pruned,
            E::SchemaViolation { .. } => ErrorCode::Schema,
            _ => ErrorCode::Kerno,
        }
    };
    coded(code, msg)
}

/// A counter, sequence or size as a JS number.
// ponytail: exact up to 2^53, far past any real sequence; switch to BigInt if one ever gets there.
fn num(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// 32 hex digits to the 16 bytes of an id.
fn parse_id(hex: &str) -> Result<[u8; 16]> {
    let bad = || invalid(format!("invalid id {hex:?}: expected 32 hex digits"));
    if hex.len() != 32 || !hex.is_ascii() {
        return Err(bad());
    }
    let mut out = [0u8; 16];
    for (byte, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| bad())?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| bad())?;
    }
    Ok(out)
}

fn unit_id(hex: &str) -> Result<UnitId> {
    parse_id(hex).map(UnitId)
}

fn relation_id(hex: &str) -> Result<RelationId> {
    parse_id(hex).map(RelationId)
}

/// What the write calls accept as content: text is stored as its UTF-8 bytes.
type Payload = Either<String, Buffer>;

fn into_bytes(p: Payload) -> Vec<u8> {
    match p {
        Either::A(text) => text.into_bytes(),
        Either::B(bytes) => bytes.to_vec(),
    }
}

fn lifecycle_name(l: Lifecycle) -> String {
    match l {
        Lifecycle::Active => "active",
        Lifecycle::Archived => "archived",
        Lifecycle::Deleted => "deleted",
    }
    .to_owned()
}

/// A unit as of its latest revision.
#[napi(object)]
pub struct Unit {
    /// 32 hex digits.
    pub id: String,
    pub unit_type: String,
    pub schema_version: u32,
    /// `read`, `query` and `find` return `active` and `archived` units as the engine does;
    /// a deleted unit has no payload.
    #[napi(ts_type = "'active' | 'archived' | 'deleted'")]
    pub lifecycle: String,
    /// The number of the unit's latest revision.
    pub revision: i64,
    /// Microseconds since the Unix epoch; descriptive, not an ordering guarantee.
    pub created_at_us: i64,
    pub updated_at_us: i64,
    /// The content; absent for a deleted unit.
    pub payload: Option<Buffer>,
}

impl From<kerno::Unit> for Unit {
    fn from(u: kerno::Unit) -> Self {
        Unit {
            id: u.id.to_string(),
            unit_type: u.unit_type,
            schema_version: u.schema_version,
            lifecycle: lifecycle_name(u.lifecycle),
            revision: num(u.head.0),
            created_at_us: u.created_at.0,
            updated_at_us: u.updated_at.0,
            payload: u.payload.map(Buffer::from),
        }
    }
}

/// A directed, typed edge between two units.
#[napi(object)]
pub struct Relation {
    /// 32 hex digits.
    pub id: String,
    /// Id of the unit it starts at.
    pub from: String,
    /// Id of the unit it ends at.
    pub to: String,
    pub relation_type: String,
}

impl From<kerno::Relation> for Relation {
    fn from(r: kerno::Relation) -> Self {
        Relation {
            id: r.id.to_string(),
            from: r.from.to_string(),
            to: r.to.to_string(),
            relation_type: r.relation_type,
        }
    }
}

/// One revision of a unit, from `history` or `readAsOf`.
#[napi(object)]
pub struct Revision {
    pub revision: i64,
    #[napi(ts_type = "'create' | 'update' | 'archive' | 'unarchive' | 'delete' | 'restore'")]
    pub kind: String,
    /// The commit that wrote it.
    pub commit_seq: i64,
    /// Microseconds since the Unix epoch; descriptive, never used for ordering.
    pub created_at_us: i64,
    #[napi(ts_type = "'active' | 'archived' | 'deleted'")]
    pub lifecycle: String,
    pub schema_version: u32,
    /// Hex digest of the verified content.
    pub content_hash: String,
    /// The verified content: present when it was asked for (`readAsOf` always asks) and the
    /// revision has one.
    pub payload: Option<Buffer>,
}

impl From<kerno::units::Revision> for Revision {
    fn from(r: kerno::units::Revision) -> Self {
        Revision {
            revision: num(r.id.rev.0),
            kind: match r.kind {
                RevisionKind::Create => "create",
                RevisionKind::Update => "update",
                RevisionKind::Archive => "archive",
                RevisionKind::Unarchive => "unarchive",
                RevisionKind::Delete => "delete",
                RevisionKind::Restore => "restore",
            }
            .to_owned(),
            commit_seq: num(r.commit_seq.0),
            created_at_us: r.created_at.0,
            lifecycle: lifecycle_name(r.lifecycle),
            schema_version: r.schema_version,
            content_hash: r.content_hash.digest.iter().map(|b| format!("{b:02x}")).collect(),
            payload: r.payload.map(Buffer::from),
        }
    }
}

/// Which units `query` returns. A field left out matches everything.
#[napi(object)]
pub struct UnitFilter {
    pub unit_type: Option<String>,
    pub schema_version: Option<f64>,
}

#[napi(object)]
pub struct OpenOptions {
    /// Write nothing and take no lock: it opens beside a live writer and reads the commits made up
    /// to the moment it opened (`refresh()` shows later ones). A write throws `READ_ONLY`, and a
    /// directory with no database throws `DATABASE_NOT_FOUND`. Default `false`.
    pub read_only: Option<bool>,
    /// The most operations one `commit` may hold. Default 10,000.
    pub max_batch_ops: Option<f64>,
}

#[napi(object)]
pub struct UpdateOptions {
    /// An identical payload writes nothing. Default `false`.
    pub if_changed: Option<bool>,
}

#[napi(object)]
pub struct DeleteOptions {
    /// Remove the unit's relations first, in the same commit. Default `false`.
    pub cascade: Option<bool>,
}

#[napi(object)]
pub struct RelateOptions {
    /// Reuse an existing relation of that type between the two units. Default `false`.
    pub unique: Option<bool>,
}

#[napi(object)]
pub struct HistoryOptions {
    /// The newest `limit` revisions (at least 1). Default 100.
    pub limit: Option<f64>,
    /// Read and verify each revision's content. Default `false`.
    pub with_payload: Option<bool>,
    /// The oldest revision to list, inclusive. Default: the oldest kept. Below it throws `PRUNED`.
    pub from_revision: Option<f64>,
}

#[napi(object)]
pub struct CommitOptions {
    /// At most 1024 bytes.
    pub reason: Option<String>,
    /// At most 256 bytes.
    pub actor: Option<Either<String, Buffer>>,
    /// At most 256 bytes.
    pub source: Option<Either<String, Buffer>>,
}

/// One operation of `commit`. Which fields it needs depends on `op`:
///
/// | `op` | fields |
/// |---|---|
/// | `store` | `unitType`, `payload` |
/// | `update` | `id`, `payload`, optional `ifChanged` |
/// | `delete` | `id`, optional `cascade` |
/// | `delete_cascade` | `id` |
/// | `archive`, `unarchive` | `id` |
/// | `restore` | `id`, `revision` |
/// | `relate` | `from`, `to`, `relationType`, optional `unique` |
/// | `unrelate` | `relationId` |
/// | `upsert` | `unitType`, `field`, `document` |
///
/// A relation end (`from`, `to`) is a unit id, or the 0-based position in the list of an earlier
/// `store` or `upsert` op of the same commit (every op counts, not only the stores). An upsert names
/// the unit it created, revised or found unchanged. Any other position (a later op, an update, a
/// relation, past the end) throws `BAD_ENDPOINT` and nothing is written.
#[napi(object)]
pub struct CommitOp {
    #[napi(
        ts_type = "'store' | 'update' | 'delete' | 'delete_cascade' | 'archive' | 'unarchive' | 'restore' | 'relate' | 'unrelate' | 'upsert'"
    )]
    pub op: String,
    pub unit_type: Option<String>,
    pub payload: Option<Either<String, Buffer>>,
    pub id: Option<String>,
    pub if_changed: Option<bool>,
    pub cascade: Option<bool>,
    /// `restore`: the revision number to copy.
    pub revision: Option<f64>,
    #[napi(ts_type = "string | number")]
    pub from: Option<Either<String, f64>>,
    #[napi(ts_type = "string | number")]
    pub to: Option<Either<String, f64>>,
    pub relation_type: Option<String>,
    pub unique: Option<bool>,
    /// `unrelate`: the relation to remove.
    pub relation_id: Option<String>,
    /// `upsert`: the unique field of `unitType`'s schema that keys the unit.
    pub field: Option<String>,
    /// `upsert`: the document to store, or to revise the keyed unit to.
    #[napi(ts_type = "Record<string, boolean | number | bigint | string | Buffer>")]
    pub document: Option<HashMap<String, Cell>>,
}

/// What an upsert did to the unit its key names.
#[napi(object)]
pub struct Upserted {
    /// 32 hex digits.
    pub id: String,
    /// `created`: no active unit held the key. `updated`: the unit got a new revision.
    /// `unchanged`: it had this document already, nothing was written.
    #[napi(ts_type = "'created' | 'updated' | 'unchanged'")]
    pub outcome: String,
}

impl From<&kerno::Upserted> for Upserted {
    fn from(u: &kerno::Upserted) -> Self {
        let outcome = match u.outcome {
            kerno::UpsertOutcome::Created => "created",
            kerno::UpsertOutcome::Updated => "updated",
            kerno::UpsertOutcome::Unchanged => "unchanged",
        };
        Upserted { id: u.id.to_string(), outcome: outcome.to_owned() }
    }
}

/// What a `commit` wrote.
#[napi(object)]
pub struct Committed {
    /// The ids of the units the commit stored, in the order of its `store` ops.
    pub units: Vec<String>,
    /// The ids of the relations it created (or, for a unique relate, found), in order.
    pub relations: Vec<String>,
    /// Updates and upserts skipped because the payload was already the current one.
    pub unchanged: u32,
    /// What each `upsert` op did, in order. An upserted unit is not in `units`. Several upserts of
    /// one key make one unit: each reports its id and one outcome (`created` when the first made it).
    pub upserts: Vec<Upserted>,
    /// The sequence of the commit; absent when nothing was written.
    pub commit_seq: Option<i64>,
}

/// What `compact` measured, in stored bytes; `null` when the engine does not know. `before` is
/// the size when the call was made (every commit so far counts; the write buffer is written out
/// first), `after` the size once the space is given back.
#[napi(object)]
pub struct CompactReport {
    pub before: Option<i64>,
    pub after: Option<i64>,
}

/// How much history of a unit type retention keeps. Set exactly one field; `null` is the same
/// as leaving it out.
#[napi(object)]
pub struct RetentionRule {
    /// Keep the newest N revisions (a whole number, 1 or more).
    pub keep_last: Option<Either<f64, Null>>,
    /// Keep the revisions younger than this many seconds (0 or more).
    pub keep_for: Option<Either<f64, Null>>,
    /// Keep what answers an as-of read at this commit sequence (a whole number) or later.
    pub keep_since: Option<Either<f64, Null>>,
}

/// What `runRetention` applies: `byType` gives a rule to a unit type and `default` is the rule of
/// every other type. A type with no rule keeps everything; `null` is the same as no rule.
#[napi(object)]
pub struct RetentionOptions {
    pub default: Option<Either<RetentionRule, Null>>,
    // napi does not emit a `type` alias into index.d.ts: the TS type is spelled out.
    #[napi(ts_type = "Record<string, RetentionRule | null> | null")]
    pub by_type: Option<Either<RulesByType, Null>>,
}

/// The rule of each unit type named in `byType`.
type RulesByType = HashMap<String, Either<RetentionRule, Null>>;

/// The value of an optional field that may also be `null`.
fn given<T>(v: Option<Either<T, Null>>) -> Option<T> {
    match v {
        Some(Either::A(t)) => Some(t),
        _ => None,
    }
}

/// What `runRetention` removed.
#[napi(object)]
pub struct RetentionReport {
    pub units_retained: i64,
    pub revisions_pruned: i64,
    pub payloads_dropped: i64,
    pub bytes_logically_freed: i64,
    /// The sequence of the last retention commit; absent when there was nothing to prune.
    pub commit_seq: Option<i64>,
}

impl RetentionRule {
    fn into_rule(self) -> Result<Rule> {
        match (given(self.keep_last), given(self.keep_for), given(self.keep_since)) {
            (Some(n), None, None) => whole("keepLast", n).map(Rule::KeepLast),
            (None, Some(secs), None) => std::time::Duration::try_from_secs_f64(secs)
                .map(Rule::KeepFor)
                .map_err(|_| invalid("keepFor is a number of seconds, 0 or more")),
            (None, None, Some(seq)) => whole("keepSince", seq).map(|s| Rule::KeepSince(kerno::units::CommitSeq(s))),
            _ => Err(invalid("a retention rule has exactly one of keepLast, keepFor, keepSince")),
        }
    }
}

/// A JS number that must be a whole number in `0..=2^53`: a fraction, a negative, NaN or an
/// infinity is refused rather than truncated.
fn whole(what: &str, n: f64) -> Result<u64> {
    if n.fract() == 0.0 && (0.0..=MAX_SAFE).contains(&n) {
        // Exact: a whole number in 0..=2^53 converts without loss.
        Ok(n as u64)
    } else {
        Err(invalid(format!("{what} must be a whole number, 0 or more, not {n}")))
    }
}

/// Like `whole`, for an optional option that the engine takes as a `u32`.
fn whole_u32(what: &str, n: Option<f64>) -> Result<Option<u32>> {
    n.map(|n| whole(what, n).and_then(|w| u32::try_from(w).map_err(|_| invalid(format!("{what} is too large: {n}")))))
        .transpose()
}

/// What `verify` found. A damaged store is reported (`clean: false`), not thrown.
#[napi(object)]
pub struct VerifyReport {
    pub clean: bool,
    /// The snapshot verified. 0 (like the counts) when `storageError` is set.
    pub commit_seq: i64,
    pub units: i64,
    pub revisions: i64,
    pub relations: i64,
    /// At most the first findings, as text.
    pub findings: Vec<String>,
    /// Findings beyond the ones listed.
    pub findings_omitted: i64,
    /// Set when storage-level damage stopped the scan.
    pub storage_error: Option<String>,
}

/// A field of a schema.
#[napi(object)]
pub struct FieldSpec {
    pub name: String,
    #[napi(js_name = "type", ts_type = "'bool' | 'int' | 'float' | 'text' | 'bytes'")]
    pub field_type: String,
    pub required: Option<bool>,
    /// No two active units of the type hold the same value. Implies `indexed`.
    pub unique: Option<bool>,
    pub indexed: Option<bool>,
}

/// A value of a document field. A whole number is an `int` unless the type's schema declares
/// the field `float`; a `bigint` is always an `int`.
pub type Field = Either5<bool, f64, BigInt, String, Buffer>;

/// A value as JS gave it where a document takes one. `None` when it is not a boolean, number,
/// bigint, string or Buffer (`null`, `undefined`, an array, an object): [`cell`] then refuses it
/// with `INVALID_ARGUMENT`, where napi alone would throw its own `InvalidArg` before any of this
/// code runs.
pub struct Cell(Option<Field>);

impl TypeName for Cell {
    fn type_name() -> &'static str {
        "Cell"
    }

    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
    }
}

impl ValidateNapiValue for Cell {
    unsafe fn validate(
        _env: napi::sys::napi_env,
        _value: napi::sys::napi_value,
    ) -> napi::Result<napi::sys::napi_value> {
        Ok(std::ptr::null_mut())
    }
}

impl FromNapiValue for Cell {
    unsafe fn from_napi_value(env: napi::sys::napi_env, value: napi::sys::napi_value) -> napi::Result<Self> {
        // SAFETY: `env` and `value` come from napi for this call, as `from_napi_value` requires.
        Ok(Cell(unsafe { Field::from_napi_value(env, value) }.ok()))
    }
}

impl ToNapiValue for Cell {
    unsafe fn to_napi_value(env: napi::sys::napi_env, value: Self) -> napi::Result<napi::sys::napi_value> {
        // SAFETY: as above; a `Cell` is only ever read from JS, so the `None` arm is never sent back.
        match value.0 {
            Some(field) => unsafe { Field::to_napi_value(env, field) },
            None => unsafe { Null::to_napi_value(env, Null) },
        }
    }
}

/// The value of `name` as a [`Field`], or `INVALID_ARGUMENT` when JS gave something else.
fn cell(name: &str, c: Cell) -> Result<Field> {
    c.0.ok_or_else(|| invalid(format!("field {name:?} is not a boolean, number, bigint, string or Buffer")))
}

/// The result of `execute`: one `Unit` per alias in each row.
#[napi(object)]
pub struct QueryResult {
    pub aliases: Vec<String>,
    pub rows: Vec<Vec<Unit>>,
}

/// The vector leg of `hybridSearch`: the vectors to rank, and the one to rank them against.
#[napi(object)]
pub struct VectorInput {
    /// The vector to look up.
    pub query: Vec<f64>,
    /// The candidates: a unit and its vector, all of the dimension of `query`.
    pub items: Vec<VectorItem>,
    /// Default `cosine`.
    #[napi(ts_type = "'cosine' | 'dot'")]
    pub metric: Option<String>,
}

#[napi(object)]
pub struct VectorItem {
    pub id: String,
    pub vector: Vec<f64>,
}

/// The relations leg of `hybridSearch`: units reached from `seeds` by walking relations.
#[napi(object)]
pub struct RelationsInput {
    /// Active units to start from; they are not hits themselves.
    pub seeds: Vec<String>,
    /// Follow only these relation types. Default: any.
    pub types: Option<Vec<String>>,
    /// Default `both`.
    #[napi(ts_type = "'out' | 'in' | 'both'")]
    pub direction: Option<String>,
    /// Most hops from a seed, 1 to 4. Default 2.
    pub max_depth: Option<f64>,
    /// Most relation entries examined, up to 100,000. Default 10,000.
    pub visit_budget: Option<f64>,
}

/// Leg weights, 0 to 1000; 0 switches a leg off. Default 1 each.
#[napi(object)]
pub struct HybridWeightsInput {
    pub keyword: Option<f64>,
    pub vector: Option<f64>,
    pub relations: Option<f64>,
}

/// One query over keywords (BM25), vectors and relations, fused by weighted reciprocal rank
/// fusion. A leg runs when its input is set and its weight is above 0; at least one must run.
#[napi(object)]
pub struct HybridQueryInput {
    /// The keyword leg. Only types with `setFulltext(type, true)` are searched.
    pub text: Option<String>,
    pub vectors: Option<VectorInput>,
    pub relations: Option<RelationsInput>,
    pub weights: Option<HybridWeightsInput>,
    /// `graph_gated` ranks the units the relations leg found first, nearest hop first.
    /// Default `rrf`.
    #[napi(ts_type = "'rrf' | 'graph_gated'")]
    pub fusion: Option<String>,
    /// The `k` of the fusion. Default 60.
    pub rrf_k: Option<f64>,
    /// Units each leg contributes, 1 to 1000. Default 100.
    pub candidates: Option<f64>,
    /// Hits returned, 1 to 1000. Default 10.
    pub limit: Option<f64>,
}

/// The 1-based rank a unit held in each leg that found it.
#[napi(object)]
pub struct LegRanks {
    pub keyword: Option<u32>,
    pub vector: Option<u32>,
    pub relations: Option<u32>,
}

#[napi(object)]
pub struct HybridHit {
    pub id: String,
    /// The sum of the leg contributions, scaled by 1e9. Compare, do not interpret.
    pub score: i64,
    pub ranks: LegRanks,
}

/// An open database. Get one with `open(path)`; call `close()` when done.
#[napi]
pub struct Db {
    // `None` once closed.
    inner: RwLock<Option<kerno::Db>>,
}

impl Db {
    /// Runs `f` on the open database; the outer error is "closed", the inner one is the
    /// engine's, left for the caller to match on.
    fn raw<T>(&self, f: impl FnOnce(&kerno::Db) -> kerno::Result<T>) -> Result<kerno::Result<T>> {
        let guard = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        let db = guard.as_ref().ok_or_else(|| coded(ErrorCode::Closed, "the database is closed"))?;
        Ok(f(db))
    }

    fn with<T>(&self, f: impl FnOnce(&kerno::Db) -> kerno::Result<T>) -> Result<T> {
        self.raw(f)?.map_err(engine_error)
    }

    /// Runs `batch` as one commit.
    fn commit_batch(&self, batch: &Batch, meta: &CommitMeta) -> Result<kerno::Committed> {
        self.with(|db| db.commit_with(batch, meta))
    }

    /// The type of the field `field` of `unit_type`'s schema, when it has one.
    fn field_hint(&self, unit_type: &str, field: &str) -> Result<Option<FieldType>> {
        let schema = self.with(|db| db.schema(unit_type))?;
        Ok(schema.and_then(|s| s.field(field).map(|d| d.ty)))
    }

    /// A JS document as a `Document`, with each number typed by the schema of `unit_type`.
    fn to_document(&self, unit_type: &str, doc: HashMap<String, Cell>) -> Result<Document> {
        let mut out = Document::new();
        for (name, value) in doc {
            let hint = self.field_hint(unit_type, &name)?;
            out = out.with(name.clone(), to_field(&name, cell(&name, value)?, hint)?);
        }
        Ok(out)
    }
}

const MAX_SAFE: f64 = 9_007_199_254_740_992.0; // 2^53

fn to_field(name: &str, v: Field, hint: Option<FieldType>) -> Result<FieldValue> {
    Ok(match v {
        Either5::A(b) => FieldValue::Bool(b),
        Either5::B(n) if hint == Some(FieldType::Float) => FieldValue::Float(n),
        Either5::B(n) if n.fract() == 0.0 && n.abs() <= MAX_SAFE => FieldValue::Int(n as i64),
        Either5::B(_) if hint == Some(FieldType::Int) => {
            return Err(invalid(format!("field {name:?} is an int: the number is not a whole number in range")));
        }
        Either5::B(n) => FieldValue::Float(n),
        Either5::C(big) => match big.get_i64() {
            (i, true) => FieldValue::Int(i),
            _ => return Err(invalid(format!("field {name:?}: the bigint does not fit in 64 bits"))),
        },
        Either5::D(s) => FieldValue::Text(s),
        Either5::E(b) => FieldValue::Bytes(b.to_vec()),
    })
}

fn from_field(v: &FieldValue) -> Field {
    match v {
        FieldValue::Bool(b) => Either5::A(*b),
        FieldValue::Int(i) if i.unsigned_abs() <= 1 << 53 => Either5::B(*i as f64),
        FieldValue::Int(i) => Either5::C(BigInt::from(*i)),
        FieldValue::Float(f) => Either5::B(*f),
        FieldValue::Text(t) => Either5::D(t.clone()),
        FieldValue::Bytes(b) => Either5::E(Buffer::from(b.clone())),
    }
}

fn field_type(name: &str) -> Result<FieldType> {
    Ok(match name {
        "bool" => FieldType::Bool,
        "int" => FieldType::Int,
        "float" => FieldType::Float,
        "text" => FieldType::Text,
        "bytes" => FieldType::Bytes,
        other => return Err(invalid(format!("unknown field type {other:?}"))),
    })
}

fn endpoint(v: Option<Either<String, f64>>, what: &str) -> Result<Endpoint> {
    match v {
        Some(Either::A(hex)) => Ok(Endpoint::Unit(unit_id(&hex)?)),
        Some(Either::B(n)) => Ok(Endpoint::Op(whole(what, n)? as usize)),
        None => Err(invalid(format!("relate: `{what}` is missing"))),
    }
}

fn need<T>(v: Option<T>, op: &str, field: &str) -> Result<T> {
    v.ok_or_else(|| invalid(format!("op {op:?}: `{field}` is missing")))
}

fn add_op(db: &Db, batch: &mut Batch, op: CommitOp) -> Result<()> {
    let name = op.op.clone();
    let name = name.as_str();
    let id = |op: &CommitOp| -> Result<UnitId> { unit_id(&need(op.id.clone(), name, "id")?) };
    match name {
        "store" => {
            let (t, p) = (need(op.unit_type.clone(), name, "unitType")?, need(op.payload, name, "payload")?);
            batch.store(&t, into_bytes(p));
        }
        "update" => {
            let target = id(&op)?;
            let bytes = into_bytes(need(op.payload, name, "payload")?);
            if op.if_changed.unwrap_or(false) {
                batch.update_if_changed(target, bytes);
            } else {
                batch.update(target, bytes);
            }
        }
        "delete" => {
            let target = id(&op)?;
            if op.cascade.unwrap_or(false) {
                batch.delete_cascade(target);
            } else {
                batch.delete(target);
            }
        }
        "delete_cascade" => {
            batch.delete_cascade(id(&op)?);
        }
        "archive" => {
            batch.archive(id(&op)?);
        }
        "unarchive" => {
            batch.unarchive(id(&op)?);
        }
        "restore" => {
            let rev = whole("revision", need(op.revision, name, "revision")?)?;
            batch.restore(id(&op)?, rev);
        }
        "relate" => {
            let (from, to) = (endpoint(op.from, "from")?, endpoint(op.to, "to")?);
            let ty = need(op.relation_type, name, "relationType")?;
            if op.unique.unwrap_or(false) {
                batch.relate_unique(from, to, &ty);
            } else {
                batch.relate(from, to, &ty);
            }
        }
        "unrelate" => {
            batch.unrelate(relation_id(&need(op.relation_id, name, "relationId")?)?);
        }
        "upsert" => {
            let (t, field) = (need(op.unit_type, name, "unitType")?, need(op.field, name, "field")?);
            let doc = db.to_document(&t, need(op.document, name, "document")?)?;
            batch.upsert(&t, &field, doc.encode().map_err(engine_error)?);
        }
        other => return Err(invalid(format!("unknown commit op {other:?}"))),
    }
    Ok(())
}

fn parse_direction(d: &str) -> Result<Direction> {
    match d {
        "out" => Ok(Direction::Out),
        "in" => Ok(Direction::In),
        other => Err(invalid(format!("invalid direction {other:?}: expected \"out\" or \"in\""))),
    }
}

/// `f64` vectors from JS to the engine's `f32`.
fn to_f32(v: &[f64]) -> Vec<f32> {
    v.iter().map(|x| *x as f32).collect()
}

/// Opens the database in the directory `path`, creating it when there is none. One process
/// holds a directory at a time: opening it again before `close()` throws `LOCKED`.
#[napi]
pub fn open(path: String, options: Option<OpenOptions>) -> Result<Db> {
    let options = options.unwrap_or(OpenOptions { read_only: None, max_batch_ops: None });
    let config = if options.read_only.unwrap_or(false) {
        Config {
            access: AccessMode::ReadOnly,
            create_if_missing: false,
            allow_unverified_platform: true,
            ..Config::default()
        }
    } else {
        Config { allow_unverified_platform: true, ..Config::default() }
    };
    let max_ops = whole_u32("maxBatchOps", options.max_batch_ops)?
        .map_or(kerno::units::DEFAULT_MAX_CHANGESET_OPS, |n| n as usize);
    let db = kerno::Db::open_with_limits(path, config, max_ops).map_err(engine_error)?;
    Ok(Db { inner: RwLock::new(Some(db)) })
}

#[napi]
impl Db {
    /// Stores a new unit of type `unitType` (1 to 64 bytes of UTF-8) and returns its id. A
    /// payload is at most 1 MiB.
    #[napi]
    pub fn store(&self, unit_type: String, payload: Payload) -> Result<String> {
        let bytes = into_bytes(payload);
        self.with(|db| db.store(&unit_type, bytes)).map(|id| id.to_string())
    }

    /// The unit `id`, or `null` when there is none.
    #[napi]
    pub fn read(&self, id: String) -> Result<Option<Unit>> {
        let id = unit_id(&id)?;
        self.with(|db| db.get(id)).map(|unit| unit.map(Unit::from))
    }

    /// Replaces the payload of `id`, adding a revision; the earlier ones stay in its history.
    /// With `ifChanged`, an identical payload writes nothing. Returns whether a revision was
    /// written.
    #[napi]
    pub fn update(&self, id: String, payload: Payload, options: Option<UpdateOptions>) -> Result<bool> {
        let id = unit_id(&id)?;
        let bytes = into_bytes(payload);
        let mut batch = Batch::new();
        if options.and_then(|o| o.if_changed).unwrap_or(false) {
            batch.update_if_changed(id, bytes);
        } else {
            batch.update(id, bytes);
        }
        Ok(self.commit_batch(&batch, &CommitMeta::default())?.unchanged == 0)
    }

    /// Deletes `id` with a tombstone revision; its history is kept. Without `cascade` a unit
    /// that still has relations throws `HAS_RELATIONS`.
    #[napi]
    pub fn delete(&self, id: String, options: Option<DeleteOptions>) -> Result<()> {
        let id = unit_id(&id)?;
        let mut batch = Batch::new();
        if options.and_then(|o| o.cascade).unwrap_or(false) {
            batch.delete_cascade(id);
        } else {
            batch.delete(id);
        }
        self.commit_batch(&batch, &CommitMeta::default()).map(|_| ())
    }

    /// Relates `from` to `to` with a relation of type `relationType` (1 to 256 bytes) and
    /// returns the relation's id. Both units must exist and be active. With `unique`, an
    /// existing relation of that type between the two units is reused.
    #[napi]
    pub fn relate(
        &self,
        from: String,
        to: String,
        relation_type: String,
        options: Option<RelateOptions>,
    ) -> Result<String> {
        let (from, to) = (unit_id(&from)?, unit_id(&to)?);
        let mut batch = Batch::new();
        if options.and_then(|o| o.unique).unwrap_or(false) {
            batch.relate_unique(from, to, &relation_type);
        } else {
            batch.relate(from, to, &relation_type);
        }
        let done = self.commit_batch(&batch, &CommitMeta::default())?;
        done.relations.first().map(ToString::to_string).ok_or_else(|| coded(ErrorCode::Kerno, "no relation id"))
    }

    /// Removes the relation `relationId`.
    #[napi]
    pub fn unrelate(&self, relation_id: String) -> Result<()> {
        let edge = self::relation_id(&relation_id)?;
        self.with(|db| db.unrelate(edge))
    }

    /// The relations that start at (`out`) or end at (`in`) the unit `id`, optionally of one
    /// `relationType`.
    #[napi]
    pub fn related(
        &self,
        id: String,
        #[napi(ts_arg_type = "'out' | 'in'")] direction: String,
        relation_type: Option<String>,
    ) -> Result<Vec<Relation>> {
        let id = unit_id(&id)?;
        let direction = parse_direction(&direction)?;
        let edges = self.with(|db| db.related(id, direction, relation_type.as_deref()))?;
        Ok(edges.into_iter().map(Relation::from).collect())
    }

    /// Every live relation, optionally of one `relationType`, in one scan: much faster than
    /// calling `related` for each unit.
    #[napi]
    pub fn relations(&self, relation_type: Option<String>) -> Result<Vec<Relation>> {
        let edges = self.with(|db| db.relations(relation_type.as_deref()))?;
        Ok(edges.into_iter().map(Relation::from).collect())
    }

    /// Every active unit that matches `filter` (all of them without one), in id order.
    #[napi]
    pub fn query(&self, filter: Option<UnitFilter>) -> Result<Vec<Unit>> {
        let filter = filter.unwrap_or(UnitFilter { unit_type: None, schema_version: None });
        let filter = kerno::UnitFilter {
            unit_type: filter.unit_type,
            schema_version: whole_u32("schemaVersion", filter.schema_version)?,
        };
        let units = self.with(|db| db.query(&filter))?;
        Ok(units.into_iter().map(Unit::from).collect())
    }

    /// Commits `ops` atomically, as one change set, and returns what it wrote. See `CommitOp`
    /// for the ops. `reason`, `actor` and `source` are recorded with every revision.
    #[napi]
    pub fn commit(&self, ops: Vec<CommitOp>, options: Option<CommitOptions>) -> Result<Committed> {
        let mut batch = Batch::new();
        for op in ops {
            add_op(self, &mut batch, op)?;
        }
        let options = options.unwrap_or(CommitOptions { reason: None, actor: None, source: None });
        let meta = CommitMeta {
            reason: options.reason,
            actor: options.actor.map(into_bytes),
            source: options.source.map(into_bytes),
        };
        let done = self.commit_batch(&batch, &meta)?;
        Ok(Committed {
            units: done.units.iter().map(ToString::to_string).collect(),
            relations: done.relations.iter().map(ToString::to_string).collect(),
            unchanged: u32::try_from(done.unchanged).unwrap_or(u32::MAX),
            upserts: done.upserts.iter().map(Upserted::from).collect(),
            commit_seq: done.commit_seq.map(num),
        })
    }

    /// The newest `limit` revisions of `id` from `fromRevision` on, newest first; empty for an
    /// unknown unit. Throws `PRUNED` when `fromRevision` is below the oldest revision retention kept.
    #[napi]
    pub fn history(&self, id: String, options: Option<HistoryOptions>) -> Result<Vec<Revision>> {
        let id = unit_id(&id)?;
        let options = options.unwrap_or(HistoryOptions { limit: None, with_payload: None, from_revision: None });
        let limit = whole_u32("limit", options.limit)?.unwrap_or(100) as usize;
        if limit == 0 {
            return Err(invalid("limit must be at least 1"));
        }
        let from = options.from_revision.map(|r| whole("fromRevision", r).map(RevNo)).transpose()?;
        let q =
            HistoryQuery { from, with_payload: options.with_payload.unwrap_or(false), ..HistoryQuery::latest(limit) };
        match self.raw(|db| db.unit_db().history(id, q))? {
            Ok(revs) => Ok(revs.into_iter().map(Revision::from).collect()),
            Err(kerno::Error::NotFound { .. }) => Ok(Vec::new()),
            Err(e) => Err(engine_error(e)),
        }
    }

    /// The revision of `id` in force once commit `commitSeq` committed, with its payload;
    /// `null` when the unit did not exist yet. A deleted unit returns its tombstone. A
    /// sequence after the last commit reads the current state. Throws `PRUNED` when retention
    /// removed that revision.
    #[napi]
    pub fn read_as_of(&self, id: String, commit_seq: f64) -> Result<Option<Revision>> {
        let id = unit_id(&id)?;
        let seq = whole("commitSeq", commit_seq)?;
        match self.with(|db| db.read_as_of(id, seq))? {
            UnitAt::Absent => Ok(None),
            UnitAt::Present(r) | UnitAt::DeletedAt(r) => Ok(Some(Revision::from(r))),
            UnitAt::Pruned { oldest } => Err(coded(
                ErrorCode::Pruned,
                format!(
                    "commit {seq} predates the oldest kept revision (revision {}): retention pruned what it read. \
                         `readAsOf` takes a commit sequence; `history` and `compare` take revision numbers",
                    oldest.0
                ),
            )),
        }
    }

    /// The sequence of the last commit (0 for an empty database): the point to pass to
    /// `readAsOf` for "now".
    #[napi]
    pub fn last_commit_seq(&self) -> Result<i64> {
        self.with(|db| db.last_commit_seq()).map(num)
    }

    /// Moves a database opened with `readOnly: true` to the commits the writer made since it
    /// opened (or since the last refresh); returns the commit sequence it now reads. Throws
    /// `INVALID_ARGUMENT` on a handle that is not read-only (a read-write handle is always current).
    #[napi]
    pub fn refresh(&self) -> Result<i64> {
        self.with(|db| db.refresh()).map(num)
    }

    /// Checks every record of the units layer. A damaged store is reported, not thrown.
    #[napi]
    pub fn verify(&self) -> Result<VerifyReport> {
        let outcome = self.raw(|db| db.verify())?;
        match outcome {
            Ok(r) => Ok(VerifyReport {
                clean: r.is_clean(),
                commit_seq: num(r.seq.0),
                units: num(r.counts.units),
                revisions: num(r.counts.revisions),
                relations: num(r.counts.relations),
                findings: r.findings.iter().map(ToString::to_string).collect(),
                findings_omitted: num(r.findings_omitted),
                storage_error: None,
            }),
            // Storage-level damage stops the scan: report it, as the doc promises.
            Err(e) if e.is_corruption() => Ok(VerifyReport {
                clean: false,
                commit_seq: 0,
                units: 0,
                revisions: 0,
                relations: 0,
                findings: vec![e.to_string()],
                findings_omitted: 0,
                storage_error: Some(e.to_string()),
            }),
            Err(e) => Err(engine_error(e)),
        }
    }

    /// Drops the payloads of deleted units and compacts the revision ranges. A deleted unit keeps
    /// its history entries but not their payloads. Returns the stored bytes before (at call time)
    /// and after.
    #[napi]
    pub fn compact(&self) -> Result<CompactReport> {
        let r = self.with(|db| db.compact())?;
        Ok(CompactReport { before: r.stored_bytes_before.map(num), after: r.stored_bytes_after.map(num) })
    }

    /// Prunes old revisions as `options` says and returns what it removed. The head is always
    /// kept. Afterwards `history` from a pruned revision and `readAsOf` before the oldest kept
    /// one throw `PRUNED`. The policy is not stored: pass it each time. Idempotent.
    #[napi(ts_args_type = "options?: RetentionOptions | null")]
    pub fn run_retention(&self, options: Option<Unknown<'_>>) -> Result<RetentionReport> {
        // Read the options here, not in napi's argument conversion, so a value of the wrong type
        // is `INVALID_ARGUMENT` like every other refused rule.
        let options = match options {
            Some(u) if u.get_type().map_err(|e| invalid(e.reason))? != napi::ValueType::Object => {
                return Err(invalid("runRetention options is an object"));
            }
            // SAFETY: `u` is an argument of this call, so its env and handle are live for the
            // whole call, and `from_napi_value` only reads it (as napi's own conversion would).
            Some(u) => unsafe { Option::<RetentionOptions>::from_napi_value(u.value().env, u.raw()) }
                .map_err(|e| invalid(format!("runRetention options: {}", e.reason)))?,
            None => None,
        };
        let options = options.unwrap_or(RetentionOptions { default: None, by_type: None });
        let mut policy =
            RetentionPolicy::all(given(options.default).map(RetentionRule::into_rule).transpose()?.unwrap_or_default());
        for (unit_type, rule) in given(options.by_type).into_iter().flatten() {
            if let Either::A(rule) = rule {
                policy = policy.with_type(&unit_type, rule.into_rule()?);
            }
        }
        let r = self.with(|db| db.run_retention(&policy))?;
        Ok(RetentionReport {
            units_retained: num(r.units_retained),
            revisions_pruned: num(r.revisions_pruned),
            payloads_dropped: num(r.payloads_dropped),
            bytes_logically_freed: num(r.bytes_logically_freed),
            commit_seq: r.change_set.map(|c| num(c.commit_seq.0)),
        })
    }

    /// Turns keyword (BM25) indexing of `unitType` on or off. It is off by default; the setting
    /// is stored in the database, and the type's active units are indexed (or cleared) in the
    /// same commit. `hybridSearch`'s keyword leg only sees types that have it on.
    #[napi]
    pub fn set_fulltext(&self, unit_type: String, enabled: bool) -> Result<()> {
        self.with(|db| db.set_fulltext(&unit_type, enabled))
    }

    /// Whether `unitType` is keyword-indexed (off by default).
    #[napi]
    pub fn fulltext(&self, unit_type: String) -> Result<bool> {
        self.with(|db| db.fulltext(&unit_type))
    }

    /// Gives `unitType` a schema. Defining the schema the type has already (field order aside) is
    /// accepted and writes nothing, so a program can define on every start; a different one throws
    /// `SCHEMA_EXISTS`. A field indexed later with `createIndex` makes the first field list a
    /// different schema. An indexed text or bytes value holds at most 254 bytes (a 0x00 byte counts
    /// twice); a longer one throws `SCHEMA` on write.
    #[napi]
    pub fn define_schema(&self, unit_type: String, fields: Vec<FieldSpec>) -> Result<()> {
        let mut defs = Vec::with_capacity(fields.len());
        for f in fields {
            let mut def = FieldDef::new(f.name, field_type(&f.field_type)?);
            if f.required.unwrap_or(false) {
                def = def.required();
            }
            if f.unique.unwrap_or(false) {
                def = def.unique();
            }
            if f.indexed.unwrap_or(false) {
                def = def.indexed();
            }
            defs.push(def);
        }
        self.with(|db| db.define_schema(&unit_type, defs))
    }

    /// Indexes `field` of `unitType` (it must be in the type's schema).
    #[napi]
    pub fn create_index(&self, unit_type: String, field: String) -> Result<()> {
        self.with(|db| db.create_index(&unit_type, &field))
    }

    /// Stores a document (an object of boolean, number, bigint, string and Buffer values) and
    /// returns its id; the type's schema, if any, validates it.
    #[napi]
    pub fn store_document(
        &self,
        unit_type: String,
        #[napi(ts_arg_type = "Record<string, boolean | number | bigint | string | Buffer>")] document: HashMap<
            String,
            Cell,
        >,
    ) -> Result<String> {
        let doc = self.to_document(&unit_type, document)?;
        self.with(|db| db.store_document(&unit_type, &doc)).map(|id| id.to_string())
    }

    /// Stores `document` as a `unitType` unit, or revises the active unit that already holds the
    /// same value of `field`, a field the type's schema declares `unique`. Throws
    /// `NOT_UNIQUE` when `field` is not declared unique (or the type has no schema) and
    /// `SCHEMA` when the schema refuses the document or it has no value in `field`.
    #[napi]
    pub fn upsert(
        &self,
        unit_type: String,
        field: String,
        #[napi(ts_arg_type = "Record<string, boolean | number | bigint | string | Buffer>")] document: HashMap<
            String,
            Cell,
        >,
    ) -> Result<Upserted> {
        let doc = self.to_document(&unit_type, document)?;
        self.with(|db| db.upsert_document(&unit_type, &field, &doc)).map(|u| Upserted::from(&u))
    }

    /// Replaces the payload of `id` with a document.
    #[napi]
    pub fn update_document(
        &self,
        id: String,
        #[napi(ts_arg_type = "Record<string, boolean | number | bigint | string | Buffer>")] document: HashMap<
            String,
            Cell,
        >,
    ) -> Result<()> {
        let id = unit_id(&id)?;
        let unit = self.with(|db| db.get(id))?.ok_or_else(|| coded(ErrorCode::NotFound, format!("no unit {id}")))?;
        let doc = self.to_document(&unit.unit_type, document)?;
        self.with(|db| db.update_document(id, &doc))
    }

    /// The payload of `id` decoded as a document, or `null` for an unknown or deleted unit.
    #[napi(ts_return_type = "Record<string, boolean | number | bigint | string | Buffer> | null")]
    pub fn read_document(&self, id: String) -> Result<Option<HashMap<String, Field>>> {
        let id = unit_id(&id)?;
        let Some(payload) = self.with(|db| db.get(id))?.and_then(|u| u.payload) else { return Ok(None) };
        let doc = Document::decode(&payload).map_err(engine_error)?;
        Ok(Some(doc.iter().map(|(k, v)| (k.to_owned(), from_field(v))).collect()))
    }

    /// The units of `unitType` whose indexed `field` equals `value`. Throws `NOT_INDEXED` when
    /// `field` is not indexed and `INVALID_ARGUMENT` for a text or bytes `value` over 254 bytes (no
    /// unit can hold it; the answer is not an empty list).
    #[napi]
    pub fn find(
        &self,
        unit_type: String,
        field: String,
        #[napi(ts_arg_type = "boolean | number | bigint | string | Buffer")] value: Cell,
    ) -> Result<Vec<Unit>> {
        let hint = self.field_hint(&unit_type, &field)?;
        let value = to_field(&field, cell(&field, value)?, hint)?;
        let units = self.with(|db| db.find(&unit_type, &field, &value))?;
        Ok(units.into_iter().map(Unit::from).collect())
    }

    /// Runs a query of the Kerno query language.
    #[napi]
    pub fn execute(&self, query: String) -> Result<QueryResult> {
        let r = self.with(|db| db.execute(&query))?;
        Ok(QueryResult {
            aliases: r.aliases,
            rows: r.rows.into_iter().map(|row| row.into_iter().map(Unit::from).collect()).collect(),
        })
    }

    /// One query over keywords, vectors and relations, fused by weighted reciprocal rank
    /// fusion; best hit first. Wraps the engine's `hybrid_search`. A keyword signal with no type
    /// switched on by `setFulltext` throws `NOT_INDEXED` (a switched-on type with no match gives an
    /// empty list). The search is not scoped to one type.
    // ponytail: the vector leg ranks the `items` of each call by brute force; a persistent
    // vector index (HNSW) needs its own handle if a caller has more than ~100k vectors.
    #[napi]
    pub fn hybrid_search(&self, query: HybridQueryInput) -> Result<Vec<HybridHit>> {
        let mut q = HybridQuery::new(whole_u32("limit", query.limit)?.unwrap_or(10) as usize);
        q.text = query.text.as_deref();
        if let Some(w) = &query.weights {
            q.weights = HybridWeights {
                keyword: whole_u32("weights.keyword", w.keyword)?.unwrap_or(1),
                vector: whole_u32("weights.vector", w.vector)?.unwrap_or(1),
                graph: whole_u32("weights.relations", w.relations)?.unwrap_or(1),
            };
        }
        match query.fusion.as_deref() {
            None | Some("rrf") => {}
            Some("graph_gated") => q.fusion = Fusion::GraphGated,
            Some(other) => {
                return Err(invalid(format!("invalid fusion {other:?}: expected \"rrf\" or \"graph_gated\"")));
            }
        }
        if let Some(k) = whole_u32("rrfK", query.rrf_k)? {
            q.rrf_k = k;
        }
        if let Some(c) = whole_u32("candidates", query.candidates)? {
            q.candidates = c as usize;
        }
        if let Some(r) = &query.relations {
            let seeds = r.seeds.iter().map(|s| unit_id(s)).collect::<Result<Vec<_>>>()?;
            // The leg's own limit bounds its walk; `candidates` bounds what it contributes.
            let limit = q.candidates.min(kerno::units::MAX_ASSOC_LIMIT);
            let mut g = AssocQuery::new(
                seeds,
                whole_u32("maxDepth", r.max_depth)?.unwrap_or(2),
                limit,
                u64::from(whole_u32("visitBudget", r.visit_budget)?.unwrap_or(10_000)),
            );
            g.relation_types = r.types.as_ref().map(|t| t.iter().cloned().collect::<BTreeSet<_>>());
            g.direction = match r.direction.as_deref() {
                None | Some("both") => AssocDirection::Both,
                Some("out") => AssocDirection::Out,
                Some("in") => AssocDirection::In,
                Some(other) => {
                    return Err(invalid(format!("invalid direction {other:?}: expected \"out\", \"in\" or \"both\"")));
                }
            };
            q.graph = Some(g);
        }
        // The set must outlive the call; it is built before the query borrows it.
        let vectors = match &query.vectors {
            None => None,
            Some(v) => {
                let metric = match v.metric.as_deref() {
                    None | Some("cosine") => VectorMetric::Cosine,
                    Some("dot") => VectorMetric::Dot,
                    Some(other) => {
                        return Err(invalid(format!("invalid metric {other:?}: expected \"cosine\" or \"dot\"")));
                    }
                };
                let refuse = |e: kerno::units::assoc_rank::VectorError| invalid(format!("vector leg: {e}"));
                let mut set = VectorSet::new(v.query.len(), metric).map_err(refuse)?;
                for item in &v.items {
                    set.insert(unit_id(&item.id)?, to_f32(&item.vector)).map_err(refuse)?;
                }
                Some((set, to_f32(&v.query)))
            }
        };
        if let Some((set, qv)) = &vectors {
            q.vector = Some(VectorLeg { index: set, query: qv });
        }
        let hits = self.with(|db| db.hybrid_search(&q))?;
        Ok(hits
            .into_iter()
            .map(|h| HybridHit {
                id: h.unit.to_string(),
                score: num(h.score),
                ranks: LegRanks { keyword: h.ranks.keyword, vector: h.ranks.vector, relations: h.ranks.graph },
            })
            .collect())
    }

    /// Closes the database and releases its directory. Closing twice is fine; any other call
    /// after it throws `CLOSED`.
    #[napi]
    pub fn close(&self) -> Result<()> {
        let db = self.inner.write().unwrap_or_else(PoisonError::into_inner).take();
        db.map_or(Ok(()), kerno::Db::close).map_err(engine_error)
    }
}
