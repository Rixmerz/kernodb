//! `compact` gives the space of deleted units back: it drops the payload of
//! every revision of a deleted unit that no retention policy keeps, and keeps
//! the trail (`REV_META`, tombstone). `compact_keeping` keeps what a policy
//! keeps. Live units and snapshots taken before are not touched.

use std::path::Path;

use kerno_core::vfs::dir_size;
use kerno_storage::KernoDb;
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

type Db = UnitDb<KernoDb>;

fn config() -> UnitConfig {
    UnitConfig {
        // Pacing only shares the write slot with other writers; there are none.
        retention_pacing: RetentionPacing { pause_ratio: 0, ..RetentionPacing::default() },
        ..UnitConfig::default()
    }
}

fn open(disk: &SimVfs) -> Db {
    UnitDb::open(disk.as_vfs(), Path::new("/db"), &config()).expect("open")
}

/// `len` bytes no compression shrinks, different for each `seed`.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = (seed + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn create(db: &Db, unit_type: &str, payload: Vec<u8>) -> UnitId {
    let new = NewUnit { unit_type: unit_type.into(), schema_version: 1, metadata: Vec::new(), payload };
    db.create_unit(new, ChangeMeta::default()).unwrap().0
}

fn update(db: &Db, id: UnitId, payload: Vec<u8>) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.update(id, UnitUpdate { payload, schema_version: None, metadata: None }, None).unwrap();
    cs.commit().unwrap();
}

fn delete(db: &Db, id: UnitId) {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.delete(id, None).unwrap();
    cs.commit().unwrap();
}

fn payload_of(db: &Db, id: UnitId, rev: u64) -> Result<Option<Vec<u8>>, UnitError> {
    db.snapshot().unwrap().get_revision(id, RevNo(rev)).map(|r| r.payload)
}

fn is_unavailable(r: &Result<Option<Vec<u8>>, UnitError>) -> bool {
    matches!(r, Err(UnitError::HistoryUnavailable { .. }))
}

/// The issue's case: 50 MB of payloads, all deleted, still 50 MB on disk
/// after a close and a reopen; `compact` brings the store down to its trail.
#[test]
fn deleting_fifty_megabytes_and_compacting_shrinks_the_store() {
    const N: u64 = 50;
    const LEN: usize = 1_000_000;
    let disk = SimVfs::new(11);
    let vfs = disk.as_vfs();
    let size = || dir_size(&*vfs, Path::new("/db")).unwrap();
    let db = open(&disk);
    let ids: Vec<UnitId> = (0..N).map(|k| create(&db, "blob", noise(k, LEN))).collect();
    db.close().unwrap();
    let full = size();
    assert!(full > N * LEN as u64, "{full} bytes hold the {N} payloads");

    let db = open(&disk);
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    for id in &ids {
        cs.delete(*id, None).unwrap();
    }
    cs.commit().unwrap();
    db.close().unwrap();
    assert!(size() >= full, "deleting alone reclaims nothing: {} of {full}", size());

    let db = open(&disk);
    let report = db.compact().unwrap();
    assert_eq!(report.payloads_dropped, N, "{report:?}");
    assert!(report.bytes_reclaimed().unwrap() > 40_000_000, "the live tables shrank: {report:?}");
    db.close().unwrap();
    let after = size();
    assert!(after < full / 10, "after compact and a close the store is {after} bytes, was {full}");

    // What is left verifies, and the trail names every revision.
    let db = open(&disk);
    assert!(db.verify_units().unwrap().is_clean());
    for id in &ids {
        let revs = db.snapshot().unwrap().history(*id, HistoryQuery::oldest(10)).unwrap();
        assert_eq!(revs.len(), 2, "the create and the tombstone stay");
        assert_eq!(revs[0].payload_len as usize, LEN, "the length and the hash outlive the payload");
        assert!(is_unavailable(&payload_of(&db, *id, 1)));
    }
    db.close().unwrap();
}

#[test]
fn compact_drops_every_payload_of_deleted_units_and_only_theirs() {
    let disk = SimVfs::new(12);
    let db = open(&disk);
    let gone = create(&db, "note", noise(1, 5000));
    update(&db, gone, noise(2, 5000));
    update(&db, gone, noise(3, 5000));
    let live = create(&db, "note", noise(4, 5000));
    update(&db, live, noise(5, 5000));
    let archived = create(&db, "note", noise(6, 5000));
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(archived, None).unwrap();
    cs.commit().unwrap();
    delete(&db, gone);
    let before = db.snapshot().unwrap();

    let report = db.compact().unwrap();
    assert_eq!(report.payloads_dropped, 3, "revisions 1..=3 of the deleted unit: {report:?}");
    for rev in 1..=3 {
        assert!(is_unavailable(&payload_of(&db, gone, rev)), "revision {rev} of the deleted unit");
    }
    let tombstone = payload_of(&db, gone, 4);
    assert!(matches!(tombstone, Ok(None)), "the tombstone stays: {tombstone:?}");
    assert_eq!(payload_of(&db, live, 1).unwrap(), Some(noise(4, 5000)), "a live unit keeps its history");
    assert_eq!(payload_of(&db, live, 2).unwrap(), Some(noise(5, 5000)));
    assert_eq!(payload_of(&db, archived, 1).unwrap(), Some(noise(6, 5000)), "an archived unit is not deleted");
    assert_eq!(db.get_unit(live).unwrap().unwrap().payload, Some(noise(5, 5000)));
    // A snapshot taken before still reads the deleted unit's payloads (I-05).
    assert_eq!(before.get_revision(gone, RevNo(2)).unwrap().payload, Some(noise(2, 5000)));
    assert!(db.verify_units().unwrap().is_clean(), "{:?}", db.verify_units().unwrap().findings);
    // Nothing left to drop: a second compact writes no change set.
    let seq = db.snapshot().unwrap().seq();
    assert_eq!(db.compact().unwrap().payloads_dropped, 0);
    assert_eq!(db.snapshot().unwrap().seq(), seq);
    db.close().unwrap();

    // The result is durable: a reopen reads the same.
    let db = open(&disk);
    assert!(is_unavailable(&payload_of(&db, gone, 1)));
    assert!(db.verify_units().unwrap().is_clean());
    db.close().unwrap();
}

#[test]
fn compact_keeping_keeps_what_the_policy_keeps() {
    let disk = SimVfs::new(13);
    let db = open(&disk);
    let deleted = |unit_type: &str, base: u64| {
        let id = create(&db, unit_type, noise(base, 3000));
        update(&db, id, noise(base + 1, 3000));
        update(&db, id, noise(base + 2, 3000));
        delete(&db, id);
        id
    };
    let (a, b) = (deleted("keep", 10), deleted("lose", 20));

    // KeepAll keeps everything: nothing is dropped.
    let all = RetentionPolicy::all(RetentionRule::KeepAll);
    assert_eq!(db.compact_keeping(&all).unwrap().payloads_dropped, 0);
    assert_eq!(payload_of(&db, a, 1).unwrap(), Some(noise(10, 3000)));

    // KeepLast(1) keeps the revision before the tombstone (design §5.2), per
    // type: "keep" keeps all of its payloads, the default drops the rest.
    let policy = RetentionPolicy::all(RetentionRule::KeepLast(1)).with_type("keep", RetentionRule::KeepAll);
    assert_eq!(db.compact_keeping(&policy).unwrap().payloads_dropped, 2, "revisions 1 and 2 of \"lose\"");
    assert_eq!(payload_of(&db, a, 1).unwrap(), Some(noise(10, 3000)), "the type whose rule keeps all");
    assert!(is_unavailable(&payload_of(&db, b, 1)) && is_unavailable(&payload_of(&db, b, 2)));
    assert_eq!(payload_of(&db, b, 3).unwrap(), Some(noise(22, 3000)), "the revision before the tombstone");

    // Plain `compact` keeps nothing, whatever the rules the types had.
    assert_eq!(db.compact().unwrap().payloads_dropped, 3 + 1);
    assert!(is_unavailable(&payload_of(&db, a, 1)) && is_unavailable(&payload_of(&db, b, 3)));
    assert!(db.verify_units().unwrap().is_clean());
    assert!(matches!(
        db.compact_keeping(&RetentionPolicy::all(RetentionRule::KeepLast(0))),
        Err(UnitError::InvalidArgument(_))
    ));
    db.close().unwrap();
}

/// A pass is bounded (here to 4 units per change set): `compact` goes on until
/// every deleted unit is done, and sees units that sit behind live ones.
#[test]
fn compact_goes_on_until_every_deleted_unit_is_done() {
    let disk = SimVfs::new(14);
    let cfg = UnitConfig { max_changeset_ops: 4, ..config() };
    let db: Db = UnitDb::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap();
    let mut gone = Vec::new();
    for k in 0..11u64 {
        let id = create(&db, "note", noise(k, 2000));
        if k % 3 != 0 {
            update(&db, id, noise(k + 100, 2000));
            delete(&db, id);
            gone.push(id);
        }
    }
    assert_eq!(db.compact().unwrap().payloads_dropped, 2 * gone.len() as u64);
    for id in gone {
        assert!(is_unavailable(&payload_of(&db, id, 1)) && is_unavailable(&payload_of(&db, id, 2)));
    }
    assert!(db.verify_units().unwrap().is_clean());
    db.close().unwrap();
}
