//! E8-05: deleting a source deletes its derived data (etapa8-acta.md §5.3,
//! ADR-0009 option 3: I-1, I-2, C-1, C-3, C-4, B-1), and a measurement of
//! what survives on disk.
//!
//! The rules run on both engines (`emulated_log` and `kerno_db`). The survival
//! measurement reads the files of the database, so it runs on `kerno_db`.
//!
//! What this file does not cover, and why:
//!
//! * Deleting a relation: a derived datum has revisions of units as sources and
//!   the crate has no relation as a source, so there is nothing to remove. A
//!   test pins that deleting a relation leaves derived data alone.
//! * A snapshot held across a delete (I-2) and the persisted records after a
//!   delete or a retention are covered by `derived_vectors.rs` (E8-04), which
//!   this file does not repeat.
//!
//! # Predictions of the survival measurement
//!
//! Written before the first run and never edited afterwards. The unit holding
//! a persisted vector whose bytes carry a sentinel is deleted.
//!
//! - **P1, before a flush.** The vector is still in the write-ahead log (or in
//!   a table, if the log rotated): at least one live file holds the sentinel.
//! - **P2, after `compact_range` over the derived keys and a flush, no
//!   snapshot held.** The tables no longer hold the sentinel. The log may still
//!   hold it; this is printed, not asserted.
//! - **P3, after a clean close, no snapshot held.** No live file holds it
//!   (ADR-0009 C-1 and C-3). This is the one stage asserted.
//! - **P4, after reopening.** No live file holds it; the reopen writes a new
//!   log that never held the vector.
//! - **P5, with a snapshot taken before the delete.** The sentinel stays in a
//!   live file after `compact_range` and until the snapshot is released; after
//!   the release and a clean close it is in none.
//!
//! No promise of physical erasure is made or asserted.
//!
//! # Sentinels in summaries: declared gap
//!
//! The acta asks for sentinel stages for a unit with a vector and with a
//! summary; only the vector is measured. Summaries (`consolidation.rs`) are
//! held in memory only: ADR-0020 fixes on-disk bytes for vectors and for
//! nothing else, so a summary has no record in any file, and there is nothing
//! on disk to measure. Deleting the unit removes the summary from the registry
//! (`delete_removes_every_datum_with_the_source`, which builds the
//! non-persisted data with the same registry), and no file ever held its text.

mod harness;

use std::collections::BTreeMap;
use std::ops::Bound;
use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::{Options, Snapshot};
use kerno_storage::KernoDb;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::{CrashKind, OpKind, SimVfs};
use kerno_units::codec::Key;
use kerno_units::codec::{TAG_DERIVED_DESC, TAG_DERIVED_SRC, TAG_DERIVED_VEC};
use kerno_units::vector_store::{DerivedKey, VectorRecipe};
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open_db<E: Target>(disk: &SimVfs) -> UnitDb<E> {
    open_units::<E>(disk.as_vfs(), &cfg()).expect("open")
}

fn open_dd<E: Target>(disk: &SimVfs) -> DerivedDb<E> {
    DerivedDb::open_persisted(open_db::<E>(disk)).expect("open_persisted").0
}

fn recipe() -> VectorRecipe {
    VectorRecipe { method: "embed".into(), version: 1, profile: "model-a".into(), params: BTreeMap::new() }
}

fn vcompute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<f32>> {
    inputs.first().map(|i| i.payload.iter().map(|b| f32::from(*b)).collect())
}

/// A value that depends on every source, so a datum over several units is
/// distinct from one over a single unit.
fn compute(_: &DerivedDescriptor, inputs: &[SourceInput]) -> Option<Vec<u8>> {
    Some(inputs.iter().flat_map(|i| i.payload.clone()).collect())
}

fn make<E: Target>(dd: &DerivedDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn edit<E: Target>(dd: &DerivedDb<E>, id: UnitId, payload: &[u8]) {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

fn build<E: Target>(dd: &DerivedDb<E>, units: &[UnitId]) -> DerivedId {
    dd.build(units.iter().copied(), "summary", 1, BTreeMap::new(), &compute).unwrap()
}

fn vector<E: Target>(dd: &DerivedDb<E>, unit: UnitId) -> DerivedId {
    dd.build_vector(unit, &recipe(), ChangeMeta::default(), &vcompute).unwrap()
}

fn delete<E: Target>(dd: &DerivedDb<E>, unit: UnitId) {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(unit, None).unwrap();
    cs.commit().unwrap();
}

fn derived_keys<E: Target>(db: &UnitDb<E>) -> usize {
    let snap = db.raw_snapshot().unwrap();
    [TAG_DERIVED_DESC, TAG_DERIVED_VEC, TAG_DERIVED_SRC]
        .iter()
        .map(|t| snap.scan(Bound::Included(&[*t][..]), Bound::Excluded(&[*t + 1][..]), usize::MAX).unwrap().len())
        .sum()
}

fn has_vector<E: Target>(db: &UnitDb<E>, id: DerivedId) -> bool {
    db.raw_get(&DerivedKey::Vec(id).encode()).unwrap().is_some()
}

// ------------------------------------------------------------------ rules

/// Deleting a unit removes every datum with that unit among its sources, found
/// through the inverse index; a datum with several sources goes when any one of
/// them is deleted; other data stay.
fn delete_removes_every_datum_with_the_source<E: Target>() {
    let disk = SimVfs::new(70);
    let dd = open_dd::<E>(&disk);
    let (a, b, c) = (make(&dd, b"a"), make(&dd, b"b"), make(&dd, b"c"));
    let on_a = build(&dd, &[a]);
    let on_ab = build(&dd, &[a, b]);
    let on_bc = build(&dd, &[b, c]);
    let on_c = build(&dd, &[c]);
    let vec_a = vector(&dd, a);
    let vec_b = vector(&dd, b);
    delete(&dd, a);
    for gone in [on_a, on_ab, vec_a] {
        assert_eq!(dd.state(&gone), None, "a datum with the deleted source is removed");
        assert_eq!(dd.descriptor(&gone), None);
        assert_eq!(dd.value(&gone), None);
        assert_eq!(dd.vector(&gone), None);
    }
    for kept in [on_bc, on_c, vec_b] {
        assert_eq!(dd.state(&kept), Some(DerivedState::Ready), "a datum without it stays");
    }
    assert!(dd.value(&on_bc).is_some() && dd.vector(&vec_b).is_some());
    // The other source of a removed multi-source datum is deleted later: nothing is left to remove.
    delete(&dd, b);
    assert_eq!(dd.state(&on_bc), None, "any one source removes the datum");
    assert_eq!(dd.state(&vec_b), None);
    assert_eq!(dd.state(&on_c), Some(DerivedState::Ready));
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 0, "no persisted record of a deleted source");
    assert!(db.verify_units().unwrap().is_clean());
}

/// Archiving and editing a source mark; only a deletion removes.
fn only_a_deletion_removes<E: Target>() {
    let disk = SimVfs::new(71);
    let dd = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a"), make(&dd, b"b"));
    let multi = build(&dd, &[a, b]);
    edit(&dd, a, b"a2");
    assert_eq!(dd.state(&multi), Some(DerivedState::Stale), "an edit marks");
    assert!(dd.descriptor(&multi).is_some());
    dd.close().unwrap();
}

/// Retention that purges a revision removes the registry entries computed from
/// it, persisted or not, several sources included; entries of retained
/// revisions stay.
fn retention_removes_the_data_of_a_purged_revision<E: Target>() {
    let disk = SimVfs::new(72);
    let dd = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a1"), make(&dd, b"b1"));
    let old_mem = build(&dd, &[a]);
    let old_multi = build(&dd, &[a, b]);
    let old_vec = vector(&dd, a);
    edit(&dd, a, b"a2");
    edit(&dd, a, b"a3");
    let new_mem = build(&dd, &[a]);
    let new_multi = build(&dd, &[a, b]);
    let new_vec = vector(&dd, a);
    let report =
        dd.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default()).unwrap();
    assert!(report.retention.complete);
    let mut removed = report.removed.clone();
    removed.sort();
    let mut want = vec![old_mem, old_multi, old_vec];
    want.sort();
    assert_eq!(removed, want, "exactly the data of the purged revision");
    for gone in [old_mem, old_multi, old_vec] {
        assert_eq!(dd.state(&gone), None);
        assert_eq!(dd.vector(&gone), None);
        assert_eq!(dd.value(&gone), None);
    }
    for kept in [new_mem, new_multi, new_vec] {
        assert_eq!(dd.state(&kept), Some(DerivedState::Ready), "retained revisions keep their data");
    }
    // Idempotent: a second run removes nothing.
    let again =
        dd.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default()).unwrap();
    assert!(again.removed.is_empty());
    dd.close().unwrap();
    let db = open_db::<E>(&disk);
    assert_eq!(derived_keys(&db), 3, "only the current vector's records remain");
    assert!(db.verify_units().unwrap().is_clean());
}

/// A relation is not a source of any derived datum: deleting one removes none.
fn deleting_a_relation_removes_no_derived_datum<E: Target>() {
    let disk = SimVfs::new(73);
    let dd = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a"), make(&dd, b"b"));
    let multi = build(&dd, &[a, b]);
    let vec_a = vector(&dd, a);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let rel = cs.create_relation(a, b, "cites", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(rel).unwrap();
    cs.commit().unwrap();
    assert_eq!(dd.state(&multi), Some(DerivedState::Ready));
    assert_eq!(dd.state(&vec_a), Some(DerivedState::Ready));
    dd.close().unwrap();
}

/// A commit that fails removes nothing: not the registry entries, not the
/// persisted records.
fn a_failed_delete_removes_nothing<E: Target>() {
    let disk = SimVfs::new(74);
    let dd = open_dd::<E>(&disk);
    let (a, b) = (make(&dd, b"a"), make(&dd, b"b"));
    let multi = build(&dd, &[a, b]);
    let vec_a = vector(&dd, a);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(a, None).unwrap();
    disk.fail_when(|op| op.kind == OpKind::SyncData, 1, None);
    let result = cs.commit();
    assert!(disk.fault_injected(), "the fault did not fire");
    assert!(result.is_err(), "a failed data sync went unreported");
    assert_eq!(dd.state(&multi), Some(DerivedState::Ready));
    assert_eq!(dd.state(&vec_a), Some(DerivedState::Ready));
    assert!(dd.value(&multi).is_some() && dd.vector(&vec_a).is_some());
    // Without a commit there is no deletion on disk either.
    let image = disk.crash_now(CrashKind::PowerLoss);
    let db = open_db::<E>(&image);
    let head = db.get_unit(a).unwrap().unwrap();
    assert_eq!(head.lifecycle == Lifecycle::Deleted, derived_keys(&db) == 0, "deletion and removal are one commit");
    assert_eq!(has_vector(&db, vec_a), head.lifecycle != Lifecycle::Deleted);
    assert!(db.verify_units().unwrap().is_clean());
}

/// An aborted change set removes nothing.
fn an_aborted_delete_removes_nothing<E: Target>() {
    let disk = SimVfs::new(75);
    let dd = open_dd::<E>(&disk);
    let a = make(&dd, b"a");
    let multi = build(&dd, &[a]);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(a, None).unwrap();
    cs.abort();
    assert_eq!(dd.state(&multi), Some(DerivedState::Ready));
    dd.close().unwrap();
}

/// A known descriptor with a deleted, purged or missing source is dropped on
/// open and reported, never registered nor rebuilt from the sources left.
fn reopen_drops_a_datum_whose_source_is_gone<E: Target>() {
    let disk = SimVfs::new(76);
    let dd = open_dd::<E>(&disk);
    let (a, b, c) = (make(&dd, b"a1"), make(&dd, b"b"), make(&dd, b"c"));
    let on_a = build(&dd, &[a]);
    let on_ab = build(&dd, &[a, b]);
    let on_b = build(&dd, &[b]);
    edit(&dd, c, b"c2");
    edit(&dd, c, b"c3");
    let on_c3 = build(&dd, &[c]);
    // Retention purges revision 1 of `c`.
    let policy = RetentionPolicy::all(RetentionRule::KeepLast(1));
    dd.run_retention(&policy, RetentionBudget::default()).unwrap();
    let missing = DerivedDescriptor::new(
        [RevisionId { unit: UnitId([0xEE; 16]), rev: RevNo(1) }],
        "summary",
        1,
        BTreeMap::new(),
        [0; 32],
    )
    .unwrap();
    let ids = [on_a, on_ab, on_b, on_c3];
    let mut known: Vec<DerivedDescriptor> = ids.iter().map(|i| dd.descriptor(i).unwrap()).collect();
    known.push(missing.clone());
    let purged = {
        // The descriptor of the purged datum is read back from before the purge:
        // rebuild it from its parts, as a caller that kept it would.
        let d = DerivedDescriptor::new([RevisionId { unit: c, rev: RevNo(1) }], "summary", 1, BTreeMap::new(), [1; 32])
            .unwrap();
        known.push(d.clone());
        d
    };
    delete(&dd, a);
    dd.close().unwrap();
    let (dd, mut dropped) = DerivedDb::open(open_db::<E>(&disk), known, &compute).unwrap();
    dropped.sort();
    let mut want = vec![on_a, on_ab, missing.id(), purged.id()];
    want.sort();
    assert_eq!(dropped, want, "exactly the data with a deleted, missing or purged source");
    for gone in &want {
        assert_eq!(dd.state(gone), None, "a dropped datum is not registered");
        assert_eq!(dd.value(gone), None);
        assert!(dd.rebuild(gone, &compute).is_err(), "and cannot be rebuilt");
    }
    for kept in [on_b, on_c3] {
        assert_eq!(dd.state(&kept), Some(DerivedState::Ready));
    }
    dd.close().unwrap();
}

/// A retention that fails to commit removes nothing from the registry.
fn a_failed_retention_removes_nothing_from_the_registry<E: Target>() {
    let disk = SimVfs::new(77);
    let dd = open_dd::<E>(&disk);
    let a = make(&dd, b"a1");
    let old_mem = build(&dd, &[a]);
    let old_vec = vector(&dd, a);
    edit(&dd, a, b"a2");
    edit(&dd, a, b"a3");
    let before = (dd.state(&old_mem), dd.state(&old_vec));
    // Retention commits without a sync of its own (decision 12c): fail the write.
    disk.fail_when(|op| op.kind == OpKind::WriteAt, 1, None);
    let result = dd.run_retention(&RetentionPolicy::all(RetentionRule::KeepLast(1)), RetentionBudget::default());
    assert!(disk.fault_injected(), "the fault did not fire");
    assert!(result.is_err(), "a failed retention went unreported");
    for kept in [old_mem, old_vec] {
        assert!(dd.descriptor(&kept).is_some(), "a retention that did not commit removed a registry entry");
    }
    assert_eq!((dd.state(&old_mem), dd.state(&old_vec)), before, "and changed no state");
}

/// A unit that cannot be read does not discard the report of a retention that
/// committed, nor stop the reconciliation of the other units.
fn an_unreadable_unit_is_reported_not_fatal<E: Target>() {
    let disk = SimVfs::new(78);
    let dd = open_dd::<E>(&disk);
    let a = make(&dd, b"a1");
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let mut note = || {
        cs.create(NewUnit { unit_type: "note".into(), schema_version: 1, metadata: vec![], payload: b"n".to_vec() })
            .unwrap()
    };
    let (n, m) = (note(), note());
    cs.commit().unwrap();
    let old_vec = vector(&dd, a);
    let note_vec = vector(&dd, n);
    let other_vec = vector(&dd, m);
    edit(&dd, a, b"a2");
    edit(&dd, a, b"a3");
    let new_vec = vector(&dd, a);
    dd.close().unwrap();
    // Break the note's head revision metadata behind the layer's back.
    let db = open_db::<E>(&disk);
    let dels = [n, m].map(|u| (Key::RevMeta { unit: u, rev: RevNo(1) }.encode().unwrap(), None)).to_vec();
    db.raw_commit(dels).unwrap();
    db.close().unwrap();
    let dd = open_dd::<E>(&disk);
    let policy = RetentionPolicy::all(RetentionRule::KeepLast(1)).with_type("note", RetentionRule::KeepAll);
    let report =
        dd.run_retention(&policy, RetentionBudget::default()).expect("a committed retention returns its report");
    let mut both = vec![n, m];
    both.sort();
    assert_eq!(report.unreadable, both, "every unreadable unit is listed, none stops the walk");
    assert_eq!(report.removed, vec![old_vec], "the readable units are still reconciled");
    assert!(
        dd.descriptor(&note_vec).is_some() && dd.descriptor(&other_vec).is_some(),
        "unreadable units' data are left alone"
    );
    assert!(dd.descriptor(&new_vec).is_some());
}

// ------------------------------------------------------------- survival

/// The sentinel of the vector: the bits of two f32 that spell it.
const SENTINEL: &[u8; 8] = b"VSENT-SP";

fn sentinel_vector() -> Vec<f32> {
    SENTINEL.as_chunks::<4>().0.iter().map(|c| f32::from_bits(u32::from_be_bytes(*c))).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn holders(disk: &SimVfs) -> Vec<String> {
    let image = disk.dump();
    let mut out: Vec<String> = image
        .files
        .iter()
        .filter(|(_, bytes)| contains(bytes, SENTINEL))
        .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    if image.unlinked_open.iter().any(|b| contains(b, SENTINEL)) {
        out.push("(unlinked, open)".into());
    }
    out.sort();
    out
}

fn stage(hold: bool, name: &str, disk: &SimVfs) -> Vec<String> {
    let files = holders(disk);
    println!("[suppression survival] hold_snapshot={hold} {name}: {files:?}");
    files
}

fn survival_of_a_deleted_vector() {
    for hold in [false, true] {
        let disk = SimVfs::new(80 + u64::from(hold));
        let dd = open_dd::<KernoDb>(&disk);
        let u = make(&dd, b"hello");
        let id = dd
            .build_vector(u, &recipe(), ChangeMeta::default(), &|_: &DerivedDescriptor, _: &[SourceInput]| {
                Some(sentinel_vector())
            })
            .unwrap();
        dd.close().unwrap();
        let db = open_db::<KernoDb>(&disk);
        assert!(!holders(&disk).is_empty(), "the sentinel is on disk before the delete");
        let held = hold.then(|| db.raw_snapshot().unwrap());
        let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
        cs.delete(u, None).unwrap();
        cs.commit().unwrap();
        assert_eq!(derived_keys(&db), 0, "I-1: no new read returns a record of the deleted source");
        let before_flush = stage(hold, "before the flush", &disk);
        db.engine_flush().unwrap();
        db.engine_compact_range(Bound::Included(&[TAG_DERIVED_DESC]), Bound::Excluded(&[TAG_DERIVED_SRC + 1])).unwrap();
        db.engine_wait_idle().unwrap();
        let after_compaction = stage(hold, "after compact_range", &disk);
        println!("[suppression survival] before-flush holders: {} file(s)", before_flush.len());
        if let Some(snap) = held {
            assert!(
                snap.get(&DerivedKey::Vec(id).encode()).unwrap().is_some(),
                "I-2: the snapshot taken before the delete still reads the vector"
            );
            println!("[suppression survival] snapshot held across compaction, holders: {after_compaction:?}");
            drop(snap);
            db.engine_wait_idle().unwrap();
        }
        db.close().unwrap();
        let closed = stage(hold, "after a clean close", &disk);
        // The one promise (ADR-0009 C-1, C-3): after compact_range and a clean
        // close, in no live file that no snapshot retains.
        assert!(closed.is_empty(), "the sentinel is in a live file after compaction and a clean close: {closed:?}");
        let db = open_db::<KernoDb>(&disk);
        stage(hold, "after reopening", &disk);
        db.close().unwrap();
        stage(hold, "after closing again", &disk);
    }
}

macro_rules! tests {
    ($e:ty; $($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                super::$name::<$e>();
            }
        )*
    };
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            tests!(
                $e;
                delete_removes_every_datum_with_the_source,
                only_a_deletion_removes,
                retention_removes_the_data_of_a_purged_revision,
                deleting_a_relation_removes_no_derived_datum,
                a_failed_delete_removes_nothing,
                an_aborted_delete_removes_nothing,
                reopen_drops_a_datum_whose_source_is_gone,
                a_failed_retention_removes_nothing_from_the_registry,
                an_unreadable_unit_is_reported_not_fatal,
            );
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);

#[test]
fn survival_of_a_deleted_vector_on_kerno_db() {
    survival_of_a_deleted_vector();
}
