//! History policies (`etapa3-design.md` §3.4, E3-05) on both engines:
//!
//! * `policy_matrix`: every policy × every read, history and restore
//!   operation, with the expected answer written out by hand, the storage
//!   left behind (IU-9), and policy changes over time (none/audit →
//!   versioned and back), then a reopen.
//! * `policy_model`: random workloads with random `set_policy` calls over
//!   several unit types, every policy-aware read compared with
//!   [`UnitModel`] and the stored revisions compared with what the model
//!   says each policy keeps (IU-9). E3-06 adds random `run_retention`
//!   rounds (random rules per type, sometimes a small budget run to
//!   completion) checked against [`UnitModel::retain`]: retention under
//!   `Audit` drops the payloads a `Versioned` period left, and under `None`
//!   everything below the head.
//!
//! A policy error is always `HistoryUnavailable`, never an empty result,
//! `NotFound` or `Absent` in its place.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::harness::seeds;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::Key;
use kerno_units::payload::PAYLOAD_CODEC_OPAQUE;
use kerno_units::*;

const DIR: &str = "/db";
const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const POLICIES: [HistoryPolicy; 3] = [HistoryPolicy::None, HistoryPolicy::Audit, HistoryPolicy::Versioned];

fn cfg(default_policy: HistoryPolicy) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c.default_policy = default_policy;
    c
}

fn open<E: TxnEngine>(disk: &SimVfs, c: &UnitConfig) -> UnitDb<E> {
    UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), c).expect("open")
}

fn meta() -> ChangeMeta {
    ChangeMeta::default()
}

fn unit(t: &str, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: 1, metadata: b"m".to_vec(), payload: payload.to_vec() }
}

fn upd(payload: &[u8]) -> UnitUpdate {
    UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }
}

fn hu(p: HistoryPolicy) -> UnitError {
    UnitError::HistoryUnavailable { policy: p }
}

/// Same error: `InvalidArgument` by variant only (messages differ), the rest
/// by every field.
fn same_err(got: &UnitError, want: &UnitError) -> bool {
    match (got, want) {
        (UnitError::InvalidArgument(_), UnitError::InvalidArgument(_)) => true,
        _ => format!("{got:?}") == format!("{want:?}"),
    }
}

#[track_caller]
fn expect_err<T: std::fmt::Debug>(r: Result<T, UnitError>, want: UnitError, ctx: &str) {
    match r {
        Err(e) if same_err(&e, &want) => {}
        other => panic!("{ctx}: got {other:?}, want Err({want:?})"),
    }
}

/// Revision numbers of the stored `REV_META` and `REV_PAYLOAD` of each unit.
type Stored = BTreeMap<UnitId, (BTreeSet<u64>, BTreeSet<u64>)>;

fn stored<E: TxnEngine>(db: &UnitDb<E>) -> Stored {
    let snap = db.engine().snapshot().expect("snapshot");
    let mut out = Stored::new();
    let mut from: Option<Vec<u8>> = None;
    loop {
        let start = match &from {
            Some(k) => Bound::Excluded(k.as_slice()),
            None => Bound::Unbounded,
        };
        let page = snap.scan(start, Bound::Unbounded, 256).expect("scan");
        let n = page.len();
        for (k, _) in &page {
            match Key::decode(k).expect("key") {
                Key::RevMeta { unit, rev } => {
                    out.entry(unit).or_default().0.insert(rev.0);
                }
                Key::RevPayload { unit, rev } => {
                    out.entry(unit).or_default().1.insert(rev.0);
                }
                _ => {}
            }
        }
        from = page.last().map(|(k, _)| k.clone());
        if n < 256 {
            return out;
        }
    }
}

fn set(revs: impl IntoIterator<Item = u64>) -> BTreeSet<u64> {
    revs.into_iter().collect()
}

// ------------------------------------------------------------ policy_matrix

/// One unit `u` (create v1, update v2, archive, unarchive: revisions 1..=4)
/// and one unit `d` (create, delete) of type `doc` under policy `p` set with
/// `set_policy` before the first write; every operation checked.
fn policy_matrix_one<E: TxnEngine>(p: HistoryPolicy, seed: u64) {
    let ctx = format!("{p:?}");
    let disk = SimVfs::new(seed);
    // The default is a different policy: the per-type record must win.
    let other = if p == HistoryPolicy::Versioned { HistoryPolicy::None } else { HistoryPolicy::Versioned };
    let c = cfg(other);
    let db = open::<E>(&disk, &c);
    let set_info = db.set_policy("doc", p, meta()).expect("set_policy");
    assert!(set_info.revisions.is_empty());
    assert_eq!(db.policy("doc").unwrap(), p, "{ctx}");
    assert_eq!(db.policy("other").unwrap(), other, "{ctx}: default from the mark");
    let (u, c1) = db.create_unit(unit("doc", b"v1"), meta()).unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(u, upd(b"v2"), Some(RevNo(1))).unwrap();
    let c2 = cs.commit().unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.archive(u, None).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.unarchive(u, None).unwrap();
    let c4 = cs.commit().unwrap();
    let (d, d1) = db.create_unit(unit("doc", b"d1"), meta()).unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.delete(d, None).unwrap();
    let d2 = cs.commit().unwrap();
    let versioned = p == HistoryPolicy::Versioned;

    // get_unit: always the current state.
    let head = db.get_unit(u).unwrap().unwrap();
    assert_eq!((head.head, head.payload.as_deref(), head.lifecycle), (RevNo(4), Some(&b"v2"[..]), Lifecycle::Active));
    assert_eq!(head.oldest, if p == HistoryPolicy::None { RevNo(4) } else { RevNo(1) }, "{ctx}: oldest");
    let dead = db.get_unit(d).unwrap().unwrap();
    assert_eq!((dead.lifecycle, dead.payload), (Lifecycle::Deleted, None));

    // get_revision / compare.
    assert_eq!(db.get_revision(u, RevNo(4)).unwrap().payload.as_deref(), Some(&b"v2"[..]), "{ctx}");
    expect_err(db.get_revision(u, RevNo(5)), UnitError::RevisionNotFound { unit: u, rev: RevNo(5) }, &ctx);
    expect_err(db.get_revision(u, RevNo(0)), UnitError::RevisionNotFound { unit: u, rev: RevNo(0) }, &ctx);
    expect_err(db.get_revision(UnitId([0xEE; 16]), RevNo(1)), UnitError::NotFound { unit: UnitId([0xEE; 16]) }, &ctx);
    for r in 1..4 {
        let got = db.get_revision(u, RevNo(r));
        if versioned {
            assert_eq!(got.unwrap().id.rev, RevNo(r));
        } else {
            expect_err(got, hu(p), &format!("{ctx}: get_revision {r}"));
        }
    }
    let cmp = db.compare(u, RevNo(1), RevNo(4));
    if versioned {
        assert!(!cmp.unwrap().same_content);
    } else {
        expect_err(cmp, hu(p), &format!("{ctx}: compare"));
    }
    assert!(db.compare(u, RevNo(4), RevNo(4)).unwrap().same_content);
    let tomb = db.get_revision(d, RevNo(2)).unwrap();
    assert_eq!((tomb.kind, tomb.payload), (RevisionKind::Delete, None));

    // history: metadata only, then with payloads.
    let meta_q = HistoryQuery::oldest(10);
    let pay_q = HistoryQuery { with_payload: true, ..meta_q };
    match p {
        HistoryPolicy::None => {
            expect_err(db.history(u, meta_q), hu(p), &ctx);
            expect_err(db.history(u, HistoryQuery::latest(1)), hu(p), &ctx);
            expect_err(db.history(u, pay_q), hu(p), &ctx);
        }
        HistoryPolicy::Audit => {
            let h = db.history(u, meta_q).unwrap();
            assert_eq!(h.len(), 4, "{ctx}: audit keeps every REV_META");
            assert_eq!(h[0].content_hash, ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, b"v1"), "{ctx}: hash kept");
            assert_eq!(h[0].payload_len, 2, "{ctx}: length kept");
            assert!(h.iter().all(|r| r.payload.is_none()));
            expect_err(db.history(u, pay_q), hu(p), &ctx);
        }
        HistoryPolicy::Versioned => {
            let h = db.history(u, pay_q).unwrap();
            assert_eq!(h[0].payload.as_deref(), Some(&b"v1"[..]));
            assert_eq!(h.len(), 4);
        }
    }
    expect_err(db.history(u, HistoryQuery::oldest(0)), UnitError::InvalidArgument(String::new()), &ctx);

    // unit_as_of / state_as_of.
    let last = d2.commit_seq;
    assert_eq!(db.unit_as_of(u, AsOf::Seq(CommitSeq(0))).unwrap(), UnitAt::Absent, "{ctx}: before creation");
    assert_eq!(db.unit_as_of(d, AsOf::Seq(c4.commit_seq)).unwrap(), UnitAt::Absent, "{ctx}: before creation");
    match db.unit_as_of(u, AsOf::Seq(last)).unwrap() {
        UnitAt::Present(r) => assert_eq!((r.id.rev, r.payload.as_deref()), (RevNo(4), Some(&b"v2"[..]))),
        other => panic!("{ctx}: {other:?}"),
    }
    assert!(matches!(db.unit_as_of(u, AsOf::Seq(c4.commit_seq)).unwrap(), UnitAt::Present(_)));
    assert!(matches!(db.unit_as_of(d, AsOf::Seq(last)).unwrap(), UnitAt::DeletedAt(_)));
    for (s, id) in [(c1.commit_seq, u), (c2.commit_seq, u), (d1.commit_seq, d)] {
        let got = db.unit_as_of(id, AsOf::Seq(s));
        if versioned {
            assert!(matches!(got.unwrap(), UnitAt::Present(_)), "{ctx}");
        } else {
            expect_err(got, hu(p), &format!("{ctx}: unit_as_of {s}"));
        }
        let state = db.state_as_of(AsOf::Seq(s), None, 10);
        if versioned {
            assert!(!state.unwrap().is_empty());
        } else {
            expect_err(state, hu(p), &format!("{ctx}: state_as_of {s}"));
        }
    }
    assert_eq!(db.state_as_of(AsOf::Seq(last), None, 10).unwrap().len(), 2, "{ctx}: current state");

    // change sets: every one is kept, the policy one as Maintenance.
    let sets = db.change_sets_since(CommitSeq(0), 100).unwrap();
    assert_eq!(sets.len(), 7, "{ctx}");
    assert_eq!((sets[0].id, sets[0].purpose), (set_info.commit_seq, Purpose::Maintenance));
    assert!(sets[0].members.is_empty());
    assert_eq!(db.change_set(c1.commit_seq).unwrap().unwrap().members.len(), 1);

    // IU-9: what is stored.
    let st = stored(&db);
    let (um, up) = &st[&u];
    let (dm, dp) = &st[&d];
    match p {
        HistoryPolicy::None => {
            assert_eq!((um, up), (&set([4]), &set([4])), "{ctx}: none keeps only the head");
            assert_eq!((dm, dp), (&set([2]), &set([])), "{ctx}");
        }
        HistoryPolicy::Audit => {
            assert_eq!((um, up), (&set(1..=4), &set([4])), "{ctx}: audit keeps no old payload");
            assert_eq!((dm, dp), (&set([1, 2]), &set([])), "{ctx}");
        }
        _ => {
            assert_eq!((um, up), (&set(1..=4), &set(1..=4)), "{ctx}");
            assert_eq!((dm, dp), (&set([1, 2]), &set([1])), "{ctx}");
        }
    }

    // restore: the head is always restorable; an older revision only under
    // Versioned; error order is kept (RevisionNotFound, Deleted first).
    let mut cs = db.begin_changeset(meta()).unwrap();
    expect_err(cs.restore(u, RevNo(9), None), UnitError::RevisionNotFound { unit: u, rev: RevNo(9) }, &ctx);
    expect_err(cs.restore(d, RevNo(1), None), UnitError::Deleted { unit: d }, &ctx);
    let r = cs.restore(u, RevNo(1), None);
    if versioned {
        assert_eq!(r.unwrap(), RevNo(5));
    } else {
        expect_err(r, hu(p), &format!("{ctx}: restore 1"));
        assert_eq!(cs.restore(u, RevNo(4), Some(RevNo(4))).unwrap(), RevNo(5), "{ctx}: restore head");
    }
    cs.commit().unwrap();
    let now = db.get_unit(u).unwrap().unwrap();
    assert_eq!(now.payload.as_deref(), Some(if versioned { &b"v1"[..] } else { &b"v2"[..] }), "{ctx}");
    let st = stored(&db);
    match p {
        HistoryPolicy::None => assert_eq!(st[&u], (set([5]), set([5])), "{ctx}"),
        HistoryPolicy::Audit => assert_eq!(st[&u], (set(1..=5), set([5])), "{ctx}"),
        _ => assert_eq!(st[&u], (set(1..=5), set(1..=5)), "{ctx}"),
    }

    // Policy change over time: to Versioned. Nothing is rewritten; what the
    // old policy dropped stays unavailable, with the right error.
    db.set_policy("doc", HistoryPolicy::Versioned, meta()).unwrap();
    assert_eq!(stored(&db), st, "{ctx}: set_policy rewrites nothing");
    match p {
        HistoryPolicy::None => {
            expect_err(db.get_revision(u, RevNo(1)), UnitError::Pruned { unit: u, oldest: RevNo(5) }, &ctx);
            assert_eq!(db.history(u, HistoryQuery::oldest(10)).unwrap().len(), 1, "{ctx}");
            assert!(matches!(db.unit_as_of(u, AsOf::Seq(c1.commit_seq)).unwrap(), UnitAt::Pruned { .. }));
            expect_err(
                db.history(u, HistoryQuery { from: Some(RevNo(1)), ..HistoryQuery::oldest(10) }),
                UnitError::Pruned { unit: u, oldest: RevNo(5) },
                &ctx,
            );
        }
        HistoryPolicy::Audit => {
            expect_err(db.get_revision(u, RevNo(1)), hu(HistoryPolicy::Audit), &ctx);
            assert_eq!(db.history(u, HistoryQuery::oldest(10)).unwrap().len(), 5, "{ctx}");
            expect_err(db.history(u, pay_q), hu(HistoryPolicy::Audit), &ctx);
            expect_err(db.unit_as_of(u, AsOf::Seq(c1.commit_seq)), hu(HistoryPolicy::Audit), &ctx);
            let mut cs = db.begin_changeset(meta()).unwrap();
            expect_err(cs.restore(u, RevNo(2), None), hu(HistoryPolicy::Audit), &ctx);
            cs.abort();
            // The tombstone of `d` has no payload by nature: still readable.
            assert_eq!(db.get_revision(d, RevNo(2)).unwrap().payload, None);
        }
        _ => {}
    }
    // New revisions under Versioned are all kept.
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(u, upd(b"v6"), None).unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(u, upd(b"v7"), None).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.get_revision(u, RevNo(6)).unwrap().payload.as_deref(), Some(&b"v6"[..]), "{ctx}");
    let st = stored(&db);
    assert!(st[&u].1.is_superset(&set([5, 6, 7])), "{ctx}");

    // And back to None: one write drops every older revision (IU-9).
    db.set_policy("doc", HistoryPolicy::None, meta()).unwrap();
    expect_err(db.get_revision(u, RevNo(6)), hu(HistoryPolicy::None), &ctx);
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(u, upd(b"v8"), None).unwrap();
    cs.commit().unwrap();
    assert_eq!(stored(&db)[&u], (set([8]), set([8])), "{ctx}: none after versioned");
    assert_eq!(db.get_unit(u).unwrap().unwrap().oldest, RevNo(8));

    // Policies persist across a reopen.
    db.close().unwrap();
    let db = open::<E>(&disk, &c);
    assert_eq!(db.policy("doc").unwrap(), HistoryPolicy::None);
    assert_eq!(db.policy("other").unwrap(), other);
    assert_eq!(db.get_unit(u).unwrap().unwrap().payload.as_deref(), Some(&b"v8"[..]));
    db.close().unwrap();
}

fn policy_matrix<E: TxnEngine>() {
    for (i, p) in POLICIES.into_iter().enumerate() {
        policy_matrix_one::<E>(p, 70 + i as u64);
    }
    // The default policy from the configuration, without set_policy.
    for p in POLICIES {
        let disk = SimVfs::new(80);
        let db = open::<E>(&disk, &cfg(p));
        let (u, _) = db.create_unit(unit("doc", b"a"), meta()).unwrap();
        let mut cs = db.begin_changeset(meta()).unwrap();
        cs.update(u, upd(b"b"), None).unwrap();
        cs.commit().unwrap();
        let r = db.get_revision(u, RevNo(1));
        if p == HistoryPolicy::Versioned {
            r.unwrap();
        } else {
            expect_err(r, hu(p), &format!("default {p:?}"));
        }
        db.close().unwrap();
        // Reopening with another default does not change the mark's.
        let other = if p == HistoryPolicy::None { HistoryPolicy::Audit } else { HistoryPolicy::None };
        let db = open::<E>(&disk, &cfg(other));
        assert_eq!(db.policy("doc").unwrap(), p);
        db.close().unwrap();
    }
    // set_policy arguments.
    let disk = SimVfs::new(81);
    let db = open::<E>(&disk, &cfg(HistoryPolicy::Versioned));
    let long = "t".repeat(MAX_POLICY_UNIT_TYPE_LEN);
    db.set_policy(&long, HistoryPolicy::Audit, meta()).expect("57 bytes");
    assert_eq!(db.policy(&long).unwrap(), HistoryPolicy::Audit);
    let longer = "t".repeat(MAX_POLICY_UNIT_TYPE_LEN + 1);
    let before = db.engine().snapshot().unwrap().seq();
    for (t, m) in [
        (longer.as_str(), meta()),
        ("", meta()),
        ("doc", ChangeMeta { purpose: Purpose::Retention, ..meta() }),
        ("doc", ChangeMeta { actor: Some(Vec::new()), ..meta() }),
    ] {
        expect_err(db.set_policy(t, HistoryPolicy::None, m), UnitError::InvalidArgument(String::new()), t);
    }
    assert_eq!(db.engine().snapshot().unwrap().seq(), before, "refused set_policy writes nothing");
    assert_eq!(db.policy(&longer).unwrap(), HistoryPolicy::Versioned, "a longer type follows the default");
    let info = db
        .set_policy(
            "doc",
            HistoryPolicy::Audit,
            ChangeMeta { purpose: Purpose::Maintenance, reason: Some("r".into()), ..meta() },
        )
        .unwrap();
    let cs = db.change_set(info.change_set).unwrap().unwrap();
    assert_eq!((cs.purpose, cs.reason.as_deref()), (Purpose::Maintenance, Some("r")));
    db.close().unwrap();
}

// ------------------------------------------------------------ policy_model

const TYPES: [&str; 3] = ["t0", "t1", "t2"];

fn check_rev(got: &Revision, m: &ModelRevision, with_payload: bool, ctx: &str) {
    assert_eq!(got.id.rev, m.rev, "{ctx}: rev");
    assert_eq!(got.kind, m.kind, "{ctx}: kind");
    assert_eq!(got.commit_seq, m.commit_seq, "{ctx}: commit_seq");
    assert_eq!(got.lifecycle, m.lifecycle, "{ctx}: lifecycle");
    assert_eq!(got.restored_from, m.restored_from, "{ctx}: restored_from");
    let bytes = m.payload.as_deref().unwrap_or(&[]);
    assert_eq!(got.content_hash, ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, bytes), "{ctx}: hash");
    assert_eq!(got.payload, if with_payload { m.payload.clone() } else { None }, "{ctx}: payload");
}

fn check_at(got: &UnitAt, want: &Option<ModelUnitAt<'_>>, ctx: &str) {
    match (got, want) {
        (UnitAt::Absent, Some(ModelUnitAt::Absent)) | (UnitAt::Pruned { .. }, None) => {}
        (UnitAt::Present(r), Some(ModelUnitAt::Present(m)))
        | (UnitAt::DeletedAt(r), Some(ModelUnitAt::DeletedAt(m))) => check_rev(r, m, true, ctx),
        _ => panic!("{ctx}: got {got:?}, want {want:?}"),
    }
}

/// `HistoryUnavailable` answers on which database and model agreed.
static UNAVAILABLE: AtomicUsize = AtomicUsize::new(0);

#[track_caller]
fn agree<T: std::fmt::Debug, W: std::fmt::Debug>(
    got: Result<T, UnitError>,
    want: Result<W, UnitError>,
    ctx: &str,
) -> Option<(T, W)> {
    match (got, want) {
        (Ok(g), Ok(w)) => Some((g, w)),
        (Err(g), Err(w)) if same_err(&g, &w) => {
            if matches!(g, UnitError::HistoryUnavailable { .. }) {
                UNAVAILABLE.fetch_add(1, AtomicOrdering::Relaxed);
            }
            None
        }
        (g, w) => panic!("{ctx}: got {g:?}, want {w:?}"),
    }
}

/// Every policy-aware read on `snap` agrees with `model`.
fn compare_reads<E: Engine>(snap: &UnitSnapshot<E>, model: &UnitModel, rng: &mut Rng, ctx: &str) {
    let last = model.last_seq();
    let ids: Vec<UnitId> = model.unit_ids().collect();
    for t in TYPES {
        assert_eq!(snap.policy(t).unwrap(), model.policy(t), "{ctx}: policy {t}");
    }
    for &id in &ids {
        let head = model.head(id).unwrap();
        let u = snap.get_unit(id).unwrap().unwrap();
        assert_eq!((u.head, u.oldest, &u.payload), (head.rev, model.oldest(id), &head.payload), "{ctx}: get_unit");
        for r in 0..=head.rev.0 + 1 {
            let c = format!("{ctx}: get_revision {id} {r}");
            if let Some((g, w)) = agree(snap.get_revision(id, RevNo(r)), model.read_revision(id, RevNo(r)), &c) {
                check_rev(&g, w, true, &c);
            }
        }
        let (a, b) = (RevNo(rng.range(1, head.rev.0)), RevNo(rng.range(1, head.rev.0)));
        let want = model.read_revision(id, a).and_then(|_| model.read_revision(id, b));
        agree(snap.compare(id, a, b), want, &format!("{ctx}: compare {a} {b}"));
        for _ in 0..4 {
            let from = rng.chance(1, 2).then(|| RevNo(rng.below(head.rev.0 + 2)));
            let to = rng.chance(1, 2).then(|| RevNo(rng.below(head.rev.0 + 3)));
            let asc = rng.chance(1, 2);
            let limit = rng.range(1, 5) as usize;
            let with_payload = rng.chance(1, 2);
            let q = HistoryQuery { from, to, order: if asc { Order::Asc } else { Order::Desc }, limit, with_payload };
            let c = format!("{ctx}: history {id} {q:?}");
            let want = model.read_history(id, from, to, asc, limit, with_payload);
            if let Some((mut g, w)) = agree(snap.history(id, q), want, &c) {
                if !asc {
                    g.reverse();
                }
                assert_eq!(g.len(), w.len(), "{c}");
                for (g, w) in g.iter().zip(w) {
                    check_rev(g, w, with_payload, &c);
                }
            }
        }
    }
    for s in 0..=last.0 {
        let at = AsOf::Seq(CommitSeq(s));
        for &id in &ids {
            let c = format!("{ctx}: unit_as_of {id} {s}");
            if let Some((g, w)) = agree(snap.unit_as_of(id, at), model.read_as_of(id, CommitSeq(s)), &c) {
                check_at(&g, &w, &c);
            }
        }
        let limit = rng.range(1, 4) as usize;
        let mut after = None;
        loop {
            let c = format!("{ctx}: state_as_of {s} after {after:?}");
            let Some((g, w)) =
                agree(snap.state_as_of(at, after, limit), model.read_state_as_of(CommitSeq(s), after, limit), &c)
            else {
                break;
            };
            assert_eq!(g.len(), w.len(), "{c}");
            for ((gu, ga), (wu, wa)) in g.iter().zip(&w) {
                assert_eq!(gu, wu, "{c}");
                check_at(ga, wa, &c);
            }
            if g.len() < limit {
                break;
            }
            after = g.last().map(|(u, _)| *u);
        }
    }
    let got = snap.change_sets_since(CommitSeq(0), 10_000).unwrap();
    let want = model.change_sets_since(CommitSeq(0), 10_000);
    assert_eq!(got.len(), want.len(), "{ctx}: change sets");
    for (g, w) in got.iter().zip(want) {
        assert_eq!((g.id, g.purpose), (w.seq, w.purpose), "{ctx}");
        let members: Vec<_> = g.members.iter().map(|m| (m.revision.unit, m.revision.rev, m.kind)).collect();
        assert_eq!(members, w.members, "{ctx}");
    }
}

/// IU-9: the stored `REV_META` are exactly `[oldest, head]` and the stored
/// `REV_PAYLOAD` exactly those revisions whose content the model keeps.
fn check_stored<E: TxnEngine>(db: &UnitDb<E>, model: &UnitModel, ctx: &str) {
    let st = stored(db);
    for id in model.unit_ids() {
        let head = model.head(id).unwrap().rev.0;
        let oldest = model.oldest(id).0;
        let metas = set(oldest..=head);
        let payloads = set(model.history(id)[oldest as usize - 1..]
            .iter()
            .filter(|r| r.content_kept && r.payload.is_some())
            .map(|r| r.rev.0));
        assert_eq!(st.get(&id).cloned().unwrap_or_default(), (metas, payloads), "{ctx}: stored revisions of {id}");
    }
}

fn policy_model<E: TxnEngine>() {
    let before = UNAVAILABLE.load(AtomicOrdering::Relaxed);
    for seed in seeds(4) {
        let mut rng = Rng::new(seed);
        let disk = SimVfs::new(seed);
        let default = POLICIES[rng.below(3) as usize];
        let mut c = cfg(default);
        c.clock = Arc::new(FixedClock::stepping(T0, 1_000_000));
        let db = open::<E>(&disk, &c);
        let mut model = UnitModel::with_default_policy(default);
        let mut kept: Vec<(UnitSnapshot<E>, UnitModel)> = Vec::new();
        for round in 0..30 {
            let ctx = format!("seed {seed} round {round}");
            if rng.chance(1, 4) {
                let t = TYPES[rng.below(3) as usize];
                let p = POLICIES[rng.below(3) as usize];
                let info = db.set_policy(t, p, meta()).expect("set_policy");
                model.set_policy(info.commit_seq, t, p).expect("model set_policy");
            }
            let mut cs = db.begin_changeset(meta()).expect("begin");
            let mut changes: Vec<ModelChange> = Vec::new();
            let mut touched = BTreeSet::new();
            for _ in 0..rng.range(1, 4) {
                let ids: Vec<UnitId> = model.unit_ids().filter(|u| !touched.contains(u)).collect();
                if ids.is_empty() || rng.chance(1, 5) {
                    let t = TYPES[rng.below(3) as usize];
                    let n = rng.below(32) as usize;
                    let payload = rng.bytes(n);
                    let id = cs.create(unit(t, &payload)).expect("create");
                    touched.insert(id);
                    changes.push(ModelChange {
                        unit: id,
                        kind: RevisionKind::Create,
                        unit_type: t.into(),
                        schema_version: 1,
                        lifecycle: Lifecycle::Active,
                        restored_from: None,
                        metadata: b"m".to_vec(),
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
                        let n = rng.below(32) as usize;
                        let payload = rng.bytes(n);
                        let r = cs.update(id, upd(&payload), None);
                        (r, same(RevisionKind::Update, Lifecycle::Active, Some(payload), None))
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
                        (cs.delete(id, None), same(RevisionKind::Delete, Lifecycle::Deleted, None, None))
                    }
                    _ => {
                        let from = RevNo(rng.range(1, h.rev.0));
                        let r = cs.restore(id, from, None);
                        let c = format!("{ctx}: restore {id} {from}");
                        match model.read_restore_source(id, from) {
                            Ok(src) => {
                                let ch = ModelChange {
                                    unit: id,
                                    kind: RevisionKind::Restore,
                                    unit_type: src.unit_type.clone(),
                                    schema_version: src.schema_version,
                                    lifecycle: Lifecycle::Active,
                                    restored_from: Some(from),
                                    metadata: src.metadata.clone(),
                                    payload: src.payload.clone(),
                                };
                                assert!(r.is_ok(), "{c}: {r:?}");
                                (r, ch)
                            }
                            Err(want) => {
                                agree(r, Err::<(), _>(want), &c);
                                continue;
                            }
                        }
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
            check_stored(&db, &model, &ctx);
            if rng.chance(1, 4) {
                retention_round(&db, &mut model, &mut rng, &ctx);
                check_stored(&db, &model, &format!("{ctx} after retention"));
            }
            if rng.chance(1, 6) {
                kept.push((db.snapshot().expect("snapshot"), model.clone()));
            }
        }
        compare_reads(&db.snapshot().unwrap(), &model, &mut rng, &format!("seed {seed}"));
        for (i, (snap, m)) in kept.iter().enumerate() {
            compare_reads(snap, m, &mut rng, &format!("seed {seed} kept snapshot {i}"));
        }
        drop(kept);
        db.close().unwrap();
        let db = open::<E>(&disk, &c);
        compare_reads(&db.snapshot().unwrap(), &model, &mut rng, &format!("seed {seed} reopened"));
        check_stored(&db, &model, &format!("seed {seed} reopened"));
        db.close().unwrap();
    }
    assert!(UNAVAILABLE.load(AtomicOrdering::Relaxed) > before, "the workload never met a policy error");
    for (name, n) in
        [("retention change sets", &RETAINED), ("audit payload drops", &DROPPED), ("pruned revisions", &PRUNED)]
    {
        assert!(n.load(AtomicOrdering::Relaxed) > 0, "the workload produced no {name}");
    }
}

/// Retention change sets, payloads dropped under `Audit` and revisions
/// pruned that the workload produced.
static RETAINED: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicUsize = AtomicUsize::new(0);
static PRUNED: AtomicUsize = AtomicUsize::new(0);

/// A random retention rule.
fn random_rule(rng: &mut Rng, last: CommitSeq) -> RetentionRule {
    match rng.below(4) {
        0 => RetentionRule::KeepAll,
        1 => RetentionRule::KeepLast(rng.range(1, 4)),
        2 => RetentionRule::KeepFor(std::time::Duration::from_secs(rng.below(last.0 + 2))),
        _ => RetentionRule::KeepSince(CommitSeq(rng.below(last.0 + 2))),
    }
}

/// One `run_retention` (with the default budget, or a small one repeated
/// until complete), each call checked against [`UnitModel::retain_within`].
fn retention_round<E: TxnEngine>(db: &UnitDb<E>, model: &mut UnitModel, rng: &mut Rng, ctx: &str) {
    let last = model.last_seq();
    let small = rng.chance(1, 3);
    let rule = |rng: &mut Rng| match random_rule(rng, last) {
        // A small budget runs several change sets, each with its own clock
        // reading, which the test cannot see when one writes nothing.
        RetentionRule::KeepFor(_) if small => RetentionRule::KeepLast(2),
        r => r,
    };
    let mut policy = RetentionPolicy::all(rule(rng));
    for t in TYPES {
        if rng.chance(1, 2) {
            policy = policy.with_type(t, rule(rng));
        }
    }
    let budget = if small {
        RetentionBudget { max_units: rng.range(1, 3) as usize, max_ops: rng.range(4, 9) as usize }
    } else {
        RetentionBudget::default()
    };
    for call in 0.. {
        assert!(call < 1000, "{ctx}: retention never completes");
        let c = format!("{ctx}: retention call {call} {policy:?} {budget:?}");
        let r = db.run_retention(&policy, budget).unwrap_or_else(|e| panic!("{c}: {e}"));
        let snap = db.snapshot().unwrap();
        let created = |s: CommitSeq| snap.change_set(s).unwrap().expect("change set").created_at;
        match &r.change_set {
            Some(info) => {
                let now = snap.change_set(info.commit_seq).unwrap().unwrap().created_at;
                let (want, complete) =
                    model.retain_within(info.commit_seq, &policy, budget, now, created).expect("model retain");
                let want = want.expect("the model retains too");
                assert_eq!(r.units_retained as usize, want.members.len(), "{c}");
                assert_eq!(r.complete, complete, "{c}");
                let got = snap.change_set(info.commit_seq).unwrap().unwrap();
                assert_eq!(got.purpose, Purpose::Retention, "{c}");
                RETAINED.fetch_add(1, AtomicOrdering::Relaxed);
                DROPPED.fetch_add(r.payloads_dropped as usize, AtomicOrdering::Relaxed);
                PRUNED.fetch_add(r.revisions_pruned as usize, AtomicOrdering::Relaxed);
            }
            None => {
                assert!(r.complete, "{c}: an empty call is complete");
                if policy.default != RetentionRule::KeepAll
                    && !matches!(policy.default, RetentionRule::KeepFor(_))
                    && !policy.by_type.values().any(|r| matches!(r, RetentionRule::KeepFor(_)))
                {
                    let mut m = model.clone();
                    let (want, _) = m.retain_within(CommitSeq(u64::MAX), &policy, budget, T0, created).unwrap();
                    assert!(want.is_none(), "{c}: the model would retain");
                }
            }
        }
        if r.complete {
            break;
        }
    }
}

macro_rules! engine_tests {
    ($m:ident, $e:ty) => {
        mod $m {
            use super::*;
            type E = $e;

            #[test]
            fn policy_matrix() {
                super::policy_matrix::<E>();
            }
            #[test]
            fn policy_model() {
                super::policy_model::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, Emulated<LogEngine>);
engine_tests!(kerno, KernoDb);
