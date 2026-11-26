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

/// Every trigram block of the store, as `(key, value)`.
fn entries(db: &UnitDb<E>) -> Vec<(Vec<u8>, Vec<u8>)> {
    let snap = db.raw_snapshot().unwrap();
    snap.scan(Bound::Included(&[TAG_TRIGRAM][..]), Bound::Excluded(&[TAG_TRIGRAM + 1][..]), usize::MAX).unwrap()
}

/// Every trigram block of the store, decoded: `(key, units)`.
fn blocks(db: &UnitDb<E>) -> Vec<(Vec<u8>, Vec<UnitId>)> {
    entries(db).into_iter().map(|(k, v)| (k, decode_block(&v).unwrap())).collect()
}

/// The units the index holds, each once, in id order.
fn indexed_units(db: &UnitDb<E>) -> Vec<UnitId> {
    let mut out: Vec<UnitId> = blocks(db).into_iter().flat_map(|(_, u)| u).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The raw writes that take `unit` out of every block that holds it.
fn without(db: &UnitDb<E>, unit: UnitId) -> Vec<(Vec<u8>, Option<Vec<u8>>)> {
    blocks(db)
        .into_iter()
        .filter(|(_, units)| units.contains(&unit))
        .map(|(k, units)| {
            let rest: Vec<UnitId> = units.into_iter().filter(|u| *u != unit).collect();
            (k, (!rest.is_empty()).then(|| encode_block(&rest)))
        })
        .collect()
}

/// The key of the tail block of the list of `gram` in `unit_type`.
fn tail(unit_type: &str, gram: Gram) -> Vec<u8> {
    block_key(&list_key(&type_prefix(unit_type), gram), None)
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
    assert_eq!(
        grams("É".as_bytes()),
        [[0xC3, 0x89, 0], [0x89, 0, 0]].map(|g: Gram| g).into_iter().rev().collect::<Vec<_>>()
    );
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
    assert!(!indexed_units(&db).contains(&n1));
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
    assert_eq!(indexed_units(&db), [other]);
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
fn the_option_does_not_move_the_layer_mark_and_survives_a_reopen() {
    let disk = SimVfs::new(13);
    {
        let db = open_plain(&disk);
        store(&db, b"plain");
        let mark = |db: &UnitDb<E>| db.snapshot().unwrap().layer_version().unwrap();
        assert_eq!(mark(&db), Some(crate::codec::LAYER_VERSION_BLOCKS), "a database starts at the cut version");
        db.set_substring("sym", true, ChangeMeta::default()).unwrap();
        db.set_substring("sym", false, ChangeMeta::default()).unwrap();
        assert_eq!(mark(&db), Some(crate::codec::LAYER_VERSION_BLOCKS));
        db.set_substring("sym", true, ChangeMeta::default()).unwrap();
        db.close().unwrap();
    }
    let db = open_plain(&disk);
    assert_eq!(found(&db, b"plain").len(), 1);
    assert_clean(&db);
}

// ------------------------------------------------------------ long lists

/// 300 units hold "common", 100 of them also "middle", and one holds "rare": the list of
/// "com" is three blocks, more than a page in the tests.
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
    assert!(all.len() > PAGE * BLOCK_MAX && middle.len() > PROBE_MAX && middle.len() <= BLOCK_MAX);
    // One long list, read past its first page: a three-byte needle and a two-byte one.
    assert_eq!(found(&db, b"com"), all);
    assert_eq!(found(&db, b"co"), all);
    assert_eq!(found(&db, b"c"), all);
    // Few candidates ("rar" is one unit) against the long lists of "common": probes.
    assert_eq!(found(&db, b"common rare"), [rare]);
    assert_eq!(found(&db, b"mon r"), [rare]);
    // A probe that finds the block of a candidate without the candidate in it.
    assert!(found(&db, b"rare 1").is_empty());
    // More candidates than probes are worth ("mid" is 100 units): the long lists are read.
    assert_eq!(found(&db, b"common middle"), middle);
    assert_eq!(found(&db, b"on mid"), middle);
    // A trigram no unit has ends the search.
    assert!(found(&db, b"common zzz").is_empty());
    assert_eq!(db.search_substring(b"common", Some(3), None).unwrap(), all[..3]);
    assert_clean(&db);
}

#[test]
fn blocks_are_cut_at_their_limit_and_deleted_when_emptied() {
    let disk = SimVfs::new(20);
    let db = open(&disk);
    let new =
        |p: String| NewUnit { unit_type: "sym".into(), schema_version: 1, metadata: vec![], payload: p.into_bytes() };
    // One change set of 300 units: the tail of each list is cut as it is written.
    let ids: Vec<UnitId> = change(&db, |cs| {
        (0..300).map(|i| cs.create(new(if i == 50 { "other".into() } else { format!("word {i}") })).unwrap()).collect()
    });
    let list = list_key(&type_prefix("sym"), *b"wor");
    let wor: Vec<(Vec<u8>, Vec<UnitId>)> = blocks(&db).into_iter().filter(|(k, _)| k.starts_with(&list)).collect();
    assert_eq!(wor.iter().map(|(_, u)| u.len()).collect::<Vec<_>>(), [BLOCK_MAX, BLOCK_MAX, 299 - 2 * BLOCK_MAX]);
    assert_eq!(
        decode_key(&wor[0].0).unwrap().2,
        Some(wor[0].1[BLOCK_MAX - 1]),
        "a cut block is bounded by its last unit"
    );
    assert_eq!(decode_key(&wor[2].0).unwrap().2, None, "the rest is the tail");
    assert_clean(&db);
    // A unit that joins a full bounded block cuts it in two.
    update(&db, ids[50], b"word 50");
    assert_eq!(found(&db, b"word"), ids);
    let sizes: Vec<usize> =
        blocks(&db).into_iter().filter(|(k, _)| k.starts_with(&list)).map(|(_, u)| u.len()).collect();
    assert_eq!(sizes, [BLOCK_MAX, 1, BLOCK_MAX, 300 - 2 * BLOCK_MAX - 1]);
    assert_clean(&db);
    // Emptying a bounded block deletes it; the others keep their units.
    change(&db, |cs| {
        for id in &ids[..BLOCK_MAX] {
            cs.delete(*id, None).unwrap();
        }
    });
    let sizes: Vec<usize> =
        blocks(&db).into_iter().filter(|(k, _)| k.starts_with(&list)).map(|(_, u)| u.len()).collect();
    assert_eq!(sizes, [1, BLOCK_MAX, 300 - 2 * BLOCK_MAX - 1]);
    assert_eq!(found(&db, b"word"), ids[BLOCK_MAX..]);
    assert_clean(&db);
    // Turning the option off and on in one change set drops what it kept, then indexes
    // every active unit again, the one it created included.
    let (fresh, moved) = change(&db, |cs| {
        let fresh = cs.create(new("word fresh".into())).unwrap();
        let upd = UnitUpdate { payload: b"gone".to_vec(), schema_version: None, metadata: None };
        cs.update(ids[299], upd, None).unwrap();
        cs.set_substring("sym", false).unwrap();
        cs.set_substring("sym", true).unwrap();
        (fresh, ids[299])
    });
    assert_eq!(found(&db, b"word fresh"), [fresh]);
    assert_eq!(found(&db, b"gone"), [moved]);
    assert!(!found(&db, b"word").contains(&moved));
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

#[test]
fn verify_reports_a_missing_an_extra_and_a_misplaced_entry() {
    let disk = SimVfs::new(16);
    let db = open(&disk);
    let id = store(&db, b"hello world");
    let other = store(&db, b"hello there");
    let note = store_as(&db, "note", b"hello world");
    assert_clean(&db);
    let before = entries(&db);
    let restore = |db: &UnitDb<E>| {
        let now: Vec<_> = entries(db).into_iter().map(|(k, _)| (k, None)).collect();
        db.raw_commit(now).unwrap();
        db.raw_commit(before.iter().map(|(k, v)| (k.clone(), Some(v.clone()))).collect()).unwrap();
        assert_clean(db);
    };

    // The unit is gone from one block of a list it is in.
    let first = without(&db, id).into_iter().next().unwrap();
    db.raw_commit(vec![first]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // The unit is in no block.
    db.raw_commit(without(&db, id)).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // A trigram the payload does not call for.
    db.raw_commit(vec![(tail("sym", *b"zzz"), Some(encode_block(&[id])))]).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // A unit whose type has no index, and one that does not exist.
    for unit in [note, UnitId([0xEE; 16])] {
        db.raw_commit(vec![(tail("sym", *b"qqq"), Some(encode_block(&[unit])))]).unwrap();
        assert_eq!(rules(&db), ["UNIT-52"]);
        restore(&db);
    }

    // The right trigrams under another type.
    let mut swap = without(&db, id);
    for (k, _) in blocks(&db).into_iter().filter(|(_, u)| u.contains(&id)) {
        let gram = decode_key(&k).unwrap().1;
        swap.push((tail("doc", gram), Some(encode_block(&[id]))));
    }
    db.raw_commit(swap).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);

    // A bounded block with a unit above its bound, and blocks whose ranges overlap.
    let hel = list_key(&type_prefix("sym"), *b"hel");
    let (lo, hi) = (id.min(other), id.max(other));
    let low_bound = vec![(tail("sym", *b"hel"), None), (block_key(&hel, Some(lo)), Some(encode_block(&[lo, hi])))];
    db.raw_commit(low_bound).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);
    // Each unit once, but the tail holds a unit below the bound of the block before it.
    let overlap =
        vec![(tail("sym", *b"hel"), Some(encode_block(&[lo]))), (block_key(&hel, Some(hi)), Some(encode_block(&[hi])))];
    db.raw_commit(overlap).unwrap();
    assert_eq!(rules(&db), ["UNIT-52"]);
    restore(&db);
}

#[test]
fn verify_reports_an_entry_that_does_not_decode() {
    let disk = SimVfs::new(17);
    let db = open(&disk);
    let id = store(&db, b"hello");
    let (k, v) = entries(&db).into_iter().next().unwrap();
    let ascending = encode_block(&[UnitId([1; 16]), UnitId([2; 16])]);
    let descending = encode_block(&[UnitId([2; 16]), UnitId([1; 16])]);
    let over = encode_block(&vec![id; BLOCK_MAX + 1]);
    let partial = [&v[..], &[0u8; 5]].concat();
    for bad in [vec![], vec![0, 0], vec![1, 1], vec![1, 0], vec![2, 0], partial, descending, over] {
        db.raw_commit(vec![(k.clone(), Some(bad.clone()))]).unwrap();
        assert!(!db.verify_units().unwrap().is_clean(), "value {bad:?}");
    }
    assert!(decode_block(&ascending).is_ok());
    db.raw_commit(vec![(k.clone(), Some(v.clone()))]).unwrap();
    assert_clean(&db);
    // A key one byte short, one with an empty type, and one with neither a bound nor the tail mark.
    let unmarked = [&k[..k.len() - 1], &[7u8][..]].concat();
    for bad_key in [k[..k.len() - 1].to_vec(), [&[TAG_TRIGRAM, 0][..], &[b'a'; 20]].concat(), unmarked] {
        db.raw_commit(vec![(bad_key.clone(), Some(v.clone()))]).unwrap();
        assert!(!db.verify_units().unwrap().is_clean(), "key {bad_key:?}");
        db.raw_commit(vec![(bad_key, None)]).unwrap();
        assert_clean(&db);
    }
    assert!(decode_key(&[TAG_TRIGRAM]).is_err());
    assert!(decode_key(&[&[TAG_TRIGRAM, 1, 0xFF][..], &[b'a'; 4]].concat()).is_err(), "a type that is not UTF-8");
    assert!(
        decode_key(&[&[TAG_TRIGRAM, 1, b's'][..], b"abc", &[BOUNDED], &[1; 15]].concat()).is_err(),
        "a short bound"
    );
    assert_eq!(
        decode_key(&[&[TAG_TRIGRAM, 1, b's'][..], b"abc", &[TAIL]].concat()).unwrap(),
        ("s".into(), *b"abc", None)
    );
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
    let stray: Vec<_> = grams(b"ghost").into_iter().map(|g| (tail("sym", g), Some(encode_block(&[ghost])))).collect();
    db.raw_commit(stray).unwrap();
    assert!(matches!(
        db.search_substring(b"ghost", None, None),
        Err(UnitError::Storage(kerno_core::Error::Corruption(m))) if m.contains("no UNIT_HEAD")
    ));
    // A block that does not decode in a list a search reads.
    db.raw_commit(vec![(tail("sym", *b"hel"), Some(vec![1, 0, 1]))]).unwrap();
    assert!(matches!(
        db.search_substring(b"hel", None, None),
        Err(UnitError::Storage(kerno_core::Error::Corruption(_)))
    ));
}

#[test]
fn a_batch_of_units_writes_the_same_blocks_as_one_commit_each() {
    let payloads: Vec<Vec<u8>> = (0..300).map(|i| format!("symbol_{i}_{}", i % 7).into_bytes()).collect();
    let (batch, single) = (SimVfs::new(22), SimVfs::new(23));
    let (one, many) = (open(&batch), open(&single));
    change(&one, |cs| {
        for p in &payloads {
            let new = NewUnit { unit_type: "sym".into(), schema_version: 1, metadata: vec![], payload: p.clone() };
            cs.create(new).unwrap();
        }
    });
    for p in &payloads {
        store(&many, p);
    }
    assert_eq!(blocks(&one), blocks(&many));
    assert_clean(&one);
}

/// The shortcuts of a flush (appending to the tail's bytes, writing a list of new units without
/// looking for its blocks) write what the general path writes: the same units committed in
/// batches of any size, with an update of an old unit among them, give the same blocks, and a
/// list left with bounded blocks and no tail takes new units in a new tail.
#[test]
fn appends_in_batches_of_any_size_write_the_same_blocks() {
    let new = |p: &str| NewUnit {
        unit_type: "sym".into(),
        schema_version: 1,
        metadata: vec![],
        payload: p.as_bytes().to_vec(),
    };
    let payloads: Vec<String> = (0..700).map(|i| format!("token_{}_{i}", i % 5)).collect();
    let (single, batched) = (SimVfs::new(31), SimVfs::new(32));
    let (one, many) = (open(&single), open(&batched));
    for p in &payloads {
        store(&one, p.as_bytes());
    }
    // Batches of 1, 7, 150 (over a block) and 300 (over two): appends that stay in the tail,
    // fill it exactly, and cut it once or more.
    let mut rest = payloads.as_slice();
    for size in [1usize, 7, 150, 300, 1, 128, 113] {
        let (now, later) = rest.split_at(size.min(rest.len()));
        rest = later;
        change(&many, |cs| {
            for p in now {
                cs.create(new(p)).unwrap();
            }
        });
    }
    assert!(rest.is_empty());
    assert_eq!(blocks(&one), blocks(&many));
    assert_clean(&many);

    // An update of an old unit before the creations of its change set: the change set does
    // not learn that its new units are above the stored ones, and writes the same blocks.
    let ids = found(&one, b"token_");
    assert_eq!(ids.len(), 700);
    for db in [&one, &many] {
        change(db, |cs| {
            let upd = UnitUpdate { payload: b"token_9_moved zzq".to_vec(), schema_version: None, metadata: None };
            cs.update(ids[3], upd, None).unwrap();
            cs.create(new("token_9_fresh zzq")).unwrap();
        });
    }
    assert_eq!(blocks(&one), blocks(&many));
    assert_eq!(found(&many, b"zzq").len(), 2);
    assert_clean(&many);

    // Empty the tail of a list that has bounded blocks: it is deleted and the bounded blocks
    // stay. New units then go to a new tail, above every bound.
    let list = list_key(&type_prefix("sym"), *b"tok");
    let of_list = |db: &UnitDb<E>| -> Vec<(Vec<u8>, Vec<UnitId>)> {
        blocks(db).into_iter().filter(|(k, _)| k.starts_with(&list)).collect()
    };
    let before = of_list(&many);
    let (tail_key, tail_units) = before.last().cloned().unwrap();
    assert_eq!(decode_key(&tail_key).unwrap().2, None);
    assert!(before.len() > 1, "the list has bounded blocks");
    change(&many, |cs| {
        for id in &tail_units {
            cs.delete(*id, None).unwrap();
        }
    });
    assert!(of_list(&many).iter().all(|(k, _)| decode_key(k).unwrap().2.is_some()), "no tail is left");
    let added: Vec<UnitId> =
        change(&many, |cs| (0..3).map(|i| cs.create(new(&format!("token_x_{i}"))).unwrap()).collect());
    let after = of_list(&many);
    assert_eq!(after.len(), before.len());
    assert_eq!(after.last().map(|(k, u)| (decode_key(k).unwrap().2, u.clone())), Some((None, added.clone())));
    assert_eq!(found(&many, b"token_x"), added);
    assert_clean(&many);
}
