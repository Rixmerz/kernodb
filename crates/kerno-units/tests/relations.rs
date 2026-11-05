//! E4-02: declared relations (ADR-0014, option 2) over `Emulated<LogEngine>`
//! and `KernoDb`.
//!
//! * `provenance_*` — provenance is mandatory and checked; declared is never
//!   confused with inferred (I-39); a damaged stored provenance is refused
//!   on read and by `verify_units`.
//! * `archive_and_delete_are_rejected_with_live_relations` — criterion 4.
//! * `abort_leaves_no_relation` / `relation_only_changeset_commits` — the
//!   record and its two index entries commit together or not at all (I-21).
//! * `differential_*` — the adjacency path equals the reference traversal on
//!   the same snapshot after every step of a seeded workload (I-32).
//! * `verifier_*` — a relation without its index entries, or an index entry
//!   without its relation, is reported as UNIT-36.
//!
//! The expectation is tracked by the test (a map of live relations), never
//! read back from the layer.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use kerno_core::WriteBatch;
use kerno_core::contract::{Engine, Options, Snapshot, TxnEngine};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::rng::Rng;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{ChangeSetRecord, Key, RelationRecord};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn open_with<E: TxnEngine>(max_ops: Option<usize>) -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    if let Some(n) = max_ops {
        c.max_changeset_ops = n;
    }
    UnitDb::<E>::open(SimVfs::new(1).as_vfs(), Path::new("/db"), &c).expect("open")
}

fn open<E: TxnEngine>() -> UnitDb<E> {
    open_with(None)
}

fn new_unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: Vec::new(), payload: payload.to_vec() }
}

fn units<E: TxnEngine>(db: &UnitDb<E>, n: usize) -> Vec<UnitId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).expect("begin");
    let ids = (0..n).map(|i| cs.create(new_unit(&[i as u8])).expect("create")).collect();
    cs.commit().expect("commit");
    ids
}

fn relate<E: TxnEngine>(
    db: &UnitDb<E>,
    from: UnitId,
    to: UnitId,
    ty: &str,
    p: RelationProvenance,
) -> Result<RelationId, UnitError> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    let id = cs.create_relation(from, to, ty, p)?;
    cs.commit()?;
    Ok(id)
}

#[track_caller]
fn assert_clean<E: TxnEngine>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().expect("verify");
    assert!(r.is_clean(), "{ctx}: verify_units found {:?}", r.findings);
}

/// Index path equals the reference path for every unit, direction and type.
#[track_caller]
fn assert_equivalent<E: Engine>(snap: &UnitSnapshot<E>, units: &[UnitId], types: &[&str]) {
    for &u in units {
        for dir in [Direction::Out, Direction::In] {
            for ty in std::iter::once(None).chain(types.iter().map(|t| Some(*t))) {
                let idx = snap.relations_of(u, dir, ty).expect("relations_of");
                let reference = snap.reference_relations_of(u, dir, ty).expect("reference_relations_of");
                assert_eq!(idx, reference, "unit {u} {dir:?} {ty:?}");
            }
        }
    }
}

// ---------------------------------------------------------------- basics

fn create_and_read<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 3);
    let r1 = relate(&db, u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    let r2 = relate(&db, u[0], u[2], "cites", RelationProvenance::Declared).unwrap();
    let r3 = relate(&db, u[0], u[1], "cites", RelationProvenance::Declared).unwrap(); // parallel
    let r4 = relate(&db, u[2], u[0], "refines", RelationProvenance::Declared).unwrap();
    let r5 = relate(&db, u[1], u[1], "cites", RelationProvenance::Declared).unwrap(); // self
    let snap = db.snapshot().unwrap();

    let rel = snap.get_relation(r1).unwrap().expect("relation");
    assert_eq!((rel.from, rel.to, rel.relation_type.as_str()), (u[0], u[1], "cites"));
    assert!(rel.provenance.is_declared());
    assert_eq!(snap.get_relation(RelationId([9; 16])).unwrap(), None);

    let out: Vec<_> = snap.relations_of(u[0], Direction::Out, None).unwrap().iter().map(|r| r.id).collect();
    assert_eq!(out, vec![r1, r2, r3]);
    let inc: Vec<_> = snap.relations_of(u[0], Direction::In, None).unwrap().iter().map(|r| r.id).collect();
    assert_eq!(inc, vec![r4]);
    let inc1: Vec<_> = snap.relations_of(u[1], Direction::In, Some("cites")).unwrap().iter().map(|r| r.id).collect();
    assert_eq!(inc1, vec![r1, r3, r5]);
    assert!(snap.relations_of(u[1], Direction::Out, Some("refines")).unwrap().is_empty());
    // A type that is a prefix of another matches only its own entries.
    assert!(snap.relations_of(u[0], Direction::Out, Some("cite")).unwrap().is_empty());
    assert!(snap.relations_of(u[0], Direction::Out, Some("")).unwrap().is_empty());
    assert_equivalent(&snap, &u, &["cites", "refines", "cite"]);
    assert_clean(&db, "after relations");

    // Deleting a relation removes the record and both entries.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(r2).unwrap();
    assert!(matches!(cs.delete_relation(r2), Err(UnitError::RelationNotFound { .. })));
    cs.commit().unwrap();
    let after = db.snapshot().unwrap();
    assert_eq!(after.get_relation(r2).unwrap(), None);
    assert_eq!(after.relations_of(u[0], Direction::Out, None).unwrap().len(), 2);
    assert_eq!(after.relations_of(u[2], Direction::In, None).unwrap().len(), 0);
    // The earlier snapshot still sees it.
    assert!(snap.get_relation(r2).unwrap().is_some());
    assert_equivalent(&after, &u, &["cites", "refines"]);
    assert_clean(&db, "after delete_relation");
}

fn invalid_input_is_rejected<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let d = RelationProvenance::Declared;
    for bad in ["", "a\0b", &"x".repeat(65)] {
        let e = cs.create_relation(u[0], u[1], bad, d.clone()).unwrap_err();
        assert!(matches!(e, UnitError::InvalidArgument(_)), "{bad:?}: {e}");
    }
    let e = cs.create_relation(u[0], UnitId([0xEE; 16]), "t", d.clone()).unwrap_err();
    assert!(matches!(e, UnitError::NotFound { .. }), "{e}");
    cs.commit().unwrap(); // nothing was written
    assert_eq!(db.snapshot().unwrap().reference_relations(|_| true).unwrap().len(), 0);

    // Archived and deleted units cannot take a relation.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[1], None).unwrap();
    cs.commit().unwrap();
    assert!(matches!(relate(&db, u[0], u[1], "t", d.clone()), Err(UnitError::InvalidArgument(_))));
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(u[1], None).unwrap();
    cs.commit().unwrap();
    assert!(matches!(relate(&db, u[0], u[1], "t", d), Err(UnitError::Deleted { .. })));
    assert_clean(&db, "after refusals");
}

// ------------------------------------------------------------ provenance

fn provenance_is_stored_and_checked<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let kinds = [
        RelationProvenance::Declared,
        RelationProvenance::Imported { source: b"wiki-export".to_vec() },
        RelationProvenance::Algorithmic { origin: b"link-extractor-1".to_vec() },
        RelationProvenance::Inferred { origin: b"embedding-sim".to_vec(), confidence_permille: 640 },
        RelationProvenance::Inferred { origin: b"o".to_vec(), confidence_permille: 1000 },
        RelationProvenance::Inferred { origin: b"o".to_vec(), confidence_permille: 0 },
    ];
    let ids: Vec<_> = kinds.iter().map(|p| relate(&db, u[0], u[1], "links", p.clone()).unwrap()).collect();
    let snap = db.snapshot().unwrap();
    for (id, p) in ids.iter().zip(&kinds) {
        assert_eq!(&snap.get_relation(*id).unwrap().unwrap().provenance, p);
    }

    // Provenance the layer must refuse to write.
    let bad = [
        RelationProvenance::Imported { source: Vec::new() },
        RelationProvenance::Algorithmic { origin: Vec::new() },
        RelationProvenance::Inferred { origin: Vec::new(), confidence_permille: 10 },
        RelationProvenance::Inferred { origin: b"o".to_vec(), confidence_permille: 1001 },
        RelationProvenance::Imported { source: vec![b'x'; 257] },
    ];
    for p in bad {
        let e = relate(&db, u[0], u[1], "links", p.clone()).unwrap_err();
        assert!(matches!(e, UnitError::InvalidArgument(_)), "{p:?}: {e}");
    }
    assert_eq!(db.snapshot().unwrap().reference_relations(|_| true).unwrap().len(), kinds.len());
    assert_clean(&db, "provenance");
}

/// I-39: an inferred relation is never read as a declared one, by the index
/// path or by the reference path, and the record format cannot express a
/// declared relation that carries an origin or a confidence.
fn declared_is_never_confused_with_inferred<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let declared = relate(&db, u[0], u[1], "supports", RelationProvenance::Declared).unwrap();
    let inferred = relate(
        &db,
        u[0],
        u[1],
        "supports",
        RelationProvenance::Inferred { origin: b"model".to_vec(), confidence_permille: 900 },
    )
    .unwrap();
    let snap = db.snapshot().unwrap();
    for rels in [
        snap.relations_of(u[0], Direction::Out, Some("supports")).unwrap(),
        snap.reference_relations_of(u[0], Direction::Out, Some("supports")).unwrap(),
    ] {
        assert_eq!(rels.len(), 2, "same ends and type, both kept apart");
        let only_declared: Vec<_> = rels.iter().filter(|r| r.provenance.is_declared()).map(|r| r.id).collect();
        assert_eq!(only_declared, vec![declared]);
        let inf = rels.iter().find(|r| r.id == inferred).unwrap();
        assert_eq!(inf.provenance.kind(), ProvenanceKind::Inferred);
        assert!(!inf.provenance.is_declared());
    }

    // Byte level: flip the kind of a stored record to `Declared` while its
    // origin and confidence stay: the decoder refuses the record.
    let raw = snap.reference_relations(|_| true).unwrap();
    assert_eq!(raw.len(), 2);
    let rec = RelationRecord {
        from: u[0],
        to: u[1],
        relation_type: "supports".into(),
        created_seq: CommitSeq(2),
        provenance: RelationProvenance::Inferred { origin: b"model".to_vec(), confidence_permille: 900 },
    };
    let mut bytes = rec.encode().unwrap();
    assert_eq!(RelationRecord::decode(&bytes).unwrap(), rec);
    let kind_at = bytes.len() - 2 - 4 - rec_origin_len(&rec) - 1;
    assert_eq!(bytes[kind_at], ProvenanceKind::Inferred.to_byte());
    bytes[kind_at] = ProvenanceKind::Declared.to_byte();
    assert!(RelationRecord::decode(&bytes).is_err(), "declared kind with an origin and a confidence");
    bytes[kind_at] = ProvenanceKind::Imported.to_byte();
    assert!(RelationRecord::decode(&bytes).is_err(), "imported kind with a confidence");
    // A record cut before its provenance is not a relation without one.
    let full = rec.encode().unwrap();
    assert!(RelationRecord::decode(&full[..kind_at]).is_err());
}

/// I-39 on stored data: a `REL` whose provenance was damaged on disk is
/// refused by `get_relation`, by the adjacency and reference paths and by
/// `verify_units`; it is never read back as another provenance.
fn stored_provenance_corruption_is_detected<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let inferred = RelationProvenance::Inferred { origin: b"model".to_vec(), confidence_permille: 900 };
    let id = relate(&db, u[0], u[1], "supports", inferred.clone()).unwrap();
    assert_clean(&db, "before damage");
    let key = Key::Relation { edge: id }.encode().unwrap();
    let good = db.engine().snapshot().unwrap().get(&key).unwrap().unwrap();
    let rec = RelationRecord::decode(&good).unwrap();
    assert_eq!(rec.provenance, inferred);
    let kind_at = good.len() - 2 - 4 - rec_origin_len(&rec) - 1;
    assert_eq!(good[kind_at], ProvenanceKind::Inferred.to_byte());

    for (name, kind) in [
        ("inferred stored as declared", ProvenanceKind::Declared),
        ("inferred stored as imported", ProvenanceKind::Imported),
        ("inferred stored as an unknown kind", ProvenanceKind::Inferred),
    ] {
        let mut bad = good.clone();
        bad[kind_at] = if name.contains("unknown") { 9 } else { kind.to_byte() };
        put_all(db.engine(), vec![(key.clone(), Some(bad))]);
        let snap = db.snapshot().unwrap();
        let e = snap.get_relation(id).unwrap_err();
        assert!(e.to_string().contains("UNIT-13"), "{name}: {e}");
        assert!(snap.relations_of(u[0], Direction::Out, None).is_err(), "{name}: adjacency path");
        assert!(snap.reference_relations(|_| true).is_err(), "{name}: reference path");
        drop(snap);
        let report = db.verify_units().unwrap();
        assert!(report.rules().contains("UNIT-13"), "{name}: {:?}", report.findings);
    }

    // Origin emptied under an inferred kind: also refused.
    let mut emptied = RelationRecord { provenance: RelationProvenance::Declared, ..rec.clone() }.encode().unwrap();
    let k = emptied.len() - 2 - 4 - 1;
    emptied[k] = ProvenanceKind::Inferred.to_byte();
    put_all(db.engine(), vec![(key.clone(), Some(emptied))]);
    assert!(db.snapshot().unwrap().get_relation(id).is_err(), "inferred without origin");
    assert!(db.verify_units().unwrap().rules().contains("UNIT-13"));

    put_all(db.engine(), vec![(key, Some(good))]);
    assert_eq!(db.snapshot().unwrap().get_relation(id).unwrap().unwrap().provenance, inferred);
    assert_clean(&db, "provenance restored");
}

fn rec_origin_len(r: &RelationRecord) -> usize {
    match &r.provenance {
        RelationProvenance::Declared => 0,
        RelationProvenance::Imported { source: o }
        | RelationProvenance::Algorithmic { origin: o }
        | RelationProvenance::Inferred { origin: o, .. } => o.len(),
    }
}

// ------------------------------------------- archive / delete (criterion 4)

fn archive_and_delete_are_rejected_with_live_relations<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 3);
    let out_rel = relate(&db, u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    let before = db.snapshot().unwrap();

    // Either end, archive and delete: rejected, with the unit and a relation named.
    for (unit, rel) in [(u[0], out_rel), (u[1], out_rel)] {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let e = cs.archive(unit, None).unwrap_err();
        assert!(
            matches!(&e, UnitError::UnitHasRelations { unit: x, relation } if *x == unit && *relation == rel),
            "{e}"
        );
        let text = e.to_string();
        assert!(
            text.contains(&unit.to_string()) && text.contains(&rel.to_string()) && text.contains("relations"),
            "{text}"
        );
        let e = cs.delete(unit, None).unwrap_err();
        assert!(matches!(e, UnitError::UnitHasRelations { .. }), "{e}");
        // The rejection leaves the change set usable.
        cs.update(u[2], UnitUpdate { payload: b"x".to_vec(), schema_version: None, metadata: None }, None).unwrap();
        cs.commit().unwrap();
    }
    let now = db.snapshot().unwrap();
    for id in [u[0], u[1]] {
        assert_eq!(now.get_unit(id).unwrap().unwrap().lifecycle, Lifecycle::Active);
    }
    assert_eq!(now.relations_of(u[0], Direction::Out, None).unwrap().len(), 1);
    assert_eq!(before.get_unit(u[0]).unwrap().unwrap().lifecycle, Lifecycle::Active);

    // A unit without relations is unaffected.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u[2], None).unwrap();
    cs.commit().unwrap();

    // Deleting the relation first, in the same change set, then the unit.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(out_rel).unwrap();
    cs.archive(u[0], None).unwrap();
    cs.delete(u[1], None).unwrap();
    cs.commit().unwrap();
    let end = db.snapshot().unwrap();
    assert_eq!(end.get_unit(u[0]).unwrap().unwrap().lifecycle, Lifecycle::Archived);
    assert_eq!(end.get_unit(u[1]).unwrap().unwrap().lifecycle, Lifecycle::Deleted);
    assert!(end.reference_relations(|_| true).unwrap().is_empty());
    assert_clean(&db, "after rejection and cleanup");
}

// ------------------------------------------------------------ atomicity

fn abort_leaves_no_relation<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.create_relation(u[0], u[1], "t", RelationProvenance::Declared).unwrap();
    // The change set sees its own relation through the guard of delete.
    assert!(matches!(cs.archive(u[0], None), Err(UnitError::UnitHasRelations { relation, .. }) if relation == id));
    cs.abort();
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.get_relation(id).unwrap(), None);
    assert!(snap.relations_of(u[0], Direction::Out, None).unwrap().is_empty());
    assert!(snap.relations_of(u[1], Direction::In, None).unwrap().is_empty());
    assert_clean(&db, "after abort");
}

fn relation_only_changeset_commits<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let seq0 = db.snapshot().unwrap().seq();
    let mut cs = db.begin_changeset(ChangeMeta { reason: Some("link".into()), ..ChangeMeta::default() }).unwrap();
    cs.create_relation(u[0], u[1], "t", RelationProvenance::Declared).unwrap();
    let info = cs.commit().unwrap();
    assert!(info.commit_seq > seq0, "a relation-only change set is a commit");
    // An empty one still is not.
    let cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let info2 = cs.commit().unwrap();
    assert_eq!(info2.commit_seq, info.commit_seq);
    let snap = db.snapshot().unwrap();
    let rel = &snap.relations_of(u[0], Direction::Out, None).unwrap()[0];
    assert_eq!(rel.created_seq, info.commit_seq);
    assert_clean(&db, "relation-only change set");

    // It is recorded as `Relation`, so an `Application` change set that lost
    // its members is still corruption (review finding F1).
    let ck = Key::ChangeSet { seq: info.commit_seq }.encode().unwrap();
    let raw = db.engine().snapshot().unwrap().get(&ck).unwrap().unwrap();
    let mut rec = ChangeSetRecord::decode(&raw).unwrap();
    assert_eq!(rec.purpose, Purpose::Relation);
    rec.purpose = Purpose::Application;
    put_all(db.engine(), vec![(ck.clone(), Some(rec.encode().unwrap()))]);
    let report = db.verify_units().unwrap();
    assert!(report.rules().contains("UNIT-33"), "{:?}", report.findings);
    rec.purpose = Purpose::Relation;
    put_all(db.engine(), vec![(ck, Some(rec.encode().unwrap()))]);
    assert_clean(&db, "relation purpose restored");
}

fn relations_count_against_the_changeset_limit<E: TxnEngine>() {
    let db = open_with::<E>(Some(3));
    let u = units(&db, 2);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for _ in 0..3 {
        cs.create_relation(u[0], u[1], "t", RelationProvenance::Declared).unwrap();
    }
    let e = cs.create_relation(u[0], u[1], "t", RelationProvenance::Declared).unwrap_err();
    assert!(matches!(e, UnitError::ChangeSetTooLarge { .. }), "{e}");
    cs.commit().unwrap();
    assert_eq!(db.snapshot().unwrap().reference_relations(|_| true).unwrap().len(), 3);
    assert_clean(&db, "at the limit");
}

// ----------------------------------------------------------- differential

type Model = BTreeMap<RelationId, (UnitId, UnitId, &'static str)>;

fn differential<E: TxnEngine>(seed: u64) {
    const TYPES: [&str; 3] = ["cites", "cite", "refines"];
    let db = open::<E>();
    let mut rng = Rng::new(seed);
    let u = units(&db, 12);
    let mut model = Model::new();
    let mut snaps = Vec::new();
    for step in 0..40 {
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        let mut staged = model.clone();
        for _ in 0..rng.range(1, 8) {
            if !staged.is_empty() && rng.chance(1, 3) {
                let idx = rng.below(staged.len() as u64) as usize;
                let id = *staged.keys().nth(idx).unwrap();
                cs.delete_relation(id).unwrap();
                staged.remove(&id);
            } else {
                let (f, t) = (u[rng.below(12) as usize], u[rng.below(12) as usize]);
                let ty = TYPES[rng.below(3) as usize];
                let p = if rng.chance(1, 2) {
                    RelationProvenance::Declared
                } else {
                    RelationProvenance::Inferred { origin: b"o".to_vec(), confidence_permille: rng.below(1001) as u16 }
                };
                let id = cs.create_relation(f, t, ty, p).unwrap();
                staged.insert(id, (f, t, ty));
            }
        }
        if rng.chance(1, 5) {
            cs.abort();
        } else {
            cs.commit().unwrap();
            model = staged;
        }
        let snap = db.snapshot().unwrap();
        assert_equivalent(&snap, &u, &TYPES);
        let all: Vec<_> = snap.reference_relations(|_| true).unwrap();
        let got: Model = all
            .iter()
            .map(|r| (r.id, (r.from, r.to, *TYPES.iter().find(|t| **t == r.relation_type).unwrap())))
            .collect();
        assert_eq!(got, model, "step {step}");
        if step % 8 == 0 {
            snaps.push((snap, model.clone()));
        }
    }
    // Older snapshots still answer for their own commit, on both paths.
    for (snap, m) in &snaps {
        assert_eq!(snap.reference_relations(|_| true).unwrap().len(), m.len());
        assert_equivalent(snap, &u, &TYPES);
    }
    assert_clean(&db, "differential");
}

/// More relations on one unit than one scan page (256), so the paged scans of
/// both paths cross a page boundary.
fn differential_across_scan_pages<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 3);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for i in 0..600usize {
        let (from, to, ty) = if i % 3 == 0 { (u[0], u[1], "a") } else { (u[2], u[0], "b") };
        cs.create_relation(from, to, ty, RelationProvenance::Declared).unwrap();
    }
    cs.commit().unwrap();
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.relations_of(u[0], Direction::In, None).unwrap().len(), 400);
    assert_eq!(snap.reference_relations(|_| true).unwrap().len(), 600);
    assert_equivalent(&snap, &u, &["a", "b"]);
    assert_clean(&db, "600 relations");
}

// -------------------------------------------------------------- verifier

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
    db.commit(&b).unwrap();
}

fn verifier_reports_index_damage<E: TxnEngine>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let id = relate(&db, u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    assert_clean(&db, "before damage");
    let report = db.verify_units().unwrap();
    assert_eq!(report.counts.relations, 1);

    let out = Key::RelationOut { unit: u[0], relation_type: "cites".into(), edge: id }.encode().unwrap();
    let inn = Key::RelationIn { unit: u[1], relation_type: "cites".into(), edge: id }.encode().unwrap();
    let rel = Key::Relation { edge: id }.encode().unwrap();
    let snap = db.engine().snapshot().unwrap();
    let (out_v, in_v, rel_v) =
        (snap.get(&out).unwrap().unwrap(), snap.get(&inn).unwrap().unwrap(), snap.get(&rel).unwrap().unwrap());
    drop(snap);
    let damages: Vec<(&str, Patch)> = vec![
        ("REL_OUT missing", vec![(out.clone(), None)]),
        ("REL_IN missing", vec![(inn.clone(), None)]),
        ("REL missing", vec![(rel.clone(), None)]),
        ("both entries missing", vec![(out.clone(), None), (inn.clone(), None)]),
    ];
    for (name, patch) in damages {
        let undo: Vec<_> = patch
            .iter()
            .map(|(k, _)| {
                let v = if *k == out {
                    &out_v
                } else if *k == inn {
                    &in_v
                } else {
                    &rel_v
                };
                (k.clone(), Some(v.clone()))
            })
            .collect();
        put_all(db.engine(), patch);
        let report = db.verify_units().unwrap();
        assert!(report.rules().contains("UNIT-36"), "{name}: {:?}", report.findings);
        // The index path refuses to answer from damaged data when it can tell.
        let snap = db.snapshot().unwrap();
        match name {
            "REL missing" => assert!(snap.relations_of(u[0], Direction::Out, None).is_err(), "{name}"),
            "REL_OUT missing" => assert!(snap.relations_of(u[0], Direction::Out, None).unwrap().is_empty()),
            _ => {}
        }
        drop(snap);
        put_all(db.engine(), undo);
        assert_clean(&db, name);
    }

    // A relation to a unit that is not Active (damage: head patched to archived).
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(id).unwrap();
    cs.archive(u[1], None).unwrap();
    cs.commit().unwrap();
    // Forge the relation record and both entries by hand, pointing at the archived unit.
    let forged = RelationRecord {
        from: u[0],
        to: u[1],
        relation_type: "cites".into(),
        created_seq: CommitSeq(1),
        provenance: RelationProvenance::Declared,
    };
    let fid = RelationId([0xAB; 16]);
    put_all(
        db.engine(),
        vec![
            (Key::Relation { edge: fid }.encode().unwrap(), Some(forged.encode().unwrap())),
            (
                Key::RelationOut { unit: u[0], relation_type: "cites".into(), edge: fid }.encode().unwrap(),
                Some(codec::Adjacency { other: u[1] }.encode()),
            ),
            (
                Key::RelationIn { unit: u[1], relation_type: "cites".into(), edge: fid }.encode().unwrap(),
                Some(codec::Adjacency { other: u[0] }.encode()),
            ),
        ],
    );
    let report = db.verify_units().unwrap();
    assert!(report.rules().contains("UNIT-36"), "{:?}", report.findings);
    assert!(report.findings.iter().any(|f| f.to_string().contains("Archived")), "{:?}", report.findings);
}

macro_rules! engine_tests {
    ($m:ident, $E:ty) => {
        mod $m {
            type E = $E;
            use super::*;
            #[test]
            fn create_and_read() {
                super::create_and_read::<E>();
            }
            #[test]
            fn invalid_input_is_rejected() {
                super::invalid_input_is_rejected::<E>();
            }
            #[test]
            fn provenance_is_stored_and_checked() {
                super::provenance_is_stored_and_checked::<E>();
            }
            #[test]
            fn declared_is_never_confused_with_inferred() {
                super::declared_is_never_confused_with_inferred::<E>();
            }
            #[test]
            fn stored_provenance_corruption_is_detected() {
                super::stored_provenance_corruption_is_detected::<E>();
            }
            #[test]
            fn archive_and_delete_are_rejected_with_live_relations() {
                super::archive_and_delete_are_rejected_with_live_relations::<E>();
            }
            #[test]
            fn abort_leaves_no_relation() {
                super::abort_leaves_no_relation::<E>();
            }
            #[test]
            fn relation_only_changeset_commits() {
                super::relation_only_changeset_commits::<E>();
            }
            #[test]
            fn relations_count_against_the_changeset_limit() {
                super::relations_count_against_the_changeset_limit::<E>();
            }
            #[test]
            fn differential_seeded() {
                for seed in 1..=4 {
                    super::differential::<E>(seed);
                }
            }
            #[test]
            fn differential_across_scan_pages() {
                super::differential_across_scan_pages::<E>();
            }
            #[test]
            fn verifier_reports_index_damage() {
                super::verifier_reports_index_damage::<E>();
            }
        }
    };
}

engine_tests!(emulated_log, EmuLog);
engine_tests!(kerno, KernoDb);
