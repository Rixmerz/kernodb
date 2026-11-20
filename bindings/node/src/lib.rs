//! Node.js bindings for Kerno: a thin napi-rs layer over the [`kerno`] facade crate.
//!
//! Ids cross the boundary as 32-digit lower-case hex strings, payloads as `Buffer` (a `string`
//! is accepted on the way in and stored as UTF-8). Every method is synchronous, like an
//! embedded database driver: a call returns when the commit is done.
// ponytail: sync calls block the event loop for the length of a commit; move to AsyncTask
// if a caller needs to keep the loop free during writes.

use std::sync::{PoisonError, RwLock};

use napi::bindgen_prelude::{Buffer, Either};
use napi_derive::napi;

type Result<T> = napi::Result<T>;

fn js_err(e: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(e.to_string())
}

/// 32 hex digits to the 16 bytes of an id.
fn parse_id(hex: &str) -> Result<[u8; 16]> {
    let bad = || js_err(format!("invalid id {hex:?}: expected 32 hex digits"));
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

fn unit_id(hex: &str) -> Result<kerno::UnitId> {
    parse_id(hex).map(kerno::UnitId)
}

/// A unit as of its latest revision.
#[napi(object)]
pub struct Unit {
    /// 32 hex digits.
    pub id: String,
    pub unit_type: String,
    pub schema_version: u32,
    /// `read` and `query` only return `active` units today.
    #[napi(ts_type = "'active' | 'archived' | 'deleted'")]
    pub lifecycle: String,
    /// Microseconds since the Unix epoch; descriptive, not an ordering guarantee.
    pub created_at_us: i64,
    pub updated_at_us: i64,
    /// The content; absent for a deleted unit.
    pub payload: Option<Buffer>,
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

/// Which units `query` returns. A field left out matches everything.
#[napi(object)]
pub struct UnitFilter {
    pub unit_type: Option<String>,
    pub schema_version: Option<u32>,
}

impl From<kerno::Unit> for Unit {
    fn from(u: kerno::Unit) -> Self {
        Unit {
            id: u.id.to_string(),
            unit_type: u.unit_type,
            schema_version: u.schema_version,
            lifecycle: match u.lifecycle {
                kerno::Lifecycle::Active => "active",
                kerno::Lifecycle::Archived => "archived",
                kerno::Lifecycle::Deleted => "deleted",
            }
            .to_owned(),
            created_at_us: u.created_at.0,
            updated_at_us: u.updated_at.0,
            payload: u.payload.map(Buffer::from),
        }
    }
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

/// An open database. Get one with `open(path)`; call `close()` when done.
#[napi]
pub struct Db {
    // `None` once closed.
    inner: RwLock<Option<kerno::Db>>,
}

impl Db {
    fn with<T>(&self, f: impl FnOnce(&kerno::Db) -> kerno::Result<T>) -> Result<T> {
        let guard = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        let db = guard.as_ref().ok_or_else(|| js_err("the database is closed"))?;
        f(db).map_err(js_err)
    }
}

/// Opens the database in the directory `path`, creating it when there is none. One process
/// holds a directory at a time: opening it again before `close()` throws.
#[napi]
pub fn open(path: String) -> Result<Db> {
    let db = kerno::Db::open(path).map_err(js_err)?;
    Ok(Db { inner: RwLock::new(Some(db)) })
}

#[napi]
impl Db {
    /// Stores a new unit of type `unitType` (1 to 64 bytes of UTF-8) and returns its id.
    #[napi]
    pub fn store(&self, unit_type: String, payload: Either<String, Buffer>) -> Result<String> {
        let payload: Vec<u8> = match payload {
            Either::A(text) => text.into_bytes(),
            Either::B(bytes) => bytes.to_vec(),
        };
        self.with(|db| db.store(&unit_type, payload)).map(|id| id.to_string())
    }

    /// The unit `id`, or `null` when there is none.
    #[napi]
    pub fn read(&self, id: String) -> Result<Option<Unit>> {
        let id = unit_id(&id)?;
        self.with(|db| db.get(id)).map(|unit| unit.map(Unit::from))
    }

    /// Relates `from` to `to` with a relation of type `relationType` (1 to 256 bytes) and
    /// returns the relation's id. Both units must exist and be active.
    #[napi]
    pub fn relate(&self, from: String, to: String, relation_type: String) -> Result<String> {
        let (from, to) = (unit_id(&from)?, unit_id(&to)?);
        self.with(|db| db.relate(from, to, &relation_type)).map(|id| id.to_string())
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
        let direction = match direction.as_str() {
            "out" => kerno::Direction::Out,
            "in" => kerno::Direction::In,
            other => return Err(js_err(format!("invalid direction {other:?}: expected \"out\" or \"in\""))),
        };
        let edges = self.with(|db| db.related(id, direction, relation_type.as_deref()))?;
        Ok(edges.into_iter().map(Relation::from).collect())
    }

    /// Every active unit that matches `filter` (all of them without one), in id order.
    #[napi]
    pub fn query(&self, filter: Option<UnitFilter>) -> Result<Vec<Unit>> {
        let filter = filter.unwrap_or(UnitFilter { unit_type: None, schema_version: None });
        let filter = kerno::UnitFilter { unit_type: filter.unit_type, schema_version: filter.schema_version };
        let units = self.with(|db| db.query(&filter))?;
        Ok(units.into_iter().map(Unit::from).collect())
    }

    /// Closes the database and releases its directory. Closing twice is fine; any other call
    /// after it throws.
    #[napi]
    pub fn close(&self) -> Result<()> {
        let db = self.inner.write().unwrap_or_else(PoisonError::into_inner).take();
        db.map_or(Ok(()), kerno::Db::close).map_err(js_err)
    }
}
