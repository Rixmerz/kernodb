//! E3-04: history, as-of reads and restore of `UnitDb` (etapa3-design.md
//! §3.5, §6.4, §8.1, §11). Every suite runs over `Emulated<LogEngine>` (the
//! trivially correct engine) and over `KernoDb`.
//!
//! * `model_as_of` (IU-6) — random change sets (with restores) compared with
//!   `UnitModel`: `unit_as_of` for every sequence and unit, `history` pages
//!   in both orders, `state_as_of` pages, `change_sets_since`, `compare`.
//! * `restore_keeps_later` (I-22) — restoring leaves every later revision
//!   byte for byte as it was.
//! * `ordering_ignores_clock` (I-26) — a clock that goes backwards changes
//!   nothing in the order of history, as-of reads or change sets.
//! * `snapshot_vs_retention` (partial, IU-8) — an old `UnitSnapshot` keeps
//!   reading its history while new change sets commit. The retention half
//!   belongs to E3-06.
//! * `history_reads_verify` — damaged revisions and change sets are
//!   `Corruption`, never `NotFound`, `Absent` or an empty result.

use std::collections::BTreeSet;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::harness::seeds;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{Key, RevisionMeta};
use kerno_units::payload::PAYLOAD_CODEC_OPAQUE;
use kerno_units::*;

const DIR: &str = "/db";
const T0: Timestamp = Timestamp(1_700_000_000_000_000);

type Dump = Vec<(Vec<u8>, Vec<u8>)>;

fn cfg_with(clock: Arc<FixedClock>) -> UnitConfig {
    let mut c = UnitConfig::with_sources(clock, Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn cfg() -> UnitConfig {
    cfg_with(Arc::new(FixedClock::new(T0)))
}

fn open<E: TxnEngine>(disk: &SimVfs, c: &UnitConfig) -> UnitDb<E> {
    UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), c).expect("open")
}

fn meta() -> ChangeMeta {
    ChangeMeta::default()
}

fn new_unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: b"m".to_vec(), payload: payload.to_vec() }
}

fn upd(payload: &[u8]) -> UnitUpdate {
    UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }
}

fn seq<E: TxnEngine>(db: &UnitDb<E>) -> CommitSeq {
    CommitSeq(db.engine().snapshot().expect("snapshot").seq())
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

fn is_corruption(e: &UnitError, rule: &str) -> bool {
    matches!(e, UnitError::Storage(kerno_core::Error::Corruption(m)) if m.contains(rule))
}

fn all(limit: usize) -> HistoryQuery {
    HistoryQuery { with_payload: true, ..HistoryQuery::oldest(limit) }
}

/// A revision read from the database agrees with the model's.
fn check_rev(got: &Revision, unit: UnitId, m: &ModelRevision, with_payload: bool, ctx: &str) {
    assert_eq!(got.id, RevisionId { unit, rev: m.rev }, "{ctx}: id");
    assert_eq!(got.kind, m.kind, "{ctx}: kind");
    assert_eq!(got.commit_seq, m.commit_seq, "{ctx}: commit_seq");
    assert_eq!(got.change_set, m.commit_seq, "{ctx}: change_set");
    assert_eq!(got.parent, (m.rev.0 > 1).then(|| RevNo(m.rev.0 - 1)), "{ctx}: parent");
    assert_eq!(got.lifecycle, m.lifecycle, "{ctx}: lifecycle");
    assert_eq!(got.schema_version, m.schema_version, "{ctx}: schema");
    assert_eq!(got.restored_from, m.restored_from, "{ctx}: restored_from");
    assert_eq!(got.metadata, m.metadata, "{ctx}: metadata");
    let bytes = m.payload.as_deref().unwrap_or(&[]);
    assert_eq!(got.content_hash, ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, bytes), "{ctx}: hash");
    assert_eq!(got.payload_len as usize, bytes.len(), "{ctx}: payload_len");
    let want = if with_payload { m.payload.clone() } else { None };
    assert_eq!(got.payload, want, "{ctx}: payload");
}

fn check_at(got: &UnitAt, unit: UnitId, want: &ModelUnitAt<'_>, ctx: &str) {
    match (got, want) {
        (UnitAt::Absent, ModelUnitAt::Absent) => {}
        (UnitAt::Present(r), ModelUnitAt::Present(m)) | (UnitAt::DeletedAt(r), ModelUnitAt::DeletedAt(m)) => {
            check_rev(r, unit, m, true, ctx)
        }
        _ => panic!("{ctx}: got {got:?}, want {want:?}"),
    }
}

/// Every read of E3-04 on `snap` agrees with `model`.
fn compare_reads<E: Engine>(snap: &UnitSnapshot<E>, model: &UnitModel, rng: &mut Rng, ctx: &str) {
    let last = model.last_seq();
    assert_eq!(snap.seq(), last, "{ctx}: sequence");
    let unknown = UnitId([0xEE; 16]);
    let ids: Vec<UnitId> = model.unit_ids().collect();
    // unit_as_of at every sequence, for every unit and an unknown one.
    for s in 0..=last.0 {
        for &id in ids.iter().chain([&unknown]) {
            let got = snap.unit_as_of(id, AsOf::Seq(CommitSeq(s))).expect("unit_as_of");
            check_at(&got, id, &model.unit_as_of(id, CommitSeq(s)), &format!("{ctx}: unit {id} at {s}"));
        }
        // state_as_of, paged with a small limit.
        let want = model.state_as_of(CommitSeq(s));
        let limit = rng.range(1, 4) as usize;
        let mut got: UnitStates = Vec::new();
        loop {
            let page = snap.state_as_of(AsOf::Seq(CommitSeq(s)), got.last().map(|(u, _)| *u), limit).expect("state");
            assert!(page.len() <= limit, "{ctx}: state page over limit");
            let n = page.len();
            got.extend(page);
            if n < limit {
                break;
            }
        }
        assert_eq!(got.len(), want.len(), "{ctx}: state_as_of({s}) size");
        for ((gu, ga), (wu, wa)) in got.iter().zip(&want) {
            assert_eq!(gu, wu, "{ctx}: state_as_of({s}) order");
            check_at(ga, *gu, wa, &format!("{ctx}: state {gu} at {s}"));
        }
    }
    assert!(matches!(snap.unit_as_of(unknown, AsOf::Seq(CommitSeq(last.0 + 1))), Err(UnitError::InvalidArgument(_))));
    // history: whole, then paged in both orders.
    for &id in &ids {
        let h = model.history(id);
        let got = snap.history(id, all(h.len() + 5)).expect("history");
        assert_eq!(got.len(), h.len(), "{ctx}: history length of {id}");
        for (g, m) in got.iter().zip(h) {
            check_rev(g, id, m, true, &format!("{ctx}: history {id}"));
        }
        let limit = rng.range(1, 4) as usize;
        let mut desc: Vec<Revision> = Vec::new();
        loop {
            let q = HistoryQuery { to: desc.last().map(|r| r.id.rev), ..HistoryQuery::latest(limit) };
            let page = snap.history(id, q).expect("history desc");
            let n = page.len();
            desc.extend(page);
            if n < limit {
                break;
            }
        }
        assert_eq!(desc.len(), h.len(), "{ctx}: desc pages of {id}");
        for (g, m) in desc.iter().zip(h.iter().rev()) {
            check_rev(g, id, m, false, &format!("{ctx}: history desc {id}"));
        }
        let mut asc: Vec<Revision> = Vec::new();
        loop {
            let q = HistoryQuery { from: asc.last().map(|r| RevNo(r.id.rev.0 + 1)), ..HistoryQuery::oldest(limit) };
            let page = snap.history(id, q).expect("history asc");
            let n = page.len();
            asc.extend(page);
            if n < limit {
                break;
            }
        }
        assert_eq!(asc.len(), h.len(), "{ctx}: asc pages of {id}");
        // compare two random revisions.
        let a = RevNo(rng.range(1, h.len() as u64));
        let b = RevNo(rng.range(1, h.len() as u64));
        let c = snap.compare(id, a, b).expect("compare");
        let (ma, mb) = (model.revision(id, a).unwrap(), model.revision(id, b).unwrap());
        check_rev(&c.a, id, ma, true, ctx);
        check_rev(&c.b, id, mb, true, ctx);
        assert_eq!(c.same_content, ma.payload == mb.payload, "{ctx}: same_content {a} {b}");
    }
    // change_sets_since from random points, paged.
    for _ in 0..3 {
        let after = CommitSeq(rng.below(last.0 + 1));
        let limit = rng.range(1, 5) as usize;
        let got = snap.change_sets_since(after, limit).expect("change_sets_since");
        let want = model.change_sets_since(after, limit);
        assert_eq!(got.len(), want.len(), "{ctx}: change sets since {after}");
        for (g, w) in got.iter().zip(want) {
            assert_eq!(g.id, w.seq, "{ctx}: change set id");
            assert_eq!(g.purpose, w.purpose);
            let members: Vec<_> = g.members.iter().map(|m| (m.revision.unit, m.revision.rev, m.kind)).collect();
            assert_eq!(members, w.members, "{ctx}: members of {}", w.seq);
            assert_eq!(snap.change_set(g.id).expect("change_set").as_ref(), Some(g));
        }
    }
}

// ------------------------------------------------------------ model_as_of

fn model_as_of<E: TxnEngine>() {
    for seed in seeds(4) {
        let mut rng = Rng::new(seed);
        let disk = SimVfs::new(seed);
        let db = open::<E>(&disk, &cfg());
        let mut model = UnitModel::new();
        let mut kept: Vec<(UnitSnapshot<E>, UnitModel)> = Vec::new();
        for round in 0..30 {
            let ctx = format!("seed {seed} round {round}");
            let mut cs = db.begin_changeset(meta()).expect("begin");
            let mut changes: Vec<ModelChange> = Vec::new();
            let mut touched = BTreeSet::new();
            for _ in 0..rng.range(1, 5) {
                let ids: Vec<UnitId> = model.unit_ids().filter(|u| !touched.contains(u)).collect();
                if ids.is_empty() || rng.chance(1, 5) {
                    let payload = {
                        let n = rng.below(64) as usize;
                        rng.bytes(n)
                    };
                    let id = cs
                        .create(NewUnit {
                            unit_type: "t".into(),
                            schema_version: 1,
                            metadata: vec![],
                            payload: payload.clone(),
                        })
                        .expect("create");
                    touched.insert(id);
                    changes.push(ModelChange {
                        unit: id,
                        kind: RevisionKind::Create,
                        unit_type: "t".into(),
                        schema_version: 1,
                        lifecycle: Lifecycle::Active,
                        restored_from: None,
                        metadata: vec![],
                        payload: Some(payload),
                    });
                    continue;
                }
                let id = ids[rng.below(ids.len() as u64) as usize];
                let h = model.head(id).cloned().expect("head");
                let same = |kind, lifecycle, payload, restored_from| ModelChange {
                    unit: id,
                    kind,
                    unit_type: h.unit_type.clone(),
                    schema_version: h.schema_version,
                    lifecycle,
                    restored_from,
                    metadata: h.metadata.clone(),
                    payload,
                };
                let (r, ch) = match rng.below(6) {
                    0 | 1 => {
                        let payload = {
                            let n = rng.below(64) as usize;
                            rng.bytes(n)
                        };
                        let m = {
                            let n = rng.below(4) as usize;
                            rng.bytes(n)
                        };
                        let r = cs.update(
                            id,
                            UnitUpdate {
                                payload: payload.clone(),
                                schema_version: Some(round),
                                metadata: Some(m.clone()),
                            },
                            None,
                        );
                        let ch = ModelChange {
                            schema_version: round,
                            metadata: m,
                            ..same(RevisionKind::Update, Lifecycle::Active, Some(payload), None)
                        };
                        (r, ch)
                    }
                    2 => (
                        cs.archive(id, None),
                        same(RevisionKind::Archive, Lifecycle::Archived, h.payload.clone(), None),
                    ),
                    3 => (
                        cs.unarchive(id, None),
                        same(RevisionKind::Unarchive, Lifecycle::Active, h.payload.clone(), None),
                    ),
                    4 if rng.chance(1, 3) => {
                        (cs.delete(id, Some(h.rev)), same(RevisionKind::Delete, Lifecycle::Deleted, None, None))
                    }
                    _ => {
                        let from = RevNo(rng.range(1, h.rev.0));
                        let src = model.revision(id, from).cloned().expect("source");
                        let r = cs.restore(id, from, Some(h.rev));
                        let ch = ModelChange {
                            unit: id,
                            kind: RevisionKind::Restore,
                            unit_type: src.unit_type,
                            schema_version: src.schema_version,
                            lifecycle: Lifecycle::Active,
                            restored_from: Some(from),
                            metadata: src.metadata,
                            payload: src.payload.clone(),
                        };
                        let ok = h.lifecycle == Lifecycle::Active && src.payload.is_some();
                        match &r {
                            Ok(_) => assert!(ok, "{ctx}: restore of {from} should fail"),
                            Err(UnitError::Deleted { .. }) => assert_eq!(h.lifecycle, Lifecycle::Deleted, "{ctx}"),
                            Err(UnitError::InvalidArgument(_)) => assert!(!ok, "{ctx}: restore of {from}: {r:?}"),
                            Err(e) => panic!("{ctx}: restore: {e}"),
                        }
                        (r, ch)
                    }
                };
                match r {
                    Ok(rev) => {
                        assert_eq!(rev, RevNo(h.rev.0 + 1), "{ctx}");
                        touched.insert(id);
                        changes.push(ch);
                    }
                    Err(UnitError::InvalidArgument(_) | UnitError::Deleted { .. }) => {}
                    Err(e) => panic!("{ctx}: {e}"),
                }
            }
            let info = cs.commit().expect("commit");
            if !changes.is_empty() {
                model.apply(info.commit_seq, Purpose::Application, changes).expect("model");
            }
            if rng.chance(1, 6) {
                kept.push((db.snapshot().expect("snapshot"), model.clone()));
            }
        }
        compare_reads(&db.snapshot().expect("snapshot"), &model, &mut rng, &format!("seed {seed}"));
        for (i, (snap, m)) in kept.iter().enumerate() {
            compare_reads(snap, m, &mut rng, &format!("seed {seed} kept snapshot {i}"));
        }
        drop(kept);
        db.close().expect("close");
        let db = open::<E>(&disk, &cfg());
        compare_reads(&db.snapshot().expect("snapshot"), &model, &mut rng, &format!("seed {seed} reopened"));
        db.close().expect("close");
    }
}

// ------------------------------------------------------------ restore_keeps_later

fn rev_records<E: TxnEngine>(db: &UnitDb<E>) -> Dump {
    dump(&db.engine().snapshot().unwrap()).into_iter().filter(|(k, _)| k[0] == 0x20 || k[0] == 0x21).collect()
}

fn restore_keeps_later<E: TxnEngine>() {
    let disk = SimVfs::new(41);
    let db = open::<E>(&disk, &cfg());
    let (a, _) = db.create_unit(new_unit(b"v1"), meta()).unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(a, UnitUpdate { payload: b"v2".to_vec(), schema_version: Some(7), metadata: Some(b"m2".to_vec()) }, None)
        .unwrap();
    cs.commit().unwrap();
    for p in [&b"v3"[..], b"v4"] {
        let mut cs = db.begin_changeset(meta()).unwrap();
        cs.update(a, upd(p), None).unwrap();
        cs.commit().unwrap();
    }
    let before = rev_records(&db);
    let snap = db.snapshot().unwrap();
    let old_history = snap.history(a, all(10)).unwrap();

    // Errors leave everything as it was.
    let s0 = seq(&db);
    let try_restore = |from: u64, expected: Option<RevNo>| db.restore(a, RevNo(from), expected, meta()).unwrap_err();
    assert!(matches!(try_restore(5, None), UnitError::RevisionNotFound { rev: RevNo(5), .. }));
    assert!(matches!(try_restore(0, None), UnitError::RevisionNotFound { rev: RevNo(0), .. }));
    assert!(matches!(try_restore(2, Some(RevNo(3))), UnitError::Conflict { found: RevNo(4), .. }));
    assert!(matches!(db.restore(UnitId([0xEE; 16]), RevNo(1), None, meta()).unwrap_err(), UnitError::NotFound { .. }));
    assert_eq!(seq(&db), s0);

    let (rev, info) = db.restore(a, RevNo(2), Some(RevNo(4)), meta()).unwrap();
    assert_eq!(rev, RevNo(5));
    // I-22: every revision before the restore is unchanged, byte for byte.
    let after = rev_records(&db);
    for kv in &before {
        assert!(after.contains(kv), "record {:02x?} changed or vanished", kv.0);
    }
    assert_eq!(after.len(), before.len() + 2, "one REV_META and one REV_PAYLOAD added");
    let r = db.get_revision(a, RevNo(5)).unwrap();
    assert_eq!((r.kind, r.restored_from, r.commit_seq), (RevisionKind::Restore, Some(RevNo(2)), info.commit_seq));
    assert_eq!(r.payload.as_deref(), Some(&b"v2"[..]));
    assert_eq!((r.schema_version, r.metadata.as_slice()), (7, &b"m2"[..]));
    let u = db.get_unit(a).unwrap().unwrap();
    assert_eq!((u.head, u.payload.as_deref(), u.schema_version), (RevNo(5), Some(&b"v2"[..]), 7));
    let h = db.history(a, all(10)).unwrap();
    assert_eq!(&h[..4], &old_history[..], "later revisions are still there");
    assert!(db.compare(a, RevNo(2), RevNo(5)).unwrap().same_content);
    assert!(!db.compare(a, RevNo(4), RevNo(5)).unwrap().same_content);
    // The old snapshot does not see the restore.
    assert_eq!(snap.history(a, all(10)).unwrap(), old_history);
    drop(snap);

    // Restoring a restore and restoring the head are new revisions too.
    assert_eq!(db.restore(a, RevNo(5), None, meta()).unwrap().0, RevNo(6));
    // One revision per unit per change set.
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(a, upd(b"x"), None).unwrap();
    assert!(matches!(cs.restore(a, RevNo(1), None), Err(UnitError::InvalidArgument(_))));
    cs.abort();
    // Archived: refused; a tombstone cannot be a source; deleted: refused.
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.archive(a, None).unwrap();
    cs.commit().unwrap();
    assert!(matches!(try_restore(1, None), UnitError::InvalidArgument(_)));
    let (b, _) = db.create_unit(new_unit(b"b1"), meta()).unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.delete(b, None).unwrap();
    cs.commit().unwrap();
    assert!(matches!(db.restore(b, RevNo(1), None, meta()).unwrap_err(), UnitError::Deleted { .. }));
    let (c, _) = db.create_unit(new_unit(b"c1"), meta()).unwrap();
    assert!(matches!(db.restore(c, RevNo(1), None, meta()), Ok((RevNo(2), _))));
    db.close().unwrap();
}

// ------------------------------------------------------------ ordering_ignores_clock

fn ordering_ignores_clock<E: TxnEngine>() {
    let disk = SimVfs::new(42);
    let clock = Arc::new(FixedClock::new(T0));
    let db = open::<E>(&disk, &cfg_with(clock.clone()));
    // Each change set reads a clock one second earlier than the previous one.
    let tick = |i: i64| clock.set(Timestamp(T0.0 - i * 1_000_000));
    tick(0);
    let (a, c1) = db.create_unit(new_unit(b"a1"), meta()).unwrap();
    let mut seqs = vec![c1.commit_seq];
    tick(1);
    let (b, _) = db.create_unit(new_unit(b"b1"), meta()).unwrap();
    for (i, p) in [&b"a2"[..], b"a3"].into_iter().enumerate() {
        tick(2 + i as i64);
        let mut cs = db.begin_changeset(meta()).unwrap();
        cs.update(a, upd(p), None).unwrap();
        seqs.push(cs.commit().unwrap().commit_seq);
    }
    tick(10);
    seqs.push(db.restore(a, RevNo(1), None, meta()).unwrap().1.commit_seq);
    tick(-5); // and forwards again
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.delete(a, None).unwrap();
    seqs.push(cs.commit().unwrap().commit_seq);

    let h = db.history(a, all(10)).unwrap();
    let got: Vec<CommitSeq> = h.iter().map(|r| r.commit_seq).collect();
    assert_eq!(got, seqs, "history follows commit_seq");
    assert!(h.windows(2).all(|w| w[0].id.rev < w[1].id.rev && w[0].commit_seq < w[1].commit_seq));
    assert!(h[..4].windows(2).all(|w| w[0].created_at > w[1].created_at), "the clock went backwards");
    let payloads = [&b"a1"[..], b"a2", b"a3", b"a1"];
    // As-of by sequence: every point between commits resolves by commit_seq.
    for s in 0..=seq(&db).0 {
        let want = seqs.iter().rposition(|&c| c.0 <= s);
        match (db.unit_as_of(a, AsOf::Seq(CommitSeq(s))).unwrap(), want) {
            (UnitAt::Absent, None) => {}
            (UnitAt::Present(r), Some(i)) if i < 4 => {
                assert_eq!(r.id.rev, RevNo(i as u64 + 1));
                assert_eq!(r.payload.as_deref(), Some(payloads[i]));
            }
            (UnitAt::DeletedAt(r), Some(4)) => assert_eq!(r.id.rev, RevNo(5)),
            (got, want) => panic!("as-of {s}: {got:?}, want index {want:?}"),
        }
    }
    let at = |s: CommitSeq| match db.unit_as_of(a, AsOf::Seq(s)).unwrap() {
        UnitAt::Present(r) => r.payload.unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(at(seqs[0]), b"a1");
    assert_eq!(at(seqs[1]), b"a2");
    assert_eq!(at(seqs[2]), b"a3");
    assert_eq!(at(seqs[3]), b"a1");
    // Change sets come in commit order whatever their timestamps say.
    let cs = db.change_sets_since(CommitSeq(0), 100).unwrap();
    assert!(cs.windows(2).all(|w| w[0].id < w[1].id));
    assert!(cs[..5].windows(2).all(|w| w[0].created_at > w[1].created_at));
    let state = db.state_as_of(AsOf::Seq(seqs[1]), None, 10).unwrap();
    assert_eq!(state.iter().map(|(u, _)| *u).collect::<Vec<_>>(), vec![a, b]);
    db.close().unwrap();
}

// ------------------------------------------------------------ snapshot_vs_retention (partial)

/// IU-8, the part E3-04 can check: an old snapshot keeps reading the history
/// of its sequence while later change sets (updates, restores, deletes)
/// commit. The retention half (pruning under an open snapshot) is E3-06's.
fn snapshot_vs_retention<E: TxnEngine>() {
    let disk = SimVfs::new(43);
    let db = open::<E>(&disk, &cfg());
    let (a, _) = db.create_unit(new_unit(b"a1"), meta()).unwrap();
    let (b, _) = db.create_unit(new_unit(b"b1"), meta()).unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(a, upd(b"a2"), None).unwrap();
    cs.commit().unwrap();
    let snap = db.snapshot().unwrap();
    let s = snap.seq();
    let read = |snap: &UnitSnapshot<E>| {
        (
            snap.history(a, all(10)).unwrap(),
            snap.history(b, all(10)).unwrap(),
            snap.unit_as_of(a, AsOf::Seq(s)).unwrap(),
            snap.state_as_of(AsOf::Seq(s), None, 10).unwrap(),
            snap.change_sets_since(CommitSeq(0), 100).unwrap(),
            snap.get_revision(a, RevNo(2)).unwrap(),
        )
    };
    let before = read(&snap);
    for i in 0..5u8 {
        let mut cs = db.begin_changeset(meta()).unwrap();
        cs.update(a, upd(&[i]), None).unwrap();
        cs.commit().unwrap();
    }
    db.restore(a, RevNo(1), None, meta()).unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.delete(b, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(read(&snap), before, "the old snapshot sees its own history");
    assert!(matches!(snap.get_revision(a, RevNo(3)), Err(UnitError::RevisionNotFound { .. })));
    assert!(matches!(snap.unit_as_of(a, AsOf::Seq(CommitSeq(s.0 + 1))), Err(UnitError::InvalidArgument(_))));
    let now = db.snapshot().unwrap();
    assert_eq!(now.history(a, all(10)).unwrap().len(), 8);
    assert!(matches!(now.unit_as_of(b, AsOf::Seq(now.seq())).unwrap(), UnitAt::DeletedAt(_)));
    assert_eq!(read(&now).0[..2], before.0[..], "the history before s is unchanged");
    drop((snap, now));
    db.close().unwrap();
}

// ------------------------------------------------------------ errors and limits

fn history_errors<E: TxnEngine>() {
    let disk = SimVfs::new(44);
    let db = open::<E>(&disk, &cfg());
    let unknown = UnitId([0xEE; 16]);
    assert!(matches!(db.history(unknown, all(5)), Err(UnitError::NotFound { .. })));
    assert!(matches!(db.get_revision(unknown, RevNo(1)), Err(UnitError::NotFound { .. })));
    assert!(matches!(db.unit_as_of(unknown, AsOf::Seq(CommitSeq(0))), Ok(UnitAt::Absent)));
    assert!(db.change_sets_since(CommitSeq(0), 5).unwrap().is_empty());
    assert!(db.state_as_of(AsOf::Seq(CommitSeq(0)), None, 5).unwrap().is_empty());
    assert_eq!(db.change_set(CommitSeq(0)).unwrap(), None);
    let (a, c) = db.create_unit(new_unit(b"1"), meta()).unwrap();
    for p in [&b"2"[..], b"3", b"4", b"5"] {
        let mut cs = db.begin_changeset(meta()).unwrap();
        cs.update(a, upd(p), None).unwrap();
        cs.commit().unwrap();
    }
    assert!(matches!(db.history(a, all(0)), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(db.state_as_of(AsOf::Seq(c.commit_seq), None, 0), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(db.change_sets_since(CommitSeq(0), 0), Err(UnitError::InvalidArgument(_))));
    assert!(matches!(
        db.history(a, HistoryQuery { from: Some(RevNo(0)), ..all(5) }),
        Err(UnitError::InvalidArgument(_))
    ));
    assert!(matches!(db.get_revision(a, RevNo(0)), Err(UnitError::RevisionNotFound { .. })));
    assert!(matches!(db.get_revision(a, RevNo(6)), Err(UnitError::RevisionNotFound { .. })));
    assert!(matches!(db.compare(a, RevNo(1), RevNo(6)), Err(UnitError::RevisionNotFound { .. })));
    let revs = |q: HistoryQuery| db.history(a, q).unwrap().iter().map(|r| r.id.rev.0).collect::<Vec<_>>();
    assert_eq!(revs(HistoryQuery { from: Some(RevNo(2)), to: Some(RevNo(4)), ..all(10) }), vec![2, 3]);
    assert_eq!(revs(HistoryQuery { from: Some(RevNo(2)), ..HistoryQuery::latest(2) }), vec![5, 4]);
    assert_eq!(revs(HistoryQuery { to: Some(RevNo(3)), ..HistoryQuery::latest(5) }), vec![2, 1]);
    assert_eq!(revs(HistoryQuery { from: Some(RevNo(4)), to: Some(RevNo(9)), ..all(10) }), vec![4, 5]);
    assert!(revs(HistoryQuery { from: Some(RevNo(6)), ..all(10) }).is_empty());
    assert!(revs(HistoryQuery { from: Some(RevNo(3)), to: Some(RevNo(3)), ..all(10) }).is_empty());
    // Metadata-only history reads no payload.
    assert!(db.history(a, HistoryQuery::oldest(10)).unwrap().iter().all(|r| r.payload.is_none()));
    let info = db.change_set(c.commit_seq).unwrap().unwrap();
    assert_eq!(
        info.members,
        vec![ChangeSetMember { revision: RevisionId { unit: a, rev: RevNo(1) }, kind: RevisionKind::Create }]
    );
    db.close().unwrap();
}

// ------------------------------------------------------------ corruption

fn history_reads_verify<E: TxnEngine>() {
    let disk = SimVfs::new(45);
    let db = open::<E>(&disk, &cfg());
    let (a, c1) = db.create_unit(new_unit(b"payload-one"), meta()).unwrap();
    let mut seqs = vec![c1.commit_seq];
    for p in [&b"payload-two"[..], b"payload-three"] {
        let mut cs = db.begin_changeset(meta()).unwrap();
        cs.update(a, upd(p), None).unwrap();
        seqs.push(cs.commit().unwrap().commit_seq);
    }
    let get = |k: &[u8]| db.engine().snapshot().unwrap().get(k).unwrap();
    let put = |k: &[u8], v: Option<&[u8]>| {
        let mut b = WriteBatch::new();
        match v {
            Some(v) => b.put(k.to_vec(), v.to_vec()),
            None => b.delete(k.to_vec()),
        };
        db.engine().commit(&b).unwrap();
    };
    let at = |s: CommitSeq| db.unit_as_of(a, AsOf::Seq(s));

    // A flipped payload byte of revision 1.
    let rp1 = Key::RevPayload { unit: a, rev: RevNo(1) }.encode().unwrap();
    let good = get(&rp1).unwrap();
    let mut bad = good.clone();
    bad[0] ^= 1;
    put(&rp1, Some(&bad));
    assert!(is_corruption(&db.get_revision(a, RevNo(1)).unwrap_err(), "UNIT-28"));
    assert!(is_corruption(&db.history(a, all(10)).unwrap_err(), "UNIT-28"));
    assert_eq!(db.history(a, HistoryQuery::oldest(10)).unwrap().len(), 3, "metadata alone is intact");
    assert!(is_corruption(&db.compare(a, RevNo(1), RevNo(2)).unwrap_err(), "UNIT-28"));
    assert!(is_corruption(&at(seqs[0]).unwrap_err(), "UNIT-28"));
    assert!(is_corruption(&db.state_as_of(AsOf::Seq(seqs[0]), None, 10).unwrap_err(), "UNIT-28"));
    assert!(is_corruption(&db.restore(a, RevNo(1), None, meta()).unwrap_err(), "UNIT-28"));
    put(&rp1, None);
    assert!(is_corruption(&db.get_revision(a, RevNo(1)).unwrap_err(), "UNIT-26"));
    put(&rp1, Some(&good));

    // A missing REV_META in the middle of the chain.
    let rm2 = Key::RevMeta { unit: a, rev: RevNo(2) }.encode().unwrap();
    let good_meta = get(&rm2).unwrap();
    put(&rm2, None);
    assert!(is_corruption(&db.get_revision(a, RevNo(2)).unwrap_err(), "missing"));
    assert!(is_corruption(&db.history(a, HistoryQuery::oldest(10)).unwrap_err(), "missing"));
    assert!(is_corruption(&db.history(a, HistoryQuery::latest(10)).unwrap_err(), "missing"));
    assert!(is_corruption(&at(seqs[1]).unwrap_err(), "missing"), "never Absent");

    // A chain whose commit_seq does not grow (IU-3).
    let mut m = RevisionMeta::decode(RevNo(2), &good_meta).unwrap();
    m.commit_seq = CommitSeq(seqs[2].0 + 1);
    put(&rm2, Some(&m.encode(RevNo(2)).unwrap()));
    assert!(is_corruption(&db.history(a, HistoryQuery::oldest(10)).unwrap_err(), "IU-3"));
    assert!(is_corruption(&at(seqs[1]).unwrap_err(), "commit_seq"));
    put(&rm2, Some(&good_meta));
    assert!(matches!(at(seqs[1]).unwrap(), UnitAt::Present(r) if r.id.rev == RevNo(2)));

    // A missing CS_MEMBER and a damaged CHANGESET.
    let mk = Key::CsMember { seq: seqs[1], ordinal: 0 }.encode().unwrap();
    let good_member = get(&mk).unwrap();
    put(&mk, None);
    assert!(is_corruption(&db.change_sets_since(CommitSeq(0), 10).unwrap_err(), "member_count"));
    assert!(is_corruption(&db.change_set(seqs[1]).unwrap_err(), "member_count"));
    put(&mk, Some(&good_member));
    let ck = Key::ChangeSet { seq: seqs[1] }.encode().unwrap();
    put(&ck, Some(&[0x01]));
    assert!(matches!(db.change_set(seqs[1]).unwrap_err(), UnitError::Storage(kerno_core::Error::Corruption(_))));
    db.close().unwrap();
}

macro_rules! engine_tests {
    ($m:ident, $e:ty) => {
        mod $m {
            use super::*;
            type E = $e;

            #[test]
            fn model_as_of() {
                super::model_as_of::<E>();
            }
            #[test]
            fn restore_keeps_later() {
                super::restore_keeps_later::<E>();
            }
            #[test]
            fn ordering_ignores_clock() {
                super::ordering_ignores_clock::<E>();
            }
            #[test]
            fn snapshot_vs_retention() {
                super::snapshot_vs_retention::<E>();
            }
            #[test]
            fn history_errors() {
                super::history_errors::<E>();
            }
            #[test]
            fn history_reads_verify() {
                super::history_reads_verify::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, Emulated<LogEngine>);
engine_tests!(kerno, KernoDb);
