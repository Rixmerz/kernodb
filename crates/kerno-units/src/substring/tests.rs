//! Tests of the trigram index on the emulated engine and a simulated disk,
//! like those of the keyword index.

use std::path::Path;
use std::sync::Arc;

use kerno_core::contract::Options;
use kerno_testkit::emulated::Emulated;
use kerno_testkit::reference::LogEngine;
use kerno_testkit::vfs::SimVfs;

use super::*;
use crate::{ChangeMeta, CounterIds, FixedClock, NewUnit, Timestamp, UnitConfig, UnitUpdate};

type E = Emulated<LogEngine>;

fn open_plain(disk: &SimVfs) -> UnitDb<E> {
    let clock = Arc::new(FixedClock::new(Timestamp(1_700_000_000_000_000)));
    let mut c = UnitConfig::with_sources(clock, Arc::new(CounterIds::new(1)));
    c.engine = Options::small_for_tests();
    let engine = E::open(disk.as_vfs(), Path::new("db"), &c.engine).expect("open engine");
    UnitDb::from_engine(engine, &c).expect("open layer")
}

/// The layer with the trigram index of the type `sym` on.
fn open(disk: &SimVfs) -> UnitDb<E> {
    let db = open_plain(disk);
    db.set_substring("sym", true, ChangeMeta::default()).expect("index sym");
    db
}

fn change<T>(db: &UnitDb<E>, f: impl FnOnce(&mut ChangeSet<'_, E>) -> T) -> T {
    let mut cs = db.begin_changeset(ChangeMeta::default()).unwrap();
    let out = f(&mut cs);
    cs.commit().unwrap();
    out
}

fn store_as(db: &UnitDb<E>, unit_type: &str, payload: &[u8]) -> UnitId {
    let new = NewUnit { unit_type: unit_type.into(), schema_version: 1, metadata: vec![], payload: payload.to_vec() };
    change(db, |cs| cs.create(new).unwrap())
}

fn store(db: &UnitDb<E>, payload: &[u8]) -> UnitId {
    store_as(db, "sym", payload)
}

fn update(db: &UnitDb<E>, id: UnitId, payload: &[u8]) {
    let upd = UnitUpdate { payload: payload.to_vec(), schema_version: None, metadata: None };
    change(db, |cs| cs.update(id, upd, None).unwrap());
}

fn found(db: &UnitDb<E>, needle: &[u8]) -> Vec<UnitId> {
    db.search_substring(needle, None, None).unwrap()
}

/// Every trigram entry of the store, as `(key, value)`.
fn entries(db: &UnitDb<E>) -> Vec<(Vec<u8>, Vec<u8>)> {
    let snap = db.raw_snapshot().unwrap();
    snap.scan(Bound::Included(&[TAG_TRIGRAM][..]), Bound::Excluded(&[TAG_TRIGRAM + 1][..]), usize::MAX).unwrap()
}

fn rules(db: &UnitDb<E>) -> Vec<String> {
    db.verify_units().unwrap().rules().into_iter().collect()
}

fn assert_clean(db: &UnitDb<E>) {
    let report = db.verify_units().unwrap();
    assert!(report.is_clean(), "verify_units found {:?}", report.findings);
}

fn invalid(r: Result<Vec<UnitId>, UnitError>) -> bool {
    matches!(r, Err(UnitError::InvalidArgument(_)))
}

// ---------------------------------------------------------------- grams

#[test]
fn grams_fold_ascii_pad_the_end_and_are_distinct() {
    assert_eq!(grams(b"AbC"), [*b"abc", [b'b', b'c', 0], [b'c', 0, 0]]);
    // A byte that is not an ASCII letter is kept as it is: É is not folded to é.
    assert_eq!(grams("É".as_bytes()), [[0xC3, 0x89, 0], [0x89, 0, 0]].map(|g: Gram| g).into_iter().rev().collect::<Vec<_>>());
    assert_eq!(grams(b"aaaa"), [[b'a', 0, 0], [b'a', b'a', 0], *b"aaa"]);
    assert!(grams(b"").is_empty());
}

#[test]
fn only_the_first_bytes_of_a_payload_are_indexed_and_searched() {
    let disk = SimVfs::new(1);
    let db = open(&disk);
    let mut long = vec![b'x'; MAX_INDEXED_BYTES - 3];
    long.extend_from_slice(b"endTAIL");
    let id = store(&db, &long);
    // "end" is the last trigram inside the limit; "tail" lies past it.
    assert_eq!(found(&db, b"xend"), [id]);
    assert_eq!(found(&db, b"end"), [id]);
    assert!(found(&db, b"endt").is_empty());
    assert!(found(&db, b"tail").is_empty());
    assert!(found(&db, b"ail").is_empty());
    assert_clean(&db);
}

// --------------------------------------------------------------- search

#[test]
fn a_needle_of_any_length_is_found_in_either_case_in_id_order() {
    let disk = SimVfs::new(2);
    let db = open(&disk);
    let a = store(&db, b"pkg.Parser.parseChangeSet(input)");
    let b = store(&db, b"pkg.Writer.flush()");
    let c = store(&db, b"other.PARSER.reset()");
    assert_eq!(found(&db, b"p"), [a, b, c]);
    assert_eq!(found(&db, b"pk"), [a, b]);
    assert_eq!(found(&db, b"PKG"), [a, b]);
    assert_eq!(found(&db, b"parser"), [a, c]);
    assert_eq!(found(&db, b"ChangeSet"), [a]);
    assert_eq!(found(&db, b"changeset(input)"), [a]);
    assert_eq!(found(&db, b"()"), [b, c]);
    assert!(found(&db, b"parserx").is_empty());
    assert!(found(&db, b"zz").is_empty());
    assert!(found(&db, b"q").is_empty());
    assert_clean(&db);
}

#[test]
fn a_unit_with_every_trigram_but_not_the_needle_is_not_a_hit() {
    let disk = SimVfs::new(3);
    let db = open(&disk);
    let apart = store(&db, b"abc bcd");
    let whole = store(&db, b"xabcdx");
    assert_eq!(found(&db, b"abcd"), [whole]);
    assert_eq!(found(&db, b"abc"), [apart, whole]);
}

#[test]
fn the_padding_of_the_last_grams_is_not_content() {
    let disk = SimVfs::new(4);
    let db = open(&disk);
    let plain = store(&db, b"ab");
    let nul = store(&db, b"ab\0c");
    // The grams of "ab" are padded with 0x00, but "ab" holds no 0x00.
    assert_eq!(found(&db, b"b\0"), [nul]);
    assert_eq!(found(&db, b"ab\0"), [nul]);
    assert_eq!(found(&db, b"\0"), [nul]);
    assert_eq!(found(&db, b"ab"), [plain, nul]);
}

#[test]
fn bytes_that_are_not_text_are_searched_too() {
    let disk = SimVfs::new(5);
    let db = open(&disk);
    let id = store(&db, &[0xFF, 0xFE, 0xFF, 0xFF, 0x00, 0x80]);
    assert_eq!(found(&db, &[0xFE, 0xFF, 0xFF]), [id]);
    assert_eq!(found(&db, &[0xFF, 0xFF]), [id]);
    assert_eq!(found(&db, &[0xFF]), [id]);
    assert!(found(&db, &[0xFE, 0xFE]).is_empty());
    assert_clean(&db);
}

#[test]
fn limit_keeps_the_first_units_in_id_order() {
    let disk = SimVfs::new(6);
    let db = open(&disk);
    let ids: Vec<UnitId> = (0..5).map(|i| store(&db, format!("needle {i}").as_bytes())).collect();
    for needle in [&b"ne"[..], b"nee", b"needle"] {
        assert_eq!(db.search_substring(needle, Some(2), None).unwrap(), ids[..2]);
        assert_eq!(db.search_substring(needle, Some(9), None).unwrap(), ids);
        assert_eq!(db.search_substring(needle, None, None).unwrap(), ids);
    }
}

#[test]
fn a_search_that_could_never_match_is_an_error() {
    let disk = SimVfs::new(7);
    let db = open_plain(&disk);
    store_as(&db, "note", b"hello");
    // No type has the index.
    assert!(matches!(db.search_substring(b"hel", None, None), Err(UnitError::NotIndexed { .. })));
    db.set_substring("sym", true, ChangeMeta::default()).unwrap();
    assert!(found(&db, b"hel").is_empty(), "the index is on for a type without units");
    assert!(invalid(db.search_substring(b"hel", Some(0), None)));
    assert!(invalid(db.search_substring(b"", None, None)));
    assert!(invalid(db.search_substring(&[b'a'; MAX_NEEDLE_LEN + 1], None, None)));
    assert!(found(&db, &[b'a'; MAX_NEEDLE_LEN]).is_empty());
    assert!(invalid(db.search_substring(b"hel", None, Some(&[]))));
    assert!(invalid(db.search_substring(b"hel", None, Some(&[""]))));
    assert!(invalid(db.search_substring(b"hel", None, Some(&["t".repeat(65).as_str()]))));
    assert!(matches!(
        db.search_substring(b"hel", None, Some(&["nothing"])),
        Err(UnitError::UnknownType { unit_type }) if unit_type == "nothing"
    ));
    // A type that exists, without the index.
    assert!(matches!(db.search_substring(b"hel", None, Some(&["note"])), Err(UnitError::NotIndexed { .. })));
}

#[test]
fn types_restrict_the_search_and_the_answers_merge_in_id_order() {
    let disk = SimVfs::new(8);
    let db = open(&disk);
    db.set_substring("doc", true, ChangeMeta::default()).unwrap();
    let s1 = store(&db, b"shared text");
    let d1 = store_as(&db, "doc", b"shared text");
    let n1 = store_as(&db, "note", b"shared text");
    let s2 = store(&db, b"shared text");
    assert_eq!(found(&db, b"shared"), [s1, d1, s2], "a type without the index is not searched");
    assert_eq!(db.search_substring(b"shared", None, Some(&["sym"])).unwrap(), [s1, s2]);
    assert_eq!(db.search_substring(b"shared", None, Some(&["doc"])).unwrap(), [d1]);
    assert_eq!(db.search_substring(b"shared", None, Some(&["doc", "sym"])).unwrap(), [s1, d1, s2]);
    assert_eq!(db.search_substring(b"shared", Some(2), None).unwrap(), [s1, d1]);
    assert!(!entries(&db).iter().any(|(k, _)| decode_key(k).unwrap().2 == n1));
    assert!(db.snapshot().unwrap().substring_enabled("doc").unwrap());
    assert!(!db.snapshot().unwrap().substring_enabled("note").unwrap());
    assert_clean(&db);
}

// ----------------------------------------------------------- lifecycle

#[test]
fn the_index_follows_updates_archives_and_deletes() {
    let disk = SimVfs::new(9);
    let db = open(&disk);
    let id = store(&db, b"first version");
    let other = store(&db, b"another unit");
    update(&db, id, b"second version");
    assert!(found(&db, b"first").is_empty());
    assert_eq!(found(&db, b"second"), [id]);
    assert_eq!(found(&db, b"version"), [id]);
    assert_clean(&db);

    change(&db, |cs| cs.archive(id, None).unwrap());
    assert!(found(&db, b"second").is_empty(), "an archived unit is not found");
    assert_clean(&db);
    change(&db, |cs| cs.unarchive(id, None).unwrap());
    assert_eq!(found(&db, b"second"), [id]);
    assert_clean(&db);

    change(&db, |cs| cs.delete(id, None).unwrap());
    assert!(found(&db, b"version").is_empty());
    assert_eq!(found(&db, b"another"), [other]);
    assert!(entries(&db).iter().all(|(k, _)| decode_key(k).unwrap().2 == other));
    assert_clean(&db);
}

#[test]
fn the_option_indexes_the_units_a_type_already_has_and_clears_them() {
    let disk = SimVfs::new(10);
    let db = open_plain(&disk);
    let a = store(&db, b"written before the index");
    let archived = store(&db, b"written and archived");
    change(&db, |cs| cs.archive(archived, None).unwrap());
    assert!(entries(&db).is_empty());

    let before = db.snapshot().unwrap().seq();
    db.set_substring("sym", true, ChangeMeta::default()).unwrap();
    assert_eq!(found(&db, b"written"), [a], "only the active units are indexed");
    assert_clean(&db);
    // The state the type already has writes nothing.
    let on = db.snapshot().unwrap().seq();
    assert!(on > before);
    db.set_substring("sym", true, ChangeMeta::default()).unwrap();
    assert_eq!(db.snapshot().unwrap().seq(), on);

    db.set_substring("sym", false, ChangeMeta::default()).unwrap();
    assert!(entries(&db).is_empty());
    assert!(!db.snapshot().unwrap().substring_enabled("sym").unwrap());
    assert!(matches!(db.search_substring(b"written", None, None), Err(UnitError::NotIndexed { .. })));
    assert_clean(&db);
    // A unit written while the index is off has no entry.
    store(&db, b"written without the index");
    assert!(entries(&db).is_empty());
    assert!(invalid(db.set_substring("", true, ChangeMeta::default()).map(|_| Vec::new())));
}

#[test]
fn the_two_options_of_a_type_are_independent() {
    let disk = SimVfs::new(11);
    let db = open(&disk);
    db.set_fulltext("sym", true, ChangeMeta::default()).unwrap();
    let id = store(&db, b"alpha beta");
    assert_eq!(found(&db, b"lph"), [id]);
    assert_eq!(db.search_text("beta", 5, None).unwrap().len(), 1);
    db.set_fulltext("sym", false, ChangeMeta::default()).unwrap();
    assert_eq!(found(&db, b"lph"), [id], "turning the keyword index off leaves the trigram index");
    assert!(db.snapshot().unwrap().substring_enabled("sym").unwrap());
    db.set_fulltext("sym", true, ChangeMeta::default()).unwrap();
    db.set_substring("sym", false, ChangeMeta::default()).unwrap();
    assert_eq!(db.search_text("beta", 5, None).unwrap().len(), 1);
    assert!(db.snapshot().unwrap().fulltext_enabled("sym").unwrap());
    assert_clean(&db);
}

#[test]
fn a_change_set_indexes_under_the_option_it_set_itself() {
    let disk = SimVfs::new(12);
    let db = open_plain(&disk);
    let new = |p: &[u8]| NewUnit { unit_type: "sym".into(), schema_version: 1, metadata: vec![], payload: p.to_vec() };
    let (before, after) = change(&db, |cs| {
        let before = cs.create(new(b"stored first")).unwrap();
        cs.set_substring("sym", true).unwrap();
        (before, cs.create(new(b"stored after")).unwrap())
    });
    assert_eq!(found(&db, b"stored"), [before, after]);
    assert_clean(&db);
}

#[test]
fn the_first_use_raises_the_layer_mark_to_8_and_it_survives_a_reopen() {
    let disk = SimVfs::new(13);
    {
        let db = open_plain(&disk);
        store(&db, b"plain");
        assert_eq!(db.snapshot().unwrap().layer_version().unwrap(), Some(crate::codec::MIN_LAYER_VERSION));
        db.set_substring("sym", true, ChangeMeta::default()).unwrap();
        assert_eq!(db.snapshot().unwrap().layer_version().unwrap(), Some(crate::codec::LAYER_VERSION_SUBSTRING));
        db.set_substring("sym", false, ChangeMeta::default()).unwrap();
        assert_eq!(db.snapshot().unwrap().layer_version().unwrap(), Some(8), "the mark is never lowered");
        db.set_substring("sym", true, ChangeMeta::default()).unwrap();
        db.close().unwrap();
    }
    let db = open_plain(&disk);
    assert_eq!(found(&db, b"plain").len(), 1);
    assert_clean(&db);
}

// ------------------------------------------------------------ long lists

/// 300 units hold "common", 100 of them also "middle" (80 of those with "common" too far
/// to matter: every unit here has "common"), and one holds "rare".
#[test]
fn lists_longer_than_a_page_are_read_whole_probed_or_merged() {
    let disk = SimVfs::new(14);
    let db = open(&disk);
    let mut all = Vec::new();
    let mut middle = Vec::new();
    for i in 0..300 {
        let text = if i % 3 == 0 { format!("common middle {i}") } else { format!("common {i}") };
        let id = store(&db, text.as_bytes());
        all.push(id);
        if i % 3 == 0 {
            middle.push(id);
        }
    }
    let rare = store(&db, b"common rare");
    all.push(rare);
    assert!(all.len() > PAGE && middle.len() > PROBE_MAX && middle.len() < PAGE);
    // One long list, read past its first page: a three-byte needle and a two-byte one.
    assert_eq!(found(&db, b"com"), all);
    assert_eq!(found(&db, b"co"), all);
    assert_eq!(found(&db, b"c"), all);
    // Few candidates ("rar" is one unit) against the long lists of "common": probes.
    assert_eq!(found(&db, b"common rare"), [rare]);
    assert_eq!(found(&db, b"mon r"), [rare]);
    // More candidates than probes are worth ("mid" is 100 units): the long lists are read.
    assert_eq!(found(&db, b"common middle"), middle);
    assert_eq!(found(&db, b"on mid"), middle);
    // A trigram no unit has ends the search.
    assert!(found(&db, b"common zzz").is_empty());
    assert_eq!(db.search_substring(b"common", Some(3), None).unwrap(), all[..3]);
    assert_clean(&db);
}

/// The index against a scan of the payloads, on words built from a small alphabet so that
/// trigrams repeat across units.
#[test]
fn the_index_answers_what_a_scan_of_the_payloads_answers() {
    let disk = SimVfs::new(15);
    let db = open(&disk);
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |n: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % n
    };
    let alphabet = b"abAB. ";
    let mut text = |len: u64| -> Vec<u8> { (0..len).map(|_| alphabet[next(6) as usize]).collect() };
    let units: Vec<(UnitId, Vec<u8>)> = (0..120)
        .map(|i| {
            let payload = text(3 + i % 23);
            (store(&db, &payload), payload)
        })
        .collect();
    for len in 1..=7u64 {
        for _ in 0..25 {
            let needle = text(len);
            let lower = needle.to_ascii_lowercase();
            let want: Vec<UnitId> = units
                .iter()
                .filter(|(_, p)| p.to_ascii_lowercase().windows(lower.len()).any(|w| w == lower))
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(found(&db, &needle), want, "needle {:?}", String::from_utf8_lossy(&needle));
        }
    }
    assert_clean(&db);
}

// --------------------------------------------------------------- verify

fn entry_of(db: &UnitDb<E>, unit: UnitId) -> (Vec<u8>, Vec<u8>) {
    entries(db).into_iter().find(|(k, _)| decode_key(k).unwrap().2 == unit).expect("the unit has entries")
}

#[test]
fn verify_reports_a_missing_an_extra_and_a_misplaced_entry() {
    let disk = SimVfs::new(16);
    let db = open(&disk);
    let id = store(&db, b"hello world");
    let note = store_as(&db, "note", b"hello world");
    assert_clean(&db);
    let (k, v) = entry_of(&db, id);

    // One entry of an indexed unit is gone.
    db.raw_commit(vec![(k.clone(), None)]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    db.raw_commit(vec![(k.clone(), Some(v.clone()))]).unwrap();
    assert_clean(&db);

    // Every entry of an indexed unit is gone.
    let mine: Vec<_> = entries(&db).into_iter().filter(|(k, _)| decode_key(k).unwrap().2 == id).collect();
    db.raw_commit(mine.iter().map(|(k, _)| (k.clone(), None)).collect()).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    db.raw_commit(mine.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect()).unwrap();
    assert_clean(&db);

    // An entry the payload does not call for.
    let extra = entry_key(&type_prefix("sym"), *b"zzz", id);
    db.raw_commit(vec![(extra.clone(), Some(entry_value()))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    db.raw_commit(vec![(extra, None)]).unwrap();

    // An entry of a unit whose type has no index, and one of a unit that does not exist.
    for unit in [note, UnitId([0xEE; 16])] {
        let stray = entry_key(&type_prefix("sym"), *b"hel", unit);
        db.raw_commit(vec![(stray.clone(), Some(entry_value()))]).unwrap();
        assert_eq!(rules(&db), ["UNIT-52"]);
        db.raw_commit(vec![(stray, None)]).unwrap();
        assert_clean(&db);
    }

    // The right entries under another type.
    let moved: Vec<_> = mine.iter().map(|(k, v)| (entry_key(&type_prefix("doc"), decode_key(k).unwrap().1, id), v)).collect();
    let mut swap: Vec<(Vec<u8>, Option<Vec<u8>>)> = mine.iter().map(|(k, _)| (k.clone(), None)).collect();
    swap.extend(moved.iter().map(|(k, v)| (k.clone(), Some((*v).clone()))));
    db.raw_commit(swap).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
}

#[test]
fn verify_reports_an_entry_that_does_not_decode() {
    let disk = SimVfs::new(17);
    let db = open(&disk);
    let id = store(&db, b"hello");
    let (k, v) = entry_of(&db, id);
    for bad in [vec![], vec![0, 0], vec![1, 1], vec![1, 0, 0], vec![2, 0]] {
        db.raw_commit(vec![(k.clone(), Some(bad.clone()))]).unwrap();
        assert!(!db.verify_units().unwrap().is_clean(), "value {bad:?}");
    }
    db.raw_commit(vec![(k.clone(), Some(v.clone()))]).unwrap();
    assert_clean(&db);
    // A key one byte short, and one with an empty type.
    for bad_key in [k[..k.len() - 1].to_vec(), [&[TAG_TRIGRAM, 0][..], &[b'a'; 19]].concat()] {
        db.raw_commit(vec![(bad_key.clone(), Some(v.clone()))]).unwrap();
        assert!(!db.verify_units().unwrap().is_clean(), "key {bad_key:?}");
        db.raw_commit(vec![(bad_key, None)]).unwrap();
        assert_clean(&db);
    }
    assert!(decode_key(&[TAG_TRIGRAM]).is_err());
    assert!(decode_key(&[&[TAG_TRIGRAM, 1, 0xFF][..], &[b'a'; 19]].concat()).is_err(), "a type that is not UTF-8");
}

#[test]
fn verify_reports_the_index_below_layer_8() {
    let disk = SimVfs::new(18);
    let db = open(&disk);
    store(&db, b"hello");
    assert_clean(&db);
    // The mark as a database that never used the index has it.
    let mark = crate::codec::LayerMeta {
        layer_version: crate::codec::LAYER_VERSION_TYPE_OPTIONS,
        history_policy: db.config().default_policy,
    };
    let mark_key = Key::LayerMeta { name: crate::codec::LAYER_MARK_NAME.into() }.encode().unwrap();
    db.raw_commit(vec![(mark_key, Some(mark.encode().unwrap()))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
}

#[test]
fn a_search_refuses_an_entry_of_a_unit_that_does_not_exist() {
    let disk = SimVfs::new(19);
    let db = open(&disk);
    store(&db, b"hello world");
    let ghost = UnitId([0xEE; 16]);
    let stray: Vec<_> =
        grams(b"ghost").into_iter().map(|g| (entry_key(&type_prefix("sym"), g, ghost), Some(entry_value()))).collect();
    db.raw_commit(stray).unwrap();
    assert!(matches!(
        db.search_substring(b"ghost", None, None),
        Err(UnitError::Storage(kerno_core::Error::Corruption(m))) if m.contains("no UNIT_HEAD")
    ));
    // A truncated key in a list a search reads.
    let short = [&type_prefix("sym")[..], b"hel", &[0u8; 15]].concat();
    db.raw_commit(vec![(short, Some(entry_value()))]).unwrap();
    assert!(matches!(
        db.search_substring(b"hel", None, None),
        Err(UnitError::Storage(kerno_core::Error::Corruption(_)))
    ));
}
