//! E9-11: `UnitError::is_locked` tells a held database from a damaged one,
//! and a `DerivedStaleError` names the unit or datum it is about, not its
//! content.
//! Every test runs on both engines.

mod harness;

use std::collections::BTreeMap;
use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type EmuLog = Emulated<LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);
const SECRET: &[u8] = b"SECRET-SOURCE-CONTENT";

fn cfg() -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    c
}

fn compute(_: &DerivedDescriptor, _: &[SourceInput]) -> Option<Vec<u8>> {
    Some(Vec::new())
}

fn a_second_open_is_locked_until_the_first_closes<E: Target>() {
    let disk = SimVfs::new(1);
    let first = open_units::<E>(disk.as_vfs(), &cfg()).expect("first open");
    let err = match open_units::<E>(disk.as_vfs(), &cfg()) {
        Ok(_) => panic!("a second handle must be refused while the first is open"),
        Err(e) => e,
    };
    assert!(matches!(err, UnitError::Storage(kerno_core::Error::Locked)), "{err:?}");
    assert!(err.is_locked());

    // The lock is the one refusal a caller can wait out.
    first.close().expect("close");
    open_units::<E>(disk.as_vfs(), &cfg()).expect("open after the first closed").close().expect("close");

    for other in [
        UnitError::Storage(kerno_core::Error::Corruption("damaged".into())),
        UnitError::Storage(kerno_core::Error::Incompatible("newer".into())),
        UnitError::NotAUnitDatabase,
        UnitError::ChangeSetClosed,
    ] {
        assert!(!other.is_locked(), "{other:?}");
    }
}

fn a_missing_source_is_named_by_its_unit_id<E: Target>() {
    let disk = SimVfs::new(1);
    let db = DerivedDb::open(open_units::<E>(disk.as_vfs(), &cfg()).expect("open"), Vec::new(), &compute).unwrap().0;

    let nobody = UnitId([0xab; 16]);
    let err = db.build([nobody], "x", 1, BTreeMap::new(), &compute).unwrap_err();
    assert!(matches!(err, DerivedStaleError::MissingSource(u) if u == nobody), "{err:?}");
    assert_eq!(err.to_string(), "source unit abababababababababababababababab does not exist or is deleted");

    // A deleted unit: the message names it and carries none of its content.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let gone = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: SECRET.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(gone, None).unwrap();
    cs.commit().unwrap();
    let err = db.build([gone], "x", 1, BTreeMap::new(), &compute).unwrap_err();
    let text = err.to_string();
    assert_eq!(text, format!("source unit {gone} does not exist or is deleted"));
    assert!(!text.contains("SECRET"), "{text}");
    db.close().unwrap();
}

/// E8 F-02: after a rebuild, building again over the same sources finds the
/// rebuilt datum; the message says so and names it by its id.
fn building_again_after_a_rebuild_names_the_rebuilt_datum<E: Target>() {
    let disk = SimVfs::new(1);
    let db = DerivedDb::open(open_units::<E>(disk.as_vfs(), &cfg()).expect("open"), Vec::new(), &compute).unwrap().0;
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let a = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: SECRET.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    let first = db.build([a], "x", 1, BTreeMap::new(), &compute).unwrap();

    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(a, UnitUpdate { payload: b"SECRET-2".to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    let rebuilt = db.rebuild(&first, &compute).unwrap();
    assert_ne!(rebuilt, first);

    let err = db.build([a], "x", 1, BTreeMap::new(), &compute).unwrap_err();
    assert!(matches!(err, DerivedStaleError::Derived(DerivedError::Duplicate(id)) if id == rebuilt), "{err:?}");
    let hex: String = rebuilt.0.iter().map(|b| format!("{b:02x}")).collect();
    let text = err.to_string();
    assert_eq!(
        text,
        format!(
            "derived datum {hex} already exists over these source revisions (built or rebuilt earlier); \
             read it by this id instead of building it again"
        )
    );
    assert!(!text.contains("SECRET"), "{text}");
    db.close().unwrap();
}

macro_rules! engines {
    ($($name:ident: $engine:ty),*) => {$(
        mod $name {
            type E = $engine;
            #[test]
            fn a_second_open_is_locked_until_the_first_closes() {
                super::a_second_open_is_locked_until_the_first_closes::<E>();
            }
            #[test]
            fn a_missing_source_is_named_by_its_unit_id() {
                super::a_missing_source_is_named_by_its_unit_id::<E>();
            }
            #[test]
            fn building_again_after_a_rebuild_names_the_rebuilt_datum() {
                super::building_again_after_a_rebuild_names_the_rebuilt_datum::<E>();
            }
        }
    )*};
}

engines!(emulated_log: crate::EmuLog, kerno_db: kerno_storage::KernoDb);
