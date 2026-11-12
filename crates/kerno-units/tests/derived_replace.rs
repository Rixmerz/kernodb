//! E9-10: atomic replacement of a stale vector with one computed elsewhere,
//! the `dim` check of the recipe, `RevisionId::first` and the identity test
//! of F-04 (changing only `profile` changes the identity). Every rule runs on
//! both engines (`emulated_log` and `kerno_db`).

mod harness;

use std::collections::BTreeMap;
use std::sync::Arc;

use harness::units::{Dump, Target, open_units, raw};
use kerno_core::contract::Options;
use kerno_testkit::vfs::SimVfs;
use kerno_units::vector_store::VectorSpec;
use kerno_units::*;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn cfg(start: u64) -> UnitConfig {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(start)));
    c.engine = Options::small_for_tests();
    c
}

fn open_dd<E: Target>(disk: &SimVfs) -> DerivedDb<E> {
    DerivedDb::open_persisted(open_units::<E>(disk.as_vfs(), &cfg(100)).expect("open")).expect("open_persisted").0
}

/// The raw store of a closed database.
fn raw_of<E: Target>(disk: &SimVfs) -> Dump {
    let db = open_units::<E>(disk.as_vfs(), &cfg(100)).expect("open");
    let d = raw(&db);
    db.close().unwrap();
    d
}

fn make<E: Target>(dd: &DerivedDb<E>, payload: &[u8]) -> UnitId {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs
        .create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() })
        .unwrap();
    cs.commit().unwrap();
    id
}

fn edit<E: Target>(dd: &DerivedDb<E>, id: UnitId, payload: &[u8]) -> RevNo {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let rev =
        cs.update(id, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
    rev
}

fn spec(source: RevisionId, profile: &str, dim: Option<&str>, vector: Vec<f32>) -> VectorSpec {
    let params = dim.map(|d| BTreeMap::from([("dim".to_owned(), d.to_owned())])).unwrap_or_default();
    VectorSpec { source, method: "embed".into(), method_version: 1, profile: profile.into(), params, vector }
}

fn put<E: Target>(dd: &DerivedDb<E>, s: &VectorSpec) -> Result<DerivedId, DerivedStaleError> {
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let id = cs.put_vector(s)?;
    cs.commit().unwrap();
    Ok(id)
}

fn replaces_a_stale_vector_with_one_computed_outside<E: Target>() {
    let disk = SimVfs::new(1);
    let dd = open_dd::<E>(&disk);
    let u = make(&dd, b"v1");
    let old = put(&dd, &spec(RevisionId::first(u), "m", Some("2"), vec![1.0, 2.0])).unwrap();
    let rev = edit(&dd, u, b"v2");
    assert_eq!(dd.state(&old), Some(DerivedState::Stale));

    let fresh = spec(RevisionId { unit: u, rev }, "m", Some("2"), vec![3.0, 4.0]);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let new = cs.replace_vector(old, &fresh).unwrap();
    cs.commit().unwrap();

    assert_ne!(new, old, "the identity includes the source revision");
    assert_eq!(dd.state(&old), None, "nothing of the old datum is left");
    assert_eq!(dd.vector(&old), None);
    assert_eq!(dd.state(&new), Some(DerivedState::Ready));
    assert_eq!(dd.vector(&new), Some(vec![3.0, 4.0]));
    dd.close().unwrap();
}

fn replace_refuses_an_unknown_datum_and_writes_nothing<E: Target>() {
    let disk = SimVfs::new(2);
    let dd = open_dd::<E>(&disk);
    let u = make(&dd, b"v1");
    let old = put(&dd, &spec(RevisionId::first(u), "m", None, vec![1.0])).unwrap();
    let rev = edit(&dd, u, b"v2");
    dd.close().unwrap();
    let before = raw_of::<E>(&disk);
    let dd = open_dd::<E>(&disk);
    let mut cs = dd.begin_changeset(ChangeMeta::default()).unwrap();
    let bogus = DerivedId(old.0.map(|b| b ^ 1));
    assert!(cs.replace_vector(bogus, &spec(RevisionId { unit: u, rev }, "m", None, vec![1.0])).is_err());
    cs.abort();
    dd.close().unwrap();
    assert_eq!(raw_of::<E>(&disk), before);
}

fn put_vector_refuses_a_dim_that_is_not_the_length<E: Target>() {
    let disk = SimVfs::new(3);
    let dd = open_dd::<E>(&disk);
    let u = make(&dd, b"v1");
    dd.close().unwrap();
    let before = raw_of::<E>(&disk);
    let dd = open_dd::<E>(&disk);
    for declared in ["3", "1", "0", "two", ""] {
        let s = spec(RevisionId::first(u), "m", Some(declared), vec![1.0, 2.0]);
        let err = put(&dd, &s).unwrap_err();
        assert!(err.to_string().contains("dim"), "dim {declared:?}: {err}");
    }
    dd.close().unwrap();
    assert_eq!(raw_of::<E>(&disk), before, "nothing was persisted");
    let dd = open_dd::<E>(&disk);
    assert!(put(&dd, &spec(RevisionId::first(u), "m", Some("2"), vec![1.0, 2.0])).is_ok());
    dd.close().unwrap();
}

fn first_revision_is_the_one_create_writes<E: Target>() {
    let disk = SimVfs::new(4);
    let dd = open_dd::<E>(&disk);
    let u = make(&dd, b"v1");
    let unit = dd.get_unit(u).unwrap().unwrap();
    assert_eq!(unit.head, RevisionId::first(u).rev);
    assert_eq!(RevisionId::first(u), RevisionId { unit: u, rev: RevNo(1) });
    dd.close().unwrap();
}

/// F-04: with the source revision, method and version equal, only the profile
/// differs, and the two data are two identities that live side by side.
fn changing_only_the_profile_changes_the_identity<E: Target>() {
    let disk = SimVfs::new(5);
    let dd = open_dd::<E>(&disk);
    let u = make(&dd, b"v1");
    let a = put(&dd, &spec(RevisionId::first(u), "model-a", None, vec![1.0])).unwrap();
    let b = put(&dd, &spec(RevisionId::first(u), "model-b", None, vec![1.0])).unwrap();
    assert_ne!(a, b);
    assert_eq!(dd.state(&a), Some(DerivedState::Ready));
    assert_eq!(dd.state(&b), Some(DerivedState::Ready));
    let same = put(&dd, &spec(RevisionId::first(u), "model-a", None, vec![1.0])).unwrap_err();
    assert!(matches!(same, DerivedStaleError::Derived(DerivedError::Duplicate(_))), "{same:?}");
    dd.close().unwrap();
}

macro_rules! engines {
    ($($m:ident: $e:ty),*) => {$(
        mod $m {
            #[test] fn replaces() { super::replaces_a_stale_vector_with_one_computed_outside::<$e>(); }
            #[test] fn replace_unknown() { super::replace_refuses_an_unknown_datum_and_writes_nothing::<$e>(); }
            #[test] fn dim() { super::put_vector_refuses_a_dim_that_is_not_the_length::<$e>(); }
            #[test] fn first() { super::first_revision_is_the_one_create_writes::<$e>(); }
            #[test] fn profile() { super::changing_only_the_profile_changes_the_identity::<$e>(); }
        }
    )*};
}

engines!(emulated_log: kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>, kerno_db: kerno_storage::KernoDb);
