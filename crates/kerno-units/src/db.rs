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
use std::sync::atomic::{AtomicBool, Ordering};

use kerno_core::contract::{Engine, Options, Snapshot, SyncEngine, Txn, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, MAX_VALUE_LEN};

use crate::clock::{Clock, SystemClock, UnitIdSource, UuidV7Source};
use crate::codec::{
    ChangeSetRecord, CsMember, Key, LAYER_MARK_NAME, LAYER_VERSION, LayerMeta, MAX_CHANGESET_FIELD_LEN,
    MAX_MEMBER_COUNT, MAX_METADATA_LEN, MAX_REV_REF_LEN, MAX_UNIT_TYPE_LEN, RevisionMeta, UnitHead,
};
use crate::error::{DecodeError, UnitError};
use crate::payload::{ContentHash, MAX_PAYLOAD_LEN, PAYLOAD_CODEC_OPAQUE, verify_payload};
use crate::types::{
    ChangeSetId, CommitSeq, HistoryPolicy, Lifecycle, Purpose, RevNo, RevisionId, RevisionKind, Timestamp, UnitId,
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
        }
    }

    fn validate(&self) -> Result<(), UnitError> {
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
    /// Whether the engine's durability is `Sync` (otherwise durable after
    /// [`UnitDb::sync`]).
    pub durable: bool,
}

/// `(id, commit)` returned by [`UnitDb::create_unit`].
pub type CreatedUnit = (UnitId, CommitInfo);

/// The units layer over a transactional engine `E` (design §6.1).
pub struct UnitDb<E: TxnEngine> {
    engine: E,
    cfg: UnitConfig,
    poisoned: AtomicBool,
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
        let m = LayerMeta::decode(&v).map_err(|e| context(e, &format!("LAYER_META \"policy/{unit_type}\"")))?;
        return Ok(m.history_policy);
    }
    match get(&layer_mark_key())? {
        Some(v) => Ok(LayerMeta::decode(&v).map_err(|e| context(e, "LAYER_META \"layer\""))?.history_policy),
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
    Present,
    /// No mark and no key at all: a new database.
    Empty,
}

/// Reads the mark with `get`, and with `any_key` whether the store holds any
/// key. A store with keys but no mark is not a units database.
fn read_mark(get: &GetFn<'_>, any_key: &dyn Fn() -> kerno_core::Result<bool>) -> Result<MarkState, UnitError> {
    match get(&layer_mark_key())? {
        Some(v) => {
            LayerMeta::decode(&v).map_err(|e| context(e, "LAYER_META \"layer\""))?;
            Ok(MarkState::Present)
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
        && (meta.kind == RevisionKind::Delete) == (head.lifecycle == Lifecycle::Deleted);
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
            Ok(_) => Ok(UnitDb { engine, cfg: cfg.clone(), poisoned: AtomicBool::new(false) }),
            Err(e) => {
                let _ = engine.close();
                Err(e)
            }
        }
    }

    /// The engine underneath. Writing to it directly bypasses the layer.
    pub fn engine(&self) -> &E {
        &self.engine
    }

    pub fn config(&self) -> &UnitConfig {
        &self.cfg
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
        self.check_poisoned()?;
        meta.validate()?;
        let txn = self.engine.begin_write()?;
        let need_mark = match read_mark(&|k| txn.get(k), &|| {
            Ok(!txn.scan(Bound::Unbounded, Bound::Unbounded, 1)?.is_empty())
        })? {
            MarkState::Present => false,
            MarkState::Empty if self.cfg.create_if_missing => true,
            MarkState::Empty => {
                return Err(UnitError::InvalidArgument("the database is empty and create_if_missing is false".into()));
            }
        };
        let seq = txn
            .read_seq()
            .checked_add(1)
            .ok_or_else(|| UnitError::InvalidArgument("the commit sequence is exhausted".into()))?;
        let now = self.cfg.clock.now();
        Ok(ChangeSet {
            db: self,
            txn: Some(txn),
            seq: CommitSeq(seq),
            meta,
            now,
            need_mark,
            members: Vec::new(),
            touched: BTreeSet::new(),
            broken: false,
            maintenance: false,
            writes: 0,
            bytes: 0,
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
        Ok(UnitSnapshot { snap: self.engine.snapshot()?, fallback: self.cfg.default_policy })
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
        let v = LayerMeta { layer_version: LAYER_VERSION, history_policy: p }.encode()?;
        cs.put(&k, &v)?;
        cs.maintenance = true;
        cs.commit()
    }
}

impl<E: TxnEngine + SyncEngine> UnitDb<E> {
    /// The engine's durability barrier (I-17): returns the durable sequence.
    pub fn sync(&self) -> Result<u64, UnitError> {
        self.check_poisoned()?;
        self.engine.sync().map_err(UnitError::from)
    }
}

/// A read view of the units at one commit sequence.
pub struct UnitSnapshot<E: Engine> {
    pub(crate) snap: E::Snapshot,
    /// The policy of a database without a layer mark.
    pub(crate) fallback: HistoryPolicy,
}

impl<E: Engine> UnitSnapshot<E> {
    pub fn seq(&self) -> CommitSeq {
        CommitSeq(self.snap.seq())
    }

    /// The history policy in force for `unit_type` at this snapshot.
    pub fn policy(&self, unit_type: &str) -> Result<HistoryPolicy, UnitError> {
        policy_of(&|k| self.snap.get(k), unit_type, self.fallback)
    }

    /// The unit at this snapshot (see [`UnitDb::get_unit`]).
    pub fn get_unit(&self, id: UnitId) -> Result<Option<Unit>, UnitError> {
        Ok(load_unit(&|k| self.snap.get(k), id)?.map(|(head, _, payload)| to_unit(id, head, payload)))
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
    members: Vec<(RevisionId, RevisionKind)>,
    touched: BTreeSet<UnitId>,
    broken: bool,
    /// A `set_policy` change set: commits without members.
    maintenance: bool,
    /// Engine writes and their key + value bytes so far.
    writes: usize,
    bytes: usize,
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

    pub(crate) fn txn(&self) -> Result<&E::Txn<'db>, UnitError> {
        match &self.txn {
            Some(t) if !self.broken => Ok(t),
            _ => Err(UnitError::ChangeSetClosed),
        }
    }

    /// Checks shared by every operation, before reading anything.
    fn admit(&self, unit: Option<UnitId>) -> Result<(), UnitError> {
        self.txn()?;
        if self.members.len() >= self.db.cfg.max_changeset_ops {
            return Err(UnitError::ChangeSetTooLarge { ops: self.members.len() + 1, bytes: self.bytes });
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
    fn write(&mut self, w: &Write) -> Result<(), UnitError> {
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

    fn apply(&mut self, plan: Planned) -> Result<RevNo, UnitError> {
        for w in &plan.writes {
            self.write(w)?;
        }
        self.touched.insert(plan.rev.unit);
        self.members.push((plan.rev, plan.kind));
        Ok(plan.rev.rev)
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
        if self.members.is_empty() && !self.maintenance {
            let current = CommitSeq(self.seq.0 - 1);
            if let Some(t) = self.txn.take() {
                t.abort();
            }
            return Ok(CommitInfo { change_set: current, commit_seq: current, revisions: Vec::new(), durable });
        }
        if self.need_mark {
            let mark = LayerMeta { layer_version: LAYER_VERSION, history_policy: self.db.cfg.default_policy };
            let v = mark.encode()?;
            self.put(&layer_mark_key(), &v)?;
        }
        let record = ChangeSetRecord {
            created_at: self.now,
            member_count: self.members.len() as u32,
            purpose: self.meta.purpose,
            reason: self.meta.reason.clone().map(String::into_bytes).unwrap_or_default(),
            actor_ref: self.actor(),
            source_ref: self.source(),
            correlation: self.meta.correlation.clone().unwrap_or_default(),
        };
        let v = record.encode()?;
        self.put(&key(Key::ChangeSet { seq: self.seq })?, &v)?;
        let txn = self.txn.take().ok_or(UnitError::ChangeSetClosed)?;
        let got = txn.commit()?;
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

    /// A retention change set (purpose `Retention`, no caller provenance).
    pub(crate) fn begin_retention(db: &'db UnitDb<E>) -> Result<Self, UnitError> {
        let mut cs = db.begin_changeset(ChangeMeta::default())?;
        cs.meta.purpose = Purpose::Retention;
        Ok(cs)
    }

    /// The clock reading of this change set.
    pub(crate) fn now(&self) -> Timestamp {
        self.now
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
        self.apply(Planned { writes, rev: RevisionId { unit, rev }, kind })?;
        Ok(())
    }

    /// Discards the change set (same as dropping it).
    pub fn abort(self) {}
}
