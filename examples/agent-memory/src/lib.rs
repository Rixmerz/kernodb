//! Memory for an agent, built on the public API of `kerno-units` only.
//!
//! A memory is a unit (text as payload, tags as metadata). Its vector is a
//! persisted derived datum; a summary of several memories is an in-memory
//! derived datum. The caller supplies every vector: [`standin::Embedder`] is a
//! deterministic stand-in for a model.
//!
//! Everything the library does not offer is kept in one extra unit, the
//! *catalog* (see [`catalog`]): the ids of the memories, the derived id of each
//! one's vector and the unit sets of the summaries. See `FRICTION.md` for why.

mod catalog;
pub mod standin;

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use kerno_core::vfs::StdVfs;
use kerno_storage::KernoDb;
use kerno_units::assoc_rank::{SIMILARITY_SCALE, VectorMetric, VectorQuery, VectorSet};
use kerno_units::{
    ChangeMeta, Clock, ConsolidationError, DerivedDb, DerivedId, DerivedStaleError, DerivedState, NewUnit,
    RetentionBudget, RetentionPolicy, RetentionRule, RevNo, RevisionId, Timestamp, TruncatingSummarizer, Unit,
    UnitConfig, UnitDb, UnitError, UnitId, UnitIdSource, UnitUpdate, VectorSpec,
};

use catalog::Catalog;
pub use standin::Embedder;

/// The type of the unit that holds a memory.
const MEMORY_TYPE: &str = "memory";
/// The unit that holds the catalog: the first id the id source mints.
const CATALOG_ID: u64 = 1;
const METHOD: &str = "standin-embed";
const PROFILE: &str = "standin-v1";
/// Longest piece of each memory the test summariser keeps.
const SUMMARY_BYTES: usize = 24;

/// Anything that can go wrong; `Display` carries no memory content.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Unit(UnitError),
    Derived(DerivedStaleError),
    Consolidation(ConsolidationError),
    /// No such memory (never stored, or forgotten).
    NotFound(u64),
    /// The input breaks a rule of this library.
    Invalid(String),
    /// The catalog is damaged.
    Corrupt(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "i/o: {e}"),
            Error::Unit(e) => write!(f, "units: {e}"),
            Error::Derived(e) => write!(f, "derived data: {e}"),
            Error::Consolidation(e) => write!(f, "summary: {e}"),
            Error::NotFound(id) => write!(f, "no memory #{id}"),
            Error::Invalid(m) => write!(f, "invalid: {m}"),
            Error::Corrupt(m) => write!(f, "damaged catalog: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
impl From<UnitError> for Error {
    fn from(e: UnitError) -> Self {
        Error::Unit(e)
    }
}
impl From<DerivedStaleError> for Error {
    fn from(e: DerivedStaleError) -> Self {
        Error::Derived(e)
    }
}
impl From<ConsolidationError> for Error {
    fn from(e: ConsolidationError) -> Self {
        Error::Consolidation(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A memory as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memory {
    pub id: u64,
    pub text: String,
    pub tags: Vec<String>,
    /// The revision number: 1 when stored, +1 per correction.
    pub revision: u64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// What recall by vector found.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub memory: Memory,
    /// Cosine similarity in `[-1, 1]`.
    pub similarity: f64,
}

/// The state of the vector of one memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorStatus {
    /// No vector was ever attached.
    None,
    Ready,
    /// The memory was corrected after the vector was made: it is not served.
    Stale,
    /// The catalog names a vector the registry does not know.
    Missing,
    Other(DerivedState),
}

/// A summary over a set of memories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryInfo {
    pub index: usize,
    pub sources: Vec<u64>,
    pub state: DerivedState,
    /// The text, only while `ready`.
    pub text: Option<String>,
}

/// What opening found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenReport {
    /// Vectors loaded `Ready` with their bytes verified.
    pub vectors_served: usize,
    /// Vectors persisted `Ready` that are served `Stale`.
    pub vectors_demoted: usize,
    /// Findings of the derived-record check.
    pub findings: usize,
    /// Summaries rebuilt from the catalog.
    pub summaries_rebuilt: usize,
}

/// Mints `00…00 ‖ n`. Unlike `CounterIds`, its counter can be moved once the
/// catalog has been read (the catalog is itself a unit, so the database must
/// be open before the next free number is known).
struct Ids(AtomicU64);

fn unit_id(n: u64) -> UnitId {
    let mut b = [0u8; 16];
    b[8..].copy_from_slice(&n.to_be_bytes());
    UnitId(b)
}

fn memory_number(u: UnitId) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&u.0[8..]);
    u64::from_be_bytes(b)
}

impl UnitIdSource for Ids {
    fn next_id(&self) -> UnitId {
        unit_id(self.0.fetch_add(1, Ordering::SeqCst))
    }
}

fn meta(reason: &str) -> ChangeMeta {
    ChangeMeta { reason: Some(reason.to_owned()), actor: Some(b"agent-memory".to_vec()), ..ChangeMeta::default() }
}

fn encode_tags(tags: &[String]) -> Result<Vec<u8>> {
    if tags.iter().any(|t| t.is_empty() || t.contains('\n')) {
        return Err(Error::Invalid("a tag is empty or contains a newline".into()));
    }
    let meta = tags.join("\n").into_bytes();
    if meta.len() > 4096 {
        return Err(Error::Invalid("tags take more than 4096 bytes".into()));
    }
    Ok(meta)
}

fn memory_of(u: &Unit) -> Result<Memory> {
    let payload = u.payload.clone().unwrap_or_default();
    let text = String::from_utf8(payload).map_err(|_| Error::Corrupt("a memory is not UTF-8".into()))?;
    let tags = String::from_utf8_lossy(&u.metadata).split('\n').filter(|t| !t.is_empty()).map(str::to_owned).collect();
    Ok(Memory {
        id: memory_number(u.id),
        text,
        tags,
        revision: u.head.0,
        created_at: u.created_at,
        updated_at: u.updated_at,
    })
}

/// The memory of an agent over one directory.
pub struct AgentMemory {
    db: DerivedDb<KernoDb>,
    ids: Arc<Ids>,
    embedder: Embedder,
    summarizer: TruncatingSummarizer,
    /// `(sources, derived id)` of each summary, in creation order.
    summaries: Mutex<Vec<(Vec<u64>, DerivedId)>>,
    /// Serialises read-modify-write of the catalog.
    write: Mutex<()>,
}

impl AgentMemory {
    /// Opens (creating if needed) the memory in `dir`.
    pub fn open(dir: &Path, clock: Arc<dyn Clock>, embedder: Embedder) -> Result<(Self, OpenReport)> {
        std::fs::create_dir_all(dir)?;
        let ids = Arc::new(Ids(AtomicU64::new(CATALOG_ID)));
        let cfg = UnitConfig::with_sources(clock, ids.clone());
        let db = UnitDb::<KernoDb>::open(StdVfs::shared(), dir, &cfg)?;
        let (db, reopen) = DerivedDb::open_persisted(db)?;
        let this = AgentMemory {
            db,
            ids,
            embedder,
            summarizer: TruncatingSummarizer::new(SUMMARY_BYTES, 1),
            summaries: Mutex::new(Vec::new()),
            write: Mutex::new(()),
        };
        let mut report = OpenReport {
            vectors_served: reopen.served,
            vectors_demoted: reopen.demoted.len(),
            findings: reopen.findings.len(),
            summaries_rebuilt: 0,
        };
        match this.db.get_unit(unit_id(CATALOG_ID))? {
            None => {
                let mut cs = this.db.begin_changeset(meta("create the catalog"))?;
                let id = cs.create(NewUnit {
                    unit_type: "catalog".into(),
                    schema_version: 1,
                    metadata: vec![],
                    payload: Catalog { next: CATALOG_ID + 1, ..Catalog::default() }.encode(),
                })?;
                debug_assert_eq!(id, unit_id(CATALOG_ID));
                cs.commit()?;
            }
            Some(u) => {
                let cat = Catalog::parse(u.payload.as_deref().unwrap_or_default())?;
                this.ids.0.store(cat.next, Ordering::SeqCst);
                // Summaries live in memory only: rebuild them from the unit
                // sets the catalog remembers.
                for sources in &cat.summaries {
                    let units = sources.iter().map(|n| unit_id(*n));
                    if let Ok(id) = this.db.build_summary(units, &this.summarizer) {
                        this.lock_summaries().push((sources.clone(), id));
                        report.summaries_rebuilt += 1;
                    }
                }
            }
        }
        Ok((this, report))
    }

    /// Closes the database.
    pub fn close(self) -> Result<()> {
        Ok(self.db.close()?)
    }

    fn lock_summaries(&self) -> MutexGuard<'_, Vec<(Vec<u64>, DerivedId)>> {
        self.summaries.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn lock_write(&self) -> MutexGuard<'_, ()> {
        self.write.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn catalog(&self) -> Result<Catalog> {
        let u = self.db.get_unit(unit_id(CATALOG_ID))?.ok_or_else(|| Error::Corrupt("no catalog unit".into()))?;
        Catalog::parse(u.payload.as_deref().unwrap_or_default())
    }

    fn entry(&self, id: u64) -> Result<Option<DerivedId>> {
        self.catalog()?.vectors.get(&id).copied().ok_or(Error::NotFound(id))
    }

    fn vector_spec(&self, source: RevisionId, vector: Vec<f32>) -> VectorSpec {
        VectorSpec {
            source,
            method: METHOD.into(),
            method_version: 1,
            profile: PROFILE.into(),
            params: self.embedder.params(),
            vector,
        }
    }

    /// Stores a memory. With `vector`, it is persisted in the same commit.
    pub fn remember(&self, text: &str, tags: &[String], vector: Option<Vec<f32>>) -> Result<u64> {
        let metadata = encode_tags(tags)?;
        let _w = self.lock_write();
        let mut cat = self.catalog()?;
        let mut cs = self.db.begin_changeset(meta("remember"))?;
        let unit = cs.create(NewUnit {
            unit_type: MEMORY_TYPE.into(),
            schema_version: 1,
            metadata,
            payload: text.as_bytes().to_vec(),
        })?;
        let n = memory_number(unit);
        let vid = match vector {
            Some(v) => Some(cs.put_vector(&self.vector_spec(RevisionId { unit, rev: RevNo(1) }, v))?),
            None => None,
        };
        cat.vectors.insert(n, vid);
        cat.next = self.ids.0.load(Ordering::SeqCst);
        cs.update(
            unit_id(CATALOG_ID),
            UnitUpdate { payload: cat.encode(), schema_version: None, metadata: None },
            None,
        )?;
        cs.commit()?;
        Ok(n)
    }

    fn save_catalog(&self, cat: &Catalog, reason: &str) -> Result<()> {
        let mut cs = self.db.begin_changeset(meta(reason))?;
        cs.update(
            unit_id(CATALOG_ID),
            UnitUpdate { payload: cat.encode(), schema_version: None, metadata: None },
            None,
        )?;
        cs.commit()?;
        Ok(())
    }

    /// Attaches a caller-supplied vector to a memory that has none, or whose
    /// vector is stale (the old one is replaced).
    pub fn attach_vector(&self, id: u64, vector: Vec<f32>) -> Result<()> {
        let _w = self.lock_write();
        let mut cat = self.catalog()?;
        let old = *cat.vectors.get(&id).ok_or(Error::NotFound(id))?;
        let unit = unit_id(id);
        let new = match old {
            None => {
                let head = self.db.get_unit(unit)?.ok_or(Error::NotFound(id))?.head;
                let mut cs = self.db.begin_changeset(meta("attach a vector"))?;
                let new = cs.put_vector(&self.vector_spec(RevisionId { unit, rev: head }, vector))?;
                cat.vectors.insert(id, Some(new));
                cs.update(
                    unit_id(CATALOG_ID),
                    UnitUpdate { payload: cat.encode(), schema_version: None, metadata: None },
                    None,
                )?;
                cs.commit()?;
                return Ok(());
            }
            Some(old) if self.db.state(&old) == Some(DerivedState::Stale) => {
                self.db.rebuild_vector(&old, meta("replace a stale vector"), &|_, _| Some(vector.clone()))?
            }
            Some(_) => return Err(Error::Invalid(format!("memory #{id} already has a vector"))),
        };
        cat.vectors.insert(id, Some(new));
        self.save_catalog(&cat, "record the new vector")
    }

    /// The state of the vector of memory `id`.
    pub fn vector_status(&self, id: u64) -> Result<VectorStatus> {
        Ok(match self.entry(id)? {
            None => VectorStatus::None,
            Some(vid) => match self.db.state(&vid) {
                Some(DerivedState::Ready) => VectorStatus::Ready,
                Some(DerivedState::Stale) => VectorStatus::Stale,
                Some(other) => VectorStatus::Other(other),
                None => VectorStatus::Missing,
            },
        })
    }

    /// The vector of memory `id`, only while it is `Ready`.
    pub fn vector(&self, id: u64) -> Result<Option<Vec<f32>>> {
        Ok(self.entry(id)?.and_then(|vid| self.db.vector(&vid)))
    }

    /// The derived id of the vector of memory `id`, if one was attached.
    pub fn vector_id(&self, id: u64) -> Result<Option<DerivedId>> {
        self.entry(id)
    }

    /// The state the registry holds for a derived datum; `None` when it knows
    /// no such datum (never built, or removed with its source).
    pub fn derived_state(&self, id: &DerivedId) -> Option<DerivedState> {
        self.db.state(id)
    }

    /// Recomputes a stale vector with the stand-in model from the memory as it
    /// is now.
    pub fn rebuild_vector(&self, id: u64) -> Result<()> {
        let _w = self.lock_write();
        let mut cat = self.catalog()?;
        let old = cat.vectors.get(&id).copied().ok_or(Error::NotFound(id))?;
        let old = old.ok_or_else(|| Error::Invalid(format!("memory #{id} has no vector to rebuild")))?;
        let embedder = self.embedder;
        let new = self.db.rebuild_vector(&old, meta("rebuild a stale vector"), &|_, inputs| {
            inputs.first().map(|i| embedder.embed(&i.payload))
        })?;
        cat.vectors.insert(id, Some(new));
        self.save_catalog(&cat, "record the rebuilt vector")
    }

    /// Reads a memory.
    pub fn get(&self, id: u64) -> Result<Memory> {
        self.catalog()?.vectors.get(&id).ok_or(Error::NotFound(id))?;
        match self.db.get_unit(unit_id(id))? {
            Some(u) if u.payload.is_some() => memory_of(&u),
            _ => Err(Error::NotFound(id)),
        }
    }

    /// Every memory, oldest first.
    pub fn all(&self) -> Result<Vec<Memory>> {
        let mut out = Vec::new();
        for id in self.catalog()?.vectors.keys() {
            out.push(self.get(*id)?);
        }
        out.sort_by_key(|m| (m.created_at, m.id));
        Ok(out)
    }

    /// Memories created in `[from, to]`, oldest first.
    pub fn recall_between(&self, from: Timestamp, to: Timestamp) -> Result<Vec<Memory>> {
        Ok(self.all()?.into_iter().filter(|m| m.created_at >= from && m.created_at <= to).collect())
    }

    /// The `n` most recently created memories, newest first.
    pub fn recent(&self, n: usize) -> Result<Vec<Memory>> {
        let mut all = self.all()?;
        all.reverse();
        all.truncate(n);
        Ok(all)
    }

    /// The `k` memories whose `Ready` vector is closest to `query` (cosine).
    /// A memory without a servable vector (none, or stale) is not a candidate.
    pub fn recall_similar(&self, query: &[f32], k: usize) -> Result<Vec<Hit>> {
        let q = VectorQuery::new(VectorMetric::Cosine, query.to_vec())
            .map_err(|e| Error::Invalid(format!("query vector: {e}")))?;
        let mut set = VectorSet::new(query.len(), VectorMetric::Cosine)
            .map_err(|e| Error::Invalid(format!("query vector: {e}")))?;
        for (id, vid) in self.catalog()?.vectors {
            if let Some(v) = vid.and_then(|vid| self.db.vector(&vid)) {
                // A vector of another dimension (another model) is skipped.
                let _ = set.insert(unit_id(id), v);
            }
        }
        let scored = set.search(&q, k).map_err(|e| Error::Invalid(format!("search: {e}")))?;
        scored
            .into_iter()
            .map(|s| {
                Ok(Hit {
                    memory: self.get(memory_number(s.unit))?,
                    similarity: s.similarity as f64 / SIMILARITY_SCALE as f64,
                })
            })
            .collect()
    }

    /// Writes a new revision of a memory. Its vector, if any, becomes stale in
    /// the same commit. Returns the new revision number.
    pub fn correct(&self, id: u64, text: &str) -> Result<u64> {
        self.entry(id)?;
        let mut cs = self.db.begin_changeset(meta("correct"))?;
        let rev = cs.update(
            unit_id(id),
            UnitUpdate { payload: text.as_bytes().to_vec(), schema_version: None, metadata: None },
            None,
        )?;
        cs.commit()?;
        Ok(rev.0)
    }

    /// Forgets a memory: the unit is deleted and, in the same commit, its
    /// vector and every summary it is a source of; the catalog entry goes too.
    pub fn forget(&self, id: u64) -> Result<()> {
        let _w = self.lock_write();
        let mut cat = self.catalog()?;
        cat.vectors.remove(&id).ok_or(Error::NotFound(id))?;
        cat.summaries.retain(|s| !s.contains(&id));
        let mut cs = self.db.begin_changeset(meta("forget"))?;
        cs.delete(unit_id(id), None)?;
        cs.update(
            unit_id(CATALOG_ID),
            UnitUpdate { payload: cat.encode(), schema_version: None, metadata: None },
            None,
        )?;
        cs.commit()?;
        self.lock_summaries().retain(|(_, sid)| self.db.state(sid).is_some());
        Ok(())
    }

    /// Summarises the given memories with the test summariser.
    pub fn summarize(&self, ids: &[u64]) -> Result<usize> {
        let _w = self.lock_write();
        let mut cat = self.catalog()?;
        let mut sources: Vec<u64> = ids.to_vec();
        sources.sort_unstable();
        sources.dedup();
        for id in &sources {
            if !cat.vectors.contains_key(id) {
                return Err(Error::NotFound(*id));
            }
        }
        let sid = self.db.build_summary(sources.iter().map(|n| unit_id(*n)), &self.summarizer)?;
        cat.summaries.push(sources.clone());
        self.save_catalog(&cat, "record a summary")?;
        let mut list = self.lock_summaries();
        list.push((sources, sid));
        Ok(list.len() - 1)
    }

    /// Rebuilds every stale summary from the current revisions of its
    /// sources. Returns how many were rebuilt.
    pub fn refresh_summaries(&self) -> Result<usize> {
        let mut list = self.lock_summaries();
        let mut n = 0;
        for (_, sid) in list.iter_mut() {
            if self.db.state(sid) == Some(DerivedState::Stale) {
                *sid = self.db.rebuild_summary(sid, &self.summarizer)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Drops every revision but the newest of every unit (retention). The bytes
    /// of the dropped revisions stay in the files until the engine compacts.
    /// Returns how many revisions were pruned.
    pub fn prune_history(&self) -> Result<u64> {
        let policy = RetentionPolicy::all(RetentionRule::KeepLast(1));
        let report = self.db.run_retention(&policy, RetentionBudget::default())?;
        Ok(report.retention.revisions_pruned)
    }

    /// The summaries, in creation order.
    pub fn summaries(&self) -> Vec<SummaryInfo> {
        self.lock_summaries()
            .iter()
            .enumerate()
            .filter_map(|(index, (sources, sid))| {
                Some(SummaryInfo {
                    index,
                    sources: sources.clone(),
                    state: self.db.state(sid)?,
                    text: self.db.summary(sid).map(|b| String::from_utf8_lossy(&b).into_owned()),
                })
            })
            .collect()
    }
}
