//! E5-03: `ExecutionObservation` (etapa5-acta.md §3, adaptive-execution.md §2,
//! ADR-0015).
//!
//! An optional, in-memory record of how queries ran, kept by the [`Observer`]
//! that every [`UnitDb`](crate::UnitDb) owns and every
//! [`UnitSnapshot`](crate::UnitSnapshot) it hands out shares. `query`,
//! `query_relations` and `assoc_query` report to it; nothing else does.
//!
//! # Levels
//!
//! | Level | Keeps |
//! |---|---|
//! | [`Off`](ObservationLevel::Off) (default) | Nothing. A query pays one atomic load. |
//! | [`Aggregate`](ObservationLevel::Aggregate) | Counters ([`ObservationStats`]): no per-execution record, no fingerprint, no text. |
//! | [`Fingerprint`](ObservationLevel::Fingerprint) | The counters and one [`ExecutionObservation`] per execution, carrying the keyed [`Fingerprint`] of the query shape. |
//! | [`Diagnostic`](ObservationLevel::Diagnostic) | The same, plus the shape text (`fingerprint_text`). |
//!
//! The default is `Off` (acta §8, decision 2). No level is inferred, parsed or
//! raised by any other setting: `Diagnostic` is in force only after a caller
//! passes it to [`Observer::set_level`]. Lowering the level takes effect on
//! what is already kept: below `Diagnostic` the retained shape texts are
//! dropped, below `Fingerprint` the retained records are.
//!
//! # What a record cannot hold
//!
//! An [`ExecutionObservation`] has no field for a filter, a relation query, a
//! unit, a payload, a key or an error message: `Observed` is read to build
//! the shape and is never stored. Its error is a class ([`ErrorKind`]), its
//! fallback is the planner's [`Fallback`] (paths and causes, no text). The
//! shape text is the one string, and only at `Diagnostic` (I-37; the sentinel
//! test over every level is E5-04).
//!
//! # Recording never fails and never blocks the query
//!
//! The query's result is computed first and returned untouched; recording
//! comes after it and cannot change it. The record path takes the observer's
//! lock with `try_lock`: if another thread holds it, the observation is
//! **lost and counted** ([`ObservationStats::dropped`]) rather than waited
//! for. A poisoned lock is recovered (its state is counters and a queue, which
//! a panic cannot leave inconsistent). A missing key counts as a loss too.
//!
//! # Retention
//!
//! In memory only (acta §8, decision 3), bounded by size and by age
//! ([`ObservationConfig`]): at most `max_records` records, and none recorded
//! more than `max_age` before the newest. Both are enforced when a record is
//! added and when records are read. The counters are fixed-size.
//!
//! ponytail: age is checked at the front of the queue only, so a clock that
//! goes backwards can leave one old record behind its newer neighbours until
//! the size bound pushes it out. Upgrade path: scan the queue on read.
//!
//! # Observing does not touch the data (I-34, observation part)
//!
//! The observer never writes to the engine, never opens a transaction and
//! never reads the layer's [`Clock`](crate::Clock) (a stepping test clock would
//! change the timestamps of later commits): it has its own, [`SystemClock`]
//! by default, replaceable with [`Observer::set_clock`]. `tests/observation.rs`
//! runs the same commits and reads with `Off` and with `Diagnostic` and
//! compares the answers, the commit sequences and the engine's contents.
//!
//! # Fields not recorded
//!
//! `rows_examined` and `bytes_read` of adaptive-execution.md §2 are not
//! applicable yet: no scan path counts them, and counting means instrumenting
//! the hot loops of the paths and the engine. `cache` and `learned` belong to
//! Stage 6.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use crate::assoc::AssocQuery;
use crate::clock::{Clock, SystemClock};
use crate::error::UnitError;
use crate::fingerprint::{Fingerprint, FingerprintKey, NormalizedQuery, RelationQuery};
use crate::index::UnitFilter;
use crate::plan::PlanPath;
use crate::planner::Fallback;
use crate::types::{CommitSeq, Timestamp};

/// How much a query leaves behind; see the [module documentation](self).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObservationLevel {
    #[default]
    Off = 0,
    Aggregate = 1,
    Fingerprint = 2,
    Diagnostic = 3,
}

impl ObservationLevel {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => ObservationLevel::Aggregate,
            2 => ObservationLevel::Fingerprint,
            3 => ObservationLevel::Diagnostic,
            _ => ObservationLevel::Off,
        }
    }
}

/// Default of [`ObservationConfig::max_records`]: a record is a few hundred
/// bytes, so this is a fraction of a megabyte.
pub const DEFAULT_MAX_RECORDS: usize = 1024;

/// Default of [`ObservationConfig::max_age`].
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(3600);

/// Retention of the per-execution records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationConfig {
    /// Most records kept; the oldest go first. At least 1.
    pub max_records: usize,
    /// Longest a record is kept, counted from its `recorded_at` to the newest
    /// one. More than zero.
    pub max_age: Duration,
}

impl Default for ObservationConfig {
    fn default() -> Self {
        ObservationConfig { max_records: DEFAULT_MAX_RECORDS, max_age: DEFAULT_MAX_AGE }
    }
}

impl ObservationConfig {
    fn validate(&self) -> Result<(), UnitError> {
        if self.max_records == 0 {
            return Err(UnitError::InvalidArgument("observation max_records must be >= 1".into()));
        }
        if self.max_age.is_zero() {
            return Err(UnitError::InvalidArgument("observation max_age must be > 0".into()));
        }
        Ok(())
    }

    fn max_age_micros(&self) -> i64 {
        i64::try_from(self.max_age.as_micros()).unwrap_or(i64::MAX)
    }
}

/// The class of a failed query: never its message, which can quote a key or
/// a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The stored bytes are not what the layer writes.
    Corruption,
    /// Any other engine error.
    Storage,
    /// The call was refused as malformed.
    Invalid,
    Other,
}

impl ErrorKind {
    pub(crate) fn of(e: &UnitError) -> Self {
        match e {
            UnitError::Storage(kerno_core::Error::Corruption(_)) => ErrorKind::Corruption,
            UnitError::Storage(_) => ErrorKind::Storage,
            UnitError::InvalidArgument(_) => ErrorKind::Invalid,
            _ => ErrorKind::Other,
        }
    }
}

/// How an observed query ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Outcome {
    Ok,
    Error(ErrorKind),
}

/// What ran: the reference path, or a path the planner chose over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanKind {
    Reference,
    Planned,
    /// Learned state: a cached result (E6-02).
    Cache,
}

impl PlanKind {
    fn of(path: PlanPath) -> Self {
        match path {
            PlanPath::UnitsReference | PlanPath::RelationsReference | PlanPath::AssocReference => PlanKind::Reference,
            PlanPath::IndexPrimary | PlanPath::IndexType | PlanPath::Adjacency | PlanPath::AssocAdjacency => {
                PlanKind::Planned
            }
            PlanPath::ResultCache => PlanKind::Cache,
        }
    }
}

/// One observed execution, kept at `Fingerprint` and `Diagnostic`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionObservation {
    /// Local, increasing per observer.
    pub observation_id: u64,
    /// Keyed hash of the query shape (ADR-0015).
    pub fingerprint: Fingerprint,
    /// The shape text, `Some` only at `Diagnostic`.
    pub fingerprint_text: Option<String>,
    /// The path that answered; `None` when the query failed before one did.
    pub path: Option<PlanPath>,
    /// [`PLANNER_VERSION`](crate::planner::PLANNER_VERSION) at the time.
    pub plan_version: u32,
    /// The sequence of the snapshot the query read (I-04).
    pub commit_seq: CommitSeq,
    /// Monotonic time from the start of the query to its end.
    pub duration_ns: u64,
    pub rows_returned: u64,
    pub outcome: Outcome,
    /// Set when the chosen path was abandoned for the reference path.
    pub fallback: Option<Fallback>,
    /// Descriptive time of the record; it orders no commit (I-26).
    pub recorded_at: Timestamp,
}

impl ExecutionObservation {
    pub fn plan_kind(&self) -> Option<PlanKind> {
        self.path.map(PlanKind::of)
    }
}

const PATHS: [PlanPath; 8] = [
    PlanPath::UnitsReference,
    PlanPath::IndexPrimary,
    PlanPath::IndexType,
    PlanPath::RelationsReference,
    PlanPath::Adjacency,
    PlanPath::ResultCache,
    PlanPath::AssocReference,
    PlanPath::AssocAdjacency,
];

fn slot(path: PlanPath) -> usize {
    PATHS.iter().position(|p| *p == path).unwrap_or(0)
}

/// Counters of every observed execution since the last
/// [`clear`](Observer::clear), kept from `Aggregate` up. They hold no text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObservationStats {
    pub level: ObservationLevel,
    /// Executions observed, successful or not.
    pub observed: u64,
    pub errors: u64,
    /// Executions that ended in a fallback to the reference path.
    pub fallbacks: u64,
    pub rows_returned: u64,
    pub duration_ns_total: u64,
    pub duration_ns_max: u64,
    /// Observations lost because recording would have waited or had no key.
    pub dropped: u64,
    /// Records pushed out by the size or age bound.
    pub evicted: u64,
    /// Records kept now.
    pub retained: usize,
    by_path: [u64; 8],
}

impl ObservationStats {
    /// Executions that were answered by `path`.
    pub fn executed(&self, path: PlanPath) -> u64 {
        self.by_path[slot(path)]
    }
}

/// What `query` reports about its start; absent when the level is `Off`.
pub(crate) struct Started {
    level: ObservationLevel,
    at: Instant,
}

/// The query that ran, borrowed to be reduced to its shape and dropped.
pub(crate) enum Observed<'a> {
    Units(&'a UnitFilter),
    Relations(&'a RelationQuery),
    Assoc(&'a AssocQuery),
}

/// What the execution ended with.
pub(crate) struct Ended {
    pub(crate) path: PlanPath,
    pub(crate) rows: usize,
    pub(crate) fallback: Option<Fallback>,
}

struct State {
    config: ObservationConfig,
    clock: Arc<dyn Clock>,
    key: Option<FingerprintKey>,
    next_id: u64,
    records: VecDeque<ExecutionObservation>,
    stats: ObservationStats,
}

impl State {
    /// Drops what the size and age bounds no longer allow, against `now`.
    fn prune(&mut self, now: Timestamp) {
        let oldest_kept = now.0.saturating_sub(self.config.max_age_micros());
        while let Some(front) = self.records.front() {
            if self.records.len() > self.config.max_records || front.recorded_at.0 < oldest_kept {
                self.records.pop_front();
                self.stats.evicted = self.stats.evicted.saturating_add(1);
            } else {
                break;
            }
        }
    }
}

/// The observation point of a database; see the [module documentation](self).
pub struct Observer {
    level: AtomicU8,
    dropped: AtomicU64,
    state: Mutex<State>,
}

impl Default for Observer {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer {
    /// An observer at `Off`, with the default retention and the system clock.
    pub fn new() -> Self {
        Observer {
            level: AtomicU8::new(ObservationLevel::Off as u8),
            dropped: AtomicU64::new(0),
            state: Mutex::new(State {
                config: ObservationConfig::default(),
                clock: Arc::new(SystemClock),
                key: None,
                next_id: 1,
                records: VecDeque::new(),
                stats: ObservationStats::default(),
            }),
        }
    }

    pub fn level(&self) -> ObservationLevel {
        ObservationLevel::from_u8(self.level.load(Ordering::Relaxed))
    }

    /// Admin calls wait for the lock; only recording never does.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets the level. From `Fingerprint` up the observer needs its local
    /// key, generated here on first use; if the system cannot give one, the
    /// error is returned and the level does not change.
    pub fn set_level(&self, level: ObservationLevel) -> std::io::Result<()> {
        let mut st = self.lock();
        if level >= ObservationLevel::Fingerprint && st.key.is_none() {
            st.key = Some(FingerprintKey::generate()?);
        }
        if level < ObservationLevel::Diagnostic {
            st.records.iter_mut().for_each(|r| r.fingerprint_text = None);
        }
        if level < ObservationLevel::Fingerprint {
            st.records.clear();
        }
        self.level.store(level as u8, Ordering::SeqCst);
        Ok(())
    }

    /// Replaces the retention bounds; records beyond them are dropped now.
    pub fn set_config(&self, config: ObservationConfig) -> Result<(), UnitError> {
        config.validate()?;
        let mut st = self.lock();
        st.config = config;
        let now = st.clock.now();
        st.prune(now);
        Ok(())
    }

    /// Replaces the observer's clock (the default is [`SystemClock`]). It is
    /// the only time source the observer reads, and it is read while the
    /// observer's lock is held, so it must not block.
    pub fn set_clock(&self, clock: Arc<dyn Clock>) {
        self.lock().clock = clock;
    }

    /// Replaces the fingerprint key, for reproducible tests. Fingerprints are
    /// comparable only under one key, so the retained records are dropped.
    pub fn set_key(&self, key: FingerprintKey) {
        let mut st = self.lock();
        st.key = Some(key);
        st.records.clear();
    }

    /// The counters, with the state of the retention.
    pub fn stats(&self) -> ObservationStats {
        let st = self.lock();
        ObservationStats {
            level: self.level(),
            dropped: self.dropped.load(Ordering::Relaxed),
            retained: st.records.len(),
            ..st.stats.clone()
        }
    }

    /// The retained records, oldest first, after dropping those past the age
    /// bound.
    pub fn records(&self) -> Vec<ExecutionObservation> {
        let mut st = self.lock();
        let now = st.clock.now();
        st.prune(now);
        st.records.iter().cloned().collect()
    }

    /// Forgets every record and every counter. Data is not affected (I-34).
    pub fn clear(&self) {
        let mut st = self.lock();
        st.records.clear();
        st.stats = ObservationStats::default();
        self.dropped.store(0, Ordering::Relaxed);
    }

    /// Called by a query before it runs: `None` at `Off`, which is all an
    /// unobserved query ever costs.
    pub(crate) fn begin(&self) -> Option<Started> {
        let level = self.level();
        (level != ObservationLevel::Off).then(|| Started { level, at: Instant::now() })
    }

    /// Called by a query after it ran, with its result already decided.
    /// Returns nothing and cannot fail: see the module documentation.
    pub(crate) fn finish(
        &self,
        started: Started,
        query: Observed<'_>,
        commit_seq: CommitSeq,
        result: Result<Ended, ErrorKind>,
    ) {
        let duration_ns = u64::try_from(started.at.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let mut st = match self.state.try_lock() {
            Ok(g) => g,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        // The level may have been lowered while the query ran: never record
        // more than is allowed now.
        let level = started.level.min(self.level());
        if level == ObservationLevel::Off {
            return;
        }
        let stats = &mut st.stats;
        stats.observed = stats.observed.saturating_add(1);
        stats.duration_ns_total = stats.duration_ns_total.saturating_add(duration_ns);
        stats.duration_ns_max = stats.duration_ns_max.max(duration_ns);
        let (path, rows, fallback, outcome) = match result {
            Ok(Ended { path, rows, fallback }) => {
                let rows = rows as u64;
                stats.rows_returned = stats.rows_returned.saturating_add(rows);
                stats.fallbacks = stats.fallbacks.saturating_add(u64::from(fallback.is_some()));
                let n = &mut stats.by_path[slot(path)];
                *n = n.saturating_add(1);
                (Some(path), rows, fallback, Outcome::Ok)
            }
            Err(kind) => {
                stats.errors = stats.errors.saturating_add(1);
                (None, 0, None, Outcome::Error(kind))
            }
        };
        if level < ObservationLevel::Fingerprint {
            return;
        }
        let Some(key) = &st.key else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let shape = match query {
            Observed::Units(f) => NormalizedQuery::of_filter(f),
            Observed::Relations(q) => NormalizedQuery::of_relations(q),
            Observed::Assoc(q) => NormalizedQuery::of_assoc(q),
        };
        let now = st.clock.now();
        let record = ExecutionObservation {
            observation_id: st.next_id,
            fingerprint: shape.fingerprint(key),
            fingerprint_text: (level == ObservationLevel::Diagnostic).then(|| shape.text().to_owned()),
            path,
            plan_version: crate::planner::PLANNER_VERSION,
            commit_seq,
            duration_ns,
            rows_returned: rows,
            outcome,
            fallback,
            recorded_at: now,
        };
        st.next_id = st.next_id.saturating_add(1);
        st.records.push_back(record);
        st.prune(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::FixedClock;
    use crate::fingerprint::QueryKind;

    fn filter(t: &str) -> UnitFilter {
        UnitFilter { unit_type: Some(t.into()), schema_version: None }
    }

    fn ok(path: PlanPath) -> Result<Ended, ErrorKind> {
        Ok(Ended { path, rows: 2, fallback: None })
    }

    /// Reports one execution of `f` at `level`-independent terms.
    fn report(o: &Observer, f: &UnitFilter, result: Result<Ended, ErrorKind>) {
        if let Some(s) = o.begin() {
            o.finish(s, Observed::Units(f), CommitSeq(7), result);
        }
    }

    fn observer_at(level: ObservationLevel) -> Observer {
        let o = Observer::new();
        o.set_key(FingerprintKey::from_bytes([9; 32]));
        o.set_level(level).unwrap();
        o
    }

    #[test]
    fn default_is_off_and_off_keeps_nothing() {
        let o = Observer::new();
        assert_eq!(ObservationLevel::default(), ObservationLevel::Off);
        assert_eq!(o.level(), ObservationLevel::Off);
        assert!(o.begin().is_none());
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        assert_eq!(o.stats(), ObservationStats::default());
        assert!(o.records().is_empty());
    }

    #[test]
    fn aggregate_counts_and_keeps_no_record() {
        let o = observer_at(ObservationLevel::Aggregate);
        let f = filter("t");
        report(&o, &f, ok(PlanPath::IndexType));
        report(&o, &f, ok(PlanPath::IndexType));
        let fb = Fallback { abandoned: PlanPath::IndexType, cause: crate::planner::FallbackCause::Corrupt };
        report(&o, &f, Ok(Ended { path: PlanPath::UnitsReference, rows: 5, fallback: Some(fb) }));
        report(&o, &f, Err(ErrorKind::Storage));
        let s = o.stats();
        assert_eq!((s.observed, s.errors, s.fallbacks, s.rows_returned), (4, 1, 1, 9));
        assert_eq!(s.executed(PlanPath::IndexType), 2);
        assert_eq!(s.executed(PlanPath::UnitsReference), 1);
        assert_eq!(s.executed(PlanPath::Adjacency), 0);
        assert_eq!((s.retained, s.evicted, s.dropped), (0, 0, 0));
        assert!(o.records().is_empty());
    }

    #[test]
    fn fingerprint_records_the_hash_and_diagnostic_adds_the_text() {
        let key = FingerprintKey::from_bytes([9; 32]);
        let want = NormalizedQuery::of_filter(&filter("t")).fingerprint(&key);
        for (level, text) in
            [(ObservationLevel::Fingerprint, None), (ObservationLevel::Diagnostic, Some("units[unit_type=?text]"))]
        {
            let o = observer_at(level);
            report(&o, &filter("t"), ok(PlanPath::IndexType));
            let r = o.records();
            assert_eq!(r.len(), 1, "{level:?}");
            assert_eq!(r[0].fingerprint, want, "{level:?}");
            assert_eq!(r[0].fingerprint_text.as_deref(), text, "{level:?}");
            assert_eq!(r[0].fingerprint.kind(), QueryKind::Units);
            assert_eq!(r[0].plan_kind(), Some(PlanKind::Planned));
            assert_eq!((r[0].commit_seq, r[0].rows_returned, r[0].outcome), (CommitSeq(7), 2, Outcome::Ok));
        }
    }

    #[test]
    fn a_failed_query_is_recorded_as_a_class_without_a_path() {
        let o = observer_at(ObservationLevel::Fingerprint);
        report(&o, &filter("t"), Err(ErrorKind::Corruption));
        let r = &o.records()[0];
        assert_eq!(
            (r.outcome, r.path, r.plan_kind(), r.rows_returned),
            (Outcome::Error(ErrorKind::Corruption), None, None, 0)
        );
    }

    #[test]
    fn lowering_the_level_drops_what_it_no_longer_allows() {
        let o = observer_at(ObservationLevel::Diagnostic);
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        o.set_level(ObservationLevel::Fingerprint).unwrap();
        let r = o.records();
        assert_eq!((r.len(), r[0].fingerprint_text.clone()), (1, None));
        o.set_level(ObservationLevel::Aggregate).unwrap();
        assert!(o.records().is_empty());
        // The counters are not records: they stay.
        assert_eq!(o.stats().observed, 1);
        o.set_level(ObservationLevel::Off).unwrap();
        o.clear();
        assert_eq!(o.stats(), ObservationStats::default());
    }

    #[test]
    fn a_query_that_outlives_a_lowering_records_only_what_is_allowed_now() {
        let o = observer_at(ObservationLevel::Diagnostic);
        let started = o.begin().unwrap();
        o.set_level(ObservationLevel::Aggregate).unwrap();
        o.finish(started, Observed::Units(&filter("t")), CommitSeq(1), ok(PlanPath::IndexType));
        assert!(o.records().is_empty());
        assert_eq!(o.stats().observed, 1);
        let started = o.begin().unwrap();
        o.set_level(ObservationLevel::Off).unwrap();
        o.finish(started, Observed::Units(&filter("t")), CommitSeq(1), ok(PlanPath::IndexType));
        assert_eq!(o.stats().observed, 1);
    }

    #[test]
    fn retention_is_bounded_by_size() {
        let o = observer_at(ObservationLevel::Fingerprint);
        o.set_config(ObservationConfig { max_records: 3, ..ObservationConfig::default() }).unwrap();
        for _ in 0..5 {
            report(&o, &filter("t"), ok(PlanPath::IndexType));
        }
        let ids: Vec<u64> = o.records().iter().map(|r| r.observation_id).collect();
        assert_eq!(ids, [3, 4, 5]);
        assert_eq!((o.stats().retained, o.stats().evicted, o.stats().observed), (3, 2, 5));
        // Shrinking the bound drops now.
        o.set_config(ObservationConfig { max_records: 1, ..ObservationConfig::default() }).unwrap();
        assert_eq!(o.records().iter().map(|r| r.observation_id).collect::<Vec<_>>(), [5]);
    }

    #[test]
    fn retention_is_bounded_by_age() {
        let o = observer_at(ObservationLevel::Fingerprint);
        let clock = Arc::new(FixedClock::new(Timestamp(1_000)));
        o.set_clock(clock.clone());
        o.set_config(ObservationConfig { max_records: 10, max_age: Duration::from_micros(100) }).unwrap();
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        clock.set(Timestamp(1_050));
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        assert_eq!(o.records().len(), 2);
        // Adding a record past the age of the first drops it.
        clock.set(Timestamp(1_120));
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        assert_eq!(o.records().iter().map(|r| r.observation_id).collect::<Vec<_>>(), [2, 3]);
        // Reading past the age of every record drops them, with no new one.
        clock.set(Timestamp(9_000));
        assert!(o.records().is_empty());
        assert_eq!(o.stats().evicted, 3);
    }

    #[test]
    fn a_config_that_keeps_nothing_is_refused() {
        let o = Observer::new();
        let bad = [
            ObservationConfig { max_records: 0, ..ObservationConfig::default() },
            ObservationConfig { max_age: Duration::ZERO, ..ObservationConfig::default() },
        ];
        for config in bad {
            assert!(matches!(o.set_config(config), Err(UnitError::InvalidArgument(_))), "{config:?}");
        }
    }

    #[test]
    fn recording_that_would_wait_is_lost_and_counted() {
        let o = observer_at(ObservationLevel::Fingerprint);
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        {
            // Another thread is inside the observer.
            let _held = o.lock();
            report(&o, &filter("t"), ok(PlanPath::IndexType));
            report(&o, &filter("t"), ok(PlanPath::IndexType));
        }
        let s = o.stats();
        assert_eq!((s.dropped, s.observed, s.retained), (2, 1, 1));
        // And it records again once the lock is free.
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        assert_eq!(o.stats().observed, 2);
    }

    #[test]
    fn a_poisoned_lock_is_recovered_not_propagated() {
        let o = Arc::new(observer_at(ObservationLevel::Fingerprint));
        let o2 = o.clone();
        let _ = std::thread::spawn(move || {
            let _held = o2.lock();
            panic!("poison the observer");
        })
        .join();
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        // Recovered, not lost: the execution is recorded and nothing panics.
        assert_eq!((o.stats().observed, o.stats().dropped, o.stats().retained), (1, 0, 1));
    }

    #[test]
    fn a_missing_key_is_a_counted_loss() {
        let o = Observer::new();
        // Simulates a key that is gone: the level is forced past `set_level`.
        o.level.store(ObservationLevel::Fingerprint as u8, Ordering::SeqCst);
        report(&o, &filter("t"), ok(PlanPath::IndexType));
        let s = o.stats();
        assert_eq!((s.dropped, s.retained, s.observed), (1, 0, 1));
    }

    #[test]
    fn error_kinds_carry_no_message() {
        let corrupt = UnitError::Storage(kerno_core::Error::Corruption("secret-key".into()));
        assert_eq!(ErrorKind::of(&corrupt), ErrorKind::Corruption);
        assert_eq!(ErrorKind::of(&UnitError::InvalidArgument("secret-key".into())), ErrorKind::Invalid);
        assert_eq!(ErrorKind::of(&UnitError::NotAUnitDatabase), ErrorKind::Other);
        assert_eq!(ErrorKind::of(&UnitError::Storage(kerno_core::Error::Poisoned("x".into()))), ErrorKind::Storage);
    }
}
