//! A damaged block is more than one error: the engine's `verify` names the key
//! range of every damaged block and `damaged_spans` says, in the layer's terms,
//! which records and which unit ids that range can hold. The units of the
//! intact blocks are not named; detection stays strict (the layer's own
//! `verify_units` still stops with `Corruption`).

use std::path::Path;
use std::sync::Arc;

use kerno_storage::{KernoDb, VerifyOptions, verify};
use kerno_testkit::vfs::SimVfs;
use kerno_units::*;

const UNITS: usize = 40;
const T0: Timestamp = Timestamp(1_700_000_000_000_000);

/// About 4 KiB that no compression shrinks and no other unit repeats.
fn payload(k: usize) -> Vec<u8> {
    let mut x = (k as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..4000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

#[test]
fn damaged_block_names_the_units_behind_it() {
    let disk = SimVfs::new(7);
    let cfg = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    let db = UnitDb::<KernoDb>::open(disk.as_vfs(), Path::new("/db"), &cfg).unwrap();
    let ids: Vec<UnitId> = (0..UNITS)
        .map(|k| {
            let new =
                NewUnit { unit_type: "note".into(), schema_version: 1, metadata: Vec::new(), payload: payload(k) };
            db.create_unit(new, ChangeMeta::default()).unwrap().0
        })
        .collect();
    db.close().unwrap();

    // The payload of unit 17 is in some block of some table: flip a bit in it.
    let target = 17;
    let mut image = disk.dump();
    let needle = payload(target)[100..140].to_vec();
    let (file, at) = image
        .files
        .iter()
        .filter(|(p, _)| p.extension().is_some_and(|e| e == "sst"))
        .find_map(|(p, bytes)| bytes.windows(needle.len()).position(|w| w == needle).map(|at| (p.clone(), at)))
        .expect("the payload of unit 17 is in a table");
    image.files.get_mut(&file).unwrap()[at + 5] ^= 0x01;
    let damaged = SimVfs::restore(&image);

    let report = verify(damaged.as_vfs(), Path::new("/db"), &VerifyOptions::default()).unwrap();
    assert!(!report.is_ok());
    assert_eq!(report.damaged.len(), 1, "one damaged block: {:?}", report.damaged);
    let range = &report.damaged[0];
    let spans = damaged_spans(&range.start, &range.end);
    let payloads: Vec<&DamagedSpan> = spans.iter().filter(|s| s.kind == "REV_PAYLOAD").collect();
    assert_eq!(payloads.len(), 1, "{spans:?}");
    assert!(payloads[0].may_hold(ids[target]), "the unit whose payload was damaged is named: {spans:?}");
    // The block holds a few payloads of about 4 KiB, not the table: the units
    // at both ends are in intact blocks and are not named.
    let named: Vec<usize> = (0..UNITS).filter(|&k| spans.iter().any(|s| s.may_hold(ids[k]))).collect();
    assert!(named.contains(&target) && named.len() < UNITS / 2, "{named:?}");
    assert!(!named.contains(&0) && !named.contains(&(UNITS - 1)), "{named:?}");

    // Detection is as strict as ever: the layer's verify and reads still fail.
    let db = KernoDb::open(
        damaged.as_vfs(),
        Path::new("/db"),
        &kerno_storage::Config {
            access: kerno_storage::AccessMode::ReadOnly,
            allow_unverified_platform: true,
            ..Default::default()
        },
    )
    .unwrap();
    let err = verify_engine(&db).expect_err("a damaged block is an error, never a partial report");
    assert!(matches!(err, UnitError::Storage(kerno_core::Error::Corruption(_))), "{err:?}");
}

#[test]
fn spans_cover_the_kinds_a_range_passes_through() {
    use kerno_units::codec::{TAG_CHANGESET, TAG_REV_META, TAG_REV_PAYLOAD, TAG_UNIT_HEAD};
    let key = |tag: u8, unit: u8| {
        let mut k = vec![tag];
        k.extend_from_slice(&[unit; 16]);
        k
    };
    // From inside UNIT_HEAD to inside REV_PAYLOAD: the head kind from its unit
    // on, REV_META whole, REV_PAYLOAD up to its unit; nothing above.
    let spans = damaged_spans(&key(TAG_UNIT_HEAD, 3), &key(TAG_REV_PAYLOAD, 9));
    let kinds: Vec<_> = spans.iter().map(|s| (s.kind, s.from_unit.is_some(), s.to_unit.is_some())).collect();
    assert_eq!(kinds, [("UNIT_HEAD", true, false), ("REV_META", false, false), ("REV_PAYLOAD", false, true)]);
    assert!(spans[1].may_hold(UnitId([0; 16])) && spans[1].may_hold(UnitId([0xff; 16])));
    assert!(spans[0].may_hold(UnitId([3; 16])) && !spans[0].may_hold(UnitId([2; 16])));
    assert!(spans[2].may_hold(UnitId([9; 16])) && !spans[2].may_hold(UnitId([10; 16])));
    // One kind, both bounds.
    let one = damaged_spans(&key(TAG_REV_META, 5), &key(TAG_REV_META, 7));
    assert_eq!(one.len(), 1);
    assert!(one[0].may_hold(UnitId([6; 16])) && !one[0].may_hold(UnitId([8; 16])) && !one[0].may_hold(UnitId([4; 16])));
    // Change sets are not keyed by a unit: the kind is named, no unit is.
    let cs = damaged_spans(&[TAG_CHANGESET, 0, 0], &[TAG_CHANGESET, 9]);
    assert_eq!((cs.len(), cs[0].kind, cs[0].keyed_by_unit), (1, "CHANGESET", false));
    assert!(!cs[0].may_hold(UnitId([1; 16])));
    assert!(damaged_spans(b"", b"x").is_empty());
}
