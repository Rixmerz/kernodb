//! P1 (post-Stage-9), defect K1: adjacency counts (`docs/file-format/units-v4.md`).
//!
//! A relation that lost its adjacency entry at one end used to be invisible
//! from that end: `relations_of` and `query_relations` answered short with no
//! error and no `Fallback`. The count record per `(unit, direction, type)`
//! makes the read compare what its scan saw with what was written. These tests
//! damage the adjacency at each end, with one or more relations, typed and
//! untyped, and require the answer to be the reference's or an explicit
//! `Corruption` / `Fallback`, never short; they pin the record layout, the
//! writer's upkeep, the verifier rules, the legacy path (a directory written
//! before layer 4 opens and works) and the limits that remain.
//!
//! Every database test runs on `emulated_log` and on `kerno_db`; the
//! committed legacy directory exists only for `kerno_db` (the other builds the
//! same content in memory).

#![cfg(feature = "test-support")]

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use harness::units::{Target, open_units, raw};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::sha256;
use kerno_testkit::vfs::{DirImage, SimVfs};
use kerno_units::adj_count::{check_shape, record};
use kerno_units::codec::{Adjacency, Key, LayerMeta, MAX_LAYER_VERSION, RelationRecord};
use kerno_units::*;

type EmuLog = kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg(first: u64) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(first)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: Target>() -> UnitDb<E> {
    open_units::<E>(SimVfs::new(5).as_vfs(), &cfg(1)).expect("open")
}

fn units<E: Target>(db: &UnitDb<E>, n: usize) -> Vec<UnitId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let ids = (0..n)
        .map(|i| {
            cs.create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: vec![i as u8] })
                .unwrap()
        })
        .collect();
    cs.commit().unwrap();
    ids
}

fn relate<E: Target>(db: &UnitDb<E>, from: UnitId, to: UnitId, ty: &str) -> RelationId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.create_relation(from, to, ty, RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    id
}

fn unrelate<E: Target>(db: &UnitDb<E>, edge: RelationId) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(edge).unwrap();
    cs.commit().unwrap();
}

fn put_all<E: Target>(db: &UnitDb<E>, puts: Vec<(Vec<u8>, Option<Vec<u8>>)>) {
    db.raw_commit(puts).unwrap();
}

#[track_caller]
fn clean<E: Target>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().unwrap();
    assert!(r.is_clean(), "{ctx}: {:?}", r.findings);
}

fn rules<E: Target>(db: &UnitDb<E>) -> BTreeSet<String> {
    db.verify_units().unwrap().rules().into_iter().map(|r| r.to_string()).collect()
}

fn mark_key() -> Vec<u8> {
    Key::LayerMeta { name: "layer".into() }.encode().unwrap()
}

fn layer_version<E: Target>(db: &UnitDb<E>) -> u32 {
    let mark = raw(db).into_iter().find(|(k, _)| *k == mark_key()).expect("mark").1;
    LayerMeta::decode_upto(&mark, MAX_LAYER_VERSION).unwrap().layer_version
}

fn count_records<E: Target>(db: &UnitDb<E>) -> BTreeMap<Vec<u8>, Vec<u8>> {
    raw(db).into_iter().filter(|(k, _)| k[0] == 0x72).collect()
}

fn is_corruption(e: &UnitError) -> bool {
    matches!(e, UnitError::Storage(kerno_core::Error::Corruption(_)))
}

fn adj_key(out: bool, unit: UnitId, ty: &str, edge: RelationId) -> Vec<u8> {
    if out {
        Key::RelationOut { unit, relation_type: ty.into(), edge }.encode().unwrap()
    } else {
        Key::RelationIn { unit, relation_type: ty.into(), edge }.encode().unwrap()
    }
}

// ---------------------------------------------------------------- layout

/// The stored bytes, pinned: `72 ‖ unit ‖ dir ‖ type ‖ 00` and `01 00 ‖ u64`.
fn record_layout_is_pinned<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    relate(&db, u[0], u[1], "cites");
    relate(&db, u[0], u[1], "cites");
    let got = count_records(&db);
    let mut out_key = vec![0x72];
    out_key.extend_from_slice(&u[0].0);
    out_key.extend_from_slice(&[0, b'c', b'i', b't', b'e', b's', 0]);
    let mut in_key = vec![0x72];
    in_key.extend_from_slice(&u[1].0);
    in_key.extend_from_slice(&[1, b'c', b'i', b't', b'e', b's', 0]);
    let value = vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    assert_eq!(got, BTreeMap::from([(out_key, value.clone()), (in_key, value)]));
    assert_eq!(layer_version(&db), 4);
    clean(&db, "two relations");
}

/// Upkeep: create adds one to each end, delete removes one, and a count that
/// reaches zero removes its record. A database with relations only is at
/// layer 4; one that never had any stays where it was.
fn counts_follow_create_and_delete<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 3);
    assert_eq!(layer_version(&db), 1);
    assert!(count_records(&db).is_empty());
    let a = relate(&db, u[0], u[1], "cites");
    let b = relate(&db, u[0], u[2], "cites");
    let c = relate(&db, u[0], u[0], "cites"); // a self relation: out and in of one unit
    let d = relate(&db, u[1], u[2], "refines");
    assert_eq!(count_records(&db).len(), 6);
    assert_eq!(
        count_records(&db)[&record(u[0], Direction::Out, "cites", 0).0],
        record(u[0], Direction::Out, "cites", 3).1
    );
    clean(&db, "four relations");
    // The same change set creates and deletes: counts are read as the change set sees them.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let e = cs.create_relation(u[2], u[1], "cites", RelationProvenance::Declared).unwrap();
    cs.create_relation(u[2], u[1], "cites", RelationProvenance::Declared).unwrap();
    cs.delete_relation(e).unwrap();
    cs.commit().unwrap();
    clean(&db, "create and delete in one change set");
    for edge in [a, b, c, d] {
        unrelate(&db, edge);
        clean(&db, "after a delete");
    }
    assert_eq!(count_records(&db).len(), 2, "only the one left in the last change set");
    assert_eq!(layer_version(&db), 4, "nothing lowers the mark");
}

/// An aborted change set leaves no count record behind and no mark change.
fn an_aborted_change_set_leaves_no_count<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    drop(cs);
    assert!(count_records(&db).is_empty());
    assert_eq!(layer_version(&db), 1);
    clean(&db, "after abort");
}

/// A create refused at encode for its provenance (an `Imported` source of
/// zero bytes) leaves no count in the open change set: a later valid write
/// and commit of the same change set must leave both ends readable and the
/// store clean.
fn a_refused_create_leaves_no_count<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 3);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let bad = RelationProvenance::Imported { source: vec![] };
    assert!(cs.create_relation(u[0], u[1], "cites", bad).is_err(), "the encode refuses the provenance");
    cs.create_relation(u[1], u[2], "cites", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let snap = db.snapshot().unwrap();
    for (unit, dir, want) in
        [(u[0], Direction::Out, 0), (u[1], Direction::In, 0), (u[1], Direction::Out, 1), (u[2], Direction::In, 1)]
    {
        let n = snap.relations_of(unit, dir, None).expect("relations_of succeeds").len();
        assert_eq!(n, want, "{unit:?} {dir:?}");
    }
    assert_eq!(count_records(&db).len(), 2, "only the counts of the one relation written");
    clean(&db, "after a refused create and a valid write");
}

// ---------------------------------------------------------------- K1

/// The answer of every read from `(unit, dir)`, typed and untyped, is the
/// reference's or an explicit corruption; the planner's answer is always the
/// reference's. Returns how many of the direct reads were an error.
#[track_caller]
fn never_short<E: Target>(db: &UnitDb<E>, unit: UnitId, dir: Direction, types: &[&str], ctx: &str) -> usize {
    let snap = db.snapshot().unwrap();
    let mut errors = 0;
    for ty in std::iter::once(None).chain(types.iter().map(|t| Some(*t))) {
        let reference = snap.reference_relations_of(unit, dir, ty).unwrap();
        match snap.relations_of(unit, dir, ty) {
            Ok(got) => assert_eq!(got, reference, "{ctx}: {ty:?} is short"),
            Err(e) => {
                assert!(is_corruption(&e), "{ctx}: {ty:?}: {e}");
                errors += 1;
            }
        }
        let q = RelationQuery { unit, direction: dir, relation_type: ty.map(Into::into) };
        let out = snap.query_relations(&q).unwrap();
        assert_eq!(out.relations, reference, "{ctx}: query {ty:?} is short");
    }
    errors
}

/// A relation with one of its two entries lost, seen from each end, with the
/// planner on the adjacency path (`filler > 0`) and without. From the end that
/// lost its entry the read is an error and the query falls back with
/// `Corrupt` to the answer of the reference.
fn one_entry_lost<E: Target>(filler: usize) {
    let db = open::<E>();
    let u = units(&db, 4);
    let lost_edge = relate(&db, u[0], u[1], "cites");
    let kept_edge = relate(&db, u[0], u[1], "refines");
    for _ in 0..filler {
        relate(&db, u[2], u[3], "other");
    }
    let cases = [
        ("REL_IN lost", adj_key(false, u[1], "cites", lost_edge), (u[0], Direction::Out), (u[1], Direction::In)),
        ("REL_OUT lost", adj_key(true, u[0], "cites", lost_edge), (u[1], Direction::In), (u[0], Direction::Out)),
    ];
    for (name, lost, kept, gone) in cases {
        let undo = vec![(lost.clone(), db.raw_get(&lost).unwrap())];
        put_all(&db, vec![(lost, None)]);
        assert!(rules(&db).contains("UNIT-36"), "{name}: verify_units reports it");
        assert!(rules(&db).contains("UNIT-45"), "{name}: and the count record that disagrees");

        // The end that kept its entry: the reference's answer, as before.
        never_short(&db, kept.0, kept.1, &["cites", "refines"], name);

        // The end that lost it: untyped, `cites` and both are errors; `refines` is still sound.
        let errors = never_short(&db, gone.0, gone.1, &["cites", "refines"], name);
        assert_eq!(errors, 2, "{name}: the untyped read and the one of the lost type refuse");
        let snap = db.snapshot().unwrap();
        let e = snap.relations_of(gone.0, gone.1, None).unwrap_err();
        assert!(is_corruption(&e), "{name}: {e}");
        assert!(snap.relations_of(gone.0, gone.1, Some("refines")).is_ok(), "{name}: other types read");
        for ty in [None, Some("cites")] {
            let q = RelationQuery { unit: gone.0, direction: gone.1, relation_type: ty.map(Into::into) };
            let out = snap.query_relations(&q).unwrap();
            assert_eq!(out.path, PlanPath::RelationsReference, "{name}");
            assert!(out.relations.iter().any(|r| r.id == lost_edge), "{name}: the lost relation is in the answer");
            if filler > 0 {
                let f = out.fallback.expect("the adjacency path was taken and abandoned");
                assert_eq!((f.abandoned, f.cause), (PlanPath::Adjacency, FallbackCause::Corrupt), "{name}");
            }
        }
        let _ = kept_edge;
        drop(snap);
        put_all(&db, undo);
        clean(&db, name);
        never_short(&db, gone.0, gone.1, &["cites", "refines"], "restored");
    }
}

/// Two relations of one type; one entry lost: the count says 2, the scan sees 1.
fn one_of_several_entries_lost<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 3);
    let first = relate(&db, u[0], u[1], "cites");
    relate(&db, u[2], u[1], "cites");
    let lost = adj_key(false, u[1], "cites", first);
    put_all(&db, vec![(lost, None)]);
    assert_eq!(never_short(&db, u[1], Direction::In, &["cites"], "one of two"), 2);
    clean_except(&db, &["UNIT-36", "UNIT-45"]);
}

#[track_caller]
fn clean_except<E: Target>(db: &UnitDb<E>, allowed: &[&str]) {
    let got = rules(db);
    let allowed: BTreeSet<String> = allowed.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(got, allowed);
}

/// A count record that is too low or too high is the same disagreement from
/// the other side: `Corruption`, the planner falls back, `verify_units` says UNIT-45.
fn a_count_that_disagrees_is_an_error<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    relate(&db, u[0], u[1], "cites");
    relate(&db, u[0], u[1], "cites");
    let key = record(u[0], Direction::Out, "cites", 0).0;
    let good = db.raw_get(&key).unwrap().unwrap();
    for wrong in [1u64, 3, 9] {
        put_all(&db, vec![(key.clone(), Some(record(u[0], Direction::Out, "cites", wrong).1))]);
        assert_eq!(never_short(&db, u[0], Direction::Out, &["cites"], "wrong count"), 2, "count {wrong}");
        clean_except(&db, &["UNIT-45"]);
    }
    put_all(&db, vec![(key.clone(), Some(good))]);
    clean(&db, "restored");
    // An orphan record: the entries are gone with the relations (UNIT-45 and a
    // read of that unit and type is an error rather than an empty answer).
    let (orphan, _) = record(u[1], Direction::Out, "cites", 1);
    put_all(&db, vec![(orphan, Some(record(u[1], Direction::Out, "cites", 1).1))]);
    assert_eq!(never_short(&db, u[1], Direction::Out, &["cites"], "orphan"), 2);
    clean_except(&db, &["UNIT-45"]);
}

/// The limits that remain, pinned so that a change to them is deliberate:
/// the entry lost **together with** its count record, and a count lowered to
/// agree with a lost entry, leave a read that agrees with itself. The
/// verifier still sees the missing entry (UNIT-36).
fn a_count_lost_with_its_entry_is_the_known_limit<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let edge = relate(&db, u[0], u[1], "cites");
    let count = record(u[1], Direction::In, "cites", 0).0;
    put_all(&db, vec![(adj_key(false, u[1], "cites", edge), None), (count, None)]);
    let snap = db.snapshot().unwrap();
    assert!(snap.relations_of(u[1], Direction::In, None).unwrap().is_empty(), "known limit: a double damage");
    assert_eq!(snap.reference_relations_of(u[1], Direction::In, None).unwrap().len(), 1);
    drop(snap);
    assert!(rules(&db).contains("UNIT-36"));
}

/// UNIT-46, and the record shapes the verifier refuses.
fn verifier_rules_of_the_count_records<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    relate(&db, u[0], u[1], "cites");
    let mark = raw(&db).into_iter().find(|(k, _)| *k == mark_key()).unwrap().1;
    let current = LayerMeta::decode_upto(&mark, MAX_LAYER_VERSION).unwrap();
    for lowered in [1, 2, 3] {
        let v = LayerMeta { layer_version: lowered, ..current.clone() }.encode().unwrap();
        put_all(&db, vec![(mark_key(), Some(v))]);
        clean_except(&db, &["UNIT-46"]);
    }
    put_all(&db, vec![(mark_key(), Some(mark))]);
    clean(&db, "mark restored");
    let key = record(u[0], Direction::Out, "cites", 1).0;
    let good = db.raw_get(&key).unwrap().unwrap();
    for (bad, rule) in [
        (vec![1, 0, 0, 0], "UNIT-9"),
        (vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0], "UNIT-10"),
        (vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0], "UNIT-12"),
        (vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 1], "UNIT-6"),
        (vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 1], "UNIT-7"),
        (vec![1, 1, 0, 0, 0, 0, 0, 0, 0, 1], "UNIT-8"),
    ] {
        put_all(&db, vec![(key.clone(), Some(bad))]);
        assert!(rules(&db).contains(rule), "{rule}: {:?}", rules(&db));
    }
    put_all(&db, vec![(key, Some(good))]);
    clean(&db, "restored");
}

// ---------------------------------------------------------------- legacy

/// What the writer before layer 4 left for a relation, byte for byte: `REL`
/// and its two adjacency entries, no count record.
fn legacy_relation<E: Target>(db: &UnitDb<E>, edge: u8, from: UnitId, to: UnitId, ty: &str) -> RelationId {
    let id = RelationId([edge; 16]);
    let rec = RelationRecord {
        from,
        to,
        relation_type: ty.into(),
        created_seq: CommitSeq(1),
        provenance: RelationProvenance::Declared,
    };
    put_all(
        db,
        vec![
            (Key::Relation { edge: id }.encode().unwrap(), Some(rec.encode().unwrap())),
            (adj_key(true, from, ty, id), Some(Adjacency { other: to }.encode())),
            (adj_key(false, to, ty, id), Some(Adjacency { other: from }.encode())),
        ],
    );
    id
}

fn build_legacy<E: Target>(db: &UnitDb<E>) -> Vec<UnitId> {
    let u = units(db, 3);
    legacy_relation(db, 0xA1, u[0], u[1], "cites");
    legacy_relation(db, 0xA2, u[0], u[2], "cites");
    legacy_relation(db, 0xA3, u[1], u[2], "refines");
    u
}

/// A store with relations and no count records serves every read as before
/// (nothing to compare with), verifies clean, and the first change to a
/// `(unit, direction, type)` records the count of what exists plus the change.
fn legacy_relations_work_and_get_a_count_when_next_changed<E: Target>() {
    let db = open::<E>();
    let u = build_legacy(&db);
    assert_eq!(layer_version(&db), 1);
    assert!(count_records(&db).is_empty());
    clean(&db, "legacy");
    assert_eq!(never_short(&db, u[0], Direction::Out, &["cites", "refines"], "legacy"), 0);
    assert_eq!(db.snapshot().unwrap().relations_of(u[0], Direction::Out, None).unwrap().len(), 2);

    let third = relate(&db, u[0], u[1], "cites");
    assert_eq!(layer_version(&db), 4);
    let counts = count_records(&db);
    assert_eq!(counts[&record(u[0], Direction::Out, "cites", 0).0], record(u[0], Direction::Out, "cites", 3).1);
    assert_eq!(counts[&record(u[1], Direction::In, "cites", 0).0], record(u[1], Direction::In, "cites", 2).1);
    assert_eq!(counts.len(), 2, "(u2, in, cites) and the others are not touched");
    clean(&db, "after the first new relation");
    // The (unit, direction, type)s that have a record are now covered.
    let lost = adj_key(true, u[0], "cites", third);
    put_all(&db, vec![(lost, None)]);
    assert_eq!(never_short(&db, u[0], Direction::Out, &["cites"], "legacy then counted"), 2);
    // The ones that have none are not (the documented limit).
    let refines = adj_key(false, u[2], "refines", RelationId([0xA3; 16]));
    put_all(&db, vec![(refines, None)]);
    assert!(db.snapshot().unwrap().relations_of(u[2], Direction::In, Some("refines")).unwrap().is_empty());
}

fn deleting_a_legacy_relation_counts_what_is_left<E: Target>() {
    let db = open::<E>();
    let u = build_legacy(&db);
    unrelate(&db, RelationId([0xA1; 16]));
    assert_eq!(layer_version(&db), 4);
    assert_eq!(
        count_records(&db)[&record(u[0], Direction::Out, "cites", 0).0],
        record(u[0], Direction::Out, "cites", 1).1
    );
    assert!(!count_records(&db).contains_key(&record(u[1], Direction::In, "cites", 0).0), "that count reached zero");
    clean(&db, "after the delete");
}

// ---------------------------------------------------------------- export

/// A database of counted relations survives export and import: the target is
/// counted too (the writer of the import keeps the counts) and verifies clean.
fn export_import_keeps_the_counts<E: Target>() {
    let src = open::<E>();
    let u = units(&src, 3);
    relate(&src, u[0], u[1], "cites");
    relate(&src, u[0], u[2], "cites");
    relate(&src, u[1], u[2], "refines");
    let bytes = kerno_units::export::export(&src).unwrap();
    let target = open_units::<E>(SimVfs::new(6).as_vfs(), &cfg(100)).unwrap();
    kerno_units::import::import(&target, &bytes).unwrap();
    assert_eq!(count_records(&target), count_records(&src));
    assert_eq!(layer_version(&target), 4);
    clean(&target, "imported");
    assert_eq!(kerno_units::export::export(&target).unwrap(), bytes);
}

// ---------------------------------------------------------------- vectors

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2), "odd hex {s:?}");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn cells(line: &str) -> Option<Vec<String>> {
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|c| c.trim().trim_matches('`').to_owned()).collect())
}

/// Each row of section 6 of `docs/file-format/units-v4.md`: positives are what
/// the production writer stores, negatives give the rule that is named.
#[test]
fn vectors_of_the_document_match_the_codec_byte_for_byte() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/file-format/units-v4.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let unit = UnitId(unhex("0102030405060708090a0b0c0d0e0f10").try_into().unwrap());
    let (mut ok, mut bad, mut marks_ok, mut marks_bad) = (0, 0, 0, 0);
    for line in text.lines() {
        let Some(c) = cells(line) else { continue };
        if c.len() != 4 || !c[0].chars().next().is_some_and(|ch| ch.is_ascii_lowercase()) || c[0] == "nombre" {
            continue;
        }
        let (name, result) = (c[0].as_str(), c[3].as_str());
        if name.starts_with("mark_") {
            let reader: u32 = c[2].parse().unwrap();
            match (result, LayerMeta::decode_upto(&unhex(&c[1]), reader)) {
                ("ok", Ok(m)) => {
                    assert_eq!(m.layer_version, 4, "{name}");
                    assert_eq!(m.encode().unwrap(), unhex(&c[1]), "{name}");
                    marks_ok += 1;
                }
                (rule, Err(e)) => {
                    assert_eq!(e.rule.id(), rule, "{name}");
                    marks_bad += 1;
                }
                (rule, Ok(_)) => panic!("{name}: decoded, wanted {rule}"),
            }
            continue;
        }
        let (key, value) = (unhex(&c[1]), unhex(&c[2]));
        match (result, check_shape(&key, &value)) {
            ("ok", Ok(count)) => {
                let dir = if key[17] == 0 { Direction::Out } else { Direction::In };
                let ty = std::str::from_utf8(&key[18..key.len() - 1]).unwrap();
                assert_eq!(record(unit, dir, ty, count), (key, value), "{name}: bytes the writer stores");
                ok += 1;
            }
            (rule, Err(e)) => {
                assert_eq!(e.rule.id(), rule, "{name}: {e}");
                bad += 1;
            }
            (rule, Ok(_)) => panic!("{name}: accepted, wanted {rule}"),
        }
    }
    assert_eq!((ok, bad, marks_ok, marks_bad), (3, 13, 1, 2));
}

// ---------------------------------------------------------------- fixture

const FIXTURE_DIR: &str = "tests/vectors/rel-counts-v4/db-legacy-relations";
const FIXTURE_MANIFEST: &str = "tests/vectors/rel-counts-v4/MANIFEST";
const FIXTURE_SEED: u64 = 44;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_files() -> BTreeMap<String, Vec<u8>> {
    let dir = root().join(FIXTURE_DIR);
    std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| {
            let e = e.unwrap();
            (e.file_name().into_string().unwrap(), std::fs::read(e.path()).unwrap())
        })
        .collect()
}

fn manifest_of(files: &BTreeMap<String, Vec<u8>>) -> String {
    files.iter().map(|(n, b)| format!("{}  {}  {n}\n", sha256::hex(&sha256::digest(b)), b.len())).collect()
}

/// Three units and three relations as the writer before layer 4 stored them
/// (`build_legacy`): mark 1, no count record.
fn build_fixture<E: Target>(disk: &SimVfs) {
    let db = open_units::<E>(disk.as_vfs(), &cfg(1)).unwrap();
    build_legacy(&db);
    db.close().unwrap();
}

/// `cargo test --release -p kerno-units --all-features --test rel_counts -- --ignored regenerate_fixture`
#[test]
#[ignore = "regenerates the committed directory"]
fn regenerate_fixture() {
    let disk = SimVfs::new(FIXTURE_SEED);
    build_fixture::<KernoDb>(&disk);
    let files: BTreeMap<String, Vec<u8>> = disk
        .dump()
        .files
        .into_iter()
        .filter_map(|(p, b)| {
            let name = p.file_name()?.to_str()?.to_owned();
            (name != "LOCK").then_some((name, b))
        })
        .collect();
    let dir = root().join(FIXTURE_DIR);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    std::fs::write(root().join(FIXTURE_MANIFEST), manifest_of(&files)).unwrap();
}

#[test]
fn fixture_manifest_lists_the_committed_files() {
    let files = fixture_files();
    assert!(!files.is_empty());
    let want = std::fs::read_to_string(root().join(FIXTURE_MANIFEST)).expect("MANIFEST");
    assert_eq!(manifest_of(&files), want, "a file of {FIXTURE_DIR} differs from MANIFEST");
}

fn fixture_disk<E: Target>() -> SimVfs {
    if E::NAME == <KernoDb as Target>::NAME {
        let image = DirImage {
            dirs: BTreeSet::from([PathBuf::from("/db")]),
            files: fixture_files().into_iter().map(|(n, b)| (Path::new("/db").join(n), b)).collect(),
            unlinked_open: Vec::new(),
        };
        SimVfs::restore(&image)
    } else {
        let disk = SimVfs::new(FIXTURE_SEED);
        build_fixture::<E>(&disk);
        disk
    }
}

/// A directory written before layer 4 (relations, no count records, mark 1)
/// opens, serves its relations, verifies clean, writes nothing on open, and
/// takes a new relation: the mark goes to 4 and the touched ends get a count.
fn legacy_directory_opens_works_and_upgrades_when_written<E: Target>() {
    let disk = fixture_disk::<E>();
    let db = open_units::<E>(disk.as_vfs(), &cfg(100)).unwrap();
    let mut id = [0u8; 16];
    id[8..].copy_from_slice(&1u64.to_be_bytes());
    let first = UnitId(id);
    id[8..].copy_from_slice(&2u64.to_be_bytes());
    let second = UnitId(id);
    assert_eq!(layer_version(&db), 1);
    assert!(count_records(&db).is_empty());
    clean(&db, "fixture");
    let snap = db.snapshot().unwrap();
    assert_eq!(snap.relations_of(first, Direction::Out, None).unwrap().len(), 2);
    assert_eq!(snap.relations_of(second, Direction::In, Some("cites")).unwrap().len(), 1);
    assert_eq!(snap.relations_of(second, Direction::Out, Some("refines")).unwrap().len(), 1);
    drop(snap);
    assert!(count_records(&db).is_empty(), "opening and reading write nothing");
    relate(&db, second, first, "cites");
    assert_eq!(layer_version(&db), 4);
    assert_eq!(count_records(&db).len(), 2);
    clean(&db, "fixture then a new relation");
    never_short(&db, first, Direction::In, &["cites"], "fixture then a new relation");
}

/// A reader that knows layer 3 refuses the mark a count record raises.
fn layer3_reader_refuses_a_database_with_counts<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    relate(&db, u[0], u[1], "cites");
    let mark = raw(&db).into_iter().find(|(k, _)| *k == mark_key()).unwrap().1;
    let e = LayerMeta::decode_upto(&mark, 3).unwrap_err();
    assert_eq!((e.rule, e.found_layer_version), (kerno_units::UnitRule::Unit16, Some(4)));
}

macro_rules! engines {
    ($($m:ident: $e:ident),*) => {$(
        mod $m {
            type E = super::$e;
            #[test]
            fn record_layout_is_pinned() {
                super::record_layout_is_pinned::<E>();
            }
            #[test]
            fn counts_follow_create_and_delete() {
                super::counts_follow_create_and_delete::<E>();
            }
            #[test]
            fn an_aborted_change_set_leaves_no_count() {
                super::an_aborted_change_set_leaves_no_count::<E>();
            }
            #[test]
            fn one_entry_lost_small_store() {
                super::one_entry_lost::<E>(0);
            }
            #[test]
            fn one_entry_lost_large_store() {
                super::one_entry_lost::<E>(70);
            }
            #[test]
            fn one_of_several_entries_lost() {
                super::one_of_several_entries_lost::<E>();
            }
            #[test]
            fn a_count_that_disagrees_is_an_error() {
                super::a_count_that_disagrees_is_an_error::<E>();
            }
            #[test]
            fn a_count_lost_with_its_entry_is_the_known_limit() {
                super::a_count_lost_with_its_entry_is_the_known_limit::<E>();
            }
            #[test]
            fn verifier_rules_of_the_count_records() {
                super::verifier_rules_of_the_count_records::<E>();
            }
            #[test]
            fn legacy_relations_work_and_get_a_count_when_next_changed() {
                super::legacy_relations_work_and_get_a_count_when_next_changed::<E>();
            }
            #[test]
            fn deleting_a_legacy_relation_counts_what_is_left() {
                super::deleting_a_legacy_relation_counts_what_is_left::<E>();
            }
            #[test]
            fn a_refused_create_leaves_no_count() {
                super::a_refused_create_leaves_no_count::<E>();
            }
            #[test]
            fn export_import_keeps_the_counts() {
                super::export_import_keeps_the_counts::<E>();
            }
            #[test]
            fn legacy_directory_opens_works_and_upgrades_when_written() {
                super::legacy_directory_opens_works_and_upgrades_when_written::<E>();
            }
            #[test]
            fn layer3_reader_refuses_a_database_with_counts() {
                super::layer3_reader_refuses_a_database_with_counts::<E>();
            }
        }
    )*};
}

engines!(emulated_log: EmuLog, kerno: KernoDb);
