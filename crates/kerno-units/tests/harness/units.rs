//! `harness::units` (etapa3-design.md §8.2): the crash sweeps of the layer
//! commit [`Step::ChangeSet`]s instead of `WriteBatch`es, so the testkit's
//! `TxnCommits` wrapper does not fit; this module plays its part.
//!
//! * The workload is a deterministic function of `(seed, seq, state)`:
//!   [`script_for`] picks change set `seq` from the unit heads the store holds
//!   after change set `seq - 1`. Each step commits exactly one change set, so
//!   step `k` is commit sequence `k`, and the id source is reset per step
//!   ([`StepIds`]). A database recovered at any sequence `s` therefore
//!   continues with exactly the change sets an undisturbed run would make.
//! * The oracle ([`Oracle`]) runs the same workload on an undisturbed
//!   `Emulated<LogEngine>` and keeps the raw store after each change set. A
//!   recovered store must equal one of them **byte for byte** (every key of
//!   every record family: head, `REV_META`, `REV_PAYLOAD`, `CHANGESET`,
//!   `CS_MEMBER`, `LAYER_META`). That proves the recovered store is one an
//!   undisturbed run of the *same* layer code writes at a change-set
//!   boundary (the engines are interchangeable and a change set is all there
//!   or not at all, I-21), but not that the layer wrote the right thing:
//!   both sides run the same `UnitDb`, so a layer bug is on both.
//! * The model check ([`check_model`]) closes that gap: the oracle also
//!   drives the independent [`UnitModel`] with the acknowledged operations
//!   (revisions built here from the script, not read back from the layer),
//!   and the recovered database's public reads (unit ids, every head with
//!   its payload and lifecycle, every retained revision, every change set
//!   and its members, the policies) must answer what the model says.
//! * [`check_store`] runs both, and also requires `verify_units` without findings and a
//!   readable head for every unit; [`keeps_working`] commits once more,
//!   closes, loses power and checks again.

use std::collections::BTreeSet;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_core::vfs::Vfs;
use kerno_core::{Durability, Error};
use kerno_storage::{Background, Config, KernoDb, TestConfig};
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, SimVfs};
use kerno_units::codec::{Key, UnitHead};
use kerno_units::payload::PAYLOAD_CODEC_OPAQUE;
use kerno_units::*;

pub const DIR: &str = "/db";
pub const T0: Timestamp = Timestamp(1_700_000_000_000_000);
/// Tag of `UNIT_HEAD` keys (units-v1.md §2).
const TAG_UNIT_HEAD: u8 = 0x10;

pub type Dump = Vec<(Vec<u8>, Vec<u8>)>;

/// One step of a crash scenario (design §8.2: the sweep wraps `UnitDb` in
/// its own `Step::ChangeSet`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `n` change sets of the workload.
    ChangeSet(u64),
    /// Clean close and reopen.
    Close,
    /// Drop the handle without closing it (an unclean end) and reopen.
    Drop,
}

// ------------------------------------------------------------ engines

/// An engine the sweeps open the layer on.
pub trait Target: TxnEngine {
    const NAME: &'static str;
    fn open_engine(vfs: Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<Self>;
}

/// The engine configuration of the sweeps: `opts` with inline flushes and
/// compactions, so that a scenario's file-system operations, and with them
/// the sweep points, are the same in every run (phase1-design.md §1 item 6).
pub fn kerno_config(opts: &Options) -> Config {
    Config { test: TestConfig { background: Background::Inline, ..TestConfig::default() }, ..Config::from(opts) }
}

impl Target for KernoDb {
    const NAME: &'static str = "kerno";
    fn open_engine(vfs: Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<Self> {
        KernoDb::open(vfs, Path::new(DIR), &kerno_config(opts))
    }
}

impl Target for Emulated<LogEngine> {
    const NAME: &'static str = "emulated-log";
    fn open_engine(vfs: Arc<dyn Vfs>, opts: &Options) -> kerno_core::Result<Self> {
        <Self as Engine>::open(vfs, Path::new(DIR), opts)
    }
}

pub fn open_units<E: Target>(vfs: Arc<dyn Vfs>, cfg: &UnitConfig) -> Result<UnitDb<E>, UnitError> {
    let engine = E::open_engine(vfs, &cfg.engine).map_err(UnitError::Storage)?;
    UnitDb::from_engine(engine, cfg)
}

// ------------------------------------------------------------ ids and config

/// Ids reset per step: change set `k` mints `k·2^16 + 1, + 2, …`, so a
/// change set redone after a crash gets the ids it got the first time.
#[derive(Clone, Debug, Default)]
pub struct StepIds(Arc<AtomicU64>);

impl StepIds {
    pub fn new() -> Self {
        StepIds::default()
    }
    pub fn start_step(&self, seq: u64) {
        self.0.store(seq << 16, Ordering::SeqCst);
    }
}

impl UnitIdSource for StepIds {
    fn next_id(&self) -> UnitId {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let mut id = [0u8; 16];
        id[8..].copy_from_slice(&n.to_be_bytes());
        UnitId(id)
    }
}

/// Fixed clock, step ids, small engine buffers (flushes every few change
/// sets) and `durability`.
pub fn unit_config(ids: &StepIds, durability: Durability) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(ids.clone()));
    c.engine = Options::small_for_tests();
    c.engine.durability = durability;
    // Each step is exactly one change set with a predicted sequence, so a
    // `Retention` step must not split into several (E3-09b pacing); the
    // paced, multi-change-set call has its own sweeps in `tests/retention.rs`.
    c.retention_pacing = RetentionPacing { units_per_change_set: usize::MAX, pause_ratio: 0 };
    c
}

// ------------------------------------------------------------ the workload

pub const TYPES: [&str; 3] = ["doc", "img", "log"];

#[derive(Clone, Debug)]
pub enum Action {
    Create { unit_type: &'static str, payload: Vec<u8> },
    Update { id: UnitId, payload: Vec<u8>, schema: Option<u32> },
    Archive(UnitId),
    Unarchive(UnitId),
    Delete(UnitId),
    Restore(UnitId, RevNo),
}

/// What one step commits: always exactly one change set.
#[derive(Clone, Debug)]
pub enum Script {
    Changes(Vec<Action>),
    Policy(&'static str, HistoryPolicy),
    /// `run_retention` with `KeepLast(n)`; when it has nothing to prune, a
    /// filler change set (one create) keeps "step `k` = sequence `k`".
    Retention(u64),
}

/// Every 7th change set is large (several payloads well above the 16 KiB
/// write buffer): one WAL frame spanning many sectors (`torn_write`) and a
/// flush inside the commit.
pub const LARGE_EVERY: u64 = 7;

fn payload(rng: &mut Rng) -> Vec<u8> {
    let len = if rng.chance(1, 8) { rng.range(1_000, 6_000) } else { rng.range(1, 300) };
    rng.bytes(len as usize)
}

/// The change set of step `seq`, chosen from the heads after step `seq - 1`.
pub fn script_for(seed: u64, seq: u64, heads: &[(UnitId, UnitHead)]) -> Script {
    let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seq.wrapping_mul(0xD1B5_4A32_D192_ED03));
    if seq.is_multiple_of(LARGE_EVERY) {
        let n = rng.range(3, 6);
        let actions = (0..n)
            .map(|i| {
                let len = rng.range(8_000, 24_000) as usize;
                Action::Create { unit_type: TYPES[i as usize % 3], payload: rng.bytes(len) }
            })
            .collect();
        return Script::Changes(actions);
    }
    if seq > 3 && rng.chance(1, 12) {
        let p = [HistoryPolicy::Versioned, HistoryPolicy::Audit, HistoryPolicy::None][rng.below(3) as usize];
        return Script::Policy(TYPES[rng.below(3) as usize], p);
    }
    if seq > 3 && rng.chance(1, 10) {
        return Script::Retention(rng.range(1, 4));
    }
    let mut actions = Vec::new();
    let mut touched = BTreeSet::new();
    for _ in 0..rng.range(1, 6) {
        if heads.is_empty() || rng.chance(1, 4) {
            actions.push(Action::Create { unit_type: TYPES[rng.below(3) as usize], payload: payload(&mut rng) });
            continue;
        }
        let (id, h) = &heads[rng.below(heads.len() as u64) as usize];
        if !touched.insert(*id) {
            continue;
        }
        let roll = rng.below(10);
        let a = match h.lifecycle {
            Lifecycle::Active => match roll {
                0..=5 => Action::Update {
                    id: *id,
                    payload: payload(&mut rng),
                    schema: rng.chance(1, 4).then(|| rng.range(1, 4) as u32),
                },
                6 => Action::Archive(*id),
                7 => Action::Delete(*id),
                // The head always has its payload (UNIT-31) under any policy.
                _ => Action::Restore(*id, h.head_rev),
            },
            Lifecycle::Archived => {
                if roll < 6 {
                    Action::Unarchive(*id)
                } else {
                    Action::Delete(*id)
                }
            }
            Lifecycle::Deleted => continue,
        };
        actions.push(a);
    }
    if actions.is_empty() {
        actions.push(Action::Create { unit_type: "doc", payload: payload(&mut rng) });
    }
    Script::Changes(actions)
}

fn meta(seq: u64) -> ChangeMeta {
    ChangeMeta { reason: Some(format!("step {seq}")), actor: Some(b"sweeper".to_vec()), ..ChangeMeta::default() }
}

fn check_seq(got: CommitSeq, seq: u64) {
    assert_eq!(got, CommitSeq(seq), "step {seq} committed with another sequence");
}

/// Commits `script` as change set `seq`.
pub fn run_script<E: TxnEngine>(db: &UnitDb<E>, ids: &StepIds, seq: u64, script: &Script) -> Result<(), UnitError> {
    ids.start_step(seq);
    match script {
        Script::Changes(actions) => {
            let mut cs = db.begin_changeset(meta(seq))?;
            for a in actions {
                match a {
                    Action::Create { unit_type, payload } => {
                        cs.create(NewUnit {
                            unit_type: (*unit_type).into(),
                            schema_version: 1,
                            metadata: format!("m{seq}").into_bytes(),
                            payload: payload.clone(),
                        })?;
                    }
                    Action::Update { id, payload, schema } => {
                        cs.update(
                            *id,
                            UnitUpdate { payload: payload.clone(), schema_version: *schema, metadata: None },
                            None,
                        )?;
                    }
                    Action::Archive(id) => {
                        cs.archive(*id, None)?;
                    }
                    Action::Unarchive(id) => {
                        cs.unarchive(*id, None)?;
                    }
                    Action::Delete(id) => {
                        cs.delete(*id, None)?;
                    }
                    Action::Restore(id, from) => {
                        cs.restore(*id, *from, None)?;
                    }
                }
            }
            check_seq(cs.commit()?.commit_seq, seq);
        }
        Script::Policy(t, p) => check_seq(db.set_policy(t, *p, meta(seq))?.commit_seq, seq),
        Script::Retention(n) => {
            let policy = RetentionPolicy::all(RetentionRule::KeepLast(*n));
            match db.run_retention(&policy, RetentionBudget::default())?.change_set {
                Some(info) => check_seq(info.commit_seq, seq),
                None => {
                    let mut cs = db.begin_changeset(meta(seq))?;
                    cs.create(NewUnit {
                        unit_type: "doc".into(),
                        schema_version: 1,
                        metadata: Vec::new(),
                        payload: b"filler".to_vec(),
                    })?;
                    check_seq(cs.commit()?.commit_seq, seq);
                }
            }
        }
    }
    Ok(())
}

/// Every unit head of the store, by id.
pub fn heads<E: TxnEngine>(db: &UnitDb<E>) -> Result<Vec<(UnitId, UnitHead)>, UnitError> {
    let snap = db.engine().snapshot()?;
    let (start, end) = Key::tag_range(TAG_UNIT_HEAD);
    let mut out = Vec::new();
    let mut last: Option<Vec<u8>> = None;
    loop {
        let from = match &last {
            Some(k) => Bound::Excluded(k.as_slice()),
            None => Bound::Included(start.as_slice()),
        };
        let page = snap.scan(from, Bound::Excluded(end.as_slice()), 64)?;
        let n = page.len();
        for (k, v) in page {
            let Ok(Key::UnitHead { unit }) = Key::decode(&k) else { panic!("bad head key {k:?}") };
            out.push((unit, UnitHead::decode(&v).expect("head decodes")));
            last = Some(k);
        }
        if n < 64 {
            return Ok(out);
        }
    }
}

/// Runs steps `from + 1..=to` and returns the last acknowledged one, with
/// the error that stopped the run, if any (the change set after the returned
/// sequence may or may not have committed).
pub fn run_steps<E: TxnEngine>(
    db: &UnitDb<E>,
    ids: &StepIds,
    seed: u64,
    from: u64,
    to: u64,
) -> (u64, Option<UnitError>) {
    for k in from + 1..=to {
        let h = match heads(db) {
            Ok(h) => h,
            Err(e) => return (k - 1, Some(e)),
        };
        if let Err(e) = run_script(db, ids, k, &script_for(seed, k, &h)) {
            return (k - 1, Some(e));
        }
    }
    (to, None)
}

// ------------------------------------------------------------ the oracle

/// The raw store after each change set of the workload of `seed`, from an
/// undisturbed `Emulated<LogEngine>` (the trivially correct engine).
pub struct Oracle {
    seed: u64,
    ids: StepIds,
    db: UnitDb<Emulated<LogEngine>>,
    _disk: SimVfs,
    dumps: Vec<Dump>,
    /// The model after the last change set run, and its copy after each
    /// one (freed with the dumps).
    model: UnitModel,
    models: Vec<UnitModel>,
}

impl Oracle {
    pub fn new(seed: u64) -> Self {
        let disk = SimVfs::new(0);
        let ids = StepIds::new();
        let db = open_units::<Emulated<LogEngine>>(disk.as_vfs(), &unit_config(&ids, Durability::Sync))
            .expect("open the oracle");
        let dumps = vec![raw(&db)];
        let model = UnitModel::new();
        let models = vec![model.clone()];
        Oracle { seed, ids, db, _disk: disk, dumps, model, models }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Frees the stores below `seq` (a long run only moves forward).
    pub fn forget_below(&mut self, seq: u64) {
        for d in self.dumps.iter_mut().take(seq as usize) {
            *d = Dump::new();
        }
        for m in self.models.iter_mut().take(seq as usize) {
            *m = UnitModel::new();
        }
    }

    /// The store after exactly `seq` change sets.
    pub fn expected(&mut self, seq: u64) -> &Dump {
        self.run_to(seq);
        &self.dumps[seq as usize]
    }

    /// The model after exactly `seq` change sets.
    pub fn model(&mut self, seq: u64) -> &UnitModel {
        self.run_to(seq);
        &self.models[seq as usize]
    }

    fn run_to(&mut self, seq: u64) {
        while self.dumps.len() as u64 <= seq {
            let k = self.dumps.len() as u64;
            let h = heads(&self.db).expect("oracle heads");
            let script = script_for(self.seed, k, &h);
            if let Err(e) = run_script(&self.db, &self.ids, k, &script) {
                panic!("the oracle failed at step {k}: {e}");
            }
            model_step(&mut self.model, k, &script);
            self.dumps.push(raw(&self.db));
            self.models.push(self.model.clone());
        }
    }
}

/// Id of the `n`-th (from 1) unit change set `seq` creates ([`StepIds`]).
fn step_id(seq: u64, n: u64) -> UnitId {
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&((seq << 16) + n).to_be_bytes());
    UnitId(id)
}

fn new_unit_change(id: UnitId, unit_type: &str, metadata: Vec<u8>, payload: Vec<u8>) -> ModelChange {
    ModelChange {
        unit: id,
        kind: RevisionKind::Create,
        unit_type: unit_type.into(),
        schema_version: 1,
        lifecycle: Lifecycle::Active,
        restored_from: None,
        metadata,
        payload: Some(payload),
    }
}

/// Applies the change set `script` commits as `seq` to `m`, deciding each
/// revision from the operation and the model's own state (never from the
/// layer's records).
pub fn model_step(m: &mut UnitModel, seq: u64, script: &Script) {
    let cs = CommitSeq(seq);
    let filler = |m: &mut UnitModel| {
        let ch = new_unit_change(step_id(seq, 1), "doc", Vec::new(), b"filler".to_vec());
        m.apply(cs, Purpose::Application, vec![ch]).expect("model filler");
    };
    match script {
        Script::Policy(t, p) => m.set_policy(cs, t, *p).expect("model set_policy"),
        Script::Retention(n) => {
            let policy = RetentionPolicy::all(RetentionRule::KeepLast(*n));
            if m.retain(cs, &policy, T0, |_| T0).expect("model retain").is_none() {
                filler(m);
            }
        }
        Script::Changes(actions) => {
            let mut created = 0;
            let mut changes = Vec::new();
            for a in actions {
                let head = |id: &UnitId| m.head(*id).cloned().expect("model head");
                let ch = match a {
                    Action::Create { unit_type, payload } => {
                        created += 1;
                        new_unit_change(
                            step_id(seq, created),
                            unit_type,
                            format!("m{seq}").into_bytes(),
                            payload.clone(),
                        )
                    }
                    Action::Update { id, payload, schema } => {
                        let h = head(id);
                        ModelChange {
                            unit: *id,
                            kind: RevisionKind::Update,
                            unit_type: h.unit_type,
                            schema_version: schema.unwrap_or(h.schema_version),
                            lifecycle: Lifecycle::Active,
                            restored_from: None,
                            metadata: h.metadata,
                            payload: Some(payload.clone()),
                        }
                    }
                    Action::Archive(id) => keep(*id, head(id), RevisionKind::Archive, Lifecycle::Archived),
                    Action::Unarchive(id) => keep(*id, head(id), RevisionKind::Unarchive, Lifecycle::Active),
                    Action::Delete(id) => {
                        let mut c = keep(*id, head(id), RevisionKind::Delete, Lifecycle::Deleted);
                        c.payload = None;
                        c
                    }
                    Action::Restore(id, from) => {
                        let src = m.read_restore_source(*id, *from).expect("model restore source").clone();
                        ModelChange {
                            unit: *id,
                            kind: RevisionKind::Restore,
                            unit_type: src.unit_type,
                            schema_version: src.schema_version,
                            lifecycle: Lifecycle::Active,
                            restored_from: Some(*from),
                            metadata: src.metadata,
                            payload: src.payload,
                        }
                    }
                };
                changes.push(ch);
            }
            m.apply(cs, Purpose::Application, changes).expect("model apply");
        }
    }
}

/// A revision that keeps the head's content and moves its lifecycle.
fn keep(id: UnitId, h: ModelRevision, kind: RevisionKind, lifecycle: Lifecycle) -> ModelChange {
    ModelChange {
        unit: id,
        kind,
        unit_type: h.unit_type,
        schema_version: h.schema_version,
        lifecycle,
        restored_from: None,
        metadata: h.metadata,
        payload: h.payload,
    }
}

// ------------------------------------------------------------ checks

pub fn dump<S: Snapshot>(s: &S) -> Dump {
    let mut out: Dump = Vec::new();
    loop {
        let from = out.last().map(|(k, _)| k.clone());
        let start = match &from {
            Some(k) => Bound::Excluded(k.as_slice()),
            None => Bound::Unbounded,
        };
        let page = s.scan(start, Bound::Unbounded, 256).expect("scan");
        let n = page.len();
        out.extend(page);
        if n < 256 {
            return out;
        }
    }
}

pub fn raw<E: TxnEngine>(db: &UnitDb<E>) -> Dump {
    dump(&db.engine().snapshot().expect("snapshot"))
}

pub fn seq<E: TxnEngine>(db: &UnitDb<E>) -> u64 {
    db.engine().snapshot().expect("snapshot").seq()
}

/// The first difference between two stores, for failure messages.
pub fn first_difference(want: &Dump, got: &Dump) -> String {
    for (i, (w, g)) in want.iter().zip(got).enumerate() {
        if w != g {
            return format!("pair #{i}: want key {:02x?}, got key {:02x?} (values equal: {})", w.0, g.0, w.1 == g.1);
        }
    }
    format!("{} pairs expected, {} found", want.len(), got.len())
}

/// `verify_units` (E3-07) finds nothing.
#[track_caller]
pub fn verified<E: TxnEngine>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().unwrap_or_else(|e| panic!("{ctx}: verify_units: {e}"));
    assert!(r.is_clean(), "{ctx}: verify_units found {:?}", r.findings);
}

/// The recovered sequence `s` is in `lo..=hi`, the store is byte for byte the
/// one the same layer code leaves after exactly `s` change sets on the
/// reference engine (I-21 at the engine boundary), `verify_units` finds
/// nothing, and the public reads agree with the independent [`UnitModel`]
/// after the same `s` acknowledged change sets ([`check_model`], which also
/// reads every head and so checks its payload hash). Returns `s`.
#[track_caller]
pub fn check_store<E: TxnEngine>(db: &UnitDb<E>, oracle: &mut Oracle, lo: u64, hi: u64, ctx: &str) -> u64 {
    let s = seq(db);
    assert!(lo <= s && s <= hi, "{ctx}: recovered {s} change sets, expected {lo}..={hi}");
    let got = raw(db);
    let want = oracle.expected(s);
    assert!(
        got == *want,
        "{ctx}: the store is not the one after {s} whole change sets: {}",
        first_difference(want, &got)
    );
    verified(db, ctx);
    check_model(db, oracle.model(s), ctx);
    s
}

/// The database's public reads answer what `model` says: the same unit ids;
/// each head (type, schema, lifecycle, revision, oldest retained revision,
/// metadata, payload); every revision from the oldest retained to the head
/// through `get_revision` (kind, sequence, lifecycle, restore source,
/// metadata, content hash, payload or the same refusal); every change set
/// with its purpose and members; the policy of every type.
#[track_caller]
pub fn check_model<E: TxnEngine>(db: &UnitDb<E>, model: &UnitModel, ctx: &str) {
    let snap = db.snapshot().unwrap_or_else(|e| panic!("{ctx}: snapshot: {e}"));
    let got_ids: Vec<UnitId> =
        heads(db).unwrap_or_else(|e| panic!("{ctx}: heads: {e}")).into_iter().map(|(id, _)| id).collect();
    let want_ids: Vec<UnitId> = model.unit_ids().collect();
    assert_eq!(got_ids, want_ids, "{ctx}: model: unit ids");
    for t in TYPES {
        assert_eq!(snap.policy(t).unwrap(), model.policy(t), "{ctx}: model: policy of {t}");
    }
    for id in want_ids {
        let h = model.head(id).expect("model head");
        let u = snap
            .get_unit(id)
            .unwrap_or_else(|e| panic!("{ctx}: get_unit {id}: {e}"))
            .unwrap_or_else(|| panic!("{ctx}: get_unit {id}: missing"));
        assert_eq!(
            (&u.unit_type, u.schema_version, u.lifecycle, u.head, u.oldest, &u.metadata, &u.payload),
            (&h.unit_type, h.schema_version, h.lifecycle, h.rev, model.oldest(id), &h.metadata, &h.payload),
            "{ctx}: model: head of {id}"
        );
        for r in model.oldest(id).0..=h.rev.0 {
            let c = format!("{ctx}: model: revision {id} {r}");
            match (snap.get_revision(id, RevNo(r)), model.read_revision(id, RevNo(r))) {
                (Ok(g), Ok(w)) => {
                    assert_eq!(
                        (g.id.rev, g.kind, g.commit_seq, g.lifecycle, g.restored_from, &g.metadata),
                        (w.rev, w.kind, w.commit_seq, w.lifecycle, w.restored_from, &w.metadata),
                        "{c}"
                    );
                    let bytes = w.payload.as_deref().unwrap_or(&[]);
                    assert_eq!(g.content_hash, ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, bytes), "{c}: hash");
                    assert_eq!(g.payload, w.payload, "{c}: payload");
                }
                (Err(g), Err(w)) => assert_eq!(format!("{g:?}"), format!("{w:?}"), "{c}"),
                (g, w) => panic!("{c}: got {g:?}, want {w:?}"),
            }
        }
    }
    let got = snap.change_sets_since(CommitSeq(0), usize::MAX).unwrap_or_else(|e| panic!("{ctx}: change sets: {e}"));
    let want = model.change_sets_since(CommitSeq(0), usize::MAX);
    assert_eq!(got.len(), want.len(), "{ctx}: model: number of change sets");
    for (g, w) in got.iter().zip(want) {
        let members: Vec<_> = g.members.iter().map(|m| (m.revision.unit, m.revision.rev, m.kind)).collect();
        assert_eq!((g.id.0, g.purpose, &members), (w.seq.0, w.purpose, &w.members), "{ctx}: model: change set");
    }
}

/// The end of a sweep point: the recovered handle commits the next change
/// set, closes, loses power, and everything is still there.
pub fn keeps_working<E: Target>(
    db: UnitDb<E>,
    disk: &SimVfs,
    cfg: &UnitConfig,
    ids: &StepIds,
    oracle: &mut Oracle,
    s: u64,
    ctx: &str,
) {
    let (acked, err) = run_steps(&db, ids, oracle.seed(), s, s + 1);
    assert!(err.is_none(), "{ctx}: change set after recovery: {err:?}");
    check_store(&db, oracle, acked, acked, &format!("{ctx}, after one more change set"));
    db.close().unwrap_or_else(|e| panic!("{ctx}: close after recovery: {e}"));
    let image = disk.crash_now(CrashKind::PowerLoss);
    let db = open_units::<E>(image.as_vfs(), cfg).unwrap_or_else(|e| panic!("{ctx}: reopen after power loss: {e}"));
    check_store(&db, oracle, s + 1, s + 1, &format!("{ctx}, then power loss after a clean close"));
}

pub fn is_poisoned(e: &UnitError) -> bool {
    matches!(e, UnitError::Storage(Error::Poisoned(_)))
}

pub fn is_io(e: &UnitError) -> bool {
    matches!(e, UnitError::Storage(Error::Io(_)))
}
