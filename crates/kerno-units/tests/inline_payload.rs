//! The payload a `UNIT_HEAD` carries (`docs/file-format/units-v11.md`).
//!
//! A unit whose payload is at most `INLINE_PAYLOAD_MAX` bytes is written with
//! that payload in its head only: its head revision has no `REV_PAYLOAD`, and
//! reading it is one point read. A later revision writes the `REV_PAYLOAD` of
//! the one it supersedes when the history keeps it. These tests pin what is
//! stored, that a read still refuses a damaged payload, what the one-read path
//! does not look at (and that `verify_units` and a write still do), and that
//! history is untouched. Every test runs on `emulated_log` and on `kerno_db`.

#![cfg(feature = "test-support")]

mod harness;

use std::sync::Arc;

use harness::units::{Target, open_units};
use kerno_core::contract::Options;
use kerno_storage::KernoDb;
use kerno_testkit::vfs::SimVfs;
use kerno_units::codec::{INLINE_PAYLOAD_MAX, Key, RevisionMeta, UnitHead};
use kerno_units::*;

type EmuLog = kerno_testkit::emulated::Emulated<kerno_testkit::reference::LogEngine>;

const T0: Timestamp = Timestamp(1_700_000_000_000_000);

fn open<E: Target>() -> UnitDb<E> {
    let mut c = UnitConfig::with_sources(Arc::new(FixedClock::new(T0)), Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    open_units::<E>(SimVfs::new(5).as_vfs(), &c).expect("open")
}

fn create<E: Target>(db: &UnitDb<E>, payload: Vec<u8>) -> UnitId {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let u = cs.create(NewUnit { unit_type: "doc".into(), schema_version: 1, metadata: vec![], payload }).unwrap();
    cs.commit().unwrap();
    u
}

fn update<E: Target>(db: &UnitDb<E>, u: UnitId, payload: Vec<u8>) -> Result<RevNo, UnitError> {
    let mut cs = db.begin_changeset(ChangeMeta::default())?;
    let rev = cs.update(u, UnitUpdate { payload, schema_version: None, metadata: None }, None)?;
    cs.commit()?;
    Ok(rev)
}

fn head_key(u: UnitId) -> Vec<u8> {
    Key::UnitHead { unit: u }.encode().unwrap()
}

fn stored_head<E: Target>(db: &UnitDb<E>, u: UnitId) -> UnitHead {
    UnitHead::decode(&db.raw_get(&head_key(u)).unwrap().expect("the head exists")).unwrap()
}

fn is_corruption<T>(r: &Result<T, UnitError>) -> bool {
    matches!(r, Err(UnitError::Storage(kerno_core::Error::Corruption(_))))
}

fn rules(report: &UnitVerifyReport) -> Vec<String> {
    report.findings.iter().map(|f| f.rule.id()).collect()
}

/// A small payload is in the head; one over the limit is not; the head always
/// carries the payload of the revision it names, and a deleted unit none.
fn a_writer_stores_small_payloads_in_the_head<E: Target>() {
    let db = open::<E>();
    let max = INLINE_PAYLOAD_MAX as usize;
    let small = create(&db, b"small".to_vec());
    let empty = create(&db, Vec::new());
    let edge = create(&db, vec![7; max]);
    let large = create(&db, vec![9; max + 1]);
    assert_eq!(stored_head(&db, small).inline_payload.as_deref(), Some(&b"small"[..]));
    assert_eq!(stored_head(&db, empty).inline_payload.as_deref(), Some(&[][..]));
    assert_eq!(stored_head(&db, edge).inline_payload, Some(vec![7; max]));
    assert_eq!(stored_head(&db, large).inline_payload, None);
    let rev_payload = |u, rev| db.raw_get(&Key::RevPayload { unit: u, rev: RevNo(rev) }.encode().unwrap()).unwrap();
    for (u, want) in [(small, b"small".to_vec()), (empty, vec![]), (edge, vec![7; max]), (large, vec![9; max + 1])] {
        assert_eq!(db.get_unit(u).unwrap().unwrap().payload, Some(want.clone()));
        // The payload is stored once: in the head when it fits there, else in the revision.
        let stored = if u == large { Some(want) } else { None };
        assert_eq!(rev_payload(u, 1), stored);
    }

    // Growing past the limit drops the copy, shrinking brings it back; each revision the
    // head leaves gets its REV_PAYLOAD (the default policy keeps the history).
    update(&db, small, vec![1; max + 1]).unwrap();
    assert_eq!(stored_head(&db, small).inline_payload, None);
    assert_eq!(db.get_unit(small).unwrap().unwrap().payload, Some(vec![1; max + 1]));
    assert_eq!(rev_payload(small, 1), Some(b"small".to_vec()));
    assert_eq!(rev_payload(small, 2), Some(vec![1; max + 1]));
    update(&db, small, b"again".to_vec()).unwrap();
    assert_eq!(stored_head(&db, small).inline_payload.as_deref(), Some(&b"again"[..]));
    assert_eq!(db.get_unit(small).unwrap().unwrap().payload.as_deref(), Some(&b"again"[..]));
    assert_eq!(db.get_unit(small).unwrap().unwrap().head, RevNo(3));
    assert_eq!(rev_payload(small, 3), None);

    // Archiving keeps the content, and the copy; deleting leaves none in the head. Both
    // revisions they supersede get their REV_PAYLOAD.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(empty, None).unwrap();
    cs.delete(edge, None).unwrap();
    cs.commit().unwrap();
    assert_eq!(stored_head(&db, empty).inline_payload.as_deref(), Some(&[][..]));
    assert_eq!(db.get_unit(empty).unwrap().unwrap().lifecycle, Lifecycle::Archived);
    assert_eq!((rev_payload(empty, 1), rev_payload(empty, 2)), (Some(vec![]), None));
    assert_eq!(stored_head(&db, edge).inline_payload, None);
    assert_eq!(db.get_unit(edge).unwrap().unwrap().payload, None);
    assert_eq!((rev_payload(edge, 1), rev_payload(edge, 2)), (Some(vec![7; max]), None));

    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
}

/// The copy in the head is checked on every read against the head's hash and
/// length, and by `verify_units`.
fn a_damaged_inline_payload_is_refused<E: Target>() {
    let db = open::<E>();
    let u = create(&db, b"content".to_vec());
    let good = db.raw_get(&head_key(u)).unwrap().unwrap();

    // Another payload of the same length: the hash no longer matches (UNIT-28).
    let mut flipped = good.clone();
    *flipped.last_mut().unwrap() ^= 0x01;
    db.raw_commit(vec![(head_key(u), Some(flipped))]).unwrap();
    assert!(is_corruption(&db.get_unit(u)), "{:?}", db.get_unit(u));
    assert!(is_corruption(&update(&db, u, b"next".to_vec())));
    assert_eq!(rules(&db.verify_units().unwrap()), ["UNIT-28"]);

    // A copy cut short or extended is not a UNIT_HEAD at all.
    for bad in [good[..good.len() - 1].to_vec(), [good.clone(), vec![0]].concat()] {
        db.raw_commit(vec![(head_key(u), Some(bad))]).unwrap();
        assert!(is_corruption(&db.get_unit(u)));
        assert!(!db.verify_units().unwrap().is_clean());
    }

    db.raw_commit(vec![(head_key(u), Some(good))]).unwrap();
    assert_eq!(db.get_unit(u).unwrap().unwrap().payload.as_deref(), Some(&b"content"[..]));
    assert!(db.verify_units().unwrap().is_clean());
}

/// What a one-read `get_unit` does not look at: the `REV_META` of the head
/// revision, and a `REV_PAYLOAD` it must not have. Damage there is found by
/// `verify_units` and refused by the next write to the unit and by a read of
/// the revision; a unit without a copy in its head is still read in full.
fn the_revision_records_are_checked_by_verify_and_by_writes<E: Target>() {
    let db = open::<E>();
    let u = create(&db, b"content".to_vec());
    let big = create(&db, vec![3; INLINE_PAYLOAD_MAX as usize + 1]);
    let meta_key = |u| Key::RevMeta { unit: u, rev: RevNo(1) }.encode().unwrap();
    let payload_key = |u| Key::RevPayload { unit: u, rev: RevNo(1) }.encode().unwrap();
    let other_meta = |u| {
        let raw = db.raw_get(&meta_key(u)).unwrap().unwrap();
        let m = RevisionMeta::decode(RevNo(1), &raw).unwrap();
        RevisionMeta { metadata: b"other".to_vec(), ..m }.encode(RevNo(1)).unwrap()
    };

    for (name, key, damage) in [
        ("REV_META disagrees with the head", meta_key(u), Some(other_meta(u))),
        ("REV_META missing", meta_key(u), None),
        ("a REV_PAYLOAD beside the copy", payload_key(u), Some(b"content".to_vec())),
        ("a REV_PAYLOAD of other bytes beside the copy", payload_key(u), Some(b"CONTENT".to_vec())),
    ] {
        let original = db.raw_get(&key).unwrap();
        db.raw_commit(vec![(key.clone(), damage)]).unwrap();
        // The head is intact and carries a payload that matches its hash: the read answers.
        assert_eq!(db.get_unit(u).unwrap().unwrap().payload.as_deref(), Some(&b"content"[..]), "{name}");
        assert!(!db.verify_units().unwrap().is_clean(), "{name}: verify must find it");
        assert!(is_corruption(&update(&db, u, b"next".to_vec())), "{name}: a write must refuse it");
        db.raw_commit(vec![(key, original)]).unwrap();
        assert!(db.verify_units().unwrap().is_clean(), "{name}: clean after the undo");
    }

    // Without a copy in the head the read is the three records, as before.
    for (name, key, damage) in [
        ("REV_META disagrees with the head", meta_key(big), Some(other_meta(big))),
        ("REV_PAYLOAD missing", payload_key(big), None),
    ] {
        let original = db.raw_get(&key).unwrap().unwrap();
        db.raw_commit(vec![(key.clone(), damage)]).unwrap();
        assert!(is_corruption(&db.get_unit(big)), "{name}");
        db.raw_commit(vec![(key, Some(original))]).unwrap();
        db.get_unit(big).unwrap().unwrap();
    }
}

/// A head that carries a payload its head revision does not have (UNIT-31), and a small
/// payload kept in the REV_PAYLOAD instead of the head (allowed: the read falls back to the
/// revision; a writer of this layer never makes one).
fn a_head_copy_is_optional_but_never_extra<E: Target>() {
    let db = open::<E>();
    let u = create(&db, b"content".to_vec());
    let head = stored_head(&db, u);

    let without = UnitHead { inline_payload: None, ..head.clone() }.encode().unwrap();
    let payload_key1 = Key::RevPayload { unit: u, rev: RevNo(1) }.encode().unwrap();
    db.raw_commit(vec![(head_key(u), Some(without)), (payload_key1, Some(b"content".to_vec()))]).unwrap();
    assert_eq!(db.get_unit(u).unwrap().unwrap().payload.as_deref(), Some(&b"content"[..]));
    assert!(db.verify_units().unwrap().is_clean());
    // The next revision is written with its copy again.
    update(&db, u, b"second".to_vec()).unwrap();
    assert_eq!(stored_head(&db, u).inline_payload.as_deref(), Some(&b"second"[..]));

    // A delete revision has no payload; a head naming it must not carry one. The codec refuses
    // the head of a Deleted unit outright, so this is an Archived head over a revision whose
    // payload was dropped.
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.archive(u, None).unwrap();
    cs.commit().unwrap();
    let top = stored_head(&db, u).head_rev;
    let meta_key = Key::RevMeta { unit: u, rev: top }.encode().unwrap();
    let meta = RevisionMeta::decode(top, &db.raw_get(&meta_key).unwrap().unwrap()).unwrap();
    let dropped = RevisionMeta { payload_present: false, ..meta }.encode(top).unwrap();
    let payload_key = Key::RevPayload { unit: u, rev: top }.encode().unwrap();
    db.raw_commit(vec![(meta_key, Some(dropped)), (payload_key, None)]).unwrap();
    let found = rules(&db.verify_units().unwrap());
    assert!(found.iter().all(|r| r == "UNIT-31") && found.len() >= 2, "{found:?}");
}

/// History, `read_as_of` and restore read the revision records, which a head
/// copy does not replace.
fn history_does_not_depend_on_the_head_copy<E: Target>() {
    let db = open::<E>();
    let u = create(&db, b"one".to_vec());
    let first = db.raw_seq().unwrap();
    update(&db, u, b"two".to_vec()).unwrap();
    update(&db, u, b"three".to_vec()).unwrap();
    let revs = db.history(u, HistoryQuery { with_payload: true, ..HistoryQuery::latest(10) }).unwrap();
    let payloads: Vec<_> = revs.iter().map(|r| r.payload.clone().unwrap()).collect();
    assert_eq!(payloads.len(), 3);
    assert!(payloads.contains(&b"one".to_vec()) && payloads.contains(&b"three".to_vec()));
    match db.unit_as_of(u, AsOf::Seq(CommitSeq(first))).unwrap() {
        UnitAt::Present(r) => assert_eq!(r.payload.as_deref(), Some(&b"one"[..])),
        other => panic!("{other:?}"),
    }
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    cs.restore(u, RevNo(1), None).unwrap();
    cs.commit().unwrap();
    assert_eq!(stored_head(&db, u).inline_payload.as_deref(), Some(&b"one"[..]));
    assert_eq!(db.get_unit(u).unwrap().unwrap().payload.as_deref(), Some(&b"one"[..]));
    assert!(db.verify_units().unwrap().is_clean());
}

macro_rules! engines {
    ($($m:ident: $E:ty),*) => {$(
        mod $m {
            use super::*;
            type E = $E;
            #[test]
            fn a_writer_stores_small_payloads_in_the_head() {
                super::a_writer_stores_small_payloads_in_the_head::<E>();
            }
            #[test]
            fn a_damaged_inline_payload_is_refused() {
                super::a_damaged_inline_payload_is_refused::<E>();
            }
            #[test]
            fn the_revision_records_are_checked_by_verify_and_by_writes() {
                super::the_revision_records_are_checked_by_verify_and_by_writes::<E>();
            }
            #[test]
            fn a_head_copy_is_optional_but_never_extra() {
                super::a_head_copy_is_optional_but_never_extra::<E>();
            }
            #[test]
            fn history_does_not_depend_on_the_head_copy() {
                super::history_does_not_depend_on_the_head_copy::<E>();
            }
        }
    )*};
}

engines!(emulated_log: EmuLog, kerno: KernoDb);

// ------------------------------------------------------------------ codec

fn head(payload: &[u8]) -> UnitHead {
    UnitHead {
        unit_type: "note".into(),
        schema_version: 1,
        lifecycle: Lifecycle::Active,
        head_rev: RevNo(2),
        oldest_rev: RevNo(1),
        created_seq: CommitSeq(1),
        updated_seq: CommitSeq(2),
        created_at: Timestamp(-1),
        updated_at: Timestamp(5),
        content_hash: ContentHash::blake3(0, payload),
        payload_len: payload.len() as u32,
        metadata: b"m".to_vec(),
        inline_payload: Some(payload.to_vec()),
    }
}

#[test]
fn the_head_codec_roundtrips_and_bounds_the_inline_payload() {
    for payload in [&b""[..], b"x", &[0u8; INLINE_PAYLOAD_MAX as usize]] {
        let h = head(payload);
        let bytes = h.encode().unwrap();
        assert_eq!(UnitHead::decode(&bytes).unwrap(), h);
        // The record without the copy is the layer 1 record, and a prefix of the one with it.
        let plain = UnitHead { inline_payload: None, ..h.clone() }.encode().unwrap();
        assert_eq!(bytes[..plain.len()], plain[..]);
        assert_eq!(bytes[plain.len()], 1);
        assert_eq!(&bytes[plain.len() + 1..], payload);
    }
    let rule = |v: &[u8]| UnitHead::decode(v).unwrap_err().rule.id();
    let good = head(b"abc").encode().unwrap();
    assert_eq!(rule(&good[..good.len() - 1]), "UNIT-9", "a copy shorter than payload_len");
    assert_eq!(rule(&[good.clone(), vec![0]].concat()), "UNIT-10", "bytes after the copy");
    let plain = UnitHead { inline_payload: None, ..head(b"abc") }.encode().unwrap();
    assert_eq!(rule(&[plain.clone(), vec![2, b'a', b'b', b'c']].concat()), "UNIT-10", "an unknown trailer");

    // What the encoder refuses is what the decoder refuses.
    let wrong_len = UnitHead { payload_len: 4, ..head(b"abc") };
    assert!(wrong_len.encode().is_err());
    let deleted = UnitHead { lifecycle: Lifecycle::Deleted, ..head(b"") };
    assert!(deleted.encode().is_err());
    let over = vec![0u8; INLINE_PAYLOAD_MAX as usize + 1];
    assert!(head(&over).encode().is_err());
    assert_eq!(UnitHead::inline_of(Some(&over)), None);
    assert_eq!(UnitHead::inline_of(Some(b"abc")), Some(b"abc".to_vec()));
    assert_eq!(UnitHead::inline_of(None), None);
}
