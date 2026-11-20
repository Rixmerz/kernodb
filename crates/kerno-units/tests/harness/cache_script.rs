//! The script the cache tests share (E6-02, E6-03): a database of units of
//! two types and two schema versions related to each other, the queries that
//! read it, and the writes that move `commit_seq` between reads. Every
//! database opened here is opened the same way, so equal scripts give equal
//! ids, sequences, answers and engine bytes.

use std::sync::Arc;

use super::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

pub const T0: Timestamp = Timestamp(1_700_000_000_000_000);

pub fn config() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

/// Every database of a test is opened the same way, so equal scripts give
/// equal ids, sequences and answers.
pub fn open<E: Target>() -> UnitDb<E> {
    open_units::<E>(SimVfs::new(1).as_vfs(), &config()).expect("open")
}

pub fn new_unit(t: &str, schema: u32, tag: u8) -> NewUnit {
    NewUnit { unit_type: t.into(), schema_version: schema, metadata: vec![tag], payload: vec![tag; 8] }
}

pub fn queries() -> Vec<UnitFilter> {
    vec![
        UnitFilter::default(),
        UnitFilter { unit_type: Some("doc".into()), schema_version: None },
        UnitFilter { unit_type: Some("img".into()), schema_version: None },
        UnitFilter { unit_type: None, schema_version: Some(2) },
        UnitFilter { unit_type: Some("doc".into()), schema_version: Some(2) },
    ]
}

pub fn relation_queries(units: &[UnitId]) -> Vec<RelationQuery> {
    let mut out = Vec::new();
    for unit in units.iter().take(3) {
        for direction in [Direction::Out, Direction::In] {
            out.push(RelationQuery { unit: *unit, direction, relation_type: None });
            out.push(RelationQuery { unit: *unit, direction, relation_type: Some("cites".into()) });
        }
    }
    out
}

/// Units of two types and two schema versions, related to each other.
pub fn seed<E: Target>(db: &UnitDb<E>) -> Vec<UnitId> {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let mut ids = Vec::new();
    for i in 0..6u8 {
        let (t, v) = if i % 2 == 0 { ("doc", 1 + u32::from(i % 4 == 0)) } else { ("img", 2) };
        ids.push(cs.create(new_unit(t, v, i)).unwrap());
    }
    for pair in ids.windows(2) {
        cs.create_relation(pair[0], pair[1], "cites", RelationProvenance::Imported { source: vec![1] }).unwrap();
        cs.create_relation(pair[1], pair[0], "mentions", RelationProvenance::Imported { source: vec![2] }).unwrap();
    }
    cs.commit().unwrap();
    db.rebuild_index().unwrap();
    ids
}

/// The write that follows round `n` of the script. Each moves `commit_seq`;
/// some also change what the queries select, one moves the index.
pub fn write<E: Target>(db: &UnitDb<E>, ids: &[UnitId], n: usize) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    match n % 5 {
        0 => {
            cs.create(new_unit("doc", 2, 100 + n as u8)).unwrap();
        }
        1 => {
            let upd = UnitUpdate { payload: vec![9; 4], schema_version: Some(2), metadata: None };
            cs.update(ids[0], upd, None).unwrap();
        }
        2 => {
            cs.create_relation(ids[0], ids[2], "cites", RelationProvenance::Imported { source: vec![3] }).unwrap();
        }
        3 => {
            let upd = UnitUpdate { payload: vec![7; 4], schema_version: Some(1), metadata: None };
            cs.update(ids[3], upd, None).unwrap();
        }
        _ => {
            cs.create(new_unit("img", 1, 200 + n as u8)).unwrap();
        }
    }
    cs.commit().unwrap();
    if n % 5 == 4 {
        db.rebuild_index().unwrap();
    }
}
