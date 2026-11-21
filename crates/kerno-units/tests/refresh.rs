//! `UnitDb::refresh`: a read-only handle opened beside a live writer sees the writer's later
//! commits only after it refreshes, and a read-write handle refuses it.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Options;
use kerno_storage::{AccessMode, Config, KernoDb};
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

const DIR: &str = "/db";
const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "note".into(), schema_version: 1, metadata: Vec::new(), payload: payload.to_vec() }
}

#[test]
fn a_read_only_handle_sees_a_later_commit_only_after_refresh() {
    let disk = SimVfs::new(1);
    let writer = UnitDb::<KernoDb>::open(disk.as_vfs(), Path::new(DIR), &cfg()).unwrap();
    let (first, _) = writer.create_unit(unit(b"one"), ChangeMeta::default()).unwrap();

    let engine_cfg = Config { access: AccessMode::ReadOnly, ..Config::from(&Options::small_for_tests()) };
    let engine = KernoDb::open(disk.as_vfs(), Path::new(DIR), &engine_cfg).unwrap();
    let reader = UnitDb::<KernoDb>::from_engine(engine, &cfg()).unwrap();
    let opened_at = reader.snapshot().unwrap().seq();
    assert!(reader.get_unit(first).unwrap().is_some());

    let (second, info) = writer.create_unit(unit(b"two"), ChangeMeta::default()).unwrap();
    let held = reader.snapshot().unwrap();
    assert!(reader.get_unit(second).unwrap().is_none(), "the reader must not see the commit before refresh");
    assert_eq!(reader.snapshot().unwrap().seq(), opened_at);

    assert_eq!(reader.refresh().unwrap(), info.commit_seq.0);
    assert_eq!(reader.get_unit(second).unwrap().unwrap().payload.as_deref(), Some(&b"two"[..]));
    assert_eq!(reader.snapshot().unwrap().seq().0, info.commit_seq.0);
    assert!(held.get_unit(second).unwrap().is_none(), "a snapshot taken before keeps its state");
}

#[test]
fn a_read_write_handle_refuses_refresh() {
    let disk = SimVfs::new(1);
    let writer = UnitDb::<KernoDb>::open(disk.as_vfs(), Path::new(DIR), &cfg()).unwrap();
    assert!(matches!(writer.refresh(), Err(UnitError::Storage(kerno_core::Error::InvalidArgument(_)))));
}

/// The reader cached the policy of `note` at the epoch it opened under; refresh must drop it.
#[test]
fn refresh_drops_the_cached_history_policies() {
    let disk = SimVfs::new(1);
    let writer = UnitDb::<KernoDb>::open(disk.as_vfs(), Path::new(DIR), &cfg()).unwrap();
    writer.create_unit(unit(b"one"), ChangeMeta::default()).unwrap();
    let engine_cfg = Config { access: AccessMode::ReadOnly, ..Config::from(&Options::small_for_tests()) };
    let engine = KernoDb::open(disk.as_vfs(), Path::new(DIR), &engine_cfg).unwrap();
    let reader = UnitDb::<KernoDb>::from_engine(engine, &cfg()).unwrap();
    let before = reader.policy("note").unwrap();

    let other = if before == HistoryPolicy::Versioned { HistoryPolicy::Audit } else { HistoryPolicy::Versioned };
    writer.set_policy("note", other, ChangeMeta::default()).unwrap();
    assert_eq!(reader.policy("note").unwrap(), before, "not visible before refresh");
    reader.refresh().unwrap();
    assert_eq!(reader.policy("note").unwrap(), other);
}
