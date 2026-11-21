//! The core both transports call: every operation of the API over one open [`kerno::Db`].
//!
//! Everything here is synchronous (the engine is). The transports run a call on a blocking thread
//! with [`Service::run`], so a slow commit never stalls the async runtime.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kerno::storage::KernoDb;
use kerno::units::assoc_rank::{VectorMetric, VectorSet};
use kerno::units::{ChangeMeta, ChangeSet, ProvenanceKind, RevNo, UnitUpdate};
use kerno::{
    AssocDirection, AssocQuery, Db, Direction as Side, Document, FieldType as KernoFieldType, HybridQuery, Lifecycle,
    Relation, Unit, UnitFilter, UnitId, VectorLeg as KernoVectorLeg,
};
use kerno::{FieldDef as KernoFieldDef, FieldValue, RelationId};
use kerno_backup::{BaseInfo, Keeper, ObjectStore, Tick};

use crate::api::{
    Access, Candidate, CreateRelation, CreateUnit, Direction, ExplainReply, Field, FieldDef, FieldType, Fulltext,
    Health, Hex, Hit, JoinStep, ListRelations, ListUnits, Metric, Payload, QueryReply, QueryRequest, Ranks,
    RelationList, RelationView, Schema, SearchReply, SearchRequest, UnitList, UnitView, UpdateUnit, from_hex,
};
use crate::error::{ApiError, Kind};

pub type Res<T> = Result<T, ApiError>;

/// Units a page of `GET /v1/units` holds when the request does not say.
const DEFAULT_PAGE: usize = 100;
/// The most units one page holds.
const MAX_PAGE: usize = 1_000;
/// What the graph leg of a search uses when the request leaves a bound out (the same defaults as
/// the MCP server's `recall`).
const DEFAULT_DEPTH: u32 = 2;
const DEFAULT_GRAPH_LIMIT: usize = 20;
const DEFAULT_VISITS: u64 = 10_000;
/// Hits a search returns when the request does not say.
const DEFAULT_HITS: usize = 10;

/// Which unit types have a schema, for the duration of one answer: a payload is rendered as
/// `fields` only for those.
type Typed = HashMap<String, bool>;

/// What the backups of one database did since the server started.
#[derive(Default)]
pub struct BackupStats {
    pub commits: AtomicU64,
    pub segments: AtomicU64,
    pub bases: AtomicU64,
    pub errors: AtomicU64,
}

struct Backups {
    keeper: Mutex<Keeper>,
    stats: BackupStats,
}

/// An open database.
pub struct Service {
    db: Db,
    dir: PathBuf,
    backups: Option<Backups>,
}

impl Service {
    /// Opens the database in `dir`, creating it when `dir` holds none.
    /// The keyword index of each type in `fulltext` is turned on (it is off by default, ADR-0023).
    pub fn open(dir: &Path, fulltext: &[String]) -> Result<Self, String> {
        let db = Db::open(dir).map_err(|e| format!("cannot open the database in {}: {e}", dir.display()))?;
        for t in fulltext {
            db.set_fulltext(t, true).map_err(|e| format!("cannot keyword-index type {t:?}: {e}"))?;
        }
        Ok(Service { db, dir: dir.to_owned(), backups: None })
    }

    /// Keeps the backups of this database in `store`: see [`Service::backup_tick`].
    pub fn with_backups(mut self, store: Arc<dyn ObjectStore>) -> Self {
        let keeper = Mutex::new(Keeper::new(&self.dir, store));
        self.backups = Some(Backups { keeper, stats: BackupStats::default() });
        self
    }

    /// What the backups did so far; `None` when this database has none.
    pub fn backup_stats(&self) -> Option<&BackupStats> {
        self.backups.as_ref().map(|b| &b.stats)
    }

    fn backups(&self) -> Res<&Backups> {
        self.backups
            .as_ref()
            .ok_or_else(|| ApiError::not_found("backups are not configured: start the server with --backup <url>"))
    }

    /// Ships what was committed since the last time, taking a base backup first when the store has
    /// none or the write-ahead log no longer holds what was not shipped.
    pub fn backup_tick(&self) -> Res<Tick> {
        let backups = self.backups()?;
        let done = {
            let mut keeper = backups.keeper.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            keeper.tick(self.db.unit_db(), kerno_backup::now_ms())
        };
        match done {
            Ok(tick) => {
                let s = &backups.stats;
                s.commits.fetch_add(tick.shipped.commits, Ordering::Relaxed);
                s.segments.fetch_add(u64::from(tick.shipped.segments), Ordering::Relaxed);
                s.bases.fetch_add(u64::from(tick.base.is_some()), Ordering::Relaxed);
                Ok(tick)
            }
            Err(e) => Err(self.backup_failed(backups, &e)),
        }
    }

    /// Takes a base backup now, whatever the chain looks like.
    pub fn backup_now(&self) -> Res<BaseInfo> {
        let backups = self.backups()?;
        let done = {
            let mut keeper = backups.keeper.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            keeper.base_now(self.db.unit_db(), kerno_backup::now_ms())
        };
        match done {
            Ok(base) => {
                backups.stats.bases.fetch_add(1, Ordering::Relaxed);
                Ok(base)
            }
            Err(e) => Err(self.backup_failed(backups, &e)),
        }
    }

    fn backup_failed(&self, backups: &Backups, e: &kerno_backup::Error) -> ApiError {
        backups.stats.errors.fetch_add(1, Ordering::Relaxed);
        ApiError::internal(format!("backup: {e}"))
    }

    pub fn close(self) -> Result<(), String> {
        self.db.close().map_err(|e| format!("cannot close the database: {e}"))
    }

    /// Runs `work` on a blocking thread. A failure the client cannot fix is logged here, once, for
    /// both transports; a panic in `work` is an internal error, not a dead server.
    pub async fn run<T: Send + 'static>(
        self: &Arc<Self>,
        work: impl FnOnce(&Service) -> Res<T> + Send + 'static,
    ) -> Res<T> {
        let svc = Arc::clone(self);
        let out = tokio::task::spawn_blocking(move || work(&svc))
            .await
            .unwrap_or_else(|e| Err(ApiError::internal(format!("a request handler panicked: {e}"))));
        if let Err(e) = &out {
            e.log();
        }
        out
    }

    // ----------------------------------------------------------------- health

    /// Healthy when the database answers a read.
    pub fn health(&self) -> Res<Health> {
        self.db.get(UnitId([0; 16]))?;
        Ok(Health { status: "ok", version: env!("CARGO_PKG_VERSION") })
    }

    /// Ready to serve: the directory is still there (a volume unmounted or removed under a running
    /// server is not) and the database answers a read.
    pub fn ready(&self) -> Res<()> {
        if !self.dir.is_dir() {
            tracing::warn!(dir = %self.dir.display(), "the database directory is gone");
            // The probe needs no token: the answer names no directory and no tenant.
            return Err(ApiError::new(Kind::Unavailable, "a database directory is gone"));
        }
        self.health().map(|_| ())
    }

    // ------------------------------------------------------------------ units

    pub fn create_unit(&self, req: CreateUnit) -> Res<UnitView> {
        let bytes = self.bytes_of(&req.unit_type, req.payload)?;
        let id = self.db.store(&req.unit_type, bytes)?;
        self.unit(id)
    }

    pub fn get_unit(&self, id: &str) -> Res<UnitView> {
        self.unit(unit_id(id)?)
    }

    pub fn list_units(&self, req: ListUnits) -> Res<UnitList> {
        let limit = req.limit.unwrap_or(DEFAULT_PAGE);
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(ApiError::invalid(format!("limit {limit} is outside 1..={MAX_PAGE}")));
        }
        let after = req.after.as_deref().map(unit_id).transpose()?;
        // ponytail: reads every unit of the type and slices the page, so a page costs the whole
        // type; fine for thousands, give the engine a ranged scan if lists get long.
        let units = self.db.query(&UnitFilter { unit_type: req.unit_type, ..UnitFilter::default() })?;
        let mut rest = units.iter().filter(|u| after.is_none_or(|a| u.id > a));
        let page: Vec<&Unit> = rest.by_ref().take(limit).collect();
        let next = rest.next().and_then(|_| page.last()).map(|u| u.id.to_string());
        let mut typed = Typed::new();
        let units = page.into_iter().map(|u| self.unit_view(&mut typed, u)).collect::<Res<_>>()?;
        Ok(UnitList { units, next })
    }

    pub fn update_unit(&self, id: &str, req: UpdateUnit, expected: Option<u64>) -> Res<UnitView> {
        let id = unit_id(id)?;
        let current = self.db.get(id)?.ok_or_else(|| missing(id))?;
        let payload = self.bytes_of(&current.unit_type, req.payload)?;
        self.change(|changes| {
            changes.update(id, UnitUpdate { payload, schema_version: None, metadata: None }, expected.map(RevNo))
        })?;
        self.unit(id)
    }

    pub fn archive_unit(&self, id: &str, expected: Option<u64>) -> Res<UnitView> {
        let id = unit_id(id)?;
        self.change(|changes| changes.archive(id, expected.map(RevNo)))?;
        self.unit(id)
    }

    pub fn unarchive_unit(&self, id: &str, expected: Option<u64>) -> Res<UnitView> {
        let id = unit_id(id)?;
        self.change(|changes| changes.unarchive(id, expected.map(RevNo)))?;
        self.unit(id)
    }

    /// Deletes the unit logically: it stays readable as a tombstone with `lifecycle: deleted`. A
    /// unit that still has relations is refused (`conflict`); delete them first.
    pub fn delete_unit(&self, id: &str, expected: Option<u64>) -> Res<()> {
        let id = unit_id(id)?;
        self.change(|changes| changes.delete(id, expected.map(RevNo))).map(|_| ())
    }

    // -------------------------------------------------------------- relations

    pub fn create_relation(&self, req: CreateRelation) -> Res<RelationView> {
        let edge = self.db.relate(unit_id(&req.from)?, unit_id(&req.to)?, &req.relation_type)?;
        let relation = self.db.unit_db().snapshot()?.get_relation(edge)?;
        relation
            .map(|r| relation_view(&r, None))
            .ok_or_else(|| ApiError::internal(format!("relation {edge} vanished after its commit")))
    }

    pub fn list_relations(&self, unit: &str, req: ListRelations) -> Res<RelationList> {
        let unit = unit_id(unit)?;
        let snap = self.db.unit_db().snapshot()?;
        if snap.get_unit(unit)?.is_none() {
            return Err(missing(unit));
        }
        let sides: &[(Side, Direction)] = match req.direction {
            Direction::Out => &[(Side::Out, Direction::Out)],
            Direction::In => &[(Side::In, Direction::In)],
            Direction::Both => &[(Side::Out, Direction::Out), (Side::In, Direction::In)],
        };
        let mut relations = Vec::new();
        for &(side, shown) in sides {
            for r in snap.relations_of(unit, side, req.relation_type.as_deref())? {
                relations.push(relation_view(&r, Some(shown)));
            }
        }
        Ok(RelationList { relations })
    }

    pub fn delete_relation(&self, id: &str) -> Res<()> {
        let edge = relation_id(id)?;
        self.change(|changes| changes.delete_relation(edge))
    }

    // ---------------------------------------------------------------- keyword index

    /// Turns the keyword index of `unit_type` on or off; its units follow in the same commit.
    pub fn set_fulltext(&self, unit_type: &str, enabled: bool) -> Res<Fulltext> {
        self.db.set_fulltext(unit_type, enabled)?;
        self.get_fulltext(unit_type)
    }

    pub fn get_fulltext(&self, unit_type: &str) -> Res<Fulltext> {
        Ok(Fulltext { unit_type: unit_type.to_owned(), enabled: self.db.fulltext(unit_type)? })
    }

    // ---------------------------------------------------------------- schemas

    pub fn define_schema(&self, req: Schema) -> Res<Schema> {
        let fields = req.fields.iter().map(field_def).collect();
        self.db.define_schema(&req.unit_type, fields)?;
        self.get_schema(&req.unit_type)
    }

    pub fn get_schema(&self, unit_type: &str) -> Res<Schema> {
        let schema = self
            .db
            .schema(unit_type)?
            .ok_or_else(|| ApiError::not_found(format!("unit type {unit_type:?} has no schema")))?;
        let fields = schema.fields().iter().map(field_def_view).collect();
        Ok(Schema { unit_type: unit_type.to_owned(), fields })
    }

    // ------------------------------------------------------------------ query

    pub fn query(&self, req: QueryRequest) -> Res<QueryReply> {
        let result = self.db.execute(&req.query)?;
        let mut typed = Typed::new();
        let rows = result
            .rows
            .iter()
            .map(|row| row.iter().map(|u| self.unit_view(&mut typed, u)).collect::<Res<Vec<_>>>())
            .collect::<Res<_>>()?;
        Ok(QueryReply { aliases: result.aliases, rows, access: access_view(&result.access) })
    }

    pub fn explain(&self, req: QueryRequest) -> Res<ExplainReply> {
        let plan = self.db.explain(&req.query)?;
        Ok(ExplainReply {
            root_alias: plan.root_alias,
            root_type: plan.root_type,
            candidates: plan
                .candidates
                .iter()
                .map(|c| Candidate { access: access_view(&c.access), cost: c.cost.0 })
                .collect(),
            chosen: plan.chosen,
            joins: plan
                .joins
                .into_iter()
                .map(|j| JoinStep {
                    alias: j.alias,
                    unit_type: j.unit_type,
                    relation_type: j.relation_type,
                    direction: match j.direction {
                        Side::Out => Direction::Out,
                        Side::In => Direction::In,
                    },
                    from: j.from,
                })
                .collect(),
        })
    }

    // ----------------------------------------------------------------- search

    pub fn search(&self, req: SearchRequest) -> Res<SearchReply> {
        let mut query = HybridQuery::new(req.limit.unwrap_or(DEFAULT_HITS));
        query.text = req.text.as_deref();
        if let Some(k) = req.rrf_k {
            query.rrf_k = k;
        }
        if let Some(n) = req.candidates {
            query.candidates = n;
        }
        if let Some(w) = &req.weights {
            let defaults = query.weights;
            query.weights.keyword = w.keyword.unwrap_or(defaults.keyword);
            query.weights.vector = w.vector.unwrap_or(defaults.vector);
            query.weights.graph = w.graph.unwrap_or(defaults.graph);
        }
        if let Some(g) = &req.graph {
            let seeds = g.seeds.iter().map(|s| unit_id(s)).collect::<Res<Vec<_>>>()?;
            let mut graph = AssocQuery::new(
                seeds,
                g.max_depth.unwrap_or(DEFAULT_DEPTH),
                g.limit.unwrap_or(DEFAULT_GRAPH_LIMIT),
                g.visit_budget.unwrap_or(DEFAULT_VISITS),
            );
            graph.direction = match g.direction {
                Direction::Out => AssocDirection::Out,
                Direction::In => AssocDirection::In,
                Direction::Both => AssocDirection::Both,
            };
            graph.relation_types = g.relation_types.as_ref().map(|t| t.iter().cloned().collect::<BTreeSet<_>>());
            query.graph = Some(graph);
        }
        // The set must outlive the query that borrows it.
        let vectors = req.vector.as_ref().map(vector_set).transpose()?;
        if let (Some(set), Some(leg)) = (&vectors, &req.vector) {
            query.vector = Some(KernoVectorLeg { index: set, query: &leg.query });
        }

        let hits = self.db.hybrid_search(&query)?;
        let mut typed = Typed::new();
        let mut out = Vec::with_capacity(hits.len());
        for hit in hits {
            // A unit deleted between the search and this read is not a hit any more.
            let Some(unit) = self.db.get(hit.unit)? else { continue };
            out.push(Hit {
                id: hit.unit.to_string(),
                score: hit.score,
                ranks: Ranks { keyword: hit.ranks.keyword, vector: hit.ranks.vector, graph: hit.ranks.graph },
                unit: self.unit_view(&mut typed, &unit)?,
            });
        }
        Ok(SearchReply { hits: out })
    }

    // --------------------------------------------------------------- internals

    /// One committed change set holding what `work` does.
    fn change<T>(&self, work: impl FnOnce(&mut ChangeSet<'_, KernoDb>) -> Result<T, kerno::Error>) -> Res<T> {
        let mut changes = self.db.unit_db().begin_changeset(ChangeMeta::default())?;
        let out = work(&mut changes)?;
        changes.commit()?;
        Ok(out)
    }

    fn unit(&self, id: UnitId) -> Res<UnitView> {
        let unit = self.db.get(id)?.ok_or_else(|| missing(id))?;
        self.unit_view(&mut Typed::new(), &unit)
    }

    fn has_schema(&self, typed: &mut Typed, unit_type: &str) -> Res<bool> {
        if let Some(&known) = typed.get(unit_type) {
            return Ok(known);
        }
        let has = self.db.schema(unit_type)?.is_some();
        typed.insert(unit_type.to_owned(), has);
        Ok(has)
    }

    fn unit_view(&self, typed: &mut Typed, unit: &Unit) -> Res<UnitView> {
        let payload = match &unit.payload {
            None => None,
            Some(bytes) if self.has_schema(typed, &unit.unit_type)? => {
                let record = Document::decode(bytes)?;
                Some(Payload::Fields(record.iter().map(|(k, v)| (k.to_owned(), Field(v.clone()))).collect()))
            }
            Some(bytes) => Some(match std::str::from_utf8(bytes) {
                Ok(text) => Payload::Text(text.to_owned()),
                Err(_) => Payload::Hex(Hex(bytes.clone())),
            }),
        };
        Ok(UnitView {
            id: unit.id.to_string(),
            unit_type: unit.unit_type.clone(),
            lifecycle: match unit.lifecycle {
                Lifecycle::Active => "active",
                Lifecycle::Archived => "archived",
                Lifecycle::Deleted => "deleted",
            },
            revision: unit.head.0,
            created_at_us: unit.created_at.0,
            updated_at_us: unit.updated_at.0,
            payload,
        })
    }

    /// The bytes to store for `payload` in a unit of `unit_type`.
    fn bytes_of(&self, unit_type: &str, payload: Payload) -> Res<Vec<u8>> {
        match payload {
            Payload::Text(text) => Ok(text.into_bytes()),
            Payload::Hex(Hex(bytes)) => Ok(bytes),
            Payload::Fields(fields) => {
                let schema = self.db.schema(unit_type)?.ok_or_else(|| {
                    ApiError::invalid(format!(
                        "unit type {unit_type:?} has no schema: send a text or hex payload, or define the schema first"
                    ))
                })?;
                let mut record = Document::new();
                for (name, Field(value)) in fields {
                    let value = match (schema.field(&name).map(|f| f.ty), value) {
                        (Some(KernoFieldType::Float), FieldValue::Int(i)) => whole_float(i),
                        (_, value) => value,
                    };
                    record = record.with(name, value);
                }
                Ok(record.encode()?)
            }
        }
    }
}

/// An integer JSON number written for a float field: the float it names, when it names one exactly.
fn whole_float(i: i64) -> FieldValue {
    let f = i as f64;
    if f as i128 == i128::from(i) { FieldValue::Float(f) } else { FieldValue::Int(i) }
}

fn missing(unit: UnitId) -> ApiError {
    ApiError::not_found(format!("unit {unit} not found"))
}

fn bad_id(what: &str) -> ApiError {
    ApiError::invalid(format!("{what} id must be 32 hex digits"))
}

fn id_bytes(text: &str, what: &str) -> Res<[u8; 16]> {
    from_hex(text).ok().and_then(|b| <[u8; 16]>::try_from(b).ok()).ok_or_else(|| bad_id(what))
}

pub fn unit_id(text: &str) -> Res<UnitId> {
    id_bytes(text, "unit").map(UnitId)
}

fn relation_id(text: &str) -> Res<RelationId> {
    id_bytes(text, "relation").map(RelationId)
}

fn relation_view(r: &Relation, direction: Option<Direction>) -> RelationView {
    RelationView {
        id: r.id.to_string(),
        from: r.from.to_string(),
        to: r.to.to_string(),
        relation_type: r.relation_type.clone(),
        provenance: match r.provenance.kind() {
            ProvenanceKind::Declared => "declared",
            ProvenanceKind::Imported => "imported",
            ProvenanceKind::Algorithmic => "algorithmic",
            ProvenanceKind::Inferred => "inferred",
        },
        direction,
    }
}

fn access_view(access: &kerno::units::Access) -> Access {
    match access {
        kerno::units::Access::TypeScan => Access::TypeScan,
        kerno::units::Access::FieldIndex { field } => Access::FieldIndex { field: field.clone() },
    }
}

fn kerno_type(t: FieldType) -> KernoFieldType {
    match t {
        FieldType::Bool => KernoFieldType::Bool,
        FieldType::Int => KernoFieldType::Int,
        FieldType::Float => KernoFieldType::Float,
        FieldType::Text => KernoFieldType::Text,
        FieldType::Bytes => KernoFieldType::Bytes,
    }
}

fn field_def(f: &FieldDef) -> KernoFieldDef {
    let mut def = KernoFieldDef::new(f.name.clone(), kerno_type(f.field_type));
    if f.required {
        def = def.required();
    }
    if f.indexed {
        def = def.indexed();
    }
    if f.unique {
        def = def.unique();
    }
    def
}

fn field_def_view(f: &KernoFieldDef) -> FieldDef {
    FieldDef {
        name: f.name.clone(),
        field_type: match f.ty {
            KernoFieldType::Bool => FieldType::Bool,
            KernoFieldType::Int => FieldType::Int,
            KernoFieldType::Float => FieldType::Float,
            KernoFieldType::Text => FieldType::Text,
            KernoFieldType::Bytes => FieldType::Bytes,
        },
        required: f.required,
        unique: f.unique,
        indexed: f.indexed,
    }
}

/// The exact vector index of one search: the vectors the request carries.
fn vector_set(leg: &crate::api::VectorLeg) -> Res<VectorSet> {
    let bad = |e: kerno::units::assoc_rank::VectorError| ApiError::invalid(format!("vector leg: {e}"));
    let metric = match leg.metric {
        Metric::Cosine => VectorMetric::Cosine,
        Metric::Dot => VectorMetric::Dot,
    };
    let mut set = VectorSet::new(leg.query.len(), metric).map_err(bad)?;
    for (id, vector) in &leg.vectors {
        set.insert(unit_id(id)?, vector.clone()).map_err(bad)?;
    }
    Ok(set)
}
