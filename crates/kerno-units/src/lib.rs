//! Kerno units layer (Stage 3).
//!
//! E3-01 specifies the on-disk layout of the layer's keys and values
//! (`docs/file-format/units-v1.md`). E3-02 implements it: the payload
//! integrity check ([`payload`]), the key and value codecs ([`codec`]), the
//! layer's errors ([`error`]), the injected clock and id sources ([`clock`])
//! and the reference model ([`model`]). E3-03 adds the database itself
//! ([`db`]): opening, change sets, create/update/archive/unarchive/delete
//! and `get_unit`. E3-04 adds history, as-of reads, change-set listing,
//! `compare` and `restore` ([`history`]). This crate stays separate from `kerno-storage`.

pub mod clock;
pub mod codec;
pub mod db;
pub mod error;
pub mod history;
pub mod model;
pub mod payload;
pub mod types;

pub use clock::{Clock, CounterIds, FixedClock, SystemClock, UnitIdSource, UuidV7Source};
pub use codec::{ChangeSetRecord, CsMember, Key, LayerMeta, Record, RevisionMeta, UnitHead};
pub use db::{
    ChangeMeta, ChangeSet, CommitInfo, CreatedUnit, DEFAULT_MAX_CHANGESET_OPS, NewUnit, Unit, UnitConfig, UnitDb,
    UnitSnapshot, UnitUpdate,
};
pub use error::{DecodeError, ErrorClass, UnitError, UnitRule};
pub use history::{
    AsOf, ChangeSetInfo, ChangeSetMember, Comparison, HistoryQuery, Order, Provenance, Revision, UnitAt, UnitStates,
};
pub use model::{ModelChange, ModelChangeSet, ModelError, ModelRevision, ModelUnitAt, UnitModel};
pub use payload::{
    ContentHash, HashAlg, MAX_PAYLOAD_LEN, PayloadDecl, PayloadError, PayloadRule, VerifiedPayload, verify_payload,
};
pub use types::{
    ChangeKind, ChangePurpose, ChangeSetId, CommitSeq, HistoryPolicy, Lifecycle, Purpose, RevNo, RevisionId,
    RevisionKind, Timestamp, UnitId,
};
