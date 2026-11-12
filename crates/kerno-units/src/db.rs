//! The units database (`etapa3-design.md` §4, §6.1–§6.4, E3-03): opening
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
    Adjacency, ChangeSetRecord, CsMember, Key, LAYER_MARK_NAME, LAYER_VERSION, LAYER_VERSION_DERIVED, LayerMeta,
    MAX_CHANGESET_FIELD_LEN, MAX_LAYER_VERSION, MAX_MEMBER_COUNT, MAX_METADATA_LEN, MAX_REV_REF_LEN, MAX_UNIT_TYPE_LEN,
    RelationRecord, RevisionMeta, TAG_REL_IN, TAG_REL_OUT, UnitHead, valid_relation_type,
};
use crate::error::{DecodeError, UnitError};
use crate::payload::{ContentHash, MAX_PAYLOAD_LEN, PAYLOAD_CODEC_OPAQUE, verify_payload};
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
            Ok(MarkState::Present(mark.layer_version))
        }
        None if any_key()? => Err(UnitError::NotAUnitDatabase),
        None => Ok(MarkState::Empty),
    }
}

/// A unit's head, its head `REV_META` and its verified payload.
type Loaded = (UnitHead, RevisionMeta, Option<Vec<u8>>);

/// Reads and checks a unit's head, its head `REV_META` and its payload.
fn load_unit(get: &GetFn<'_>, id: UnitId) -> Result<Option<Loaded>, UnitError> {
    let Some(raw) = get(&key(Key::UnitHead { unit: id })?)? else {
        return Ok(None);
    };
    let head = UnitHead::decode(&raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {id}")))?;
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
    let stored = get(&key(Key::RevPayload { unit: id, rev })?)?;
    let verified = verify_payload(&meta.payload_decl(), stored.as_deref())
        .map_err(|e| crate::PayloadError { detail: format!("unit {id} rev {rev}: {}", e.detail), ..e })?;
    let payload = verified.bytes().map(<[u8]>::to_vec);
    Ok(Some((head, meta, payload)))
}

fn to_unit(id: UnitId, head: UnitHead, payload: Option<Vec<u8>>) -> Unit {
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
            MarkState::Empty if self.cfg.create_if_missing => (true, LAYER_VERSION),
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
            members: Vec::new(),
            relation_ops: 0,
            touched: BTreeSet::new(),
            broken: false,
            maintenance: false,
            index: false,
            writes: 0,
            bytes: 0,
            deferred: false,
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
        })
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
        Ok(load_unit(&|k| self.snap.get(k), id)?.map(|(head, _, payload)| to_unit(id, head, payload)))
    }

    /// The relation `id` at this snapshot, or `None`.
    pub fn get_relation(&self, id: RelationId) -> Result<Option<Relation>, UnitError> {
        let Some(raw) = self.snap.get(&key(Key::Relation { edge: id })?)? else {
            return Ok(None);
        };
        let record = RelationRecord::decode(&raw).map_err(|e| context(e, &format!("REL {id}")))?;
        Ok(Some(Relation::new(id, record)))
    }

    /// The relations that start at (`Direction::Out`) or end at
    /// (`Direction::In`) `unit`, optionally of one `relation_type`, read from
    /// the adjacency index: a range scan, not a scan of all relations. In
    /// index order: relation type, then relation id. Every entry is checked
    /// against its `REL` record; a missing record or a disagreement is
    /// `Storage(Corruption)`. The result must equal
    /// [`UnitSnapshot::reference_relations_of`] on the same snapshot (I-32).
    pub fn relations_of(
        &self,
        unit: UnitId,
        dir: Direction,
        relation_type: Option<&str>,
    ) -> Result<Vec<Relation>, UnitError> {
        let tag = match dir {
            Direction::Out => TAG_REL_OUT,
            Direction::In => TAG_REL_IN,
        };
        let Some((start, end)) = Key::adjacency_range(tag, unit, relation_type) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        scan_all(&self.snap, start, end, |k, v| {
            let (edge, ty) = match Key::decode(k)? {
                Key::RelationOut { relation_type, edge, .. } | Key::RelationIn { relation_type, edge, .. } => {
                    (edge, relation_type)
                }
                _ => return Err(corruption("a key in an adjacency range is not an adjacency key".into())),
            };
            let other = Adjacency::decode(v).map_err(|e| context(e, &format!("adjacency entry of {edge}")))?.other;
            let Some(rel) = self.get_relation(edge)? else {
                return Err(corruption(format!("adjacency entry of relation {edge} has no REL record")));
            };
            let (here, there) = if dir == Direction::Out { (rel.from, rel.to) } else { (rel.to, rel.from) };
            if here != unit || there != other || rel.relation_type != ty {
                return Err(corruption(format!("adjacency entry of relation {edge} disagrees with its REL record")));
            }
            out.push(rel);
            Ok(())
        })?;
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
}

/// One engine write of a planned operation.
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
            return Err(UnitError::ChangeSetTooLarge { ops: ops + 1, bytes: self.bytes });
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
            return Err(UnitError::ChangeSetTooLarge { ops: ops + 1, bytes: self.bytes });
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
    fn put(&mut self, k: &[u8], v: &[u8]) -> Result<(), UnitError> {
        self.write(&Write::Put(k.to_vec(), v.to_vec()))
    }

    /// Writes one engine operation; on an engine error the change set is broken.
    pub(crate) fn write(&mut self, w: &Write) -> Result<(), UnitError> {
        let Some(txn) = self.txn.as_mut() else {
            return Err(UnitError::ChangeSetClosed);
        };
        let len = match w {
            Write::Put(k, v) | Write::DeleteRange(k, v) => k.len() + v.len(),
            Write::Delete(k) => k.len(),
        };
        let (writes, bytes) = (self.writes + 1, self.bytes + len);
        let r = match w {
            Write::Put(k, v) => txn.put(k, v),
            Write::Delete(k) => txn.delete(k),
            Write::DeleteRange(s, e) => txn.delete_range(s, e),
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
                    kerno_core::Error::InvalidArgument(_) => UnitError::ChangeSetTooLarge { ops: writes, bytes },
                    e => UnitError::Storage(e),
                })
            }
        }
    }

    /// [`apply`](Self::apply) without the derived follow-up, for a plan that
    /// already carries it.
    fn apply_planned_only(&mut self, plan: Planned) -> Result<RevNo, UnitError> {
        for w in &plan.writes {
            self.write(w)?;
        }
        self.touched.insert(plan.rev.unit);
        self.members.push((plan.rev, plan.kind));
        Ok(plan.rev.rev)
    }

    fn apply(&mut self, plan: Planned) -> Result<RevNo, UnitError> {
        // The persisted derived data of the unit follow the source in this
        // same transaction (ADR-0020 §S3): read before the first write, so a
        // refusal leaves the change set untouched.
        let derived = self.derived_follow(plan.rev.unit, plan.kind)?;
        for w in plan.writes.iter().chain(&derived) {
            self.write(w)?;
        }
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
    /// [module documentation](self)); may move `head.oldest_rev`.
    fn supersede(&self, unit: UnitId, head: &mut UnitHead) -> Result<Vec<Write>, UnitError> {
        let rev = head.head_rev;
        if rev.0 == 1 {
            return Ok(Vec::new());
        }
        match self.policy(&head.unit_type)? {
            HistoryPolicy::Versioned => Ok(Vec::new()),
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

    /// The records of a new revision `rev` of `unit`: what its policy drops
    /// of the superseded revisions, head, `REV_META`, `REV_PAYLOAD` (when
    /// there is a payload) and `CS_MEMBER`.
    fn plan(
        &self,
        unit: UnitId,
        mut head: UnitHead,
        kind: RevisionKind,
        payload: Option<&[u8]>,
        restored_from: Option<RevNo>,
    ) -> Result<Planned, UnitError> {
        let mut writes = self.supersede(unit, &mut head)?;
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
        let ordinal = u32::try_from(self.members.len())
            .map_err(|_| UnitError::ChangeSetTooLarge { ops: self.members.len() + 1, bytes: self.bytes })?;
        let member = CsMember { unit, rev_no: rev, kind };
        writes.push(Write::Put(key(Key::UnitHead { unit })?, head.encode()?));
        writes.push(Write::Put(key(Key::RevMeta { unit, rev })?, meta.encode(rev)?));
        if let Some(p) = payload {
            writes.push(Write::Put(key(Key::RevPayload { unit, rev })?, p.to_vec()));
        }
        writes.push(Write::Put(key(Key::CsMember { seq: self.seq, ordinal })?, member.encode()?));
        Ok(Planned { writes, rev: RevisionId { unit, rev }, kind })
    }

    /// The current head of `id` (verified), refusing a missing unit, an
    /// `expected` revision that is not the head and a deleted unit, in that
    /// order.
    fn current(&self, id: UnitId, expected: Option<RevNo>) -> Result<Loaded, UnitError> {
        let txn = self.txn()?;
        let Some((head, meta, payload)) = load_unit(&|k| txn.get(k), id)? else {
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
        Ok((head, meta, payload))
    }

    fn next_head(&self, head: &UnitHead) -> UnitHead {
        UnitHead { head_rev: RevNo(head.head_rev.0 + 1), updated_seq: self.seq, updated_at: self.now, ..head.clone() }
    }

    /// Creates a unit with a fresh id from the id source and returns it.
    /// An id that already exists is [`UnitError::AlreadyExists`] (a deleted
    /// unit's id is never reused).
    pub fn create(&mut self, new: NewUnit) -> Result<UnitId, UnitError> {
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
        if self.touched.contains(&id) || self.txn()?.get(&key(Key::UnitHead { unit: id })?)?.is_some() {
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
            content_hash: ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, &new.payload),
            payload_len: new.payload.len() as u32,
            metadata: new.metadata,
        };
        let plan = self.plan(id, head, RevisionKind::Create, Some(&new.payload), None)?;
        self.apply(plan)?;
        Ok(id)
    }

    /// A new revision with new content. The unit must be `Active`.
    pub fn update(&mut self, id: UnitId, upd: UnitUpdate, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.admit(Some(id))?;
        self.check_payload(&upd.payload)?;
        if let Some(m) = &upd.metadata {
            Self::check_metadata(m)?;
        }
        let (head, _, _) = self.current(id, expected)?;
        if head.lifecycle != Lifecycle::Active {
            return Err(UnitError::InvalidArgument(format!("unit {id} is archived; unarchive it before updating")));
        }
        let mut next = self.next_head(&head);
        next.lifecycle = Lifecycle::Active;
        next.content_hash = ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, &upd.payload);
        next.payload_len = upd.payload.len() as u32;
        if let Some(v) = upd.schema_version {
            next.schema_version = v;
        }
        if let Some(m) = upd.metadata {
            next.metadata = m;
        }
        let plan = self.plan(id, next, RevisionKind::Update, Some(&upd.payload), None)?;
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
        let (head, _, payload) = self.current(id, expected)?;
        if head.lifecycle != from {
            return Err(UnitError::InvalidArgument(format!("unit {id} is {:?}, not {from:?}", head.lifecycle)));
        }
        if to == Lifecycle::Archived {
            self.ensure_no_relations(id)?;
        }
        let Some(payload) = payload else {
            return Err(corruption(format!("unit {id}: a {:?} head revision has no payload", head.lifecycle)));
        };
        let mut next = self.next_head(&head);
        next.lifecycle = to;
        let plan = self.plan(id, next, kind, Some(&payload), None)?;
        self.apply(plan)
    }

    /// Archives an `Active` unit: a revision that copies its payload.
    pub fn archive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.relifecycle(id, expected, Lifecycle::Active, Lifecycle::Archived, RevisionKind::Archive)
    }

    /// Makes an `Archived` unit `Active` again: a revision that copies its payload.
    pub fn unarchive(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.relifecycle(id, expected, Lifecycle::Archived, Lifecycle::Active, RevisionKind::Unarchive)
    }

    /// Deletes an `Active` or `Archived` unit logically: a tombstone revision
    /// without payload (hash of the empty payload, length 0) and the head
    /// kept with `lifecycle = Deleted` (design §3.2).
    pub fn delete(&mut self, id: UnitId, expected: Option<RevNo>) -> Result<RevNo, UnitError> {
        self.admit(Some(id))?;
        let (head, _, _) = self.current(id, expected)?;
        self.ensure_no_relations(id)?;
        let mut next = self.next_head(&head);
        next.lifecycle = Lifecycle::Deleted;
        next.content_hash = ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, &[]);
        next.payload_len = 0;
        let plan = self.plan(id, next, RevisionKind::Delete, None, None)?;
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
        self.admit(Some(id))?;
        let (head, _, _) = self.current(id, expected)?;
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
        crate::history::check_content(&source)?;
        let Some(payload) = crate::history::read_payload(&get, id, from, &source)? else {
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
        let plan = self.plan(id, next, RevisionKind::Restore, Some(&payload), Some(from))?;
        self.apply(plan)
    }

    /// Refuses with [`UnitError::UnitHasRelations`] when a relation, in
    /// either direction, touches `unit` as this change set sees it (ADR-0014,
    /// criterion 4). Two single-entry scans of the adjacency indexes.
    fn ensure_no_relations(&self, unit: UnitId) -> Result<(), UnitError> {
        let txn = self.txn()?;
        for tag in [TAG_REL_OUT, TAG_REL_IN] {
            let Some((start, end)) = Key::adjacency_range(tag, unit, None) else {
                continue;
            };
            if let Some((k, _)) = txn.scan(Bound::Included(&start), Bound::Excluded(&end), 1)?.into_iter().next() {
                return match Key::decode(&k)? {
                    Key::RelationOut { edge, .. } | Key::RelationIn { edge, .. } => {
                        Err(UnitError::UnitHasRelations { unit, relation: edge })
                    }
                    _ => Err(corruption("a key in an adjacency range is not an adjacency key".into())),
                };
            }
        }
        Ok(())
    }

    /// The lifecycle of `unit` as this change set sees it, reading only its
    /// `UNIT_HEAD`; `NotFound` or `Deleted` for a unit that cannot take a
    /// relation, `InvalidArgument` for an archived one.
    fn require_active(&self, unit: UnitId) -> Result<(), UnitError> {
        let txn = self.txn()?;
        let Some(raw) = txn.get(&key(Key::UnitHead { unit })?)? else {
            return Err(UnitError::NotFound { unit });
        };
        let head = UnitHead::decode(&raw).map_err(|e| context(e, &format!("UNIT_HEAD of unit {unit}")))?;
        match head.lifecycle {
            Lifecycle::Active => Ok(()),
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
        self.admit_relation()?;
        if !valid_relation_type(relation_type) {
            return Err(UnitError::InvalidArgument(format!(
                "relation_type of {} bytes is outside 1..={} or has a NUL",
                relation_type.len(),
                crate::codec::MAX_RELATION_TYPE_LEN
            )));
        }
        self.require_active(from)?;
        self.require_active(to)?;
        let edge = RelationId(self.db.cfg.ids.next_id().0);
        if self.txn()?.get(&key(Key::Relation { edge })?)?.is_some() {
            return Err(UnitError::InvalidArgument(format!("relation id {edge} already exists")));
        }
        let record =
            RelationRecord { from, to, relation_type: relation_type.to_owned(), created_seq: self.seq, provenance };
        let t = || relation_type.to_owned();
        let writes = [
            Write::Put(key(Key::Relation { edge })?, record.encode()?),
            Write::Put(
                key(Key::RelationOut { unit: from, relation_type: t(), edge })?,
                Adjacency { other: to }.encode(),
            ),
            Write::Put(
                key(Key::RelationIn { unit: to, relation_type: t(), edge })?,
                Adjacency { other: from }.encode(),
            ),
        ];
        self.apply_relation_writes(&writes)?;
        Ok(edge)
    }

    /// Deletes a relation: its record and both adjacency entries, in this
    /// change set. Relations have no history of their own (ADR-0014): the
    /// change set that deleted it is the only trace.
    pub fn delete_relation(&mut self, edge: RelationId) -> Result<(), UnitError> {
        self.admit_relation()?;
        let txn = self.txn()?;
        let Some(raw) = txn.get(&key(Key::Relation { edge })?)? else {
            return Err(UnitError::RelationNotFound { relation: edge });
        };
        let r = RelationRecord::decode(&raw).map_err(|e| context(e, &format!("REL {edge}")))?;
        let t = || r.relation_type.clone();
        let writes = [
            Write::Delete(key(Key::Relation { edge })?),
            Write::Delete(key(Key::RelationOut { unit: r.from, relation_type: t(), edge })?),
            Write::Delete(key(Key::RelationIn { unit: r.to, relation_type: t(), edge })?),
        ];
        self.apply_relation_writes(&writes)
    }

    fn admit_relation(&self) -> Result<(), UnitError> {
        self.txn()?;
        let ops = self.members.len() + self.relation_ops;
        if ops >= self.db.cfg.max_changeset_ops {
            return Err(UnitError::ChangeSetTooLarge { ops: ops + 1, bytes: self.bytes });
        }
        Ok(())
    }

    fn apply_relation_writes(&mut self, writes: &[Write]) -> Result<(), UnitError> {
        for w in writes {
            self.write(w)?;
        }
        self.relation_ops += 1;
        Ok(())
    }

    /// The unit as this change set sees it, its own writes included.
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        let txn = self.txn()?;
        Ok(load_unit(&|k| txn.get(k), id)?.map(|(head, _, payload)| to_unit(id, head, payload)))
    }

    /// Commits every operation as one engine commit (I-21). A change set
    /// without operations writes nothing and returns the current sequence.
    pub fn commit(mut self) -> Result<CommitInfo, UnitError> {
        self.txn()?;
        let durable = self.db.cfg.engine.durability == Durability::Sync;
        if self.members.is_empty()
            && !self.maintenance
            && !self.index
            && self.relation_ops == 0
            && self.derived_ops == 0
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
        // The mark goes to version 2 in the transaction of the first derived
        // record, and no writer lowers it (ADR-0020 §S3.4, §S3.5).
        let wants = if self.derived_ops > 0 { LAYER_VERSION_DERIVED } else { LAYER_VERSION };
        if self.need_mark {
            let mark = LayerMeta { layer_version: wants, history_policy: self.db.cfg.default_policy };
            let v = mark.encode()?;
            self.put(&layer_mark_key(), &v)?;
        } else if self.layer_version < wants {
            let txn = self.txn()?;
            let current = match txn.get(&layer_mark_key())? {
                Some(raw) => {
                    LayerMeta::decode_upto(&raw, MAX_LAYER_VERSION).map_err(|e| context(e, "LAYER_META \"layer\""))?
                }
                None => return Err(UnitError::NotAUnitDatabase),
            };
            let v = LayerMeta { layer_version: wants, history_policy: current.history_policy }.encode()?;
            self.put(&layer_mark_key(), &v)?;
        }
        let record = ChangeSetRecord {
            created_at: self.now,
            member_count: self.members.len() as u32,
            // A member-less application change set is relation-only: say so,
            // so a lost-members change set stays detectable (UNIT-30).
            purpose: if self.meta.purpose == Purpose::Application && self.members.is_empty() {
                // ADR-0020 §B.5: derived records only, no relation touched.
                if self.relation_ops == 0 && self.derived_ops > 0 { Purpose::Derived } else { Purpose::Relation }
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
        let (got, durable) = if self.deferred {
            let (got, waited) = txn.commit_deferred()?;
            (got, durable && waited)
        } else {
            (txn.commit()?, durable)
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
    ) -> Result<(), UnitError> {
        self.txn()?;
        if self.touched.contains(&unit) {
            return Err(UnitError::InvalidArgument(format!("unit {unit} already retained in this change set")));
        }
        let ordinal = u32::try_from(self.members.len())
            .map_err(|_| UnitError::ChangeSetTooLarge { ops: self.members.len() + 1, bytes: self.bytes })?;
        let member = CsMember { unit, rev_no: rev, kind };
        writes.push(Write::Put(key(Key::CsMember { seq: self.seq, ordinal })?, member.encode()?));
        // A revision retention purges takes the derived data computed from it
        // (ADR-0020 §S5.1), in this same change set.
        if self.layer_version >= LAYER_VERSION_DERIVED {
            let txn = self.txn()?;
            let purge = crate::vector_store::SourceEvent::Purged(rev);
            writes.extend(crate::vector_store::plan_source_event(&crate::vector_store::TxnReader(txn), unit, purge)?);
        }
        self.apply_planned_only(Planned { writes, rev: RevisionId { unit, rev }, kind })?;
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
