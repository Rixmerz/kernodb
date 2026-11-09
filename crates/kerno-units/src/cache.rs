//! E6-01: the dependency model and the learned-state store of the result
//! cache (etapa6-acta.md §3, ADR-0016 G4 and G7, ADR-0015).
//!
//! The one optimization of Stage 6 is a cache of query results keyed by the
//! query's [`Fingerprint`]. This module holds what that optimization **is**
//! (a [`CacheEntry`] with its [`Dependencies`]) and where it **lives** (the
//! [`ResultCache`]). It is not wired into `query` or the planner yet (E6-02):
//! nothing in the crate reads it unless a caller asks.
//!
//! # Dependencies (G4)
//!
//! An entry is created with every dependency it was computed under, and
//! there is no way to create one without them: [`CacheEntry::new`] takes a
//! [`Dependencies`], whose fields are not optional (the index status is
//! `None` only for a relation query, which has no index status, I-23).
//!
//! | [`Dependency`] | Version | Changes when |
//! |---|---|---|
//! | `SchemaVersion` | [`LAYER_VERSION`] | the layer's record formats change |
//! | `PlannerVersion` | [`PLANNER_VERSION`] | the planner's behaviour changes |
//! | `IndexState` | `Ready`, `Building`, `Stale`, `Failed` | the index the entry read changes freshness |
//! | `IndexWatermark` | the commit the index reflects | the index is rebuilt |
//! | `CommitSeq` | the snapshot's commit sequence | any commit; **exact match** |
//!
//! [`ResultCache::lookup`] compares them all with the current ones **before
//! every use** ([`Dependencies::first_change`]); the first that differs, in
//! the order of the table, is named in the answer and the entry is dropped.
//! An entry never serves another `commit_seq`: there is no "the data it read
//! did not change" exception (G4, acta §2.3).
//!
//! # The store
//!
//! In memory only (acta §8, decision 3), separate from user data: the store
//! holds no database handle, opens no transaction and writes nothing to the
//! engine, so it cannot take part in a user transaction. Its entries hold
//! user content (a result is rows), which is why nothing here is persisted or
//! logged.
//!
//! * **Off by default** (acta §8, decision 2). While off, no entry is read or
//!   written and a lookup costs one atomic load. Turning it off drops every
//!   entry.
//! * **Per-query switch**: [`ResultCache::disable_query`] turns the cache off
//!   for one fingerprint and drops its entry.
//! * **Bounded** by size and age ([`CacheConfig`]): at most `max_entries`
//!   entries and `max_bytes` (estimated) of results, none older than
//!   `max_age`. The oldest go first.
//! * [`ResultCache::clear`] deletes all learned state: entries, log and
//!   counters. It keeps the switches, like `Observer::clear` keeps the level.
//!
//! # The transition log (G7)
//!
//! Every decision about an optimization leaves a [`TransitionRecord`]: create
//! (with its [`Evidence`]), activate, use, fallback with its cause,
//! invalidate with the dependency that changed, enable/disable and delete.
//! Each carries [`OPTIMIZATION_VERSION`] and the planner version. Per ADR-0015
//! a record holds the fingerprint hash, the commit sequence and counters,
//! nothing else: no field exists for a filter, a unit, a row or a message.
//! The log has the same retention as the entries and is dropped by `clear`.
//!
//! ponytail: entries sit in a hash map and the bounds find the oldest entry
//! by scanning it. With the default 256 entries that is a few hundred
//! comparisons per insertion past the bound; switch to an ordered index if
//! `max_entries` is raised into the thousands.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::clock::{Clock, SystemClock};
use crate::codec::LAYER_VERSION;
use crate::db::{Relation, Unit};
use crate::error::UnitError;
use crate::fingerprint::{Fingerprint, QueryKind};
use crate::index::{IndexState, IndexStatus};
use crate::planner::PLANNER_VERSION;
use crate::types::{CommitSeq, RelationProvenance, Timestamp};

/// The version of the result-cache optimization, recorded in every
/// transition. Bumped when what an entry means changes.
pub const OPTIMIZATION_VERSION: u32 = 1;

/// Default of [`CacheConfig::max_entries`].
pub const DEFAULT_MAX_ENTRIES: usize = 256;

/// Default of [`CacheConfig::max_bytes`]: 16 MiB of results.
pub const DEFAULT_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Default of [`CacheConfig::max_age`].
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(3600);

/// Default of [`CacheConfig::max_log_records`].
pub const DEFAULT_MAX_LOG_RECORDS: usize = 1024;

/// What an entry depends on; see the [module documentation](self). The order
/// of [`ALL`](Self::ALL) is the order [`Dependencies::first_change`] checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dependency {
    SchemaVersion,
    PlannerVersion,
    IndexState,
    IndexWatermark,
    CommitSeq,
}

impl Dependency {
    /// Every declared dependency, in the order they are checked.
    pub const ALL: [Dependency; 5] = [
        Dependency::SchemaVersion,
        Dependency::PlannerVersion,
        Dependency::IndexState,
        Dependency::IndexWatermark,
        Dependency::CommitSeq,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Dependency::SchemaVersion => "schema_version",
            Dependency::PlannerVersion => "planner_version",
            Dependency::IndexState => "index_state",
            Dependency::IndexWatermark => "index_watermark",
            Dependency::CommitSeq => "commit_seq",
        }
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The versions an entry was computed under, or the ones in force now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dependencies {
    /// The commit sequence of the snapshot the result was read at (I-04).
    pub commit_seq: CommitSeq,
    /// The status of the primary index the planner saw: `Some` for a unit
    /// query, `None` for a relation query.
    pub index: Option<IndexStatus>,
    /// [`LAYER_VERSION`] when the result was read.
    pub schema_version: u32,
    /// [`PLANNER_VERSION`] when the result was read.
    pub planner_version: u32,
}

impl Dependencies {
    /// The dependencies in force now for a snapshot at `commit_seq` that saw
    /// this `index` status.
    pub fn current(commit_seq: CommitSeq, index: Option<IndexStatus>) -> Self {
        Dependencies { commit_seq, index, schema_version: LAYER_VERSION, planner_version: PLANNER_VERSION }
    }

    /// The version number of one dependency; `None` when it does not apply
    /// (the index of a relation query, or a watermark that never existed).
    /// Every dependency has one: that is what makes it declared.
    pub fn version_of(&self, dependency: Dependency) -> Option<u64> {
        match dependency {
            Dependency::SchemaVersion => Some(u64::from(self.schema_version)),
            Dependency::PlannerVersion => Some(u64::from(self.planner_version)),
            Dependency::IndexState => self.index.map(|i| match i.state {
                IndexState::Ready => 0,
                IndexState::Building => 1,
                IndexState::Stale => 2,
                IndexState::Failed => 3,
            }),
            Dependency::IndexWatermark => self.index.and_then(|i| i.watermark).map(|w| w.0),
            Dependency::CommitSeq => Some(self.commit_seq.0),
        }
    }

    /// The first dependency, in [`Dependency::ALL`] order, whose version
    /// differs from `now`; `None` when every one matches.
    pub fn first_change(&self, now: &Dependencies) -> Option<Dependency> {
        Dependency::ALL.into_iter().find(|d| self.version_of(*d) != now.version_of(*d))
    }
}

/// A cached answer: the rows, as the query returned them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CachedResult {
    Units(Vec<Unit>),
    Relations(Vec<Relation>),
}

impl CachedResult {
    pub fn kind(&self) -> QueryKind {
        match self {
            CachedResult::Units(_) => QueryKind::Units,
            CachedResult::Relations(_) => QueryKind::Relations,
        }
    }

    /// An estimate of the memory the rows hold: the structs plus their
    /// variable-length parts. It bounds the store; it does not need to be exact.
    pub fn approx_bytes(&self) -> usize {
        match self {
            CachedResult::Units(units) => units
                .iter()
                .map(|u| {
                    std::mem::size_of::<Unit>()
                        + u.unit_type.len()
                        + u.metadata.len()
                        + u.payload.as_ref().map_or(0, Vec::len)
                })
                .sum(),
            CachedResult::Relations(relations) => relations
                .iter()
                .map(|r| {
                    let provenance = match &r.provenance {
                        RelationProvenance::Declared => 0,
                        RelationProvenance::Imported { source } => source.len(),
                        RelationProvenance::Algorithmic { origin } | RelationProvenance::Inferred { origin, .. } => {
                            origin.len()
                        }
                    };
                    std::mem::size_of::<Relation>() + r.relation_type.len() + provenance
                })
                .sum(),
        }
    }
}

/// Why an optimization was created (spec §9): how many observations proposed
/// it and over how long. Counters only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Evidence {
    /// Observations of this fingerprint behind the decision. At least 1.
    pub observations: u64,
    /// The span those observations cover.
    pub window: Duration,
}

/// A cache entry: a result and every dependency it was computed under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    fingerprint: Fingerprint,
    dependencies: Dependencies,
    result: Arc<CachedResult>,
    evidence: Evidence,
}

impl CacheEntry {
    /// An entry for the query of `fingerprint`. Refused when it could not be
    /// explained truthfully: the result is of the other kind of query, the
    /// index status does not match the kind (a unit query records one, a
    /// relation query none, as in [`Plan::new`](crate::Plan::new)), or there
    /// is no evidence.
    pub fn new(
        fingerprint: Fingerprint,
        dependencies: Dependencies,
        result: CachedResult,
        evidence: Evidence,
    ) -> Result<Self, CacheError> {
        let kind = fingerprint.kind();
        if result.kind() != kind {
            return Err(CacheError::KindMismatch);
        }
        if dependencies.index.is_some() != (kind == QueryKind::Units) {
            return Err(CacheError::IndexStatus { kind });
        }
        if evidence.observations == 0 {
            return Err(CacheError::NoEvidence);
        }
        Ok(CacheEntry { fingerprint, dependencies, result: Arc::new(result), evidence })
    }

    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    pub fn dependencies(&self) -> &Dependencies {
        &self.dependencies
    }

    pub fn evidence(&self) -> Evidence {
        self.evidence
    }
}

/// Why the store refused something. A class, never content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheError {
    /// The cache is off, globally or for this query: nothing is written.
    Disabled,
    /// The result is of the other kind of query than the fingerprint's.
    KindMismatch,
    /// The index status is absent for a unit query or present for a relation query.
    IndexStatus { kind: QueryKind },
    /// An optimization without evidence is not created (spec §9).
    NoEvidence,
    /// The result alone is larger than `max_bytes`.
    TooLarge,
    /// No entry for that fingerprint.
    NoEntry,
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Disabled => f.write_str("the result cache is disabled"),
            CacheError::KindMismatch => f.write_str("the result is not of the fingerprint's kind of query"),
            CacheError::IndexStatus { kind } => write!(f, "the index status does not match a {kind:?} query"),
            CacheError::NoEvidence => f.write_str("a cache entry needs at least one observation as evidence"),
            CacheError::TooLarge => f.write_str("the result is larger than the cache's byte bound"),
            CacheError::NoEntry => f.write_str("no cache entry for that fingerprint"),
        }
    }
}

impl std::error::Error for CacheError {}

/// Retention of the entries and of the log; see the [module documentation](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheConfig {
    /// Most entries kept. At least 1.
    pub max_entries: usize,
    /// Most bytes of results kept, as [`CachedResult::approx_bytes`] counts. At least 1.
    pub max_bytes: usize,
    /// Longest an entry is kept, from its creation. More than zero.
    pub max_age: Duration,
    /// Most transition records kept. At least 1. They also expire at `max_age`.
    pub max_log_records: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            max_entries: DEFAULT_MAX_ENTRIES,
            max_bytes: DEFAULT_MAX_BYTES,
            max_age: DEFAULT_MAX_AGE,
            max_log_records: DEFAULT_MAX_LOG_RECORDS,
        }
    }
}

impl CacheConfig {
    fn validate(&self) -> Result<(), UnitError> {
        let bad = |what: &str| Err(UnitError::InvalidArgument(format!("cache {what}")));
        if self.max_entries == 0 {
            return bad("max_entries must be >= 1");
        }
        if self.max_bytes == 0 {
            return bad("max_bytes must be >= 1");
        }
        if self.max_log_records == 0 {
            return bad("max_log_records must be >= 1");
        }
        if self.max_age.is_zero() {
            return bad("max_age must be > 0");
        }
        Ok(())
    }

    fn max_age_micros(&self) -> i64 {
        i64::try_from(self.max_age.as_micros()).unwrap_or(i64::MAX)
    }
}

/// Which switch a transition is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Switch {
    /// The global one: the record has no fingerprint.
    Global,
    /// The per-query one: the record has the query's fingerprint.
    Query,
}

/// Why the planner abandoned the cache for the reference path (I-33). The
/// planner's own `FallbackCause` describes a path that failed; these describe
/// the learned state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheFallbackCause {
    /// A precondition checked at this use did not hold.
    PreconditionFailed,
    /// A dependency changed ([`Lookup::Invalidated`]).
    Invalidated,
    /// The cache is off for this query.
    Disabled,
    /// The cached answer disagreed with the reference path. The entry is dropped.
    Discrepancy,
}

/// Why an entry was deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeleteReason {
    /// Asked for with [`ResultCache::delete`].
    Requested,
    /// A newer entry for the same fingerprint took its place.
    Replaced,
    /// Pushed out by `max_entries` or `max_bytes`.
    EvictedBySize,
    /// Older than `max_age`.
    EvictedByAge,
    /// Dropped with the cache or the query switched off.
    Disabled,
    /// Dropped after a discrepancy with the reference path.
    Discrepancy,
}

/// What happened to an optimization (G7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    /// An entry was created, on this evidence. It serves nothing until activated.
    Created {
        evidence: Evidence,
    },
    Activated,
    /// An active entry served a lookup.
    Used,
    Fallback {
        cause: CacheFallbackCause,
    },
    /// A dependency of the entry changed; the entry is dropped with this record.
    Invalidated {
        dependency: Dependency,
    },
    Enabled {
        switch: Switch,
    },
    Disabled {
        switch: Switch,
    },
    Deleted {
        reason: DeleteReason,
    },
    /// [`ResultCache::clear`] deleted this many entries and every earlier record.
    Cleared {
        entries: u64,
    },
}

/// One record of the transition log. Fingerprint hashes, versions and
/// counters only (ADR-0015).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionRecord {
    /// Local, increasing per store.
    pub id: u64,
    /// The query's fingerprint; `None` for the global switch and for `clear`.
    pub fingerprint: Option<Fingerprint>,
    pub transition: Transition,
    pub optimization_version: u32,
    /// The planner version the entry was computed under; [`PLANNER_VERSION`]
    /// for a record with no entry.
    pub planner_version: u32,
    /// The snapshot sequence of the entry, when the record is about one.
    pub commit_seq: Option<CommitSeq>,
    /// Descriptive time of the record; it orders no commit (I-26).
    pub recorded_at: Timestamp,
}

/// The answer of [`ResultCache::lookup`].
#[derive(Clone, Debug)]
pub enum Lookup {
    /// An active entry whose dependencies all match the current ones. The
    /// planner still validates the rows before returning them (G5).
    Hit(Arc<CachedResult>),
    /// No entry (or it had expired and was dropped).
    Miss,
    /// An entry exists but was never activated.
    Inactive,
    /// A dependency changed; the entry was dropped. Run the reference path.
    Invalidated(Dependency),
    /// The cache is off, globally or for this query. Nothing was read.
    Disabled,
}

/// Counters of the store since the last [`clear`](ResultCache::clear).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub enabled: bool,
    /// Entries read: lookups that found one. Zero while the cache is off.
    pub reads: u64,
    /// Entries written.
    pub writes: u64,
    pub hits: u64,
    pub invalidations: u64,
    pub fallbacks: u64,
    /// Entries pushed out by the size or age bound.
    pub evicted: u64,
    /// Transition records pushed out by their bounds.
    pub log_evicted: u64,
    /// Entries now.
    pub entries: usize,
    /// Estimated bytes of results now.
    pub bytes: usize,
}

struct Slot {
    entry: CacheEntry,
    active: bool,
    created_at: Timestamp,
    bytes: usize,
    /// Insertion order: the lowest is the oldest.
    seq: u64,
}

struct State {
    config: CacheConfig,
    clock: Arc<dyn Clock>,
    disabled_queries: HashSet<Fingerprint>,
    entries: HashMap<Fingerprint, Slot>,
    bytes: usize,
    next_slot: u64,
    next_id: u64,
    log: VecDeque<TransitionRecord>,
    stats: CacheStats,
}

impl State {
    fn record(
        &mut self,
        fingerprint: Option<Fingerprint>,
        transition: Transition,
        entry: Option<&Dependencies>,
        at: Timestamp,
    ) {
        let record = TransitionRecord {
            id: self.next_id,
            fingerprint,
            transition,
            optimization_version: OPTIMIZATION_VERSION,
            planner_version: entry.map_or(PLANNER_VERSION, |d| d.planner_version),
            commit_seq: entry.map(|d| d.commit_seq),
            recorded_at: at,
        };
        self.next_id = self.next_id.saturating_add(1);
        self.log.push_back(record);
        self.prune_log(at);
    }

    fn prune_log(&mut self, now: Timestamp) {
        let oldest_kept = now.0.saturating_sub(self.config.max_age_micros());
        while let Some(front) = self.log.front() {
            if self.log.len() > self.config.max_log_records || front.recorded_at.0 < oldest_kept {
                self.log.pop_front();
                self.stats.log_evicted = self.stats.log_evicted.saturating_add(1);
            } else {
                break;
            }
        }
    }

    /// Takes an entry out. `reason` is the `Deleted` record to leave; `None`
    /// when another record (invalidation) already says why it is gone.
    fn remove(&mut self, fingerprint: &Fingerprint, reason: Option<DeleteReason>, at: Timestamp) -> bool {
        let Some(slot) = self.entries.remove(fingerprint) else {
            return false;
        };
        self.bytes = self.bytes.saturating_sub(slot.bytes);
        if let Some(reason) = reason {
            self.record(Some(*fingerprint), Transition::Deleted { reason }, Some(&slot.entry.dependencies), at);
        }
        true
    }

    /// Drops what the age and size bounds no longer allow, oldest first.
    fn prune_entries(&mut self, now: Timestamp) {
        let oldest_kept = now.0.saturating_sub(self.config.max_age_micros());
        let mut expired: Vec<(u64, Fingerprint)> =
            self.entries.iter().filter(|(_, s)| s.created_at.0 < oldest_kept).map(|(f, s)| (s.seq, *f)).collect();
        expired.sort_by_key(|(seq, _)| *seq);
        for (_, fp) in expired {
            self.remove(&fp, Some(DeleteReason::EvictedByAge), now);
            self.stats.evicted = self.stats.evicted.saturating_add(1);
        }
        while self.entries.len() > self.config.max_entries || self.bytes > self.config.max_bytes {
            let Some(oldest) = self.entries.iter().min_by_key(|(_, s)| s.seq).map(|(f, _)| *f) else {
                break;
            };
            self.remove(&oldest, Some(DeleteReason::EvictedBySize), now);
            self.stats.evicted = self.stats.evicted.saturating_add(1);
        }
    }

    /// Drops every entry, leaving one `Deleted` record each.
    fn drop_all_entries(&mut self, reason: DeleteReason, at: Timestamp) {
        let mut all: Vec<(u64, Fingerprint)> = self.entries.iter().map(|(f, s)| (s.seq, *f)).collect();
        all.sort_by_key(|(seq, _)| *seq);
        for (_, fp) in all {
            self.remove(&fp, Some(reason), at);
        }
    }
}

/// The learned state of the result cache and its transition log; see the
/// [module documentation](self).
pub struct ResultCache {
    /// Read without the lock so that a cache that is off costs one load; every
    /// write happens under the lock, and every operation that reads or writes
    /// an entry checks it again under the lock.
    enabled: AtomicBool,
    state: Mutex<State>,
}

impl Default for ResultCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ResultCache {
    /// A cache that is **off**, with the default bounds and the system clock.
    pub fn new() -> Self {
        ResultCache {
            enabled: AtomicBool::new(false),
            state: Mutex::new(State {
                config: CacheConfig::default(),
                clock: Arc::new(SystemClock),
                disabled_queries: HashSet::new(),
                entries: HashMap::new(),
                bytes: 0,
                next_slot: 1,
                next_id: 1,
                log: VecDeque::new(),
                stats: CacheStats::default(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Recovered: the state is maps and counters that a panic cannot leave
        // inconsistent in a way a dependency check would not catch.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Turns the cache on or off for every query. Turning it off drops every
    /// entry (each leaves a `Deleted` record) and records the switch.
    pub fn set_enabled(&self, on: bool) {
        let mut st = self.lock();
        if self.enabled.load(Ordering::Acquire) == on {
            return;
        }
        let now = st.clock.now();
        self.enabled.store(on, Ordering::Release);
        let switch = Switch::Global;
        if on {
            st.record(None, Transition::Enabled { switch }, None, now);
        } else {
            st.record(None, Transition::Disabled { switch }, None, now);
            st.drop_all_entries(DeleteReason::Disabled, now);
        }
    }

    /// Turns the cache off for one query and drops its entry.
    pub fn disable_query(&self, fingerprint: Fingerprint) {
        let mut st = self.lock();
        if !st.disabled_queries.insert(fingerprint) {
            return;
        }
        let now = st.clock.now();
        st.record(Some(fingerprint), Transition::Disabled { switch: Switch::Query }, None, now);
        st.remove(&fingerprint, Some(DeleteReason::Disabled), now);
    }

    /// Undoes [`disable_query`](Self::disable_query).
    pub fn enable_query(&self, fingerprint: Fingerprint) {
        let mut st = self.lock();
        if st.disabled_queries.remove(&fingerprint) {
            let now = st.clock.now();
            st.record(Some(fingerprint), Transition::Enabled { switch: Switch::Query }, None, now);
        }
    }

    /// Whether the cache is on globally and not off for this query.
    pub fn query_enabled(&self, fingerprint: &Fingerprint) -> bool {
        self.enabled() && !self.lock().disabled_queries.contains(fingerprint)
    }

    /// Replaces the bounds; what they no longer allow is dropped now.
    pub fn set_config(&self, config: CacheConfig) -> Result<(), UnitError> {
        config.validate()?;
        let mut st = self.lock();
        st.config = config;
        let now = st.clock.now();
        st.prune_entries(now);
        st.prune_log(now);
        Ok(())
    }

    /// Replaces the store's clock (the default is [`SystemClock`]). It is the
    /// only time source the store reads, and it is read under the store's
    /// lock, so it must not block.
    pub fn set_clock(&self, clock: Arc<dyn Clock>) {
        self.lock().clock = clock;
    }

    /// Stores `entry`, **not yet active**. Writes nothing and returns
    /// [`CacheError::Disabled`] while the cache is off for this query. An
    /// entry of the same fingerprint is replaced.
    pub fn create(&self, entry: CacheEntry) -> Result<(), CacheError> {
        if !self.enabled() {
            return Err(CacheError::Disabled);
        }
        let mut st = self.lock();
        let fp = entry.fingerprint;
        if !self.enabled() || st.disabled_queries.contains(&fp) {
            return Err(CacheError::Disabled);
        }
        let bytes = entry.result.approx_bytes();
        if bytes > st.config.max_bytes {
            return Err(CacheError::TooLarge);
        }
        let now = st.clock.now();
        st.remove(&fp, Some(DeleteReason::Replaced), now);
        st.record(Some(fp), Transition::Created { evidence: entry.evidence }, Some(&entry.dependencies), now);
        let seq = st.next_slot;
        st.next_slot = st.next_slot.saturating_add(1);
        st.bytes = st.bytes.saturating_add(bytes);
        st.entries.insert(fp, Slot { entry, active: false, created_at: now, bytes, seq });
        st.stats.writes = st.stats.writes.saturating_add(1);
        st.prune_entries(now);
        Ok(())
    }

    /// Makes the entry of `fingerprint` servable. An entry exists only with
    /// all its dependencies ([`CacheEntry::new`]), so this checks none; it
    /// is the point at which the decision to use it is recorded. Activating
    /// an active entry records nothing.
    pub fn activate(&self, fingerprint: &Fingerprint) -> Result<(), CacheError> {
        if !self.enabled() {
            return Err(CacheError::Disabled);
        }
        let mut st = self.lock();
        if !self.enabled() || st.disabled_queries.contains(fingerprint) {
            return Err(CacheError::Disabled);
        }
        let now = st.clock.now();
        let slot = st.entries.get_mut(fingerprint).ok_or(CacheError::NoEntry)?;
        if slot.active {
            return Ok(());
        }
        slot.active = true;
        let deps = slot.entry.dependencies;
        st.record(Some(*fingerprint), Transition::Activated, Some(&deps), now);
        Ok(())
    }

    /// The entry for `fingerprint`, if it may be used at `now`: the cache is
    /// on for the query, the entry is active, within its age, and **every
    /// dependency matches `now`** (G4). A dependency that does not match
    /// drops the entry and is named in [`Lookup::Invalidated`].
    pub fn lookup(&self, fingerprint: &Fingerprint, now: &Dependencies) -> Lookup {
        if !self.enabled() {
            return Lookup::Disabled;
        }
        let mut st = self.lock();
        if !self.enabled() || st.disabled_queries.contains(fingerprint) {
            return Lookup::Disabled;
        }
        let at = st.clock.now();
        let oldest_kept = at.0.saturating_sub(st.config.max_age_micros());
        let Some((created_at, active, deps)) =
            st.entries.get(fingerprint).map(|s| (s.created_at, s.active, s.entry.dependencies))
        else {
            return Lookup::Miss;
        };
        st.stats.reads = st.stats.reads.saturating_add(1);
        if created_at.0 < oldest_kept {
            st.remove(fingerprint, Some(DeleteReason::EvictedByAge), at);
            st.stats.evicted = st.stats.evicted.saturating_add(1);
            return Lookup::Miss;
        }
        if !active {
            return Lookup::Inactive;
        }
        if let Some(dependency) = deps.first_change(now) {
            st.remove(fingerprint, None, at);
            st.record(Some(*fingerprint), Transition::Invalidated { dependency }, Some(&deps), at);
            st.stats.invalidations = st.stats.invalidations.saturating_add(1);
            return Lookup::Invalidated(dependency);
        }
        let Some(result) = st.entries.get(fingerprint).map(|s| s.entry.result.clone()) else {
            return Lookup::Miss;
        };
        st.record(Some(*fingerprint), Transition::Used, Some(&deps), at);
        st.stats.hits = st.stats.hits.saturating_add(1);
        Lookup::Hit(result)
    }

    /// Records that a query fell back to the reference path for `cause`. A
    /// [`Discrepancy`](CacheFallbackCause::Discrepancy) also drops the entry:
    /// it is no longer trusted. Records nothing while the cache is off.
    pub fn fallback(&self, fingerprint: &Fingerprint, cause: CacheFallbackCause) {
        if !self.enabled() {
            return;
        }
        let mut st = self.lock();
        if !self.enabled() {
            return;
        }
        let now = st.clock.now();
        let deps = st.entries.get(fingerprint).map(|s| s.entry.dependencies);
        st.record(Some(*fingerprint), Transition::Fallback { cause }, deps.as_ref(), now);
        st.stats.fallbacks = st.stats.fallbacks.saturating_add(1);
        if cause == CacheFallbackCause::Discrepancy {
            st.remove(fingerprint, Some(DeleteReason::Discrepancy), now);
        }
    }

    /// Deletes the entry of `fingerprint`; whether there was one.
    pub fn delete(&self, fingerprint: &Fingerprint) -> bool {
        let mut st = self.lock();
        let now = st.clock.now();
        st.remove(fingerprint, Some(DeleteReason::Requested), now)
    }

    /// Deletes all learned state: every entry, the log and the counters. The
    /// switches stay. One [`Transition::Cleared`] record is left, so the
    /// deletion itself is on the record. Data is not affected (I-34).
    pub fn clear(&self) {
        let mut st = self.lock();
        let entries = st.entries.len() as u64;
        let now = st.clock.now();
        st.entries.clear();
        st.bytes = 0;
        st.log.clear();
        st.stats = CacheStats::default();
        st.record(None, Transition::Cleared { entries }, None, now);
    }

    /// The counters, with the state of the bounds.
    pub fn stats(&self) -> CacheStats {
        let st = self.lock();
        CacheStats { enabled: self.enabled(), entries: st.entries.len(), bytes: st.bytes, ..st.stats }
    }

    /// The transition log, oldest first, after dropping what is past its age.
    pub fn records(&self) -> Vec<TransitionRecord> {
        let mut st = self.lock();
        let now = st.clock.now();
        st.prune_log(now);
        st.log.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::fingerprint::{FingerprintKey, NormalizedQuery};
    use crate::index::UnitFilter;

    const T0: Timestamp = Timestamp(1_000_000);

    /// A fingerprint per name: the shape ignores literals, so the names differ
    /// by key (fingerprints are comparable only under one key).
    fn fp(name: &str) -> Fingerprint {
        let key = FingerprintKey::from_bytes([name.as_bytes()[0]; 32]);
        NormalizedQuery::of_filter(&UnitFilter { unit_type: Some("literal".into()), schema_version: None })
            .fingerprint(&key)
    }

    /// A different shape: filtering by schema version only.
    fn fp_other() -> Fingerprint {
        let key = FingerprintKey::from_bytes([5; 32]);
        NormalizedQuery::of_filter(&UnitFilter { unit_type: None, schema_version: Some(1) }).fingerprint(&key)
    }

    fn ready(watermark: u64) -> Option<IndexStatus> {
        Some(IndexStatus { state: IndexState::Ready, watermark: Some(CommitSeq(watermark)) })
    }

    fn deps(seq: u64) -> Dependencies {
        Dependencies::current(CommitSeq(seq), ready(seq))
    }

    fn evidence() -> Evidence {
        Evidence { observations: 3, window: Duration::from_secs(60) }
    }

    fn entry_for(f: Fingerprint, d: Dependencies) -> CacheEntry {
        CacheEntry::new(f, d, CachedResult::Units(Vec::new()), evidence()).unwrap()
    }

    fn entry(d: Dependencies) -> CacheEntry {
        entry_for(fp("t"), d)
    }

    /// A cache that is on, with a clock the test steps.
    fn on() -> (ResultCache, Arc<FixedClock>) {
        let c = ResultCache::new();
        let clock = Arc::new(FixedClock::new(T0));
        c.set_clock(clock.clone());
        c.set_enabled(true);
        (c, clock)
    }

    /// An active entry for `fp("t")` read at `deps(10)`.
    fn active() -> (ResultCache, Arc<FixedClock>) {
        let (c, clock) = on();
        c.create(entry(deps(10))).unwrap();
        c.activate(&fp("t")).unwrap();
        (c, clock)
    }

    fn kinds(c: &ResultCache) -> Vec<Transition> {
        c.records().into_iter().map(|r| r.transition).collect()
    }

    #[test]
    fn dependency_vectors() {
        // Pins the set, the order, the names and the versions: changing any
        // of them is a change of the optimization, not a refactor.
        assert_eq!(OPTIMIZATION_VERSION, 1);
        assert_eq!(LAYER_VERSION, 1);
        assert_eq!(PLANNER_VERSION, 1);
        let names: Vec<&str> = Dependency::ALL.iter().map(|d| d.name()).collect();
        assert_eq!(names, ["schema_version", "planner_version", "index_state", "index_watermark", "commit_seq"]);

        let index = Some(IndexStatus { state: IndexState::Stale, watermark: Some(CommitSeq(7)) });
        let d = Dependencies::current(CommitSeq(9), index);
        let versions: Vec<Option<u64>> = Dependency::ALL.iter().map(|k| d.version_of(*k)).collect();
        assert_eq!(versions, [Some(1), Some(1), Some(2), Some(7), Some(9)]);

        // A relation query records no index: two versions do not apply.
        let r = Dependencies::current(CommitSeq(9), None);
        let versions: Vec<Option<u64>> = Dependency::ALL.iter().map(|k| r.version_of(*k)).collect();
        assert_eq!(versions, [Some(1), Some(1), None, None, Some(9)]);

        // An index that never built has a state and no watermark.
        let never =
            Dependencies::current(CommitSeq(1), Some(IndexStatus { state: IndexState::Failed, watermark: None }));
        let versions: Vec<Option<u64>> = Dependency::ALL.iter().map(|k| never.version_of(*k)).collect();
        assert_eq!(versions, [Some(1), Some(1), Some(3), None, Some(1)]);

        // The state numbers are pinned one by one.
        let state = |s| Dependencies::current(CommitSeq(1), Some(IndexStatus { state: s, watermark: None }));
        let numbers: Vec<Option<u64>> =
            [IndexState::Ready, IndexState::Building, IndexState::Stale, IndexState::Failed]
                .into_iter()
                .map(|s| state(s).version_of(Dependency::IndexState))
                .collect();
        assert_eq!(numbers, [Some(0), Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn first_change_vectors() {
        let base = deps(10);
        let cases = [
            (base, None),
            (Dependencies { commit_seq: CommitSeq(11), ..base }, Some(Dependency::CommitSeq)),
            (Dependencies { commit_seq: CommitSeq(9), ..base }, Some(Dependency::CommitSeq)),
            (Dependencies { schema_version: 2, ..base }, Some(Dependency::SchemaVersion)),
            (Dependencies { planner_version: 2, ..base }, Some(Dependency::PlannerVersion)),
            (Dependencies { index: ready(11), ..base }, Some(Dependency::IndexWatermark)),
            (
                Dependencies {
                    index: Some(IndexStatus { state: IndexState::Stale, watermark: Some(CommitSeq(10)) }),
                    ..base
                },
                Some(Dependency::IndexState),
            ),
            // The index disappears (a relation query's status against a unit entry).
            (Dependencies { index: None, ..base }, Some(Dependency::IndexState)),
            // Several changes: the first in the checking order is named.
            (
                Dependencies { commit_seq: CommitSeq(11), schema_version: 2, planner_version: 2, ..base },
                Some(Dependency::SchemaVersion),
            ),
            (Dependencies { commit_seq: CommitSeq(11), planner_version: 2, ..base }, Some(Dependency::PlannerVersion)),
        ];
        for (now, want) in cases {
            assert_eq!(base.first_change(&now), want, "{now:?}");
        }
    }

    /// I-35: one case per declared dependency. The `match` is exhaustive on
    /// purpose: a dependency added to the enum does not compile until it has
    /// a case here.
    #[test]
    fn each_declared_dependency_invalidates_the_entry_and_is_named() {
        for dependency in Dependency::ALL {
            let base = deps(10);
            let now = match dependency {
                Dependency::SchemaVersion => Dependencies { schema_version: base.schema_version + 1, ..base },
                Dependency::PlannerVersion => Dependencies { planner_version: base.planner_version + 1, ..base },
                Dependency::IndexState => Dependencies {
                    index: Some(IndexStatus { state: IndexState::Stale, watermark: Some(CommitSeq(10)) }),
                    ..base
                },
                Dependency::IndexWatermark => Dependencies { index: ready(11), ..base },
                Dependency::CommitSeq => Dependencies { commit_seq: CommitSeq(11), ..base },
            };
            let (c, _) = active();
            // Unchanged dependencies: served.
            assert!(matches!(c.lookup(&fp("t"), &base), Lookup::Hit(_)), "{dependency}: control");
            // Exactly this one changed: not served, and named.
            match c.lookup(&fp("t"), &now) {
                Lookup::Invalidated(d) => assert_eq!(d, dependency, "{dependency}"),
                other => panic!("{dependency}: expected Invalidated, got {other:?}"),
            }
            // The entry is gone: the old dependencies do not bring it back.
            assert!(matches!(c.lookup(&fp("t"), &base), Lookup::Miss), "{dependency}: dropped");
            let last = c.records().into_iter().rev().find(|r| matches!(r.transition, Transition::Invalidated { .. }));
            let last = last.unwrap_or_else(|| panic!("{dependency}: no Invalidated record"));
            assert_eq!(last.transition, Transition::Invalidated { dependency });
            assert_eq!(c.stats().invalidations, 1);
        }
    }

    #[test]
    fn an_entry_never_serves_another_commit_seq() {
        for seq in [0, 1, 9, 11, 12, u64::MAX] {
            // A fresh entry per probe: a mismatch drops it.
            let (c, _) = active();
            let now = Dependencies { commit_seq: CommitSeq(seq), ..deps(10) };
            assert!(!matches!(c.lookup(&fp("t"), &now), Lookup::Hit(_)), "served seq {seq}");
        }
        let (c, _) = active();
        assert!(matches!(c.lookup(&fp("t"), &deps(10)), Lookup::Hit(_)), "control: its own seq is served");
    }

    #[test]
    fn an_entry_exists_only_with_its_dependencies() {
        let units = || CachedResult::Units(Vec::new());
        let relations = || CachedResult::Relations(Vec::new());
        let rel_fp = {
            let key = FingerprintKey::from_bytes([5; 32]);
            NormalizedQuery::of_relations(&crate::fingerprint::RelationQuery {
                unit: crate::types::UnitId([1; 16]),
                direction: crate::types::Direction::Out,
                relation_type: None,
            })
            .fingerprint(&key)
        };
        // A unit query without an index status, or a relation query with one.
        assert_eq!(
            CacheEntry::new(fp("t"), Dependencies::current(CommitSeq(1), None), units(), evidence()),
            Err(CacheError::IndexStatus { kind: QueryKind::Units })
        );
        assert_eq!(
            CacheEntry::new(rel_fp, deps(1), relations(), evidence()),
            Err(CacheError::IndexStatus { kind: QueryKind::Relations })
        );
        assert!(CacheEntry::new(rel_fp, Dependencies::current(CommitSeq(1), None), relations(), evidence()).is_ok());
        // A result of the other kind.
        assert_eq!(CacheEntry::new(fp("t"), deps(1), relations(), evidence()), Err(CacheError::KindMismatch));
        // No evidence.
        let none = Evidence { observations: 0, window: Duration::ZERO };
        assert_eq!(CacheEntry::new(fp("t"), deps(1), units(), none), Err(CacheError::NoEvidence));
    }

    #[test]
    fn an_entry_is_not_served_until_activated() {
        let (c, _) = on();
        c.create(entry(deps(10))).unwrap();
        assert!(matches!(c.lookup(&fp("t"), &deps(10)), Lookup::Inactive));
        assert_eq!(c.activate(&fp_other()), Err(CacheError::NoEntry));
        c.activate(&fp("t")).unwrap();
        assert!(matches!(c.lookup(&fp("t"), &deps(10)), Lookup::Hit(_)));
        // Activating twice records once.
        c.activate(&fp("t")).unwrap();
        let activations = kinds(&c).iter().filter(|t| **t == Transition::Activated).count();
        assert_eq!(activations, 1);
    }

    #[test]
    fn disabled_stores_nothing_and_reads_nothing() {
        let c = ResultCache::new();
        assert!(!c.enabled(), "off by default");
        let now = deps(10);
        assert_eq!(c.create(entry(now)), Err(CacheError::Disabled));
        assert_eq!(c.activate(&fp("t")), Err(CacheError::Disabled));
        assert!(matches!(c.lookup(&fp("t"), &now), Lookup::Disabled));
        c.fallback(&fp("t"), CacheFallbackCause::Disabled);
        let s = c.stats();
        assert_eq!((s.enabled, s.entries, s.bytes, s.reads, s.writes, s.hits), (false, 0, 0, 0, 0, 0));
        assert!(c.records().is_empty(), "off: no record either");

        // On, with an entry; then off again: the entry is gone and is not read.
        c.set_enabled(true);
        c.create(entry(now)).unwrap();
        c.activate(&fp("t")).unwrap();
        c.set_enabled(false);
        assert_eq!(c.stats().entries, 0);
        let reads = c.stats().reads;
        assert!(matches!(c.lookup(&fp("t"), &now), Lookup::Disabled));
        assert_eq!(c.stats().reads, reads, "a disabled lookup reads no entry");
        assert_eq!(c.create(entry(now)), Err(CacheError::Disabled));
        assert_eq!(c.stats().entries, 0);
    }

    #[test]
    fn the_per_query_switch_stops_one_query_and_drops_its_entry() {
        let (c, _) = on();
        let other = entry_for(fp_other(), deps(10));
        c.create(entry(deps(10))).unwrap();
        c.create(other).unwrap();
        c.activate(&fp("t")).unwrap();
        c.activate(&fp_other()).unwrap();

        c.disable_query(fp("t"));
        assert!(!c.query_enabled(&fp("t")));
        assert!(c.query_enabled(&fp_other()));
        assert_eq!(c.stats().entries, 1);
        assert!(matches!(c.lookup(&fp("t"), &deps(10)), Lookup::Disabled));
        assert_eq!(c.create(entry(deps(10))), Err(CacheError::Disabled));
        assert_eq!(c.activate(&fp("t")), Err(CacheError::Disabled));
        // The other query is not affected.
        assert!(matches!(c.lookup(&fp_other(), &deps(10)), Lookup::Hit(_)));
        // And it comes back.
        c.enable_query(fp("t"));
        c.create(entry(deps(10))).unwrap();
        assert_eq!(c.stats().entries, 2);
    }

    #[test]
    fn retention_is_bounded_by_the_number_of_entries() {
        let (c, _) = on();
        c.set_config(CacheConfig { max_entries: 2, ..CacheConfig::default() }).unwrap();
        let fps = [fp("a"), fp("b"), fp("c")];
        for f in fps {
            c.create(entry_for(f, deps(10))).unwrap();
            c.activate(&f).unwrap();
        }
        let s = c.stats();
        assert_eq!((s.entries, s.evicted), (2, 1));
        // The oldest went: its lookup misses, the others hit.
        assert!(matches!(c.lookup(&fps[0], &deps(10)), Lookup::Miss));
        assert!(matches!(c.lookup(&fps[1], &deps(10)), Lookup::Hit(_)));
        assert!(matches!(c.lookup(&fps[2], &deps(10)), Lookup::Hit(_)));
        assert!(kinds(&c).contains(&Transition::Deleted { reason: DeleteReason::EvictedBySize }));
        // Shrinking the bound drops now.
        c.set_config(CacheConfig { max_entries: 1, ..CacheConfig::default() }).unwrap();
        assert_eq!(c.stats().entries, 1);
        assert!(matches!(c.lookup(&fps[2], &deps(10)), Lookup::Hit(_)));
    }

    fn unit_of(len: usize) -> Unit {
        Unit {
            id: crate::types::UnitId([1; 16]),
            unit_type: "t".into(),
            schema_version: 1,
            lifecycle: crate::types::Lifecycle::Active,
            head: crate::types::RevNo(1),
            oldest: crate::types::RevNo(1),
            created_seq: CommitSeq(1),
            updated_seq: CommitSeq(1),
            created_at: T0,
            updated_at: T0,
            metadata: Vec::new(),
            payload: Some(vec![0; len]),
            content_hash: crate::payload::ContentHash::blake3(0, &vec![0; len]),
        }
    }

    #[test]
    fn retention_is_bounded_by_bytes() {
        let (c, _) = on();
        let big = CachedResult::Units(vec![unit_of(1000)]);
        let one = big.approx_bytes();
        assert!(one >= 1000);
        c.set_config(CacheConfig { max_bytes: one * 2 + one / 2, ..CacheConfig::default() }).unwrap();
        for t in ["a", "b", "c"] {
            let e = CacheEntry::new(fp(t), deps(10), big.clone(), evidence()).unwrap();
            c.create(e).unwrap();
        }
        let s = c.stats();
        assert_eq!((s.entries, s.bytes, s.evicted), (2, one * 2, 1));
        // A result larger than the whole bound is refused, and nothing changes.
        c.set_config(CacheConfig { max_bytes: one - 1, ..CacheConfig::default() }).unwrap();
        assert_eq!(c.stats().entries, 0, "shrinking the bound drops what no longer fits");
        let e = CacheEntry::new(fp("z"), deps(10), big, evidence()).unwrap();
        assert_eq!(c.create(e), Err(CacheError::TooLarge));
        assert_eq!(c.stats().entries, 0);
    }

    #[test]
    fn retention_is_bounded_by_age() {
        let (c, clock) = on();
        c.set_config(CacheConfig { max_age: Duration::from_micros(100), ..CacheConfig::default() }).unwrap();
        c.create(entry_for(fp("a"), deps(10))).unwrap();
        c.activate(&fp("a")).unwrap();
        clock.set(Timestamp(T0.0 + 99));
        assert!(matches!(c.lookup(&fp("a"), &deps(10)), Lookup::Hit(_)), "within its age");
        // Past its age: not served, dropped, and counted.
        clock.set(Timestamp(T0.0 + 101));
        assert!(matches!(c.lookup(&fp("a"), &deps(10)), Lookup::Miss));
        assert_eq!(c.stats().entries, 0);
        assert!(kinds(&c).contains(&Transition::Deleted { reason: DeleteReason::EvictedByAge }));
        // Creating drops the entries that aged without being looked up.
        c.create(entry_for(fp("b"), deps(10))).unwrap();
        clock.set(Timestamp(T0.0 + 300));
        c.create(entry_for(fp("c"), deps(10))).unwrap();
        assert_eq!(c.stats().entries, 1);
        assert!(matches!(c.lookup(&fp("b"), &deps(10)), Lookup::Miss));
    }

    #[test]
    fn the_log_is_bounded_by_size_and_age() {
        let (c, clock) = on();
        c.set_config(CacheConfig { max_log_records: 3, max_age: Duration::from_micros(100), ..CacheConfig::default() })
            .unwrap();
        c.create(entry(deps(10))).unwrap();
        c.activate(&fp("t")).unwrap();
        for _ in 0..4 {
            c.lookup(&fp("t"), &deps(10));
        }
        let records = c.records();
        assert_eq!(records.len(), 3, "at most max_log_records");
        assert!(records.iter().all(|r| r.transition == Transition::Used));
        assert!(c.stats().log_evicted >= 4);
        // Past the age, the log empties on read.
        clock.set(Timestamp(T0.0 + 1000));
        assert!(c.records().is_empty());
    }

    #[test]
    fn a_config_that_keeps_nothing_is_refused() {
        let c = ResultCache::new();
        let d = CacheConfig::default();
        let bad = [
            CacheConfig { max_entries: 0, ..d },
            CacheConfig { max_bytes: 0, ..d },
            CacheConfig { max_log_records: 0, ..d },
            CacheConfig { max_age: Duration::ZERO, ..d },
        ];
        for config in bad {
            assert!(matches!(c.set_config(config), Err(UnitError::InvalidArgument(_))), "{config:?}");
        }
    }

    /// G7: every transition, driven through the store's API, finds its record.
    #[test]
    fn every_transition_leaves_a_record() {
        let (c, _) = on();
        let f = fp("t");
        let g = fp_other();
        let ev = evidence();

        c.create(entry(deps(10))).unwrap(); // Created (with its evidence)
        c.activate(&f).unwrap(); // Activated
        c.lookup(&f, &deps(10)); // Used
        c.fallback(&f, CacheFallbackCause::PreconditionFailed); // Fallback with each cause
        c.fallback(&f, CacheFallbackCause::Invalidated);
        c.fallback(&f, CacheFallbackCause::Disabled);
        c.lookup(&f, &Dependencies { commit_seq: CommitSeq(11), ..deps(10) }); // Invalidated (the commit_seq)
        c.create(entry_for(g, deps(10))).unwrap();
        c.activate(&g).unwrap();
        c.fallback(&g, CacheFallbackCause::Discrepancy); // Fallback, then Deleted(Discrepancy)
        c.create(entry(deps(10))).unwrap();
        c.create(entry(deps(10))).unwrap(); // Deleted(Replaced)
        assert!(c.delete(&f)); // Deleted(Requested)
        c.disable_query(g); // Disabled(Query)
        c.enable_query(g); // Enabled(Query)
        c.create(entry(deps(10))).unwrap();
        c.set_enabled(false); // Disabled(Global), Deleted(Disabled)
        c.set_enabled(true); // Enabled(Global)

        let want = [
            Transition::Created { evidence: ev },
            Transition::Activated,
            Transition::Used,
            Transition::Fallback { cause: CacheFallbackCause::PreconditionFailed },
            Transition::Fallback { cause: CacheFallbackCause::Invalidated },
            Transition::Fallback { cause: CacheFallbackCause::Disabled },
            Transition::Invalidated { dependency: Dependency::CommitSeq },
            Transition::Created { evidence: ev },
            Transition::Activated,
            Transition::Fallback { cause: CacheFallbackCause::Discrepancy },
            Transition::Deleted { reason: DeleteReason::Discrepancy },
            Transition::Created { evidence: ev },
            Transition::Deleted { reason: DeleteReason::Replaced },
            Transition::Created { evidence: ev },
            Transition::Deleted { reason: DeleteReason::Requested },
            Transition::Disabled { switch: Switch::Query },
            Transition::Enabled { switch: Switch::Query },
            Transition::Created { evidence: ev },
            Transition::Disabled { switch: Switch::Global },
            Transition::Deleted { reason: DeleteReason::Disabled },
            Transition::Enabled { switch: Switch::Global },
        ];
        // `Enabled(Global)` for the first `set_enabled(true)` of `on()` opens the log.
        let mut got = kinds(&c);
        assert_eq!(got.remove(0), Transition::Enabled { switch: Switch::Global });
        assert_eq!(got, want);

        // Deleted by size and by age have their records in the bounds tests;
        // the wipe is on the record too.
        c.clear();
        assert_eq!(kinds(&c), [Transition::Cleared { entries: 0 }]);
    }

    #[test]
    fn records_carry_versions_and_the_fingerprint_and_nothing_else() {
        let (c, _) = active();
        c.lookup(&fp("t"), &deps(10));
        let records = c.records();
        assert!(records.len() >= 4);
        for r in &records {
            assert_eq!(r.optimization_version, OPTIMIZATION_VERSION);
            assert_eq!(r.planner_version, PLANNER_VERSION);
        }
        let used = records.iter().find(|r| r.transition == Transition::Used).unwrap();
        assert_eq!((used.fingerprint, used.commit_seq), (Some(fp("t")), Some(CommitSeq(10))));
        // Ids increase.
        assert!(records.windows(2).all(|w| w[0].id < w[1].id));
        // The global switch has no fingerprint and no sequence.
        let global = records.iter().find(|r| r.transition == Transition::Enabled { switch: Switch::Global }).unwrap();
        assert_eq!((global.fingerprint, global.commit_seq), (None, None));

        // The planner version of an entry built under another planner is kept.
        let old = Dependencies { planner_version: 0, ..deps(10) };
        let (c, _) = on();
        c.create(entry(old)).unwrap();
        let created = c.records().into_iter().find(|r| matches!(r.transition, Transition::Created { .. })).unwrap();
        assert_eq!(created.planner_version, 0);
    }

    #[test]
    fn clear_deletes_all_state_and_keeps_the_switches() {
        let (c, _) = active();
        c.disable_query(fp_other());
        c.lookup(&fp("t"), &deps(10));
        c.clear();
        let s = c.stats();
        assert_eq!((s.entries, s.bytes, s.reads, s.writes, s.hits, s.evicted), (0, 0, 0, 0, 0, 0));
        assert_eq!(kinds(&c), [Transition::Cleared { entries: 1 }]);
        assert!(matches!(c.lookup(&fp("t"), &deps(10)), Lookup::Miss));
        // The switches are not learned state.
        assert!(c.enabled());
        assert!(!c.query_enabled(&fp_other()));
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let (c, _) = active();
        let c = Arc::new(c);
        let c2 = c.clone();
        let _ = std::thread::spawn(move || {
            let _held = c2.lock();
            panic!("poison the cache");
        })
        .join();
        assert!(matches!(c.lookup(&fp("t"), &deps(10)), Lookup::Hit(_)));
    }

    #[test]
    fn a_hit_shares_the_result_without_copying_it() {
        let (c, _) = on();
        let e = CacheEntry::new(fp("t"), deps(10), CachedResult::Units(vec![unit_of(10)]), evidence()).unwrap();
        c.create(e).unwrap();
        c.activate(&fp("t")).unwrap();
        let (Lookup::Hit(a), Lookup::Hit(b)) = (c.lookup(&fp("t"), &deps(10)), c.lookup(&fp("t"), &deps(10))) else {
            panic!("expected two hits");
        };
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.kind(), QueryKind::Units);
    }
}
