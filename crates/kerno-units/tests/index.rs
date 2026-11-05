//! E4-03: the primary index and basic filters (ADR-0007) over
//! `Emulated<LogEngine>` and `KernoDb`.
//!
//! * `states_and_watermark` — ready, stale and the watermark through a
//!   sequence of commits; the build is an `Index` change set (I-23).
//! * `building_is_reported_while_a_build_waits` — `Building` is seen while a
//!   build waits for the write slot, and only through the handle.
//! * `snapshots_keep_their_own_status` — an older snapshot stays `Ready`.
//! * `failed_build_is_recorded_and_repaired` — a damaged head fails the build,
//!   the status says so and the entries do not change; a rebuild repairs it.
//! * `status_survives_reopen` — ready, stale and failed are the stored truth.
//! * `differential_*` — the index path equals the reference path on the same
//!   snapshot for every filter after every step of a seeded workload, with and
//!   without a fresh index (I-32).
//! * `rebuilt_equals_fresh` — an index rebuilt after churn is byte for byte
//!   the one a database that never had one builds from the same operations
//!   (I-24).
//! * `verifier_*` — damage to the index is UNIT-37, and a merely stale index
//!   is not damage; the index path refuses what it can see.
//! * E4-04: `check` holds `IDX_PRIMARY` and `IDX_TYPE` each to the
//!   reference path; damage to `IDX_TYPE` alone is UNIT-38; the rebuild and
//!   crash tests cover both, since they are one generation.
//! * `index_every_point` — a crash before every mutating I/O operation of a
//!   scenario with builds: the stored status is the one of the recovered
//!   commit, whole generation or none (I-21, I-23).
//!
//! The expectation is tracked by the test (a model of the units), never read
//! back from the layer.

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use harness::units::{StepIds, Target, dump, open_units, seq, unit_config, verified};
use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_core::{Durability, WriteBatch};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::{CrashKind, SimVfs};
use kerno_units::codec::{IndexMeta, Key, PrimaryEntry};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const TYPES: [&str; 3] = ["a", "b", "c"];
const SCHEMAS: [u32; 3] = [1, 2, 3];

fn config(first_id: u64) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(first_id)));
    c.engine = Options::small_for_tests();
    c
}

fn open_on<E: TxnEngine>(vfs: &SimVfs, first_id: u64) -> UnitDb<E> {
    UnitDb::<E>::open(vfs.as_vfs(), Path::new("/db"), &config(first_id)).expect("open")
}

fn open<E: TxnEngine>() -> UnitDb<E> {
    open_on(&SimVfs::new(1), 1)
}

fn new_unit(t: &str, schema: u32, payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: schema, metadata: Vec::new(), payload: payload.to_vec() }
}

/// New units `(type, schema_version)` in one change set.
fn create_units<E: TxnEngine>(db: &UnitDb<E>, spec: &[(&str, u32)]) -> Vec<UnitId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    let ids =
        spec.iter().enumerate().map(|(i, (t, v))| cs.create(new_unit(t, *v, &[i as u8])).expect("create")).collect();
    cs.commit().expect("commit");
    ids
}

fn upd(payload: &[u8], schema: Option<u32>) -> UnitUpdate {
    UnitUpdate { payload: payload.to_vec(), schema_version: schema, metadata: None }
}

fn ready(w: u64) -> IndexStatus {
    IndexStatus { state: IndexState::Ready, watermark: Some(CommitSeq(w)) }
}

fn stale(w: Option<u64>) -> IndexStatus {
    IndexStatus { state: IndexState::Stale, watermark: w.map(CommitSeq) }
}

fn failed(w: Option<u64>) -> IndexStatus {
    IndexStatus { state: IndexState::Failed, watermark: w.map(CommitSeq) }
}

#[track_caller]
fn assert_clean<E: TxnEngine>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().expect("verify");
    assert!(r.is_clean(), "{ctx}: verify_units found {:?}", r.findings);
}

/// Puts (`Some`) and deletions (`None`) applied straight to the engine.
type Patch = Vec<(Vec<u8>, Option<Vec<u8>>)>;

fn put_all<E: Engine>(db: &E, puts: Patch) {
    let mut b = WriteBatch::new();
    for (k, v) in puts {
        match v {
            Some(v) => b.put(k, v),
            None => b.delete(k),
        };
    }
    db.commit(&b).expect("raw commit");
}

/// The stored values of `keys` right now, as the patch that restores them.
fn undo_of<E: Engine>(db: &E, keys: impl IntoIterator<Item = Vec<u8>>) -> Patch {
    let snap = db.snapshot().expect("snapshot");
    keys.into_iter()
        .map(|k| {
            let v = snap.get(&k).expect("get");
            (k, v)
        })
        .collect()
}

/// Every stored `IDX_PRIMARY` and `IDX_TYPE` pair, raw, in key order.
fn raw_entries<E: TxnEngine>(db: &UnitDb<E>) -> Vec<(Vec<u8>, Vec<u8>)> {
    dump(&db.engine().snapshot().expect("snapshot")).into_iter().filter(|(k, _)| k[0] == 0x51 || k[0] == 0x52).collect()
}

fn type_key(unit_type: &str, schema_version: u32, unit: UnitId) -> Vec<u8> {
    Key::IndexType { unit_type: unit_type.into(), schema_version, unit }.encode().expect("key")
}

fn type_value() -> Vec<u8> {
    vec![1, 0]
}

fn entry_key(unit: UnitId) -> Vec<u8> {
    Key::IndexPrimary { unit }.encode().expect("key")
}

fn entry_value(unit_type: &str, schema_version: u32) -> Vec<u8> {
    PrimaryEntry { unit_type: unit_type.into(), schema_version }.encode().expect("entry")
}

fn meta_key() -> Vec<u8> {
    Key::IndexMeta.encode().expect("key")
}

fn meta_value(watermark: u64, entries: u64) -> Vec<u8> {
    IndexMeta { outcome: BuildOutcome::Succeeded, watermark: CommitSeq(watermark), entries }.encode().expect("meta")
}

// ------------------------------------------------------------ the model

/// `(unit_type, schema_version, lifecycle)` of every unit the test created,
/// and, for the units `random_step` wrote, their revisions (see [`Revs`]).
#[derive(Clone, Default)]
struct Model(BTreeMap<UnitId, (String, u32, Lifecycle)>, Revs);

/// Per unit: the oldest revision retention left and the `schema_version` of
/// every revision (`schemas[rev_no - 1]`), so that a `restore` knows which
/// sources exist and what schema it brings back.
#[derive(Clone, Default)]
struct Revs(BTreeMap<UnitId, (u64, Vec<u32>)>);

/// Retention rule of the `run_retention` steps of [`random_step`].
const KEEP: u64 = 2;

/// The filter, written out case by case (not the layer's `matches`).
fn wants(f: &UnitFilter, unit_type: &str, schema: u32) -> bool {
    match (&f.unit_type, f.schema_version) {
        (None, None) => true,
        (Some(t), None) => t == unit_type,
        (None, Some(v)) => v == schema,
        (Some(t), Some(v)) => t == unit_type && v == schema,
    }
}

impl Model {
    /// Ids of the active units matching `f`, in id order.
    fn active(&self, f: &UnitFilter) -> Vec<UnitId> {
        self.0.iter().filter(|(_, (t, v, l))| *l == Lifecycle::Active && wants(f, t, *v)).map(|(id, _)| *id).collect()
    }
}

/// Every combination of the two conditions, plus values nothing has.
fn filters() -> Vec<UnitFilter> {
    let mut out = vec![UnitFilter::default()];
    let types = || TYPES.iter().copied().chain(["none"]);
    let schemas = || SCHEMAS.iter().copied().chain([9]);
    out.extend(types().map(|t| UnitFilter { unit_type: Some(t.into()), schema_version: None }));
    out.extend(schemas().map(|v| UnitFilter { unit_type: None, schema_version: Some(v) }));
    for t in TYPES {
        for v in SCHEMAS {
            out.push(UnitFilter { unit_type: Some(t.into()), schema_version: Some(v) });
        }
    }
    out
}

/// One random change set, or an abort; `model` follows what was committed.
fn random_step<E: TxnEngine>(db: &UnitDb<E>, rng: &mut Rng, model: &mut Model) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    let mut staged = model.clone();
    let mut touched = BTreeSet::new();
    for _ in 0..rng.range(1, 6) {
        let candidates: Vec<UnitId> = staged
            .0
            .iter()
            .filter(|(id, (_, _, l))| !touched.contains(*id) && *l != Lifecycle::Deleted)
            .map(|(id, _)| *id)
            .collect();
        let create = candidates.is_empty() || rng.chance(1, 3);
        if create {
            let (t, v) = (TYPES[rng.below(3) as usize], SCHEMAS[rng.below(3) as usize]);
            let id = cs.create(new_unit(t, v, &rng.bytes(4))).expect("create");
            staged.0.insert(id, (t.into(), v, Lifecycle::Active));
            staged.1.0.insert(id, (1, vec![v]));
            touched.insert(id);
            continue;
        }
        let id = candidates[rng.below(candidates.len() as u64) as usize];
        let entry = staged.0.get_mut(&id).expect("candidate");
        let revs = staged.1.0.get_mut(&id);
        match (entry.2, rng.below(4)) {
            (Lifecycle::Active, 0) => {
                let v = rng.chance(1, 2).then(|| SCHEMAS[rng.below(3) as usize]);
                cs.update(id, upd(&rng.bytes(4), v), None).expect("update");
                if let Some(v) = v {
                    entry.1 = v;
                }
            }
            (Lifecycle::Active, 1) => {
                cs.archive(id, None).expect("archive");
                entry.2 = Lifecycle::Archived;
            }
            // Restore an older retained revision: the head takes its schema
            // version, so the index entry of the unit changes (E3-04 `restore`).
            (Lifecycle::Active, 3) if revs.as_ref().is_some_and(|(oldest, s)| (s.len() as u64) > *oldest) => {
                let (oldest, schemas) = revs.as_ref().expect("checked");
                let from = rng.range(*oldest, schemas.len() as u64 - 1);
                cs.restore(id, RevNo(from), None).expect("restore");
                entry.1 = schemas[from as usize - 1];
            }
            (Lifecycle::Archived, 0 | 1 | 3) => {
                cs.unarchive(id, None).expect("unarchive");
                entry.2 = Lifecycle::Active;
            }
            _ => {
                cs.delete(id, None).expect("delete");
                entry.2 = Lifecycle::Deleted;
            }
        }
        let schema = entry.1;
        if let Some((_, schemas)) = staged.1.0.get_mut(&id) {
            schemas.push(schema);
        }
        touched.insert(id);
    }
    if rng.chance(1, 6) {
        cs.abort();
    } else {
        cs.commit().expect("commit");
        *model = staged;
    }
    // Retention commits its own change set and changes no head, so the index
    // goes stale without any entry changing; restores then see fewer sources.
    if rng.chance(1, 8) {
        db.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(KEEP)), RetentionBudget::default())
            .expect("run_retention");
        for (oldest, schemas) in model.1.0.values_mut() {
            *oldest = (*oldest).max((schemas.len() as u64).saturating_sub(KEEP - 1));
        }
    }
}

/// The reference path is the model's answer; the index path is the
/// reference's when the index is `Ready` at this snapshot, and refuses
/// otherwise; `query` always answers like the reference and says which path.
/// Returns the status it saw.
#[track_caller]
fn check<E: Engine>(snap: &UnitSnapshot<E>, model: &Model, ctx: &str) -> IndexStatus {
    let status = snap.index_status().expect("index_status");
    assert_eq!(
        status.state == IndexState::Ready,
        status.watermark == Some(snap.seq()),
        "{ctx}: Ready iff watermark = seq"
    );
    for f in filters() {
        let reference = snap.reference_filter(&f).expect("reference_filter");
        let ids: Vec<UnitId> = reference.iter().map(|u| u.id).collect();
        assert_eq!(ids, model.active(&f), "{ctx}: reference path vs model, {f:?}");
        let out = snap.query(&f).expect("query");
        assert_eq!(out.units, reference, "{ctx}: query vs reference, {f:?}");
        // E5-02: `query` follows the plan the snapshot explains for the
        // filter, and asking twice gives the same plan (determinism).
        let key = FingerprintKey::from_bytes([0x42; 32]);
        let plan = snap.explain_filter(&key, &f).expect("explain_filter");
        assert_eq!(snap.explain_filter(&key, &f).expect("explain_filter"), plan, "{ctx}: plan is deterministic, {f:?}");
        assert_eq!(plan.chosen().path, out.path, "{ctx}: query ran the planned path, {f:?}");
        assert_eq!(plan.index(), Some(status), "{ctx}: the plan saw the snapshot's status, {f:?}");
        if let (IndexState::Ready, Some(watermark)) = (status.state, status.watermark) {
            assert_eq!(
                snap.indexed_scan(&f).expect("indexed_scan"),
                reference,
                "{ctx}: index path vs reference, {f:?}"
            );
            // Each index on its own, not only the one `Auto` picks (E4-04).
            assert_eq!(
                snap.indexed_scan_via(IndexPath::Primary, &f).expect("primary path"),
                reference,
                "{ctx}: IDX_PRIMARY vs reference, {f:?}"
            );
            let by_type = snap.indexed_scan_via(IndexPath::ByType, &f);
            if f.unit_type.is_some() {
                assert_eq!(by_type.expect("type path"), reference, "{ctx}: IDX_TYPE vs reference, {f:?}");
            } else {
                assert!(matches!(by_type, Err(UnitError::InvalidArgument(_))), "{ctx}: {by_type:?}");
            }
            // E5-02: with a `Ready` index the planner picks by cost, so the
            // source is whatever the path says; an answer is never a fallback
            // here, the index is exact.
            let want = match out.path {
                PlanPath::UnitsReference => QuerySource::Reference { index: status },
                PlanPath::IndexPrimary | PlanPath::IndexType => QuerySource::Index { watermark },
                other => panic!("{ctx}: a unit query went through {other:?}"),
            };
            assert_eq!(out.source, want, "{ctx}: {f:?}");
            assert_eq!(out.fallback, None, "{ctx}: {f:?}");
        } else {
            let refused = snap.indexed_scan(&f);
            let want = UnitError::IndexNotReady { state: status.state, watermark: status.watermark }.to_string();
            assert!(
                matches!(&refused, Err(e @ UnitError::IndexNotReady { .. }) if e.to_string() == want),
                "{ctx}: {refused:?}"
            );
            assert_eq!(out.source, QuerySource::Reference { index: status }, "{ctx}: {f:?}");
        }
    }
    status
}

// ---------------------------------------------------------------- states

fn states_and_watermark<E: TxnEngine>() {
    let db = open::<E>();
    let all = UnitFilter::default();
    assert_eq!(db.index_status().unwrap(), stale(None), "empty database: never built");
    let u = create_units(&db, &[("a", 1), ("b", 1), ("a", 2)]); // commit 1
    assert_eq!(db.index_status().unwrap(), stale(None), "units, but never built");
    let snap = db.snapshot().unwrap();
    assert!(matches!(
        snap.indexed_scan(&all),
        Err(UnitError::IndexNotReady { state: IndexState::Stale, watermark: None })
    ));
    let out = snap.query(&all).unwrap();
    assert_eq!((out.units.len(), out.source), (3, QuerySource::Reference { index: stale(None) }));

    // The build is commit 2 and its own watermark.
    assert_eq!(db.rebuild_index().unwrap(), ready(2));
    assert_eq!(db.index_status().unwrap(), ready(2));
    let built = db.snapshot().unwrap();
    assert_eq!(built.seq(), CommitSeq(2));
    let info = built.change_set(CommitSeq(2)).unwrap().expect("the build is a change set");
    assert_eq!((info.purpose, info.members.len()), (Purpose::Index, 0));
    assert_eq!(built.indexed_scan(&all).unwrap().iter().map(|x| x.id).collect::<Vec<_>>(), u);
    // No condition: the primary index costs what the reference does, and the
    // tie goes to the reference. A selective type goes to the index.
    let out = built.query(&all).unwrap();
    assert_eq!((out.path, out.source), (PlanPath::UnitsReference, QuerySource::Reference { index: ready(2) }));
    let by_type = UnitFilter { unit_type: Some("a".into()), schema_version: None };
    let out = built.query(&by_type).unwrap();
    assert_eq!((out.path, out.source), (PlanPath::IndexType, QuerySource::Index { watermark: CommitSeq(2) }));
    assert_eq!(out.units.iter().map(|x| x.id).collect::<Vec<_>>(), [u[0], u[2]]);
    assert_clean(&db, "after the first build");

    // An empty change set commits nothing, so the index is still ready.
    db.begin_changeset(ChangeMeta::default()).unwrap().commit().unwrap();
    assert_eq!(db.index_status().unwrap(), ready(2));

    // Any commit after the watermark makes it stale, keeping the watermark.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(u[0], upd(b"new", Some(7)), None).unwrap(); // commit 3
    cs.commit().unwrap();
    assert_eq!(db.index_status().unwrap(), stale(Some(2)));
    let after = db.snapshot().unwrap();
    let out = after.query(&UnitFilter { schema_version: Some(7), ..UnitFilter::default() }).unwrap();
    assert_eq!(out.source, QuerySource::Reference { index: stale(Some(2)) });
    assert_eq!(out.units.len(), 1, "the stale index does not hide the new schema version");
    assert_clean(&db, "a stale index is not damage");

    // Rebuilding replaces the generation: ready at the new commit.
    assert_eq!(db.rebuild_index().unwrap(), ready(4));
    let again = db.snapshot().unwrap();
    assert_eq!(again.indexed_scan(&UnitFilter { schema_version: Some(7), ..UnitFilter::default() }).unwrap().len(), 1);
    // Rebuilding a ready index builds another generation.
    assert_eq!(db.rebuild_index().unwrap(), ready(5));

    // A commit that touches no unit still makes it stale: freshness is never assumed.
    db.set_policy("a", HistoryPolicy::Audit, ChangeMeta::default()).unwrap(); // commit 6
    assert_eq!(db.index_status().unwrap(), stale(Some(5)));
    assert_eq!(db.rebuild_index().unwrap(), ready(7));
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(u[0], u[1], "t", RelationProvenance::Declared).unwrap(); // commit 8
    cs.commit().unwrap();
    assert_eq!(db.index_status().unwrap(), stale(Some(7)));
    assert_clean(&db, "end");
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::yield_now();
    }
}

fn building_is_reported_while_a_build_waits<E: TxnEngine>() {
    let db = open::<E>();
    create_units(&db, &[("a", 1), ("b", 1)]); // commit 1
    assert_eq!(db.rebuild_index().unwrap(), ready(2));
    // Hold the write slot: the build must wait for it.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let held = cs.create(new_unit("c", 1, b"held")).unwrap();
    std::thread::scope(|s| {
        let build = s.spawn(|| db.rebuild_index());
        wait_until("the build to be reported", || db.index_status().unwrap().state == IndexState::Building);
        // The watermark reported is the one still stored.
        assert_eq!(
            db.index_status().unwrap(),
            IndexStatus { state: IndexState::Building, watermark: Some(CommitSeq(2)) }
        );
        // A snapshot is a view of stored data: a build in flight is not in it.
        assert_eq!(db.snapshot().unwrap().index_status().unwrap(), ready(2));
        cs.commit().unwrap(); // commit 3: releases the slot
        let status = build.join().expect("the build thread").unwrap();
        assert_eq!(status, ready(4));
    });
    assert_eq!(db.index_status().unwrap(), ready(4), "Building ends with the build");
    let snap = db.snapshot().unwrap();
    let ids: Vec<UnitId> = snap.indexed_scan(&UnitFilter::default()).unwrap().iter().map(|u| u.id).collect();
    assert!(ids.contains(&held), "the build ran after the held change set");
    assert_clean(&db, "after the build");

    // A build that fails is no longer counted either.
    let head = Key::UnitHead { unit: held }.encode().unwrap();
    let good = db.engine().snapshot().unwrap().get(&head).unwrap().unwrap();
    put_all(db.engine(), vec![(head, Some(good[..good.len() - 1].to_vec()))]);
    assert!(db.rebuild_index().is_err());
    assert_eq!(db.index_status().unwrap().state, IndexState::Failed);
}

fn snapshots_keep_their_own_status<E: TxnEngine>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("b", 1)]);
    db.rebuild_index().unwrap(); // commit 2
    let old = db.snapshot().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[0], None).unwrap();
    cs.commit().unwrap();
    db.rebuild_index().unwrap(); // commit 4
    let all = UnitFilter::default();
    // The old snapshot still sees its own generation, ready at its own commit.
    assert_eq!(old.index_status().unwrap(), ready(2));
    assert_eq!(old.indexed_scan(&all).unwrap().len(), 2);
    assert_eq!(old.reference_filter(&all).unwrap(), old.indexed_scan(&all).unwrap());
    let new = db.snapshot().unwrap();
    assert_eq!(new.index_status().unwrap(), ready(4));
    assert_eq!(new.indexed_scan(&all).unwrap().len(), 1);
    // A snapshot older than the first build has no index at all.
    let before = SimVfs::new(2);
    let db2 = open_on::<E>(&before, 1);
    create_units(&db2, &[("a", 1)]);
    let none = db2.snapshot().unwrap();
    db2.rebuild_index().unwrap();
    assert_eq!(none.index_status().unwrap(), stale(None));
    assert_eq!(db2.snapshot().unwrap().index_status().unwrap(), ready(2));
}

fn failed_build_is_recorded_and_repaired<E: TxnEngine>() {
    let db = open::<E>();
    let all = UnitFilter::default();
    let u = create_units(&db, &[("a", 1), ("b", 1), ("c", 1)]); // commit 1
    let head = Key::UnitHead { unit: u[1] }.encode().unwrap();
    let good = db.engine().snapshot().unwrap().get(&head).unwrap().unwrap();
    let damage = || put_all(db.engine(), vec![(head.clone(), Some(good[..good.len() - 1].to_vec()))]);
    let repair = || put_all(db.engine(), vec![(head.clone(), Some(good.clone()))]);

    // The first build fails: failed, and nothing was ever built.
    damage();
    let e = db.rebuild_index().unwrap_err();
    assert!(matches!(&e, UnitError::Storage(kerno_core::Error::Corruption(_))), "{e}");
    assert_eq!(db.index_status().unwrap(), failed(None));
    assert!(raw_entries(&db).is_empty(), "a failed build stores no entry");
    let snap = db.snapshot().unwrap();
    assert!(matches!(
        snap.indexed_scan(&all),
        Err(UnitError::IndexNotReady { state: IndexState::Failed, watermark: None })
    ));
    // The reference path opens every unit, so it cannot answer either, and
    // `query` does not hide that behind a stale answer.
    assert!(snap.query(&all).is_err());

    // Repaired and rebuilt: ready again, and exact.
    repair();
    let status = db.rebuild_index().unwrap();
    assert_eq!(status.state, IndexState::Ready);
    assert_eq!(status.watermark, Some(db.snapshot().unwrap().seq()));
    assert_clean(&db, "repaired");
    let mut model = Model::default();
    for (id, (t, v)) in u.iter().zip([("a", 1), ("b", 1), ("c", 1)]) {
        model.0.insert(*id, (t.into(), v, Lifecycle::Active));
    }
    assert_eq!(check(&db.snapshot().unwrap(), &model, "repaired").state, IndexState::Ready);

    // A failure after a good build keeps its watermark and its entries.
    let entries = raw_entries(&db);
    damage();
    assert!(db.rebuild_index().is_err());
    assert_eq!(db.index_status().unwrap(), failed(status.watermark.map(|w| w.0)));
    assert_eq!(raw_entries(&db), entries, "the failed build changed no entry");
    repair();
    assert_eq!(db.index_status().unwrap().state, IndexState::Failed, "repairing the data does not clear it");
    assert_eq!(db.rebuild_index().unwrap().state, IndexState::Ready);
    assert_clean(&db, "repaired again");
}

fn status_survives_reopen<E: TxnEngine>() {
    let vfs = SimVfs::new(1);
    let all = UnitFilter::default();
    let db = open_on::<E>(&vfs, 1);
    let u = create_units(&db, &[("a", 1), ("b", 2)]);
    let built = db.rebuild_index().unwrap();
    db.close().unwrap();

    // Ready is stored, so a clean close and reopen keep it ready.
    let db = open_on::<E>(&vfs, 100);
    assert_eq!(db.index_status().unwrap(), built);
    assert_eq!(db.snapshot().unwrap().indexed_scan(&all).unwrap().len(), 2);

    // Stale across an unclean end.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[0], None).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.index_status().unwrap().state, IndexState::Stale);
    drop(db);
    let db = open_on::<E>(&vfs, 200);
    assert_eq!(db.index_status().unwrap(), IndexStatus { state: IndexState::Stale, watermark: built.watermark });

    // Failed across a close.
    let head = Key::UnitHead { unit: u[1] }.encode().unwrap();
    let good = db.engine().snapshot().unwrap().get(&head).unwrap().unwrap();
    put_all(db.engine(), vec![(head, Some(good[..good.len() - 1].to_vec()))]);
    assert!(db.rebuild_index().is_err());
    let failure = db.index_status().unwrap();
    assert_eq!(failure, IndexStatus { state: IndexState::Failed, watermark: built.watermark });
    db.close().unwrap();
    let db = open_on::<E>(&vfs, 300);
    assert_eq!(db.index_status().unwrap(), failure);
}

// ------------------------------------------------------- planner fallback

/// E5-02: with the index in `status`, which is not `Ready`, the plan lists
/// every index candidate as unusable for that state, chooses the reference
/// path and says why, and `query` answers as the reference does (I-23, I-32).
#[track_caller]
fn assert_reference_fallback<E: Engine>(snap: &UnitSnapshot<E>, status: IndexStatus, ctx: &str) {
    let key = FingerprintKey::from_bytes([0x42; 32]);
    for f in filters() {
        let plan = snap.explain_filter(&key, &f).expect("explain_filter");
        assert_eq!(plan.index(), Some(status), "{ctx}: {f:?}");
        assert_eq!(plan.chosen().path, PlanPath::UnitsReference, "{ctx}: {f:?}");
        assert_eq!(plan.reason(), Reason::Fallback { index: status.state }, "{ctx}: {f:?}");
        for c in plan.candidates().iter().filter(|c| c.path != PlanPath::UnitsReference) {
            assert_eq!(c.usability, Usability::Unusable(Unusable::IndexNotReady(status.state)), "{ctx}: {f:?}");
        }
        let out = snap.query(&f).expect("query");
        assert_eq!(out.path, PlanPath::UnitsReference, "{ctx}: {f:?}");
        assert_eq!(out.source, QuerySource::Reference { index: status }, "{ctx}: {f:?}");
        assert_eq!(out.units, snap.reference_filter(&f).expect("reference"), "{ctx}: {f:?}");
        // The reason is the index state, not a failed check.
        assert_eq!(out.fallback, None, "{ctx}: {f:?}");
    }
}

fn planner_falls_back_when_the_index_is_not_ready<E: TxnEngine>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("b", 1), ("a", 2)]); // commit 1
    // Never built.
    assert_reference_fallback(&db.snapshot().unwrap(), stale(None), "never built");

    // Ready: the planner may use the index, and a selective type does.
    assert_eq!(db.rebuild_index().unwrap(), ready(2));
    let snap = db.snapshot().unwrap();
    let by_type = UnitFilter { unit_type: Some("a".into()), schema_version: None };
    assert_eq!(snap.query(&by_type).unwrap().path, PlanPath::IndexType);

    // Stale: a commit after the build.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[1], None).unwrap();
    cs.commit().unwrap();
    assert_reference_fallback(&db.snapshot().unwrap(), stale(Some(2)), "stale");
    // An older snapshot keeps the verdict of its own commit.
    assert_eq!(snap.query(&by_type).unwrap().path, PlanPath::IndexType);

    // Building: the handle reports it while a build waits, but a snapshot is a
    // view of stored data, so the planner on it sees the stored generation.
    assert_eq!(db.rebuild_index().unwrap(), ready(4));
    let mut held = db.begin_changeset(ChangeMeta::default()).unwrap();
    held.create(new_unit("c", 1, b"held")).unwrap();
    std::thread::scope(|s| {
        let build = s.spawn(|| db.rebuild_index());
        wait_until("the build to be reported", || db.index_status().unwrap().state == IndexState::Building);
        let during = db.snapshot().unwrap();
        assert_eq!(during.index_status().unwrap(), ready(4));
        assert_eq!(during.query(&by_type).unwrap().units, during.reference_filter(&by_type).unwrap());
        held.commit().unwrap();
        build.join().expect("the build thread").unwrap();
    });

    // Failed: repairing the data does not clear it, so the reference answers.
    let head = Key::UnitHead { unit: u[0] }.encode().unwrap();
    let good = db.engine().snapshot().unwrap().get(&head).unwrap().unwrap();
    put_all(db.engine(), vec![(head.clone(), Some(good[..good.len() - 1].to_vec()))]);
    assert!(db.rebuild_index().is_err());
    put_all(db.engine(), vec![(head, Some(good))]);
    let snap = db.snapshot().unwrap();
    let status = snap.index_status().unwrap();
    assert_eq!(status.state, IndexState::Failed);
    assert_reference_fallback(&snap, status, "failed");
}

// ---------------------------------------------------------- differential

fn differential<E: TxnEngine>(seed: u64) {
    let db = open::<E>();
    let mut rng = Rng::new(seed);
    let mut model = Model::default();
    let mut kept = Vec::new();
    let (mut ready_steps, mut stale_steps) = (0, 0);
    for step in 0..40 {
        random_step(&db, &mut rng, &mut model);
        if rng.chance(1, 3) {
            db.rebuild_index().expect("rebuild");
        }
        let snap = db.snapshot().unwrap();
        match check(&snap, &model, &format!("seed {seed} step {step}")).state {
            IndexState::Ready => ready_steps += 1,
            _ => stale_steps += 1,
        }
        if step % 6 == 0 {
            kept.push((snap, model.clone()));
        }
    }
    assert!(ready_steps >= 5 && stale_steps >= 5, "the workload must exercise both paths: {ready_steps}/{stale_steps}");
    // Older snapshots answer for their own commit, on both paths.
    for (i, (snap, m)) in kept.iter().enumerate() {
        check(snap, m, &format!("seed {seed} kept snapshot {i}"));
    }
    assert_clean(&db, "differential");
}

/// More active units than one scan page (256), so the build's head scan and
/// the entry scan both cross a page boundary.
fn differential_across_scan_pages<E: TxnEngine>() {
    let db = open::<E>();
    let mut model = Model::default();
    for chunk in 0..7usize {
        let spec: Vec<(&str, u32)> =
            (0..100).map(|i| (TYPES[(chunk * 100 + i) % 3], SCHEMAS[(chunk * 100 + i) / 3 % 3])).collect();
        for (id, (t, v)) in create_units(&db, &spec).into_iter().zip(spec) {
            model.0.insert(id, (t.into(), v, Lifecycle::Active));
        }
    }
    let ids: Vec<UnitId> = model.0.keys().copied().collect();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for (n, id) in ids.iter().enumerate() {
        if n % 5 == 0 {
            cs.archive(*id, None).unwrap();
            model.0.get_mut(id).unwrap().2 = Lifecycle::Archived;
        } else if n % 7 == 0 {
            cs.delete(*id, None).unwrap();
            model.0.get_mut(id).unwrap().2 = Lifecycle::Deleted;
        }
    }
    cs.commit().unwrap();
    assert_eq!(db.rebuild_index().unwrap().state, IndexState::Ready);
    let snap = db.snapshot().unwrap();
    assert!(model.active(&UnitFilter::default()).len() > 256);
    assert_eq!(check(&snap, &model, "700 units").state, IndexState::Ready);
    assert_clean(&db, "700 units");
}

// ----------------------------------------------------------- rebuild

/// Runs the same seeded workload on a database, `steps` change sets.
fn churn<E: TxnEngine>(db: &UnitDb<E>, rng: &mut Rng, model: &mut Model, steps: usize) {
    for _ in 0..steps {
        random_step(db, rng, model);
    }
}

fn rebuilt_equals_fresh<E: TxnEngine>(seed: u64) {
    // `a` builds in the middle of its history and rebuilds at the end; `b`
    // never had an index. Same operations, same ids (`CounterIds`).
    let (a, b) = (open::<E>(), open::<E>());
    let (mut rng_a, mut rng_b) = (Rng::new(seed), Rng::new(seed));
    let (mut model_a, mut model_b) = (Model::default(), Model::default());
    churn(&a, &mut rng_a, &mut model_a, 15);
    churn(&b, &mut rng_b, &mut model_b, 15);
    a.rebuild_index().unwrap();
    let before = raw_entries(&a);
    churn(&a, &mut rng_a, &mut model_a, 25);
    churn(&b, &mut rng_b, &mut model_b, 25);
    assert_eq!(model_a.0, model_b.0, "both databases ran the same operations");
    assert!(raw_entries(&b).is_empty());

    let rebuilt = a.rebuild_index().unwrap();
    let fresh = b.rebuild_index().unwrap();
    assert_eq!((rebuilt.state, fresh.state), (IndexState::Ready, IndexState::Ready));
    let (rebuilt_entries, fresh_entries) = (raw_entries(&a), raw_entries(&b));
    assert_eq!(rebuilt_entries, fresh_entries, "seed {seed}: a rebuilt index is a fresh one");
    assert_ne!(before, rebuilt_entries, "seed {seed}: the churn changed the active set, so this proves something");
    // ... and both are exactly the model's active units.
    let active = || model_a.0.iter().filter(|(_, (_, _, l))| *l == Lifecycle::Active);
    let mut want: Vec<(Vec<u8>, Vec<u8>)> =
        active().map(|(id, (t, v, _))| (entry_key(*id), entry_value(t, *v))).collect();
    let mut by_type: Vec<(Vec<u8>, Vec<u8>)> =
        active().map(|(id, (t, v, _))| (type_key(t, *v, *id), type_value())).collect();
    by_type.sort();
    want.extend(by_type);
    assert_eq!(fresh_entries, want, "seed {seed}: the entries are the model's active units");
    for (db, name) in [(&a, "rebuilt"), (&b, "fresh")] {
        let snap = db.snapshot().unwrap();
        assert_eq!(check(&snap, &model_a, &format!("seed {seed} {name}")).state, IndexState::Ready);
        assert_clean(db, name);
    }
    // Rebuilding what is already ready changes no entry.
    a.rebuild_index().unwrap();
    assert_eq!(raw_entries(&a), fresh_entries, "seed {seed}: rebuilding twice");
}

// -------------------------------------------------------------- verifier

fn verifier_reports_index_damage<E: TxnEngine>() {
    let db = open::<E>();
    let u = create_units(&db, &[("a", 1), ("b", 1), ("a", 2), ("c", 3)]); // commit 1
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[3], None).unwrap(); // commit 2
    cs.commit().unwrap();
    assert_eq!(db.rebuild_index().unwrap(), ready(3));
    assert_clean(&db, "built");
    assert_eq!(db.verify_units().unwrap().counts.index_entries, 3);

    let ghost = UnitId([0xEE; 16]);
    let truncated = {
        let mut v = entry_value("a", 1);
        v.pop();
        v
    };
    // The count in `IDX_META` is kept right wherever the damage changes the
    // number of entries, so that each rule has to find its damage by itself.
    let damages: Vec<(&str, Patch)> = vec![
        ("entry missing", vec![(entry_key(u[0]), None), (meta_key(), Some(meta_value(3, 2)))]),
        (
            "entry of an archived unit",
            vec![(entry_key(u[3]), Some(entry_value("c", 3))), (meta_key(), Some(meta_value(3, 4)))],
        ),
        ("entry with another type", vec![(entry_key(u[0]), Some(entry_value("b", 1)))]),
        ("entry with another schema version", vec![(entry_key(u[0]), Some(entry_value("a", 9)))]),
        (
            "entry of a unit that does not exist",
            vec![(entry_key(ghost), Some(entry_value("a", 1))), (meta_key(), Some(meta_value(3, 4)))],
        ),
        ("IDX_META missing", vec![(meta_key(), None)]),
        ("watermark after the snapshot", vec![(meta_key(), Some(meta_value(999, 3)))]),
        ("wrong entry count", vec![(meta_key(), Some(meta_value(3, 4)))]),
    ];
    // The secondary index (E4-04): every damage that leaves IDX_PRIMARY right
    // is UNIT-38 and nothing else.
    let type_damages: Vec<(&str, Patch)> = vec![
        ("type entry missing", vec![(type_key("a", 1, u[0]), None)]),
        ("type entry of an archived unit", vec![(type_key("c", 3, u[3]), Some(type_value()))]),
        (
            "type entry under another type",
            vec![(type_key("a", 1, u[0]), None), (type_key("b", 1, u[0]), Some(type_value()))],
        ),
        (
            "type entry under another schema",
            vec![(type_key("a", 1, u[0]), None), (type_key("a", 7, u[0]), Some(type_value()))],
        ),
        ("type entry of a unit that does not exist", vec![(type_key("a", 1, ghost), Some(type_value()))]),
    ];
    for (name, patch) in type_damages {
        let undo = undo_of(db.engine(), patch.iter().map(|(k, _)| k.clone()));
        put_all(db.engine(), patch);
        let report = db.verify_units().unwrap();
        assert_eq!(report.rules().into_iter().collect::<Vec<_>>(), ["UNIT-38"], "{name}: {:?}", report.findings);
        put_all(db.engine(), undo);
        assert_clean(&db, name);
    }
    // A type entry that does not decode is a structure finding only.
    let undo = undo_of(db.engine(), [type_key("a", 1, u[0])]);
    put_all(db.engine(), vec![(type_key("a", 1, u[0]), Some(vec![1, 0, 0]))]);
    let report = db.verify_units().unwrap();
    assert_eq!(report.rules().into_iter().collect::<Vec<_>>(), ["UNIT-10"], "{:?}", report.findings);
    put_all(db.engine(), undo);
    assert_clean(&db, "type entry restored");

    for (name, patch) in damages {
        let undo = undo_of(db.engine(), patch.iter().map(|(k, _)| k.clone()));
        put_all(db.engine(), patch);
        let report = db.verify_units().unwrap();
        assert!(report.rules().contains("UNIT-37"), "{name}: {:?}", report.findings);
        assert!(report.findings.iter().all(|f| f.class() == ErrorClass::Corruption), "{name}");
        put_all(db.engine(), undo);
        assert_clean(&db, name);
    }

    // An entry that does not decode is one structure finding, not noise.
    let undo = undo_of(db.engine(), [entry_key(u[0])]);
    put_all(db.engine(), vec![(entry_key(u[0]), Some(truncated))]);
    let report = db.verify_units().unwrap();
    assert_eq!(report.rules().into_iter().collect::<Vec<_>>(), ["UNIT-9"], "{:?}", report.findings);
    put_all(db.engine(), undo);
    assert_clean(&db, "entry restored");

    // Messages carry ids and numbers, never a unit type (IU-10).
    put_all(db.engine(), vec![(entry_key(u[0]), Some(entry_value("sentinel-type", 1)))]);
    let report = db.verify_units().unwrap();
    assert!(report.findings.iter().all(|f| !f.to_string().contains("sentinel")), "{:?}", report.findings);
    put_all(db.engine(), vec![(entry_key(u[0]), Some(entry_value("a", 1)))]);

    // Staleness is not damage: units that changed or appeared after the
    // watermark may disagree with the entries, and the verifier says nothing.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(u[0], upd(b"x", Some(5)), None).unwrap();
    cs.archive(u[1], None).unwrap();
    let fresh = cs.create(new_unit("d", 1, b"new")).unwrap();
    cs.commit().unwrap();
    assert_eq!(db.index_status().unwrap().state, IndexState::Stale);
    assert_clean(&db, "a stale index");
    // But an entry for a unit created after the watermark is damage.
    put_all(db.engine(), vec![(entry_key(fresh), Some(entry_value("d", 1)))]);
    let report = db.verify_units().unwrap();
    assert!(report.rules().contains("UNIT-37"), "{:?}", report.findings);
}

/// The index path checks each unit it returns against the entry; what it
/// cannot see (a unit missing from the index, or an entry that its filter
/// skips) only the verifier and a rebuild deal with.
fn index_path_refuses_what_it_can_see<E: TxnEngine>() {
    let db = open::<E>();
    let all = UnitFilter::default();
    let by_type = |t: &str| UnitFilter { unit_type: Some(t.into()), schema_version: None };
    let u = create_units(&db, &[("a", 1), ("b", 1), ("c", 1)]);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[2], None).unwrap();
    cs.commit().unwrap();
    db.rebuild_index().unwrap();

    // A forged ready index: the patch and an `IDX_META` that claims the
    // commit the patch itself makes (a plain raw commit would make it stale).
    let forge = |mut patch: Patch, entries: u64| {
        let next = seq(&db) + 1;
        patch.push((meta_key(), Some(meta_value(next, entries))));
        put_all(db.engine(), patch);
        assert_eq!(db.index_status().unwrap().state, IndexState::Ready);
    };

    // An entry that disagrees with the head is refused when the filter reaches it.
    forge(vec![(entry_key(u[0]), Some(entry_value("b", 1)))], 2);
    let snap = db.snapshot().unwrap();
    let e = snap.indexed_scan_via(IndexPath::Primary, &by_type("b")).unwrap_err();
    assert!(matches!(&e, UnitError::Storage(kerno_core::Error::Corruption(_))), "{e}");
    assert!(db.verify_units().unwrap().rules().contains("UNIT-37"));
    // ... but a filter that skips it silently loses the unit: this is what
    // the differential test would flag, and the verifier reports.
    let primary = |f: &UnitFilter| snap.indexed_scan_via(IndexPath::Primary, f).unwrap();
    assert_ne!(primary(&by_type("a")), snap.reference_filter(&by_type("a")).unwrap());
    assert_eq!(db.rebuild_index().unwrap().state, IndexState::Ready);
    assert_eq!(
        check(&db.snapshot().unwrap(), &model_of(&u, &[("a", 1), ("b", 1)]), "rebuilt").state,
        IndexState::Ready
    );

    // The entry of an archived unit.
    forge(vec![(entry_key(u[2]), Some(entry_value("c", 1)))], 3);
    let snap = db.snapshot().unwrap();
    assert!(snap.indexed_scan(&all).is_err());
    assert!(
        snap.indexed_scan_via(IndexPath::Primary, &by_type("a")).is_ok(),
        "entries the filter skips are not opened"
    );
    // The entry of a unit that does not exist.
    forge(vec![(entry_key(u[2]), None), (entry_key(UnitId([0xEE; 16])), Some(entry_value("a", 1)))], 3);
    assert!(db.snapshot().unwrap().indexed_scan(&all).is_err());
    // An entry that is gone.
    forge(vec![(entry_key(UnitId([0xEE; 16])), None), (entry_key(u[1]), None)], 1);
    assert_ne!(
        db.snapshot().unwrap().indexed_scan(&all).unwrap(),
        db.snapshot().unwrap().reference_filter(&all).unwrap()
    );
    assert!(db.verify_units().unwrap().rules().contains("UNIT-37"));

    // The same for IDX_TYPE (E4-04): an entry whose key disagrees with the
    // head is refused; a missing one silently loses the unit, which the
    // verifier reports.
    assert_eq!(db.rebuild_index().unwrap().state, IndexState::Ready);
    forge(vec![(type_key("a", 1, u[0]), None), (type_key("b", 1, u[0]), Some(type_value()))], 2);
    let snap = db.snapshot().unwrap();
    let e = snap.indexed_scan(&by_type("b")).unwrap_err();
    assert!(matches!(&e, UnitError::Storage(kerno_core::Error::Corruption(_))), "{e}");
    assert_ne!(snap.indexed_scan(&by_type("a")).unwrap(), snap.reference_filter(&by_type("a")).unwrap());
    assert!(db.verify_units().unwrap().rules().contains("UNIT-38"));
    forge(vec![(type_key("b", 1, u[0]), None), (type_key("c", 1, u[2]), Some(type_value()))], 2);
    let e = db.snapshot().unwrap().indexed_scan(&by_type("c")).unwrap_err();
    assert!(matches!(&e, UnitError::Storage(kerno_core::Error::Corruption(_))), "{e}");

    // A rebuild repairs all of it.
    assert_eq!(db.rebuild_index().unwrap().state, IndexState::Ready);
    assert_clean(&db, "rebuilt after damage");
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.indexed_scan(&all).unwrap(), snap.reference_filter(&all).unwrap());
}

fn model_of(units: &[UnitId], spec: &[(&str, u32)]) -> Model {
    Model(
        units.iter().zip(spec).map(|(id, (t, v))| (*id, ((*t).into(), *v, Lifecycle::Active))).collect(),
        Revs::default(),
    )
}

// ----------------------------------------------------------------- crash

const SWEEP_STEPS: u64 = 5;

/// The `n`-th (from 1) unit of step 1 ([`StepIds`]).
fn sweep_unit(n: u64) -> UnitId {
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&((1u64 << 16) + n).to_be_bytes());
    UnitId(id)
}

/// Step `k` commits exactly one change set, so it is commit sequence `k`:
/// 1 creates three units, 2 builds, 3 changes a schema version, 4 rebuilds,
/// 5 archives a unit.
fn sweep_step<E: TxnEngine>(db: &UnitDb<E>, ids: &StepIds, k: u64) -> Result<(), UnitError> {
    ids.start_step(k);
    match k {
        1 => {
            let mut cs = db.begin_changeset(ChangeMeta::default())?;
            for (t, v) in [("a", 1), ("b", 1), ("a", 1)] {
                cs.create(new_unit(t, v, b"p"))?;
            }
            cs.commit().map(|_| ())
        }
        2 | 4 => db.rebuild_index().map(|_| ()),
        3 => {
            let mut cs = db.begin_changeset(ChangeMeta::default())?;
            cs.update(sweep_unit(1), upd(b"q", Some(2)), None)?;
            cs.commit().map(|_| ())
        }
        _ => {
            let mut cs = db.begin_changeset(ChangeMeta::default())?;
            cs.archive(sweep_unit(2), None)?;
            cs.commit().map(|_| ())
        }
    }
}

/// The stored status after exactly `s` whole steps.
fn sweep_status(s: u64) -> IndexStatus {
    match s {
        0 | 1 => stale(None),
        2 => ready(2),
        3 => stale(Some(2)),
        4 => ready(4),
        _ => stale(Some(4)),
    }
}

/// The active units after `s` whole steps, from the steps above.
fn sweep_model(s: u64) -> Model {
    let mut m = Model::default();
    if s >= 1 {
        let v1 = if s >= 3 { 2 } else { 1 };
        let l2 = if s >= 5 { Lifecycle::Archived } else { Lifecycle::Active };
        m.0.insert(sweep_unit(1), ("a".into(), v1, Lifecycle::Active));
        m.0.insert(sweep_unit(2), ("b".into(), 1, l2));
        m.0.insert(sweep_unit(3), ("a".into(), 1, Lifecycle::Active));
    }
    m
}

fn sweep_disk(point: u64) -> SimVfs {
    let disk = SimVfs::new(point.wrapping_mul(1_000_003) ^ 0x1D);
    if point % 2 == 1 {
        disk.set_tear_unit(1);
    }
    disk
}

/// Runs the steps until the device fails: `(acked, attempted)` steps.
fn run_sweep<E: Target>(disk: &SimVfs, cfg: &UnitConfig, ids: &StepIds) -> (u64, u64) {
    let Ok(db) = open_units::<E>(disk.as_vfs(), cfg) else { return (0, 0) };
    for k in 1..=SWEEP_STEPS {
        if sweep_step(&db, ids, k).is_err() {
            return (k - 1, k);
        }
    }
    (SWEEP_STEPS, SWEEP_STEPS)
}

fn index_every_point<E: Target>(kind: CrashKind) {
    let name = format!("index_every_point {} {kind:?}", E::NAME);
    let ids = StepIds::new();
    let cfg = unit_config(&ids, Durability::Sync);
    let dry = SimVfs::new(0);
    assert_eq!(run_sweep::<E>(&dry, &cfg, &ids), (SWEEP_STEPS, SWEEP_STEPS), "the dry run must complete");
    let total = dry.mutating_ops() + 5;
    for point in 1..=total {
        let ctx = format!("{name}: crash before mutating op #{point} of {total}");
        let disk = sweep_disk(point);
        disk.crash_after(point, kind);
        let (acked, attempted) = run_sweep::<E>(&disk, &cfg, &ids);
        let image = disk.take_crash_image().unwrap_or_else(|| disk.crash_now(kind));
        let db = open_units::<E>(image.as_vfs(), &cfg).unwrap_or_else(|e| panic!("{ctx}: recovery: {e}"));
        let s = seq(&db);
        assert!(acked <= s && s <= attempted, "{ctx}: recovered {s} steps, expected {acked}..={attempted}");

        // I-23 after a crash: the status is the one of the recovered commit,
        // so an interrupted build left the previous generation, whole.
        let snap = db.snapshot().unwrap_or_else(|e| panic!("{ctx}: snapshot: {e}"));
        assert_eq!(snap.index_status().unwrap(), sweep_status(s), "{ctx}: status after {s} steps");
        check(&snap, &sweep_model(s), &ctx);
        verified(&db, &ctx);

        // The recovered database builds again, and the index is exact.
        let rebuilt = db.rebuild_index().unwrap_or_else(|e| panic!("{ctx}: rebuild after recovery: {e}"));
        assert_eq!(rebuilt, ready(s + 1), "{ctx}");
        check(&db.snapshot().unwrap(), &sweep_model(s), &format!("{ctx}, rebuilt"));
        verified(&db, &format!("{ctx}, rebuilt"));
    }
}

macro_rules! engine_tests {
    ($m:ident, $E:ty) => {
        mod $m {
            type E = $E;
            use super::*;
            #[test]
            fn states_and_watermark() {
                super::states_and_watermark::<E>();
            }
            #[test]
            fn building_is_reported_while_a_build_waits() {
                super::building_is_reported_while_a_build_waits::<E>();
            }
            #[test]
            fn snapshots_keep_their_own_status() {
                super::snapshots_keep_their_own_status::<E>();
            }
            #[test]
            fn failed_build_is_recorded_and_repaired() {
                super::failed_build_is_recorded_and_repaired::<E>();
            }
            #[test]
            fn status_survives_reopen() {
                super::status_survives_reopen::<E>();
            }
            #[test]
            fn planner_falls_back_when_the_index_is_not_ready() {
                super::planner_falls_back_when_the_index_is_not_ready::<E>();
            }
            #[test]
            fn differential_seeded() {
                for seed in 1..=6 {
                    super::differential::<E>(seed);
                }
            }
            #[test]
            fn differential_across_scan_pages() {
                super::differential_across_scan_pages::<E>();
            }
            #[test]
            fn rebuilt_equals_fresh() {
                for seed in 1..=6 {
                    super::rebuilt_equals_fresh::<E>(seed);
                }
            }
            #[test]
            fn verifier_reports_index_damage() {
                super::verifier_reports_index_damage::<E>();
            }
            #[test]
            fn index_path_refuses_what_it_can_see() {
                super::index_path_refuses_what_it_can_see::<E>();
            }
            #[test]
            fn index_every_point_power_loss() {
                super::index_every_point::<E>(CrashKind::PowerLoss);
            }
            #[test]
            fn index_every_point_process_kill() {
                super::index_every_point::<E>(CrashKind::ProcessKill);
            }
        }
    };
}

engine_tests!(emulated_log, EmuLog);
engine_tests!(kerno, KernoDb);
