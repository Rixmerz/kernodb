//! Kerno units layer (Stage 3).
//!
//! E3-01 specifies the on-disk layout of the layer's keys and values
//! (`docs/file-format/units-v1.md`). E3-02 implements it: the payload
//! integrity check ([`payload`]), the key and value codecs ([`codec`]), the
//! layer's errors ([`error`]), the injected clock and id sources ([`clock`])
//! and the reference model ([`model`]). E3-03 adds the database itself
//! ([`db`]): opening, change sets, create/update/archive/unarchive/delete
//! and `get_unit`. E3-04 adds history, as-of reads, change-set listing,
//! `compare` and `restore` ([`history`]). E3-05 adds history policies and
//! E3-06 retention and GC ([`retention`]),
//! and E3-07 the offline verifier `verify_units` ([`verify`]). E4-01 adds the
//! index-free reference query path ([`reference`]), and E4-02 declared
//! relations with their adjacency indexes ([`db`], ADR-0014). E4-03 adds the
//! primary index with basic filters and its freshness state ([`index`],
//! ADR-0007). E5-01 adds the normalized query fingerprint ([`fingerprint`],
//! ADR-0015) and the explainable plan ([`plan`]); E5-02 the deterministic
//! planner that `query` follows ([`planner`]); E5-03 the optional
//! `ExecutionObservation` ([`observation`]). E6-01 adds the dependency model and
//! learned-state store of the result cache ([`cache`]). E7-01 adds the associative
//! query model and its reference path ([`assoc`]); E7-02 its indexed path and its
//! plan ([`planner`]). E8-04 persists vectors as derived data, layer version 2
//! ([`vector_store`], ADR-0020). E8-06 adds summaries as derived data ([`consolidation`]). This crate
//! stays separate from `kerno-storage`.

pub mod assoc;
pub mod assoc_rank;
pub mod cache;
pub mod clock;
pub mod codec;
pub mod consolidation;
pub mod db;
pub mod derived;
pub mod derived_stale;
pub mod error;
pub mod export;
pub mod fingerprint;
pub mod history;
pub mod index;
pub mod model;
pub mod observation;
pub mod payload;
pub mod plan;
pub mod planner;
pub mod reclaim;
pub mod reference;
pub mod retention;
pub mod suppression;
pub mod types;
pub mod vector_store;
pub mod verify;

pub use assoc::{
    AssocDirection, AssocEnding, AssocError, AssocHit, AssocQuery, AssocQueryError, AssocResult, MAX_ASSOC_DEPTH,
    MAX_ASSOC_LIMIT, MAX_ASSOC_VISITS, ProvenanceFilter, SeqWindow, TraceStep,
};
pub use cache::{
    CacheConfig, CacheEntry, CacheError, CacheFallbackCause, CacheStats, CachedQuery, CachedResult, DeleteReason,
    Dependencies, Dependency, Evidence, Lookup, MissCause, OPTIMIZATION_VERSION, ResultCache, Switch, Transition,
    TransitionRecord,
};
pub use clock::{Clock, CounterIds, FixedClock, SystemClock, UnitIdSource, UuidV7Source};
pub use codec::{ChangeSetRecord, CsMember, Key, LayerMeta, Record, RevisionMeta, UnitHead};
pub use consolidation::{ConsolidationError, KIND_PARAM, Summarizer, TruncatingSummarizer, open_summaries};
pub use db::{
    ChangeMeta, ChangeSet, CommitInfo, CreatedUnit, DEFAULT_MAX_CHANGESET_OPS, MAX_POLICY_UNIT_TYPE_LEN, NewUnit,
    Relation, Unit, UnitConfig, UnitDb, UnitSnapshot, UnitUpdate,
};
pub use derived::{DerivedDescriptor, DerivedError, DerivedId, DerivedRegistry, DerivedState};
pub use derived_stale::{Compute, DerivedChangeSet, DerivedDb, DerivedStaleError, SourceInput, input_hash};
pub use error::{DecodeError, ErrorClass, UnitError, UnitRule};
pub use fingerprint::{
    FINGERPRINT_VERSION, Fingerprint, FingerprintAlg, FingerprintKey, NormalizedQuery, QueryKind, RelationQuery,
};
pub use history::{
    AsOf, ChangeSetInfo, ChangeSetMember, Comparison, HistoryPage, HistoryQuery, Order, Provenance, Revision,
    RevisionView, UnitAt, UnitStates,
};
pub use index::{IndexPath, IndexState, IndexStatus, QueryOutcome, QuerySource, UnitFilter};
pub use model::{ModelChange, ModelChangeSet, ModelError, ModelRevision, ModelUnitAt, UnitModel};
pub use observation::{
    DEFAULT_MAX_AGE, DEFAULT_MAX_RECORDS, ErrorKind, ExecutionObservation, ObservationConfig, ObservationLevel,
    ObservationStats, Observer, Outcome, PlanKind,
};
pub use payload::{
    ContentHash, HashAlg, MAX_PAYLOAD_LEN, PayloadDecl, PayloadError, PayloadRule, VerifiedPayload, verify_payload,
};
pub use plan::{Candidate, Cost, Plan, PlanError, PlanPath, Reason, Unusable, Usability};
pub use planner::{AssocOutcome, Fallback, FallbackCause, PLANNER_VERSION, RelationsOutcome, Violation};
pub use retention::{CompactReport, RetentionBudget, RetentionPacing, RetentionPolicy, RetentionReport, RetentionRule};
pub use suppression::SuppressionReport;
pub use types::{
    BuildOutcome, ChangeKind, ChangePurpose, ChangeSetId, CommitSeq, Direction, HistoryPolicy, Lifecycle,
    ProvenanceKind, Purpose, RelationId, RelationProvenance, RevNo, RevisionId, RevisionKind, Timestamp, UnitId,
};
pub use vector_store::{
    DerivedDecodeError, DerivedFinding, DerivedKey, DerivedRule, ReopenReport, RepairReport, StaleCause, VectorCompute,
    VectorDescriptor, VectorRecipe, VectorSpec, VectorState,
};
pub use verify::{
    CrossRule, UnitVerifyReport, VerifyCounts, VerifyFinding, VerifyRule, verify_engine, verify_immutable,
    verify_snapshot,
};
