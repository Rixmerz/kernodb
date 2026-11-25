//! ADR-0026: the adjacency index as posting blocks (`docs/file-format/units-v9.md` §4).
//!
//! The tests read the stored blocks themselves (tag `0x41` / `0x42`) and check what the writer
//! leaves in them, then check that the reads, the verifier and the guard of a unit with relations
//! agree with the reference path. Every database test runs on `emulated_log` and on `kerno_db`.

#![cfg(feature = "test-support")]

mod harness;

use std::collections::BTreeSet;
use std::sync::Arc;

use harness::units::{Target, open_units, raw};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{ADJ_BLOCK_MAX, AdjBlock, Key};
use kerno_units::*;

type EmuLog = kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn open<E: Target>() -> UnitDb<E> {
    open_units::<E>(SimVfs::new(5).as_vfs(), &cfg()).expect("open")
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

/// One change set that relates `from` to every unit of `to`; the relation ids in order.
fn fan_out<E: Target>(db: &UnitDb<E>, from: UnitId, to: &[UnitId], ty: &str) -> Vec<RelationId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let ids = to.iter().map(|t| cs.create_relation(from, *t, ty, RelationProvenance::Declared).unwrap()).collect();
    cs.commit().unwrap();
    ids
}

/// A stored adjacency block: its key and its entries.
type StoredBlock = (Vec<u8>, Vec<(RelationId, UnitId)>);

/// The adjacency blocks of `(unit, dir, type)` as stored.
fn blocks<E: Target>(db: &UnitDb<E>, dir: Direction, unit: UnitId, ty: &str) -> Vec<StoredBlock> {
    let tag = if dir == Direction::Out { 0x41 } else { 0x42 };
    let (start, end) = Key::adjacency_range(tag, unit, Some(ty)).unwrap();
    raw(db)
        .into_iter()
        .filter(|(k, _)| *k >= start && *k < end)
        .map(|(k, v)| (k, AdjBlock::decode(&v).unwrap().entries))
        .collect()
}

#[track_caller]
fn clean<E: Target>(db: &UnitDb<E>, ctx: &str) {
    let r = db.verify_units().unwrap();
    assert!(r.is_clean(), "{ctx}: {:?}", r.findings);
}

/// The index read of `(unit, dir)` equals the reference's, relation by relation.
#[track_caller]
fn assert_reads_agree<E: Target>(db: &UnitDb<E>, unit: UnitId, dir: Direction, ty: Option<&str>) -> Vec<Relation> {
    let snap = db.snapshot().unwrap();
    let got = snap.relations_of(unit, dir, ty).unwrap();
    let mut want = snap.reference_relations_of(unit, dir, ty).unwrap();
    want.sort_by(|a, b| (&a.relation_type, a.id).cmp(&(&b.relation_type, b.id)));
    assert_eq!(got, want);
    got
}

fn a_batch_is_cut_in_blocks_in_relation_order<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 1 + 150);
    let edges = fan_out(&db, u[0], &u[1..], "cites");
    let out = blocks(&db, Direction::Out, u[0], "cites");
    assert_eq!(
        out.iter().map(|b| b.1.len()).collect::<Vec<_>>(),
        [ADJ_BLOCK_MAX, ADJ_BLOCK_MAX, 150 - 2 * ADJ_BLOCK_MAX]
    );
    let listed: Vec<RelationId> = out.iter().flat_map(|b| b.1.iter().map(|e| e.0)).collect();
    assert_eq!(listed, edges, "ascending across the blocks");
    // The first two blocks are bounded by their last relation, the third is the tail.
    let bound = |e: RelationId| {
        Key::RelationOut { unit: u[0], relation_type: "cites".into(), bound: Some(e) }.encode().unwrap()
    };
    let tail = Key::RelationOut { unit: u[0], relation_type: "cites".into(), bound: None }.encode().unwrap();
    assert_eq!(out.iter().map(|b| b.0.clone()).collect::<Vec<_>>(), [bound(edges[63]), bound(edges[127]), tail]);
    // Each target has one in block: 150 lists of one entry, not one record per relation more.
    for (t, e) in u[1..].iter().zip(&edges) {
        assert_eq!(blocks(&db, Direction::In, *t, "cites")[0].1, [(*e, u[0])]);
    }
    assert_eq!(assert_reads_agree(&db, u[0], Direction::Out, None).len(), 150);
    clean(&db, "150 relations");
}

fn a_delete_from_a_middle_block_and_a_block_emptied<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 1 + 150);
    let edges = fan_out(&db, u[0], &u[1..], "cites");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(edges[70]).unwrap();
    cs.commit().unwrap();
    let sizes =
        |db: &UnitDb<E>| blocks(db, Direction::Out, u[0], "cites").iter().map(|b| b.1.len()).collect::<Vec<_>>();
    assert_eq!(sizes(&db), [64, 63, 22]);
    assert_eq!(assert_reads_agree(&db, u[0], Direction::Out, Some("cites")).len(), 149);
    clean(&db, "one deleted");
    // Every relation of the first block: the block is deleted, not left empty.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for e in &edges[..64] {
        cs.delete_relation(*e).unwrap();
    }
    cs.commit().unwrap();
    assert_eq!(sizes(&db), [63, 22]);
    assert_eq!(assert_reads_agree(&db, u[0], Direction::Out, None).len(), 85);
    clean(&db, "a block emptied");
    // All of them: the list has no block and no count record.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for e in &edges[64..] {
        if *e != edges[70] {
            cs.delete_relation(*e).unwrap();
        }
    }
    cs.commit().unwrap();
    assert!(blocks(&db, Direction::Out, u[0], "cites").is_empty());
    assert!(assert_reads_agree(&db, u[0], Direction::Out, None).is_empty());
    clean(&db, "all deleted");
}

fn an_end_may_be_both_ends_and_edges_may_be_parallel<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let own = cs.create_relation(u[0], u[0], "self", RelationProvenance::Declared).unwrap();
    let p1 = cs.create_relation(u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    let p2 = cs.create_relation(u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    let back = cs.create_relation(u[1], u[0], "cites", RelationProvenance::Declared).unwrap();
    cs.commit().unwrap();
    // A relation of a unit to itself is in its out list and in its in list.
    assert_eq!(blocks(&db, Direction::Out, u[0], "self")[0].1, [(own, u[0])]);
    assert_eq!(blocks(&db, Direction::In, u[0], "self")[0].1, [(own, u[0])]);
    // Parallel relations are two entries of one list, in id order.
    assert_eq!(blocks(&db, Direction::Out, u[0], "cites")[0].1, [(p1, u[1]), (p2, u[1])]);
    assert_eq!(blocks(&db, Direction::In, u[0], "cites")[0].1, [(back, u[1])]);
    // Both directions, typed and untyped, equal the reference.
    for dir in [Direction::Out, Direction::In] {
        for unit in &u {
            assert_reads_agree(&db, *unit, dir, None);
            assert_reads_agree(&db, *unit, dir, Some("cites"));
        }
    }
    clean(&db, "self and parallel");
}

fn a_later_change_set_adds_to_the_list_it_finds<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 4);
    let a = fan_out(&db, u[0], &[u[1]], "cites")[0];
    let b = fan_out(&db, u[0], &[u[2]], "cites")[0];
    let c = fan_out(&db, u[0], &[u[3], u[1]], "cites");
    // One list: the entries of the three change sets are in it, not the last one's alone.
    let out = blocks(&db, Direction::Out, u[0], "cites");
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].1, [(a, u[1]), (b, u[2]), (c[0], u[3]), (c[1], u[1])]);
    assert_eq!(blocks(&db, Direction::In, u[1], "cites")[0].1, [(a, u[0]), (c[1], u[0])]);
    assert_eq!(assert_reads_agree(&db, u[0], Direction::Out, None).len(), 4);
    clean(&db, "three change sets");
}

fn created_and_deleted_in_one_change_set_leaves_nothing<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 2);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let gone = cs.create_relation(u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    let kept = cs.create_relation(u[0], u[1], "cites", RelationProvenance::Declared).unwrap();
    cs.delete_relation(gone).unwrap();
    cs.commit().unwrap();
    assert_eq!(blocks(&db, Direction::Out, u[0], "cites")[0].1, [(kept, u[1])]);
    assert_eq!(assert_reads_agree(&db, u[0], Direction::Out, None).len(), 1);
    // Created and deleted with nothing else of the list: no block at all.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let only = cs.create_relation(u[1], u[0], "other", RelationProvenance::Declared).unwrap();
    cs.delete_relation(only).unwrap();
    cs.commit().unwrap();
    assert!(blocks(&db, Direction::Out, u[1], "other").is_empty());
    assert!(blocks(&db, Direction::In, u[0], "other").is_empty());
    clean(&db, "created and deleted");
}

fn a_unit_with_a_relation_is_refused_whatever_the_change_set_has_flushed<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 3);
    let rel = fan_out(&db, u[0], &[u[1]], "cites")[0];
    // A relation of the transaction's blocks.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    assert!(matches!(cs.delete(u[1], None), Err(UnitError::UnitHasRelations { relation, .. }) if relation == rel));
    assert!(matches!(cs.archive(u[0], None), Err(UnitError::UnitHasRelations { relation, .. }) if relation == rel));
    // The refusal left the change set usable, and a relation it creates counts too.
    let mine = cs.create_relation(u[2], u[0], "cites", RelationProvenance::Declared).unwrap();
    assert!(matches!(cs.delete(u[2], None), Err(UnitError::UnitHasRelations { relation, .. }) if relation == mine));
    // A relation the change set removes no longer counts, one it keeps still does.
    cs.delete_relation(rel).unwrap();
    assert!(matches!(cs.delete(u[0], None), Err(UnitError::UnitHasRelations { relation, .. }) if relation == mine));
    cs.delete_relation(mine).unwrap();
    cs.delete(u[0], None).unwrap();
    cs.delete(u[2], None).unwrap();
    cs.commit().unwrap();
    clean(&db, "relations removed, units deleted");
}

fn a_relation_of_a_large_list_may_be_the_only_one_removed<E: Target>() {
    // A unit whose only relations are in a list of several blocks is still refused after most go.
    let db = open::<E>();
    let u = units(&db, 1 + 70);
    let edges = fan_out(&db, u[0], &u[1..], "cites");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for e in &edges[..69] {
        cs.delete_relation(*e).unwrap();
    }
    assert!(
        matches!(cs.delete(u[0], None), Err(UnitError::UnitHasRelations { relation, .. }) if relation == edges[69])
    );
    cs.delete_relation(edges[69]).unwrap();
    cs.delete(u[0], None).unwrap();
    cs.commit().unwrap();
    clean(&db, "all gone");
}

fn an_import_writes_blocks_too<E: Target>() {
    let src = open::<E>();
    let u = units(&src, 1 + 70);
    fan_out(&src, u[0], &u[1..], "cites");
    let bytes = kerno_units::export::export(&src).unwrap();
    let target = open_units::<E>(SimVfs::new(6).as_vfs(), &cfg()).unwrap();
    kerno_units::import::import(&target, &bytes).unwrap();
    assert_eq!(blocks(&target, Direction::Out, u[0], "cites").iter().map(|b| b.1.len()).collect::<Vec<_>>(), [64, 6]);
    assert_eq!(assert_reads_agree(&target, u[0], Direction::Out, None).len(), 70);
    clean(&target, "imported");
}

fn the_verifier_sees_a_block_out_of_its_range_or_undecodable<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 1 + 70);
    let edges = fan_out(&db, u[0], &u[1..], "cites");
    let out = blocks(&db, Direction::Out, u[0], "cites");
    let (key, mut entries) = out[0].clone();
    // The first block, bounded by its 64th relation, swaps it for the 65th.
    entries.pop();
    entries.push(out[1].1[0]);
    let forged = AdjBlock { entries }.encode();
    let good = db.raw_get(&key).unwrap().unwrap();
    db.raw_commit(vec![(key.clone(), Some(forged))]).unwrap();
    let rules: BTreeSet<String> = db.verify_units().unwrap().rules().into_iter().map(|r| r.to_string()).collect();
    assert!(rules.contains("UNIT-36"), "{rules:?}");
    db.raw_commit(vec![(key.clone(), Some(good.clone()))]).unwrap();
    clean(&db, "restored");
    for (bad, rule) in [
        (vec![1, 0], "UNIT-9"),
        ([&good[..], &[0][..]].concat(), "UNIT-9"),
        ([&[2u8][..], &good[1..]].concat(), "UNIT-7"),
    ] {
        db.raw_commit(vec![(key.clone(), Some(bad))]).unwrap();
        let rules: BTreeSet<String> = db.verify_units().unwrap().rules().into_iter().map(|r| r.to_string()).collect();
        assert!(rules.contains(rule), "{rule}: {rules:?}");
        // The index read refuses, and the planner answers from the reference.
        assert!(db.snapshot().unwrap().relations_of(u[0], Direction::Out, None).is_err());
    }
    db.raw_commit(vec![(key, Some(good))]).unwrap();
    let _ = edges;
    clean(&db, "restored again");
}

fn one_change_set_removes_below_the_tail_and_adds_above_it<E: Target>() {
    // The tail is the block of the ids above the last bound: one change set whose lowest id sits
    // in a bounded block still finds the tail for the ids that follow.
    let db = open::<E>();
    let u = units(&db, 1 + 70 + 2);
    let edges = fan_out(&db, u[0], &u[1..71], "cites");
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(edges[3]).unwrap();
    cs.delete_relation(edges[66]).unwrap();
    let added: Vec<RelationId> =
        u[71..].iter().map(|t| cs.create_relation(u[0], *t, "cites", RelationProvenance::Declared).unwrap()).collect();
    cs.commit().unwrap();
    let out = blocks(&db, Direction::Out, u[0], "cites");
    let kept: Vec<RelationId> = edges.iter().copied().filter(|e| *e != edges[3] && *e != edges[66]).collect();
    let listed: Vec<Vec<RelationId>> = out.iter().map(|b| b.1.iter().map(|e| e.0).collect()).collect();
    assert_eq!(listed[0], edges[..64].iter().copied().filter(|e| *e != edges[3]).collect::<Vec<_>>());
    assert_eq!(listed[1], [&kept[63..], &added[..]].concat());
    assert_eq!(out.len(), 2);
    assert_eq!(assert_reads_agree(&db, u[0], Direction::Out, Some("cites")).len(), 70);
    clean(&db, "removed below the tail, added above it");
}

fn a_damaged_tail_stops_only_the_changes_that_need_it<E: Target>() {
    let db = open::<E>();
    let u = units(&db, 1 + 70 + 1);
    let edges = fan_out(&db, u[0], &u[1..71], "cites");
    let out = blocks(&db, Direction::Out, u[0], "cites");
    assert_eq!(out.len(), 2);
    let (first, tail) = (out[0].0.clone(), out[1].0.clone());
    db.raw_commit(vec![(tail, Some(vec![1, 0]))]).unwrap();
    // A removal in the bounded block does not read the tail, and is written.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete_relation(edges[3]).unwrap();
    cs.commit().unwrap();
    let stored = AdjBlock::decode(&db.raw_get(&first).unwrap().unwrap()).unwrap();
    assert_eq!(stored.entries.len(), 63);
    assert!(stored.entries.iter().all(|e| e.0 != edges[3]));
    // A relation above the bounded block is the tail's, which cannot be read: the commit is refused.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.create_relation(u[0], u[71], "cites", RelationProvenance::Declared).unwrap();
    assert!(matches!(cs.commit(), Err(UnitError::Storage(kerno_core::Error::Corruption(_)))));
}

macro_rules! engines {
    ($($m:ident: $E:ty),*) => {$(
        mod $m {
            use super::*;
            type E = $E;
            #[test]
            fn a_batch_is_cut_in_blocks_in_relation_order() {
                super::a_batch_is_cut_in_blocks_in_relation_order::<E>();
            }
            #[test]
            fn a_delete_from_a_middle_block_and_a_block_emptied() {
                super::a_delete_from_a_middle_block_and_a_block_emptied::<E>();
            }
            #[test]
            fn an_end_may_be_both_ends_and_edges_may_be_parallel() {
                super::an_end_may_be_both_ends_and_edges_may_be_parallel::<E>();
            }
            #[test]
            fn a_later_change_set_adds_to_the_list_it_finds() {
                super::a_later_change_set_adds_to_the_list_it_finds::<E>();
            }
            #[test]
            fn created_and_deleted_in_one_change_set_leaves_nothing() {
                super::created_and_deleted_in_one_change_set_leaves_nothing::<E>();
            }
            #[test]
            fn a_unit_with_a_relation_is_refused_whatever_the_change_set_has_flushed() {
                super::a_unit_with_a_relation_is_refused_whatever_the_change_set_has_flushed::<E>();
            }
            #[test]
            fn a_relation_of_a_large_list_may_be_the_only_one_removed() {
                super::a_relation_of_a_large_list_may_be_the_only_one_removed::<E>();
            }
            #[test]
            fn an_import_writes_blocks_too() {
                super::an_import_writes_blocks_too::<E>();
            }
            #[test]
            fn the_verifier_sees_a_block_out_of_its_range_or_undecodable() {
                super::the_verifier_sees_a_block_out_of_its_range_or_undecodable::<E>();
            }
            #[test]
            fn one_change_set_removes_below_the_tail_and_adds_above_it() {
                super::one_change_set_removes_below_the_tail_and_adds_above_it::<E>();
            }
            #[test]
            fn a_damaged_tail_stops_only_the_changes_that_need_it() {
                super::a_damaged_tail_stops_only_the_changes_that_need_it::<E>();
            }
        }
    )*};
}

engines!(emulated_log: EmuLog, kerno: KernoDb);
