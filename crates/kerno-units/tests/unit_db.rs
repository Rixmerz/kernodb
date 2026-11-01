//! E3-03: change sets and CRUD of `UnitDb` (etapa3-design.md §4, §6, §8.1,
//! §11). Every generic suite runs over `Emulated<LogEngine>` (the trivially
//! correct engine) and over `KernoDb`.
//!
//! * `model_ops` — random change sets (create, update, archive, unarchive,
//!   delete, invalid operations, aborts) compared with `UnitModel` after each
//!   one; old snapshots re-read at the end and the database reopened.
//! * `seq_prediction` — the predicted `commit_seq` is the committed one, with
//!   concurrent blind commits and change sets; aborts consume nothing.
//! * `changeset_every_point` (fast version) — a crash before every mutating
//!   I/O operation of a fixed scenario recovers exactly the store after a
//!   whole prefix of change sets (I-21 by construction).
//! * limits, errors, open/compatibility and corruption checks.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use kerno_core::contract::{Engine, Options, Snapshot, Txn, TxnEngine};
use kerno_core::{Durability, WriteBatch};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::{Emulated, EmulatedTxn};
use kerno_testkit::harness::seeds;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, SimVfs, Vfs};
use kerno_units::codec::{Key, LayerMeta, Record, RevisionMeta, UnitHead};
use kerno_units::payload::PAYLOAD_CODEC_OPAQUE;
use kerno_units::*;

const DIR: &str = "/db";
const T0: Timestamp = Timestamp(1_700_000_000_000_000);

type Dump = Vec<(Vec<u8>, Vec<u8>)>;

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: TxnEngine>(disk: &SimVfs, c: &UnitConfig) -> Result<UnitDb<E>, UnitError> {
    UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), c)
}

fn dump<S: Snapshot>(s: &S) -> Dump {
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

fn raw<E: TxnEngine>(db: &UnitDb<E>) -> Dump {
    dump(&db.engine().snapshot().expect("snapshot"))
}

fn seq<E: TxnEngine>(db: &UnitDb<E>) -> u64 {
    db.engine().snapshot().expect("snapshot").seq()
}

fn meta() -> ChangeMeta {
    ChangeMeta::default()
}

fn new_unit(t: &str, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: 1, metadata: b"m".to_vec(), payload: payload.to_vec() }
}

fn upd(payload: &[u8]) -> UnitUpdate {
    UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }
}

fn is_corruption(e: &UnitError, rule: &str) -> bool {
    matches!(e, UnitError::Storage(kerno_core::Error::Corruption(m)) if m.contains(rule))
}

// ------------------------------------------------------------ model_ops

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Ok,
    NotFound,
    Conflict,
    Deleted,
    Invalid,
}

fn outcome<T>(r: &Result<T, UnitError>) -> Outcome {
    match r {
        Ok(_) => Outcome::Ok,
        Err(UnitError::NotFound { .. }) => Outcome::NotFound,
        Err(UnitError::Conflict { .. }) => Outcome::Conflict,
        Err(UnitError::Deleted { .. }) => Outcome::Deleted,
        Err(UnitError::InvalidArgument(_)) => Outcome::Invalid,
        Err(e) => panic!("unexpected error {e}"),
    }
}

/// The unit the model says `get_unit` returns.
fn expected_unit(model: &UnitModel, id: UnitId) -> Option<Unit> {
    let h = model.history(id);
    let head = h.last()?;
    Some(Unit {
        id,
        unit_type: head.unit_type.clone(),
        schema_version: head.schema_version,
        lifecycle: head.lifecycle,
        head: head.rev,
        oldest: RevNo(1),
        created_seq: h[0].commit_seq,
        updated_seq: head.commit_seq,
        created_at: T0,
        updated_at: T0,
        metadata: head.metadata.clone(),
        payload: head.payload.clone(),
        content_hash: ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, head.payload.as_deref().unwrap_or(&[])),
    })
}

fn compare<E: Engine>(snap: &UnitSnapshot<E>, model: &UnitModel, ctx: &str) {
    assert_eq!(snap.seq(), model.last_seq(), "{ctx}: sequence");
    for id in model.unit_ids().chain([UnitId([0xEE; 16])]) {
        let got = snap.get_unit(id).unwrap_or_else(|e| panic!("{ctx}: get_unit {id}: {e}"));
        assert_eq!(got, expected_unit(model, id), "{ctx}: unit {id}");
    }
}

fn model_ops<E: TxnEngine>() {
    for seed in seeds(6) {
        let mut rng = Rng::new(seed);
        let disk = SimVfs::new(seed);
        let c = cfg();
        let mut db = open::<E>(&disk, &c).expect("open");
        let mut model = UnitModel::new();
        let mut kept: Vec<(UnitSnapshot<E>, UnitModel)> = Vec::new();
        let unknown = UnitId([0xEE; 16]);
        for round in 0..40 {
            let ctx = format!("seed {seed} round {round}");
            let before = seq(&db);
            let mut cs = db.begin_changeset(meta()).expect("begin");
            assert_eq!(cs.commit_seq(), CommitSeq(before + 1), "{ctx}: predicted sequence");
            let mut changes: Vec<ModelChange> = Vec::new();
            let mut touched = BTreeSet::new();
            for _ in 0..rng.range(1, 8) {
                let ids: Vec<UnitId> = model.unit_ids().collect();
                let op = if ids.is_empty() { 0 } else { rng.below(6) };
                if op == 0 {
                    let len = if rng.chance(1, 10) { 2000 } else { rng.below(300) as usize };
                    let payload = rng.bytes(len);
                    let ty = format!("t{}", rng.below(3));
                    let id = cs
                        .create(NewUnit {
                            unit_type: ty.clone(),
                            schema_version: 1,
                            metadata: vec![],
                            payload: payload.clone(),
                        })
                        .expect("create");
                    assert!(model.head(id).is_none() && touched.insert(id), "{ctx}: fresh id");
                    changes.push(ModelChange {
                        unit: id,
                        kind: RevisionKind::Create,
                        unit_type: ty,
                        schema_version: 1,
                        lifecycle: Lifecycle::Active,
                        restored_from: None,
                        metadata: vec![],
                        payload: Some(payload),
                    });
                    continue;
                }
                let id = if op == 5 { unknown } else { ids[rng.below(ids.len() as u64) as usize] };
                let head = model.head(id).cloned();
                let expected = match (rng.below(4), &head) {
                    (0, Some(h)) => Some(RevNo(h.rev.0 + 1)),
                    (1, Some(h)) => Some(h.rev),
                    _ => None,
                };
                // What the layer must answer, in its documented order.
                let mut predicted = if touched.contains(&id) {
                    Outcome::Invalid
                } else {
                    match &head {
                        None => Outcome::NotFound,
                        Some(h) if expected.is_some_and(|e| e != h.rev) => Outcome::Conflict,
                        Some(h) if h.lifecycle == Lifecycle::Deleted => Outcome::Deleted,
                        Some(_) => Outcome::Ok,
                    }
                };
                let cur = head.clone();
                let (result, change) = match rng.below(4) {
                    0 => {
                        let len = rng.below(200) as usize;
                        let payload = rng.bytes(len);
                        let meta_len = rng.below(10) as usize;
                        let new_meta = rng.chance(1, 2).then(|| rng.bytes(meta_len));
                        let new_schema = rng.chance(1, 3).then(|| rng.below(5) as u32);
                        let r = cs.update(
                            id,
                            UnitUpdate {
                                payload: payload.clone(),
                                schema_version: new_schema,
                                metadata: new_meta.clone(),
                            },
                            expected,
                        );
                        if predicted == Outcome::Ok && cur.as_ref().unwrap().lifecycle != Lifecycle::Active {
                            predicted = Outcome::Invalid;
                        }
                        let ch = cur.map(|h| ModelChange {
                            unit: id,
                            kind: RevisionKind::Update,
                            unit_type: h.unit_type,
                            schema_version: new_schema.unwrap_or(h.schema_version),
                            lifecycle: Lifecycle::Active,
                            restored_from: None,
                            metadata: new_meta.unwrap_or(h.metadata),
                            payload: Some(payload),
                        });
                        (r, ch)
                    }
                    1 | 2 => {
                        let archive = rng.below(2) == 0;
                        let r = if archive { cs.archive(id, expected) } else { cs.unarchive(id, expected) };
                        let (from, to, kind) = if archive {
                            (Lifecycle::Active, Lifecycle::Archived, RevisionKind::Archive)
                        } else {
                            (Lifecycle::Archived, Lifecycle::Active, RevisionKind::Unarchive)
                        };
                        if predicted == Outcome::Ok && cur.as_ref().unwrap().lifecycle != from {
                            predicted = Outcome::Invalid;
                        }
                        let ch =
                            cur.map(|h| ModelChange { unit: id, kind, lifecycle: to, restored_from: None, ..base(h) });
                        (r, ch)
                    }
                    _ => {
                        let r = cs.delete(id, expected);
                        let ch = cur.map(|h| ModelChange {
                            unit: id,
                            kind: RevisionKind::Delete,
                            lifecycle: Lifecycle::Deleted,
                            payload: None,
                            restored_from: None,
                            ..base(h)
                        });
                        (r, ch)
                    }
                };
                assert_eq!(outcome(&result), predicted, "{ctx}: unit {id}: {result:?}");
                if let Ok(rev) = result {
                    let ch = change.expect("change");
                    assert_eq!(rev, RevNo(head.unwrap().rev.0 + 1), "{ctx}: new revision");
                    touched.insert(id);
                    changes.push(ch);
                }
            }
            // The change set reads its own writes.
            if let Some(ch) = changes.last() {
                let seen = cs.get_unit(ch.unit).expect("own read").expect("own unit");
                assert_eq!(seen.lifecycle, ch.lifecycle, "{ctx}: own write");
                assert_eq!(seen.updated_seq, CommitSeq(before + 1), "{ctx}: own write sequence");
            }
            if rng.chance(1, 6) {
                if rng.chance(1, 2) {
                    cs.abort();
                } else {
                    drop(cs);
                }
                assert_eq!(seq(&db), before, "{ctx}: abort consumed a sequence");
            } else {
                let info = cs.commit().expect("commit");
                assert!(info.durable);
                if changes.is_empty() {
                    assert_eq!(info.commit_seq, CommitSeq(before), "{ctx}: empty change set");
                    assert!(info.revisions.is_empty());
                } else {
                    assert_eq!(info.commit_seq, CommitSeq(before + 1), "{ctx}");
                    assert_eq!(info.change_set, info.commit_seq);
                    let applied = model.apply(info.commit_seq, Purpose::Application, changes).expect("model");
                    let revs: Vec<RevisionId> = applied
                        .expect("non-empty")
                        .members
                        .iter()
                        .map(|&(unit, rev, _)| RevisionId { unit, rev })
                        .collect();
                    assert_eq!(info.revisions, revs, "{ctx}: revisions");
                }
            }
            let snap = db.snapshot().expect("snapshot");
            compare(&snap, &model, &ctx);
            if rng.chance(1, 5) {
                kept.push((snap, model.clone()));
            }
        }
        for (i, (snap, m)) in kept.iter().enumerate() {
            compare(snap, m, &format!("seed {seed} kept snapshot {i}"));
        }
        drop(kept);
        db.close().expect("close");
        db = open::<E>(&disk, &c).expect("reopen");
        compare(&db.snapshot().expect("snapshot"), &model, &format!("seed {seed} after reopen"));
        db.close().expect("close");
    }
}

/// The fields of a change that keeps the unit's content.
fn base(h: ModelRevision) -> ModelChange {
    ModelChange {
        unit: UnitId([0; 16]),
        kind: h.kind,
        unit_type: h.unit_type,
        schema_version: h.schema_version,
        lifecycle: h.lifecycle,
        restored_from: None,
        metadata: h.metadata,
        payload: h.payload,
    }
}

// ------------------------------------------------------------ seq_prediction

fn seq_prediction<E: TxnEngine>() {
    let disk = SimVfs::new(1);
    let mut c = cfg();
    c.engine.durability = Durability::Buffered;
    let db = open::<E>(&disk, &c).expect("open");

    // Deterministic part: aborts, drops and empty change sets consume nothing.
    let (a, info) = db.create_unit(new_unit("doc", b"one"), meta()).expect("create");
    assert_eq!(info.commit_seq, CommitSeq(1));
    let mut cs = db.begin_changeset(meta()).expect("begin");
    cs.update(a, upd(b"two"), None).expect("update");
    cs.abort();
    let mut cs = db.begin_changeset(meta()).expect("begin");
    cs.create(new_unit("doc", b"x")).expect("create");
    drop(cs);
    let info = db.begin_changeset(meta()).expect("begin").commit().expect("empty commit");
    assert_eq!((info.commit_seq, info.revisions.len()), (CommitSeq(1), 0));
    assert_eq!(seq(&db), 1);
    let mut cs = db.begin_changeset(meta()).expect("begin");
    assert_eq!(cs.commit_seq(), CommitSeq(2));
    cs.update(a, upd(b"two"), Some(RevNo(1))).expect("update");
    assert_eq!(cs.commit().expect("commit").commit_seq, CommitSeq(2));

    // Concurrent part: change sets and blind commits interleave.
    let blind_key = Key::LayerMeta { name: "probe".into() }.encode().expect("key");
    let infos: Mutex<Vec<CommitInfo>> = Mutex::new(Vec::new());
    let blind: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for t in 0..2u64 {
            let (db, infos) = (&db, &infos);
            s.spawn(move || {
                let mut rng = Rng::new(100 + t);
                let mut mine: Vec<UnitId> = Vec::new();
                for _ in 0..25 {
                    let mut cs = db.begin_changeset(meta()).expect("begin");
                    let id = cs.create(new_unit("doc", &rng.bytes(16))).expect("create");
                    if let Some(&old) = mine.last() {
                        cs.update(old, upd(&rng.bytes(8)), None).expect("update");
                    }
                    mine.push(id);
                    if rng.chance(1, 4) {
                        mine.pop();
                        cs.abort();
                        continue;
                    }
                    infos.lock().unwrap().push(cs.commit().expect("commit"));
                }
            });
        }
        for _ in 0..2 {
            let (db, blind, k) = (&db, &blind, blind_key.clone());
            s.spawn(move || {
                for _ in 0..25 {
                    let mut b = WriteBatch::new();
                    b.delete(k.clone());
                    blind.lock().unwrap().push(db.engine().commit(&b).expect("blind commit"));
                }
            });
        }
    });
    let infos = infos.into_inner().unwrap();
    let mut all: Vec<u64> = blind.into_inner().unwrap();
    all.extend(infos.iter().map(|i| i.commit_seq.0));
    all.sort_unstable();
    let last = seq(&db);
    assert_eq!(all, (3..=last).collect::<Vec<_>>(), "every commit got the next sequence; aborts consumed none");
    let snap = db.engine().snapshot().expect("snapshot");
    for info in &infos {
        let cs_raw = snap.get(&Key::ChangeSet { seq: info.commit_seq }.encode().unwrap()).unwrap().expect("CHANGESET");
        let Record::ChangeSet { record, .. } =
            Record::decode(&Key::ChangeSet { seq: info.commit_seq }.encode().unwrap(), &cs_raw).unwrap()
        else {
            unreachable!()
        };
        assert_eq!(record.member_count as usize, info.revisions.len());
        for r in &info.revisions {
            let raw =
                snap.get(&Key::RevMeta { unit: r.unit, rev: r.rev }.encode().unwrap()).unwrap().expect("REV_META");
            let m = RevisionMeta::decode(r.rev, &raw).expect("decode");
            assert_eq!(m.commit_seq, info.commit_seq, "REV_META carries the committed sequence");
        }
    }
    drop(snap);
    db.close().expect("close");
}

// ------------------------------------------------------------ changeset_every_point

type ScenarioStep<E> = fn(&mut ChangeSet<'_, E>) -> Result<(), UnitError>;

fn id(n: u64) -> UnitId {
    let mut b = [0u8; 16];
    b[8..].copy_from_slice(&n.to_be_bytes());
    UnitId(b)
}

fn steps<E: TxnEngine>() -> [ScenarioStep<E>; 4] {
    [
        |cs| {
            cs.create(new_unit("doc", &[1; 300]))?;
            cs.create(new_unit("doc", &[2; 900]))?;
            cs.create(new_unit("img", &[3; 50]))?;
            Ok(())
        },
        |cs| {
            cs.update(id(1), upd(&[4; 400]), Some(RevNo(1)))?;
            cs.archive(id(2), None)?;
            cs.delete(id(3), None)?;
            Ok(())
        },
        |cs| {
            cs.unarchive(id(2), None)?;
            cs.update(
                id(1),
                UnitUpdate { payload: vec![5; 2000], schema_version: Some(2), metadata: Some(b"v2".to_vec()) },
                None,
            )?;
            cs.create(new_unit("doc", &[6; 10]))?;
            Ok(())
        },
        |cs| {
            cs.delete(id(1), None)?;
            cs.update(id(4), upd(&[7; 64]), None)?;
            Ok(())
        },
    ]
}

/// Runs the first `limit` change sets of the fixed scenario and returns how
/// many committed (they stop at the first error).
fn scenario<E: TxnEngine>(db: &UnitDb<E>, limit: usize) -> usize {
    for (i, step) in steps::<E>().iter().take(limit).enumerate() {
        let m = ChangeMeta { reason: Some(format!("step {i}")), actor: Some(b"tester".to_vec()), ..meta() };
        let Ok(mut cs) = db.begin_changeset(m) else { return i };
        if step(&mut cs).is_err() || cs.commit().is_err() {
            return i;
        }
    }
    limit.min(4)
}

/// Fast version of the sweep of design §8.2: a crash before every mutating
/// I/O operation of the scenario (open included). After recovery the whole
/// store must equal the store after exactly `s` change sets, `s` being the
/// recovered sequence: no change set is ever half there (I-21).
fn changeset_every_point<E: TxnEngine>(kind: CrashKind, durability: Durability) {
    let config = || {
        let mut c = cfg();
        c.engine.durability = durability;
        c
    };
    let mut prefixes: Vec<Dump> = Vec::new();
    let mut total = 0;
    for k in 0..=4 {
        let disk = SimVfs::new(7);
        let db = open::<E>(&disk, &config()).expect("open");
        assert_eq!(scenario(&db, k), k);
        prefixes.push(raw(&db));
        total = disk.mutating_ops();
        drop(db);
    }
    assert!(total > 0);
    for point in 1..=total {
        let ctx = format!("{kind:?} {durability:?} before mutating op #{point} of {total}");
        let disk = SimVfs::new(7);
        disk.crash_after(point, kind);
        let acked = match open::<E>(&disk, &config()) {
            Ok(db) => {
                let n = scenario(&db, 4);
                drop(db);
                n
            }
            Err(_) => 0,
        };
        assert!(disk.has_crashed(), "{ctx}: the crash did not fire");
        let image = disk.take_crash_image().expect("crash image");
        // Ids after recovery come from a range the scenario never uses.
        let after = UnitConfig { ids: Arc::new(CounterIds::new(1_000)), ..config() };
        let db = open::<E>(&image, &after).unwrap_or_else(|e| panic!("{ctx}: reopen: {e}"));
        let s = seq(&db) as usize;
        assert!(s >= acked && s <= (acked + 1).min(4), "{ctx}: recovered {s} change sets, {acked} acknowledged");
        assert!(raw(&db) == prefixes[s], "{ctx}: the store is not the one after {s} whole change sets");
        for n in 1..=4 {
            db.get_unit(id(n)).unwrap_or_else(|e| panic!("{ctx}: get_unit: {e}"));
        }
        // And the database keeps working.
        db.create_unit(new_unit("doc", b"after"), meta())
            .unwrap_or_else(|e| panic!("{ctx}: write after recovery: {e}"));
        drop(db);
    }
}

// ------------------------------------------------------------ records and compatibility

/// Every pair the layer writes decodes with the production codecs, every
/// payload verifies, and the change set records match their members (IU-4).
fn written_records_decode<E: TxnEngine>() {
    let disk = SimVfs::new(3);
    let db = open::<E>(&disk, &cfg()).expect("open");
    assert_eq!(scenario(&db, 4), 4);
    let all = raw(&db);
    let mut records = Vec::new();
    for (k, v) in &all {
        records.push(Record::decode(k, v).unwrap_or_else(|e| panic!("{e}")));
    }
    let tags: BTreeSet<u8> = all.iter().map(|(k, _)| k[0]).collect();
    assert_eq!(tags, BTreeSet::from([0x01, 0x10, 0x20, 0x21, 0x30, 0x31]));
    let metas: BTreeMap<(UnitId, RevNo), RevisionMeta> = records
        .iter()
        .filter_map(|r| match r {
            Record::RevMeta { unit, rev, meta } => Some(((*unit, *rev), meta.clone())),
            _ => None,
        })
        .collect();
    let payloads: BTreeMap<(UnitId, RevNo), Vec<u8>> = records
        .iter()
        .filter_map(|r| match r {
            Record::RevPayload { unit, rev, payload } => Some(((*unit, *rev), payload.clone())),
            _ => None,
        })
        .collect();
    for ((unit, rev), m) in &metas {
        verify_payload(&m.payload_decl(), payloads.get(&(*unit, *rev)).map(Vec::as_slice)).expect("payload verifies");
        assert_eq!(m.parent_rev, (rev.0 > 1).then(|| RevNo(rev.0 - 1)), "parent_rev = rev - 1");
        assert_eq!(m.actor_ref, b"tester");
        if m.kind == RevisionKind::Delete {
            assert!(!m.payload_present && m.payload_len == 0);
        }
    }
    assert_eq!(payloads.len(), metas.values().filter(|m| m.payload_present).count());
    let mut members: BTreeMap<CommitSeq, u32> = BTreeMap::new();
    for r in &records {
        match r {
            Record::LayerMeta { name, meta } => {
                assert_eq!(name, "layer");
                assert_eq!(*meta, LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned });
            }
            Record::CsMember { seq, member, .. } => {
                *members.entry(*seq).or_default() += 1;
                assert_eq!(metas[&(member.unit, member.rev_no)].commit_seq, *seq, "member names its revision");
                assert_eq!(metas[&(member.unit, member.rev_no)].kind, member.kind);
            }
            _ => {}
        }
    }
    for r in &records {
        if let Record::ChangeSet { seq, record } = r {
            assert_eq!(record.member_count, members[seq]);
            assert_eq!(record.reason, format!("step {}", seq.0 - 1).into_bytes());
            assert_eq!(record.purpose, Purpose::Application);
            assert_eq!(record.created_at, T0);
        }
    }
    let heads = records.iter().filter(|r| matches!(r, Record::UnitHead { .. })).count();
    assert_eq!(heads, 4);
    // Deleted unit: head kept, no payload.
    let gone = db.get_unit(id(1)).unwrap().expect("tombstone head");
    assert_eq!((gone.lifecycle, gone.payload, gone.head), (Lifecycle::Deleted, None, RevNo(4)));
    assert_eq!(gone.content_hash, ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, &[]));
    let back = db.get_unit(id(2)).unwrap().expect("unit 2");
    assert_eq!(
        (back.lifecycle, back.payload.as_deref(), back.head),
        (Lifecycle::Active, Some(&[2u8; 900][..]), RevNo(3))
    );
}

// ------------------------------------------------------------ limits

fn payload_limits<E: TxnEngine>() {
    let disk = SimVfs::new(4);
    let mut c = cfg();
    c.max_payload_len = 4096;
    let db = open::<E>(&disk, &c).expect("open");
    let (a, _) = db.create_unit(new_unit("doc", &[0; 4096]), meta()).expect("exactly at the limit");
    let mut cs = db.begin_changeset(meta()).unwrap();
    let e = cs.create(new_unit("doc", &[0; 4097])).unwrap_err();
    assert!(matches!(e, UnitError::PayloadTooLarge { len: 4097, max: 4096 }), "{e}");
    let e = cs.update(a, upd(&[0; 4097]), None).unwrap_err();
    assert!(matches!(e, UnitError::PayloadTooLarge { len: 4097, max: 4096 }), "{e}");
    // Refused operations leave no effect and the change set usable.
    assert_eq!(cs.commit().unwrap().revisions.len(), 0);
    assert_eq!(seq(&db), 1);
    assert_eq!(db.get_unit(a).unwrap().unwrap().head, RevNo(1));
    // Configuration limits.
    for bad in [UnitConfig { max_payload_len: (1 << 26) + 1, ..cfg() }, UnitConfig { max_changeset_ops: 0, ..cfg() }] {
        assert!(matches!(open::<E>(&SimVfs::new(0), &bad), Err(UnitError::InvalidArgument(_))));
    }
    db.close().unwrap();
}

fn changeset_limits<E: TxnEngine>() {
    let disk = SimVfs::new(5);
    let mut c = cfg();
    c.max_changeset_ops = 3;
    let db = open::<E>(&disk, &c).expect("open");
    let mut cs = db.begin_changeset(meta()).unwrap();
    for _ in 0..3 {
        cs.create(new_unit("doc", b"x")).expect("within the limit");
    }
    let e = cs.create(new_unit("doc", b"x")).unwrap_err();
    assert!(matches!(e, UnitError::ChangeSetTooLarge { ops: 4, .. }), "{e}");
    let info = cs.commit().expect("the three operations commit");
    assert_eq!(info.revisions.len(), 3);
    assert_eq!(seq(&db), 1);
    assert_eq!(raw(&db).iter().filter(|(k, _)| k[0] == 0x10).count(), 3, "nothing of the refused operation");
    db.close().unwrap();
}

fn field_limits<E: TxnEngine>() {
    let disk = SimVfs::new(6);
    let db = open::<E>(&disk, &cfg()).expect("open");
    let mut cs = db.begin_changeset(meta()).unwrap();
    let long = "x".repeat(65);
    for t in ["", long.as_str()] {
        assert!(matches!(cs.create(new_unit(t, b"p")), Err(UnitError::InvalidArgument(_))), "unit_type {t:?}");
    }
    cs.create(new_unit(&"x".repeat(64), b"p")).expect("64-byte unit_type");
    let big_meta = NewUnit { metadata: vec![0; 4097], ..new_unit("doc", b"p") };
    assert!(matches!(cs.create(big_meta), Err(UnitError::InvalidArgument(_))));
    cs.create(NewUnit { metadata: vec![0; 4096], ..new_unit("doc", b"p") }).expect("4096-byte metadata");
    cs.create(new_unit("doc", b"")).expect("empty payload");
    let info = cs.commit().unwrap();
    assert_eq!(info.revisions.len(), 3);
    let empty = db.get_unit(info.revisions[2].unit).unwrap().unwrap();
    assert_eq!(empty.payload, Some(Vec::new()));
    let bad_metas = [
        ChangeMeta { reason: Some("r".repeat(1025)), ..meta() },
        ChangeMeta { actor: Some(vec![1; 257]), ..meta() },
        ChangeMeta { source: Some(vec![1; 257]), ..meta() },
        ChangeMeta { correlation: Some(vec![1; 1025]), ..meta() },
        ChangeMeta { actor: Some(Vec::new()), ..meta() },
        ChangeMeta { purpose: Purpose::Retention, ..meta() },
        ChangeMeta { purpose: Purpose::Maintenance, ..meta() },
    ];
    for m in bad_metas {
        assert!(matches!(db.begin_changeset(m.clone()), Err(UnitError::InvalidArgument(_))), "{m:?}");
    }
    let ok = ChangeMeta {
        reason: Some("r".repeat(1024)),
        actor: Some(vec![1; 256]),
        source: Some(vec![2; 256]),
        correlation: Some(vec![3; 1024]),
        purpose: Purpose::Application,
    };
    let mut cs = db.begin_changeset(ok).expect("at every limit");
    cs.create(new_unit("doc", b"p")).unwrap();
    cs.commit().unwrap();
    assert_eq!(seq(&db), 2);
    db.close().unwrap();
}

// ------------------------------------------------------------ errors

fn operation_errors<E: TxnEngine>() {
    let disk = SimVfs::new(8);
    let db = open::<E>(&disk, &cfg()).expect("open");
    let (a, _) = db.create_unit(new_unit("doc", b"a1"), meta()).unwrap();
    let ghost = UnitId([0xAB; 16]);
    let mut cs = db.begin_changeset(meta()).unwrap();
    assert!(matches!(cs.update(ghost, upd(b"x"), None), Err(UnitError::NotFound { unit }) if unit == ghost));
    assert!(matches!(cs.archive(ghost, None), Err(UnitError::NotFound { .. })));
    assert!(matches!(cs.delete(ghost, None), Err(UnitError::NotFound { .. })));
    let e = cs.update(a, upd(b"x"), Some(RevNo(7))).unwrap_err();
    assert!(matches!(e, UnitError::Conflict { unit, expected: RevNo(7), found: RevNo(1) } if unit == a), "{e}");
    assert!(matches!(cs.unarchive(a, None), Err(UnitError::InvalidArgument(_))), "unarchive of an active unit");
    cs.update(a, upd(b"a2"), Some(RevNo(1))).unwrap();
    assert!(matches!(cs.archive(a, None), Err(UnitError::InvalidArgument(_))), "second revision in one change set");
    cs.commit().unwrap();

    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.archive(a, Some(RevNo(2))).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    assert!(matches!(cs.update(a, upd(b"x"), None), Err(UnitError::InvalidArgument(_))), "update of an archived unit");
    assert!(matches!(cs.archive(a, None), Err(UnitError::InvalidArgument(_))), "archive twice");
    cs.delete(a, None).expect("delete an archived unit");
    cs.commit().unwrap();
    let u = db.get_unit(a).unwrap().unwrap();
    assert_eq!((u.lifecycle, u.head, u.payload), (Lifecycle::Deleted, RevNo(4), None));

    let mut cs = db.begin_changeset(meta()).unwrap();
    assert!(matches!(cs.update(a, upd(b"x"), None), Err(UnitError::Deleted { .. })));
    assert!(matches!(cs.unarchive(a, None), Err(UnitError::Deleted { .. })));
    assert!(matches!(cs.delete(a, None), Err(UnitError::Deleted { .. })));
    // `expected` is checked before the tombstone.
    assert!(matches!(cs.delete(a, Some(RevNo(3))), Err(UnitError::Conflict { .. })));
    assert_eq!(cs.commit().unwrap().commit_seq, CommitSeq(4), "nothing to commit");
    assert_eq!(db.get_unit(ghost).unwrap(), None);
    db.close().unwrap();
}

struct SameId;
impl UnitIdSource for SameId {
    fn next_id(&self) -> UnitId {
        UnitId([7; 16])
    }
}

fn already_exists<E: TxnEngine>() {
    let disk = SimVfs::new(9);
    let mut c = cfg();
    c.ids = Arc::new(SameId);
    let db = open::<E>(&disk, &c).expect("open");
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.create(new_unit("doc", b"1")).unwrap();
    assert!(matches!(cs.create(new_unit("doc", b"2")), Err(UnitError::AlreadyExists { .. })), "within the change set");
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.delete(UnitId([7; 16]), None).unwrap();
    cs.commit().unwrap();
    // A deleted unit's id is never reused.
    assert!(matches!(db.create_unit(new_unit("doc", b"3"), meta()), Err(UnitError::AlreadyExists { .. })));
    assert_eq!(seq(&db), 2);
    db.close().unwrap();
}

// ------------------------------------------------------------ open and the layer mark

fn raw_commit<E: Engine>(disk: &SimVfs, puts: &[(&[u8], &[u8])]) {
    let e = E::open(disk.as_vfs(), Path::new(DIR), &Options::small_for_tests()).expect("engine");
    let mut b = WriteBatch::new();
    for (k, v) in puts {
        b.put(k.to_vec(), v.to_vec());
    }
    e.commit(&b).expect("commit");
    e.close().expect("close");
}

fn open_and_mark<E: TxnEngine>() {
    let mark_key = Key::LayerMeta { name: "layer".into() }.encode().unwrap();

    // A new database: open writes nothing; the mark comes with the first change set.
    let disk = SimVfs::new(10);
    let db = open::<E>(&disk, &cfg()).expect("open");
    assert_eq!(seq(&db), 0);
    db.begin_changeset(meta()).unwrap().commit().unwrap();
    assert_eq!(seq(&db), 0, "an empty change set writes no mark");
    db.create_unit(new_unit("doc", b"x"), meta()).unwrap();
    let mark = db.engine().snapshot().unwrap().get(&mark_key).unwrap().expect("mark");
    assert_eq!(
        LayerMeta::decode(&mark).unwrap(),
        LayerMeta { layer_version: 1, history_policy: HistoryPolicy::Versioned }
    );
    db.close().unwrap();
    open::<E>(&disk, &cfg()).expect("reopen a units database").close().unwrap();

    // Keys but no mark.
    let disk = SimVfs::new(11);
    raw_commit::<E>(&disk, &[(b"\x7fforeign", b"v")]);
    assert!(matches!(open::<E>(&disk, &cfg()), Err(UnitError::NotAUnitDatabase)));

    // A newer layer, a damaged mark.
    let disk = SimVfs::new(12);
    raw_commit::<E>(&disk, &[(&mark_key, &[1, 0, 0, 0, 0, 2, 2])]);
    assert!(matches!(open::<E>(&disk, &cfg()), Err(UnitError::IncompatibleLayer { found: 2, supported: 1 })));
    let disk = SimVfs::new(13);
    raw_commit::<E>(&disk, &[(&mark_key, &[0, 0, 0, 0, 0, 1, 2])]);
    let e = open::<E>(&disk, &cfg()).err().expect("damaged mark");
    assert!(is_corruption(&e, "UNIT-6"), "{e}");
    let disk = SimVfs::new(14);
    raw_commit::<E>(&disk, &[(&mark_key, &[2, 0, 0, 0, 0, 1, 2])]);
    let e = open::<E>(&disk, &cfg()).err().expect("newer envelope");
    assert!(matches!(e, UnitError::Storage(kerno_core::Error::Incompatible(_))), "{e}");
    // Refusals wrote nothing.
    let e = E::open(disk.as_vfs(), Path::new(DIR), &Options::small_for_tests()).unwrap();
    assert_eq!(e.snapshot().unwrap().seq(), 1);
    e.close().unwrap();

    // create_if_missing = false: an empty database reads as empty and is not written.
    let disk = SimVfs::new(15);
    let c = UnitConfig { create_if_missing: false, ..cfg() };
    let db = open::<E>(&disk, &c).expect("open");
    assert_eq!(db.get_unit(id(1)).unwrap(), None);
    assert!(matches!(db.begin_changeset(meta()), Err(UnitError::InvalidArgument(_))));
    assert_eq!(seq(&db), 0);
    db.close().unwrap();

    // History policies other than Versioned belong to E3-05.
    for p in [HistoryPolicy::None, HistoryPolicy::Audit] {
        let c = UnitConfig { default_policy: p, ..cfg() };
        assert!(matches!(open::<E>(&SimVfs::new(16), &c), Err(UnitError::InvalidArgument(_))));
    }
}

// ------------------------------------------------------------ corruption

/// A key, what to store there (`None` deletes it) and the rule expected.
type Damage<'a> = (&'a [u8], Option<Vec<u8>>, &'a str);

fn corruption_detected<E: TxnEngine>() {
    let disk = SimVfs::new(17);
    let db = open::<E>(&disk, &cfg()).expect("open");
    let (a, _) = db.create_unit(new_unit("doc", b"payload-bytes"), meta()).unwrap();
    let rp = Key::RevPayload { unit: a, rev: RevNo(1) }.encode().unwrap();
    let rm = Key::RevMeta { unit: a, rev: RevNo(1) }.encode().unwrap();
    let hk = Key::UnitHead { unit: a }.encode().unwrap();
    let snap = db.engine().snapshot().unwrap();
    let (good_payload, good_meta, good_head) =
        (snap.get(&rp).unwrap().unwrap(), snap.get(&rm).unwrap().unwrap(), snap.get(&hk).unwrap().unwrap());
    drop(snap);
    let put = |k: &[u8], v: Option<&[u8]>| {
        let mut b = WriteBatch::new();
        match v {
            Some(v) => b.put(k.to_vec(), v.to_vec()),
            None => b.delete(k.to_vec()),
        };
        db.engine().commit(&b).unwrap();
    };
    let mut flipped = good_payload.clone();
    flipped[0] ^= 1;
    let mut head_bad = good_head.clone();
    head_bad[1] = 1; // reserved_flags
    let damages: Vec<Damage<'_>> = vec![
        (&rp, Some(flipped), "UNIT-28"),
        (&rp, Some(good_payload[..3].to_vec()), "UNIT-27"),
        (&rp, None, "UNIT-26"),
        (&rm, None, "REV_META is missing"),
        (&hk, Some(head_bad), "UNIT-8"),
    ];
    for (k, v, rule) in damages {
        put(k, v.as_deref());
        let e = db.get_unit(a).unwrap_err();
        assert!(is_corruption(&e, rule), "{rule}: {e}");
        // Writes over the damaged unit refuse too, without effect.
        let before = seq(&db);
        let mut cs = db.begin_changeset(meta()).unwrap();
        let e = cs.update(a, upd(b"new"), None).unwrap_err();
        assert!(is_corruption(&e, rule), "{rule}: {e}");
        assert!(cs.commit().unwrap().revisions.is_empty());
        assert_eq!(seq(&db), before);
        put(&rp, Some(&good_payload));
        put(&rm, Some(&good_meta));
        put(&hk, Some(&good_head));
        assert_eq!(db.get_unit(a).unwrap().unwrap().payload.as_deref(), Some(&b"payload-bytes"[..]));
    }
    // A head that disagrees with its REV_META (another revision's hash).
    let mut other = UnitHead::decode(&good_head).unwrap();
    other.content_hash = ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, b"other");
    put(&hk, Some(&other.encode().unwrap()));
    assert!(is_corruption(&db.get_unit(a).unwrap_err(), "disagrees"));
    db.close().unwrap();
}

// ------------------------------------------------------------ engines

/// `Emulated<LogEngine>` whose transactions report a read sequence one too
/// high, so the layer's prediction misses the engine's real sequence.
struct Skew(Emulated<LogEngine>);
struct SkewTxn<'a>(EmulatedTxn<'a, LogEngine>);

impl Engine for Skew {
    type Snapshot = <Emulated<LogEngine> as Engine>::Snapshot;
    fn name() -> &'static str {
        "skew"
    }
    fn open(vfs: Arc<dyn Vfs>, dir: &Path, opts: &Options) -> kerno_core::Result<Self> {
        Ok(Skew(Emulated::open(vfs, dir, opts)?))
    }
    fn commit(&self, batch: &WriteBatch) -> kerno_core::Result<u64> {
        self.0.commit(batch)
    }
    fn snapshot(&self) -> kerno_core::Result<Self::Snapshot> {
        self.0.snapshot()
    }
    fn close(self) -> kerno_core::Result<()> {
        self.0.close()
    }
}

impl TxnEngine for Skew {
    type Txn<'a> = SkewTxn<'a>;
    fn begin_write(&self) -> kerno_core::Result<SkewTxn<'_>> {
        Ok(SkewTxn(self.0.begin_write()?))
    }
}

impl Txn for SkewTxn<'_> {
    fn read_seq(&self) -> u64 {
        self.0.read_seq() + 1
    }
    fn get(&self, key: &[u8]) -> kerno_core::Result<Option<Vec<u8>>> {
        self.0.get(key)
    }
    fn scan(&self, s: Bound<&[u8]>, e: Bound<&[u8]>, limit: usize) -> kerno_core::Result<Dump> {
        self.0.scan(s, e, limit)
    }
    fn put(&mut self, key: &[u8], value: &[u8]) -> kerno_core::Result<()> {
        self.0.put(key, value)
    }
    fn delete(&mut self, key: &[u8]) -> kerno_core::Result<()> {
        self.0.delete(key)
    }
    fn delete_range(&mut self, s: &[u8], e: &[u8]) -> kerno_core::Result<()> {
        self.0.delete_range(s, e)
    }
    fn commit(self) -> kerno_core::Result<u64> {
        self.0.commit()
    }
}

#[test]
fn seq_mismatch_poisons_the_handle() {
    let db = open::<Skew>(&SimVfs::new(18), &cfg()).expect("open");
    let e = db.create_unit(new_unit("doc", b"x"), meta()).unwrap_err();
    assert!(matches!(e, UnitError::Storage(kerno_core::Error::Poisoned(_))), "{e}");
    assert!(matches!(db.begin_changeset(meta()), Err(UnitError::Storage(kerno_core::Error::Poisoned(_)))));
    assert!(matches!(db.get_unit(id(1)), Err(UnitError::Storage(kerno_core::Error::Poisoned(_)))));
}

/// The engine's own batch limit, hit in the middle of an operation, breaks
/// the change set: nothing of it is ever committed.
#[test]
fn kerno_batch_limit_breaks_the_changeset() {
    use kerno_storage::{Config, Limits};
    let disk = SimVfs::new(19);
    let mut config = Config::from(&Options::small_for_tests());
    config.limits = Limits { max_batch_ops: 6, ..Limits::default() };
    let engine = KernoDb::open(disk.as_vfs(), Path::new(DIR), &config).expect("engine");
    let db = UnitDb::from_engine(engine, &cfg()).expect("open");
    // Mark + head + REV_META + REV_PAYLOAD + member + CHANGESET = 6: at the limit.
    db.create_unit(new_unit("doc", b"1"), meta()).expect("exactly at the batch limit");
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.create(new_unit("doc", b"2")).unwrap();
    let e = cs.create(new_unit("doc", b"3")).unwrap_err();
    assert!(matches!(e, UnitError::ChangeSetTooLarge { ops: 7, .. }), "{e}");
    assert!(matches!(cs.create(new_unit("doc", b"4")), Err(UnitError::ChangeSetClosed)));
    assert!(matches!(cs.get_unit(id(1)), Err(UnitError::ChangeSetClosed)));
    assert!(matches!(cs.commit(), Err(UnitError::ChangeSetClosed)));
    assert_eq!(seq(&db), 1);
    assert_eq!(raw(&db).iter().filter(|(k, _)| k[0] == 0x10).count(), 1);
    db.create_unit(new_unit("doc", b"5"), meta()).expect("the database keeps working");
    db.close().unwrap();
}

macro_rules! engine_tests {
    ($m:ident, $e:ty) => {
        mod $m {
            use super::*;
            type E = $e;

            #[test]
            fn model_ops() {
                super::model_ops::<E>();
            }
            #[test]
            fn seq_prediction() {
                super::seq_prediction::<E>();
            }
            #[test]
            fn changeset_every_point_power_loss_sync() {
                super::changeset_every_point::<E>(CrashKind::PowerLoss, Durability::Sync);
            }
            #[test]
            fn changeset_every_point_process_kill_buffered() {
                super::changeset_every_point::<E>(CrashKind::ProcessKill, Durability::Buffered);
            }
            #[test]
            fn written_records_decode() {
                super::written_records_decode::<E>();
            }
            #[test]
            fn payload_limits() {
                super::payload_limits::<E>();
            }
            #[test]
            fn changeset_limits() {
                super::changeset_limits::<E>();
            }
            #[test]
            fn field_limits() {
                super::field_limits::<E>();
            }
            #[test]
            fn operation_errors() {
                super::operation_errors::<E>();
            }
            #[test]
            fn already_exists() {
                super::already_exists::<E>();
            }
            #[test]
            fn open_and_mark() {
                super::open_and_mark::<E>();
            }
            #[test]
            fn corruption_detected() {
                super::corruption_detected::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, Emulated<LogEngine>);
engine_tests!(kerno, KernoDb);
