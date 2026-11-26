//! The units database (`stage3-design.md` §4, §6.1–§6.4, E3-03): opening
//! with the layer mark, change sets with `create`, `update`, `archive`,
//! `unarchive` and `delete`, and `get_unit`.
//!
//! # Contracts
//!
//! * **One change set = one engine transaction (I-21).** Every record of a
//!   change set (`UNIT_HEAD`, `REV_META`, `REV_PAYLOAD`, `CS_MEMBER`,
//!   `CHANGESET` and, for the first one of a new database, the layer mark)
//!   is written into the single [`Txn`] that [`UnitDb::begin_changeset`]
//!   opens, and reaches the engine only through that transaction's commit.
//!   Aborting or dropping a [`ChangeSet`] writes nothing and consumes no
//!   sequence.
//! * **Sequence prediction (design §2.5).** The change set's
//!   `commit_seq` is `txn.read_seq() + 1`, computed before anything is
//!   written; after the commit the layer checks that the engine returned that
//!   very number. A mismatch cannot be undone: the commit is reported as
//!   `Storage(Poisoned)` and the handle refuses every later call until it is
//!   reopened.
//! * **One revision per unit per change set.** `rev_no` and `commit_seq`
//!   grow together per unit (I-26, IU-3), so a second operation on a unit
//!   already touched by the same change set is refused with
//!   `InvalidArgument` and leaves the change set usable.
//! * **Validation before writing.** Every operation checks its limits and
//!   preconditions and encodes all its records before its first write, so a
//!   refused operation has no effect and the change set stays usable. Only
//!   an engine error *during* the writes (a batch limit, an I/O error) can
//!   interrupt an operation half way; the change set is then broken: that
//!   operation returns the error, every later call returns
//!   [`UnitError::ChangeSetClosed`], and `commit` aborts it, so a partial
//!   operation is never committed.
//! * **Reads verify.** Every read of a `REV_PAYLOAD` goes through
//!   [`verify_payload`] against its `REV_META`, and the head is checked
//!   against that `REV_META`; a mismatch is `Storage(Corruption)`.
//! * **Determinism.** Time comes only from the injected [`Clock`] (one
//!   reading per change set, used for the change set and all its revisions)
//!   and ids only from the injected [`UnitIdSource`].
//!
//! `restore` (E3-04) lives here because it is a change-set operation; the
//! history and as-of reads (E3-04) are in [`crate::history`], retention and
//! `compact` (E3-06) in [`crate::retention`], and `verify_units` (E3-07) in
//! [`crate::verify`].
//!
//! # History policies (design §3.4, E3-05)
//!
//! Each unit type has a [`HistoryPolicy`]: the record `LAYER_META`
//! `policy/<unit_type>` written by [`UnitDb::set_policy`], or else the one in
//! the layer mark (the [`UnitConfig::default_policy`] in force when the mark
//! was written). The policy in force when a revision is written decides what
//! happens to the revision it supersedes, in the same change set:
//!
//! * `Versioned`: nothing; every revision is kept.
//! * `Audit`: the superseded head loses its `REV_PAYLOAD` and its `REV_META`
//!   is rewritten with `payload_present = 0` (hash and length are kept: a
//!   history of metadata and hashes).
//! * `None`: every `REV_META` and `REV_PAYLOAD` of the unit below the new
//!   revision is deleted (two range deletions) and `oldest_rev` becomes the
//!   new revision.
//!
//! Changing the policy rewrites nothing. Reads check the policy in force
//! *now* and answer [`UnitError::HistoryUnavailable`] instead of an empty
//! result whenever they would need what that policy does not keep (see
//! [`crate::history`]).

use std::collections::BTreeSet;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use kerno_core::contract::{Engine, Options, Snapshot, SyncEngine, Txn, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, MAX_VALUE_LEN};

use crate::clock::{Clock, SystemClock, UnitIdSource, UuidV7Source};
use crate::codec::{
    AdjBlock, ChangeSetRecord, CsMember, Key, LAYER_MARK_NAME, LAYER_VERSION_DERIVED, LayerMeta,
    MAX_CHANGESET_FIELD_LEN, MAX_LAYER_VERSION, MAX_MEMBER_COUNT, MAX_METADATA_LEN, MAX_REV_REF_LEN, MAX_UNIT_TYPE_LEN,
    RelationRecord, RevisionMeta, TAG_UNIT_HEAD, UnitHead, valid_relation_type,
};
use crate::error::{DecodeError, UnitError};
use crate::payload::{ContentHash, MAX_PAYLOAD_LEN, PAYLOAD_CODEC_OPAQUE, verify_payload_with};
use crate::types::{
    ChangeSetId, CommitSeq, Direction, HistoryPolicy, Lifecycle, Purpose, RelationId, RelationProvenance, RevNo,
    RevisionId, RevisionKind, Timestamp, UnitId,
};

/// Default of [`UnitConfig::max_changeset_ops`] (design §4).
pub const DEFAULT_MAX_CHANGESET_OPS: usize = 10_000;

/// Options of [`UnitDb::open`] and [`UnitDb::from_engine`].
#[derive(Clone)]
pub struct UnitConfig {
    /// Options the engine is opened with by [`UnitDb::open`]. Its
    /// `durability` is also what [`CommitInfo::durable`] reports, so a caller
    /// of [`UnitDb::from_engine`] must set it to the engine's real mode.
    pub engine: Options,
    /// Whether the first change set of an empty database may create the
    /// layer mark (design §2.6). When `false`, an empty database opens and
    /// reads as empty, and [`UnitDb::begin_changeset`] refuses to write.
    pub create_if_missing: bool,
    /// The history policy written in the layer mark by the first change set
    /// of a new database: the policy of every unit type without its own
    /// [`UnitDb::set_policy`]. Once the mark exists, it is what counts.
    pub default_policy: HistoryPolicy,
    /// Most operations in one change set (design §4), `1..=2^20`.
    pub max_changeset_ops: usize,
    /// Largest payload accepted, `<= 2^26` (UNIT-22). It must not exceed the
    /// engine's value limit (1 MiB by default, like this default).
    pub max_payload_len: usize,
    /// Source of every timestamp the layer writes.
    pub clock: Arc<dyn Clock>,
    /// Source of new unit ids.
    pub ids: Arc<dyn UnitIdSource>,
    /// How [`UnitDb::run_retention`] shares the write slot (E3-09b).
    pub retention_pacing: crate::retention::RetentionPacing,
    /// Whether the normal reads (a unit by id, `find`, queries, neighbours, search hits) check
    /// each payload they load against its content hash (UNIT-28). Default `false`: the bytes
    /// come out of SST blocks whose checksum the engine already checked. The length and
    /// presence rules are checked either way; `verify_units`, history reads, export, import,
    /// checked reads ([`UnitDb::set_checked_reads`]) and the reads a write builds on always
    /// check the hash.
    pub verify_payload_on_read: bool,
}

impl UnitConfig {
    /// The defaults with an injected clock and id source.
    pub fn with_sources(clock: Arc<dyn Clock>, ids: Arc<dyn UnitIdSource>) -> Self {
        UnitConfig {
            engine: Options::default(),
            create_if_missing: true,
            default_policy: HistoryPolicy::Versioned,
            max_changeset_ops: DEFAULT_MAX_CHANGESET_OPS,
            max_payload_len: MAX_VALUE_LEN,
            clock,
            ids,
            retention_pacing: crate::retention::RetentionPacing::default(),
            verify_payload_on_read: false,
        }
    }

    fn validate(&self) -> Result<(), UnitError> {
        if self.retention_pacing.units_per_change_set == 0 {
            return Err(UnitError::InvalidArgument("retention_pacing.units_per_change_set must be >= 1".into()));
        }
        if self.max_changeset_ops == 0 || self.max_changeset_ops > MAX_MEMBER_COUNT as usize {
            return Err(UnitError::InvalidArgument(format!(
                "max_changeset_ops {} is outside 1..={MAX_MEMBER_COUNT}",
                self.max_changeset_ops
            )));
        }
        // The engine refuses values above MAX_VALUE_LEN; a larger limit would
        // turn an oversized payload into a broken change set instead of
        // PayloadTooLarge (design §4).
        let limit = (MAX_PAYLOAD_LEN as usize).min(kerno_core::MAX_VALUE_LEN);
        if self.max_payload_len > limit {
            return Err(UnitError::InvalidArgument(format!(
                "max_payload_len {} exceeds {limit}",
                self.max_payload_len
            )));
        }
        Ok(())
    }
}

impl Default for UnitConfig {
    /// System clock and UUIDv7 ids; tests inject [`crate::FixedClock`] and
    /// [`crate::CounterIds`] with [`UnitConfig::with_sources`].
    fn default() -> Self {
        Self::with_sources(Arc::new(SystemClock), Arc::new(UuidV7Source::new()))
    }
}

impl std::fmt::Debug for UnitConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnitConfig")
            .field("engine", &self.engine)
            .field("create_if_missing", &self.create_if_missing)
            .field("default_policy", &self.default_policy)
            .field("max_changeset_ops", &self.max_changeset_ops)
            .field("max_payload_len", &self.max_payload_len)
            .field("retention_pacing", &self.retention_pacing)
            .finish_non_exhaustive()
    }
}

/// Provenance of a change set (design §6.3). `None` is absent; an empty
/// `Some` is refused because the format stores both as length 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeMeta {
    /// At most 1024 bytes.
    pub reason: Option<String>,
    /// At most 256 bytes: it is stored in the change set and in every
    /// revision of it (`REV_META.actor_ref`).
    pub actor: Option<Vec<u8>>,
    /// At most 256 bytes, stored like `actor`.
    pub source: Option<Vec<u8>>,
    /// At most 1024 bytes.
    pub correlation: Option<Vec<u8>>,
    /// Only [`Purpose::Application`] for a change set; [`UnitDb::set_policy`]
    /// also accepts `Maintenance` (and records `Maintenance` either way).
    /// `Retention` is written only by [`UnitDb::run_retention`].
    pub purpose: Purpose,
}

impl Default for ChangeMeta {
    /// No provenance, purpose `Application`.
    fn default() -> Self {
        ChangeMeta { reason: None, actor: None, source: None, correlation: None, purpose: Purpose::Application }
    }
}

impl ChangeMeta {
    fn validate(&self) -> Result<(), UnitError> {
        if self.purpose != Purpose::Application {
            return Err(UnitError::InvalidArgument(format!(
                "change set purpose {:?} is reserved to the layer",
                self.purpose
            )));
        }
        let fields: [(&str, Option<&[u8]>, u32); 4] = [
            ("reason", self.reason.as_deref().map(str::as_bytes), MAX_CHANGESET_FIELD_LEN),
            ("actor", self.actor.as_deref(), MAX_REV_REF_LEN),
            ("source", self.source.as_deref(), MAX_REV_REF_LEN),
            ("correlation", self.correlation.as_deref(), MAX_CHANGESET_FIELD_LEN),
        ];
        for (name, value, max) in fields {
            match value {
                Some([]) => {
                    return Err(UnitError::InvalidArgument(format!("{name} is empty; use None for absent")));
                }
                Some(v) if v.len() > max as usize => {
                    return Err(UnitError::InvalidArgument(format!("{name} of {} bytes exceeds {max}", v.len())));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// A new unit (design §6.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewUnit {
    /// 1..=64 bytes of UTF-8.
    pub unit_type: String,
    pub schema_version: u32,
    /// At most 4096 bytes.
    pub metadata: Vec<u8>,
    /// Opaque bytes, at most [`UnitConfig::max_payload_len`].
    pub payload: Vec<u8>,
}

/// A new revision of an existing unit's content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitUpdate {
    pub payload: Vec<u8>,
    /// `None` keeps the current schema version.
    pub schema_version: Option<u32>,
    /// `None` keeps the current metadata.
    pub metadata: Option<Vec<u8>>,
}

/// A unit as of its head revision (design §6.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub id: UnitId,
    pub unit_type: String,
    pub schema_version: u32,
    pub lifecycle: Lifecycle,
    pub head: RevNo,
    pub oldest: RevNo,
    pub created_seq: CommitSeq,
    pub updated_seq: CommitSeq,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub metadata: Vec<u8>,
    /// Verified payload of the head revision; `None` for a deleted unit.
    pub payload: Option<Vec<u8>>,
    pub content_hash: ContentHash,
}

/// Result of [`ChangeSet::commit`] (design §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    /// Equal to `commit_seq`. For an empty change set, both are the current
    /// sequence and nothing was written.
    pub change_set: ChangeSetId,
    pub commit_seq: CommitSeq,
    /// The revisions the change set created, in operation order.
    pub revisions: Vec<RevisionId>,
    /// Whether the change set was durable when `commit` returned: the
    /// engine's durability is `Sync` and the commit was not deferred.
    /// Otherwise it is durable after [`UnitDb::sync`] or, for a deferred
    /// `Retention` change set (decision 12c), also after the next durable
    /// change set returns.
    pub durable: bool,
}

/// `(id, commit)` returned by [`UnitDb::create_unit`].
pub type CreatedUnit = (UnitId, CommitInfo);

/// The units layer over a transactional engine `E` (design §6.1).
pub struct UnitDb<E: TxnEngine> {
    /// Reached only by the layer's own modules (`compact`, `reclaim`); no
    /// public method hands it out.
    pub(crate) engine: E,
    cfg: UnitConfig,
    poisoned: AtomicBool,
    /// Where an incomplete retention pass resumes (see
    /// [`crate::retention`]); the lock also serializes retention calls.
    pub(crate) retention: Mutex<Option<crate::retention::RetentionCursor>>,
    /// Bumped before and after every commit that may change a history
    /// policy (odd while one is in flight): a retention cursor taken under
    /// another value is not resumed, and the policy cache is per value.
    pub(crate) policy_epoch: AtomicU64,
    /// See [`PolicyCache`].
    pub(crate) policy_cache: Arc<PolicyCache>,
    /// Primary-index builds waiting for the write slot or running (E4-03):
    /// what [`UnitDb::index_status`] reports as `Building`.
    pub(crate) index_builds: AtomicUsize,
    /// Where `query` and `query_relations` report (E5-03); shared with every
    /// snapshot.
    observer: Arc<crate::observation::Observer>,
    /// The result cache (E6-02): off until a caller switches it on; shared
    /// with every snapshot. In memory only, outside every transaction.
    cache: Arc<crate::cache::ResultCache>,
    /// The embedding provider the caller registered (see [`crate::embed`]);
    /// none until one is. In memory only: it is never written to the database.
    pub(crate) embedder: Mutex<Option<Arc<dyn crate::embed::EmbeddingProvider>>>,
    /// What reads at the newest commit already loaded and verified (see [`crate::read_cache`]).
    pub(crate) read_cache: Arc<crate::read_cache::ReadCache>,
    /// Reads check a unit's head against its head revision ([`UnitDb::set_checked_reads`]).
    checked_reads: AtomicBool,
}

fn layer_mark_key() -> Vec<u8> {
    let mut k = vec![crate::codec::TAG_LAYER_META];
    k.extend_from_slice(LAYER_MARK_NAME.as_bytes());
    k
}

/// Prefix of the `LAYER_META` name of a unit type's policy.
const POLICY_PREFIX: &str = "policy/";

/// Longest unit type that can carry its own policy: the `LAYER_META` name
/// `policy/<unit_type>` is at most 64 bytes.
pub const MAX_POLICY_UNIT_TYPE_LEN: usize = crate::codec::MAX_LAYER_NAME_LEN - POLICY_PREFIX.len();

fn policy_key(unit_type: &str) -> Option<Vec<u8>> {
    if unit_type.is_empty() || unit_type.len() > MAX_POLICY_UNIT_TYPE_LEN {
        return None;
    }
    let mut k = vec![crate::codec::TAG_LAYER_META];
    k.extend_from_slice(POLICY_PREFIX.as_bytes());
    k.extend_from_slice(unit_type.as_bytes());
    Some(k)
}

/// The policy in force for `unit_type`: its own record, else the layer
/// mark's, else (no mark yet) `fallback`.
pub(crate) fn policy_of(get: &GetFn<'_>, unit_type: &str, fallback: HistoryPolicy) -> Result<HistoryPolicy, UnitError> {
    if let Some(k) = policy_key(unit_type)
        && let Some(v) = get(&k)?
    {
        let m = LayerMeta::decode_upto(&v, MAX_LAYER_VERSION)
            .map_err(|e| context(e, &format!("LAYER_META \"policy/{unit_type}\"")))?;
        return Ok(m.history_policy);
    }
    match get(&layer_mark_key())? {
        Some(v) => Ok(LayerMeta::decode_upto(&v, MAX_LAYER_VERSION)
            .map_err(|e| context(e, "LAYER_META \"layer\""))?
            .history_policy),
        None => Ok(fallback),
    }
}

pub(crate) fn key(k: Key) -> Result<Vec<u8>, UnitError> {
    k.encode()
}

/// Adds the record's identity to a decode error, keeping its rule and class.
pub(crate) fn context(e: DecodeError, what: &str) -> DecodeError {
    DecodeError { detail: format!("{what}: {}", e.detail), ..e }
}

pub(crate) fn corruption(msg: String) -> UnitError {
    UnitError::Storage(kerno_core::Error::Corruption(msg))
}

/// A point read: a snapshot's or a transaction's `get`.
pub(crate) type GetFn<'a> = dyn Fn(&[u8]) -> kerno_core::Result<Option<Vec<u8>>> + 'a;

/// What the layer mark says (design §2.6).
enum MarkState {
    /// The mark is there, with this `layer_version`.
    Present(u32),
    /// No mark and no key at all: a new database.
    Empty,
}

/// Reads the mark with `get`, and with `any_key` whether the store holds any
/// key. A store with keys but no mark is not a units database.
fn read_mark(get: &GetFn<'_>, any_key: &dyn Fn() -> kerno_core::Result<bool>) -> Result<MarkState, UnitError> {
    match get(&layer_mark_key())? {
        Some(v) => {
            let mark = LayerMeta::decode_upto(&v, MAX_LAYER_VERSION).map_err(|e| context(e, "LAYER_META \"layer\""))?;
            if mark.layer_version < crate::codec::MIN_LAYER_VERSION {
                return Err(UnitError::LayerTooOld {
                    found: mark.layer_version,
                    minimum: crate::codec::MIN_LAYER_VERSION,
                });
            }
            Ok(MarkState::Present(mark.layer_version))
        }
        None if any_key()? => Err(UnitError::NotAUnitDatabase),
        None => Ok(MarkState::Empty),
    }
}

/// A unit's head and its verified payload. The head's `inline_payload` is taken out of it: the
/// payload is the second element, wherever it was stored.
type Loaded = (UnitHead, Option<Vec<u8>>);

/// Reads and checks a unit: its head and the payload of its head revision.
///
/// A head that carries its payload (`units-v10.md` §2) is the whole read: one point read, and
/// the payload is checked against the `content_hash` and `payload_len` of the head it came
/// with (UNIT-27, UNIT-28). The `REV_META` and `REV_PAYLOAD` of the head revision are not
/// read: that the head mirrors them (IU-2), and that the two copies of the payload are the
/// same bytes, is what `verify_units` checks, and what [`load_unit_checked`], the
/// read every write builds on, still checks. Any other head is read as [`load_unit_checked`]
/// reads it.
///
/// `hash`: whether the payload's content hash is checked (UNIT-28); the normal reads pass
/// [`UnitConfig::verify_payload_on_read`], every read a write builds on passes `true`.
pub(crate) fn load_unit(get: &GetFn<'_>, id: UnitId, hash: bool) -> Result<Option<Loaded>, UnitError> {
    let raw = get(&key(Key::UnitHead { unit: id })?)?;
    load_unit_from_head(get, id, raw, hash)
}

/// [`load_unit`] for a caller that already read the unit's head record (`raw`, `None` when
/// absent): `get` is asked only for the records after it.
pub(crate) fn load_unit_from_head(
    get: &GetFn<'_>,
    id: UnitId,
    raw: Option<Vec<u8>>,
    hash: bool,
) -> Result<Option<Loaded>, UnitError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut head = UnitHead::decode(&raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {id}")))?;
    let Some(inline) = head.inline_payload.take() else {
        return load_revision(get, id, head, None, hash).map(Some);
    };
    check_inline(id, &head, &inline, hash)?;
    Ok(Some((head, Some(inline))))
}

/// A head among planned writes and the payload of its revision ([`planned_head`]).
pub(crate) type PlannedHead<'w> = (UnitHead, Option<&'w [u8]>);

/// The head `writes` put for `unit`, decoded, and the payload of its head revision as those
/// writes hold it: the copy at the end of the head when the head carries one (`units-v11.md`
/// §2), else the `REV_PAYLOAD` among the writes. `None`: no head among the writes.
pub(crate) fn planned_head(unit: UnitId, writes: &[Write]) -> Result<Option<PlannedHead<'_>>, UnitError> {
    let head_key = key(Key::UnitHead { unit })?;
    let Some(Write::Put(_, raw)) = writes.iter().find(|w| matches!(w, Write::Put(k, _) if *k == head_key)) else {
        return Ok(None);
    };
    let mut head = UnitHead::decode(raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
    // The copy is the head's trailer, its last bytes: borrowed from the write, not the decode.
    let payload = match head.inline_payload.take() {
        Some(p) => Some(&raw[raw.len() - p.len()..]),
        None => {
            let payload_key = key(Key::RevPayload { unit, rev: head.head_rev })?;
            writes.iter().find_map(|w| match w {
                Write::Put(k, p) if *k == payload_key => Some(p.as_slice()),
                _ => None,
            })
        }
    };
    Ok(Some((head, payload)))
}

/// [`load_unit`] that always reads the head revision as well: the head must mirror its
/// `REV_META` (IU-2), the `REV_PAYLOAD` must be the payload the revision declares, and an
/// inline payload must be those same bytes.
pub(crate) fn load_unit_checked(get: &GetFn<'_>, id: UnitId) -> Result<Option<Loaded>, UnitError> {
    let Some(raw) = get(&key(Key::UnitHead { unit: id })?)? else {
        return Ok(None);
    };
    let mut head = UnitHead::decode(&raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {id}")))?;
    // The copy is held to the REV_PAYLOAD, which is held to the REV_META, which the head must
    // mirror: in that order, so a head that disagrees with its revision is named as such.
    let inline = head.inline_payload.take();
    load_revision(get, id, head, inline, true).map(Some)
}

/// UNIT-27 and UNIT-28 for a payload stored in the head, against the head's own declaration.
fn check_inline(id: UnitId, head: &UnitHead, inline: &[u8], hash: bool) -> Result<(), UnitError> {
    verify_payload_with(&head.inline_decl(), Some(inline), hash)
        .map_err(|e| crate::PayloadError { detail: format!("unit {id} rev {}: {}", head.head_rev, e.detail), ..e })?;
    Ok(())
}

/// The head revision of `head`: its `REV_META`, which the head must mirror (IU-2), and its
/// verified `REV_PAYLOAD`, which must be `inline` when the head carried one.
fn load_revision(
    get: &GetFn<'_>,
    id: UnitId,
    head: UnitHead,
    inline: Option<Vec<u8>>,
    hash: bool,
) -> Result<Loaded, UnitError> {
    let rev = head.head_rev;
    let Some(raw_meta) = get(&key(Key::RevMeta { unit: id, rev })?)? else {
        return Err(corruption(format!("unit {id}: UNIT_HEAD names revision {rev} but its REV_META is missing")));
    };
    let meta =
        RevisionMeta::decode(rev, &raw_meta).map_err(|e| context(e, &format!("REV_META of unit {id} rev {rev}")))?;
    // IU-2: the head mirrors its revision.
    let consistent = meta.content_hash == head.content_hash
        && meta.payload_len == head.payload_len
        && meta.schema_version == head.schema_version
        && meta.commit_seq == head.updated_seq
        && meta.metadata == head.metadata
        && crate::verify::lifecycle_after(meta.kind) == head.lifecycle;
    if !consistent {
        return Err(corruption(format!("unit {id}: UNIT_HEAD disagrees with the REV_META of revision {rev}")));
    }
    // A payload the head carries has no REV_PAYLOAD of its revision (`units-v11.md` §2).
    let stored = match get(&key(Key::RevPayload { unit: id, rev })?)? {
        Some(p) if inline.is_some() => {
            return Err(corruption(format!(
                "unit {id}: revision {rev} has a REV_PAYLOAD although its UNIT_HEAD carries the payload ({} bytes)",
                p.len()
            )));
        }
        Some(p) => Some(p),
        None => inline.clone(),
    };
    let verified = verify_payload_with(&meta.payload_decl(), stored.as_deref(), hash)
        .map_err(|e| crate::PayloadError { detail: format!("unit {id} rev {rev}: {}", e.detail), ..e })?;
    if inline.is_some() && inline.as_deref() != verified.bytes() {
        return Err(corruption(format!(
            "unit {id}: the payload in its UNIT_HEAD is not the REV_PAYLOAD of revision {rev}"
        )));
    }
    let payload = verified.bytes().map(<[u8]>::to_vec);
    Ok((head, payload))
}

/// Point reads of a snapshot served from forward scans: a read of the unit
/// scans (`REV_META` and `REV_PAYLOAD` keys are `tag ‖ unit ‖ rev`, so the
/// records of consecutive units are adjacent) refills a page at the key asked
/// and answers the next keys from it. A key behind the page, or past it, scans
/// again, so any order of reads is correct and an ascending one is cheap.
///
/// `ponytail:` a unit with a long history has its old revisions between two
/// heads, so a page holds fewer units; a refill that served at most two reads
/// shrinks the next page to [`SEEK_PAGE`]. Upgrade path: a `HEAD_REV`-keyed
/// index of the head revisions if long histories on a scan become common.
pub(crate) struct PagedGets<'a, S: Snapshot> {
    snap: &'a S,
    page: std::cell::RefCell<Page>,
}

struct Page {
    tag: u8,
    from: Vec<u8>,
    rows: Vec<(Vec<u8>, Vec<u8>)>,
    /// The page ends before the end of the range: a key past its last row is unknown.
    full: bool,
    served: usize,
    limit: usize,
}

const WIDE_PAGE: usize = 256;
const SEEK_PAGE: usize = 8;
/// A page ends once its rows hold this many bytes (the first row is always kept).
const PAGE_BYTES: usize = 1 << 20;

impl<'a, S: Snapshot> PagedGets<'a, S> {
    /// Reads of keys that start with `tag`.
    pub(crate) fn new(snap: &'a S, tag: u8) -> Self {
        let page = Page { tag, from: Vec::new(), rows: Vec::new(), full: false, served: 0, limit: WIDE_PAGE };
        PagedGets { snap, page: std::cell::RefCell::new(page) }
    }

    pub(crate) fn get(&self, key: &[u8]) -> kerno_core::Result<Option<Vec<u8>>> {
        let mut p = self.page.borrow_mut();
        let covered = !p.rows.is_empty()
            && key >= p.from.as_slice()
            && (!p.full || p.rows.last().is_some_and(|(last, _)| key <= last.as_slice()));
        if !covered {
            p.limit = if p.served <= 2 && !p.rows.is_empty() { SEEK_PAGE } else { WIDE_PAGE };
            let end = [p.tag + 1];
            // The page also stops at PAGE_BYTES: a payload record can be MAX_PAYLOAD_LEN long, and the
            // rows past the one asked for may be of units the caller never reads.
            let (mut rows, mut bytes, mut cut) = (Vec::new(), 0usize, false);
            let mut visit = |k: &[u8], v: &[u8]| {
                rows.push((k.to_vec(), v.to_vec()));
                bytes += k.len() + v.len();
                if bytes >= PAGE_BYTES {
                    cut = true;
                    // Stops the scan; the error is swallowed below, `cut` says why it stopped.
                    return Err(kerno_core::Error::InvalidArgument("page full".into()));
                }
                Ok(())
            };
            let scanned = self.snap.scan_visit(Bound::Included(key), Bound::Excluded(&end), p.limit, &mut visit);
            if !cut {
                scanned?;
            }
            p.full = cut || rows.len() == p.limit;
            p.rows = rows;
            p.from = key.to_vec();
            p.served = 0;
        }
        p.served += 1;
        Ok(p.rows.binary_search_by(|(k, _)| k.as_slice().cmp(key)).ok().map(|at| p.rows[at].1.clone()))
    }
}

/// [`load_unit`] reading the revision records of consecutive units from pages.
pub(crate) struct UnitPages<'a, S: Snapshot> {
    heads: PagedGets<'a, S>,
    metas: PagedGets<'a, S>,
    payloads: PagedGets<'a, S>,
    checked: bool,
    hash: bool,
}

impl<'a, S: Snapshot> UnitPages<'a, S> {
    /// `checked`: read as [`load_unit_checked`] does ([`UnitDb::set_checked_reads`]).
    /// `hash`: check payload hashes ([`UnitConfig::verify_payload_on_read`]).
    pub(crate) fn new(snap: &'a S, checked: bool, hash: bool) -> Self {
        UnitPages {
            heads: PagedGets::new(snap, TAG_UNIT_HEAD),
            metas: PagedGets::new(snap, crate::codec::TAG_REV_META),
            payloads: PagedGets::new(snap, crate::codec::TAG_REV_PAYLOAD),
            checked,
            hash,
        }
    }

    /// [`load_unit`] of `id`; `head` is its already scanned `UNIT_HEAD` pair, when the caller has it.
    pub(crate) fn load(&self, id: UnitId, head: Option<(&[u8], &[u8])>) -> Result<Option<Loaded>, UnitError> {
        let get = |k: &[u8]| match k.first() {
            Some(&crate::codec::TAG_REV_META) => self.metas.get(k),
            Some(&crate::codec::TAG_REV_PAYLOAD) => self.payloads.get(k),
            _ => match head {
                Some((hk, hv)) if hk == k => Ok(Some(hv.to_vec())),
                _ => self.heads.get(k),
            },
        };
        if self.checked { load_unit_checked(&get, id) } else { load_unit(&get, id, self.hash) }
    }
}

pub(crate) fn to_unit(id: UnitId, head: UnitHead, payload: Option<Vec<u8>>) -> Unit {
    Unit {
        id,
        unit_type: head.unit_type,
        schema_version: head.schema_version,
        lifecycle: head.lifecycle,
        head: head.head_rev,
        oldest: head.oldest_rev,
        created_seq: head.created_seq,
        updated_seq: head.updated_seq,
        created_at: head.created_at,
        updated_at: head.updated_at,
        metadata: head.metadata,
        payload,
        content_hash: head.content_hash,
    }
}

impl<E: TxnEngine> UnitDb<E> {
    /// Opens the engine in `dir` with `cfg.engine` and checks the layer mark
    /// without writing anything (design §2.6): a database with keys but no
    /// mark is [`UnitError::NotAUnitDatabase`], a mark of a newer layer is
    /// [`UnitError::IncompatibleLayer`], a damaged mark is
    /// `Storage(Corruption)`. An empty database opens; its mark is written
    /// by its first change set.
    pub fn open(vfs: Arc<dyn Vfs>, dir: &Path, cfg: &UnitConfig) -> Result<Self, UnitError> {
        cfg.validate()?;
        let engine = E::open(vfs, dir, &cfg.engine)?;
        Self::from_engine(engine, cfg)
    }

    /// [`open`](Self::open) over an engine the caller opened (for instance
    /// with engine-specific settings). On error the engine is closed.
    pub fn from_engine(engine: E, cfg: &UnitConfig) -> Result<Self, UnitError> {
        let checked = cfg.validate().and_then(|()| {
            let snap = engine.snapshot()?;
            read_mark(&|k| snap.get(k), &|| Ok(!snap.scan(Bound::Unbounded, Bound::Unbounded, 1)?.is_empty()))
        });
        match checked {
            Ok(_) => Ok(UnitDb {
                engine,
                cfg: cfg.clone(),
                poisoned: AtomicBool::new(false),
                retention: Mutex::new(None),
                policy_epoch: AtomicU64::new(0),
                policy_cache: Arc::default(),
                index_builds: AtomicUsize::new(0),
                observer: Arc::default(),
                cache: Arc::default(),
                read_cache: Arc::new(crate::read_cache::ReadCache::new(crate::read_cache::DEFAULT_READ_CACHE_BYTES)),
                checked_reads: AtomicBool::new(false),
                embedder: Mutex::new(None),
            }),
            Err(e) => {
                let _ = engine.close();
                Err(e)
            }
        }
    }

    pub fn config(&self) -> &UnitConfig {
        &self.cfg
    }

    /// The observation point of this database (E5-03): `Off` until a caller
    /// sets a level.
    pub fn observer(&self) -> &crate::observation::Observer {
        &self.observer
    }

    /// The result cache of this database (E6-02): **off** by default. While it
    /// is off, `query` and `query_relations` behave exactly as without it.
    pub fn result_cache(&self) -> &crate::cache::ResultCache {
        &self.cache
    }

    /// The engine underneath, for this crate's own modules only. Nothing
    /// outside the crate can reach it (rule 9 of the Stage 9 acta).
    pub(crate) fn inner_engine(&self) -> &E {
        &self.engine
    }

    pub(crate) fn check_poisoned(&self) -> Result<(), UnitError> {
        if self.poisoned.load(Ordering::SeqCst) {
            return Err(UnitError::Storage(kerno_core::Error::Poisoned(
                "a change set committed with a sequence other than the predicted one; reopen".into(),
            )));
        }
        Ok(())
    }

    /// Closes the engine.
    pub fn close(self) -> Result<(), UnitError> {
        self.engine.close().map_err(UnitError::from)
    }

    /// Starts a change set: takes the engine's write slot (blocking while
    /// another change set or blind commit holds it; calling it on a thread
    /// that already holds one deadlocks, as `begin_write` does) and predicts
    /// its commit sequence.
    pub fn begin_changeset(&self, meta: ChangeMeta) -> Result<ChangeSet<'_, E>, UnitError> {
        self.begin_changeset_at(meta, None)
    }

    /// [`begin_changeset`](Self::begin_changeset) with the clock reading
    /// `now` taken by the caller (`None`: read it after taking the slot).
    pub(crate) fn begin_changeset_at(
        &self,
        meta: ChangeMeta,
        now: Option<Timestamp>,
    ) -> Result<ChangeSet<'_, E>, UnitError> {
        self.check_poisoned()?;
        meta.validate()?;
        let txn = self.engine.begin_write()?;
        let (need_mark, layer_version) = match read_mark(&|k| txn.get(k), &|| {
            Ok(!txn.scan(Bound::Unbounded, Bound::Unbounded, 1)?.is_empty())
        })? {
            MarkState::Present(v) => (false, v),
            MarkState::Empty if self.cfg.create_if_missing => (true, crate::codec::MIN_LAYER_VERSION),
            MarkState::Empty => {
                return Err(UnitError::InvalidArgument("the database is empty and create_if_missing is false".into()));
            }
        };
        let seq = txn
            .read_seq()
            .checked_add(1)
            .ok_or_else(|| UnitError::InvalidArgument("the commit sequence is exhausted".into()))?;
        let now = now.unwrap_or_else(|| self.cfg.clock.now());
        Ok(ChangeSet {
            db: self,
            txn: Some(txn),
            seq: CommitSeq(seq),
            meta,
            now,
            need_mark,
            layer_version,
            derived_ops: 0,
            token_ops: 0,
            active_known: std::collections::HashSet::default(),
            rel_ceiling: RelCeiling::Unknown,
            fresh: std::cell::Cell::new(Fresh::Unknown),
            schema_ops: 0,
            type_option_ops: 0,
            substring_pending: Default::default(),
            adj_pending: Default::default(),
            text_pending: Default::default(),
            text_stats: None,
            text_ords: std::cell::Cell::new(None),
            members: Vec::new(),
            relation_ops: 0,
            touched: BTreeSet::new(),
            broken: false,
            maintenance: false,
            index: false,
            writes: 0,
            bytes: 0,
            deferred: false,
            options_cache: std::cell::RefCell::default(),
            schema_cache: std::cell::RefCell::default(),
            unique_free: std::cell::RefCell::default(),
            base_field_empty: std::cell::RefCell::default(),
            prepared_text: std::cell::RefCell::default(),
            prepared_hash: std::cell::Cell::default(),
            base: std::cell::OnceCell::new(),
            ranges_deleted: false,
            unique_staged: Default::default(),
            live_pending: Default::default(),
            no_derived: std::cell::Cell::new(None),
        })
    }

    /// A change set with a single `create`.
    pub fn create_unit(&self, new: NewUnit, meta: ChangeMeta) -> Result<CreatedUnit, UnitError> {
        let mut cs = self.begin_changeset(meta)?;
        let id = cs.create(new)?;
        Ok((id, cs.commit()?))
    }

    /// The unit as of the latest commit, or `None` if it never existed. A
    /// deleted unit is returned with `lifecycle = Deleted` and no payload.
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        self.snapshot()?.get_unit(id)
    }

    /// A consistent read view (I-05).
    pub fn snapshot(&self) -> Result<UnitSnapshot<E>, UnitError> {
        self.check_poisoned()?;
        let before = self.policy_epoch.load(Ordering::SeqCst);
        let snap = self.engine.snapshot()?;
        let stable = before.is_multiple_of(2) && self.policy_epoch.load(Ordering::SeqCst) == before;
        let policies = stable.then(|| (before, self.policy_cache.clone()));
        Ok(UnitSnapshot {
            snap,
            fallback: self.cfg.default_policy,
            policies,
            observer: self.observer.clone(),
            cache: self.cache.clone(),
            read_cache: self.read_cache.clone(),
            checked_reads: self.checked_reads.load(Ordering::Relaxed),
            verify_payload_on_read: self.cfg.verify_payload_on_read,
        })
    }

    /// Whether a read of a unit also reads its head revision, as every read did before layer
    /// 10. Off by default.
    ///
    /// Off, a unit whose head carries its payload (`units-v10.md`, `units-v11.md`) is read with
    /// one point read: the payload is checked against the head's hash, and the `REV_META` of the
    /// head revision is not looked at, so damage that is only there (or a `REV_PAYLOAD` that must
    /// not exist) is found by `verify_units`, by the next write to the unit and by a read of its
    /// history, not by `get_unit`. On, every read of a unit also reads the `REV_META` and the
    /// `REV_PAYLOAD` of its head revision and refuses a head that does not mirror its
    /// `REV_META` (IU-2), a payload that does not match it, or a `REV_PAYLOAD` beside the
    /// payload the head carries. The snapshots taken after the call follow it; the read cache
    /// is emptied.
    pub fn set_checked_reads(&self, on: bool) {
        self.checked_reads.store(on, Ordering::Relaxed);
        self.read_cache.clear();
    }

    /// Sets how many bytes of verified units the read cache may hold ([`crate::DEFAULT_READ_CACHE_BYTES`]
    /// by default); `0` turns it off. The cache starts empty either way.
    pub fn set_read_cache_bytes(&self, bytes: usize) {
        self.read_cache.set_budget(bytes);
    }

    /// The history policy in force for `unit_type` (see the [module
    /// documentation](self)).
    pub fn policy(&self, unit_type: &str) -> Result<HistoryPolicy, UnitError> {
        self.snapshot()?.policy(unit_type)
    }

    /// Sets the history policy of `unit_type` (design §3.4): one change set
    /// with purpose `Maintenance`, no members, that writes the `LAYER_META`
    /// record `policy/<unit_type>`. Existing revisions are not rewritten; the
    /// policy applies to the revisions written from now on and to every read.
    /// `unit_type` must be 1..=57 bytes ([`MAX_POLICY_UNIT_TYPE_LEN`]); a
    /// longer type always follows the default policy. `meta.purpose` must be
    /// `Application` or `Maintenance`. Setting the policy already in force
    /// still commits (the change set records who asked).
    pub fn set_policy(&self, unit_type: &str, p: HistoryPolicy, meta: ChangeMeta) -> Result<CommitInfo, UnitError> {
        let Some(k) = policy_key(unit_type) else {
            return Err(UnitError::InvalidArgument(format!(
                "unit_type of {} bytes is outside 1..={MAX_POLICY_UNIT_TYPE_LEN} for a policy",
                unit_type.len()
            )));
        };
        if meta.purpose == Purpose::Retention {
            return Err(UnitError::InvalidArgument("set_policy with purpose Retention".into()));
        }
        let mut cs = self.begin_changeset(ChangeMeta { purpose: Purpose::Application, ..meta })?;
        cs.meta.purpose = Purpose::Maintenance;
        // Never below the mark (ADR-0020 §S3.5): a database with derived data
        // keeps writing version 2.
        let v = LayerMeta { layer_version: cs.layer_version, history_policy: p }.encode()?;
        cs.put(&k, &v)?;
        cs.maintenance = true;
        cs.commit()
    }
}

/// Accessors that return data from the engine underneath, so a caller that
/// needs the stored bytes does not need the engine (E9-08a). None of them
/// hands out the engine. The reads check poisoning as [`UnitDb::snapshot`]
/// does; `raw_commit` writes below the layer on purpose and exists only with
/// the `test-support` feature (D23).
impl<E: TxnEngine> UnitDb<E> {
    /// A read-only view of the raw keys and values at the current commit.
    #[doc(hidden)]
    pub fn raw_snapshot(&self) -> Result<E::Snapshot, UnitError> {
        self.check_poisoned()?;
        self.engine.snapshot().map_err(UnitError::from)
    }

    /// The value stored under a raw key now, `None` when absent.
    #[doc(hidden)]
    pub fn raw_get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, UnitError> {
        self.raw_snapshot()?.get(key).map_err(UnitError::from)
    }

    /// The engine's last commit sequence.
    #[doc(hidden)]
    pub fn raw_seq(&self) -> Result<u64, UnitError> {
        Ok(self.raw_snapshot()?.seq())
    }

    /// Writes raw entries in one engine commit, in order (`None` deletes the
    /// key; a key repeated in `entries` keeps its last entry), and returns the
    /// commit sequence. It exists only for tests that damage stored bytes, and
    /// only with the `test-support` feature (or in this crate's unit tests).
    ///
    /// It bypasses everything the layer does around a commit:
    /// - the policy epoch, and so the policy cache and the retention cursor:
    ///   a raw write of a `policy/<type>` record is not seen by
    ///   [`UnitDb::policy`] while the cache holds the old value, and an
    ///   incomplete retention pass resumes as if nothing had changed;
    /// - the poisoned check: it writes even when the database is poisoned;
    /// - the removal of derived records and suppression that a layer delete
    ///   performs: deleting a unit's keys here leaves its derived records
    ///   and suppression entries behind.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn raw_commit(&self, entries: Vec<(Vec<u8>, Option<Vec<u8>>)>) -> Result<u64, UnitError> {
        let mut batch = kerno_core::WriteBatch::new();
        for (k, v) in entries {
            match v {
                Some(v) => batch.put(k, v),
                None => batch.delete(k),
            };
        }
        self.engine.commit(&batch).map_err(UnitError::from)
    }

    /// The engine's counters by name.
    pub fn storage_stats(&self) -> Vec<(String, u64)> {
        self.engine.stats()
    }
}

/// Engine test hooks as methods of the layer, for the engines that have them.
/// Only with the `test-support` feature (or in this crate's unit tests), so a
/// production build has no physical-reclamation method on an arbitrary range:
/// its public reclamation calls are [`UnitDb::reclaim`], over the whole key
/// space, and [`UnitDb::compact`], over the revision ranges (`REV_META` and
/// `REV_PAYLOAD`), both through the public `Reclaim` trait (D23).
#[cfg(any(test, feature = "test-support"))]
impl<E: TxnEngine + kerno_core::contract::TestHooks> UnitDb<E> {
    /// Flushes the memtable to a table.
    #[doc(hidden)]
    pub fn hook_flush(&self) -> Result<(), UnitError> {
        self.engine.flush().map_err(UnitError::from)
    }

    /// Blocks until no background work is running or queued.
    #[doc(hidden)]
    pub fn hook_wait_idle(&self) -> Result<(), UnitError> {
        self.engine.wait_idle().map_err(UnitError::from)
    }

    /// Compacts the raw key range `[start, end)` of the engine.
    #[doc(hidden)]
    pub fn hook_compact_range(&self, start: Bound<&[u8]>, end: Bound<&[u8]>) -> Result<(), UnitError> {
        self.engine.compact_range(start, end).map_err(UnitError::from)
    }
}

impl<E: TxnEngine + SyncEngine> UnitDb<E> {
    /// The engine's durability barrier (I-17): returns the durable sequence.
    pub fn sync(&self) -> Result<u64, UnitError> {
        self.check_poisoned()?;
        self.engine.sync().map_err(UnitError::from)
    }
}

impl<E: TxnEngine + kerno_core::contract::Refresh> UnitDb<E> {
    /// Moves a read-only handle to the commits the writer made since it
    /// opened (or since the last refresh) and returns the commit sequence it
    /// now reads. Snapshots taken before keep their state. `Storage(InvalidArgument)`
    /// on a handle that is not read-only.
    pub fn refresh(&self) -> Result<u64, UnitError> {
        self.check_poisoned()?;
        let seq = kerno_core::contract::Refresh::refresh(&self.engine)?;
        // The writer may have changed a history policy: drop what the cache
        // read. Two steps keep the epoch even, as a commit does.
        self.policy_epoch.fetch_add(2, Ordering::SeqCst);
        Ok(seq)
    }
}

/// Bumps the policy epoch when dropped: the second half of the pair around
/// a commit that may change a policy.
struct EpochBump<'a>(&'a AtomicU64);

impl Drop for EpochBump<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// History policies read by snapshots, valid for one policy epoch: every
/// snapshot taken while the epoch held the same even value sees the same
/// policy records (only commits that bump the epoch write them), so what
/// one of them read answers for the others (E3-09b, `history` fixed cost).
#[derive(Default)]
pub(crate) struct PolicyCache(Mutex<(u64, std::collections::HashMap<String, HistoryPolicy>)>);

/// A read view of the units at one commit sequence.
pub struct UnitSnapshot<E: Engine> {
    pub(crate) snap: E::Snapshot,
    /// The policy of a database without a layer mark.
    pub(crate) fallback: HistoryPolicy,
    /// The policy cache and the epoch this snapshot may use it under;
    /// `None` when a policy commit was in flight while it was taken.
    pub(crate) policies: Option<(u64, Arc<PolicyCache>)>,
    /// The database's observer: `query` reports to it.
    pub(crate) observer: Arc<crate::observation::Observer>,
    /// The database's result cache: `query` and `query_relations` consult it.
    pub(crate) cache: Arc<crate::cache::ResultCache>,
    /// The database's read cache: `get_unit` and `schema` consult it.
    pub(crate) read_cache: Arc<crate::read_cache::ReadCache>,
    /// [`UnitDb::set_checked_reads`] when the snapshot was taken.
    pub(crate) checked_reads: bool,
    /// [`UnitConfig::verify_payload_on_read`].
    pub(crate) verify_payload_on_read: bool,
}

impl<E: Engine> UnitSnapshot<E> {
    pub fn seq(&self) -> CommitSeq {
        CommitSeq(self.snap.seq())
    }

    /// The history policy in force for `unit_type` at this snapshot.
    pub fn policy(&self, unit_type: &str) -> Result<HistoryPolicy, UnitError> {
        if let Some((epoch, cache)) = &self.policies {
            let g = cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if g.0 == *epoch
                && let Some(p) = g.1.get(unit_type)
            {
                return Ok(*p);
            }
        }
        let p = policy_of(&|k| self.snap.get(k), unit_type, self.fallback)?;
        if let Some((epoch, cache)) = &self.policies {
            let mut g = cache.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if g.0 <= *epoch {
                if g.0 != *epoch {
                    *g = (*epoch, std::collections::HashMap::new());
                }
                g.1.insert(unit_type.to_owned(), p);
            }
        }
        Ok(p)
    }

    /// The `layer_version` of the layer mark at this snapshot, or `None`
    /// without a mark.
    pub fn layer_version(&self) -> Result<Option<u32>, UnitError> {
        match self.snap.get(&layer_mark_key())? {
            Some(v) => Ok(Some(
                LayerMeta::decode_upto(&v, MAX_LAYER_VERSION)
                    .map_err(|e| context(e, "LAYER_META \"layer\""))?
                    .layer_version,
            )),
            None => Ok(None),
        }
    }

    /// The unit at this snapshot (see [`UnitDb::get_unit`]).
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        Ok(self.get_unit_shared(id)?.map(|u| (*u).clone()))
    }

    /// [`get_unit`](Self::get_unit), sharing the unit the read cache holds instead of cloning it.
    pub fn get_unit_shared(&self, id: UnitId) -> Result<Option<Arc<Unit>>, UnitError> {
        let seq = self.snap.seq();
        if let Some(hit) = self.read_cache.unit(seq, id) {
            return Ok(Some(hit));
        }
        let get = |k: &[u8]| self.snap.get(k);
        let loaded = if self.checked_reads {
            load_unit_checked(&get, id)?
        } else {
            load_unit(&get, id, self.verify_payload_on_read)?
        };
        let unit = loaded.map(|(head, payload)| Arc::new(to_unit(id, head, payload)));
        if let Some(u) = &unit {
            self.read_cache.put_unit(seq, u);
        }
        Ok(unit)
    }

    /// [`get_unit`](Self::get_unit) of each id, in the order of `ids`, with the heads of the
    /// units the read cache does not hold read by one [`Snapshot::multi_get`] (each block once,
    /// on a few threads when there are many). Each unit is then loaded and checked as
    /// `get_unit` does; a head without its payload reads the rest as `get_unit` does.
    pub fn get_units(&self, ids: &[UnitId]) -> Result<Vec<Option<Unit>>, UnitError> {
        Ok(self.get_units_shared(ids)?.into_iter().map(|u| u.map(|u| (*u).clone())).collect())
    }

    /// [`get_units`](Self::get_units), sharing the units the read cache holds instead of cloning them.
    pub fn get_units_shared(&self, ids: &[UnitId]) -> Result<Vec<Option<Arc<Unit>>>, UnitError> {
        let seq = self.snap.seq();
        let mut out: Vec<Option<Option<Arc<Unit>>>> =
            ids.iter().map(|&id| self.read_cache.unit(seq, id).map(Some)).collect();
        let missing: Vec<usize> = (0..ids.len()).filter(|&i| out[i].is_none()).collect();
        if missing.len() < 2 {
            for &i in &missing {
                out[i] = Some(self.get_unit_shared(ids[i])?);
            }
            return Ok(out.into_iter().map(|u| u.expect("every id is answered")).collect());
        }
        let keys: Vec<Vec<u8>> =
            missing.iter().map(|&i| key(Key::UnitHead { unit: ids[i] })).collect::<Result<_, _>>()?;
        let refs: Vec<&[u8]> = keys.iter().map(Vec::as_slice).collect();
        let heads = self.snap.multi_get(&refs)?;
        let mut loaded_units: Vec<Arc<Unit>> = Vec::with_capacity(missing.len());
        let snap_get = |q: &[u8]| self.snap.get(q);
        for ((&i, k), head) in missing.iter().zip(&keys).zip(heads) {
            let id = ids[i];
            let loaded = if self.checked_reads {
                // The head comes from the batch, any other record from the snapshot.
                let head = std::cell::Cell::new(Some(head));
                let get = |q: &[u8]| match head.take() {
                    Some(h) if q == k.as_slice() => Ok(h),
                    h => {
                        head.set(h);
                        self.snap.get(q)
                    }
                };
                load_unit_checked(&get, id)?
            } else {
                load_unit_from_head(&snap_get, id, head, self.verify_payload_on_read)?
            };
            let unit = loaded.map(|(head, payload)| Arc::new(to_unit(id, head, payload)));
            if let Some(u) = &unit {
                loaded_units.push(Arc::clone(u));
            }
            out[i] = Some(unit);
        }
        self.read_cache.put_units(seq, &loaded_units);
        Ok(out.into_iter().map(|u| u.expect("every id is answered")).collect())
    }

    /// Whether a unit with this id ever existed at this snapshot (a deleted one did): one read of
    /// its head, without loading or checking its payload.
    pub fn unit_exists(&self, id: UnitId) -> Result<bool, UnitError> {
        if self.read_cache.unit(self.snap.seq(), id).is_some() {
            return Ok(true);
        }
        Ok(crate::history::read_head(&|k| self.snap.get(k), id)?.is_some())
    }

    /// The relation `id` at this snapshot, or `None`.
    pub fn get_relation(&self, id: RelationId) -> Result<Option<Relation>, UnitError> {
        self.relation_checked(id, &mut std::collections::HashSet::new(), None)
    }

    /// [`get_relation`](Self::get_relation) for a caller that reads many relations at one
    /// snapshot: `active` holds the units already found `Active` (each head is read once), and
    /// `scanned` names the adjacency side the caller read itself and compares with the record,
    /// so that entry is not read a second time.
    fn relation_checked(
        &self,
        id: RelationId,
        active: &mut std::collections::HashSet<UnitId>,
        scanned: Option<Direction>,
    ) -> Result<Option<Relation>, UnitError> {
        let Some(raw) = self.snap.get(&key(Key::Relation { edge: id })?)? else {
            return Ok(None);
        };
        let record = RelationRecord::decode(&raw).map_err(|e| context(e, &format!("REL {id}")))?;
        // K5 (UNIT-36): the record is consistent with the snapshot, its two
        // ends are Active units, and both adjacency entries exist. Two head
        // reads and two entry reads per call.
        if record.created_seq.0 > self.snap.seq() {
            return Err(corruption(format!(
                "relation {id}: created_seq {} after the snapshot {}",
                record.created_seq,
                self.snap.seq()
            )));
        }
        for unit in [record.from, record.to] {
            if active.contains(&unit) {
                continue;
            }
            let get = |k: &[u8]| self.snap.get(k);
            match crate::history::read_head(&get, unit)? {
                Some(h) if h.lifecycle == Lifecycle::Active => {
                    active.insert(unit);
                }
                Some(h) => return Err(corruption(format!("relation {id}: unit {unit} is {:?}", h.lifecycle))),
                None => return Err(corruption(format!("relation {id}: unit {unit} does not exist"))),
            }
        }
        for (side, dir, unit, other) in
            [("REL_OUT", Direction::Out, record.from, record.to), ("REL_IN", Direction::In, record.to, record.from)]
        {
            if scanned == Some(dir) {
                continue;
            }
            if self.adjacency_entry(dir, unit, &record.relation_type, id)? != Some(other) {
                return Err(corruption(format!("relation {id}: {side} entry missing or wrong")));
            }
        }
        Ok(Some(Relation::new(id, record)))
    }

    /// The relations that start at (`Direction::Out`) or end at
    /// (`Direction::In`) `unit`, optionally of one `relation_type`, read from
    /// the adjacency index: a range scan, not a scan of all relations. In
    /// index order: relation type, then relation id. Every entry is checked
    /// against its `REL` record; a missing record or a disagreement is
    /// `Storage(Corruption)`. The result must equal
    /// [`UnitSnapshot::reference_relations_of`] on the same snapshot (I-32).
    ///
    /// The read is never silently short (K1, P1): the entries it scanned are
    /// compared, per relation type, with the count records of
    /// [`crate::adj_count`] (one point read for a typed read, one range scan
    /// of the unit's count records for an untyped one), and a disagreement is
    /// `Storage(Corruption)`. The limits (a store written before layer 4, a
    /// count lost with its entries) are in that module.
    pub fn relations_of(
        &self,
        unit: UnitId,
        dir: Direction,
        relation_type: Option<&str>,
    ) -> Result<Vec<Relation>, UnitError> {
        let tag = crate::adjacency::tag_of(dir);
        let Some((start, end)) = Key::adjacency_range(tag, unit, relation_type) else {
            return Err(UnitError::InvalidArgument("relation_type is not a valid relation type name".into()));
        };
        // What this sequence already read and checked for this unit, direction and type.
        let seq = self.snap.seq();
        let cache_key = (unit, tag, relation_type.map(str::to_owned));
        if let Some(hit) = self.read_cache.relations(seq, &cache_key) {
            return Ok((*hit).clone());
        }
        // K1: the counts the scan must meet, by type (P1).
        let expected = crate::adj_count::expected(&self.snap, unit, dir, relation_type)?;
        let mut seen: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
        let mut out = Vec::new();
        // The heads this read already found `Active`: `unit` is an end of every relation here.
        let mut active = std::collections::HashSet::new();
        scan_all(&self.snap, start, end, |k, v| {
            let ty = match Key::decode(k)? {
                Key::RelationOut { relation_type, .. } | Key::RelationIn { relation_type, .. } => relation_type,
                _ => return Err(corruption("a key in an adjacency range is not an adjacency key".into())),
            };
            let block = AdjBlock::decode(v).map_err(|e| context(e, "adjacency block"))?;
            for (edge, other) in block.entries {
                // The entry just scanned is this side's: it is compared with the record below
                // instead of being read again.
                let Some(rel) = self.relation_checked(edge, &mut active, Some(dir))? else {
                    return Err(corruption(format!("adjacency entry of relation {edge} has no REL record")));
                };
                let (here, there) = if dir == Direction::Out { (rel.from, rel.to) } else { (rel.to, rel.from) };
                if here != unit || there != other || rel.relation_type != ty {
                    return Err(corruption(format!(
                        "adjacency entry of relation {edge} disagrees with its REL record"
                    )));
                }
                *seen.entry(ty.clone()).or_insert(0) += 1;
                out.push(rel);
            }
            Ok(())
        })?;
        crate::adj_count::compare(unit, &expected, &seen)?;
        self.read_cache.put_relations(seq, cache_key, &out);
        Ok(out)
    }
}

/// A relation between two units (ADR-0014).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub id: RelationId,
    pub from: UnitId,
    pub to: UnitId,
    pub relation_type: String,
    /// The commit sequence of the change set that created it; that change
    /// set's record holds the actor, source and reason.
    pub created_seq: CommitSeq,
    pub provenance: RelationProvenance,
}

impl Relation {
    pub(crate) fn new(id: RelationId, r: RelationRecord) -> Self {
        Relation {
            id,
            from: r.from,
            to: r.to,
            relation_type: r.relation_type,
            created_seq: r.created_seq,
            provenance: r.provenance,
        }
    }
}

/// Keys read per engine scan call by the relation reads.
const REL_PAGE: usize = 256;

/// Calls `visit` on every pair with a key in `[start, end)` of `snap`, in
/// key order, [`REL_PAGE`] pairs per scan call.
pub(crate) fn scan_all<S: Snapshot>(
    snap: &S,
    start: Vec<u8>,
    end: Vec<u8>,
    mut visit: impl FnMut(&[u8], &[u8]) -> Result<(), UnitError>,
) -> Result<(), UnitError> {
    let mut cursor = start;
    let mut inclusive = true;
    loop {
        let lo = if inclusive { Bound::Included(cursor.as_slice()) } else { Bound::Excluded(cursor.as_slice()) };
        let page = snap.scan(lo, Bound::Excluded(&end), REL_PAGE)?;
        let n = page.len();
        for (k, v) in &page {
            visit(k, v)?;
        }
        match page.into_iter().last() {
            Some((k, _)) if n == REL_PAGE => {
                cursor = k;
                inclusive = false;
            }
            _ => return Ok(()),
        }
    }
}

/// An open change set: one engine write transaction (see the [module
/// documentation](self)). Dropping it aborts it.
pub struct ChangeSet<'db, E: TxnEngine + 'db> {
    db: &'db UnitDb<E>,
    txn: Option<E::Txn<'db>>,
    seq: CommitSeq,
    meta: ChangeMeta,
    now: Timestamp,
    need_mark: bool,
    /// The `layer_version` of the mark this change set read (`1` when it is
    /// about to write the first mark). A database at `2` has persisted
    /// derived data to keep in step with its sources.
    pub(crate) layer_version: u32,
    /// Persisted-derived writes so far (ADR-0020): they make the change set
    /// non-empty and decide its purpose when it has no members.
    pub(crate) derived_ops: usize,
    /// Public id records written so far (ADR-0005): they raise the mark to
    /// layer version 3 and always travel with a member.
    pub(crate) token_ops: usize,
    /// Units `require_active` found `Active` in this change set. Every write
    /// goes through [`ChangeSet::write`], which drops a unit from here when
    /// it writes or deletes its `UNIT_HEAD` (and clears it on a range
    /// delete), so an entry is never stale.
    active_known: std::collections::HashSet<UnitId, std::hash::BuildHasherDefault<IdHasher>>,
    /// A relation id at or above every `REL` record this change set sees (the
    /// transaction's view, its own writes included), once one is known: see
    /// [`ChangeSet::relation_id_is_free`].
    rel_ceiling: RelCeiling,
    /// Whether the units this change set creates have ids above every unit
    /// the database held: see [`ChangeSet::learn_fresh`].
    fresh: std::cell::Cell<Fresh>,
    /// Schema and index definitions written so far (ADR-0021): they make the
    /// change set non-empty and, with no member, give it the purpose `Index`.
    pub(crate) schema_ops: usize,
    /// Type-options records written or removed so far (ADR-0022): they make
    /// the change set non-empty and raise the mark to layer version 7.
    pub(crate) type_option_ops: usize,
    /// Trigram index changes planned so far, written at commit by
    /// [`ChangeSet::flush_substring`] (ADR-0025).
    pub(crate) substring_pending: crate::substring::PendingGrams,
    /// Adjacency changes and counts planned so far, written at commit by
    /// [`ChangeSet::flush_adjacency`] (ADR-0026). A second relation of the same
    /// `(unit, direction, type)` in one change set reads its count here instead of from the
    /// transaction (and never counts the entries again).
    pub(crate) adj_pending: crate::adjacency::PendingAdjacency,
    /// Posting changes and the statistics delta planned so far, written at
    /// commit by [`ChangeSet::flush_fulltext`] (ADR-0026).
    pub(crate) text_pending: crate::fulltext::PendingPostings,
    pub(crate) text_stats: Option<(i64, i64)>,
    /// The keyword ordinals this change set gave out: the first (`next_ord` of `STATS` when it
    /// began) and the next to give (`units-v12.md`). `None` until the first is given.
    pub(crate) text_ords: std::cell::Cell<Option<(u64, u64)>>,
    members: Vec<(RevisionId, RevisionKind)>,
    /// Relations created or deleted so far (E4-02): they are not members, but
    /// they count against `max_changeset_ops` and make the change set non-empty.
    relation_ops: usize,
    touched: BTreeSet<UnitId>,
    broken: bool,
    /// A `set_policy` change set: commits without members.
    maintenance: bool,
    /// A primary-index build or failure marker (E4-03): purpose `Index`,
    /// commits without members, and changes no policy.
    index: bool,
    /// Engine writes and their key + value bytes so far.
    writes: usize,
    bytes: usize,
    /// A retention change set: committed with deferred durability
    /// (decision 12c), see [`UnitDb::run_retention`].
    deferred: bool,
    /// The options byte of the unit types this change set looked up (`0`: no
    /// record), so a batch reads a type's options once. [`ChangeSet::write_owned`]
    /// empties it when a type-options record is written.
    pub(crate) options_cache: std::cell::RefCell<std::collections::HashMap<String, u8>>,
    /// The schemas this change set read, by unit type (`None`: no schema), so a batch decodes a
    /// type's schema once. [`ChangeSet::write_owned`] empties it when a schema record is written.
    /// The unique-field prefix (`field_prefix ‖ value`) an `upsert` just found held by no unit,
    /// while it creates the unit; cleared by any write.
    pub(crate) unique_free: std::cell::RefCell<Option<Vec<u8>>>,
    /// Per unique field (`field_prefix`), whether the snapshot `base` holds no entry of it at
    /// all, learned with one seek: then only this change set's own entries can hold a value.
    /// Emptied with `schema_cache`.
    pub(crate) base_field_empty: std::cell::RefCell<std::collections::HashMap<Vec<u8>, bool>>,
    /// A snapshot of the state the transaction began on, taken at its first use: what a read
    /// of keys this change set has not written answers from, without the transaction folding
    /// its buffer of writes for a scan (see [`ChangeSet::base`]).
    base: std::cell::OnceCell<Option<E::Snapshot>>,
    /// The change set deleted a range: the snapshot is no longer the transaction's view of the
    /// keys it did not write.
    ranges_deleted: bool,
    /// The entries of unique fields this change set put (`true`) or deleted (`false`), in
    /// order, by `field_prefix ‖ value`: with `base`, who holds a unique value.
    pub(crate) unique_staged: std::collections::HashMap<Vec<u8>, Vec<(UnitId, bool)>>,
    /// The live counts of the unit types this change set moved, by count key, written once at
    /// commit by [`ChangeSet::flush_live`].
    pub(crate) live_pending: std::collections::HashMap<Vec<u8>, u64>,
    /// The document of the payload a `create_prepared` is creating, cut ahead of it.
    pub(crate) prepared_text: std::cell::RefCell<Option<crate::fulltext::PreparedText>>,
    /// The content hash (with the payload length it was computed over) of the payload the next
    /// create or update writes, computed ahead of it (`create_prepared`, `upsert_prepared`).
    pub(crate) prepared_hash: std::cell::Cell<Option<(usize, ContentHash)>>,
    pub(crate) schema_cache:
        std::cell::RefCell<std::collections::HashMap<String, Option<std::sync::Arc<crate::schema::Schema>>>>,
    /// Whether the database held no `DERIVED_SRC` record when this change set
    /// first asked (`None`: not asked yet). Without one, and while the change
    /// set writes no derived record itself, no unit has derived data to follow.
    no_derived: std::cell::Cell<Option<bool>>,
}

/// Hashes a unit id for a set inside one change set: the id's own bytes, folded. An id is
/// random or a counter in its low bytes, and the set holds the change set's own units, so a
/// keyed hash buys nothing here and costs most of a lookup.
#[derive(Default)]
pub(crate) struct IdHasher(u64);

impl IdHasher {
    fn mix(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(29) ^ word).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

impl std::hash::Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for w in words {
            self.mix(u64::from_le_bytes(*w));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.mix(u64::from_le_bytes(word));
        }
        // The table takes its index from the low bits and its tag from the high ones.
        self.0 ^= self.0 >> 32;
    }

    // The length prefix of a slice (a unit id's 16 bytes) and the words of a key: no buffer.
    fn write_usize(&mut self, n: usize) {
        self.mix(n as u64);
    }

    fn write_u64(&mut self, n: u64) {
        self.mix(n);
        self.0 ^= self.0 >> 32;
    }
}

/// What a change set knows about the ids of the units it creates.
#[derive(Clone, Copy)]
enum Fresh {
    /// Not asked yet.
    Unknown,
    /// When asked, the transaction held no `UNIT_HEAD` with this id or a
    /// higher one: a unit with such an id is created by this change set,
    /// after every unit that existed.
    From(UnitId),
    /// Asked about a unit that exists, or one below a stored unit.
    No,
}

/// What a change set knows about the highest `REL` record it sees.
#[derive(Clone, Copy)]
enum RelCeiling {
    /// Not asked yet.
    Unknown,
    /// No `REL` record of the transaction's view has an id above this one.
    At(RelationId),
    /// A fresh id was minted below a stored one (a clock set back, an import
    /// of later ids): every id is checked by a point read.
    Unordered,
}

/// One engine write of a planned operation.
#[derive(Clone)]
pub(crate) enum Write {
    Put(Vec<u8>, Vec<u8>),
    Delete(Vec<u8>),
    DeleteRange(Vec<u8>, Vec<u8>),
}

/// Records of one operation, encoded before any is written.
struct Planned {
    writes: Vec<Write>,
    rev: RevisionId,
    kind: RevisionKind,
    /// How the operation moves the live count of a unit type (P5): `+1` when
    /// a unit becomes `Active`, `-1` when one stops being.
    live: Option<(String, i64)>,
}

impl<'db, E: TxnEngine + 'db> ChangeSet<'db, E> {
    /// The sequence this change set will commit with.
    pub fn commit_seq(&self) -> CommitSeq {
        self.seq
    }

    /// Checks before a persisted-derived write: the change set is open and
    /// has room for one more operation.
    pub(crate) fn admit_derived(&self) -> Result<(), UnitError> {
        self.txn()?;
        let ops = self.members.len() + self.relation_ops + self.derived_ops;
        if ops >= self.db.cfg.max_changeset_ops {
            return Err(UnitError::ChangeSetTooLarge { ops: ops + 1, bytes: self.bytes, engine_limit: None });
        }
        Ok(())
    }

    /// Whether `unit` has a revision in this change set.
    pub(crate) fn is_touched(&self, unit: UnitId) -> bool {
        self.touched.contains(&unit)
    }

    /// The state the transaction began on, as a snapshot, while it is still the transaction's
    /// view of every key this change set did not write (`None` once it deleted a range, or if
    /// the engine's snapshot is not at the transaction's sequence). The write slot is held from
    /// `begin_write` on, so nothing commits in between.
    pub(crate) fn base(&self) -> Result<Option<&E::Snapshot>, UnitError> {
        if self.ranges_deleted {
            return Ok(None);
        }
        if self.base.get().is_none() {
            let snap = self.db.engine.snapshot()?;
            let same = snap.seq() == self.txn()?.read_seq();
            let _ = self.base.set(same.then_some(snap));
        }
        Ok(self.base.get().and_then(Option::as_ref))
    }

    /// Records a write of the field-index entry `k` when its field is unique (see
    /// `unique_staged`).
    fn stage_unique(&mut self, k: &[u8], put: bool) -> Result<(), UnitError> {
        let Ok((unit_type, field, _, unit)) = crate::schema::entry_key_parts(k) else {
            return Ok(());
        };
        let unique = self.read_schema(unit_type)?.is_some_and(|s| s.field(field).is_some_and(|f| f.unique));
        if unique {
            self.unique_staged.entry(k[..k.len() - 16].to_vec()).or_default().push((unit, put));
        }
        Ok(())
    }

    pub(crate) fn txn(&self) -> Result<&E::Txn<'db>, UnitError> {
        match &self.txn {
            Some(t) if !self.broken => Ok(t),
            _ => Err(UnitError::ChangeSetClosed),
        }
    }

    /// Checks shared by every operation, before reading anything.
    fn admit(&self, unit: Option<UnitId>) -> Result<(), UnitError> {
        self.txn()?;
        let ops = self.members.len() + self.relation_ops;
        if ops >= self.db.cfg.max_changeset_ops {
            return Err(UnitError::ChangeSetTooLarge { ops: ops + 1, bytes: self.bytes, engine_limit: None });
        }
        if let Some(u) = unit
            && self.touched.contains(&u)
        {
            return Err(UnitError::InvalidArgument(format!("unit {u} already has a revision in this change set")));
        }
        Ok(())
    }

    fn check_payload(&self, payload: &[u8]) -> Result<(), UnitError> {
        let max = self.db.cfg.max_payload_len;
        if payload.len() > max {
            return Err(UnitError::PayloadTooLarge { len: payload.len(), max });
        }
        Ok(())
    }

    fn check_metadata(metadata: &[u8]) -> Result<(), UnitError> {
        if metadata.len() > MAX_METADATA_LEN as usize {
            return Err(UnitError::InvalidArgument(format!(
                "metadata of {} bytes exceeds {MAX_METADATA_LEN}",
                metadata.len()
            )));
        }
        Ok(())
    }

    /// Writes one put; on an engine error the change set is broken.
    /// Raises the layer mark to `version` in this change set, when it is below (never lowers
    /// it). A new database writes its mark at commit, at the raised version.
    pub(crate) fn raise_layer(&mut self, version: u32) -> Result<(), UnitError> {
        if self.layer_version >= version {
            return Ok(());
        }
        if !self.need_mark {
            let Some(raw) = self.txn()?.get(&layer_mark_key())? else {
                return Err(corruption("the layer mark is gone".into()));
            };
            let mut mark = LayerMeta::decode_upto(&raw, MAX_LAYER_VERSION).map_err(|e| context(e, "LAYER_META \"layer\""))?;
            mark.layer_version = version;
            let v = mark.encode()?;
            self.put(&layer_mark_key(), &v)?;
        }
        self.layer_version = version;
        Ok(())
    }

    fn put(&mut self, k: &[u8], v: &[u8]) -> Result<(), UnitError> {
        self.write_owned(Write::Put(k.to_vec(), v.to_vec()))
    }

    /// Writes one engine operation; on an engine error the change set is broken.
    pub(crate) fn write(&mut self, w: &Write) -> Result<(), UnitError> {
        self.write_owned(w.clone())
    }

    /// [`write`](Self::write) of an operation the caller no longer needs: the
    /// engine keeps its buffers instead of copying them (the path of every
    /// record of a unit or a relation).
    pub(crate) fn write_owned(&mut self, w: Write) -> Result<(), UnitError> {
        if self.txn.is_none() {
            return Err(UnitError::ChangeSetClosed);
        }
        // Before the write: a write that fails breaks the change set anyway.
        match &w {
            Write::DeleteRange(..) => self.ranges_deleted = true,
            Write::Put(k, _) | Write::Delete(k) if k.first() == Some(&crate::schema::TAG_FIELD_INDEX) => {
                self.stage_unique(k, matches!(w, Write::Put(..)))?;
            }
            _ => {}
        }
        let Some(txn) = self.txn.as_mut() else {
            return Err(UnitError::ChangeSetClosed);
        };
        let len = match &w {
            Write::Put(k, v) | Write::DeleteRange(k, v) => k.len() + v.len(),
            Write::Delete(k) => k.len(),
        };
        let (writes, bytes) = (self.writes + 1, self.bytes + len);
        // A type-options record, or a range that may cover some: forget what was read.
        let options_stale = match &w {
            Write::Put(k, _) | Write::Delete(k) => k.first() == Some(&crate::type_options::TAG_TYPE_OPTIONS),
            Write::DeleteRange(..) => true,
        };
        if options_stale {
            self.options_cache.get_mut().clear();
        }
        let schema_stale = match &w {
            Write::Put(k, _) | Write::Delete(k) => k.first() == Some(&crate::schema::TAG_SCHEMA),
            Write::DeleteRange(..) => true,
        };
        if schema_stale {
            self.schema_cache.get_mut().clear();
            self.base_field_empty.get_mut().clear();
        }
        *self.unique_free.get_mut() = None;
        // A `REL` record above the ceiling raises it, whoever writes it (a
        // relation created here, one imported with its own id).
        if let (RelCeiling::At(top), Write::Put(k, _)) = (self.rel_ceiling, &w)
            && k.len() == 17
            && k[0] == crate::codec::TAG_REL
            && k[1..] > top.0[..]
        {
            let mut id = [0u8; 16];
            id.copy_from_slice(&k[1..]);
            self.rel_ceiling = RelCeiling::At(RelationId(id));
        }
        if !self.active_known.is_empty() {
            match &w {
                Write::Put(k, _) | Write::Delete(k) => {
                    if k.len() == 17 && k[0] == crate::codec::TAG_UNIT_HEAD {
                        let mut id = [0u8; 16];
                        id.copy_from_slice(&k[1..]);
                        self.active_known.remove(&UnitId(id));
                    }
                }
                Write::DeleteRange(..) => self.active_known.clear(),
            }
        }
        let r = match w {
            Write::Put(k, v) => txn.put_owned(k, v),
            Write::Delete(k) => txn.delete(&k),
            Write::DeleteRange(s, e) => txn.delete_range(&s, &e),
        };
        match r {
            Ok(()) => {
                self.writes = writes;
                self.bytes = bytes;
                Ok(())
            }
            Err(e) => {
                self.broken = true;
                Err(match e {
                    // The engine refuses a write over its batch or value
                    // limits before any effect (design §4).
                    kerno_core::Error::InvalidArgument(m) => {
                        UnitError::ChangeSetTooLarge { ops: writes, bytes, engine_limit: Some(m) }
                    }
                    e => UnitError::Storage(e),
                })
            }
        }
    }

    /// Runs one public operation and closes the change set when it fails after writing.
    /// A write error already breaks it; this also covers an error from any later step, so an
    /// operation that left part of its effect in the transaction cannot be committed (TXN-01).
    /// An operation that fails before its first write leaves the change set usable.
    pub(crate) fn guarded<T, X>(&mut self, op: impl FnOnce(&mut Self) -> Result<T, X>) -> Result<T, X> {
        let before = self.writes;
        let result = op(self);
        if result.is_err() && self.writes != before {
            self.broken = true;
        }
        result
    }

    /// [`apply`](Self::apply) without the derived follow-up, for a plan that
    /// already carries it.
    fn apply_planned_only(&mut self, plan: Planned, live: Option<(Vec<u8>, u64)>) -> Result<RevNo, UnitError> {
        if let Some(l) = live {
            self.put_live(l)?;
        }
        for w in plan.writes {
            self.write_owned(w)?;
        }
        self.touched.insert(plan.rev.unit);
        self.members.push((plan.rev, plan.kind));
        Ok(plan.rev.rev)
    }

    fn apply(&mut self, plan: Planned) -> Result<RevNo, UnitError> {
        // The persisted derived data of the unit follow the source in this
        // same transaction (ADR-0020 §S3): read before the first write, so a
        // refusal leaves the change set untouched.
        let mut derived = self.derived_follow(plan.rev.unit, plan.kind)?;
        // A deleted unit takes its public id mapping with it, so its token
        // then resolves like one never issued (ADR-0005).
        if plan.kind == RevisionKind::Delete {
            derived.extend(self.plan_token_removal(plan.rev.unit)?);
        }
        // The live count of the unit's type moves in this same transaction
        // (P5). Planned here, by reads, before the first write: a refusal
        // leaves no count without its head.
        let live = plan.live.as_ref().map(|(t, d)| self.plan_live(t, *d)).transpose()?;
        // The full-text entries of the unit follow its text in this same
        // transaction, planned by reads like the count above.
        let text = self.plan_fulltext(plan.rev.unit, &plan.writes)?;
        // So do its trigram entries, when its type is searched by substring.
        let grams = self.plan_substring(plan.rev.unit, &plan.writes)?;
        let unit = plan.rev.unit;
        // The schema check and the user index entries of the unit follow it
        // too; a unit the schema refuses is refused here, before any write.
        let fields = self.plan_schema(plan.rev.unit, &plan.writes)?;
        if let Some(l) = live {
            self.put_live(l)?;
        }
        for w in plan.writes.into_iter().chain(derived).chain(text.writes).chain(fields) {
            self.write_owned(w)?;
        }
        self.stage_fulltext(text.change);
        self.stage_substring(unit, grams);
        self.touched.insert(plan.rev.unit);
        self.members.push((plan.rev, plan.kind));
        Ok(plan.rev.rev)
    }

    /// What the persisted derived data of `unit` need when a revision of
    /// `kind` is written: stale marks for a change, removal for a deletion.
    /// Nothing for a database below layer version 2.
    fn derived_follow(&self, unit: UnitId, kind: RevisionKind) -> Result<Vec<Write>, UnitError> {
        if self.layer_version < LAYER_VERSION_DERIVED && self.derived_ops == 0 {
            return Ok(Vec::new());
        }
        let txn = self.txn()?;
        // A unit being created has no derived data yet, and neither has any unit of a database
        // without a single `DERIVED_SRC` record: one probe per change set instead of a scan
        // per operation. A derived write of this change set makes the probe stale, so it
        // then counts for nothing.
        if kind == RevisionKind::Create {
            return Ok(Vec::new());
        }
        if self.derived_ops == 0 {
            let none = match self.no_derived.get() {
                Some(none) => none,
                None => {
                    let (start, end) = Key::tag_range(crate::codec::TAG_DERIVED_SRC);
                    let none = txn.scan(Bound::Included(&start), Bound::Excluded(&end), 1)?.is_empty();
                    self.no_derived.set(Some(none));
                    none
                }
            };
            if none {
                return Ok(Vec::new());
            }
        }
        let event = if kind == RevisionKind::Delete {
            crate::vector_store::SourceEvent::Deleted
        } else {
            crate::vector_store::SourceEvent::Changed
        };
        crate::vector_store::plan_source_event(&crate::vector_store::TxnReader(txn), unit, event)
    }

    fn actor(&self) -> Vec<u8> {
        self.meta.actor.clone().unwrap_or_default()
    }

    fn source(&self) -> Vec<u8> {
        self.meta.source.clone().unwrap_or_default()
    }

    /// The history policy in force for `unit_type`, as this change set sees it.
    pub(crate) fn policy(&self, unit_type: &str) -> Result<HistoryPolicy, UnitError> {
        let txn = self.txn()?;
        policy_of(&|k| txn.get(k), unit_type, self.db.cfg.default_policy)
    }

    /// What the policy does to the revisions `head` supersedes (see the
    /// [module documentation](self)); may move `head.oldest_rev`. `outgoing`
    /// is the payload of the revision superseded: when the old head carried
    /// it, it has no `REV_PAYLOAD` yet (`units-v11.md` §2), and a history that
    /// keeps it gets one now.
    fn supersede(&self, unit: UnitId, head: &mut UnitHead, outgoing: Option<&[u8]>) -> Result<Vec<Write>, UnitError> {
        let rev = head.head_rev;
        if rev.0 == 1 {
            return Ok(Vec::new());
        }
        match self.policy(&head.unit_type)? {
            HistoryPolicy::Versioned => match outgoing.filter(|p| UnitHead::inline_of(Some(p)).is_some()) {
                Some(p) => Ok(vec![Write::Put(key(Key::RevPayload { unit, rev: RevNo(rev.0 - 1) })?, p.to_vec())]),
                None => Ok(Vec::new()),
            },
            HistoryPolicy::Audit => {
                let prev = RevNo(rev.0 - 1);
                let txn = self.txn()?;
                let mut meta = crate::history::read_meta(&|k| txn.get(k), unit, prev)?;
                if !meta.payload_present {
                    return Ok(Vec::new());
                }
                meta.payload_present = false;
                Ok(vec![
                    Write::Put(key(Key::RevMeta { unit, rev: prev })?, meta.encode(prev)?),
                    Write::Delete(key(Key::RevPayload { unit, rev: prev })?),
                ])
            }
            HistoryPolicy::None => {
                head.oldest_rev = rev;
                let mut out = Vec::with_capacity(2);
                for (tag, k) in [
                    (crate::codec::TAG_REV_META, Key::RevMeta { unit, rev }),
                    (crate::codec::TAG_REV_PAYLOAD, Key::RevPayload { unit, rev }),
                ] {
                    let (start, _) = Key::unit_rev_range(tag, unit);
                    out.push(Write::DeleteRange(start, key(k)?));
                }
                Ok(out)
            }
        }
    }

    /// The records of a new revision `rev` of `unit`: what its policy does
    /// to the superseded revision (whose payload is `outgoing`), head,
    /// `REV_META`, `REV_PAYLOAD` (when there is a payload the head does not
    /// carry) and `CS_MEMBER`.
    fn plan(
        &self,
        unit: UnitId,
        mut head: UnitHead,
        kind: RevisionKind,
        payload: Option<std::borrow::Cow<'_, [u8]>>,
        restored_from: Option<RevNo>,
        outgoing: Option<&[u8]>,
    ) -> Result<Planned, UnitError> {
        let mut writes = self.supersede(unit, &mut head, outgoing)?;
        // The head carries the payload of the revision it now names, never an older one's.
        head.inline_payload = UnitHead::inline_of(payload.as_deref());
        let rev = head.head_rev;
        let meta = RevisionMeta {
            kind,
            parent_rev: (rev.0 > 1).then(|| RevNo(rev.0 - 1)),
            commit_seq: self.seq,
            created_at: self.now,
            schema_version: head.schema_version,
            content_hash: head.content_hash,
            payload_len: head.payload_len,
            payload_present: payload.is_some(),
            restored_from,
            metadata: head.metadata.clone(),
            actor_ref: self.actor(),
            source_ref: self.source(),
        };
        let ordinal = u32::try_from(self.members.len()).map_err(|_| UnitError::ChangeSetTooLarge {
            ops: self.members.len() + 1,
            bytes: self.bytes,
            engine_limit: None,
        })?;
        let member = CsMember { unit, rev_no: rev, kind };
        writes.push(Write::Put(key(Key::UnitHead { unit })?, head.encode()?));
        writes.push(Write::Put(key(Key::RevMeta { unit, rev })?, meta.encode(rev)?));
        // A payload the head carries is stored there only (`units-v11.md` §2).
        if let Some(p) = payload.filter(|_| head.inline_payload.is_none()) {
            // An owned payload (a create's) moves into the write: no copy.
            writes.push(Write::Put(key(Key::RevPayload { unit, rev })?, p.into_owned()));
        }
        writes.push(Write::Put(key(Key::CsMember { seq: self.seq, ordinal })?, member.encode()?));
        Ok(Planned { writes, rev: RevisionId { unit, rev }, kind, live: None })
    }

    /// The current head of `id` (verified), refusing a missing unit, an
    /// `expected` revision that is not the head and a deleted unit, in that
    /// order.
    fn current(&self, id: UnitId, expected: Option<RevNo>) -> Result<Loaded, UnitError> {
        let txn = self.txn()?;
        // A write builds on the unit: read all of its head revision (IU-2), not the head alone.
        let Some((head, payload)) = load_unit_checked(&|k| txn.get(k), id)? else {
            return Err(UnitError::NotFound { unit: id });
        };
        if let Some(exp) = expected
            && exp != head.head_rev
        {
            return Err(UnitError::Conflict { unit: id, expected: exp, found: head.head_rev });
        }
        if head.lifecycle == Lifecycle::Deleted {
            return Err(UnitError::Deleted { unit: id });
        }
        Ok((head, payload))
    }

    /// The content hash of `payload`: the one computed ahead of this operation when there is one
    /// for a payload of its length, else computed here.
    fn content_hash_of(&self, payload: &[u8]) -> ContentHash {
        match self.prepared_hash.take() {
            Some((len, hash)) if len == payload.len() => hash,
            _ => ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, payload),
        }
    }

    fn next_head(&self, head: &UnitHead) -> UnitHead {
        UnitHead { head_rev: RevNo(head.head_rev.0 + 1), updated_seq: self.seq, updated_at: self.now, ..head.clone() }
    }

    /// Creates a unit with a fresh id from the id source and returns it.
    /// An id that already exists is [`UnitError::AlreadyExists`] (a deleted
    /// unit's id is never reused).
    pub fn create(&mut self, new: NewUnit) -> Result<UnitId, UnitError> {
        self.guarded(|s| s.create_unguarded(new))
    }

    fn create_unguarded(&mut self, new: NewUnit) -> Result<UnitId, UnitError> {
        self.admit(None)?;
        if new.unit_type.is_empty() || new.unit_type.len() > MAX_UNIT_TYPE_LEN as usize {
            return Err(UnitError::InvalidArgument(format!(
                "unit_type of {} bytes is outside 1..={MAX_UNIT_TYPE_LEN}",
                new.unit_type.len()
            )));
        }
        Self::check_metadata(&new.metadata)?;
        self.check_payload(&new.payload)?;
        let id = self.db.cfg.ids.next_id();
        if self.touched.contains(&id) {
            return Err(UnitError::AlreadyExists { unit: id });
        }
        // Ids are minted ascending: once the change set knows (one seek) that no stored unit has
        // an id at or above one it created, a later id needs no read to know it is new (a unit
        // created in this change set is in `touched`).
        self.learn_fresh(id)?;
        let fresh = self.fresh_from().is_some_and(|f| id >= f);
        if !fresh && self.txn()?.get(&key(Key::UnitHead { unit: id })?)?.is_some() {
            return Err(UnitError::AlreadyExists { unit: id });
        }
        let head = UnitHead {
            unit_type: new.unit_type,
            schema_version: new.schema_version,
            lifecycle: Lifecycle::Active,
            head_rev: RevNo(1),
            oldest_rev: RevNo(1),
            created_seq: self.seq,
            updated_seq: self.seq,
            created_at: self.now,
            updated_at: self.now,
            content_hash: self.content_hash_of(&new.payload),
            payload_len: new.payload.len() as u32,
            metadata: new.metadata,
            inline_payload: None,
        };
        let unit_type = head.unit_type.clone();
        let mut plan =
            self.plan(id, head, RevisionKind::Create, Some(std::borrow::Cow::Owned(new.payload)), None, None)?;
        plan.live = Some((unit_type, 1));
        self.apply(plan)?;
        Ok(id)
    }

    /// A new revision with new content. The unit must be `Active`.
    pub fn update(&mut self, id: UnitId, upd: UnitUpdate, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.guarded(|s| s.update_unguarded(id, upd, expected))
    }

    fn update_unguarded(&mut self, id: UnitId, upd: UnitUpdate, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.admit(Some(id))?;
        self.check_payload(&upd.payload)?;
        if let Some(m) = &upd.metadata {
            Self::check_metadata(m)?;
        }
        let (head, old) = self.current(id, expected)?;
        if head.lifecycle != Lifecycle::Active {
            return Err(UnitError::InvalidArgument(format!("unit {id} is archived; unarchive it before updating")));
        }
        let mut next = self.next_head(&head);
        next.lifecycle = Lifecycle::Active;
        next.content_hash = self.content_hash_of(&upd.payload);
        next.payload_len = upd.payload.len() as u32;
        if let Some(v) = upd.schema_version {
            next.schema_version = v;
        }
        if let Some(m) = upd.metadata {
            next.metadata = m;
        }
        let plan = self.plan(
            id,
            next,
            RevisionKind::Update,
            Some(std::borrow::Cow::Owned(upd.payload)),
            None,
            old.as_deref(),
        )?;
        self.apply(plan)
    }

    /// `Archive` or `Unarchive`: a revision that copies the current payload.
    fn relifecycle(
        &mut self,
        id: UnitId,
        expected: Option<RevNo>,
        from: Lifecycle,
        to: Lifecycle,
        kind: RevisionKind,
    ) -> Result<RevNo, UnitError> {
        self.admit(Some(id))?;
        let (head, payload) = self.current(id, expected)?;
        if head.lifecycle != from {
            let (is, need) = (format!("{:?}", head.lifecycle).to_lowercase(), format!("{from:?}").to_lowercase());
            return Err(UnitError::InvalidArgument(format!("unit {id} is {is}, not {need}")));
        }
        if to == Lifecycle::Archived {
            self.ensure_no_relations(id)?;
        }
        let Some(payload) = payload else {
            return Err(corruption(format!("unit {id}: a {:?} head revision has no payload", head.lifecycle)));
        };
        let mut next = self.next_head(&head);
        next.lifecycle = to;
        let live = (next.unit_type.clone(), if to == Lifecycle::Archived { -1 } else { 1 });
        let mut plan = self.plan(id, next, kind, Some(std::borrow::Cow::Borrowed(&payload)), None, Some(&payload))?;
        plan.live = Some(live);
        self.apply(plan)
    }

    /// Archives an `Active` unit: a revision that copies its payload.
    pub fn archive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.guarded(|s| s.archive_unguarded(id, expected))
    }

    fn archive_unguarded(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.relifecycle(id, expected, Lifecycle::Active, Lifecycle::Archived, RevisionKind::Archive)
    }

    /// Makes an `Archived` unit `Active` again: a revision that copies its payload.
    pub fn unarchive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.guarded(|s| s.unarchive_unguarded(id, expected))
    }

    fn unarchive_unguarded(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.relifecycle(id, expected, Lifecycle::Archived, Lifecycle::Active, RevisionKind::Unarchive)
    }

    /// Deletes an `Active` or `Archived` unit logically: a tombstone revision
    /// without payload (hash of the empty payload, length 0) and the head
    /// kept with `lifecycle = Deleted` (design §3.2).
    pub fn delete(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.guarded(|s| s.delete_unguarded(id, expected))
    }

    fn delete_unguarded(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.admit(Some(id))?;
        let (head, old) = self.current(id, expected)?;
        self.ensure_no_relations(id)?;
        let mut next = self.next_head(&head);
        next.lifecycle = Lifecycle::Deleted;
        next.content_hash = ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, &[]);
        next.payload_len = 0;
        let mut plan = self.plan(id, next, RevisionKind::Delete, None, None, old.as_deref())?;
        // An archived unit already left the live count.
        plan.live = (head.lifecycle == Lifecycle::Active).then(|| (head.unit_type.clone(), -1));
        self.apply(plan)
    }

    /// Restores revision `from` (design §3.2, I-22): a new `Restore` revision
    /// whose payload, schema version and metadata are copies of `from`'s;
    /// every revision after `from` stays as it was. The unit must be
    /// `Active`; `from` must be a retained revision with a payload (not a
    /// tombstone). `from` above the head is [`UnitError::RevisionNotFound`];
    /// below the head under `None`/`Audit`, or one whose payload an `Audit`
    /// period dropped, [`UnitError::HistoryUnavailable`]; below the oldest
    /// retained one [`UnitError::Pruned`]. The source payload is verified
    /// before it is copied, and it must fit the [`UnitConfig::max_payload_len`]
    /// in force now ([`UnitError::PayloadTooLarge`] otherwise).
    pub fn restore(&mut self, id: UnitId, from: RevNo, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.guarded(|s| s.restore_unguarded(id, from, expected))
    }

    fn restore_unguarded(&mut self, id: UnitId, from: RevNo, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.admit(Some(id))?;
        let (head, old) = self.current(id, expected)?;
        if head.lifecycle != Lifecycle::Active {
            return Err(UnitError::InvalidArgument(format!(
                "unit {id} is {:?}; only an Active unit is restored",
                head.lifecycle
            )));
        }
        let policy = self.policy(&head.unit_type)?;
        let txn = self.txn()?;
        let get = |k: &[u8]| txn.get(k);
        crate::history::check_bounds(&head, id, from, policy)?;
        let source = crate::history::read_meta(&get, id, from)?;
        crate::history::check_content(source.kind, source.payload_present)?;
        let Some(payload) = crate::history::read_payload(&get, id, from, &source.payload_decl())? else {
            return Err(UnitError::InvalidArgument(format!("revision {from} of unit {id} is a tombstone")));
        };
        // The restored content is a new revision: it obeys the limit in force
        // now, like `update` (E3-06 note), even if it was written under a
        // larger one.
        self.check_payload(&payload)?;
        let mut next = self.next_head(&head);
        next.lifecycle = Lifecycle::Active;
        next.content_hash = source.content_hash;
        next.payload_len = source.payload_len;
        next.schema_version = source.schema_version;
        next.metadata = source.metadata;
        let plan = self.plan(
            id,
            next,
            RevisionKind::Restore,
            Some(std::borrow::Cow::Borrowed(&payload)),
            Some(from),
            old.as_deref(),
        )?;
        self.apply(plan)
    }

    /// Refuses with [`UnitError::UnitHasRelations`] when a relation, in
    /// either direction, touches `unit` as this change set sees it (ADR-0014,
    /// criterion 4): the blocks of the transaction and the adjacency changes
    /// the change set keeps ([`ChangeSet::relation_of_unit`]).
    fn ensure_no_relations(&self, unit: UnitId) -> Result<(), UnitError> {
        match self.relation_of_unit(unit)? {
            Some(relation) => Err(UnitError::UnitHasRelations { unit, relation }),
            None => Ok(()),
        }
    }

    /// The lifecycle of `unit` as this change set sees it, reading only its
    /// `UNIT_HEAD`; `NotFound` or `Deleted` for a unit that cannot take a
    /// relation, `InvalidArgument` for an archived one.
    fn require_active(&mut self, unit: UnitId) -> Result<(), UnitError> {
        if self.active_known.contains(&unit) {
            return Ok(());
        }
        let txn = self.txn()?;
        // `UNIT_HEAD` is `tag ‖ unit`: built on the stack, and only the lifecycle is read (the
        // head carries the payload of a small unit, which a relation does not need).
        let mut head_key = [0u8; 17];
        head_key[0] = crate::codec::TAG_UNIT_HEAD;
        head_key[1..].copy_from_slice(&unit.0);
        let Some(raw) = txn.get(&head_key)? else {
            return Err(UnitError::NotFound { unit });
        };
        let lifecycle =
            UnitHead::decode_lifecycle(&raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
        match lifecycle {
            Lifecycle::Active => {
                self.active_known.insert(unit);
                Ok(())
            }
            Lifecycle::Deleted => Err(UnitError::Deleted { unit }),
            Lifecycle::Archived => Err(UnitError::InvalidArgument(format!(
                "unit {unit} is archived; a relation needs Active units at both ends"
            ))),
        }
    }

    /// Creates the relation `from -> to` of type `relation_type` (1..=64 B,
    /// no NUL) with a fresh id from the id source (ADR-0014, option 2): the
    /// `REL` record and its two adjacency entries are written in this change
    /// set, so they commit together or not at all (I-21). Both units must be
    /// `Active` as this change set sees them, which includes units it
    /// creates. `provenance` is mandatory and checked: see
    /// [`RelationProvenance`]. The actor, source and reason of the change set
    /// are the relation's. Parallel relations (same ends and type) are
    /// allowed; a unit may relate to itself.
    pub fn create_relation(
        &mut self,
        from: UnitId,
        to: UnitId,
        relation_type: &str,
        provenance: RelationProvenance,
    ) -> Result<RelationId, UnitError> {
        self.guarded(|s| s.create_relation_unguarded(from, to, relation_type, provenance))
    }

    fn create_relation_unguarded(
        &mut self,
        from: UnitId,
        to: UnitId,
        relation_type: &str,
        provenance: RelationProvenance,
    ) -> Result<RelationId, UnitError> {
        self.admit_relation()?;
        if !valid_relation_type(relation_type) {
            let max = crate::codec::MAX_RELATION_TYPE_LEN;
            let rule = if relation_type.contains('\0') {
                "it contains a NUL".to_owned()
            } else {
                format!("its length of {} bytes is outside 1..={max}", relation_type.len())
            };
            return Err(UnitError::InvalidArgument(format!("relation_type is refused: {rule}")));
        }
        self.require_active(from)?;
        self.require_active(to)?;
        let edge = RelationId(self.db.cfg.ids.next_id().0);
        let rel_key = key(Key::Relation { edge })?;
        if !self.relation_id_is_free(edge, &rel_key)? {
            return Err(UnitError::InvalidArgument(format!("relation id {edge} already exists")));
        }
        let record =
            RelationRecord { from, to, relation_type: relation_type.to_owned(), created_seq: self.seq, provenance };
        // Everything that can refuse the relation (the record's encode, the
        // key) runs before a count is touched: an error after the counts
        // would leave them in the open change set without their entries.
        let write = Write::Put(rel_key, record.encode()?);
        // Counts before the entries: the count tells whether the lists have a block.
        let plans = self.plan_relation(from, to, relation_type, 1)?;
        self.apply_relation(write, edge, (from, to), plans, true)?;
        Ok(edge)
    }

    /// Deletes a relation: its record and both adjacency entries, in this
    /// change set. Relations have no history of their own (ADR-0014): the
    /// change set that deleted it is the only trace.
    pub fn delete_relation(&mut self, edge: RelationId) -> Result<(), UnitError> {
        self.guarded(|s| s.delete_relation_unguarded(edge))
    }

    fn delete_relation_unguarded(&mut self, edge: RelationId) -> Result<(), UnitError> {
        self.admit_relation()?;
        let txn = self.txn()?;
        let Some(raw) = txn.get(&key(Key::Relation { edge })?)? else {
            return Err(UnitError::RelationNotFound { relation: edge });
        };
        let r = RelationRecord::decode(&raw).map_err(|e| context(e, &format!("REL {edge}")))?;
        let write = Write::Delete(key(Key::Relation { edge })?);
        let plans = self.plan_relation(r.from, r.to, &r.relation_type, -1)?;
        self.apply_relation(write, edge, (r.from, r.to), plans, false)
    }

    /// Whether no `REL` record with the id `edge` (key `rel_key`) exists as
    /// this change set sees it.
    ///
    /// Ids are minted in ascending order, so the id of a new relation is
    /// almost always above every stored one. The change set learns that once,
    /// with one seek from the first id it mints to the end of the `REL`
    /// records: nothing there means no record has that id or a higher one.
    /// From then on it keeps the highest id it knows of (`rel_ceiling`, which
    /// [`write_owned`](Self::write_owned) raises with every `REL` record
    /// written through this change set), and an id above it cannot exist: no
    /// read at all. An id at or below the ceiling, or a database that holds
    /// ids above the fresh ones, is answered by the point read, as before.
    /// A deletion only removes records, so it never invalidates the ceiling.
    fn relation_id_is_free(&mut self, edge: RelationId, rel_key: &[u8]) -> Result<bool, UnitError> {
        match self.rel_ceiling {
            RelCeiling::At(top) if edge > top => return Ok(true),
            RelCeiling::Unknown => {
                let (_, end) = Key::tag_range(crate::codec::TAG_REL);
                if self.txn()?.scan(Bound::Included(rel_key), Bound::Excluded(&end), 1)?.is_empty() {
                    // No record has an id above `edge` (nor `edge` itself,
                    // which the ceiling does not claim: an id equal to it
                    // is read again).
                    self.rel_ceiling = RelCeiling::At(edge);
                    return Ok(true);
                }
                self.rel_ceiling = RelCeiling::Unordered;
            }
            _ => {}
        }
        Ok(self.txn()?.get(rel_key)?.is_none())
    }

    /// Told of a unit an index is about to take in for the first time, before
    /// the unit's `UNIT_HEAD` is written: learns, once per change set and with
    /// one seek, whether the units created from here on are above every unit
    /// the database holds (ids are minted in ascending order, so a bulk load
    /// creates nothing else).
    ///
    /// [`fresh_from`](Self::fresh_from) is what the index flushes use it for:
    /// an id at or above it belongs to a unit this change set created, so no
    /// stored posting block holds it or a higher id, and its entries go after
    /// every entry of their list. A list with no tail block then needs no
    /// seek to find its other blocks (`blocks::flush`).
    pub(crate) fn learn_fresh(&self, unit: UnitId) -> Result<(), UnitError> {
        if !matches!(self.fresh.get(), Fresh::Unknown) {
            return Ok(());
        }
        let (_, end) = Key::tag_range(crate::codec::TAG_UNIT_HEAD);
        let from = key(Key::UnitHead { unit })?;
        let none = self.txn()?.scan(Bound::Included(&from), Bound::Excluded(&end), 1)?.is_empty();
        self.fresh.set(if none { Fresh::From(unit) } else { Fresh::No });
        Ok(())
    }

    /// The id from which every unit is one this change set created, above
    /// every unit that existed before it, when [`learn_fresh`](Self::learn_fresh) found one.
    pub(crate) fn fresh_from(&self) -> Option<UnitId> {
        match self.fresh.get() {
            Fresh::From(unit) => Some(unit),
            _ => None,
        }
    }

    fn admit_relation(&self) -> Result<(), UnitError> {
        self.txn()?;
        let ops = self.members.len() + self.relation_ops;
        if ops >= self.db.cfg.max_changeset_ops {
            return Err(UnitError::ChangeSetTooLarge { ops: ops + 1, bytes: self.bytes, engine_limit: None });
        }
        Ok(())
    }

    /// Writes the `REL` record of a relation and keeps its adjacency entries
    /// (`add`: they are put, else removed) for commit.
    pub(crate) fn apply_relation(
        &mut self,
        write: Write,
        edge: RelationId,
        ends: (UnitId, UnitId),
        plans: [crate::adj_count::ListPlan; 2],
        add: bool,
    ) -> Result<(), UnitError> {
        self.write_owned(write)?;
        self.stage_adjacency(edge, ends, plans, add);
        self.relation_ops += 1;
        Ok(())
    }

    /// The unit as this change set sees it, its own writes included.
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        let txn = self.txn()?;
        Ok(load_unit(&|k| txn.get(k), id, true)?.map(|(head, payload)| to_unit(id, head, payload)))
    }

    /// Commits every operation as one engine commit (I-21). A change set
    /// without operations writes nothing and returns the current sequence.
    pub fn commit(mut self) -> Result<CommitInfo, UnitError> {
        self.txn()?;
        self.flush_live()?;
        self.flush_adjacency()?;
        // The keyword index before the trigram index: their keys ascend in that order, and a
        // flush that only writes above what was written so far lets the transaction answer its
        // seeks without ordering those writes first (the two do not read each other's keys).
        self.flush_fulltext()?;
        self.flush_substring()?;
        let durable = self.db.cfg.engine.durability == Durability::Sync;
        if self.members.is_empty()
            && !self.maintenance
            && !self.index
            && self.relation_ops == 0
            && self.derived_ops == 0
            && self.schema_ops == 0
            && self.type_option_ops == 0
        {
            let current = CommitSeq(self.seq.0 - 1);
            if let Some(t) = self.txn.take() {
                t.abort();
            }
            // A deferred retention commit (decision 12c) may have left
            // `current` not yet synced: durable only if the engine says so.
            // An engine that reports no `durable_seq` syncs every commit.
            let durable = durable
                && kerno_core::contract::Engine::stats(&self.db.engine)
                    .into_iter()
                    .find(|(n, _)| n == "durable_seq")
                    .is_none_or(|(_, v)| v >= current.0);
            return Ok(CommitInfo { change_set: current, commit_seq: current, revisions: Vec::new(), durable });
        }
        // The policies may change with this commit: no retention pass
        // resumes across it, and no snapshot taken while it is in flight
        // uses the policy cache (the epoch is odd until it ends, whatever
        // the outcome).
        let db = self.db;
        let _epoch = (self.need_mark || self.maintenance).then(|| {
            db.policy_epoch.fetch_add(1, Ordering::SeqCst);
            EpochBump(&db.policy_epoch)
        });
        // A new database starts at the cut version (ADR-0026), whatever its first
        // change set writes; an older one is refused at open, so a mark that
        // exists is already the one every writer uses.
        if self.need_mark {
            // The cut version, or higher when this change set raised it (`raise_layer`).
            let mark = LayerMeta { layer_version: self.layer_version, history_policy: self.db.cfg.default_policy };
            let v = mark.encode()?;
            self.put(&layer_mark_key(), &v)?;
        }
        let record = ChangeSetRecord {
            created_at: self.now,
            member_count: self.members.len() as u32,
            // A member-less application change set is relation-only: say so,
            // so a lost-members change set stays detectable (UNIT-30).
            purpose: if self.meta.purpose == Purpose::Application && self.members.is_empty() {
                // ADR-0020 §B.5: derived records only, no relation touched.
                if self.relation_ops > 0 {
                    Purpose::Relation
                } else if self.schema_ops > 0 || self.type_option_ops > 0 {
                    // Schema, index or type-option definitions only: no unit is a member.
                    Purpose::Index
                } else if self.derived_ops > 0 {
                    Purpose::Derived
                } else {
                    Purpose::Relation
                }
            } else {
                self.meta.purpose
            },
            reason: self.meta.reason.clone().map(String::into_bytes).unwrap_or_default(),
            actor_ref: self.actor(),
            source_ref: self.source(),
            correlation: self.meta.correlation.clone().unwrap_or_default(),
        };
        let v = record.encode()?;
        self.put(&key(Key::ChangeSet { seq: self.seq })?, &v)?;
        let txn = self.txn.take().ok_or(UnitError::ChangeSetClosed)?;
        // The engine measures a batch once more at commit, as it encodes: with its framing a
        // batch whose writes each passed can still be over the limit. It refuses before any
        // effect, and it is the same refusal as a write over the limit (#13).
        let (writes, bytes) = (self.writes, self.bytes);
        let too_large = |e: kerno_core::Error| match e {
            kerno_core::Error::InvalidArgument(m) => {
                UnitError::ChangeSetTooLarge { ops: writes, bytes, engine_limit: Some(m) }
            }
            e => UnitError::Storage(e),
        };
        let (got, durable) = if self.deferred {
            let (got, waited) = txn.commit_deferred().map_err(too_large)?;
            (got, durable && waited)
        } else {
            (txn.commit().map_err(too_large)?, durable)
        };
        if got != self.seq.0 {
            self.db.poisoned.store(true, Ordering::SeqCst);
            return Err(UnitError::Storage(kerno_core::Error::Poisoned(format!(
                "the engine committed the change set with sequence {got}, predicted {}",
                self.seq
            ))));
        }
        Ok(CommitInfo {
            change_set: self.seq,
            commit_seq: self.seq,
            revisions: self.members.iter().map(|(r, _)| *r).collect(),
            durable,
        })
    }

    /// An index change set (purpose `Index`, no caller provenance), for
    /// [`UnitDb::rebuild_index`]. Its commit sequence is the watermark it
    /// writes: it changes no unit, so the state it read is the state at
    /// that commit.
    pub(crate) fn begin_index(db: &'db UnitDb<E>) -> Result<Self, UnitError> {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        cs.meta.purpose = Purpose::Index;
        cs.index = true;
        Ok(cs)
    }

    /// A retention change set (purpose `Retention`, no caller provenance).
    pub(crate) fn begin_retention(db: &'db UnitDb<E>, now: Timestamp) -> Result<Self, UnitError> {
        let mut cs = db.begin_changeset_at(ChangeMeta::default(), Some(now))?;
        cs.meta.purpose = Purpose::Retention;
        cs.deferred = true;
        Ok(cs)
    }

    /// Writes what retention does to `unit` plus its member `(unit, rev,
    /// kind)` (the first retained revision); on an engine error the change
    /// set is broken.
    pub(crate) fn retain(
        &mut self,
        unit: UnitId,
        rev: RevNo,
        kind: RevisionKind,
        mut writes: Vec<Write>,
        live: Option<(&str, i64)>,
    ) -> Result<(), UnitError> {
        self.txn()?;
        if self.touched.contains(&unit) {
            return Err(UnitError::InvalidArgument(format!("unit {unit} already retained in this change set")));
        }
        let ordinal = u32::try_from(self.members.len()).map_err(|_| UnitError::ChangeSetTooLarge {
            ops: self.members.len() + 1,
            bytes: self.bytes,
            engine_limit: None,
        })?;
        let member = CsMember { unit, rev_no: rev, kind };
        writes.push(Write::Put(key(Key::CsMember { seq: self.seq, ordinal })?, member.encode()?));
        // A revision retention purges takes the derived data computed from it
        // (ADR-0020 §S5.1), in this same change set.
        if self.layer_version >= LAYER_VERSION_DERIVED {
            let txn = self.txn()?;
            let purge = crate::vector_store::SourceEvent::Purged(rev);
            writes.extend(crate::vector_store::plan_source_event(&crate::vector_store::TxnReader(txn), unit, purge)?);
        }
        // Last read of the operation: after it only writes remain, so a
        // refusal above leaves no count without its head (P5).
        let live = live.map(|(t, d)| self.plan_live(t, d)).transpose()?;
        // An import writes the head of a unit here: its text is indexed too.
        let text = self.plan_fulltext(unit, &writes)?;
        writes.extend(text.writes);
        self.apply_planned_only(Planned { writes, rev: RevisionId { unit, rev }, kind, live: None }, live)?;
        self.stage_fulltext(text.change);
        Ok(())
    }

    /// Discards the change set (same as dropping it).
    pub fn abort(self) {}
}

#[cfg(test)]
mod accessor_tests {
    use super::*;
    use kerno_testkit::emulated::Emulated;
    use kerno_testkit::reference::LogEngine;
    use kerno_testkit::vfs::SimVfs;

    fn open<E: TxnEngine>(disk: &SimVfs) -> UnitDb<E> {
        let cfg = UnitConfig::default();
        let engine = E::open(disk.as_vfs(), Path::new("db"), &cfg.engine).expect("open engine");
        UnitDb::from_engine(engine, &cfg).expect("open layer")
    }

    fn raw_accessors<E: TxnEngine>() {
        let disk = SimVfs::new(0);
        let db = open::<E>(&disk);
        let k = b"\xf0raw-key".to_vec();
        let other = b"\xf0other".to_vec();
        assert_eq!(db.raw_get(&k).unwrap(), None);
        let before = db.raw_seq().unwrap();
        let seq = db.raw_commit(vec![(k.clone(), Some(b"v1".to_vec()))]).unwrap();
        assert!(seq > before, "a raw commit moves the sequence");
        assert_eq!(db.raw_seq().unwrap(), seq);
        // What one engine commit moves the sequence by.
        let one_commit = seq - before;
        assert_eq!(db.raw_get(&k).unwrap(), Some(b"v1".to_vec()));
        // Two different keys stored: each reads its own value.
        db.raw_commit(vec![(other.clone(), Some(b"v2".to_vec()))]).unwrap();
        assert_eq!(db.raw_get(&k).unwrap(), Some(b"v1".to_vec()));
        assert_eq!(db.raw_get(&other).unwrap(), Some(b"v2".to_vec()));
        assert_eq!(db.raw_get(b"\xf0absent").unwrap(), None);
        // Several entries in one commit, two keys repeated: the last entry
        // of each wins, whether it puts or deletes. The sequence moves by one
        // commit, not by one commit per entry.
        let before = db.raw_seq().unwrap();
        let seq = db
            .raw_commit(vec![
                (b"\xf0m1".to_vec(), Some(b"a".to_vec())),
                (b"\xf0m2".to_vec(), Some(b"b".to_vec())),
                (b"\xf0m1".to_vec(), Some(b"c".to_vec())),
                (b"\xf0m3".to_vec(), Some(b"d".to_vec())),
                (b"\xf0m3".to_vec(), None),
            ])
            .unwrap();
        assert_eq!(seq - before, one_commit, "several entries make one commit");
        assert_eq!(db.raw_seq().unwrap(), seq);
        assert_eq!(db.raw_get(b"\xf0m1").unwrap(), Some(b"c".to_vec()));
        assert_eq!(db.raw_get(b"\xf0m2").unwrap(), Some(b"b".to_vec()));
        assert_eq!(db.raw_get(b"\xf0m3").unwrap(), None);
        // A snapshot taken before a later raw commit keeps its view.
        let held = db.raw_snapshot().unwrap();
        db.raw_commit(vec![(k.clone(), None)]).unwrap();
        assert_eq!(held.get(&k).unwrap(), Some(b"v1".to_vec()));
        assert_eq!(db.raw_get(&k).unwrap(), None);
        drop(held);
        // The reads refuse a poisoned database, as `snapshot` does.
        db.poisoned.store(true, Ordering::SeqCst);
        let poisoned = |r: Result<(), UnitError>| matches!(r, Err(UnitError::Storage(kerno_core::Error::Poisoned(_))));
        assert!(poisoned(db.snapshot().map(drop)));
        assert!(poisoned(db.raw_snapshot().map(drop)), "raw_snapshot checks poisoning");
        assert!(poisoned(db.raw_get(&other).map(drop)), "raw_get checks poisoning");
        assert!(poisoned(db.raw_seq().map(drop)), "raw_seq checks poisoning");
        db.poisoned.store(false, Ordering::SeqCst);
        db.close().unwrap();
    }

    /// A counter the engine must report: a missing one fails the test.
    fn stat<E: TxnEngine>(db: &UnitDb<E>, name: &str) -> u64 {
        let found = db.storage_stats().into_iter().find(|(n, _)| n == name);
        found.unwrap_or_else(|| panic!("the engine reports no counter {name:?}")).1
    }

    #[test]
    fn raw_accessors_emulated_log() {
        raw_accessors::<Emulated<LogEngine>>();
    }

    #[test]
    fn raw_accessors_kerno_db() {
        raw_accessors::<kerno_storage::KernoDb>();
    }

    #[test]
    fn storage_stats_empty_on_emulated_log() {
        let disk = SimVfs::new(0);
        let db = open::<Emulated<LogEngine>>(&disk);
        db.raw_commit(vec![(b"\xf0a".to_vec(), Some(b"1".to_vec()))]).unwrap();
        assert_eq!(db.storage_stats(), Vec::new(), "the reference engine reports no counters");
        db.close().unwrap();
    }

    #[test]
    fn storage_stats_non_empty_on_kerno_db() {
        let disk = SimVfs::new(0);
        let db = open::<kerno_storage::KernoDb>(&disk);
        let commits = stat(&db, "commits");
        db.raw_commit(vec![(b"\xf0a".to_vec(), Some(b"1".to_vec()))]).unwrap();
        db.raw_commit(vec![(b"\xf0b".to_vec(), Some(b"2".to_vec()))]).unwrap();
        assert!(!db.storage_stats().is_empty());
        assert_eq!(stat(&db, "commits"), commits + 2, "the counters count calls");
        db.close().unwrap();
    }

    /// `compact` covers exactly `[TAG_REV_META, TAG_REV_PAYLOAD + 1)`: the
    /// tombstones under those two tags are dropped, those under the tag just
    /// below or just above are not. Each neighbour is tried alone, in a table
    /// of its own that stays below the L0 trigger, so that only the call
    /// under test compacts.
    #[test]
    fn compact_covers_exactly_the_revision_tags_kerno_db() {
        use crate::codec::{TAG_REV_META, TAG_REV_PAYLOAD};
        let keys = |tag: u8| (0..4u8).map(move |j| vec![tag, 0xee, j]);
        // (the tags that get tombstones, the tombstones compact must drop)
        let cases: [(&[u8], u64); 4] =
            [(&[TAG_REV_META], 4), (&[TAG_REV_PAYLOAD], 4), (&[TAG_REV_META - 1], 0), (&[TAG_REV_PAYLOAD + 1], 0)];
        for (tags, want) in cases {
            let disk = SimVfs::new(0);
            let db = open::<kerno_storage::KernoDb>(&disk);
            let puts: Vec<_> = tags.iter().flat_map(|t| keys(*t)).map(|k| (k, Some(vec![7u8; 64]))).collect();
            db.raw_commit(puts).unwrap();
            // Push the values to the last level so that tombstones can be dropped.
            db.hook_compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
            db.hook_wait_idle().unwrap();
            db.raw_commit(tags.iter().flat_map(|t| keys(*t)).map(|k| (k, None)).collect()).unwrap();
            db.hook_flush().unwrap();
            db.hook_wait_idle().unwrap();
            let dropped = stat(&db, "tombstones_dropped");
            db.compact().unwrap();
            db.hook_wait_idle().unwrap();
            let got = stat(&db, "tombstones_dropped") - dropped;
            assert_eq!(got, want, "tombstones dropped by compact under tags {tags:x?}");
            db.close().unwrap();
        }
    }

    /// Each hook leaves an effect on the engine's counters that only the
    /// real call produces; a hook that answered `Ok(())` fails here.
    #[test]
    fn hooks_reach_the_engine_kerno_db() {
        let disk = SimVfs::new(0);
        let mut cfg = UnitConfig::default();
        cfg.engine.write_buffer_bytes = 1 << 20;
        // Commits do not sync, so the sync latency set below slows the
        // background flush only.
        cfg.engine.durability = Durability::Buffered;
        let engine =
            <kerno_storage::KernoDb as Engine>::open(disk.as_vfs(), Path::new("db"), &cfg.engine).expect("open engine");
        let db = UnitDb::from_engine(engine, &cfg).expect("open layer");
        let key = |j: u8| vec![0xf0, j];
        let value = |j: u8| vec![j; 4096];
        // hook_flush: a memtable well below the write buffer becomes a table.
        db.raw_commit((0..32u8).map(|j| (key(j), Some(value(j)))).collect()).unwrap();
        let flushes = stat(&db, "flushes");
        assert!(stat(&db, "memtable_bytes") > 0);
        db.hook_flush().unwrap();
        assert_eq!(stat(&db, "flushes"), flushes + 1, "hook_flush ran a flush");
        assert_eq!(stat(&db, "memtable_bytes"), 0, "hook_flush emptied the memtable");
        // hook_wait_idle: commit until one fills the write buffer and the
        // memtable rotates; the flush of that full memtable then runs on a
        // background thread, and only wait_idle waits for it. The latency on
        // every sync keeps that flush running far longer than the reads of
        // the counters below take.
        disk.set_sync_latency(std::time::Duration::from_millis(20));
        // Each batch is 256 KiB against a 1 MiB write buffer, so a rotation is
        // due within a few batches; the bound stops a loop that would never
        // see one.
        let (flushes, rotations) = (stat(&db, "flushes"), stat(&db, "memtable_rotations"));
        for batch in 0..64u32 {
            if stat(&db, "memtable_rotations") != rotations {
                break;
            }
            let bulk = |i: u32| [&[0xf1][..], &batch.to_be_bytes(), &i.to_be_bytes()].concat();
            db.raw_commit((0..64).map(|i| (bulk(i), Some(vec![0x5a; 4096]))).collect()).unwrap();
        }
        assert_ne!(stat(&db, "memtable_rotations"), rotations, "64 batches of 256 KiB rotated no memtable");
        db.hook_wait_idle().unwrap();
        disk.set_sync_latency(std::time::Duration::ZERO);
        let rotated = stat(&db, "memtable_rotations") - rotations;
        assert_eq!(stat(&db, "immutable_memtables"), 0, "hook_wait_idle waited for the flush");
        assert_eq!(stat(&db, "flushes"), flushes + rotated, "every rotated memtable was flushed");
        // hook_compact_range over a bounded range whose ends differ in kind
        // (included start, excluded end). Everything is first pushed to the
        // last level, so that L0 holds only the two tables written next: the
        // tombstones of keys 10..=19, inside [key 10, key 20), and those of
        // keys 20..=31, outside it. Their key ranges do not overlap, and two
        // tables stay below the L0 compaction trigger, so only the call
        // under test compacts them. Only the 10 tombstones inside the range
        // are dropped: unbounded ends would drop 22, swapped ends none, and
        // an included end would take key 20's table too.
        db.hook_compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
        db.hook_wait_idle().unwrap();
        db.raw_commit((10..20u8).map(|j| (key(j), None)).collect()).unwrap();
        db.hook_flush().unwrap();
        db.raw_commit((20..32u8).map(|j| (key(j), None)).collect()).unwrap();
        db.hook_flush().unwrap();
        db.hook_wait_idle().unwrap();
        let dropped = stat(&db, "tombstones_dropped");
        db.hook_compact_range(Bound::Included(&key(10)), Bound::Excluded(&key(20))).unwrap();
        db.hook_wait_idle().unwrap();
        assert_eq!(stat(&db, "tombstones_dropped") - dropped, 10, "only the range's tombstones were dropped");
        for j in 0..32u8 {
            let want = (j < 10).then(|| value(j));
            assert_eq!(db.raw_get(&key(j)).unwrap(), want, "key {j}");
        }
        // The tombstones outside the range were kept: compacting everything
        // drops them now.
        let dropped = stat(&db, "tombstones_dropped");
        db.hook_compact_range(Bound::Unbounded, Bound::Unbounded).unwrap();
        db.hook_wait_idle().unwrap();
        assert_eq!(stat(&db, "tombstones_dropped") - dropped, 12, "the tombstones outside the range were kept");
        db.close().unwrap();
    }
}

#[cfg(test)]
mod guarded_tests {
    use super::*;
    use kerno_testkit::emulated::Emulated;
    use kerno_testkit::reference::LogEngine;
    use kerno_testkit::vfs::SimVfs;

    type E = Emulated<LogEngine>;

    fn open(disk: &SimVfs) -> UnitDb<E> {
        let cfg = UnitConfig::default();
        let engine = E::open(disk.as_vfs(), Path::new("db"), &cfg.engine).expect("open engine");
        UnitDb::from_engine(engine, &cfg).expect("open layer")
    }

    fn note() -> NewUnit {
        NewUnit { unit_type: "note".into(), schema_version: 1, metadata: Vec::new(), payload: b"p".to_vec() }
    }

    #[test]
    fn an_operation_that_writes_then_fails_closes_the_change_set() {
        let disk = SimVfs::new(0);
        let db = open(&disk);
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let r: Result<(), UnitError> = cs.guarded(|s| {
            s.create(note())?;
            Err(UnitError::InvalidArgument("late failure".into()))
        });
        assert!(matches!(r, Err(UnitError::InvalidArgument(_))));
        assert!(matches!(cs.commit(), Err(UnitError::ChangeSetClosed)));
        db.close().unwrap();
    }

    #[test]
    fn an_operation_that_fails_before_writing_leaves_the_change_set_committable() {
        let disk = SimVfs::new(0);
        let db = open(&disk);
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let r =
            cs.update(UnitId([9; 16]), UnitUpdate { payload: Vec::new(), schema_version: None, metadata: None }, None);
        assert!(matches!(r, Err(UnitError::NotFound { .. })), "{r:?}");
        let id = cs.create(note()).unwrap();
        cs.commit().unwrap();
        assert!(db.get_unit(id).unwrap().is_some());
        db.close().unwrap();
    }
}
