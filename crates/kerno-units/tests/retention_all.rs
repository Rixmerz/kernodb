//! `UnitDb::run_retention_all`: retention run until a pass completes.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

const DIR: &str = "/db";
const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn open(disk: &SimVfs) -> UnitDb<KernoDb> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::stepping(T0, 1_000_000)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    UnitDb::open(disk.as_vfs(), Path::new(DIR), &c).unwrap()
}

fn unit(payload: &[u8]) -> NewUnit {
    NewUnit { unit_type: "note".into(), schema_version: 1, metadata: Vec::new(), payload: payload.to_vec() }
}

fn update(db: &UnitDb<KernoDb>, id: UnitId, payload: &[u8]) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

/// A pass reads at most 4 096 heads with nothing to do. `KeepFor` has no cursor in the handle, so
/// a bare `run_retention` restarts at the first unit every time and never reaches the unit that
/// has something to prune; `run_retention_all` resumes where the pass stopped.
#[test]
fn it_reaches_a_unit_behind_thousands_with_nothing_to_prune_under_keep_for() {
    let disk = SimVfs::new(7);
    let db = open(&disk);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for _ in 0..4_100 {
        cs.create(unit(b"x")).unwrap();
    }
    cs.commit().unwrap();
    let (last, _) = db.create_unit(unit(b"v1"), ChangeMeta::default()).unwrap();
    update(&db, last, b"v2");
    update(&db, last, b"v3");

    let policy = RetentionPolicy::all(RetentionRule::KeepFor(Duration::ZERO));
    let bare = db.run_retention(&policy, RetentionBudget::default()).unwrap();
    assert!(!bare.complete && bare.revisions_pruned == 0, "{bare:?}");

    let all = db.run_retention_all(&policy).unwrap();
    assert!(all.complete);
    assert_eq!((all.units_retained, all.revisions_pruned), (1, 2), "{all:?}");
    assert_eq!(all.change_sets.len(), 1);
    assert_eq!(all.change_set.as_ref(), all.change_sets.last());
    assert_eq!(db.get_unit(last).unwrap().unwrap().oldest, RevNo(3));
    assert!(db.verify_units().unwrap().is_clean());
}

/// Several passes (two units each) are added up, and the unit's older revisions answer `Pruned`.
#[test]
fn it_adds_up_the_passes_of_a_small_budget() {
    let disk = SimVfs::new(8);
    let db = {
        let mut c =
            UnitConfig::with_sources(Arc::new(FixedClock::stepping(T0, 1_000_000)), Arc::new(CounterIds::new(1)));
        c.engine = Options::small_for_tests();
        c.max_changeset_ops = 2;
        UnitDb::<KernoDb>::open(disk.as_vfs(), Path::new(DIR), &c).unwrap()
    };
    let ids: Vec<(UnitId, CommitSeq)> = (0..5)
        .map(|_| {
            let (id, created) = db.create_unit(unit(b"v1"), ChangeMeta::default()).unwrap();
            update(&db, id, b"v2");
            update(&db, id, b"v3");
            (id, created.commit_seq)
        })
        .collect();
    let (first, created) = ids[0];
    assert!(matches!(db.unit_as_of(first, AsOf::Seq(created)).unwrap(), UnitAt::Present(r) if r.id.rev == RevNo(1)));

    let r = db.run_retention_all(&RetentionPolicy::all(RetentionRule::KeepLast(1))).unwrap();
    assert!(r.complete);
    assert_eq!((r.units_retained, r.revisions_pruned), (5, 10), "{r:?}");
    assert!(r.change_sets.len() >= 3, "two units a pass: {r:?}");
    for (id, created) in ids {
        assert_eq!(db.get_unit(id).unwrap().unwrap().oldest, RevNo(3));
        assert_eq!(db.unit_as_of(id, AsOf::Seq(created)).unwrap(), UnitAt::Pruned { oldest: RevNo(3) });
        assert!(matches!(
            db.history(id, HistoryQuery { from: Some(RevNo(1)), ..HistoryQuery::oldest(10) }),
            Err(UnitError::Pruned { .. })
        ));
    }
    assert!(db.verify_units().unwrap().is_clean());
    // Idempotent: nothing left to do.
    let again = db.run_retention_all(&RetentionPolicy::all(RetentionRule::KeepLast(1))).unwrap();
    assert_eq!((again.units_retained, again.change_set), (0, None));
}
