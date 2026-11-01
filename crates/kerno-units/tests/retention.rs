//! E3-06: retention and GC (`etapa3-design.md` §5, §7 IU-7/IU-8, §8.2,
//! §11). The generic suites run over `Emulated<LogEngine>` and `KernoDb`.
//!
//! * `retention_rules` — every rule by hand: what is pruned and kept (the
//!   head, the revision before a tombstone, the revision in force at a
//!   cut-off), the reads that answer `Pruned` afterwards, the `Retention`
//!   change set and its members, the report, budgets, idempotence, the
//!   `Audit` cleanup of payloads a `Versioned` period left, the `None`
//!   cleanup, and argument errors.
//! * `retention_every_point` (IU-7) — a crash before every mutating I/O
//!   operation of `run_retention`: the recovered store is exactly the one
//!   before or the one after it, every unit keeps its head.
//! * `restore_payload_limit` — `restore` obeys the `max_payload_len` in
//!   force (E3-06 note).
//! * `history_sim_reclaims_space` (`KernoDb` only: it needs the engine's
//!   directed compaction) — a `history_sim`-like workload of many revisions,
//!   pruned to the last one: `compact` reclaims measurable space, and not
//!   while a snapshot still sees the pruned revisions (I-05).
//!
//! IU-8 with retention (`snapshot_vs_retention`) is in `tests/history.rs`;
//! the model-based retention rounds are in `tests/policies.rs`.

use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kerno_core::Durability;
use kerno_core::contract::{Options, Snapshot, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::{CrashKind, SimVfs};
use kerno_units::codec::{Key, RevisionMeta};
use kerno_units::payload::PAYLOAD_CODEC_OPAQUE;
use kerno_units::*;

const DIR: &str = "/db";
const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const SECOND: i64 = 1_000_000;

type Dump = Vec<(Vec<u8>, Vec<u8>)>;

/// A clock that moves one second per reading (one per change set).
fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::stepping(T0, SECOND)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: TxnEngine>(disk: &SimVfs, c: &UnitConfig) -> UnitDb<E> {
    UnitDb::<E>::open(disk.as_vfs(), Path::new(DIR), c).expect("open")
}

fn meta() -> ChangeMeta {
    ChangeMeta::default()
}

/// `verify_units` (E3-07) finds nothing.
#[track_caller]
fn verified<E: TxnEngine>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().unwrap_or_else(|e| panic!("{ctx}: verify_units: {e}"));
    assert!(r.is_clean(), "{ctx}: verify_units found {:?}", r.findings);
}

fn new_unit(t: &str, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: 1, metadata: b"m".to_vec(), payload: payload.to_vec() }
}

fn upd(payload: &[u8]) -> UnitUpdate {
    UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }
}

fn dump<S: Snapshot>(s: &S) -> Dump {
    let mut out: Dump = Vec::new();
    loop {
        let start = match out.last() {
            Some((k, _)) => Bound::Excluded(k.clone()),
            None => Bound::Unbounded,
        };
        let page = s.scan(start.as_ref().map(Vec::as_slice), Bound::Unbounded, 256).expect("scan");
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

/// Revision numbers of the stored `REV_META` and `REV_PAYLOAD` of `unit`.
fn stored(d: &Dump, unit: UnitId) -> (Vec<u64>, Vec<u64>) {
    let (mut metas, mut payloads) = (Vec::new(), Vec::new());
    for (k, _) in d {
        match Key::decode(k).expect("key") {
            Key::RevMeta { unit: u, rev } if u == unit => metas.push(rev.0),
            Key::RevPayload { unit: u, rev } if u == unit => payloads.push(rev.0),
            _ => {}
        }
    }
    (metas, payloads)
}

fn update<E: TxnEngine>(db: &UnitDb<E>, id: UnitId, p: &[u8]) -> CommitSeq {
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.update(id, upd(p), None).unwrap();
    cs.commit().unwrap().commit_seq
}

fn delete<E: TxnEngine>(db: &UnitDb<E>, id: UnitId) -> CommitSeq {
    let mut cs = db.begin_changeset(meta()).unwrap();
    cs.delete(id, None).unwrap();
    cs.commit().unwrap().commit_seq
}

/// A unit of type `t` with revisions 1..=n (payloads `[i; 10]`) and the
/// commit sequence of each.
fn unit_with<E: TxnEngine>(db: &UnitDb<E>, t: &str, n: u8) -> (UnitId, Vec<CommitSeq>) {
    let (id, c) = db.create_unit(new_unit(t, &[1; 10]), meta()).unwrap();
    let mut seqs = vec![c.commit_seq];
    for i in 2..=n {
        seqs.push(update(db, id, &[i; 10]));
    }
    (id, seqs)
}

fn range(lo: u64, hi: u64) -> Vec<u64> {
    (lo..=hi).collect()
}

fn run<E: TxnEngine>(db: &UnitDb<E>, p: &RetentionPolicy) -> RetentionReport {
    db.run_retention(p, RetentionBudget::default()).expect("run_retention")
}

// ------------------------------------------------------------ retention_rules

fn retention_rules<E: TxnEngine>() {
    let disk = SimVfs::new(60);
    let db = open::<E>(&disk, &cfg());
    // Nothing to do on an empty database: no change set, no mark.
    let r = run(&db, &RetentionPolicy::all(RetentionRule::KeepLast(1)));
    assert_eq!((r.change_set.is_none(), r.complete, r.units_examined), (true, true, 0));
    assert!(raw(&db).is_empty(), "retention on an empty database writes nothing");

    let (a, sa) = unit_with(&db, "doc", 6);
    let (b, _) = unit_with(&db, "doc", 3);
    let before = raw(&db);
    let seq_before = db.snapshot().unwrap().seq();

    // KeepAll (the default policy) does nothing.
    let r = run(&db, &RetentionPolicy::default());
    assert_eq!((r.change_set, r.units_examined, r.complete), (None, 2, true));
    assert!(raw(&db) == before);

    // KeepLast(2) for "doc".
    let p = RetentionPolicy::default().with_type("doc", RetentionRule::KeepLast(2));
    let r = run(&db, &p);
    let info = r.change_set.clone().expect("a retention change set");
    assert_eq!(info.commit_seq, CommitSeq(seq_before.0 + 1));
    assert_eq!((r.units_examined, r.units_retained, r.revisions_pruned, r.payloads_dropped), (2, 2, 5, 0));
    assert!(r.bytes_logically_freed > 5 * 35, "{r:?}");
    let d = raw(&db);
    assert_eq!(stored(&d, a), (range(5, 6), range(5, 6)));
    assert_eq!(stored(&d, b), (range(2, 3), range(2, 3)));
    let cs = db.change_set(info.change_set).unwrap().unwrap();
    assert_eq!(cs.purpose, Purpose::Retention);
    assert_eq!(
        cs.members,
        vec![
            ChangeSetMember { revision: RevisionId { unit: a, rev: RevNo(5) }, kind: RevisionKind::Update },
            ChangeSetMember { revision: RevisionId { unit: b, rev: RevNo(2) }, kind: RevisionKind::Update },
        ]
    );
    // The reads after pruning.
    let u = db.get_unit(a).unwrap().unwrap();
    assert_eq!((u.head, u.oldest, u.payload.as_deref()), (RevNo(6), RevNo(5), Some(&[6u8; 10][..])));
    assert!(matches!(db.get_revision(a, RevNo(4)), Err(UnitError::Pruned { oldest: RevNo(5), .. })));
    assert_eq!(db.get_revision(a, RevNo(5)).unwrap().payload.as_deref(), Some(&[5u8; 10][..]));
    assert!(matches!(db.restore(a, RevNo(1), None, meta()), Err(UnitError::Pruned { .. })));
    assert!(matches!(db.unit_as_of(a, AsOf::Seq(sa[3])).unwrap(), UnitAt::Pruned { oldest: RevNo(5) }));
    assert!(matches!(db.unit_as_of(a, AsOf::Seq(sa[4])).unwrap(), UnitAt::Present(r) if r.id.rev == RevNo(5)));
    assert_eq!(db.unit_as_of(a, AsOf::Seq(CommitSeq(0))).unwrap(), UnitAt::Absent);
    let revs = |q: HistoryQuery| -> Vec<u64> { db.history(a, q).unwrap().iter().map(|r| r.id.rev.0).collect() };
    assert_eq!(revs(HistoryQuery::oldest(10)), vec![5, 6]);
    assert_eq!(revs(HistoryQuery::latest(10)), vec![6, 5]);
    // Carried item: a page that ends at or below the oldest retained
    // revision is Pruned, never an empty page.
    for to in [0, 1, 4, 5] {
        for q in [HistoryQuery::oldest(10), HistoryQuery::latest(10)] {
            let q = HistoryQuery { to: Some(RevNo(to)), ..q };
            assert!(matches!(db.history(a, q), Err(UnitError::Pruned { oldest: RevNo(5), .. })), "to {to}");
        }
    }
    assert!(matches!(
        db.history(a, HistoryQuery { from: Some(RevNo(4)), ..HistoryQuery::oldest(10) }),
        Err(UnitError::Pruned { .. })
    ));
    assert!(revs(HistoryQuery { from: Some(RevNo(5)), to: Some(RevNo(5)), ..HistoryQuery::oldest(10) }).is_empty());
    // An unpruned unit: to = 1 is still an empty page.
    let (c, _) = unit_with(&db, "img", 2);
    assert!(db.history(c, HistoryQuery { to: Some(RevNo(1)), ..HistoryQuery::oldest(10) }).unwrap().is_empty());

    // Idempotent: the same retention again writes nothing.
    let after = raw(&db);
    let r = run(&db, &p);
    assert_eq!((r.change_set, r.complete, r.units_examined), (None, true, 3));
    assert!(raw(&db) == after);

    // KeepSince keeps the revision in force at s.
    let (k, sk) = unit_with(&db, "k", 5);
    let r = run(&db, &RetentionPolicy::default().with_type("k", RetentionRule::KeepSince(sk[2])));
    assert_eq!(r.revisions_pruned, 2);
    assert_eq!(db.get_unit(k).unwrap().unwrap().oldest, RevNo(3));
    assert!(matches!(db.unit_as_of(k, AsOf::Seq(sk[2])).unwrap(), UnitAt::Present(r) if r.id.rev == RevNo(3)));
    // KeepSince before every revision keeps everything.
    assert!(run(&db, &RetentionPolicy::all(RetentionRule::KeepSince(CommitSeq(0)))).change_set.is_none());

    // KeepFor: revisions are one second apart (one clock reading per change
    // set); the retention change set reads the clock once more.
    let (f, _) = unit_with(&db, "f", 5);
    let r = run(&db, &RetentionPolicy::default().with_type("f", RetentionRule::KeepFor(Duration::from_secs(2))));
    // Revision 5 was created 1 s before the retention, 4 two seconds before:
    // both are kept, and revision 3, in force at the cut-off.
    assert_eq!(r.revisions_pruned, 2);
    assert_eq!(db.get_unit(f).unwrap().unwrap().oldest, RevNo(3));
    let r = run(&db, &RetentionPolicy::default().with_type("f", RetentionRule::KeepFor(Duration::ZERO)));
    assert_eq!(db.get_unit(f).unwrap().unwrap().oldest, RevNo(5), "{r:?}");

    // A deleted unit keeps its tombstone and the revision before it.
    let (x, _) = unit_with(&db, "x", 4);
    delete(&db, x);
    run(&db, &RetentionPolicy::default().with_type("x", RetentionRule::KeepLast(1)));
    let d = raw(&db);
    assert_eq!(stored(&d, x), (range(4, 5), range(4, 4)));
    assert_eq!(db.get_unit(x).unwrap().unwrap().lifecycle, Lifecycle::Deleted);
    let h = db.history(x, HistoryQuery::oldest(10)).unwrap();
    assert_eq!(h.iter().map(|r| r.kind).collect::<Vec<_>>(), vec![RevisionKind::Update, RevisionKind::Delete]);

    // Budgets: one unit per call, resumed until complete.
    let (y1, _) = unit_with(&db, "y", 3);
    let (y2, _) = unit_with(&db, "y", 3);
    let p = RetentionPolicy::default().with_type("y", RetentionRule::KeepLast(1));
    let one = RetentionBudget { max_units: 1, max_ops: 100 };
    let r1 = db.run_retention(&p, one).unwrap();
    assert_eq!((r1.units_retained, r1.complete), (1, false));
    assert_eq!(db.get_unit(y1).unwrap().unwrap().oldest, RevNo(3));
    assert_eq!(db.get_unit(y2).unwrap().unwrap().oldest, RevNo(1));
    let r2 = db.run_retention(&p, one).unwrap();
    assert_eq!((r2.units_retained, r2.complete), (1, true));
    assert_eq!(db.get_unit(y2).unwrap().unwrap().oldest, RevNo(3));
    assert!(db.run_retention(&p, one).unwrap().change_set.is_none());

    // Argument errors write nothing.
    let s = db.snapshot().unwrap().seq();
    for (p, b) in [
        (RetentionPolicy::all(RetentionRule::KeepLast(0)), RetentionBudget::default()),
        (RetentionPolicy::default().with_type("doc", RetentionRule::KeepLast(0)), RetentionBudget::default()),
        (RetentionPolicy::default(), RetentionBudget { max_units: 0, max_ops: 10 }),
        (RetentionPolicy::default(), RetentionBudget { max_units: 1, max_ops: 3 }),
    ] {
        assert!(matches!(db.run_retention(&p, b), Err(UnitError::InvalidArgument(_))), "{p:?} {b:?}");
    }
    assert_eq!(db.snapshot().unwrap().seq(), s);
    // Application change sets may not claim purpose Retention.
    assert!(matches!(
        db.begin_changeset(ChangeMeta { purpose: Purpose::Retention, ..meta() }),
        Err(UnitError::InvalidArgument(_))
    ));
    verified(&db, "retention_rules");
    db.close().unwrap();
}

/// Retention enforces the history policy on what a policy change left.
fn retention_cleans_policies<E: TxnEngine>() {
    let disk = SimVfs::new(61);
    let db = open::<E>(&disk, &cfg());
    // A Versioned period, then Audit: revisions 1..=3 keep their payloads
    // (only the superseded head, 4, loses it when 5 is written).
    let (a, _) = unit_with(&db, "aud", 4);
    db.set_policy("aud", HistoryPolicy::Audit, meta()).unwrap();
    update(&db, a, &[5; 10]);
    assert_eq!(stored(&raw(&db), a), (range(1, 5), vec![1, 2, 3, 5]));
    // A Versioned period, then None without a new revision.
    let (n, _) = unit_with(&db, "non", 3);
    db.set_policy("non", HistoryPolicy::None, meta()).unwrap();
    assert_eq!(stored(&raw(&db), n), (range(1, 3), range(1, 3)));

    // KeepAll still enforces the policies, with a budget of 4 writes: the
    // first call drops one payload of `a` (member + 2 writes), and stops.
    let p = RetentionPolicy::default();
    let small = RetentionBudget { max_units: 10, max_ops: 4 };
    let r = db.run_retention(&p, small).unwrap();
    assert_eq!((r.units_retained, r.payloads_dropped, r.complete), (1, 1, false));
    assert_eq!(stored(&raw(&db), a), (range(1, 5), vec![2, 3, 5]));
    let mut calls = 1;
    while !db.run_retention(&p, small).unwrap().complete {
        calls += 1;
        assert!(calls < 10);
    }
    let d = raw(&db);
    assert_eq!(stored(&d, a), (range(1, 5), vec![5]), "Audit keeps metadata, drops old payloads");
    assert_eq!(stored(&d, n), (vec![3], vec![3]), "None keeps only the head");
    // The rewritten REV_META keep hash and length, without payload.
    let m = RevisionMeta::decode(
        RevNo(1),
        &db.engine()
            .snapshot()
            .unwrap()
            .get(&Key::RevMeta { unit: a, rev: RevNo(1) }.encode().unwrap())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(!m.payload_present);
    assert_eq!((m.payload_len, m.content_hash), (10, ContentHash::blake3(PAYLOAD_CODEC_OPAQUE, &[1; 10])));
    assert_eq!(db.history(a, HistoryQuery::oldest(10)).unwrap().len(), 5);
    // Back to Versioned: the old content is gone for good, never corruption.
    db.set_policy("aud", HistoryPolicy::Versioned, meta()).unwrap();
    assert!(matches!(
        db.get_revision(a, RevNo(2)),
        Err(UnitError::HistoryUnavailable { policy: HistoryPolicy::Audit })
    ));
    db.set_policy("non", HistoryPolicy::Versioned, meta()).unwrap();
    assert!(matches!(db.get_revision(n, RevNo(2)), Err(UnitError::Pruned { oldest: RevNo(3), .. })));
    assert_eq!(db.get_unit(n).unwrap().unwrap().payload.as_deref(), Some(&[3u8; 10][..]));
    verified(&db, "retention_cleans_policies");
    db.close().unwrap();
}

/// Retention reads verify: a hole in the retained revisions is corruption,
/// not "nothing to prune".
fn retention_detects_damage<E: TxnEngine>() {
    let disk = SimVfs::new(62);
    let db = open::<E>(&disk, &cfg());
    let (a, _) = unit_with(&db, "doc", 4);
    let mut b = kerno_core::WriteBatch::new();
    b.delete(Key::RevMeta { unit: a, rev: RevNo(2) }.encode().unwrap());
    kerno_core::contract::Engine::commit(db.engine(), &b).unwrap();
    let s = db.snapshot().unwrap().seq();
    let e =
        db.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default()).unwrap_err();
    assert!(matches!(&e, UnitError::Storage(kerno_core::Error::Corruption(m)) if m.contains("missing")), "{e}");
    assert_eq!(db.snapshot().unwrap().seq(), s, "nothing written");
    db.close().unwrap();
}

// ------------------------------------------------------------ restore limit

fn restore_payload_limit<E: TxnEngine>() {
    let disk = SimVfs::new(63);
    let mut c = cfg();
    let db = open::<E>(&disk, &c);
    let (a, _) = db.create_unit(new_unit("doc", &[7; 100]), meta()).unwrap();
    update(&db, a, &[8; 10]);
    db.close().unwrap();
    c.max_payload_len = 50;
    let db = open::<E>(&disk, &c);
    let s = db.snapshot().unwrap().seq();
    let e = db.restore(a, RevNo(1), None, meta()).unwrap_err();
    assert!(matches!(e, UnitError::PayloadTooLarge { len: 100, max: 50 }), "{e}");
    assert_eq!(db.snapshot().unwrap().seq(), s);
    // Reading it, and restoring one that fits, still work.
    assert_eq!(db.get_revision(a, RevNo(1)).unwrap().payload_len, 100);
    db.restore(a, RevNo(2), None, meta()).unwrap();
    verified(&db, "restore_payload_limit");
    db.close().unwrap();
}

// ------------------------------------------------------------ change-set limit

fn retention_respects_changeset_limit<E: TxnEngine>() {
    let disk = SimVfs::new(64);
    let mut c = cfg();
    c.max_changeset_ops = 3;
    let db = open::<E>(&disk, &c);
    for _ in 0..7 {
        let (id, _) = db.create_unit(new_unit("doc", b"a"), meta()).unwrap();
        update(&db, id, b"b");
    }
    let p = RetentionPolicy::all(RetentionRule::KeepLast(1));
    let mut retained = Vec::new();
    loop {
        let r = run(&db, &p);
        assert!(r.units_retained <= 3, "{r:?}");
        retained.push(r.units_retained);
        if r.complete {
            break;
        }
    }
    assert_eq!(retained.iter().sum::<u64>() as usize, 7, "{retained:?}");
    assert_eq!(run(&db, &p).units_retained, 0);
    verified(&db, "retention_respects_changeset_limit");
    db.close().unwrap();
}

// ------------------------------------------------------------ retention_every_point

/// Units with long histories under several policies, a deleted one and
/// payloads an Audit period must drop.
fn every_point_setup<E: TxnEngine>(db: &UnitDb<E>) -> Vec<UnitId> {
    let (a, _) = unit_with(db, "doc", 8);
    let (b, _) = unit_with(db, "doc", 5);
    delete(db, b);
    let (c, _) = unit_with(db, "aud", 6);
    db.set_policy("aud", HistoryPolicy::Audit, meta()).unwrap();
    update(db, c, &[9; 300]);
    let (d, _) = unit_with(db, "img", 2);
    vec![a, b, c, d]
}

fn every_point_policy() -> RetentionPolicy {
    RetentionPolicy::all(RetentionRule::KeepLast(2))
        .with_type("img", RetentionRule::KeepAll)
        .with_type("aud", RetentionRule::KeepAll)
}

/// IU-7: a crash before every mutating I/O operation of `run_retention`
/// (and of the close after it) recovers exactly the store before it or the one after it (the retention
/// is one engine commit), and every unit keeps its head.
fn retention_every_point<E: TxnEngine>(kind: CrashKind, durability: Durability) {
    let config = || {
        let mut c = cfg();
        c.engine.durability = durability;
        c
    };
    let disk = SimVfs::new(9);
    let db = open::<E>(&disk, &config());
    let ids = every_point_setup(&db);
    let before = raw(&db);
    let heads: Vec<Unit> = ids.iter().map(|id| db.get_unit(*id).unwrap().unwrap()).collect();
    let start = disk.mutating_ops();
    let r = db.run_retention(&every_point_policy(), RetentionBudget::default()).unwrap();
    assert!(r.change_set.is_some() && r.revisions_pruned > 0 && r.payloads_dropped > 0, "{r:?}");
    let after = raw(&db);
    // The sweep covers the retention commit and the close that writes it to
    // tables (flush of the range deletions), so that both the WAL frame and
    // the table files are cut at every point.
    db.close().unwrap();
    let total = disk.mutating_ops() - start;
    assert!(total > 2, "retention and close did too little I/O: {total}");
    for point in 1..=total {
        let ctx = format!("{kind:?} {durability:?} before mutating op #{point} of {total}");
        let disk = SimVfs::new(9);
        let db = open::<E>(&disk, &config());
        every_point_setup(&db);
        disk.crash_after(point, kind);
        let acked = db.run_retention(&every_point_policy(), RetentionBudget::default()).is_ok();
        let _ = db.close();
        assert!(disk.has_crashed(), "{ctx}: the crash did not fire");
        let image = disk.take_crash_image().expect("crash image");
        let after_cfg = UnitConfig { ids: Arc::new(CounterIds::new(1_000)), ..config() };
        let db = UnitDb::<E>::open(image.as_vfs(), Path::new(DIR), &after_cfg)
            .unwrap_or_else(|e| panic!("{ctx}: reopen: {e}"));
        let got = raw(&db);
        verified(&db, &ctx);
        let whole = got == after;
        assert!(whole || got == before, "{ctx}: retention is half applied");
        if acked && durability == Durability::Sync {
            assert!(whole, "{ctx}: an acknowledged Sync retention was lost");
        }
        for (id, h) in ids.iter().zip(&heads) {
            let u = db.get_unit(*id).unwrap_or_else(|e| panic!("{ctx}: get_unit: {e}")).expect("unit");
            assert_eq!((u.head, &u.payload, u.lifecycle), (h.head, &h.payload, h.lifecycle), "{ctx}: head kept");
            let hist = db.history(*id, HistoryQuery::oldest(100)).unwrap_or_else(|e| panic!("{ctx}: history: {e}"));
            assert_eq!(hist.first().map(|r| r.id.rev), Some(u.oldest), "{ctx}: oldest_rev matches the keys");
        }
        // Retention can run again and finishes the job.
        db.run_retention(&every_point_policy(), RetentionBudget::default())
            .unwrap_or_else(|e| panic!("{ctx}: retention after recovery: {e}"));
        let mut done: Dump = raw(&db);
        // Only the change set sequence and clock may differ from `after`.
        done.retain(|(k, _)| k[0] != 0x30 && k[0] != 0x31);
        let mut want = after.clone();
        want.retain(|(k, _)| k[0] != 0x30 && k[0] != 0x31);
        assert!(done == want, "{ctx}: retention after recovery differs");
        verified(&db, &format!("{ctx}, retention after recovery"));
        drop(db);
    }
}

// ------------------------------------------------------------ history_sim

/// A `history_sim`-like workload (many revisions of few units, a few KiB
/// each): retention to the last revision frees logically what `compact`
/// then reclaims, once no snapshot needs it (I-05).
fn history_sim_reclaims_space() {
    let disk = SimVfs::new(64);
    let db = open::<KernoDb>(&disk, &cfg());
    let mut ids = Vec::new();
    for u in 0..8u8 {
        let (id, _) = db.create_unit(new_unit("doc", &vec![u; 4096]), meta()).unwrap();
        ids.push(id);
    }
    for round in 0..24u8 {
        let mut cs = db.begin_changeset(meta()).unwrap();
        for (i, id) in ids.iter().enumerate() {
            cs.update(*id, upd(&vec![round ^ i as u8; 4096]), None).unwrap();
        }
        cs.commit().unwrap();
    }
    let first = db.compact().unwrap();
    let full = first.stored_bytes_after.expect("KernoDb reports its table bytes");
    assert!(full > 8 * 25 * 4096 / 2, "the history is on disk: {first:?}");

    let old = db.snapshot().unwrap();
    let r = run(&db, &RetentionPolicy::all(RetentionRule::KeepLast(1)));
    assert_eq!(r.revisions_pruned, 8 * 24);
    assert!(r.bytes_logically_freed >= 8 * 24 * 4096, "{r:?}");
    // A snapshot from before the retention still reads everything (IU-8),
    // even across a compaction.
    db.compact().unwrap();
    assert_eq!(
        old.history(ids[0], HistoryQuery { with_payload: true, ..HistoryQuery::oldest(100) }).unwrap().len(),
        25
    );
    drop(old);
    let c = db.compact().unwrap();
    let reclaimed = full.saturating_sub(c.stored_bytes_after.unwrap());
    assert!(reclaimed >= 8 * 24 * 4096 / 2, "compact reclaimed {reclaimed} of {full} bytes: {c:?}");
    for id in &ids {
        assert_eq!(db.get_unit(*id).unwrap().unwrap().payload.unwrap().len(), 4096);
    }
    verified(&db, "history_sim_reclaims_space");
    db.close().unwrap();
}

#[test]
fn kerno_history_sim_reclaims_space() {
    history_sim_reclaims_space();
}

macro_rules! engine_tests {
    ($m:ident, $e:ty) => {
        mod $m {
            use super::*;
            type E = $e;

            #[test]
            fn retention_rules() {
                super::retention_rules::<E>();
            }
            #[test]
            fn retention_cleans_policies() {
                super::retention_cleans_policies::<E>();
            }
            #[test]
            fn retention_detects_damage() {
                super::retention_detects_damage::<E>();
            }
            #[test]
            fn retention_respects_changeset_limit() {
                super::retention_respects_changeset_limit::<E>();
            }
            #[test]
            fn restore_payload_limit() {
                super::restore_payload_limit::<E>();
            }
            #[test]
            fn retention_every_point_power_loss_sync() {
                super::retention_every_point::<E>(CrashKind::PowerLoss, Durability::Sync);
            }
            #[test]
            fn retention_every_point_process_kill_buffered() {
                super::retention_every_point::<E>(CrashKind::ProcessKill, Durability::Buffered);
            }
        }
    };
}

engine_tests!(emulated_log, Emulated<LogEngine>);
engine_tests!(kerno, KernoDb);
